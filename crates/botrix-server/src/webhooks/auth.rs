//! Bearer token primitives for inbound webhook authentication.
//!
//! Tokens are high-entropy random strings. Only the SHA-256 hex digest is
//! ever persisted (see §16.2); the plaintext is shown to the operator exactly
//! once at generation time. Validation compares digests in constant time to
//! avoid timing side channels.

use constant_time_eq::constant_time_eq;
use sha2::{Digest, Sha256};

/// Number of random bytes in a generated token (256 bits of entropy).
const TOKEN_BYTES: usize = 32;

/// Generate a new random token, returning its URL-safe plaintext form.
///
/// The returned string is hex-encoded (64 chars) and carries
/// `TOKEN_BYTES * 8` bits of entropy.
pub fn generate_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    rand::fill(&mut bytes);
    hex_encode(&bytes)
}

/// Compute the SHA-256 hex digest of a token, as stored in the database.
pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    hex_encode(&digest)
}

/// Return whether `presented` matches the stored `expected_hash`.
///
/// Hashes `presented`, then compares the two digests in constant time.
pub fn verify_token(presented: &str, expected_hash: &str) -> bool {
    let presented_hash = hash_token(presented);
    constant_time_eq(presented_hash.as_bytes(), expected_hash.as_bytes())
}

/// Extract a Bearer token from an `Authorization` header value.
///
/// Accepts `Bearer <token>` (case-insensitive scheme). Returns `None` if the
/// header is absent or malformed.
pub fn parse_bearer(header_value: Option<&str>) -> Option<&str> {
    let value = header_value?;
    let (scheme, token) = value.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("bearer") && !token.is_empty() {
        Some(token.trim())
    } else {
        None
    }
}

/// Lowercase hex-encode a byte slice.
fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_have_expected_length_and_entropy() {
        // 32 bytes hex-encoded => 64 chars.
        let token = generate_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generated_tokens_are_unique() {
        let a = generate_token();
        let b = generate_token();
        assert_ne!(a, b);
    }

    #[test]
    fn hash_is_deterministic_and_not_plaintext() {
        let token = "my-secret-token";
        let h1 = hash_token(token);
        let h2 = hash_token(token);
        assert_eq!(h1, h2);
        assert_ne!(h1, token);
        // SHA-256 hex digest is 64 chars.
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn verify_accepts_matching_token() {
        let token = generate_token();
        let hash = hash_token(&token);
        assert!(verify_token(&token, &hash));
    }

    #[test]
    fn verify_rejects_wrong_token() {
        let hash = hash_token("the-real-token");
        assert!(!verify_token("a-different-token", &hash));
    }

    #[test]
    fn parse_bearer_extracts_token() {
        assert_eq!(parse_bearer(Some("Bearer abc123")), Some("abc123"));
        assert_eq!(parse_bearer(Some("bearer abc123")), Some("abc123"));
    }

    #[test]
    fn parse_bearer_rejects_malformed() {
        assert_eq!(parse_bearer(None), None);
        assert_eq!(parse_bearer(Some("abc123")), None);
        assert_eq!(parse_bearer(Some("Basic abc123")), None);
        assert_eq!(parse_bearer(Some("Bearer ")), None);
    }
}
