//! Command trigger API types.
//!
//! Triggers are defined in the YAML config and are read-only over the API.
//! Secret values are write-only: they appear only in [`SecretSetRequest`]
//! (request) and never in any response type.

use serde::{Deserialize, Serialize};

/// Whether a named secret currently has a value. Never carries the value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretStatusDto {
    /// Secret name.
    pub name: String,
    /// Whether a value is stored.
    pub is_set: bool,
}

/// A config-defined command trigger as shown by `GET /api/v1/triggers`.
///
/// Read-only: triggers are defined in the YAML config. Only secret values can
/// be changed through the API, and they never appear in any response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerInfo {
    /// Lowercase command word.
    pub command: String,
    /// Human-readable description shown by `help`.
    pub description: String,
    /// Whether the command is active.
    pub enabled: bool,
    /// HTTP method used for the webhook call.
    pub method: String,
    /// Webhook URL.
    pub url: String,
    /// Names of the configured outbound headers (values are not returned).
    pub header_names: Vec<String>,
    /// Name of the referenced access profile, if any.
    pub access_profile: Option<String>,
    /// Effective allowed room IDs after applying the profile (empty means any).
    pub allowed_rooms: Vec<String>,
    /// Effective allowed user IDs after applying the profile (empty means any).
    pub allowed_users: Vec<String>,
    /// Declared secrets and whether each has a stored value.
    pub secrets: Vec<SecretStatusDto>,
}

/// Request body for `PUT /api/v1/triggers/{command}/secrets/{name}`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretSetRequest {
    /// The new plaintext secret value. Write-only.
    pub value: String,
}

impl std::fmt::Debug for SecretSetRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretSetRequest")
            .field("value", &"[redacted]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "s3cr3t-value-that-must-not-leak";

    fn sample_info() -> TriggerInfo {
        TriggerInfo {
            command: "deploy".into(),
            description: "d".into(),
            enabled: true,
            method: "POST".into(),
            url: "https://example.com".into(),
            header_names: vec!["Authorization".into()],
            access_profile: Some("ops".into()),
            allowed_rooms: vec!["!r:example.com".into()],
            allowed_users: vec![],
            secrets: vec![SecretStatusDto {
                name: "TOKEN".into(),
                is_set: true,
            }],
        }
    }

    #[test]
    fn trigger_info_round_trips_through_json() {
        let info = sample_info();

        let json = serde_json::to_string(&info).unwrap();
        let back: TriggerInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(back, info);
    }

    #[test]
    fn trigger_info_serializes_no_secret_value_key() {
        let value = serde_json::to_value(sample_info()).unwrap();

        let mut keys: Vec<&str> = value["secrets"][0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();

        assert_eq!(keys, ["is_set", "name"]);
    }

    #[test]
    fn secret_set_request_debug_redacts_the_value() {
        // Arrange
        let request = SecretSetRequest {
            value: SECRET.into(),
        };

        // Act
        let debug = format!("{request:?}");

        // Assert
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains(SECRET), "leaked secret: {debug}");
    }

    #[test]
    fn secret_set_request_deserializes_from_the_wire_shape() {
        let request: SecretSetRequest = serde_json::from_str(r#"{"value":"abc"}"#).unwrap();

        assert_eq!(request.value, "abc");
    }
}
