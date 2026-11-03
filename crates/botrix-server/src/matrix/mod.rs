//! Matrix session management.
//!
//! Implements OAuth Device Authorization Grant, session restore, token refresh,
//! E2EE bootstrapping, room invitation handling, and message delivery.
//!
//! Full implementation is Phase 6. This module provides the error type and
//! placeholder stubs used by other modules during earlier phases.

pub mod auth;
pub mod client;
pub mod crypto;
pub mod sender;
pub mod session;
pub mod worker;

pub use sender::MatrixSender;

use async_trait::async_trait;
use thiserror::Error;

use crate::queue::sender::{MessageSender, SendError};

/// A placeholder [`MessageSender`] used until the real Matrix client exists.
///
/// Every send fails transiently, so notifications stay queued and are retried
/// rather than lost or marked delivered. Phase 6 replaces this with a sender
/// backed by the live `matrix-sdk` client.
#[derive(Debug, Default)]
pub struct StubSender;

#[async_trait]
impl MessageSender for StubSender {
    async fn send(&self, destination: &str, _message: &str) -> Result<(), SendError> {
        Err(SendError::Transient(format!(
            "Matrix client not connected; cannot deliver to '{destination}'"
        )))
    }
}

/// Errors that can occur in the Matrix subsystem.
#[derive(Debug, Error)]
pub enum MatrixError {
    /// The Matrix SDK returned an error.
    #[error("matrix SDK error: {0}")]
    Sdk(String),
    /// No active Matrix session is available.
    #[error("no active Matrix session")]
    NoSession,
    /// The OAuth Device Authorization Grant flow is not in progress.
    #[error("no OAuth flow in progress")]
    NoOAuthFlow,
    /// The OAuth device code expired before the user authorized.
    #[error("OAuth device code expired")]
    OAuthExpired,
    /// E2EE has not been bootstrapped; a recovery key is required.
    #[error("E2EE not bootstrapped — submit the recovery key")]
    E2eeNotBootstrapped,
    /// The recovery key was rejected by the server.
    #[error("invalid recovery key")]
    InvalidRecoveryKey,
    /// Encrypting an OAuth token for storage failed.
    #[error("token encryption failed: {0}")]
    TokenEncryption(String),
    /// Decrypting a stored OAuth token failed (bad passphrase, tampering, or
    /// corruption).
    #[error("token decryption failed: {0}")]
    TokenDecryption(String),
    /// A destination name was not found in the configuration.
    #[error("unknown destination '{0}'")]
    UnknownDestination(String),
    /// A configured room ID could not be parsed.
    #[error("invalid room ID '{room_id}': {reason}")]
    InvalidRoomId {
        /// The offending room ID.
        room_id: String,
        /// Why parsing failed.
        reason: String,
    },
    /// Building or opening the Matrix client/store failed.
    #[error("failed to build Matrix client: {0}")]
    ClientBuild(String),
    /// A database error occurred while persisting session state.
    #[error("database error: {0}")]
    Database(#[from] crate::db::DbError),
}
