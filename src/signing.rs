#![cfg(feature = "signing")]

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use ed25519_dalek::{pkcs8::DecodePrivateKey, SigningKey, Signer};
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};

use crate::types::{KeyPair, PohTx};

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

/// Generate a fresh Ed25519 [`KeyPair`] (PKCS8 PEM private, SPKI PEM public).
pub fn generate_key_pair() -> Result<KeyPair, Box<dyn std::error::Error>> {
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let signing_key = SigningKey::from_bytes(&seed);
    Ok(KeyPair {
        signing_private_key: private_key_to_pem(&signing_key),
        signing_public_key:  public_key_to_pem(&signing_key.verifying_key()),
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

// ── Transaction hash ──────────────────────────────────────────────────────

/// SHA-256 of the canonical JSON payload, returned as a lowercase hex string.
pub fn compute_tx_hash(from: &str, to: &str, amount: i64, fee: i64, nonce: i64, timestamp: i64, memo: &str) -> String {
    let canonical = format!(
        r#"{{"from":{},"to":{},"amount":{},"fee":{},"nonce":{},"timestamp":{},"memo":{}}}"#,
        serde_json::to_string(from).unwrap(),
        serde_json::to_string(to).unwrap(),
        amount, fee, nonce, timestamp,
        serde_json::to_string(memo).unwrap(),
    );
    let digest = Sha256::digest(canonical.as_bytes());
    bytes_to_hex(&digest)
}

// ── Transaction building ──────────────────────────────────────────────────

/// Build an unsigned [`PohTx`]. `amount_poh` is in whole POH (1 POH = 1e9 μPOH).
pub fn build_transfer(from: &str, to: &str, amount_poh: f64, nonce: i64, fee: i64, memo: &str) -> Result<PohTx, Box<dyn std::error::Error>> {
    if amount_poh <= 0.0 {
        return Err("amount_poh must be positive".into());
    }
    let amount = (amount_poh * 1_000_000_000.0).round() as i64;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as i64;
    Ok(PohTx {
        from: from.to_owned(), to: to.to_owned(),
        amount, fee, nonce, timestamp,
        memo: memo.to_owned(),
        tx_hash: None, signature: None, signing_public_key: None,
    })
}

// ── Transaction signing ───────────────────────────────────────────────────

/// Sign a [`PohTx`], filling in `tx_hash`, `signature`, and `signing_public_key`.
pub fn sign_transaction(tx: &PohTx, private_key_pem: &str) -> Result<PohTx, Box<dyn std::error::Error>> {
    let signing_key = SigningKey::from_pkcs8_pem(private_key_pem)
        .map_err(|e| format!("failed to decode PKCS8 PEM: {e}"))?;

    let tx_hash = compute_tx_hash(&tx.from, &tx.to, tx.amount, tx.fee, tx.nonce, tx.timestamp, &tx.memo);
    let signature = signing_key.sign(tx_hash.as_bytes());
    let signature_b64 = B64.encode(signature.to_bytes());
    let public_pem = public_key_to_pem(&signing_key.verifying_key());

    Ok(PohTx {
        tx_hash: Some(tx_hash),
        signature: Some(signature_b64),
        signing_public_key: Some(public_pem),
        from: tx.from.clone(), to: tx.to.clone(),
        amount: tx.amount, fee: tx.fee, nonce: tx.nonce, timestamp: tx.timestamp,
        memo: tx.memo.clone(),
    })
}
