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
/// The client should display `verification_uri` and `user_code` to the user,
/// then poll `GET /api/v1/matrix/connect/poll` every few seconds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectResponse {
    /// The URL the user must visit to authorize the device.
    pub verification_uri: String,
    /// The short code the user must enter at `verification_uri`.
    pub user_code: String,
    /// Seconds until the device code expires.
    pub expires_in: u64,
}

/// Response body for `GET /api/v1/matrix/connect/poll`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectPollResponse {
    /// Current polling status.
    pub status: ConnectPollStatus,
    /// Human-readable error message, present when `status` is `error`.
    pub message: Option<String>,
}

/// Status returned by the OAuth polling endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectPollStatus {
    /// Authorization is still pending — the user has not yet acted.
    Pending,
    /// Authorization succeeded and the Matrix session is active.
    Connected,
    /// The device code expired before the user authorized.
    Expired,
    /// An error occurred during the authorization flow.
    Error,
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
