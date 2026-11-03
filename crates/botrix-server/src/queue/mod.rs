//! Background notification delivery worker.
//!
//! Picks up `pending` notifications from the database and delivers them to
//! Matrix, with exponential backoff retry logic.
//!
//! Full implementation is Phase 5.

pub mod worker;

use thiserror::Error;

/// Errors that can occur in the background queue.
#[derive(Debug, Error)]
pub enum QueueError {
    /// A database operation failed.
    #[error("database error: {0}")]
    Database(#[from] crate::db::DbError),
    /// The Matrix client rejected the message.
    #[error("matrix delivery failed: {0}")]
    MatrixDelivery(String),
}
