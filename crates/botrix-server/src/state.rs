//! Shared application state passed to every request handler.

use std::sync::Arc;

use axum::extract::FromRef;
use axum_csrf::CsrfConfig;

use crate::config::Config;
use crate::db::Pool;

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
}

/// Allows the `CsrfToken` extractor to pull the CSRF config out of state.
impl FromRef<AppState> for CsrfConfig {
    fn from_ref(state: &AppState) -> Self {
        state.csrf_config.clone()
    }
}
