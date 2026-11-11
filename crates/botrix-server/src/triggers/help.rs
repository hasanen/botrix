//! Built-in `help` command: lists the triggers a user may run in a room.

use botrix_common::config::Config;

use super::authz::is_authorized;
use super::lookup::profile_for;

/// Header line of a non-empty help reply.
const HEADER: &str = "Available commands:";
/// Reply when the user can run no commands in the room.
const NONE_VISIBLE: &str = "No commands available.";

/// Builds the help text for `user_id` in `room_id`.
///
/// Only enabled triggers that pass [`is_authorized`] are listed, sorted by
/// lowercase command, one `command - description` per line (the
/// ` - description` part is omitted when the description is empty).
pub fn build_help(config: &Config, room_id: &str, user_id: &str) -> String {
    let mut visible: Vec<(String, &str)> = config
        .triggers
        .iter()
        .filter(|(_, t)| t.enabled && is_authorized(t, profile_for(config, t), room_id, user_id))
        .map(|(command, t)| (command.to_lowercase(), t.description.as_str()))
        .collect();
    if visible.is_empty() {
        return NONE_VISIBLE.to_string();
    }
    visible.sort();

    let mut out = String::from(HEADER);
    for (command, description) in visible {
        out.push('\n');
        out.push_str(&command);
        if !description.is_empty() {
            out.push_str(" - ");
            out.push_str(description);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use botrix_common::config::{AccessProfileConfig, MatrixConfig, TriggerConfig};

    use super::*;

    fn trigger(description: &str, enabled: bool, rooms: &[&str], users: &[&str]) -> TriggerConfig {
        TriggerConfig {
            description: description.into(),
            enabled,
            method: "POST".into(),
            url: "https://example.com".into(),
            headers: Default::default(),
            secrets: Vec::new(),
            response_template: None,
            access_profile: None,
            allowed_rooms: rooms.iter().map(|s| s.to_string()).collect(),
            allowed_users: users.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn config(entries: Vec<(&str, TriggerConfig)>) -> Config {
        Config {
            matrix: MatrixConfig {
                homeserver: "https://matrix.example.com".to_owned(),
                destinations: Default::default(),
            },
            webhooks: Default::default(),
            triggers: entries
                .into_iter()
                .map(|(name, t)| (name.to_owned(), t))
                .collect(),
            access_profiles: HashMap::new(),
            server: Default::default(),
        }
    }

    #[test]
    fn disabled_trigger_is_hidden() {
        let cfg = config(vec![
            ("on", trigger("Enabled", true, &[], &[])),
            ("off", trigger("Disabled", false, &[], &[])),
        ]);
        let out = build_help(&cfg, "!r:x", "@u:x");
        assert_eq!(out, "Available commands:\non - Enabled");
    }

    #[test]
    fn unauthorized_room_is_hidden() {
        let cfg = config(vec![
            ("a", trigger("A", true, &["!ok:x"], &[])),
            ("b", trigger("B", true, &["!other:x"], &[])),
        ]);
        let out = build_help(&cfg, "!ok:x", "@u:x");
        assert_eq!(out, "Available commands:\na - A");
    }

    #[test]
    fn unauthorized_user_is_hidden() {
        let cfg = config(vec![
            ("a", trigger("A", true, &[], &["@ok:x"])),
            ("b", trigger("B", true, &[], &["@other:x"])),
        ]);
        let out = build_help(&cfg, "!r:x", "@ok:x");
        assert_eq!(out, "Available commands:\na - A");
    }

    #[test]
    fn access_profile_restrictions_apply() {
        // Arrange
        let mut cfg = config(vec![("a", trigger("A", true, &[], &[]))]);
        cfg.triggers.get_mut("a").unwrap().access_profile = Some("ops".into());
        cfg.access_profiles.insert(
            "ops".into(),
            AccessProfileConfig {
                allowed_rooms: vec!["!ops:x".into()],
                allowed_users: vec![],
            },
        );

        // Act / Assert
        assert_eq!(
            build_help(&cfg, "!ops:x", "@u:x"),
            "Available commands:\na - A"
        );
        assert_eq!(
            build_help(&cfg, "!other:x", "@u:x"),
            "No commands available."
        );
    }

    #[test]
    fn output_is_sorted_by_command() {
        let cfg = config(vec![
            ("zeta", trigger("Z", true, &[], &[])),
            ("alpha", trigger("A", true, &[], &[])),
            ("mid", trigger("M", true, &[], &[])),
        ]);
        let out = build_help(&cfg, "!r:x", "@u:x");
        assert_eq!(out, "Available commands:\nalpha - A\nmid - M\nzeta - Z");
    }

    #[test]
    fn commands_are_shown_lowercase() {
        let cfg = config(vec![("Deploy", trigger("D", true, &[], &[]))]);
        assert_eq!(
            build_help(&cfg, "!r:x", "@u:x"),
            "Available commands:\ndeploy - D"
        );
    }

    #[test]
    fn empty_description_omits_separator() {
        let cfg = config(vec![("bare", trigger("", true, &[], &[]))]);
        let out = build_help(&cfg, "!r:x", "@u:x");
        assert_eq!(out, "Available commands:\nbare");
    }

    #[test]
    fn none_visible_returns_message() {
        let cfg = config(vec![
            ("off", trigger("Disabled", false, &[], &[])),
            ("locked", trigger("Locked", true, &["!other:x"], &[])),
        ]);
        assert_eq!(build_help(&cfg, "!r:x", "@u:x"), "No commands available.");
        assert_eq!(
            build_help(&config(vec![]), "!r:x", "@u:x"),
            "No commands available."
        );
    }
}
