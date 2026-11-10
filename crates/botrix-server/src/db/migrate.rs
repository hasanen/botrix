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
    (
        3,
        include_str!("../../migrations/0003_triggers_from_config.sql"),
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

    fn table_exists(pool: &Pool, table: &str) -> bool {
        let conn = pool.get().unwrap();
        let found: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        found == 1
    }

    #[test]
    fn only_trigger_secrets_remains_and_migrating_twice_is_idempotent() {
        // Arrange
        let pool = mem_pool();

        // Act
        run(&pool).unwrap();
        run(&pool).unwrap();

        // Assert
        assert!(table_exists(&pool, "trigger_secrets"));
        for gone in ["access_profiles", "command_triggers", "trigger_headers"] {
            assert!(!table_exists(&pool, gone), "table {gone} should be dropped");
        }
        let conn = pool.get().unwrap();
        let columns: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info('trigger_secrets') ORDER BY cid")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(columns, ["command", "name", "ciphertext", "updated_at"]);
        let applied: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM _migrations WHERE version = 3",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(applied, 1);
    }

    #[test]
    fn upgrades_a_database_that_already_has_the_old_trigger_tables() {
        // Arrange: a database migrated up to 0002 and holding trigger rows.
        let pool = mem_pool();
        {
            let conn = pool.get().unwrap();
            conn.execute_batch(
                "CREATE TABLE _migrations (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL);",
            )
            .unwrap();
            for (version, sql) in &MIGRATIONS[..2] {
                conn.execute_batch(sql).unwrap();
                conn.execute(
                    "INSERT INTO _migrations (version, applied_at) VALUES (?1, 0)",
                    [version],
                )
                .unwrap();
            }
            conn.execute_batch(
                "INSERT INTO access_profiles (name, created_at, updated_at) VALUES ('ops', 0, 0);
                 INSERT INTO command_triggers
                   (command, method, url, access_profile_id, created_at, updated_at)
                   VALUES ('deploy', 'POST', 'https://example.com', 1, 0, 0);
                 INSERT INTO trigger_headers (trigger_id, name, value) VALUES (1, 'X', 'y');
                 INSERT INTO trigger_secrets (trigger_id, name, ciphertext, updated_at)
                   VALUES (1, 'TOKEN', 'abc', 0);",
            )
            .unwrap();
        }

        // Act
        run(&pool).unwrap();

        // Assert
        assert!(table_exists(&pool, "trigger_secrets"));
        assert!(!table_exists(&pool, "command_triggers"));
        let conn = pool.get().unwrap();
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM trigger_secrets", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0, "old UI-defined secrets are not carried over");
    }
}
