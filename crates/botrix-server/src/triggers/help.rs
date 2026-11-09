//! Built-in `help` command: lists the triggers a user may run in a room.

use crate::db::models::{AccessProfile, CommandTrigger};

use super::authz::is_authorized;

/// Header line of a non-empty help reply.
const HEADER: &str = "Available commands:";
/// Reply when the user can run no commands in the room.
const NONE_VISIBLE: &str = "No commands available.";

/// Builds the help text for `user_id` in `room_id`.
///
/// Only enabled triggers that pass [`is_authorized`] are listed, sorted by
/// command, one `command - description` per line (the ` - description` part is
/// omitted when the description is empty).
pub fn build_help(
    entries: &[(CommandTrigger, Option<AccessProfile>)],
    room_id: &str,
    user_id: &str,
) -> String {
    let mut visible: Vec<&CommandTrigger> = entries
        .iter()
        .filter(|(t, p)| t.enabled && is_authorized(t, p.as_ref(), room_id, user_id))
        .map(|(t, _)| t)
        .collect();
    if visible.is_empty() {
        return NONE_VISIBLE.to_string();
    }
    visible.sort_by(|a, b| a.command.cmp(&b.command));

    let mut out = String::from(HEADER);
    for t in visible {
        out.push('\n');
        out.push_str(&t.command);
        if !t.description.is_empty() {
            out.push_str(" - ");
            out.push_str(&t.description);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        command: &str,
        description: &str,
        enabled: bool,
        rooms: &[&str],
        users: &[&str],
    ) -> (CommandTrigger, Option<AccessProfile>) {
        (
            CommandTrigger {
                id: 1,
                command: command.into(),
                description: description.into(),
                enabled,
                method: "POST".into(),
                url: "https://example.com".into(),
                response_template: None,
                access_profile_id: None,
                allowed_rooms: rooms.iter().map(|s| s.to_string()).collect(),
                allowed_users: users.iter().map(|s| s.to_string()).collect(),
                created_at: 0,
                updated_at: 0,
            },
            None,
        )
    }

    #[test]
    fn disabled_trigger_is_hidden() {
        let entries = vec![
            entry("on", "Enabled", true, &[], &[]),
            entry("off", "Disabled", false, &[], &[]),
        ];
        let out = build_help(&entries, "!r:x", "@u:x");
        assert_eq!(out, "Available commands:\non - Enabled");
    }

    #[test]
    fn unauthorized_room_is_hidden() {
        let entries = vec![
            entry("a", "A", true, &["!ok:x"], &[]),
            entry("b", "B", true, &["!other:x"], &[]),
        ];
        let out = build_help(&entries, "!ok:x", "@u:x");
        assert_eq!(out, "Available commands:\na - A");
    }

    #[test]
    fn unauthorized_user_is_hidden() {
        let entries = vec![
            entry("a", "A", true, &[], &["@ok:x"]),
            entry("b", "B", true, &[], &["@other:x"]),
        ];
        let out = build_help(&entries, "!r:x", "@ok:x");
        assert_eq!(out, "Available commands:\na - A");
    }

    #[test]
    fn output_is_sorted_by_command() {
        let entries = vec![
            entry("zeta", "Z", true, &[], &[]),
            entry("alpha", "A", true, &[], &[]),
            entry("mid", "M", true, &[], &[]),
        ];
        let out = build_help(&entries, "!r:x", "@u:x");
        assert_eq!(out, "Available commands:\nalpha - A\nmid - M\nzeta - Z");
    }

    #[test]
    fn empty_description_omits_separator() {
        let entries = vec![entry("bare", "", true, &[], &[])];
        let out = build_help(&entries, "!r:x", "@u:x");
        assert_eq!(out, "Available commands:\nbare");
    }

    #[test]
    fn none_visible_returns_message() {
        let entries = vec![
            entry("off", "Disabled", false, &[], &[]),
            entry("locked", "Locked", true, &["!other:x"], &[]),
        ];
        assert_eq!(
            build_help(&entries, "!r:x", "@u:x"),
            "No commands available."
        );
        assert_eq!(build_help(&[], "!r:x", "@u:x"), "No commands available.");
    }
}
