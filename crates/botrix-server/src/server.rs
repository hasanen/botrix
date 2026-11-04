//! Axum router assembly and server startup.

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use axum_csrf::{CsrfConfig, Key, SameSite};
use thiserror::Error;
use tower_sessions::{Expiry, SessionManagerLayer};

use crate::config::Config;
use crate::db::session_store::SqliteSessionStore;
use crate::session;
use crate::state::AppState;

/// Minimum length of `BOTRIX_SESSION_SECRET` in bytes (§16.4).
const MIN_SESSION_SECRET_BYTES: usize = 32;

/// Maximum body size for the authenticated UI API (plan §16.6). Bodies here are
/// small JSON forms; the inbound webhook endpoint has its own, configurable
/// limit (`server.max_body_bytes`).
const API_MAX_BODY_BYTES: usize = 64 * 1024;

/// Errors that can occur during server startup.
#[derive(Debug, Error)]
pub enum ServerError {
    /// A required environment variable is missing or invalid.
    #[error("missing required environment variable '{name}': {reason}")]
    MissingEnv {
        /// Name of the required environment variable.
        name: &'static str,
        /// Why the variable is considered missing or invalid.
        reason: String,
    },
    /// The database could not be initialized.
    #[error("database initialization failed: {0}")]
    Database(#[from] crate::db::DbError),
    /// The TCP listener could not be bound.
    #[error("failed to bind {addr}: {source}")]
    Bind {
        /// Address that could not be bound.
        addr: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The Axum server returned an error.
    #[error("server error: {0}")]
    Axum(#[from] std::io::Error),
}

/// Start the HTTP server with the given configuration.
///
/// This function blocks until the server exits.
pub async fn run(config: Config, data_dir: &str) -> Result<(), ServerError> {
    // Validate required environment variables before doing any other work,
    // so misconfiguration fails fast before binding a port.
    let session_secret = require_env("BOTRIX_SESSION_SECRET")?;
    validate_session_secret(&session_secret)?;
    let _crypto_passphrase = require_env("BOTRIX_CRYPTO_PASSPHRASE")?;

    // Initialize the application database (runs migrations and crash recovery).
    let db_path = format!("{data_dir}/botrix.db");
    let pool = crate::db::init(&db_path)?;

    // Start the periodic history cleanup task.
    crate::db::maintenance::spawn_cleanup_task(pool.clone());

    // Attempt to restore a persisted Matrix session (§9.4). On success the
    // worker and API share a live-client-backed sender; otherwise they use the
    // stub sender that keeps notifications queued. Matrix connectivity is not
    // required for the server to run.
    let crypto_passphrase = _crypto_passphrase;
    let matrix_client = match crate::matrix::client::restore_client(
        &pool,
        &config,
        data_dir,
        &crypto_passphrase,
    )
    .await
    {
        Ok(client) => client,
        Err(err) => {
            tracing::error!(error = %err, "error during Matrix session restore; continuing disconnected");
            crate::db::logs::record(
                &pool,
                botrix_common::types::LogLevel::Error,
                crate::db::logs::Component::Matrix,
                "session restore failed; continuing disconnected",
                Some(&err.to_string()),
            );
            None
        }
    };
    if let Some(client) = &matrix_client {
        crate::db::logs::record(
            &pool,
            botrix_common::types::LogLevel::Info,
            crate::db::logs::Component::Matrix,
            "session restored",
            client.user_id().map(|u| u.to_string()).as_deref(),
        );
    }

    let sender: std::sync::Arc<dyn crate::queue::sender::MessageSender> = match &matrix_client {
        Some(client) => {
            tracing::info!("Matrix session active; using live message sender");
            std::sync::Arc::new(crate::matrix::MatrixSender::new(
                client.clone(),
                crate::matrix::client::destination_map(&config),
            ))
        }
        None => {
            tracing::info!("Matrix not connected; using stub message sender");
            std::sync::Arc::new(crate::matrix::StubSender)
        }
    };

    // Start the background notification worker with the chosen sender.
    crate::queue::worker::spawn(pool.clone(), sender.clone());

    // For local HTTP testing, allow disabling Secure cookies via env so the
    // browser accepts them over plain http://localhost. Defaults to secure.
    let secure_cookies = std::env::var("BOTRIX_INSECURE_COOKIES")
        .map(|v| v != "1" && v.to_lowercase() != "true")
        .unwrap_or(true);

    let pool_for_log = pool.clone();
    let mut router = build_app_with_matrix(
        pool,
        config.clone(),
        &session_secret,
        secure_cookies,
        sender,
        matrix_client,
    )
    .await?;

    // Serve the frontend. BOTRIX_FRONTEND_DIR (development) serves from disk;
    // otherwise the build embedded in the binary is used. Both fall back to
    // index.html for client-side routes.
    if let Ok(dir) = std::env::var("BOTRIX_FRONTEND_DIR") {
        use tower_http::services::{ServeDir, ServeFile};
        let index = format!("{dir}/index.html");
        let serve = ServeDir::new(&dir).fallback(ServeFile::new(index));
        router = router.fallback_service(serve);
        tracing::info!(dir, "serving frontend assets from disk");
    } else {
        router = router.fallback(crate::assets::embedded_handler);
        tracing::info!("serving embedded frontend assets");
    }

    let addr = format!("{}:{}", config.server.host, config.server.port);
    tracing::info!(%addr, "starting Botrix server");
    crate::db::logs::record(
        &pool_for_log,
        botrix_common::types::LogLevel::Info,
        crate::db::logs::Component::System,
        "server started",
        Some(&format!("version {}", env!("CARGO_PKG_VERSION"))),
    );

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|source| ServerError::Bind {
            addr: addr.clone(),
            source,
        })?;

    axum::serve(listener, router).await?;

    Ok(())
}

/// Build a fully-layered application router for the given database pool.
///
/// Used by both production startup and integration tests. `secure_cookies`
/// should be `false` in tests (no TLS) and `true` in production.
pub async fn build_app(
    pool: crate::db::Pool,
    config: Config,
    session_secret: &str,
    secure_cookies: bool,
) -> Result<Router, ServerError> {
    // Default to the stub sender and no Matrix client. Production startup uses
    // [`build_app_with_matrix`] to inject a live-client-backed sender.
    let sender: Arc<dyn crate::queue::sender::MessageSender> = Arc::new(crate::matrix::StubSender);
    build_app_with_matrix(pool, config, session_secret, secure_cookies, sender, None).await
}

/// Build the application router, injecting the message sender and optional
/// Matrix client into shared state.
///
/// Used by production startup so the test-message endpoint and the delivery
/// worker share one sender, and the Matrix API can introspect the live client.
pub async fn build_app_with_matrix(
    pool: crate::db::Pool,
    config: Config,
    session_secret: &str,
    secure_cookies: bool,
    sender: Arc<dyn crate::queue::sender::MessageSender>,
    matrix_client: Option<matrix_sdk::Client>,
) -> Result<Router, ServerError> {
    // The sessions table is created by our migrations; no store migrate step.
    let session_store = SqliteSessionStore::new(pool.clone());

    // The configurable idle/absolute limits are enforced per request by
    // `session::enforce_timeouts` (so changes apply without a restart). The
    // layer's expiry is only a backstop that must outlive any allowed limit.
    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(secure_cookies)
        .with_http_only(true)
        .with_same_site(tower_sessions::cookie::SameSite::Strict)
        .with_expiry(Expiry::OnInactivity(time::Duration::seconds(
            crate::db::settings::MAX_ABSOLUTE_LIFETIME_SECS,
        )));

    let csrf_key = Key::from(&derive_key_material(session_secret));
    let csrf_config = CsrfConfig::default()
        .with_cookie_name("botrix_csrf")
        .with_http_only(true)
        .with_secure(secure_cookies)
        .with_cookie_same_site(SameSite::Strict)
        .with_key(Some(csrf_key));

    let state = AppState {
        pool,
        config: Arc::new(config),
        csrf_config,
        sender,
        matrix_client,
    };

    // Layer order matters: the timeout check runs inside the session layer
    // (it needs the `Session`) but before any handler.
    let timeouts = axum::middleware::from_fn_with_state(state.clone(), session::enforce_timeouts);
    Ok(build_router(state).layer(timeouts).layer(session_layer))
}

/// Assemble the API router with the given application state.
fn build_router(state: AppState) -> Router {
    use crate::api::{
        auth, logs, matrix as api_matrix, notifications, settings, webhooks as api_webhooks,
    };
    use crate::webhooks::handler as webhook_handler;
    use axum::extract::DefaultBodyLimit;

    let max_body = state.config.server.max_body_bytes;

    // The public, token-authenticated inbound webhook endpoint. Its body limit
    // is raised to at least the configured maximum so the handler's explicit
    // 413 check governs rather than Axum's 2 MiB default.
    let notify = Router::new()
        .route(
            "/api/v1/webhooks/notify/{identifier}",
            post(webhook_handler::notify),
        )
        .layer(DefaultBodyLimit::max(max_body.max(2 * 1024 * 1024)));

    Router::new()
        .route("/health", get(|| async { axum::http::StatusCode::OK }))
        // Auth & session
        .route("/api/v1/auth/csrf", get(auth::csrf))
        .route("/api/v1/setup", post(auth::setup))
        .route("/api/v1/auth/login", post(auth::login))
        .route("/api/v1/auth/logout", post(auth::logout))
        .route("/api/v1/auth/me", get(auth::me))
        // Settings
        .route(
            "/api/v1/settings",
            get(settings::get_settings).put(settings::update_settings),
        )
        .route("/api/v1/settings/password", post(settings::change_password))
        .route("/api/v1/settings/info", get(settings::app_info))
        // History
        .route("/api/v1/notifications", get(notifications::list))
        .route("/api/v1/logs", get(logs::list))
        // Webhook management (authenticated)
        .route("/api/v1/webhooks", get(api_webhooks::list))
        .route(
            "/api/v1/webhooks/{id}/token",
            post(api_webhooks::generate_token).delete(api_webhooks::revoke_token),
        )
        // Matrix integration (authenticated)
        .route("/api/v1/matrix/status", get(api_matrix::status))
        .route("/api/v1/matrix/rooms", get(api_matrix::rooms))
        .route(
            "/api/v1/matrix/rooms/{name}/join",
            post(api_matrix::join_room),
        )
        .route("/api/v1/matrix/connect", post(api_matrix::connect))
        .route("/api/v1/matrix/connect/poll", get(api_matrix::connect_poll))
        .route("/api/v1/matrix/disconnect", post(api_matrix::disconnect))
        .route(
            "/api/v1/matrix/test-message",
            post(api_matrix::test_message),
        )
        .route(
            "/api/v1/matrix/recovery-key",
            post(api_matrix::recovery_key),
        )
        // Applies to the routes above only (layers affect routes added before
        // them); the webhook router merged below keeps its own limit. Axum
        // rejects over-limit bodies with 413 while reading, before parsing.
        .layer(DefaultBodyLimit::max(API_MAX_BODY_BYTES))
        .merge(notify)
        .with_state(state)
}

fn require_env(name: &'static str) -> Result<String, ServerError> {
    std::env::var(name).map_err(|_| ServerError::MissingEnv {
        name,
        reason: "variable is not set".to_owned(),
    })
}

/// Derive 64 bytes of key material from the session secret via SHA-512.
///
/// Used to produce a cookie signing key of the length `Key::from` requires,
/// deterministically from the operator-provided secret.
fn derive_key_material(secret: &str) -> [u8; 64] {
    use sha2::{Digest, Sha512};
    let digest = Sha512::digest(secret.as_bytes());
    let mut out = [0u8; 64];
    out.copy_from_slice(&digest);
    out
}

/// Validate that the session secret has at least [`MIN_SESSION_SECRET_BYTES`].
fn validate_session_secret(secret: &str) -> Result<(), ServerError> {
    // `str::len` is the UTF-8 byte length, which is what we want here.
    if secret.len() < MIN_SESSION_SECRET_BYTES {
        return Err(ServerError::MissingEnv {
            name: "BOTRIX_SESSION_SECRET",
            reason: format!(
                "must be at least {MIN_SESSION_SECRET_BYTES} bytes, got {}",
                secret.len()
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_short_session_secret() {
        let result = validate_session_secret("too-short");
        assert!(matches!(result, Err(ServerError::MissingEnv { .. })));
    }

    #[test]
    fn accepts_long_enough_session_secret() {
        // Exactly 32 bytes.
        let secret = "0123456789abcdef0123456789abcdef";
        assert_eq!(secret.len(), 32);
        assert!(validate_session_secret(secret).is_ok());
    }
}
