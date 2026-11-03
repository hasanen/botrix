//! Inbound webhook system — token auth, template rendering, and notification persistence.
//!
//! Full implementation is Phase 4.

pub mod auth;
pub mod handler;
pub mod template;

use thiserror::Error;

/// Errors that can occur in the webhook subsystem.
#[derive(Debug, Error)]
pub enum WebhookError {
    /// Bearer token is missing, malformed, or does not match.
    #[error("unauthorized")]
    Unauthorized,
    /// Request body exceeds the configured size limit.
    #[error("payload too large")]
    PayloadTooLarge,
    /// Request body is not valid JSON.
    #[error("invalid JSON payload: {0}")]
    InvalidJson(String),
    /// No webhook is configured for the given identifier.
    #[error("webhook '{0}' not found")]
    NotFound(String),
    /// The webhook is disabled in configuration.
    #[error("webhook '{0}' is disabled")]
    Disabled(String),
    /// Template rendering failed.
    #[error("template error: {0}")]
    Template(#[from] template::TemplateError),
    /// A database error occurred.
    #[error("database error: {0}")]
    Database(#[from] crate::db::DbError),
}
