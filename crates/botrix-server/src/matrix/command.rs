//! Pure parsing of bot commands from Matrix message bodies.
//!
//! A message is a command only when the bot is listed in `m.mentions` AND the
//! body starts with a mention of the bot (full MXID, or the localpart with an
//! optional `@` prefix), optionally followed by `:` or `,`. No Matrix I/O.

use matrix_sdk::ruma::{OwnedUserId, UserId};

/// A command parsed from a message addressed to the bot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    /// The first word after the mention, lowercased.
    pub command: String,
    /// The raw remainder after the command word, trimmed.
    pub arguments: String,
    /// The remainder split on whitespace.
    pub args: Vec<String>,
}

/// Parses `body` as a command addressed to `bot_mxid`.
///
/// Returns `None` unless `bot_mxid` is in `mentioned_user_ids`, the body starts
/// (after leading whitespace) with a mention of the bot, and a command word
/// follows the mention.
pub fn parse_command(
    body: &str,
    mentioned_user_ids: &[OwnedUserId],
    bot_mxid: &UserId,
) -> Option<ParsedCommand> {
    if !mentioned_user_ids.iter().any(|id| id == bot_mxid) {
        return None;
    }

    let rest = strip_bot_mention(body.trim_start(), bot_mxid)?;
    let rest = rest.strip_prefix([':', ',']).unwrap_or(rest).trim_start();

    let (word, remainder) = match rest.split_once(char::is_whitespace) {
        Some((word, remainder)) => (word, remainder),
        None => (rest, ""),
    };
    if word.is_empty() {
        return None;
    }

    let arguments = remainder.trim();
    Some(ParsedCommand {
        command: word.to_lowercase(),
        arguments: arguments.to_owned(),
        args: arguments.split_whitespace().map(str::to_owned).collect(),
    })
}

/// Strips a leading mention of the bot (full MXID, `@localpart`, or
/// `localpart`, ASCII case-insensitive) and returns the text after it.
///
/// The mention must end at a word boundary: whitespace, `:`, `,` or end of
/// input, so `botrixer` is not a mention of `botrix`.
fn strip_bot_mention<'a>(text: &'a str, bot_mxid: &UserId) -> Option<&'a str> {
    let localpart = bot_mxid.localpart();
    let candidates = [
        bot_mxid.as_str().to_owned(),
        format!("@{localpart}"),
        localpart.to_owned(),
    ];

    candidates.iter().find_map(|candidate| {
        let head = text.get(..candidate.len())?;
        if !head.eq_ignore_ascii_case(candidate) {
            return None;
        }
        let rest = &text[candidate.len()..];
        let at_boundary = rest
            .chars()
            .next()
            .is_none_or(|c| c.is_whitespace() || c == ':' || c == ',');
        at_boundary.then_some(rest)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bot() -> OwnedUserId {
        UserId::parse("@botrix:example.org").unwrap()
    }

    fn parse(body: &str) -> Option<ParsedCommand> {
        let bot = bot();
        parse_command(body, std::slice::from_ref(&bot), &bot)
    }

    #[test]
    fn full_mxid_mention_parses() {
        let parsed = parse("@botrix:example.org deploy prod").unwrap();
        assert_eq!(parsed.command, "deploy");
        assert_eq!(parsed.arguments, "prod");
    }

    #[test]
    fn localpart_mentions_parse_with_separators() {
        for body in [
            "botrix deploy",
            "@botrix deploy",
            "@botrix: deploy",
            "botrix, deploy",
        ] {
            let parsed = parse(body).unwrap_or_else(|| panic!("{body} should parse"));
            assert_eq!(parsed.command, "deploy", "{body}");
        }
    }

    #[test]
    fn leading_whitespace_is_allowed() {
        assert_eq!(parse("  \n @botrix help").unwrap().command, "help");
    }

    #[test]
    fn missing_m_mentions_entry_returns_none() {
        let bot = bot();
        assert_eq!(parse_command("@botrix deploy", &[], &bot), None);
        let other = UserId::parse("@alice:example.org").unwrap();
        assert_eq!(parse_command("@botrix deploy", &[other], &bot), None);
    }

    #[test]
    fn mention_not_at_start_returns_none() {
        assert_eq!(parse("hey @botrix deploy"), None);
    }

    #[test]
    fn longer_word_sharing_prefix_is_not_a_mention() {
        assert_eq!(parse("botrixer deploy"), None);
    }

    #[test]
    fn command_is_case_insensitive() {
        assert_eq!(parse("@botrix DePLoy").unwrap().command, "deploy");
    }

    #[test]
    fn arguments_preserve_case_and_inner_spacing() {
        let parsed = parse("@botrix deploy   Prod   EU-West  ").unwrap();
        assert_eq!(parsed.arguments, "Prod   EU-West");
    }

    #[test]
    fn args_split_on_whitespace() {
        let parsed = parse("@botrix deploy Prod   EU-West").unwrap();
        assert_eq!(parsed.args, vec!["Prod", "EU-West"]);
    }

    #[test]
    fn bare_help_has_no_arguments() {
        let parsed = parse("@botrix help").unwrap();
        assert_eq!(parsed.command, "help");
        assert_eq!(parsed.arguments, "");
        assert!(parsed.args.is_empty());
    }

    #[test]
    fn mention_without_command_returns_none() {
        assert_eq!(parse("@botrix"), None);
        assert_eq!(parse("@botrix:   "), None);
        assert_eq!(parse("@botrix:example.org,"), None);
    }
}
