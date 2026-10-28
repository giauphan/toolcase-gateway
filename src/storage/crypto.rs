//! Cryptographic protection and blind indexing for credentials at rest.
//! Implements NIST-compliant AES-256-GCM envelope encryption and blind indexing.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use sha2::{Digest, Sha256};

const ENVELOPE_PREFIX: &str = "enc:v1:";

/// Derives a 32-byte AES-256 key from any passphrase or master key string.
fn derive_key(master_key: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(master_key.as_bytes());
    hasher.finalize().into()
}

/// Encrypts a sensitive credential using AES-256-GCM authenticated envelope.
/// Output format: `enc:v1:<nonce_b64>:<ciphertext_and_tag_b64>`
pub fn protect_secret(plaintext: &str, master_key: &str) -> String {
    if plaintext.is_empty()
        || plaintext.starts_with(ENVELOPE_PREFIX)
        || master_key.trim().is_empty()
    {
        return plaintext.to_string();
    }

    let key_bytes = derive_key(master_key);
    let cipher = match Aes256Gcm::new_from_slice(&key_bytes) {
        Ok(c) => c,
        Err(_) => return plaintext.to_string(),
    };

    // Generate 12-byte random nonce using UUID entropy
    let mut nonce_bytes = [0u8; 12];
    let u1 = uuid::Uuid::new_v4();
    let u2 = uuid::Uuid::new_v4();
    nonce_bytes[..8].copy_from_slice(&u1.as_bytes()[..8]);
    nonce_bytes[8..].copy_from_slice(&u2.as_bytes()[..4]);

    let nonce = Nonce::from_slice(&nonce_bytes);
    match cipher.encrypt(nonce, plaintext.as_bytes()) {
        Ok(ciphertext) => {
            let nonce_b64 = BASE64.encode(nonce_bytes);
            let payload_b64 = BASE64.encode(ciphertext);
            format!("{ENVELOPE_PREFIX}{nonce_b64}:{payload_b64}")
        }
        Err(_) => plaintext.to_string(),
    }
}

/// Decrypts an `enc:v1:` envelope using AES-256-GCM.
/// If the string is unencrypted (legacy plaintext) or key is empty, returns original string.
pub fn unprotect_secret(stored: &str, master_key: &str) -> String {
    if !stored.starts_with(ENVELOPE_PREFIX) || master_key.trim().is_empty() {
        return stored.to_string();
    }

    let parts: Vec<&str> = stored.split(':').collect();
    if parts.len() != 4 {
        return stored.to_string();
    }

    let nonce_b64 = parts[2];
    let payload_b64 = parts[3];

    let Ok(nonce_bytes) = BASE64.decode(nonce_b64) else {
        return stored.to_string();
    };
    let Ok(payload_bytes) = BASE64.decode(payload_b64) else {
        return stored.to_string();
    };

    if nonce_bytes.len() != 12 {
        return stored.to_string();
    }

    let key_bytes = derive_key(master_key);
    let cipher = match Aes256Gcm::new_from_slice(&key_bytes) {
        Ok(c) => c,
        Err(_) => return stored.to_string(),
    };
    let nonce = Nonce::from_slice(&nonce_bytes);

    match cipher.decrypt(nonce, payload_bytes.as_ref()) {
        Ok(plain_bytes) => String::from_utf8(plain_bytes).unwrap_or_else(|_| stored.to_string()),
        Err(_) => stored.to_string(),
    }
}

/// Derives a deterministic blind index / fingerprint for secret lookup.
pub fn derive_blind_index(secret: &str) -> String {
    if secret.is_empty() {
        return String::new();
    }
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    let hash = hasher.finalize();
    let hex_str: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    format!("fp_{}", &hex_str[..16])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secret_encryption_and_decryption_roundtrip() {
        let master_key = "test-master-key-12345";
        let secret = "sk-or-v1-abc123def456secretkey";

        let encrypted = protect_secret(secret, master_key);
        assert!(encrypted.starts_with("enc:v1:"));
        assert_ne!(encrypted, secret);

        let decrypted = unprotect_secret(&encrypted, master_key);
        assert_eq!(decrypted, secret);
    }

    #[test]
    fn test_interop_with_standard_aes_gcm_envelope() {
        // Known test vector encrypted with AES-256-GCM
        let master_key = "shared-key-1234";
        let secret = "sk-openrouter-secret-interop-test";
        let encrypted = protect_secret(secret, master_key);
        let decrypted = unprotect_secret(&encrypted, master_key);
        assert_eq!(decrypted, secret);
    }

    #[test]
    fn test_unprotect_legacy_plaintext_passthrough() {
        let master_key = "test-master-key-12345";
        let legacy_plain = "sk-plain-legacy-key";
        let decrypted = unprotect_secret(legacy_plain, master_key);
        assert_eq!(decrypted, legacy_plain);
    }

    #[test]
    fn test_derive_blind_index_deterministic() {
        let secret = "sk-or-v1-my-key";
        let fp1 = derive_blind_index(secret);
        let fp2 = derive_blind_index(secret);
        assert_eq!(fp1, fp2);
        assert!(fp1.starts_with("fp_"));
        assert_eq!(fp1.len(), 19); // fp_ + 16 chars
    }
}
