//! The message-delivery boundary between the queue and the Matrix subsystem.
//!
//! The worker depends only on [`MessageSender`], not on `matrix-sdk`. Phase 6
//! provides the real implementation backed by the Matrix client; tests use an
//! in-memory fake. This keeps the retry/state-machine logic fully testable
//! without a live Matrix connection.

use async_trait::async_trait;
use thiserror::Error;

/// Error returned when a message could not be delivered.
#[derive(Debug, Error)]
pub enum SendError {
    /// Delivery failed but may succeed on retry (network, rate limit, etc.).
    #[error("transient delivery failure: {0}")]
    Transient(String),
    /// Delivery failed permanently and must not be retried (e.g. the
    /// destination room does not exist in config).
    #[error("permanent delivery failure: {0}")]
    Permanent(String),
}

/// Delivers a rendered message to a logical destination.
#[async_trait]
pub trait MessageSender: Send + Sync {
    /// Send `message` to the room identified by the logical `destination`
    /// name (as defined in `matrix.destinations`).
    async fn send(&self, destination: &str, message: &str) -> Result<(), SendError>;
}
