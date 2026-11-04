//! Authentication and session API types.

use serde::{Deserialize, Serialize};

/// Request body for `POST /api/v1/auth/login`.
#[derive(Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    /// Admin username.
    pub username: String,
    /// Admin password (plaintext; transmitted over TLS).
    pub password: String,
}

impl std::fmt::Debug for LoginRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginRequest")
            .field("username", &self.username)
            .field("password", &"[redacted]")
            .finish()
    }
}

/// Response body for `GET /api/v1/auth/me`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Authenticated admin username.
    pub username: String,
}

/// Request body for `POST /api/v1/setup`.
#[derive(Clone, Serialize, Deserialize)]
pub struct SetupRequest {
    /// Initial admin username.
    pub username: String,
    /// Initial admin password (plaintext; transmitted over TLS).
    pub password: String,
}

impl std::fmt::Debug for SetupRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetupRequest")
            .field("username", &self.username)
            .field("password", &"[redacted]")
            .finish()
    }
}

/// Response body for `GET /api/v1/auth/csrf`.
///
/// The client must send `token` back in the `X-CSRF-Token` header on every
/// state-mutating request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsrfResponse {
    /// The CSRF authenticity token to echo back in `X-CSRF-Token`.
    pub token: String,
}
