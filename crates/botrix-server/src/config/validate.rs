//! Validation rules applied to the deserialized configuration.
//!
//! All rules in §7.1 of the implementation plan are enforced here.

use thiserror::Error;

use botrix_common::config::Config;

/// A single validation failure.
#[derive(Debug, Error)]
pub enum ValidationError {
    /// `matrix.homeserver` is missing or not a valid HTTPS URL.
    #[error("matrix.homeserver must be a valid HTTPS URL, got: '{0}'")]
    InvalidHomeserver(String),

    /// A destination `room_id` does not match the required format.
    #[error(
        "destinations.{name}.room_id has invalid format '{room_id}' — a room ID must start with '!' (e.g. '!abc:example.com' or '!abc' for newer rooms); a '#alias:server' is not a room ID"
    )]
    InvalidRoomId {
        /// Destination name whose room ID is invalid.
        name: String,
        /// The offending room ID value.
        room_id: String,
    },

    /// A webhook references a destination that is not defined.
    #[error(
        "webhooks.{webhook}.destination '{destination}' is not defined in matrix.destinations"
    )]
    UnknownDestination {
        /// Webhook identifier that references the missing destination.
        webhook: String,
        /// The destination name that is not defined.
        destination: String,
    },

    /// A webhook `msg_template` is empty.
    #[error("webhooks.{0}.msg_template must not be empty")]
    EmptyTemplate(String),

    /// `server.port` is out of range.
    #[error("server.port must be between 1 and 65535, got: {0}")]
    InvalidPort(u16),

    /// A trigger command is empty or contains whitespace.
    #[error("triggers.'{0}': the command must be non-empty and contain no whitespace")]
    InvalidTriggerCommand(String),

    /// A trigger uses the name of a built-in command.
    #[error("triggers.{0}: 'help' is a built-in command and cannot be redefined")]
    ReservedTriggerCommand(String),

    /// Two trigger keys are the same command once lowercased.
    #[error(
        "triggers.{0}: another trigger defines the same command (commands are case-insensitive)"
    )]
    DuplicateTriggerCommand(String),

    /// A trigger's HTTP method is not one of the supported uppercase verbs.
    #[error(
        "triggers.{command}.method must be one of GET, POST, PUT, PATCH, DELETE, got: '{method}'"
    )]
    InvalidTriggerMethod {
        /// Trigger command.
        command: String,
        /// The offending method value.
        method: String,
    },

    /// A trigger's URL is not an absolute `http`/`https` URL with a host.
    #[error("triggers.{command}.url must be an http or https URL with a host, got: '{url}'")]
    InvalidTriggerUrl {
        /// Trigger command.
        command: String,
        /// The offending URL value.
        url: String,
    },

    /// A trigger references an access profile that is not defined.
    #[error("triggers.{command}.access_profile '{profile}' is not defined in access_profiles")]
    UnknownAccessProfile {
        /// Trigger command.
        command: String,
        /// The profile name that is not defined.
        profile: String,
    },

    /// An allowed room ID (trigger or access profile) is malformed.
    #[error(
        "{owner}.allowed_rooms has invalid room ID '{room_id}' — a room ID must start with '!'"
    )]
    InvalidAllowedRoom {
        /// Config path of the list, e.g. `triggers.deploy`.
        owner: String,
        /// The offending room ID.
        room_id: String,
    },

    /// An allowed user ID (trigger or access profile) is malformed.
    #[error(
        "{owner}.allowed_users has invalid user ID '{user_id}' — expected '@localpart:server'"
    )]
    InvalidAllowedUser {
        /// Config path of the list, e.g. `access_profiles.ops`.
        owner: String,
        /// The offending user ID.
        user_id: String,
    },

    /// A declared secret name is empty or uses characters outside `A-Z a-z 0-9 _ -`.
    #[error(
        "triggers.{command}.secrets has invalid name '{name}' — use only letters, digits, '_' and '-'"
    )]
    InvalidSecretName {
        /// Trigger command.
        command: String,
        /// The offending secret name.
        name: String,
    },

    /// A secret name is declared twice on one trigger.
    #[error("triggers.{command}.secrets declares '{name}' more than once")]
    DuplicateSecretName {
        /// Trigger command.
        command: String,
        /// The duplicated secret name.
        name: String,
    },

    /// A header value references `{{secret:NAME}}` for a secret the trigger does not declare.
    #[error(
        "triggers.{command}.headers.{header} references secret '{secret}' which is not listed in triggers.{command}.secrets"
    )]
    UndeclaredHeaderSecret {
        /// Trigger command.
        command: String,
        /// Header name.
        header: String,
        /// The undeclared secret name.
        secret: String,
    },

    /// A credential-bearing header (e.g. `Authorization`) has a literal value.
    #[error(
        "triggers.{command}.headers.{header} must use a {{{{secret:NAME}}}} placeholder; do not put credentials in the config file"
    )]
    CredentialHeaderWithoutSecret {
        /// Trigger command.
        command: String,
        /// Header name.
        header: String,
    },
}

/// Methods a trigger may use. Uppercase only, matching what the caller sends.
const TRIGGER_METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

/// The built-in command that lists triggers; triggers may not use this name.
const HELP_COMMAND: &str = "help";

/// Header names that conventionally carry credentials. Their values must come
/// from a `{{secret:NAME}}` placeholder so tokens are never committed.
const CREDENTIAL_HEADERS: [&str; 4] = [
    "authorization",
    "proxy-authorization",
    "cookie",
    "x-api-key",
];

/// Opening and closing markers of a secret placeholder in a header value.
const SECRET_OPEN: &str = "{{secret:";
const SECRET_CLOSE: &str = "}}";

/// Run all validation rules against `config`.
///
/// Returns the first error encountered, or `Ok(())` if all rules pass.
pub fn validate(config: &Config) -> Result<(), ValidationError> {
    validate_homeserver(&config.matrix.homeserver)?;

    for (name, dest) in &config.matrix.destinations {
        validate_room_id(name, &dest.room_id)?;
    }

    for (webhook_id, webhook) in &config.webhooks {
        if !config
            .matrix
            .destinations
            .contains_key(&webhook.destination)
        {
            return Err(ValidationError::UnknownDestination {
                webhook: webhook_id.clone(),
                destination: webhook.destination.clone(),
            });
        }

        if webhook.msg_template.trim().is_empty() {
            return Err(ValidationError::EmptyTemplate(webhook_id.clone()));
        }
    }

    for (name, profile) in &config.access_profiles {
        validate_allowlists(
            &format!("access_profiles.{name}"),
            &profile.allowed_rooms,
            &profile.allowed_users,
        )?;
    }

    validate_triggers(config)?;

    if config.server.port == 0 {
        return Err(ValidationError::InvalidPort(config.server.port));
    }

    Ok(())
}

fn validate_triggers(config: &Config) -> Result<(), ValidationError> {
    let mut seen = std::collections::HashSet::new();

    for (command, trigger) in &config.triggers {
        if command.is_empty() || command.chars().any(char::is_whitespace) {
            return Err(ValidationError::InvalidTriggerCommand(command.clone()));
        }
        let lowered = command.to_lowercase();
        if lowered == HELP_COMMAND {
            return Err(ValidationError::ReservedTriggerCommand(command.clone()));
        }
        if !seen.insert(lowered) {
            return Err(ValidationError::DuplicateTriggerCommand(command.clone()));
        }

        if !TRIGGER_METHODS.contains(&trigger.method.as_str()) {
            return Err(ValidationError::InvalidTriggerMethod {
                command: command.clone(),
                method: trigger.method.clone(),
            });
        }

        if !is_http_url_with_host(&trigger.url) {
            return Err(ValidationError::InvalidTriggerUrl {
                command: command.clone(),
                url: trigger.url.clone(),
            });
        }

        if let Some(profile) = &trigger.access_profile {
            if !config.access_profiles.contains_key(profile) {
                return Err(ValidationError::UnknownAccessProfile {
                    command: command.clone(),
                    profile: profile.clone(),
                });
            }
        }

        validate_allowlists(
            &format!("triggers.{command}"),
            &trigger.allowed_rooms,
            &trigger.allowed_users,
        )?;
        validate_secrets_and_headers(command, trigger)?;
    }

    Ok(())
}

fn validate_secrets_and_headers(
    command: &str,
    trigger: &botrix_common::config::TriggerConfig,
) -> Result<(), ValidationError> {
    let mut declared = std::collections::HashSet::new();
    for name in &trigger.secrets {
        if !is_valid_secret_name(name) {
            return Err(ValidationError::InvalidSecretName {
                command: command.to_owned(),
                name: name.clone(),
            });
        }
        if !declared.insert(name.as_str()) {
            return Err(ValidationError::DuplicateSecretName {
                command: command.to_owned(),
                name: name.clone(),
            });
        }
    }

    for (header, value) in &trigger.headers {
        let placeholders = secret_placeholders(value);
        if let Some(secret) = placeholders.iter().find(|s| !declared.contains(**s)) {
            return Err(ValidationError::UndeclaredHeaderSecret {
                command: command.to_owned(),
                header: header.clone(),
                secret: (*secret).to_owned(),
            });
        }
        let is_credential = CREDENTIAL_HEADERS.contains(&header.to_ascii_lowercase().as_str());
        if is_credential && placeholders.is_empty() {
            return Err(ValidationError::CredentialHeaderWithoutSecret {
                command: command.to_owned(),
                header: header.clone(),
            });
        }
    }

    Ok(())
}

fn validate_allowlists(
    owner: &str,
    rooms: &[String],
    users: &[String],
) -> Result<(), ValidationError> {
    if let Some(room_id) = rooms.iter().find(|r| !is_valid_room_id(r)) {
        return Err(ValidationError::InvalidAllowedRoom {
            owner: owner.to_owned(),
            room_id: room_id.clone(),
        });
    }
    if let Some(user_id) = users.iter().find(|u| !is_valid_user_id(u)) {
        return Err(ValidationError::InvalidAllowedUser {
            owner: owner.to_owned(),
            user_id: user_id.clone(),
        });
    }
    Ok(())
}

/// Names inside `{{secret:NAME}}` placeholders in `value`, in order.
///
/// Mirrors the scanning in `triggers::webhook::resolve_secrets`: an opening
/// marker without a closing `}}` ends the scan.
fn secret_placeholders(value: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.find(SECRET_OPEN) {
        let after_open = &rest[start + SECRET_OPEN.len()..];
        let Some(end) = after_open.find(SECRET_CLOSE) else {
            break;
        };
        names.push(&after_open[..end]);
        rest = &after_open[end + SECRET_CLOSE.len()..];
    }
    names
}

fn is_valid_secret_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn is_http_url_with_host(raw: &str) -> bool {
    url::Url::parse(raw)
        .map(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
        .unwrap_or(false)
}

fn is_valid_user_id(user_id: &str) -> bool {
    // `@localpart:server`, both parts non-empty, no whitespace or NUL.
    let Some(rest) = user_id.strip_prefix('@') else {
        return false;
    };
    let Some((local, server)) = rest.split_once(':') else {
        return false;
    };
    !local.is_empty()
        && !server.is_empty()
        && !user_id.chars().any(|c| c.is_whitespace() || c == '\0')
}

fn validate_homeserver(homeserver: &str) -> Result<(), ValidationError> {
    if homeserver.starts_with("https://") && homeserver.len() > "https://".len() {
        Ok(())
    } else {
        Err(ValidationError::InvalidHomeserver(homeserver.to_owned()))
    }
}

fn validate_room_id(name: &str, room_id: &str) -> Result<(), ValidationError> {
    // Match Matrix itself (ruma's `room_id::validate`): a room ID is the `!`
    // sigil followed by a non-empty opaque localpart. The `:server` suffix is
    // only present on v1 room IDs — newer rooms (format v2 / MSC4291) have
    // none — so it must NOT be required. Reject a lone sigil and NUL bytes.
    if is_valid_room_id(room_id) {
        Ok(())
    } else {
        Err(ValidationError::InvalidRoomId {
            name: name.to_owned(),
            room_id: room_id.to_owned(),
        })
    }
}

fn is_valid_room_id(room_id: &str) -> bool {
    room_id.starts_with('!') && room_id.len() > 1 && !room_id.contains('\0')
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use std::collections::BTreeMap;

    use botrix_common::config::{
        AccessProfileConfig, Config, DestinationConfig, MatrixConfig, ServerConfig, TriggerConfig,
        WebhookConfig,
    };

    use super::*;

    fn valid_config() -> Config {
        let mut destinations = HashMap::new();
        destinations.insert(
            "alerts".to_owned(),
            DestinationConfig {
                room_id: "!abc123:example.com".to_owned(),
                enabled: true,
            },
        );

        let mut webhooks = HashMap::new();
        webhooks.insert(
            "my_hook".to_owned(),
            WebhookConfig {
                enabled: true,
                destination: "alerts".to_owned(),
                msg_template: "Hello ${name}".to_owned(),
            },
        );

        Config {
            matrix: MatrixConfig {
                homeserver: "https://matrix.example.com".to_owned(),
                destinations,
            },
            webhooks,
            triggers: HashMap::new(),
            access_profiles: HashMap::new(),
            server: ServerConfig::default(),
        }
    }

    fn trigger() -> TriggerConfig {
        TriggerConfig {
            description: "Deploy the app".to_owned(),
            enabled: true,
            method: "POST".to_owned(),
            url: "https://ci.example.com/hook".to_owned(),
            headers: BTreeMap::new(),
            secrets: Vec::new(),
            response_template: None,
            access_profile: None,
            allowed_rooms: Vec::new(),
            allowed_users: Vec::new(),
        }
    }

    fn config_with_trigger(command: &str, trigger: TriggerConfig) -> Config {
        let mut cfg = valid_config();
        cfg.triggers.insert(command.to_owned(), trigger);
        cfg
    }

    #[test]
    fn valid_config_passes() {
        assert!(validate(&valid_config()).is_ok());
    }

    #[test]
    fn rejects_http_homeserver() {
        let mut cfg = valid_config();
        cfg.matrix.homeserver = "http://matrix.example.com".to_owned();
        assert!(matches!(
            validate(&cfg),
            Err(ValidationError::InvalidHomeserver(_))
        ));
    }

    #[test]
    fn rejects_empty_homeserver() {
        let mut cfg = valid_config();
        cfg.matrix.homeserver = String::new();
        assert!(matches!(
            validate(&cfg),
            Err(ValidationError::InvalidHomeserver(_))
        ));
    }

    #[test]
    fn rejects_bad_room_id_no_bang() {
        let mut cfg = valid_config();
        cfg.matrix.destinations.get_mut("alerts").unwrap().room_id =
            "abc123:example.com".to_owned();
        assert!(matches!(
            validate(&cfg),
            Err(ValidationError::InvalidRoomId { .. })
        ));
    }

    #[test]
    fn accepts_v1_room_id_with_server() {
        let mut cfg = valid_config();
        cfg.matrix.destinations.get_mut("alerts").unwrap().room_id =
            "!abc123:example.com".to_owned();
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn accepts_room_id_without_server_part() {
        // Current Matrix room IDs (format v2 / MSC4291) have no `:server`
        // suffix; ruma's own validation accepts them, so Botrix must too.
        let mut cfg = valid_config();
        cfg.matrix.destinations.get_mut("alerts").unwrap().room_id =
            "!Pmc2TacQ9099q3Gc5s-C8FDLrd1Sd7vyKPp7UgzYz1Q".to_owned();
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn rejects_empty_room_id() {
        let mut cfg = valid_config();
        cfg.matrix.destinations.get_mut("alerts").unwrap().room_id = String::new();
        assert!(matches!(
            validate(&cfg),
            Err(ValidationError::InvalidRoomId { .. })
        ));
    }

    #[test]
    fn rejects_room_id_that_is_only_the_sigil() {
        let mut cfg = valid_config();
        cfg.matrix.destinations.get_mut("alerts").unwrap().room_id = "!".to_owned();
        assert!(matches!(
            validate(&cfg),
            Err(ValidationError::InvalidRoomId { .. })
        ));
    }

    #[test]
    fn rejects_room_alias_used_as_room_id() {
        // A `#alias:server` is not a room ID; catch the common mistake.
        let mut cfg = valid_config();
        cfg.matrix.destinations.get_mut("alerts").unwrap().room_id =
            "#general:example.com".to_owned();
        assert!(matches!(
            validate(&cfg),
            Err(ValidationError::InvalidRoomId { .. })
        ));
    }

    #[test]
    fn rejects_unknown_destination() {
        let mut cfg = valid_config();
        cfg.webhooks.get_mut("my_hook").unwrap().destination = "nonexistent".to_owned();
        assert!(matches!(
            validate(&cfg),
            Err(ValidationError::UnknownDestination { .. })
        ));
    }

    #[test]
    fn rejects_empty_template() {
        let mut cfg = valid_config();
        cfg.webhooks.get_mut("my_hook").unwrap().msg_template = "   ".to_owned();
        assert!(matches!(
            validate(&cfg),
            Err(ValidationError::EmptyTemplate(_))
        ));
    }

    #[test]
    fn rejects_port_zero() {
        let mut cfg = valid_config();
        cfg.server.port = 0;
        assert!(matches!(
            validate(&cfg),
            Err(ValidationError::InvalidPort(0))
        ));
    }

    // ---- triggers ----

    #[test]
    fn accepts_a_fully_specified_trigger() {
        // Arrange
        let mut t = trigger();
        t.headers.insert(
            "Authorization".to_owned(),
            "Bearer {{secret:TOKEN}}".to_owned(),
        );
        t.headers.insert("X-Source".to_owned(), "botrix".to_owned());
        t.secrets = vec!["TOKEN".to_owned()];
        t.response_template = Some("${status}".to_owned());
        t.access_profile = Some("ops".to_owned());
        t.allowed_rooms = vec!["!ops:example.com".to_owned()];
        t.allowed_users = vec!["@alice:example.com".to_owned()];
        let mut cfg = config_with_trigger("deploy", t);
        cfg.access_profiles.insert(
            "ops".to_owned(),
            AccessProfileConfig {
                allowed_rooms: vec!["!ops:example.com".to_owned()],
                allowed_users: vec!["@bob:example.com".to_owned()],
            },
        );

        // Act
        let result = validate(&cfg);

        // Assert
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn rejects_empty_trigger_command() {
        let cfg = config_with_trigger("", trigger());
        assert!(matches!(
            validate(&cfg),
            Err(ValidationError::InvalidTriggerCommand(_))
        ));
    }

    #[test]
    fn rejects_trigger_command_with_whitespace() {
        let cfg = config_with_trigger("deploy prod", trigger());
        assert!(matches!(
            validate(&cfg),
            Err(ValidationError::InvalidTriggerCommand(_))
        ));
    }

    #[test]
    fn rejects_reserved_help_command_in_any_case() {
        for name in ["help", "HELP", "Help"] {
            let cfg = config_with_trigger(name, trigger());
            assert!(
                matches!(
                    validate(&cfg),
                    Err(ValidationError::ReservedTriggerCommand(_))
                ),
                "{name} should be reserved"
            );
        }
    }

    #[test]
    fn rejects_commands_that_collide_after_lowercasing() {
        // Arrange
        let mut cfg = config_with_trigger("Deploy", trigger());
        cfg.triggers.insert("deploy".to_owned(), trigger());

        // Act
        let result = validate(&cfg);

        // Assert
        assert!(matches!(
            result,
            Err(ValidationError::DuplicateTriggerCommand(_))
        ));
    }

    #[test]
    fn accepts_every_supported_method() {
        for method in ["GET", "POST", "PUT", "PATCH", "DELETE"] {
            let mut t = trigger();
            t.method = method.to_owned();
            assert!(validate(&config_with_trigger("run", t)).is_ok(), "{method}");
        }
    }

    #[test]
    fn rejects_unsupported_or_lowercase_method() {
        for method in ["", "post", "TRACE", "FETCH"] {
            let mut t = trigger();
            t.method = method.to_owned();
            assert!(
                matches!(
                    validate(&config_with_trigger("run", t)),
                    Err(ValidationError::InvalidTriggerMethod { .. })
                ),
                "{method:?} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_trigger_urls_that_are_not_http_or_https() {
        for url in [
            "",
            "not a url",
            "ftp://example.com/x",
            "file:///etc/passwd",
            "https://",
        ] {
            let mut t = trigger();
            t.url = url.to_owned();
            assert!(
                matches!(
                    validate(&config_with_trigger("run", t)),
                    Err(ValidationError::InvalidTriggerUrl { .. })
                ),
                "{url:?} should be rejected"
            );
        }
    }

    #[test]
    fn accepts_http_trigger_url() {
        let mut t = trigger();
        t.url = "http://ci.internal:8080/hook?x=1".to_owned();
        assert!(validate(&config_with_trigger("run", t)).is_ok());
    }

    #[test]
    fn rejects_unknown_access_profile() {
        let mut t = trigger();
        t.access_profile = Some("missing".to_owned());
        assert!(matches!(
            validate(&config_with_trigger("run", t)),
            Err(ValidationError::UnknownAccessProfile { .. })
        ));
    }

    #[test]
    fn rejects_malformed_inline_room_and_user_ids() {
        // Arrange
        let mut bad_room = trigger();
        bad_room.allowed_rooms = vec!["#alias:example.com".to_owned()];
        let mut bad_user = trigger();
        bad_user.allowed_users = vec!["alice".to_owned()];

        // Act / Assert
        assert!(matches!(
            validate(&config_with_trigger("run", bad_room)),
            Err(ValidationError::InvalidAllowedRoom { .. })
        ));
        assert!(matches!(
            validate(&config_with_trigger("run", bad_user)),
            Err(ValidationError::InvalidAllowedUser { .. })
        ));
    }

    #[test]
    fn rejects_malformed_ids_in_access_profiles() {
        for (rooms, users) in [
            (vec!["nope".to_owned()], vec![]),
            (vec![], vec!["@missing-server".to_owned()]),
            (vec![], vec!["@:example.com".to_owned()]),
        ] {
            let mut cfg = valid_config();
            cfg.access_profiles.insert(
                "ops".to_owned(),
                AccessProfileConfig {
                    allowed_rooms: rooms,
                    allowed_users: users,
                },
            );
            assert!(matches!(
                validate(&cfg),
                Err(ValidationError::InvalidAllowedRoom { .. })
                    | Err(ValidationError::InvalidAllowedUser { .. })
            ));
        }
    }

    #[test]
    fn rejects_invalid_secret_names() {
        for name in ["", "has space", "a}}b", "tok:en"] {
            let mut t = trigger();
            t.secrets = vec![name.to_owned()];
            assert!(
                matches!(
                    validate(&config_with_trigger("run", t)),
                    Err(ValidationError::InvalidSecretName { .. })
                ),
                "{name:?} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_duplicate_secret_names() {
        let mut t = trigger();
        t.secrets = vec!["TOKEN".to_owned(), "TOKEN".to_owned()];
        assert!(matches!(
            validate(&config_with_trigger("run", t)),
            Err(ValidationError::DuplicateSecretName { .. })
        ));
    }

    #[test]
    fn rejects_header_placeholder_for_undeclared_secret() {
        // Arrange
        let mut t = trigger();
        t.headers
            .insert("X-Token".to_owned(), "{{secret:TOKEN}}".to_owned());

        // Act
        let result = validate(&config_with_trigger("run", t));

        // Assert
        assert!(matches!(
            result,
            Err(ValidationError::UndeclaredHeaderSecret { .. })
        ));
    }

    #[test]
    fn accepts_header_placeholders_for_declared_secrets() {
        let mut t = trigger();
        t.secrets = vec!["A".to_owned(), "B_2".to_owned()];
        t.headers.insert(
            "X-Multi".to_owned(),
            "{{secret:A}}-{{secret:B_2}}".to_owned(),
        );
        assert!(validate(&config_with_trigger("run", t)).is_ok());
    }

    #[test]
    fn rejects_literal_value_in_credential_headers() {
        for name in [
            "Authorization",
            "authorization",
            "Proxy-Authorization",
            "Cookie",
            "X-Api-Key",
        ] {
            let mut t = trigger();
            t.headers
                .insert(name.to_owned(), "Bearer abc123".to_owned());
            assert!(
                matches!(
                    validate(&config_with_trigger("run", t)),
                    Err(ValidationError::CredentialHeaderWithoutSecret { .. })
                ),
                "{name} with a literal value should be rejected"
            );
        }
    }

    #[test]
    fn accepts_literal_value_in_non_credential_headers() {
        let mut t = trigger();
        t.headers
            .insert("Content-Type".to_owned(), "application/json".to_owned());
        assert!(validate(&config_with_trigger("run", t)).is_ok());
    }
}
