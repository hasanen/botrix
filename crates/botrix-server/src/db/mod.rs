//! Database access layer — SQLite via sqlx.
//!
//! Manages the application database at `$BOTRIX_DATA_DIR/botrix.db`.
//! The Matrix SDK manages its own stores separately under `$BOTRIX_DATA_DIR/matrix/`.

pub mod models;

use thiserror::Error;

/// Errors that can occur during database initialization or queries.
#[derive(Debug, Error)]
pub enum DbError {
    /// SQLx returned an error.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    /// A required database migration failed.
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

/// Shared connection pool type.
pub type Pool = sqlx::SqlitePool;

/// Open (or create) the application database and run all pending migrations.
pub async fn init(db_path: &str) -> Result<Pool, DbError> {
    // Ensure the parent directory exists.
    if let Some(parent) = std::path::Path::new(db_path).parent() {
        std::fs::create_dir_all(parent).ok();
    }

    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true);

    let pool = sqlx::SqlitePool::connect_with(options).await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    tracing::info!(db_path, "database initialized");
    Ok(pool)
}
