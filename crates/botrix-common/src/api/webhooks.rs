//! Webhook API types.

use serde::{Deserialize, Serialize};

use crate::types::NotificationStatus;

/// A single webhook endpoint entry returned by `GET /api/v1/webhooks`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookInfo {
    /// Webhook identifier (matches the YAML key and the URL path segment).
    pub id: String,
    /// Whether the webhook endpoint is enabled in config.
    pub enabled: bool,
    /// Logical destination name.
    pub destination: String,
    /// Whether a valid token is currently set.
    pub has_token: bool,
}

/// Response for `POST /api/v1/webhooks/:id/token`.
///
/// The plaintext token is returned exactly once and must be copied immediately.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenGenerateResponse {
    /// Plaintext token (shown once, never stored).
    pub token: String,
}

/// A single notification history entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationEntry {
    /// Database row ID.
    pub id: i64,
    /// Webhook identifier that produced this notification.
    pub webhook_id: String,
    /// Logical destination name.
    pub destination: String,
    /// Rendered message text (may be truncated for display).
    pub rendered_message: String,
    /// Delivery status.
    pub status: NotificationStatus,
    /// Number of delivery attempts made.
    pub attempts: i64,
    /// Last error message, if any.
    pub last_error: Option<String>,
    /// Unix timestamp when the notification was created.
    pub created_at: i64,
    /// Unix timestamp when the notification was successfully delivered, if any.
    pub delivered_at: Option<i64>,
}

/// Paginated list of notifications.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationListResponse {
    /// Items on this page.
    pub items: Vec<NotificationEntry>,
    /// Current page number (1-indexed).
    pub page: u32,
    /// Items per page.
    pub per_page: u32,
    /// Total number of notifications.
    pub total: i64,
}
