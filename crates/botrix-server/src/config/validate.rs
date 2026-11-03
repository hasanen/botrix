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
        "destinations.{name}.room_id has invalid format '{room_id}' — expected '!<id>:<server>'"
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
}

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

    if config.server.port == 0 {
        return Err(ValidationError::InvalidPort(config.server.port));
    }

    Ok(())
}

fn validate_homeserver(homeserver: &str) -> Result<(), ValidationError> {
    if homeserver.starts_with("https://") && homeserver.len() > "https://".len() {
        Ok(())
    } else {
        Err(ValidationError::InvalidHomeserver(homeserver.to_owned()))
    }
}

fn validate_room_id(name: &str, room_id: &str) -> Result<(), ValidationError> {
    // Room IDs must match `!<localpart>:<server>` — starts with '!', contains ':'
    if room_id.starts_with('!') && room_id.contains(':') {
        Ok(())
    } else {
        Err(ValidationError::InvalidRoomId {
            name: name.to_owned(),
            room_id: room_id.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use botrix_common::config::{
        Config, DestinationConfig, MatrixConfig, ServerConfig, WebhookConfig,
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
            server: ServerConfig::default(),
        }
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
    fn rejects_bad_room_id_no_colon() {
        let mut cfg = valid_config();
        cfg.matrix.destinations.get_mut("alerts").unwrap().room_id = "!abc123".to_owned();
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
}
