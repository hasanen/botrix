//! Shared enumerations and newtype wrappers used across the API boundary.

use serde::{Deserialize, Serialize};

/// Connection status of the Matrix bot account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixConnectionStatus {
    /// No session is configured.
    Disconnected,
    /// OAuth Device Authorization Grant is in progress.
    Connecting,
    /// Session is active and the sync loop is running.
    Connected,
    /// An error occurred; manual intervention may be required.
    Error,
}

/// Bot membership state in a Matrix room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomMembershipState {
    /// The bot has not been invited or joined.
    Unknown,
    /// The bot has been invited but has not yet accepted.
    Invited,
    /// The bot is a member of the room.
    Joined,
    /// The bot has left the room.
    Left,
    /// The bot has been banned from the room.
    Banned,
}

/// Delivery status of a notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationStatus {
    /// Waiting to be picked up by the background worker.
    Pending,
    /// Currently being processed by the background worker.
    Processing,
    /// Successfully delivered to Matrix.
    Delivered,
    /// All retry attempts exhausted; delivery failed.
    Failed,
}

/// Log severity level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LogLevel {
    /// Informational message.
    Info,
    /// Warning that does not prevent operation.
    Warn,
    /// Error that may affect functionality.
    Error,
}
