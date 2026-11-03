//! Database access layer — SQLite via rusqlite + r2d2.
//!
//! Manages the application database at `$BOTRIX_DATA_DIR/botrix.db`. rusqlite
//! shares the same native SQLite library as matrix-sdk, so a single `sqlite3`
//! is linked (avoids the plan §19.5 conflict). Query functions are synchronous;
//! callers in async contexts should keep calls short (they are small, indexed
//! queries on a local file).

pub mod auth;
pub mod maintenance;
pub mod matrix;
pub mod migrate;
pub mod models;
pub mod session_store;
pub mod settings;
pub mod webhooks;

use r2d2_sqlite::SqliteConnectionManager;
use thiserror::Error;

/// Errors that can occur during database initialization or queries.
#[derive(Debug, Error)]
pub enum DbError {
    /// A rusqlite query or connection error.
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// A connection pool error.
    #[error("connection pool error: {0}")]
    Pool(#[from] r2d2::Error),
    /// A migration failed.
    #[error("migration error: {0}")]
    Migrate(String),
}

/// Shared connection pool type.
pub type Pool = r2d2::Pool<SqliteConnectionManager>;

/// A pooled connection.
pub type Conn = r2d2::PooledConnection<SqliteConnectionManager>;

/// Open (or create) the application database, apply migrations, and run
/// startup crash recovery.
pub fn init(db_path: &str) -> Result<Pool, DbError> {
    if let Some(parent) = std::path::Path::new(db_path).parent() {
        std::fs::create_dir_all(parent).ok();
    }

    let manager = SqliteConnectionManager::file(db_path).with_init(|c| {
        // WAL for better concurrency; enforce foreign keys.
        c.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )
    });
    let pool = Pool::new(manager)?;

    migrate::run(&pool)?;

    let recovered = maintenance::recover_stuck_notifications(&pool)?;
    if recovered > 0 {
        tracing::warn!(
            count = recovered,
            "reset notifications stuck in 'processing' to 'pending' after restart"
        );
    }

    tracing::info!(db_path, "database initialized");
    Ok(pool)
}

/// Current Unix timestamp in seconds.
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
/// Create an in-memory pool with migrations applied, for tests.
pub fn test_pool() -> Pool {
    let manager = SqliteConnectionManager::memory()
        .with_init(|c| c.execute_batch("PRAGMA foreign_keys = ON;"));
    // A single connection so the in-memory DB persists across calls.
    let pool = Pool::builder().max_size(1).build(manager).expect("pool");
    migrate::run(&pool).expect("migrate");
    pool
}
