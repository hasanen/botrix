//! Database access layer — SQLite via rusqlite + r2d2.
//!
//! Manages the application database at `$BOTRIX_DATA_DIR/botrix.db`. rusqlite
//! shares the same native SQLite library as matrix-sdk, so a single `sqlite3`
//! is linked (avoids the plan §19.5 conflict). Query functions are synchronous;
//! callers in async contexts should keep calls short (they are small, indexed
//! queries on a local file).

/// Implement `Debug` for a row type, printing the listed fields normally and
/// the `secret` fields as `[redacted]` so credentials never reach logs.
macro_rules! debug_redacting {
    ($ty:ty { $($shown:ident),* $(,)? ; secret: $($secret:ident),+ $(,)? }) => {
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty))
                    $(.field(stringify!($shown), &self.$shown))*
                    $(.field(stringify!($secret), &"[redacted]"))+
                    .finish()
            }
        }
    };
}
pub(crate) use debug_redacting;

pub mod auth;
pub mod history;
pub mod logs;
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
    /// A stored value is not one of the values the application understands.
    #[error("unexpected value '{value}' in column {column}")]
    InvalidValue {
        /// Fully qualified column name (`table.column`).
        column: &'static str,
        /// The unexpected stored value.
        value: String,
    },
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

    // Switch to WAL once, on a single connection, *before* the pool exists.
    // The mode is stored in the database file, so pooled connections inherit
    // it. Doing this per pooled connection makes them race on a fresh
    // database, and SQLite does not always apply the busy timeout to a
    // concurrent WAL switch ("database is locked").
    {
        let conn = rusqlite::Connection::open(db_path)?;
        conn.execute_batch("PRAGMA busy_timeout = 5000; PRAGMA journal_mode = WAL;")?;
    }

    let manager = SqliteConnectionManager::file(db_path).with_init(|c| {
        // Per-connection settings (not persisted in the file).
        c.execute_batch("PRAGMA busy_timeout = 5000; PRAGMA foreign_keys = ON;")
    });
    let pool = Pool::new(manager)?;

    migrate::run(&pool)?;

    let recovered = maintenance::recover_stuck_notifications(&pool)?;
    if recovered > 0 {
        tracing::warn!(
            count = recovered,
            "reset notifications stuck in 'processing' to 'pending' after restart"
        );
        logs::record(
            &pool,
            botrix_common::types::LogLevel::Warn,
            logs::Component::System,
            "recovered interrupted deliveries after restart",
            Some(&format!("{recovered} notification(s) returned to pending")),
        );
    }

    tracing::info!(db_path, "database initialized");
    Ok(pool)
}

/// Open an existing application database read-only.
///
/// Unlike [`init`], this applies no migrations and performs no crash recovery,
/// so it is safe to use against a database a running server is using (e.g. for
/// `botrix status`). Fails if the file does not exist.
pub fn open_read_only(db_path: &str) -> Result<Pool, DbError> {
    use rusqlite::OpenFlags;
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    // Open once directly so a missing/unreadable file fails immediately
    // instead of r2d2 retrying until its connection timeout.
    rusqlite::Connection::open_with_flags(db_path, flags)?;
    let manager = SqliteConnectionManager::file(db_path).with_flags(flags);
    Ok(Pool::new(manager)?)
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

#[cfg(test)]
mod debug_redaction_tests {
    //! Credential-bearing row types must not reveal them through `{:?}`.

    use super::matrix::{MatrixState, SessionRecord};
    use super::models;

    fn assert_hidden(debug: &str, secrets: &[&str]) {
        for secret in secrets {
            assert!(!debug.contains(secret), "leaked '{secret}': {debug}");
        }
        assert!(debug.contains("[redacted]"), "no marker: {debug}");
    }

    #[test]
    fn session_record_hides_tokens_but_keeps_identity() {
        let debug = format!(
            "{:?}",
            SessionRecord {
                user_id: "@bot:example.com".into(),
                homeserver: "https://matrix.example.com".into(),
                device_id: "DEV".into(),
                oauth_client_id: None,
                access_token: "enc-access".into(),
                refresh_token: Some("enc-refresh".into()),
            }
        );
        assert!(debug.contains("@bot:example.com"));
        assert_hidden(&debug, &["enc-access", "enc-refresh"]);
    }

    #[test]
    fn matrix_state_hides_tokens() {
        let debug = format!(
            "{:?}",
            MatrixState {
                user_id: Some("@bot:example.com".into()),
                homeserver: None,
                access_token: Some("enc-access".into()),
                refresh_token: Some("enc-refresh".into()),
                device_id: None,
                oauth_client_id: None,
                connected_at: None,
                last_token_refresh: None,
                status: "connected".into(),
            }
        );
        assert!(debug.contains("connected"));
        assert_hidden(&debug, &["enc-access", "enc-refresh"]);
    }

    #[test]
    fn admin_row_hides_the_password_hash() {
        let debug = format!(
            "{:?}",
            models::Admin {
                id: 1,
                username: "alice".into(),
                password_hash: "$argon2id$v=19$secret-hash".into(),
                created_at: 0,
                updated_at: 0,
            }
        );
        assert!(debug.contains("alice"));
        assert_hidden(&debug, &["$argon2id$v=19$secret-hash"]);
    }

    #[test]
    fn webhook_token_row_hides_the_token_hash() {
        let debug = format!(
            "{:?}",
            models::WebhookToken {
                id: 1,
                webhook_id: "wud".into(),
                token_hash: Some("deadbeefhash".into()),
                enabled: true,
                created_at: None,
                updated_at: 0,
            }
        );
        assert!(debug.contains("wud"));
        assert_hidden(&debug, &["deadbeefhash"]);
    }

    #[test]
    fn models_matrix_state_hides_tokens() {
        let debug = format!(
            "{:?}",
            models::MatrixState {
                id: 1,
                user_id: None,
                homeserver: None,
                access_token: Some("enc-access".into()),
                refresh_token: Some("enc-refresh".into()),
                device_id: None,
                oauth_client_id: None,
                connected_at: None,
                last_token_refresh: None,
                status: "connected".into(),
            }
        );
        assert_hidden(&debug, &["enc-access", "enc-refresh"]);
    }
}
