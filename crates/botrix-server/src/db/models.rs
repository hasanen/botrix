//! Row types that document the shape of each database table.

/// A row from the `admins` table.
pub struct Admin {
    /// Primary key.
    pub id: i64,
    /// Unique admin username.
    pub username: String,
    /// Argon2id password hash.
    pub password_hash: String,
    /// Unix timestamp of creation.
    pub created_at: i64,
    /// Unix timestamp of last update.
    pub updated_at: i64,
}

/// A row from the `matrix_state` table (singleton, `id = 1`).
pub struct MatrixState {
    /// Primary key (always 1).
    pub id: i64,
    /// Matrix user ID, e.g. `@botrix:example.com`.
    pub user_id: Option<String>,
    /// Homeserver URL.
    pub homeserver: Option<String>,
    /// AES-256-GCM encrypted access token (base64).
    pub access_token: Option<String>,
    /// AES-256-GCM encrypted refresh token (base64).
    pub refresh_token: Option<String>,
    /// Matrix device ID.
    pub device_id: Option<String>,
    /// OAuth client ID.
    pub oauth_client_id: Option<String>,
    /// Unix timestamp of first successful connection.
    pub connected_at: Option<i64>,
    /// Unix timestamp of last token refresh.
    pub last_token_refresh: Option<i64>,
    /// Connection status string (see CHECK constraint in migration).
    pub status: String,
}

/// A row from the `webhook_tokens` table.
pub struct WebhookToken {
    /// Primary key.
    pub id: i64,
    /// Webhook identifier matching the YAML key.
    pub webhook_id: String,
    /// SHA-256 hex digest of the token; `NULL` means no token is set.
    pub token_hash: Option<String>,
    /// Whether the webhook endpoint is enabled.
    pub enabled: bool,
    /// Unix timestamp when the token was first created.
    pub created_at: Option<i64>,
    /// Unix timestamp of last update.
    pub updated_at: i64,
}

/// A row from the `notifications` table.
#[derive(Debug)]
pub struct Notification {
    /// Primary key.
    pub id: i64,
    /// Webhook identifier that produced this notification.
    pub webhook_id: String,
    /// Logical destination name.
    pub destination: String,
    /// Fully rendered message text.
    pub rendered_message: String,
    /// Delivery status string (see CHECK constraint in migration).
    pub status: String,
    /// Number of delivery attempts made so far.
    pub attempts: i64,
    /// Last error message, if any.
    pub last_error: Option<String>,
    /// Unix timestamp of insertion.
    pub created_at: i64,
    /// Unix timestamp of successful delivery.
    pub delivered_at: Option<i64>,
    /// Unix timestamp after which this row may be deleted (created_at + 30 days).
    pub expires_at: i64,
}

/// A row from the `log_events` table.
#[derive(Debug)]
pub struct LogEvent {
    /// Primary key.
    pub id: i64,
    /// Severity level: `INFO`, `WARN`, or `ERROR`.
    pub level: String,
    /// Component that produced the event: `matrix`, `webhook`, `queue`, `auth`.
    pub component: String,
    /// Human-readable message.
    pub message: String,
    /// Optional JSON-encoded details (no secrets).
    pub details: Option<String>,
    /// Unix timestamp of the event.
    pub created_at: i64,
}

/// A row from the `session_settings` table (singleton, `id = 1`).
#[derive(Debug)]
pub struct SessionSettings {
    /// Primary key (always 1).
    pub id: i64,
    /// Session idle timeout in seconds.
    pub idle_timeout_secs: i64,
    /// Absolute session lifetime in seconds.
    pub absolute_lifetime_secs: i64,
    /// Unix timestamp of last update.
    pub updated_at: i64,
}

super::debug_redacting!(Admin { id, username, created_at, updated_at; secret: password_hash });
super::debug_redacting!(MatrixState {
    id, user_id, homeserver, device_id, oauth_client_id, connected_at,
    last_token_refresh, status;
    secret: access_token, refresh_token
});
super::debug_redacting!(WebhookToken { id, webhook_id, enabled, created_at, updated_at; secret: token_hash });
