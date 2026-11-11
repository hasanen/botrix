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
use crate::triggers::handler::HandlerContext;

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
    /// Handle to the background sync task, aborted on disconnect.
    sync_task: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
    /// Logical destination name → room ID, from config.
    destinations: HashMap<String, String>,
    /// Dependencies for the bot-command event handler registered on each client.
    command_ctx: Arc<HandlerContext>,
}

impl MatrixHandle {
    /// Create an empty handle (disconnected).
    pub fn new(destinations: HashMap<String, String>, command_ctx: Arc<HandlerContext>) -> Self {
        Self {
            client: Arc::new(RwLock::new(None)),
            sender: Arc::new(SwappableSender::disconnected()),
            sync_task: Arc::new(RwLock::new(None)),
            destinations,
            command_ctx,
        }
    }

    /// Create a handle already holding a restored, connected client, and start
    /// its background sync loop.
    pub fn connected(
        client: Client,
        destinations: HashMap<String, String>,
        command_ctx: Arc<HandlerContext>,
    ) -> Self {
        let sender = SwappableSender::live(MatrixSender::new(client.clone(), destinations.clone()));
        // Register before sync starts so no early message is missed.
        super::command_handler::register(&client, command_ctx.clone());
        let sync_task = super::worker::spawn_sync(client.clone());
        Self {
            client: Arc::new(RwLock::new(Some(client))),
            sender: Arc::new(sender),
            sync_task: Arc::new(RwLock::new(Some(sync_task))),
            destinations,
            command_ctx,
        }
    }

    /// The sender to hand to the delivery worker and the test-message endpoint.
    pub fn sender(&self) -> Arc<dyn MessageSender> {
        self.sender.clone()
    }

    /// The shared command-handling dependencies (pool, passphrase, webhook caller).
    pub fn command_ctx(&self) -> &Arc<HandlerContext> {
        &self.command_ctx
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

    /// Mark the handle connected: route delivery through `client` and start its
    /// background sync loop (so E2EE keys and membership stay current).
    pub async fn activate(&self, client: Client) {
        self.sender
            .set_live(MatrixSender::new(client.clone(), self.destinations.clone()))
            .await;
        // Each login builds a fresh client, so this registers once per client.
        super::command_handler::register(&client, self.command_ctx.clone());
        let task = super::worker::spawn_sync(client.clone());
        if let Some(old) = self.sync_task.write().await.replace(task) {
            old.abort();
        }
        *self.client.write().await = Some(client);
    }

    /// Drop the client, stop the sync loop, and stop delivering (on disconnect).
    pub async fn deactivate(&self) {
        self.sender.clear().await;
        if let Some(task) = self.sync_task.write().await.take() {
            task.abort();
        }
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

    fn command_ctx() -> Arc<HandlerContext> {
        Arc::new(HandlerContext {
            pool: crate::db::test_pool(),
            config: Arc::new(botrix_common::config::Config {
                matrix: botrix_common::config::MatrixConfig {
                    homeserver: "https://matrix.example.com".to_owned(),
                    destinations: Default::default(),
                },
                webhooks: Default::default(),
                triggers: Default::default(),
                access_profiles: Default::default(),
                server: Default::default(),
            }),
            crypto_passphrase: Arc::from("test-passphrase"),
            caller: Arc::new(crate::triggers::webhook::FakeCaller::new()),
        })
    }

    fn destinations() -> HashMap<String, String> {
        HashMap::from([("containers".to_owned(), "!r:example.com".to_owned())])
    }

    #[tokio::test]
    async fn a_new_handle_is_disconnected_and_queues() {
        // Arrange
        let handle = MatrixHandle::new(destinations(), command_ctx());

        // Act + Assert
        assert!(!handle.is_connected().await);
        assert!(handle.client().await.is_none());
        let err = handle.sender().send("containers", "hi").await.unwrap_err();
        assert!(matches!(err, SendError::Transient(_)));
    }

    #[tokio::test]
    async fn deactivate_returns_to_queuing() {
        // Arrange: a disconnected handle (no client available in a unit test).
        let handle = MatrixHandle::new(destinations(), command_ctx());

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
        let handle = MatrixHandle::new(destinations(), command_ctx());
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
