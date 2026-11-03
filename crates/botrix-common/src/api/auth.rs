//! Authentication and session API types.

use serde::{Deserialize, Serialize};

/// Request body for `POST /api/v1/auth/login`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    /// Admin username.
    pub username: String,
    /// Admin password (plaintext; transmitted over TLS).
    pub password: String,
}

/// Response body for `GET /api/v1/auth/me`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Authenticated admin username.
    pub username: String,
}

/// Request body for `POST /api/v1/setup`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupRequest {
    /// Initial admin username.
    pub username: String,
    /// Initial admin password (plaintext; transmitted over TLS).
    pub password: String,
}
