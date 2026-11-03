//! Axum router assembly and server startup.

use thiserror::Error;

use crate::config::Config;

/// Errors that can occur during server startup.
#[derive(Debug, Error)]
pub enum ServerError {
    /// A required environment variable is missing or invalid.
    #[error("missing required environment variable '{name}': {reason}")]
    MissingEnv { name: &'static str, reason: String },
    /// The database could not be initialized.
    #[error("database initialization failed: {0}")]
    Database(#[from] crate::db::DbError),
    /// The TCP listener could not be bound.
    #[error("failed to bind {addr}: {source}")]
    Bind {
        addr: String,
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
    // Validate required environment variables before doing any other work.
    let _session_secret = require_env("BOTRIX_SESSION_SECRET")?;
    let _crypto_passphrase = require_env("BOTRIX_CRYPTO_PASSPHRASE")?;

    // Initialize the application database.
    let db_path = format!("{data_dir}/botrix.db");
    let _pool = crate::db::init(&db_path).await?;

    let addr = format!("{}:{}", config.server.host, config.server.port);
    tracing::info!(%addr, "starting Botrix server");

    let router = build_router();

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|source| ServerError::Bind {
            addr: addr.clone(),
            source,
        })?;

    axum::serve(listener, router).await?;

    Ok(())
}

fn build_router() -> axum::Router {
    // Placeholder router — routes are wired in later phases.
    axum::Router::new().route(
        "/health",
        axum::routing::get(|| async { axum::http::StatusCode::OK }),
    )
}

fn require_env(name: &'static str) -> Result<String, ServerError> {
    std::env::var(name).map_err(|_| ServerError::MissingEnv {
        name,
        reason: "variable is not set".to_owned(),
    })
}
