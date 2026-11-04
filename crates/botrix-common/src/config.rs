//! Configuration struct types shared between the server and frontend.
//!
//! These types are used for deserialization only; file I/O is handled
//! exclusively in `botrix-server`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Root configuration file structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Matrix homeserver and destination configuration.
    pub matrix: MatrixConfig,
    /// Webhook endpoint definitions.
    #[serde(default)]
    pub webhooks: HashMap<String, WebhookConfig>,
    /// HTTP server configuration.
    #[serde(default)]
    pub server: ServerConfig,
}

/// Matrix-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixConfig {
    /// HTTPS URL of the Matrix homeserver.
    pub homeserver: String,
    /// Named room destinations.
    #[serde(default)]
    pub destinations: HashMap<String, DestinationConfig>,
}

/// A single Matrix room destination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestinationConfig {
    /// Matrix room ID, e.g. `!abc123:example.com`.
    pub room_id: String,
    /// Whether messages can be sent to this destination (default: `true`).
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// A single webhook endpoint definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    /// Whether this endpoint accepts requests (default: `true`).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Logical destination name from `matrix.destinations`.
    pub destination: String,
    /// Message template string using `${variable}` syntax.
    pub msg_template: String,
}

/// HTTP server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Address to bind (default: `"0.0.0.0"`).
    #[serde(default = "default_host")]
    pub host: String,
    /// Port to listen on (default: `8080`).
    #[serde(default = "default_port")]
    pub port: u16,
    /// Maximum inbound webhook body size in bytes (default: 256 KiB).
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: usize,
    /// Public base URL at which Botrix is reachable, e.g.
    /// `https://botrix.example.com`. Required for the Matrix OAuth login
    /// redirect to come back to the right place; the homeserver redirects to
    /// `{public_url}/api/v1/matrix/callback`. When unset, the callback origin
    /// is derived from the request, which only works without a reverse proxy.
    #[serde(default)]
    pub public_url: Option<String>,
}

impl ServerConfig {
    /// The absolute OAuth redirect URI, from `public_url` if set.
    ///
    /// Returns `None` when `public_url` is not configured, so the caller can
    /// fall back to deriving it from the incoming request.
    pub fn oauth_redirect_uri(&self) -> Option<String> {
        self.public_url
            .as_deref()
            .map(|base| format!("{}{}", base.trim_end_matches('/'), OAUTH_CALLBACK_PATH))
    }
}

/// Path of the OAuth authorization-code redirect (callback) endpoint.
pub const OAUTH_CALLBACK_PATH: &str = "/api/v1/matrix/callback";

/// Build the OAuth redirect URI from an explicit origin (scheme://host[:port]).
///
/// Used to derive the callback from the request origin when `public_url` is
/// not configured.
pub fn oauth_redirect_uri_for_origin(origin: &str) -> String {
    format!("{}{}", origin.trim_end_matches('/'), OAUTH_CALLBACK_PATH)
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            max_body_bytes: default_max_body_bytes(),
            public_url: None,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_host() -> String {
    "0.0.0.0".to_owned()
}

fn default_port() -> u16 {
    8080
}

fn default_max_body_bytes() -> usize {
    // 256 KiB
    262_144
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_uri_is_none_without_public_url() {
        assert_eq!(ServerConfig::default().oauth_redirect_uri(), None);
    }

    #[test]
    fn redirect_uri_appends_callback_path() {
        let cfg = ServerConfig {
            public_url: Some("https://botrix.example.com".to_owned()),
            ..ServerConfig::default()
        };
        assert_eq!(
            cfg.oauth_redirect_uri().as_deref(),
            Some("https://botrix.example.com/api/v1/matrix/callback")
        );
    }

    #[test]
    fn redirect_uri_tolerates_trailing_slash() {
        let cfg = ServerConfig {
            public_url: Some("https://botrix.example.com/".to_owned()),
            ..ServerConfig::default()
        };
        assert_eq!(
            cfg.oauth_redirect_uri().as_deref(),
            Some("https://botrix.example.com/api/v1/matrix/callback")
        );
    }

    #[test]
    fn redirect_uri_for_origin_builds_absolute_url() {
        assert_eq!(
            oauth_redirect_uri_for_origin("http://localhost:8080"),
            "http://localhost:8080/api/v1/matrix/callback"
        );
    }
}
