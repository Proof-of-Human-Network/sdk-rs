//! chat-crypto — portable public-job chat encryption for the POH Rust SDK.
//!
//! Public compute jobs are raced by miners the requester doesn't control, so the
//! on-chain record of the prompt/reply is sealed to the requester's X25519 key:
//!
//! ```text
//! X25519 (ECDH) -> HKDF-SHA256 -> AES-256-GCM
//! ```
//!
//! Byte-identical to the node reference (poh-miner `src/security/chat-crypto.js`,
//! verified round-trip) and the JS/Python SDKs. See CHAT-CRYPTO.md for the wire format.
//!
//! Enable with the `chatcrypto` feature.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use hkdf::Hkdf;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

const SEAL_INFO: &[u8] = b"poh-chat-seal-v1";
const SCALAR_INFO: &[u8] = b"poh-x25519-v1";

/// A sealed chat envelope (all fields base64).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedEnvelope {
    pub v: u8,
    pub alg: String,
    /// raw 32-byte ephemeral public key
    pub epk: String,
    /// 12-byte IV
    pub iv: String,
    /// ciphertext || 16-byte GCM tag
    pub ct: String,
}

/// A wallet's raw 32-byte X25519 encryption keypair (base64).
#[derive(Debug, Clone)]
pub struct EncryptionKeypair {
    pub public_key_b64: String,
    pub private_key_b64: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ChatCryptoError {
    #[error("base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("recipient X25519 pubkey must be 32 bytes")]
    BadRecipientKey,
    #[error("private scalar must be 32 bytes")]
    BadPrivateKey,
    #[error("unsupported chat-crypto envelope")]
    BadEnvelope,
    #[error("AES-GCM decryption failed")]
    Decrypt,
}

fn to_arr32(b: &[u8]) -> Option<[u8; 32]> {
    if b.len() == 32 {
        let mut a = [0u8; 32];
        a.copy_from_slice(b);
        Some(a)
    } else {
        None
    }
}

fn derive_key(shared: &[u8], recipient_pub: &[u8], epk: &[u8]) -> [u8; 32] {
    let mut salt = Vec::with_capacity(recipient_pub.len() + epk.len());
    salt.extend_from_slice(recipient_pub);
    salt.extend_from_slice(epk);
    let hk = Hkdf::<Sha256>::new(Some(&salt), shared);
    let mut okm = [0u8; 32];
    hk.expand(SEAL_INFO, &mut okm).expect("32 is a valid length");
    okm
}

/// Deterministically derive the wallet's X25519 keypair from a stable secret (its
/// ed25519 signing private key PEM), matching the node.
pub fn derive_encryption_keypair(stable_secret: &[u8]) -> EncryptionKeypair {
    let hk = Hkdf::<Sha256>::new(Some(&[]), stable_secret);
    let mut scalar = [0u8; 32];
    hk.expand(SCALAR_INFO, &mut scalar).expect("32 is a valid length");
    let secret = StaticSecret::from(scalar);
    let public = PublicKey::from(&secret);
    EncryptionKeypair {
        public_key_b64: B64.encode(public.as_bytes()),
        private_key_b64: B64.encode(scalar),
    }
}

/// Seal a plaintext to a recipient's raw X25519 public key (base64).
pub fn seal(recipient_pub_b64: &str, plaintext: &[u8]) -> Result<SealedEnvelope, ChatCryptoError> {
    let recipient_raw =
        to_arr32(&B64.decode(recipient_pub_b64)?).ok_or(ChatCryptoError::BadRecipientKey)?;
    let recipient_pub = PublicKey::from(recipient_raw);

    let mut esk_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut esk_bytes);
    let esk = StaticSecret::from(esk_bytes);
    let epk = PublicKey::from(&esk);
    let shared = esk.diffie_hellman(&recipient_pub);
    let key = derive_key(shared.as_bytes(), &recipient_raw, epk.as_bytes());

    let mut iv = [0u8; 12];
    OsRng.fill_bytes(&mut iv);
    let cipher = Aes256Gcm::new((&key).into());
    let ct = cipher
        .encrypt(Nonce::from_slice(&iv), Payload { msg: plaintext, aad: &[] })
        .map_err(|_| ChatCryptoError::Decrypt)?;

    Ok(SealedEnvelope {
        v: 1,
        alg: "x25519-hkdf-sha256-aes256gcm".to_string(),
        epk: B64.encode(epk.as_bytes()),
        iv: B64.encode(iv),
        ct: B64.encode(ct),
    })
}

/// Open an envelope with the recipient's raw X25519 private scalar (base64).
pub fn open(env: &SealedEnvelope, private_scalar_b64: &str) -> Result<String, ChatCryptoError> {
    if env.v != 1 {
        return Err(ChatCryptoError::BadEnvelope);
    }
    let scalar = to_arr32(&B64.decode(private_scalar_b64)?).ok_or(ChatCryptoError::BadPrivateKey)?;
    let secret = StaticSecret::from(scalar);
    let recipient_pub = PublicKey::from(&secret);
    let epk_raw = to_arr32(&B64.decode(&env.epk)?).ok_or(ChatCryptoError::BadEnvelope)?;
    let shared = secret.diffie_hellman(&PublicKey::from(epk_raw));
    let key = derive_key(shared.as_bytes(), recipient_pub.as_bytes(), &epk_raw);

    let iv = B64.decode(&env.iv)?;
    let ct = B64.decode(&env.ct)?;
    let cipher = Aes256Gcm::new((&key).into());
    let pt = cipher
        .decrypt(Nonce::from_slice(&iv), Payload { msg: &ct, aad: &[] })
        .map_err(|_| ChatCryptoError::Decrypt)?;
    Ok(String::from_utf8_lossy(&pt).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let kp = derive_encryption_keypair(b"rust-secret");
        let env = seal(&kp.public_key_b64, b"hello rust").unwrap();
        assert_eq!(open(&env, &kp.private_key_b64).unwrap(), "hello rust");
    }

    #[test]
    fn deterministic_keypair() {
        let a = derive_encryption_keypair(b"same");
        let b = derive_encryption_keypair(b"same");
        assert_eq!(a.public_key_b64, b.public_key_b64);
        assert_ne!(derive_encryption_keypair(b"diff").public_key_b64, a.public_key_b64);
    }

    // Byte-compat with the node reference: this envelope was produced by the node's
    // src/security/chat-crypto.js for derive_encryption_keypair("rust-interop").
    #[test]
    fn opens_node_sealed_envelope() {
        let kp = derive_encryption_keypair(b"rust-interop");
        assert_eq!(kp.public_key_b64, "XWeuTjf5gk1B9EUaRYB0mBaRRudIfFn2CZkcsFp2NWc=");
        let env = SealedEnvelope {
            v: 1,
            alg: "x25519-hkdf-sha256-aes256gcm".to_string(),
            epk: "9Jgr/SzalkizcEPDyTPgaWL0zreJPcpxPzkQA33GgSw=".to_string(),
            iv: "5vNG7exFDLDJRdlb".to_string(),
            ct: "B+2GpffQMNnXB0UhDhtBT5Vw7e3FJWnL/XMsTObXel7O26NtIAhv".to_string(),
        };
        assert_eq!(open(&env, &kp.private_key_b64).unwrap(), "hello from node to rust");
    }

    #[test]
    fn wrong_key_fails() {
        let kp = derive_encryption_keypair(b"a");
        let other = derive_encryption_keypair(b"b");
        let env = seal(&kp.public_key_b64, b"x").unwrap();
        assert!(open(&env, &other.private_key_b64).is_err());
    }
}
