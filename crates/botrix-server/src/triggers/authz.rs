//! Pure authorization helpers for command triggers.
//!
//! A non-empty inline allowlist on a trigger overrides the referenced access
//! profile's list for that dimension. Each dimension (room, user) passes when
//! its effective list is empty or contains the id; both must pass.

use crate::db::models::{AccessProfile, CommandTrigger};

/// Resolves the effective `(rooms, users)` allowlists for a trigger.
///
/// For each dimension, a non-empty inline list wins; otherwise the profile's
/// list is used, or empty when there is no profile.
pub fn effective_allowlists(
    trigger: &CommandTrigger,
    profile: Option<&AccessProfile>,
) -> (Vec<String>, Vec<String>) {
    fn pick(inline: &[String], from_profile: Option<&Vec<String>>) -> Vec<String> {
        if !inline.is_empty() {
            inline.to_vec()
        } else {
            from_profile.cloned().unwrap_or_default()
        }
    }
    (
        pick(&trigger.allowed_rooms, profile.map(|p| &p.allowed_rooms)),
        pick(&trigger.allowed_users, profile.map(|p| &p.allowed_users)),
    )
}

/// Returns whether `user_id` may run the trigger in `room_id`.
///
/// Room and user checks are ANDed; an empty effective list is unrestricted.
pub fn is_authorized(
    trigger: &CommandTrigger,
    profile: Option<&AccessProfile>,
    room_id: &str,
    user_id: &str,
) -> bool {
    let (rooms, users) = effective_allowlists(trigger, profile);
    let passes = |list: &[String], id: &str| list.is_empty() || list.iter().any(|x| x == id);
    passes(&rooms, room_id) && passes(&users, user_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trigger(rooms: &[&str], users: &[&str]) -> CommandTrigger {
        CommandTrigger {
            id: 1,
            command: "deploy".into(),
            description: String::new(),
            enabled: true,
            method: "POST".into(),
            url: "https://example.com".into(),
            response_template: None,
            access_profile_id: None,
            allowed_rooms: rooms.iter().map(|s| s.to_string()).collect(),
            allowed_users: users.iter().map(|s| s.to_string()).collect(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn profile(rooms: &[&str], users: &[&str]) -> AccessProfile {
        AccessProfile {
            id: 1,
            name: "p".into(),
            allowed_rooms: rooms.iter().map(|s| s.to_string()).collect(),
            allowed_users: users.iter().map(|s| s.to_string()).collect(),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn empty_empty_allows_anyone() {
        let t = trigger(&[], &[]);
        assert!(is_authorized(&t, None, "!r:x", "@u:x"));
    }

    #[test]
    fn room_only_restricts() {
        let t = trigger(&["!ok:x"], &[]);
        assert!(is_authorized(&t, None, "!ok:x", "@anyone:x"));
        assert!(!is_authorized(&t, None, "!other:x", "@anyone:x"));
    }

    #[test]
    fn user_only_restricts() {
        let t = trigger(&[], &["@ok:x"]);
        assert!(is_authorized(&t, None, "!any:x", "@ok:x"));
        assert!(!is_authorized(&t, None, "!any:x", "@bad:x"));
    }

    #[test]
    fn both_restrict_with_and_logic() {
        let t = trigger(&["!ok:x"], &["@ok:x"]);
        assert!(is_authorized(&t, None, "!ok:x", "@ok:x"));
        assert!(!is_authorized(&t, None, "!bad:x", "@ok:x"));
        assert!(!is_authorized(&t, None, "!ok:x", "@bad:x"));
        assert!(!is_authorized(&t, None, "!bad:x", "@bad:x"));
    }

    #[test]
    fn profile_used_when_inline_empty() {
        let t = trigger(&[], &[]);
        let p = profile(&["!p:x"], &["@p:x"]);
        assert_eq!(
            effective_allowlists(&t, Some(&p)),
            (vec!["!p:x".to_string()], vec!["@p:x".to_string()])
        );
        assert!(is_authorized(&t, Some(&p), "!p:x", "@p:x"));
        assert!(!is_authorized(&t, Some(&p), "!other:x", "@p:x"));
        assert!(!is_authorized(&t, Some(&p), "!p:x", "@other:x"));
    }

    #[test]
    fn inline_overrides_non_empty_profile_per_dimension() {
        let t = trigger(&["!inline:x"], &[]);
        let p = profile(&["!p:x"], &["@p:x"]);
        // Rooms come from inline, users fall back to the profile.
        assert_eq!(
            effective_allowlists(&t, Some(&p)),
            (vec!["!inline:x".to_string()], vec!["@p:x".to_string()])
        );
        assert!(is_authorized(&t, Some(&p), "!inline:x", "@p:x"));
        assert!(!is_authorized(&t, Some(&p), "!p:x", "@p:x"));
    }

    #[test]
    fn no_profile_and_empty_inline_yields_empty_lists() {
        let t = trigger(&[], &[]);
        assert_eq!(effective_allowlists(&t, None), (vec![], vec![]));
    }
}
