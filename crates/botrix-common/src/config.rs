//! Configuration struct types shared between the server and frontend.
//!
//! These types are used for deserialization only; file I/O is handled
//! exclusively in `botrix-server`.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

/// Root configuration file structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Matrix homeserver and destination configuration.
    pub matrix: MatrixConfig,
    /// Webhook endpoint definitions.
    #[serde(default)]
    pub webhooks: HashMap<String, WebhookConfig>,
    /// Command triggers, keyed by command name (matched case-insensitively).
    #[serde(default)]
    pub triggers: HashMap<String, TriggerConfig>,
    /// Reusable room/user allowlists that triggers can reference by name.
    #[serde(default)]
    pub access_profiles: HashMap<String, AccessProfileConfig>,
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

/// A command trigger: a Matrix command that calls an external HTTP webhook.
///
/// Secret *values* are never part of the config; only their names are declared
/// in [`TriggerConfig::secrets`] and referenced from header values as
/// `{{secret:NAME}}`. Unknown keys are rejected so that a typo such as
/// `allowed_room` cannot silently drop an access restriction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerConfig {
    /// Human-readable description shown by the `help` command.
    #[serde(default)]
    pub description: String,
    /// Whether the command is active (default: `true`).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// HTTP method: `GET`, `POST`, `PUT`, `PATCH`, or `DELETE`.
    pub method: String,
    /// Webhook URL (`http` or `https`).
    pub url: String,
    /// Outbound headers; values may contain `{{secret:NAME}}` placeholders.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Names of the secrets this trigger uses. Values are set in the web UI.
    #[serde(default)]
    pub secrets: Vec<String>,
    /// Template used to render the reply from a JSON response; `None` means a
    /// status-only reply.
    #[serde(default)]
    pub response_template: Option<String>,
    /// Name of an entry in `access_profiles` to take allowlists from.
    #[serde(default)]
    pub access_profile: Option<String>,
    /// Inline allowed room IDs; non-empty overrides the profile's list.
    #[serde(default)]
    pub allowed_rooms: Vec<String>,
    /// Inline allowed user IDs; non-empty overrides the profile's list.
    #[serde(default)]
    pub allowed_users: Vec<String>,
}

/// A reusable allowlist of rooms and users.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessProfileConfig {
    /// Allowed Matrix room IDs (empty means any room).
    #[serde(default)]
    pub allowed_rooms: Vec<String>,
    /// Allowed Matrix user IDs (empty means any user).
    #[serde(default)]
    pub allowed_users: Vec<String>,
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

/// Whether a URL's host is a loopback address (`localhost`, `127.0.0.1`, `::1`).
///
/// Homeservers running Matrix Authentication Service reject a loopback host in
/// the OAuth client/redirect URI, so a loopback `public_url` makes the Matrix
/// connect flow fail. The server warns about this at startup.
pub fn is_loopback_url(url: &str) -> bool {
    // Strip scheme and any path to get the authority (host[:port]).
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let authority = after_scheme.split('/').next().unwrap_or("");

    let host = if let Some(rest) = authority.strip_prefix('[') {
        // Bracketed IPv6: `[::1]:8081` → the part inside the brackets.
        rest.split(']').next().unwrap_or("")
    } else if authority.matches(':').count() >= 2 {
        // Bare IPv6 with no brackets (no port can be attached this way).
        authority
    } else {
        // host or host:port
        authority.split(':').next().unwrap_or("")
    };

    let host = host.to_ascii_lowercase();
    host == "localhost" || host == "127.0.0.1" || host == "::1"
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

    #[test]
    fn loopback_urls_are_detected() {
        for url in [
            "http://localhost:8080",
            "https://localhost",
            "http://127.0.0.1:8081",
            "https://127.0.0.1/api/v1/matrix/callback",
            "http://[::1]:8080",
            "https://LOCALHOST:9000",
        ] {
            assert!(is_loopback_url(url), "{url} should be loopback");
        }
    }

    #[test]
    fn non_loopback_urls_are_not_flagged() {
        for url in [
            "https://botrix.example.com",
            "https://botrix.local",
            "https://10.50.10.28:8081",
            "https://192.168.1.5",
            "http://matrix.internal:8080/path",
        ] {
            assert!(!is_loopback_url(url), "{url} should not be loopback");
        }
    }
}
