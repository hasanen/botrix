//! A `tower-sessions` [`SessionStore`] backed by our rusqlite pool.
//!
//! Replaces `tower-sessions-sqlx-store` (which pins an incompatible sqlx /
//! libsqlite3-sys). Records are serialized with `serde_json` and stored in the
//! `sessions` table keyed by the session id; `expiry_date` is kept as a Unix
//! timestamp so expired rows can be swept.

use async_trait::async_trait;
use tower_sessions::session::{Id, Record};
use tower_sessions::session_store::{self, ExpiredDeletion, SessionStore};

use super::Pool;

/// SQLite-backed session store using the application connection pool.
#[derive(Debug, Clone)]
pub struct SqliteSessionStore {
    pool: Pool,
}

impl SqliteSessionStore {
    /// Create a new store over the given pool.
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

fn encode(record: &Record) -> session_store::Result<String> {
    serde_json::to_string(record).map_err(|e| session_store::Error::Encode(e.to_string()))
}

fn backend<E: std::fmt::Display>(e: E) -> session_store::Error {
    session_store::Error::Backend(e.to_string())
}

#[async_trait]
impl SessionStore for SqliteSessionStore {
    async fn save(&self, record: &Record) -> session_store::Result<()> {
        let pool = self.pool.clone();
        let id = record.id.0.to_string();
        let expiry = record.expiry_date.unix_timestamp();
        let data = encode(record)?;
        tokio::task::spawn_blocking(move || {
            let conn = pool.get().map_err(backend)?;
            conn.execute(
                "INSERT INTO sessions (id, data, expiry_date) VALUES (?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET data = excluded.data, expiry_date = excluded.expiry_date",
                rusqlite::params![id, data, expiry],
            )
            .map_err(backend)?;
            Ok::<_, session_store::Error>(())
        })
        .await
        .map_err(backend)?
    }

    async fn load(&self, session_id: &Id) -> session_store::Result<Option<Record>> {
        let pool = self.pool.clone();
        let id = session_id.0.to_string();
        let now = crate::db::now_unix();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get().map_err(backend)?;
            let data: Option<String> = match conn.query_row(
                "SELECT data FROM sessions WHERE id = ?1 AND expiry_date > ?2",
                rusqlite::params![id, now],
                |r| r.get(0),
            ) {
                Ok(v) => Some(v),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(backend(e)),
            };
            match data {
                Some(json) => {
                    let record: Record = serde_json::from_str(&json)
                        .map_err(|e| session_store::Error::Decode(e.to_string()))?;
                    Ok(Some(record))
                }
                None => Ok(None),
            }
        })
        .await
        .map_err(backend)?
    }

    async fn delete(&self, session_id: &Id) -> session_store::Result<()> {
        let pool = self.pool.clone();
        let id = session_id.0.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get().map_err(backend)?;
            conn.execute("DELETE FROM sessions WHERE id = ?1", rusqlite::params![id])
                .map_err(backend)?;
            Ok::<_, session_store::Error>(())
        })
        .await
        .map_err(backend)?
    }
}

#[async_trait]
impl ExpiredDeletion for SqliteSessionStore {
    async fn delete_expired(&self) -> session_store::Result<()> {
        let pool = self.pool.clone();
        let now = crate::db::now_unix();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get().map_err(backend)?;
            conn.execute(
                "DELETE FROM sessions WHERE expiry_date <= ?1",
                rusqlite::params![now],
            )
            .map_err(backend)?;
            Ok::<_, session_store::Error>(())
        })
        .await
        .map_err(backend)?
    }
}
