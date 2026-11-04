//! Matrix API types.

use serde::{Deserialize, Serialize};

use crate::types::{MatrixConnectionStatus, RoomMembershipState};

/// Response body for `GET /api/v1/matrix/status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixStatus {
    /// Current connection status.
    pub status: MatrixConnectionStatus,
    /// Matrix user ID, e.g. `@botrix:example.com`.
    pub user_id: Option<String>,
    /// Homeserver URL.
    pub homeserver: Option<String>,
    /// Whether E2EE has been bootstrapped.
    pub e2ee_bootstrapped: bool,
    /// Unix timestamp of the last successful connection, if any.
    pub connected_at: Option<i64>,
}

/// Response body for `POST /api/v1/matrix/connect`.
///
/// Botrix uses the OAuth 2.0 Authorization Code flow. The client should open
/// `authorization_url` (the homeserver's login page); after the operator logs
/// in there, the homeserver redirects back to Botrix's callback endpoint,
/// which completes the login. There is nothing to poll.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectResponse {
    /// The homeserver authorization URL to open in the browser.
    pub authorization_url: String,
}

/// Request body for `POST /api/v1/matrix/disconnect`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisconnectRequest {
    /// Must be `true` to proceed; acts as a confirmation guard.
    pub confirm: bool,
}

/// A single destination room entry returned by `GET /api/v1/matrix/rooms`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomInfo {
    /// Logical destination name from config.
    pub name: String,
    /// Matrix room ID.
    pub room_id: String,
    /// Resolved display name of the room, if known.
    pub display_name: Option<String>,
    /// Current membership state of the bot in this room.
    pub membership: RoomMembershipState,
    /// Whether the room uses E2EE.
    pub encrypted: bool,
    /// Whether this destination is enabled in config.
    pub enabled: bool,
}

/// Request body for `POST /api/v1/matrix/test-message`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestMessageRequest {
    /// Logical destination name to send the test message to.
    pub destination: String,
}

/// Request body for `POST /api/v1/matrix/recovery-key`.
#[derive(Clone, Serialize, Deserialize)]
pub struct RecoveryKeyRequest {
    /// The Matrix key backup recovery key.
    pub recovery_key: String,
}

impl std::fmt::Debug for RecoveryKeyRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecoveryKeyRequest")
            .field("recovery_key", &"[redacted]")
            .finish()
    }
}
