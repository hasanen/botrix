//! Minimal forward-only migration runner for rusqlite.
//!
//! Migrations are embedded at compile time and applied in order. A
//! `_migrations` table records which have run, so startup is idempotent.

use super::{DbError, Pool};

/// Embedded migrations, applied in array order. `(version, sql)`.
const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../../migrations/0001_initial.sql")),
    (
        2,
        include_str!("../../migrations/0002_command_triggers.sql"),
    ),
];

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
        assert_eq!(count, MIGRATIONS.len() as i64);
        // matrix_state singleton seeded by the migration.
        let ms: i64 = conn
            .query_row("SELECT COUNT(*) FROM matrix_state", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ms, 1);
    }

    #[test]
    fn command_trigger_tables_exist_and_migrating_twice_is_idempotent() {
        // Arrange
        let pool = mem_pool();

        // Act
        run(&pool).unwrap();
        run(&pool).unwrap();

        // Assert
        let conn = pool.get().unwrap();
        for table in [
            "access_profiles",
            "command_triggers",
            "trigger_headers",
            "trigger_secrets",
        ] {
            let found: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(found, 1, "table {table} missing");
        }
        let applied: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM _migrations WHERE version = 2",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(applied, 1);
    }
}
