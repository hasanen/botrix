//! Application-level encryption of OAuth tokens at rest (plan §16.1).
//!
//! The Matrix `access_token` and `refresh_token` persisted in the
//! `matrix_state` table must be encrypted before they touch disk. This module
//! derives a 256-bit key from `BOTRIX_CRYPTO_PASSPHRASE` with HKDF-SHA256 and
//! encrypts each token with AES-256-GCM.
//!
//! This is distinct from, and layered on top of, the `matrix-sdk` SQLite store
//! passphrase (§16.10), which protects the SDK's own crypto/state stores. Here
//! we protect the two OAuth tokens the application itself persists.
//!
//! ## On-the-wire format
//!
//! Each encrypted value is stored as lowercase hex of
//! `nonce (12 bytes) || ciphertext || GCM tag (16 bytes)`. The random nonce is
//! generated per encryption so encrypting the same token twice yields distinct
//! ciphertexts.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::Sha256;

use super::MatrixError;

/// Length of the AES-256 key in bytes.
const KEY_LEN: usize = 32;

/// Length of the AES-GCM nonce in bytes (96 bits, the AES-GCM standard).
const NONCE_LEN: usize = 12;

/// HKDF `info` string binding derived keys to this specific use.
///
/// Changing this value invalidates all previously encrypted tokens, so it is
/// treated as a stable protocol constant.
const HKDF_INFO: &[u8] = b"botrix:matrix-oauth-token:v1";

/// Derive the AES-256 key from the operator passphrase via HKDF-SHA256.
///
/// No salt is used so the derivation is deterministic for a given passphrase;
/// confidentiality rests on the passphrase secrecy and the per-message nonce.
fn derive_key(passphrase: &str) -> [u8; KEY_LEN] {
    let hkdf = Hkdf::<Sha256>::new(None, passphrase.as_bytes());
    let mut key = [0u8; KEY_LEN];
    // `expand` only fails if the output length exceeds 255*HashLen; 32 bytes
    // is well within range for SHA-256, so this cannot fail here.
    hkdf.expand(HKDF_INFO, &mut key)
        .unwrap_or_else(|_| unreachable!("HKDF expand of 32 bytes is always valid"));
    key
}

/// Build an AES-256-GCM cipher from the passphrase.
fn cipher(passphrase: &str) -> Aes256Gcm {
    let key_bytes = derive_key(passphrase);
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    Aes256Gcm::new(key)
}

/// Encrypt a token string, returning hex of `nonce || ciphertext || tag`.
///
/// # Errors
///
/// Returns [`MatrixError::TokenEncryption`] if the AEAD operation fails.
pub fn encrypt_token(passphrase: &str, plaintext: &str) -> Result<String, MatrixError> {
    let cipher = cipher(passphrase);

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| MatrixError::TokenEncryption(e.to_string()))?;

    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(hex_encode(&out))
}

/// Decrypt a token produced by [`encrypt_token`].
///
/// # Errors
///
/// Returns [`MatrixError::TokenDecryption`] if the input is malformed, the
/// passphrase is wrong, or the authentication tag does not verify.
pub fn decrypt_token(passphrase: &str, stored: &str) -> Result<String, MatrixError> {
    let bytes = hex_decode(stored)
        .ok_or_else(|| MatrixError::TokenDecryption("value is not valid hex".to_owned()))?;
    if bytes.len() <= NONCE_LEN {
        return Err(MatrixError::TokenDecryption(
            "ciphertext too short".to_owned(),
        ));
    }
    let (nonce_bytes, ciphertext) = bytes.split_at(NONCE_LEN);
    let nonce = Nonce::from_slice(nonce_bytes);

    let cipher = cipher(passphrase);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| MatrixError::TokenDecryption(e.to_string()))?;

    String::from_utf8(plaintext)
        .map_err(|e| MatrixError::TokenDecryption(format!("decrypted token is not UTF-8: {e}")))
}

/// Lowercase hex-encode a byte slice.
fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Decode a lowercase/uppercase hex string, returning `None` if malformed.
fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let (pairs, _) = s.as_bytes().as_chunks::<2>();
    for pair in pairs {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASS: &str = "correct horse battery staple";

    #[test]
    fn round_trip_recovers_plaintext() {
        // Arrange
        let token = "syt_dGhpcy1pcy1hLW1vY2stdG9rZW4_abc123";

        // Act
        let encrypted = encrypt_token(PASS, token).unwrap();
        let decrypted = decrypt_token(PASS, &encrypted).unwrap();

        // Assert
        assert_eq!(decrypted, token);
    }

    #[test]
    fn ciphertext_differs_from_plaintext() {
        let token = "my-access-token";
        let encrypted = encrypt_token(PASS, token).unwrap();
        assert_ne!(encrypted, token);
        assert!(!encrypted.contains(token));
    }

    #[test]
    fn same_plaintext_encrypts_to_distinct_ciphertexts() {
        // The random per-message nonce must make outputs differ.
        let token = "repeat-me";
        let a = encrypt_token(PASS, token).unwrap();
        let b = encrypt_token(PASS, token).unwrap();
        assert_ne!(a, b);
        // ...yet both decrypt back to the same plaintext.
        assert_eq!(decrypt_token(PASS, &a).unwrap(), token);
        assert_eq!(decrypt_token(PASS, &b).unwrap(), token);
    }

    #[test]
    fn wrong_passphrase_fails_to_decrypt() {
        let encrypted = encrypt_token(PASS, "secret").unwrap();
        let err = decrypt_token("a-different-passphrase", &encrypted).unwrap_err();
        assert!(matches!(err, MatrixError::TokenDecryption(_)));
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        // Flip the last hex nibble; the GCM tag must no longer verify.
        let mut encrypted = encrypt_token(PASS, "secret").unwrap();
        let last = encrypted.pop().unwrap();
        let flipped = if last == '0' { '1' } else { '0' };
        encrypted.push(flipped);
        let err = decrypt_token(PASS, &encrypted).unwrap_err();
        assert!(matches!(err, MatrixError::TokenDecryption(_)));
    }

    #[test]
    fn malformed_hex_is_rejected() {
        let err = decrypt_token(PASS, "not-hex!!").unwrap_err();
        assert!(matches!(err, MatrixError::TokenDecryption(_)));
    }

    #[test]
    fn too_short_input_is_rejected() {
        // 4 hex chars = 2 bytes, less than the 12-byte nonce.
        let err = decrypt_token(PASS, "abcd").unwrap_err();
        assert!(matches!(err, MatrixError::TokenDecryption(_)));
    }

    #[test]
    fn empty_token_round_trips() {
        let encrypted = encrypt_token(PASS, "").unwrap();
        assert_eq!(decrypt_token(PASS, &encrypted).unwrap(), "");
    }
}
