//! Matrix session management.
//!
//! Implements OAuth Device Authorization Grant, session restore, token refresh,
//! E2EE bootstrapping, room invitation handling, and message delivery.
//!
//! Full implementation is Phase 6. This module provides the error type and
//! placeholder stubs used by other modules during earlier phases.

pub mod auth;
pub mod session;
pub mod worker;

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
    /// A database error occurred while persisting session state.
    #[error("database error: {0}")]
    Database(#[from] crate::db::DbError),
}
