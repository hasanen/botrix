//! Minimal forward-only migration runner for rusqlite.
//!
//! Migrations are embedded at compile time and applied in order. A
//! `_migrations` table records which have run, so startup is idempotent.

use super::{DbError, Pool};

/// Embedded migrations, applied in array order. `(version, sql)`.
const MIGRATIONS: &[(i64, &str)] = &[(1, include_str!("../../migrations/0001_initial.sql"))];

/// Apply all pending migrations.
pub fn run(pool: &Pool) -> Result<(), DbError> {
    let conn = pool.get()?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (
            version INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL
        );",
    )?;

    let applied: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM _migrations",
            [],
            |r| r.get(0),
        )
        .map_err(DbError::from)?;

    for (version, sql) in MIGRATIONS {
        if *version > applied {
            conn.execute_batch(sql)?;
            conn.execute(
                "INSERT INTO _migrations (version, applied_at) VALUES (?1, ?2)",
                rusqlite::params![version, super::now_unix()],
            )?;
            tracing::info!(version, "applied migration");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use r2d2_sqlite::SqliteConnectionManager;

    fn mem_pool() -> Pool {
        let mgr = SqliteConnectionManager::memory();
        Pool::builder().max_size(1).build(mgr).unwrap()
    }

    #[test]
    fn applies_and_is_idempotent() {
        let pool = mem_pool();
        run(&pool).unwrap();
        // Second run is a no-op (no error, tables already exist).
        run(&pool).unwrap();

        let conn = pool.get().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM _migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        // matrix_state singleton seeded by the migration.
        let ms: i64 = conn
            .query_row("SELECT COUNT(*) FROM matrix_state", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ms, 1);
    }
}
