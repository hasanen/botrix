//! Persistence for command-trigger secrets.
//!
//! Triggers, headers and access profiles live in the YAML config. The database
//! holds only the encrypted secret values, keyed by `(command, name)`. The
//! command is stored lowercase and matched case-insensitively. Rows whose
//! command or name is no longer in the config are left in place and ignored by
//! callers.

use thiserror::Error;

use super::models::TriggerSecret;
use super::{now_unix, DbError, Pool};

/// Errors that can occur during trigger secret operations.
#[derive(Debug, Error)]
pub enum TriggerError {
    /// A database operation failed.
    #[error("database error: {0}")]
    Database(#[from] DbError),
}

/// Insert a secret, or replace its ciphertext if `name` already exists for the command.
///
/// `ciphertext` must already be encrypted; no crypto happens here.
pub fn upsert_secret(
    pool: &Pool,
    command: &str,
    name: &str,
    ciphertext: &str,
) -> Result<(), TriggerError> {
    let conn = pool.get().map_err(DbError::from)?;
    conn.execute(
        "INSERT INTO trigger_secrets (command, name, ciphertext, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(command, name)
         DO UPDATE SET ciphertext = excluded.ciphertext, updated_at = excluded.updated_at",
        rusqlite::params![command.to_lowercase(), name, ciphertext, now_unix()],
    )
    .map_err(DbError::from)?;
    Ok(())
}

/// List a command's secrets (still encrypted) ordered by name.
pub fn list_secrets(pool: &Pool, command: &str) -> Result<Vec<TriggerSecret>, TriggerError> {
    let conn = pool.get().map_err(DbError::from)?;
    let mut stmt = conn
        .prepare(
            "SELECT command, name, ciphertext, updated_at FROM trigger_secrets
             WHERE command = ?1 ORDER BY name",
        )
        .map_err(DbError::from)?;
    let secrets = stmt
        .query_map(rusqlite::params![command.to_lowercase()], |r| {
            Ok(TriggerSecret {
                command: r.get(0)?,
                name: r.get(1)?,
                ciphertext: r.get(2)?,
                updated_at: r.get(3)?,
            })
        })
        .map_err(DbError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(DbError::from)?;
    Ok(secrets)
}

/// Delete one secret; returns whether a row was removed.
pub fn delete_secret(pool: &Pool, command: &str, name: &str) -> Result<bool, TriggerError> {
    let conn = pool.get().map_err(DbError::from)?;
    let n = conn
        .execute(
            "DELETE FROM trigger_secrets WHERE command = ?1 AND name = ?2",
            rusqlite::params![command.to_lowercase(), name],
        )
        .map_err(DbError::from)?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_pool;

    #[test]
    fn upsert_then_list_returns_the_secret() {
        // Arrange
        let pool = test_pool();

        // Act
        upsert_secret(&pool, "deploy", "TOKEN", "cipher-1").unwrap();
        let secrets = list_secrets(&pool, "deploy").unwrap();

        // Assert
        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0].command, "deploy");
        assert_eq!(secrets[0].name, "TOKEN");
        assert_eq!(secrets[0].ciphertext, "cipher-1");
        assert!(secrets[0].updated_at > 0);
    }

    #[test]
    fn upsert_replaces_the_ciphertext_of_an_existing_secret() {
        // Arrange
        let pool = test_pool();
        upsert_secret(&pool, "deploy", "TOKEN", "old").unwrap();

        // Act
        upsert_secret(&pool, "deploy", "TOKEN", "new").unwrap();
        let secrets = list_secrets(&pool, "deploy").unwrap();

        // Assert
        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0].ciphertext, "new");
    }

    #[test]
    fn list_is_ordered_by_name_and_scoped_to_the_command() {
        // Arrange
        let pool = test_pool();
        upsert_secret(&pool, "deploy", "ZED", "z").unwrap();
        upsert_secret(&pool, "deploy", "ALPHA", "a").unwrap();
        upsert_secret(&pool, "other", "TOKEN", "o").unwrap();

        // Act
        let deploy = list_secrets(&pool, "deploy").unwrap();
        let other = list_secrets(&pool, "other").unwrap();

        // Assert
        let names: Vec<&str> = deploy.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["ALPHA", "ZED"]);
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].ciphertext, "o");
    }

    #[test]
    fn list_is_empty_for_a_command_without_secrets() {
        let pool = test_pool();
        assert!(list_secrets(&pool, "deploy").unwrap().is_empty());
    }

    #[test]
    fn command_is_matched_case_insensitively() {
        // Arrange
        let pool = test_pool();

        // Act
        upsert_secret(&pool, "Deploy", "TOKEN", "c").unwrap();
        let upper = list_secrets(&pool, "DEPLOY").unwrap();
        upsert_secret(&pool, "DEPLOY", "TOKEN", "c2").unwrap();

        // Assert
        assert_eq!(upper.len(), 1);
        assert_eq!(upper[0].command, "deploy");
        let all = list_secrets(&pool, "deploy").unwrap();
        assert_eq!(all.len(), 1, "same secret, not a duplicate row");
        assert_eq!(all[0].ciphertext, "c2");
    }

    #[test]
    fn secret_names_are_case_sensitive() {
        let pool = test_pool();
        upsert_secret(&pool, "deploy", "token", "a").unwrap();
        upsert_secret(&pool, "deploy", "TOKEN", "b").unwrap();
        assert_eq!(list_secrets(&pool, "deploy").unwrap().len(), 2);
    }

    #[test]
    fn delete_removes_only_the_named_secret_and_reports_it() {
        // Arrange
        let pool = test_pool();
        upsert_secret(&pool, "deploy", "A", "a").unwrap();
        upsert_secret(&pool, "deploy", "B", "b").unwrap();
        upsert_secret(&pool, "other", "A", "x").unwrap();

        // Act
        let removed = delete_secret(&pool, "deploy", "A").unwrap();
        let again = delete_secret(&pool, "deploy", "A").unwrap();

        // Assert
        assert!(removed);
        assert!(!again);
        let names: Vec<String> = list_secrets(&pool, "deploy")
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, ["B"]);
        assert_eq!(list_secrets(&pool, "other").unwrap().len(), 1);
    }
}
