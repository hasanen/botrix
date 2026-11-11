//! Looks up triggers and access profiles in the loaded configuration.

use botrix_common::config::{AccessProfileConfig, Config, TriggerConfig};

/// Finds the trigger for `command`, matching case-insensitively.
///
/// Returns the lowercase command (the key used for stored secrets) together
/// with the trigger. Validation guarantees that no two config keys collide
/// after lowercasing, so at most one trigger matches.
pub fn find_trigger<'a>(config: &'a Config, command: &str) -> Option<(String, &'a TriggerConfig)> {
    let wanted = command.to_lowercase();
    config
        .triggers
        .iter()
        .find(|(key, _)| key.to_lowercase() == wanted)
        .map(|(_, trigger)| (wanted, trigger))
}

/// The access profile a trigger references, if it references one that exists.
pub fn profile_for<'a>(
    config: &'a Config,
    trigger: &TriggerConfig,
) -> Option<&'a AccessProfileConfig> {
    trigger
        .access_profile
        .as_ref()
        .and_then(|name| config.access_profiles.get(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use botrix_common::config::MatrixConfig;

    fn trigger(profile: Option<&str>) -> TriggerConfig {
        TriggerConfig {
            description: String::new(),
            enabled: true,
            method: "POST".into(),
            url: "https://example.com".into(),
            headers: Default::default(),
            secrets: Vec::new(),
            response_template: None,
            access_profile: profile.map(String::from),
            allowed_rooms: Vec::new(),
            allowed_users: Vec::new(),
        }
    }

    fn config() -> Config {
        let mut triggers = std::collections::HashMap::new();
        triggers.insert("Deploy".to_owned(), trigger(Some("ops")));
        triggers.insert("build".to_owned(), trigger(None));
        let mut access_profiles = std::collections::HashMap::new();
        access_profiles.insert(
            "ops".to_owned(),
            AccessProfileConfig {
                allowed_rooms: vec!["!ops:x".to_owned()],
                allowed_users: Vec::new(),
            },
        );
        Config {
            matrix: MatrixConfig {
                homeserver: "https://matrix.example.com".to_owned(),
                destinations: Default::default(),
            },
            webhooks: Default::default(),
            triggers,
            access_profiles,
            server: Default::default(),
        }
    }

    #[test]
    fn find_trigger_matches_case_insensitively_and_returns_lowercase_key() {
        // Arrange
        let cfg = config();

        // Act
        let found = find_trigger(&cfg, "dEpLoY");

        // Assert
        let (key, t) = found.unwrap();
        assert_eq!(key, "deploy");
        assert_eq!(t.access_profile.as_deref(), Some("ops"));
    }

    #[test]
    fn find_trigger_returns_none_for_unknown_command() {
        assert!(find_trigger(&config(), "nope").is_none());
    }

    #[test]
    fn profile_for_resolves_the_referenced_profile() {
        // Arrange
        let cfg = config();
        let (_, with_profile) = find_trigger(&cfg, "deploy").unwrap();
        let (_, without) = find_trigger(&cfg, "build").unwrap();

        // Act / Assert
        let profile = profile_for(&cfg, with_profile).unwrap();
        assert_eq!(profile.allowed_rooms, ["!ops:x"]);
        assert!(profile_for(&cfg, without).is_none());
    }
}
