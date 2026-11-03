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
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            max_body_bytes: default_max_body_bytes(),
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
