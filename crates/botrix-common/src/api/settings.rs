//! Settings API types.

use serde::{Deserialize, Serialize};

/// Response body for `GET /api/v1/settings`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Session idle timeout in seconds.
    pub idle_timeout_secs: i64,
    /// Absolute session lifetime in seconds.
    pub absolute_lifetime_secs: i64,
}

/// Request body for `PUT /api/v1/settings`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSettingsRequest {
    /// Session idle timeout in seconds.
    pub idle_timeout_secs: i64,
    /// Absolute session lifetime in seconds.
    pub absolute_lifetime_secs: i64,
}

/// Request body for `POST /api/v1/settings/password`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangePasswordRequest {
    /// Current password for verification.
    pub current_password: String,
    /// New password to set.
    pub new_password: String,
}

/// Response body for `GET /api/v1/settings/info`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    /// Application version string, e.g. `"0.1.0"`.
    pub version: String,
    /// Git commit hash of the build, if available.
    pub build_hash: Option<String>,
}
