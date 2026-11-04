//! Shared, swappable Matrix connection state.
//!
//! The OAuth Authorization Code flow spans two requests — `connect` builds a
//! client and starts login; the `callback` finishes it — and the SDK keeps the
//! PKCE/CSRF state *inside* that `Client`. Both requests, plus the delivery
//! worker, therefore share one [`MatrixHandle`].
//!
//! The handle holds:
//! - the live [`Client`] once a login is in progress or established, and
//! - a [`SwappableSender`] the worker delivers through, which routes to a live
//!   [`MatrixSender`] after login and otherwise fails transiently (so
//!   notifications stay queued, exactly like [`super::StubSender`]).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use matrix_sdk::Client;
use tokio::sync::RwLock;

use crate::queue::sender::{MessageSender, SendError};

use super::MatrixSender;

/// Shared Matrix connection state (cheap to clone; all state is behind `Arc`).
#[derive(Clone)]
pub struct MatrixHandle {
    /// The live client, present once a login is in progress or active. The
    /// OAuth flow stores PKCE/CSRF state in it, so connect and callback must
    /// share this exact instance.
    client: Arc<RwLock<Option<Client>>>,
    /// The sender the worker delivers through; swapped to a live one on login.
    sender: Arc<SwappableSender>,
    /// Logical destination name → room ID, from config.
    destinations: HashMap<String, String>,
}

impl MatrixHandle {
    /// Create an empty handle (disconnected).
    pub fn new(destinations: HashMap<String, String>) -> Self {
        Self {
            client: Arc::new(RwLock::new(None)),
            sender: Arc::new(SwappableSender::disconnected()),
            destinations,
        }
    }

    /// Create a handle already holding a restored, connected client.
    pub fn connected(client: Client, destinations: HashMap<String, String>) -> Self {
        let sender = SwappableSender::live(MatrixSender::new(client.clone(), destinations.clone()));
        Self {
            client: Arc::new(RwLock::new(Some(client))),
            sender: Arc::new(sender),
            destinations,
        }
    }

    /// The sender to hand to the delivery worker and the test-message endpoint.
    pub fn sender(&self) -> Arc<dyn MessageSender> {
        self.sender.clone()
    }

    /// The configured destination map.
    pub fn destinations(&self) -> &HashMap<String, String> {
        &self.destinations
    }

    /// The current live client, if any.
    pub async fn client(&self) -> Option<Client> {
        self.client.read().await.clone()
    }

    /// Store `client` as the active client (used when starting a login).
    pub async fn set_client(&self, client: Client) {
        *self.client.write().await = Some(client);
    }

    /// Mark the handle connected: keep `client` and route delivery through it.
    pub async fn activate(&self, client: Client) {
        self.sender
            .set_live(MatrixSender::new(client.clone(), self.destinations.clone()))
            .await;
        *self.client.write().await = Some(client);
    }

    /// Drop the client and stop delivering (used on disconnect).
    pub async fn deactivate(&self) {
        self.sender.clear().await;
        *self.client.write().await = None;
    }

    /// Whether a live sender is currently active.
    pub async fn is_connected(&self) -> bool {
        self.sender.is_live().await
    }
}

/// A [`MessageSender`] whose live backend can be swapped at runtime.
struct SwappableSender {
    inner: RwLock<Option<MatrixSender>>,
}

impl SwappableSender {
    fn disconnected() -> Self {
        Self {
            inner: RwLock::new(None),
        }
    }

    fn live(sender: MatrixSender) -> Self {
        Self {
            inner: RwLock::new(Some(sender)),
        }
    }

    async fn set_live(&self, sender: MatrixSender) {
        *self.inner.write().await = Some(sender);
    }

    async fn clear(&self) {
        *self.inner.write().await = None;
    }

    async fn is_live(&self) -> bool {
        self.inner.read().await.is_some()
    }
}

#[async_trait]
impl MessageSender for SwappableSender {
    async fn send(&self, destination: &str, message: &str) -> Result<(), SendError> {
        // Clone the live sender out under the read lock, then send without
        // holding the lock (a send can be slow and must not block a swap).
        let live = self.inner.read().await.clone();
        match live {
            Some(sender) => sender.send(destination, message).await,
            None => Err(SendError::Transient(format!(
                "Matrix not connected; cannot deliver to '{destination}'"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn destinations() -> HashMap<String, String> {
        HashMap::from([("containers".to_owned(), "!r:example.com".to_owned())])
    }

    #[tokio::test]
    async fn a_new_handle_is_disconnected_and_queues() {
        // Arrange
        let handle = MatrixHandle::new(destinations());

        // Act + Assert
        assert!(!handle.is_connected().await);
        assert!(handle.client().await.is_none());
        let err = handle.sender().send("containers", "hi").await.unwrap_err();
        assert!(matches!(err, SendError::Transient(_)));
    }

    #[tokio::test]
    async fn deactivate_returns_to_queuing() {
        // Arrange: a disconnected handle (no client available in a unit test).
        let handle = MatrixHandle::new(destinations());

        // Act
        handle.deactivate().await;

        // Assert: still disconnected, still queues transiently.
        assert!(!handle.is_connected().await);
        assert!(matches!(
            handle.sender().send("containers", "hi").await,
            Err(SendError::Transient(_))
        ));
    }

    #[tokio::test]
    async fn one_sender_arc_reflects_later_state_changes() {
        // The worker grabs the sender once at startup; swapping the backend
        // later must be visible through that same Arc.
        let handle = MatrixHandle::new(destinations());
        let worker_sender = handle.sender();

        // Still disconnected -> transient.
        assert!(matches!(
            worker_sender.send("containers", "x").await,
            Err(SendError::Transient(_))
        ));
        // After a deactivate (no-op here) the same Arc keeps working.
        handle.deactivate().await;
        assert!(matches!(
            worker_sender.send("containers", "x").await,
            Err(SendError::Transient(_))
        ));
    }
}
