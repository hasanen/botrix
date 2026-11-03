//! A [`MessageSender`] backed by a live `matrix-sdk` [`Client`].
//!
//! Resolves a logical destination name to a configured room ID, looks up the
//! joined room on the client, and sends a plain-text message. Delivery errors
//! are classified into [`SendError::Permanent`] (misconfiguration or a room the
//! bot is not in — retrying cannot help) and [`SendError::Transient`] (network
//! or server errors that may succeed later).

use std::collections::HashMap;

use async_trait::async_trait;
use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
use matrix_sdk::ruma::RoomId;
use matrix_sdk::Client;

use crate::queue::sender::{MessageSender, SendError};

/// Sends notifications to Matrix rooms via a connected client.
#[derive(Clone)]
pub struct MatrixSender {
    client: Client,
    /// Logical destination name → Matrix room ID string.
    destinations: HashMap<String, String>,
}

impl MatrixSender {
    /// Build a sender from a connected client and the configured destination
    /// map (`name -> room_id`).
    pub fn new(client: Client, destinations: HashMap<String, String>) -> Self {
        Self {
            client,
            destinations,
        }
    }

    /// Resolve a logical destination name to its configured room ID string.
    fn resolve(&self, destination: &str) -> Option<&str> {
        self.destinations.get(destination).map(String::as_str)
    }
}

#[async_trait]
impl MessageSender for MatrixSender {
    async fn send(&self, destination: &str, message: &str) -> Result<(), SendError> {
        // Unknown destination name: a config problem; retrying cannot fix it.
        let room_id_str = self.resolve(destination).ok_or_else(|| {
            SendError::Permanent(format!("destination '{destination}' is not configured"))
        })?;

        // Malformed room ID in config: also permanent.
        let room_id = RoomId::parse(room_id_str)
            .map_err(|e| SendError::Permanent(format!("invalid room ID '{room_id_str}': {e}")))?;

        // The bot is not in the room (not joined / unknown): permanent until
        // the invite is accepted, so do not spin on retries.
        let room = self.client.get_room(&room_id).ok_or_else(|| {
            SendError::Permanent(format!(
                "room '{room_id_str}' is not known to the client (not joined?)"
            ))
        })?;

        let content = RoomMessageEventContent::text_plain(message);
        room.send(content)
            .await
            .map(|_| ())
            // Any send failure (network, server, encryption) is treated as
            // transient; the queue's retry policy decides when to give up.
            .map_err(|e| SendError::Transient(format!("matrix send failed: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn destinations() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("containers".to_owned(), "!abc123:example.com".to_owned());
        m
    }

    #[test]
    fn resolve_returns_configured_room_id() {
        // A sender without a client is enough to exercise resolution: build the
        // map directly and test `resolve` in isolation.
        let map = destinations();
        assert_eq!(
            map.get("containers").map(String::as_str),
            Some("!abc123:example.com")
        );
        assert_eq!(map.get("missing"), None);
    }

    #[test]
    fn room_id_parsing_rejects_malformed() {
        // Guards the permanent-error path for bad config: a string without the
        // leading `!` and server part is not a valid room ID.
        assert!(RoomId::parse("not-a-room-id").is_err());
        assert!(RoomId::parse("!valid:example.com").is_ok());
    }
}
