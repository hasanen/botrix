//! Command trigger and access profile API types.
//!
//! Secret values are write-only: they appear only in [`SecretInput`] (request)
//! and never in any response type.

use serde::{Deserialize, Serialize};

/// A reusable allowlist of rooms and users, returned by `GET /api/v1/access-profiles`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessProfileDto {
    /// Database row ID.
    pub id: i64,
    /// Unique profile name.
    pub name: String,
    /// Allowed Matrix room IDs (empty means any room).
    pub allowed_rooms: Vec<String>,
    /// Allowed Matrix user IDs (empty means any user).
    pub allowed_users: Vec<String>,
}

/// Request body for creating or updating an access profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessProfileInput {
    /// Unique profile name.
    pub name: String,
    /// Allowed Matrix room IDs (empty means any room).
    pub allowed_rooms: Vec<String>,
    /// Allowed Matrix user IDs (empty means any user).
    pub allowed_users: Vec<String>,
}

/// An outbound HTTP header; the value may contain `{{secret:NAME}}` placeholders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderDto {
    /// Header name.
    pub name: String,
    /// Header value template.
    pub value: String,
}

/// Whether a named secret currently has a value. Never carries the value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretStatusDto {
    /// Secret name.
    pub name: String,
    /// Whether a value is stored.
    pub is_set: bool,
}

/// A command trigger returned by the triggers API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerDto {
    /// Database row ID.
    pub id: i64,
    /// Command word (matched case-insensitively).
    pub command: String,
    /// Human-readable description shown by `help`.
    pub description: String,
    /// Whether the trigger is active.
    pub enabled: bool,
    /// HTTP method used for the webhook call.
    pub method: String,
    /// Webhook URL.
    pub url: String,
    /// Template used to render the reply from a JSON response.
    pub response_template: Option<String>,
    /// Linked access profile, if any.
    pub access_profile_id: Option<i64>,
    /// Extra allowed room IDs for this trigger.
    pub allowed_rooms: Vec<String>,
    /// Extra allowed user IDs for this trigger.
    pub allowed_users: Vec<String>,
    /// Outbound headers.
    pub headers: Vec<HeaderDto>,
    /// Secret names and set-flags (no values).
    pub secrets: Vec<SecretStatusDto>,
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

/// A secret change in a trigger create/update request.
///
/// `value: None` with `clear: false` leaves the stored secret unchanged.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretInput {
    /// Secret name.
    pub name: String,
    /// New plaintext value, if it should be set or replaced.
    #[serde(default)]
    pub value: Option<String>,
    /// Remove the stored secret.
    #[serde(default)]
    pub clear: bool,
}

impl std::fmt::Debug for SecretInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretInput")
            .field("name", &self.name)
            .field("value", &self.value.as_ref().map(|_| "[redacted]"))
            .field("clear", &self.clear)
            .finish()
    }
}

/// Request body for creating or updating a trigger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerInput {
    /// Command word (matched case-insensitively).
    pub command: String,
    /// Human-readable description shown by `help`.
    pub description: String,
    /// Whether the trigger is active.
    pub enabled: bool,
    /// HTTP method used for the webhook call.
    pub method: String,
    /// Webhook URL.
    pub url: String,
    /// Template used to render the reply from a JSON response.
    pub response_template: Option<String>,
    /// Linked access profile, if any.
    pub access_profile_id: Option<i64>,
    /// Extra allowed room IDs for this trigger.
    pub allowed_rooms: Vec<String>,
    /// Extra allowed user IDs for this trigger.
    pub allowed_users: Vec<String>,
    /// Outbound headers.
    pub headers: Vec<HeaderDto>,
    /// Secret changes (omitted secrets stay unchanged).
    pub secrets: Vec<SecretInput>,
}

/// Request body for `POST /api/v1/triggers/{id}/test`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerTestRequest {
    /// Optional sample argument text, as if typed after the command.
    #[serde(default)]
    pub args: Option<String>,
}

/// Result of a trigger dry-run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerTestResponse {
    /// HTTP status returned by the webhook, if a response was received.
    pub status: Option<u16>,
    /// The reply that would be sent to the room.
    pub reply: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "s3cr3t-value-that-must-not-leak";

    fn sample_input() -> TriggerInput {
        TriggerInput {
            command: "deploy".into(),
            description: "Deploy the app".into(),
            enabled: true,
            method: "POST".into(),
            url: "https://example.com/hook".into(),
            response_template: Some("{{status}}".into()),
            access_profile_id: Some(3),
            allowed_rooms: vec!["!room:example.org".into()],
            allowed_users: vec!["@alice:example.org".into()],
            headers: vec![HeaderDto {
                name: "Authorization".into(),
                value: "Bearer {{secret:TOKEN}}".into(),
            }],
            secrets: vec![SecretInput {
                name: "TOKEN".into(),
                value: Some(SECRET.into()),
                clear: false,
            }],
        }
    }

    #[test]
    fn trigger_input_round_trips_through_json() {
        let input = sample_input();

        let json = serde_json::to_string(&input).unwrap();
        let back: TriggerInput = serde_json::from_str(&json).unwrap();

        assert_eq!(back, input);
    }

    #[test]
    fn secret_input_without_value_or_clear_means_unchanged() {
        let secret: SecretInput = serde_json::from_str(r#"{"name":"TOKEN"}"#).unwrap();

        assert_eq!(secret.value, None);
        assert!(!secret.clear);
    }

    #[test]
    fn trigger_dto_serializes_no_secret_value_key() {
        let dto = TriggerDto {
            id: 1,
            command: "deploy".into(),
            description: "d".into(),
            enabled: true,
            method: "GET".into(),
            url: "https://example.com".into(),
            response_template: None,
            access_profile_id: None,
            allowed_rooms: vec![],
            allowed_users: vec![],
            headers: vec![],
            secrets: vec![SecretStatusDto {
                name: "TOKEN".into(),
                is_set: true,
            }],
        };

        let value = serde_json::to_value(&dto).unwrap();
        let secret = &value["secrets"][0];
        let mut keys: Vec<&str> = secret
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
    fn trigger_info_serializes_no_secret_value_key() {
        let info = TriggerInfo {
            command: "deploy".into(),
            description: "d".into(),
            enabled: true,
            method: "POST".into(),
            url: "https://example.com".into(),
            header_names: vec!["Authorization".into()],
            access_profile: None,
            allowed_rooms: vec![],
            allowed_users: vec![],
            secrets: vec![SecretStatusDto {
                name: "TOKEN".into(),
                is_set: true,
            }],
        };

        let value = serde_json::to_value(&info).unwrap();

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
    fn secret_input_debug_redacts_the_value() {
        let debug = format!(
            "{:?}",
            SecretInput {
                name: "TOKEN".into(),
                value: Some(SECRET.into()),
                clear: false,
            }
        );

        assert!(debug.contains("TOKEN"));
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains(SECRET), "leaked secret: {debug}");
    }

    #[test]
    fn trigger_input_debug_does_not_leak_secret_value() {
        let debug = format!("{:?}", sample_input());

        assert!(!debug.contains(SECRET), "leaked secret: {debug}");
    }
}
