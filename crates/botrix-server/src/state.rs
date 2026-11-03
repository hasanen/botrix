//! Shared application state passed to every request handler.

use std::sync::Arc;

use axum::extract::FromRef;
use axum_csrf::CsrfConfig;

use crate::config::Config;
use crate::db::Pool;
use crate::queue::sender::MessageSender;

/// Application state shared across all Axum handlers.
///
/// Cheap to clone: the pool and config are reference-counted, and
/// [`CsrfConfig`] is clone-by-value.
#[derive(Clone)]
pub struct AppState {
    /// Application database connection pool.
    pub pool: Pool,
    /// Parsed and validated configuration.
    pub config: Arc<Config>,
    /// CSRF middleware configuration.
    pub csrf_config: CsrfConfig,
    /// The message sender shared with the delivery worker. Used by the
    /// test-message endpoint to deliver through the same path as queued
    /// notifications. This is the stub sender when Matrix is not connected.
    pub sender: Arc<dyn MessageSender>,
    /// The live Matrix client, present only when a session was restored.
    /// Handlers use it for best-effort room introspection; its absence means
    /// room membership/encryption cannot be determined.
    pub matrix_client: Option<matrix_sdk::Client>,
}

/// Allows the `CsrfToken` extractor to pull the CSRF config out of state.
impl FromRef<AppState> for CsrfConfig {
    fn from_ref(state: &AppState) -> Self {
        state.csrf_config.clone()
    }
}
