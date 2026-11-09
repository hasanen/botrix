//! Matrix event handler that turns room messages addressed to the bot into
//! command executions and replies.
//!
//! The decision of whether a message is a command lives in the pure
//! [`extract_command`]; [`register`] only wires it to the SDK.

use std::sync::Arc;

use matrix_sdk::ruma::events::room::message::{
    AddMentions, ForwardThread, OriginalSyncRoomMessageEvent, RoomMessageEventContent,
};
use matrix_sdk::ruma::{OwnedUserId, UserId};
use matrix_sdk::{Client, Room};

use crate::matrix::command::{parse_command, ParsedCommand};
use crate::triggers::handler::{handle_command, HandlerContext, IncomingMessage};

/// Returns the command in `body` if it was sent by someone other than the bot
/// and addresses `bot` (see [`parse_command`]).
pub fn extract_command(
    sender: &UserId,
    bot: &UserId,
    body: &str,
    mentioned_user_ids: &[OwnedUserId],
) -> Option<ParsedCommand> {
    if sender == bot {
        return None;
    }
    parse_command(body, mentioned_user_ids, bot)
}

/// Registers a handler on `client` that runs bot commands found in room
/// messages and replies to the original message with the result.
pub fn register(client: &Client, ctx: Arc<HandlerContext>) {
    client.add_event_handler(
        move |ev: OriginalSyncRoomMessageEvent, room: Room, client: Client| {
            let ctx = Arc::clone(&ctx);
            async move {
                let Some(bot) = client.user_id() else {
                    return;
                };
                let mentions: Vec<OwnedUserId> = ev
                    .content
                    .mentions
                    .as_ref()
                    .map(|m| m.user_ids.iter().cloned().collect())
                    .unwrap_or_default();
                let Some(parsed) = extract_command(&ev.sender, bot, ev.content.body(), &mentions)
                else {
                    return;
                };

                let msg = IncomingMessage {
                    raw_message: ev.content.body().to_owned(),
                    sender: ev.sender.to_string(),
                    room_id: room.room_id().to_string(),
                    event_id: ev.event_id.to_string(),
                };
                tokio::spawn(async move {
                    let reply = handle_command(&ctx, &parsed, &msg).await;
                    let content = RoomMessageEventContent::text_plain(reply).make_reply_to(
                        &ev,
                        ForwardThread::No,
                        AddMentions::Yes,
                    );
                    if let Err(e) = room.send(content).await {
                        tracing::warn!(error = %e, "failed to send command reply");
                    }
                });
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: &str) -> OwnedUserId {
        UserId::parse(id).unwrap()
    }

    #[test]
    fn own_message_is_ignored() {
        let bot = user("@botrix:example.org");

        let result = extract_command(
            &bot,
            &bot,
            "@botrix:example.org ping",
            std::slice::from_ref(&bot),
        );

        assert_eq!(result, None);
    }

    #[test]
    fn message_without_mention_is_ignored() {
        let bot = user("@botrix:example.org");
        let alice = user("@alice:example.org");

        let result = extract_command(&alice, &bot, "@botrix:example.org ping", &[]);

        assert_eq!(result, None);
    }

    #[test]
    fn mentioned_command_is_parsed() {
        let bot = user("@botrix:example.org");
        let alice = user("@alice:example.org");

        let result = extract_command(
            &alice,
            &bot,
            "@botrix:example.org Ping a b",
            std::slice::from_ref(&bot),
        );

        let parsed = result.expect("command should parse");
        assert_eq!(parsed.command, "ping");
        assert_eq!(parsed.args, vec!["a", "b"]);
    }
}
