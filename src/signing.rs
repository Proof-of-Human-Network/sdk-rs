#![cfg(feature = "signing")]

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use ed25519_dalek::{pkcs8::DecodePrivateKey, SigningKey, Signer};
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};

use crate::types::{KeyPair, DAITx};

// ── DER prefixes for Ed25519 PEM ──────────────────────────────────────────────
//
// PKCS8 private key DER prefix (16 bytes) + 32-byte seed = 48-byte DER
const PKCS8_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06,
    0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];
// SPKI public key DER prefix (12 bytes) + 32-byte public key = 44-byte DER
const SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65,
    0x70, 0x03, 0x21, 0x00,
];

// ── PEM helpers ───────────────────────────────────────────────────────────────

fn pem_encode(der: &[u8], type_str: &str) -> String {
    let b64 = B64.encode(der);
    let lines: String = b64
        .as_bytes()
        .chunks(64)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    format!("-----BEGIN {type_str}-----\n{lines}\n-----END {type_str}-----\n")
}

fn private_key_to_pem(key: &SigningKey) -> String {
    let seed: &[u8; 32] = key.as_bytes();
    let mut der = [0u8; 48];
    der[..16].copy_from_slice(&PKCS8_PREFIX);
    der[16..].copy_from_slice(seed);
    pem_encode(&der, "PRIVATE KEY")
}

fn public_key_to_pem(key: &ed25519_dalek::VerifyingKey) -> String {
    let pub_bytes = key.to_bytes();
    let mut der = [0u8; 44];
    der[..12].copy_from_slice(&SPKI_PREFIX);
    der[12..].copy_from_slice(&pub_bytes);
    pem_encode(&der, "PUBLIC KEY")
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// ── Key generation ─────────────────────────────────────────────────────────

/// Derive the canonical dai address bound to an ed25519 SPKI PEM public key.
pub fn derive_address_from_signing_key(signing_public_key: &str) -> String {
    let digest = Sha256::digest(signing_public_key.as_bytes());
    format!("dai{}", bytes_to_hex(&digest)[..40].to_string())
}

/// Generate a fresh Ed25519 [`KeyPair`] (PKCS8 PEM private, SPKI PEM public).
pub fn generate_key_pair() -> Result<KeyPair, Box<dyn std::error::Error>> {
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let signing_key = SigningKey::from_bytes(&seed);
    let signing_public_key = public_key_to_pem(&signing_key.verifying_key());
    Ok(KeyPair {
        signing_private_key: private_key_to_pem(&signing_key),
        signing_public_key:  signing_public_key.clone(),
        address:             derive_address_from_signing_key(&signing_public_key),
    })
}

// ── Low-level signing ──────────────────────────────────────────────────────

/// Sign arbitrary UTF-8 `message` bytes with a PKCS8 PEM private key.
/// Returns the 64-byte Ed25519 signature as standard base64.
pub fn sign_data(message: &str, private_key_pem: &str) -> Result<String, Box<dyn std::error::Error>> {
    let signing_key = SigningKey::from_pkcs8_pem(private_key_pem)
        .map_err(|e| format!("failed to decode PKCS8 PEM: {e}"))?;
    let signature = signing_key.sign(message.as_bytes());
    Ok(B64.encode(signature.to_bytes()))
}

/// Create the registration proof: sign the wallet address, return base64.
pub fn create_signing_proof(wallet_address: &str, private_key_pem: &str) -> Result<String, Box<dyn std::error::Error>> {
    sign_data(wallet_address, private_key_pem)
}

/// Create the rotation proof required to replace an existing registered key.
pub fn create_rotation_proof(
    address: &str,
    new_signing_public_key: &str,
    existing_private_key_pem: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let payload = format!(
        r#"{{"action":"rotate-key","address":{},"newSigningPublicKey":{}}}"#,
        serde_json::to_string(address).unwrap(),
        serde_json::to_string(new_signing_public_key).unwrap(),
    );
    sign_data(&payload, existing_private_key_pem)
}

// ── Transaction hash ──────────────────────────────────────────────────────

/// SHA-256 of the canonical JSON payload, returned as a lowercase hex string.
pub fn compute_tx_hash(from: &str, to: &str, amount: i64, fee: i64, nonce: i64, timestamp: i64, memo: &str) -> String {
    compute_tx_hash_with_currency(from, to, amount, fee, nonce, timestamp, memo, None)
}

/// Currency-aware tx hash. LOCKSTEP with the node: `currency` joins the
/// preimage after `memo` ONLY when non-DAI — a DAI tx hashes byte-identically
/// to the historical shape.
#[allow(clippy::too_many_arguments)]
pub fn compute_tx_hash_with_currency(from: &str, to: &str, amount: i64, fee: i64, nonce: i64, timestamp: i64, memo: &str, currency: Option<&str>) -> String {
    let currency_part = match currency {
        Some(c) if c != "DAI" => format!(r#","currency":{}"#, serde_json::to_string(c).unwrap()),
        _ => String::new(),
    };
    let canonical = format!(
        r#"{{"from":{},"to":{},"amount":{},"fee":{},"nonce":{},"timestamp":{},"memo":{}{}}}"#,
        serde_json::to_string(from).unwrap(),
        serde_json::to_string(to).unwrap(),
        amount, fee, nonce, timestamp,
        serde_json::to_string(memo).unwrap(),
        currency_part,
    );
    let digest = Sha256::digest(canonical.as_bytes());
    bytes_to_hex(&digest)
}

/// Decimals for an on-chain asset (mirror of the node's /api/assets registry).
pub fn decimals_of(currency: Option<&str>) -> u32 {
    match currency {
        Some("aiGEL") | Some("aiKGS") | Some("aiAMD") | Some("aiETB") | Some("aiBTN") => 2,
        _ => 9,
    }
}

// ── Job IDs ────────────────────────────────────────────────────────────────

/// Generate a client-side job id (`job-<millis>-<8 random hex chars>`), matching
/// the shape the node itself would generate if `id` were omitted from the request.
/// Fee-required jobs must fix the id before signing, since the payment proof is
/// bound to it.
pub fn generate_job_id() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let mut suffix = [0u8; 4];
    OsRng.fill_bytes(&mut suffix);
    format!("job-{}-{}", millis, bytes_to_hex(&suffix))
}

// ── Job fee payment ────────────────────────────────────────────────────────

/// Compute the canonical payment hash for a job fee. Binds the fee to one
/// specific job + miner + amount + nonce, so a signature over it can't be
/// replayed against a different job or a higher budget. Must byte-for-byte
/// match the node's own `computeJobPaymentHash`.
pub fn compute_job_payment_hash(
    job_id: &str,
    requester_address: &str,
    miner_address: &str,
    amount: i64,
    nonce: i64,
) -> String {
    compute_job_payment_hash_with_currency(job_id, requester_address, miner_address, amount, nonce, None)
}

/// Currency-aware job payment hash. LOCKSTEP with the node: `currency` is the
/// SIXTH key ONLY when non-DAI.
pub fn compute_job_payment_hash_with_currency(
    job_id: &str,
    requester_address: &str,
    miner_address: &str,
    amount: i64,
    nonce: i64,
    currency: Option<&str>,
) -> String {
    let currency_part = match currency {
        Some(c) if c != "DAI" => format!(r#","currency":{}"#, serde_json::to_string(c).unwrap()),
        _ => String::new(),
    };
    let canonical = format!(
        r#"{{"jobId":{},"requesterAddress":{},"minerAddress":{},"amount":{},"nonce":{}{}}}"#,
        serde_json::to_string(job_id).unwrap(),
        serde_json::to_string(requester_address).unwrap(),
        serde_json::to_string(miner_address).unwrap(),
        amount, nonce,
        currency_part,
    );
    let digest = Sha256::digest(canonical.as_bytes());
    bytes_to_hex(&digest)
}

/// Sign a fee payment authorizing a fee-required job (skill execution, or a
/// model/dataset compute job). The result (`txHash`, `signature`) goes in the
/// `paymentTx` field of a `POST /job` request — the node verifies the
/// signature and debits the requester's balance before it will run the job.
pub fn sign_job_payment(
    job_id: &str,
    requester_address: &str,
    miner_address: &str,
    amount: i64,
    nonce: i64,
    private_key_pem: &str,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let tx_hash = compute_job_payment_hash(job_id, requester_address, miner_address, amount, nonce);
    let signature = sign_data(&tx_hash, private_key_pem)?;
    Ok((tx_hash, signature))
}

// ── Transaction building ──────────────────────────────────────────────────

/// Build an unsigned [`DAITx`]. `amount_dai` is in whole DAI (1 DAI = 1e9 μDAI).
pub fn build_transfer(from: &str, to: &str, amount_dai: f64, nonce: i64, fee: i64, memo: &str) -> Result<DAITx, Box<dyn std::error::Error>> {
    if amount_dai <= 0.0 {
        return Err("amount_dai must be positive".into());
    }
    let amount = (amount_dai * 1_000_000_000.0).round() as i64;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as i64;
    Ok(DAITx {
        from: from.to_owned(), to: to.to_owned(),
        amount, fee, nonce, timestamp,
        memo: memo.to_owned(),
        currency: None,
        tx_hash: None, signature: None, signing_public_key: None,
    })
}

/// Build an unsigned [`DAITx`] denominated in a stablecoin. `amount` is in the
/// asset's DISPLAY units (e.g. 12.50 aiGEL → 1250 raw at 2 decimals).
pub fn build_transfer_with_currency(from: &str, to: &str, amount_display: f64, nonce: i64, fee: i64, memo: &str, currency: &str) -> Result<DAITx, Box<dyn std::error::Error>> {
    if amount_display <= 0.0 {
        return Err("amount must be positive".into());
    }
    let cur = if currency == "DAI" { None } else { Some(currency.to_owned()) };
    let amount = (amount_display * 10f64.powi(decimals_of(cur.as_deref()) as i32)).round() as i64;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as i64;
    Ok(DAITx {
        from: from.to_owned(), to: to.to_owned(),
        amount, fee, nonce, timestamp,
        memo: memo.to_owned(),
        currency: cur,
        tx_hash: None, signature: None, signing_public_key: None,
    })
}

// ── Transaction signing ───────────────────────────────────────────────────

/// Sign a [`DAITx`], filling in `tx_hash`, `signature`, and `signing_public_key`.
pub fn sign_transaction(tx: &DAITx, private_key_pem: &str) -> Result<DAITx, Box<dyn std::error::Error>> {
    let signing_key = SigningKey::from_pkcs8_pem(private_key_pem)
        .map_err(|e| format!("failed to decode PKCS8 PEM: {e}"))?;

    let tx_hash = compute_tx_hash_with_currency(&tx.from, &tx.to, tx.amount, tx.fee, tx.nonce, tx.timestamp, &tx.memo, tx.currency.as_deref());
    let signature = signing_key.sign(tx_hash.as_bytes());
    let signature_b64 = B64.encode(signature.to_bytes());
    let public_pem = public_key_to_pem(&signing_key.verifying_key());

    Ok(DAITx {
        tx_hash: Some(tx_hash),
        signature: Some(signature_b64),
        signing_public_key: Some(public_pem),
        from: tx.from.clone(), to: tx.to.clone(),
        amount: tx.amount, fee: tx.fee, nonce: tx.nonce, timestamp: tx.timestamp,
        memo: tx.memo.clone(),
        currency: tx.currency.clone(),
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{pkcs8::DecodePublicKey, Verifier, VerifyingKey};

    #[test]
    fn generate_key_pair_returns_valid_pem_strings() {
        let kp = generate_key_pair().unwrap();
        assert!(kp.signing_private_key.contains("-----BEGIN PRIVATE KEY-----"));
        assert!(kp.signing_public_key.contains("-----BEGIN PUBLIC KEY-----"));
        assert!(kp.address.starts_with("dai"));
        assert_eq!(kp.address, derive_address_from_signing_key(&kp.signing_public_key));
    }

    #[test]
    fn generate_key_pair_produces_different_keys_each_call() {
        let kp1 = generate_key_pair().unwrap();
        let kp2 = generate_key_pair().unwrap();
        assert_ne!(kp1.signing_private_key, kp2.signing_private_key);
    }

    #[test]
    fn sign_data_returns_64_byte_signature() {
        let kp = generate_key_pair().unwrap();
        let sig = sign_data("hello world", &kp.signing_private_key).unwrap();
        let bytes = B64.decode(&sig).unwrap();
        assert_eq!(bytes.len(), 64);
    }

    #[test]
    fn sign_data_is_deterministic() {
        let kp = generate_key_pair().unwrap();
        let s1 = sign_data("msg", &kp.signing_private_key).unwrap();
        let s2 = sign_data("msg", &kp.signing_private_key).unwrap();
        assert_eq!(s1, s2);
    }

    #[test]
    fn sign_data_differs_for_different_messages() {
        let kp = generate_key_pair().unwrap();
        let s1 = sign_data("A", &kp.signing_private_key).unwrap();
        let s2 = sign_data("B", &kp.signing_private_key).unwrap();
        assert_ne!(s1, s2);
    }

    #[test]
    fn create_signing_proof_equals_sign_data_of_address() {
        let kp = generate_key_pair().unwrap();
        let addr = "dai_test_address";
        assert_eq!(
            create_signing_proof(addr, &kp.signing_private_key).unwrap(),
            sign_data(addr, &kp.signing_private_key).unwrap()
        );
    }

    #[test]
    fn sign_data_verifies_with_public_key() {
        use ed25519_dalek::Signature;
        let kp = generate_key_pair().unwrap();
        let sig_b64 = sign_data("verify me", &kp.signing_private_key).unwrap();
        let sig_bytes = B64.decode(&sig_b64).unwrap();
        let vk = VerifyingKey::from_public_key_pem(&kp.signing_public_key).unwrap();
        let sig = Signature::from_slice(&sig_bytes).unwrap();
        vk.verify(b"verify me", &sig).expect("signature must be valid");
    }

    #[test]
    fn compute_tx_hash_returns_64_char_hex() {
        let h = compute_tx_hash("daiA", "daiB", 1_000_000_000, 0, 1, 1_700_000_000_000, "");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn compute_tx_hash_is_deterministic() {
        let h1 = compute_tx_hash("daiA", "daiB", 1_000_000_000, 0, 1, 1_700_000_000_000, "");
        let h2 = compute_tx_hash("daiA", "daiB", 1_000_000_000, 0, 1, 1_700_000_000_000, "");
        assert_eq!(h1, h2);
    }

    #[test]
    fn compute_tx_hash_differs_for_different_amounts() {
        let h1 = compute_tx_hash("daiA", "daiB", 1_000_000_000, 0, 1, 1_700_000_000_000, "");
        let h2 = compute_tx_hash("daiA", "daiB", 2_000_000_000, 0, 1, 1_700_000_000_000, "");
        assert_ne!(h1, h2);
    }

    /// Fixed value computed by the node's own algorithm — `crypto.createHash('sha256')
    /// .update(JSON.stringify({from,to,amount,fee,nonce,timestamp,memo})).digest('hex')` —
    /// for these exact inputs. The node recomputes and verifies this hash server-side
    /// (WalletManager.applyTransaction), so any mismatch here means real transactions
    /// built by this crate would be silently rejected.
    #[test]
    fn compute_tx_hash_matches_node_reference_value() {
        let h = compute_tx_hash("daiA", "daiB", 1_000_000_000, 5, 3, 1_700_000_000_000, "hello");
        assert_eq!(h, "e309a41e0c088876f2763f8d01ae434ff060bd4391202d555be1d96ee0f14c8a");
    }

    // ── job payment ──────────────────────────────────────────────────────────

    #[test]
    fn compute_job_payment_hash_returns_64_char_hex() {
        let h = compute_job_payment_hash("job-1", "daiA", "daiMiner", 500_000_000, 0);
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn compute_job_payment_hash_is_deterministic() {
        let h1 = compute_job_payment_hash("job-1", "daiA", "daiMiner", 500_000_000, 0);
        let h2 = compute_job_payment_hash("job-1", "daiA", "daiMiner", 500_000_000, 0);
        assert_eq!(h1, h2);
    }

    /// Fixed value computed by the node's own algorithm for these exact inputs — see
    /// `computeJobPaymentHash` in miner-node.js. The node recomputes and verifies this
    /// hash server-side before debiting the requester, so any mismatch here means real
    /// jobs submitted by this crate would be rejected outright. Same fixture used in
    /// the JS, Python, iOS, and Android SDKs.
    #[test]
    fn compute_job_payment_hash_matches_node_reference_value() {
        let h = compute_job_payment_hash("job-abc", "daiAlice", "daiMiner", 500_000_000, 3);
        assert_eq!(h, "1ed86280c1ab64d60d55a232a1c339299d32d8bd45e5f2bf26ff72b26d8908c0");
    }

    #[test]
    fn sign_job_payment_returns_tx_hash_and_signature() {
        let kp = generate_key_pair().unwrap();
        let (tx_hash, signature) = sign_job_payment(
            "job-1", "daiA", "daiMiner", 500_000_000, 0, &kp.signing_private_key,
        ).unwrap();
        assert_eq!(tx_hash, compute_job_payment_hash("job-1", "daiA", "daiMiner", 500_000_000, 0));
        assert!(!signature.is_empty());
    }

    #[test]
    fn build_transfer_converts_dai_to_μdai() {
        let tx = build_transfer("daiA", "daiB", 1.5, 3, 0, "").unwrap();
        assert_eq!(tx.amount, 1_500_000_000);
    }

    #[test]
    fn build_transfer_rejects_zero_amount() {
        assert!(build_transfer("daiA", "daiB", 0.0, 1, 0, "").is_err());
    }

    #[test]
    fn sign_transaction_fills_in_signing_fields() {
        let kp = generate_key_pair().unwrap();
        let tx = build_transfer("daiA", "daiB", 2.0, 1, 0, "").unwrap();
        let signed = sign_transaction(&tx, &kp.signing_private_key).unwrap();
        assert!(signed.tx_hash.is_some());
        assert!(signed.signature.is_some());
        assert!(signed.signing_public_key.as_ref().unwrap().contains("-----BEGIN PUBLIC KEY-----"));
    }

    #[test]
    fn sign_transaction_preserves_original_fields() {
        let kp = generate_key_pair().unwrap();
        let tx = build_transfer("daiA", "daiB", 3.0, 7, 500, "hello").unwrap();
        let signed = sign_transaction(&tx, &kp.signing_private_key).unwrap();
        assert_eq!(signed.from, tx.from);
        assert_eq!(signed.to, tx.to);
        assert_eq!(signed.amount, tx.amount);
        assert_eq!(signed.nonce, tx.nonce);
    }

    #[test]
    fn sign_transaction_signature_verifies() {
        use ed25519_dalek::Signature;
        let kp = generate_key_pair().unwrap();
        let tx = build_transfer("daiA", "daiB", 1.0, 1, 0, "").unwrap();
        let signed = sign_transaction(&tx, &kp.signing_private_key).unwrap();
        let sig_bytes = B64.decode(signed.signature.as_ref().unwrap()).unwrap();
        let vk = VerifyingKey::from_public_key_pem(signed.signing_public_key.as_ref().unwrap()).unwrap();
        let sig = Signature::from_slice(&sig_bytes).unwrap();
        vk.verify(signed.tx_hash.as_ref().unwrap().as_bytes(), &sig)
            .expect("signature must be valid");
    }
}
