//! Persistence for command triggers (headers and secrets are handled elsewhere).

use rusqlite::OptionalExtension;
use thiserror::Error;

use super::models::{CommandTrigger, TriggerHeader, TriggerSecret};
use super::{now_unix, DbError, Pool};

/// Errors that can occur during command trigger operations.
#[derive(Debug, Error)]
pub enum TriggerError {
    /// A database operation failed.
    #[error("database error: {0}")]
    Database(#[from] DbError),
    /// A stored allow-list is not a valid JSON array of strings.
    #[error("invalid JSON in command trigger: {0}")]
    Json(#[from] serde_json::Error),
    /// Another trigger already uses this command.
    #[error("a command trigger for '{0}' already exists")]
    DuplicateCommand(String),
}

/// Writable fields of a command trigger.
#[derive(Debug, Clone)]
pub struct TriggerInput<'a> {
    /// Command name; lowercased before storage.
    pub command: &'a str,
    /// Human-readable description.
    pub description: &'a str,
    /// Whether the trigger is enabled.
    pub enabled: bool,
    /// HTTP method: `GET`, `POST`, `PUT`, `PATCH`, or `DELETE`.
    pub method: &'a str,
    /// Target URL.
    pub url: &'a str,
    /// Response template; `None` means a status-only reply.
    pub response_template: Option<&'a str>,
    /// Optional access profile ID.
    pub access_profile_id: Option<i64>,
    /// Allowed room IDs.
    pub allowed_rooms: &'a [String],
    /// Allowed user MXIDs.
    pub allowed_users: &'a [String],
}

/// Columns selected for every query; order matches [`map_row`].
const COLUMNS: &str = "id, command, description, enabled, method, url, response_template, \
                       access_profile_id, allowed_rooms, allowed_users, created_at, updated_at";

/// Raw row with the JSON columns still encoded.
struct RawRow {
    id: i64,
    command: String,
    description: String,
    enabled: bool,
    method: String,
    url: String,
    response_template: Option<String>,
    access_profile_id: Option<i64>,
    allowed_rooms: String,
    allowed_users: String,
    created_at: i64,
    updated_at: i64,
}

fn map_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RawRow> {
    Ok(RawRow {
        id: r.get(0)?,
        command: r.get(1)?,
        description: r.get(2)?,
        enabled: r.get(3)?,
        method: r.get(4)?,
        url: r.get(5)?,
        response_template: r.get(6)?,
        access_profile_id: r.get(7)?,
        allowed_rooms: r.get(8)?,
        allowed_users: r.get(9)?,
        created_at: r.get(10)?,
        updated_at: r.get(11)?,
    })
}

fn decode(raw: RawRow) -> Result<CommandTrigger, TriggerError> {
    Ok(CommandTrigger {
        id: raw.id,
        command: raw.command,
        description: raw.description,
        enabled: raw.enabled,
        method: raw.method,
        url: raw.url,
        response_template: raw.response_template,
        access_profile_id: raw.access_profile_id,
        allowed_rooms: serde_json::from_str(&raw.allowed_rooms)?,
        allowed_users: serde_json::from_str(&raw.allowed_users)?,
        created_at: raw.created_at,
        updated_at: raw.updated_at,
    })
}

/// Map a UNIQUE violation on `command` to [`TriggerError::DuplicateCommand`].
fn map_write_error(e: rusqlite::Error, command: &str) -> TriggerError {
    match &e {
        rusqlite::Error::SqliteFailure(err, _)
            if err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
        {
            TriggerError::DuplicateCommand(command.to_owned())
        }
        _ => TriggerError::Database(DbError::from(e)),
    }
}

/// Create a trigger and return the stored row.
pub fn create(pool: &Pool, input: &TriggerInput<'_>) -> Result<CommandTrigger, TriggerError> {
    let now = now_unix();
    let command = input.command.to_lowercase();
    let rooms = serde_json::to_string(input.allowed_rooms)?;
    let users = serde_json::to_string(input.allowed_users)?;
    let conn = pool.get().map_err(DbError::from)?;
    let raw = conn
        .query_row(
            &format!(
                "INSERT INTO command_triggers
                   (command, description, enabled, method, url, response_template,
                    access_profile_id, allowed_rooms, allowed_users, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
                 RETURNING {COLUMNS}"
            ),
            rusqlite::params![
                command,
                input.description,
                input.enabled,
                input.method,
                input.url,
                input.response_template,
                input.access_profile_id,
                rooms,
                users,
                now
            ],
            map_row,
        )
        .map_err(|e| map_write_error(e, &command))?;
    decode(raw)
}

/// Fetch a trigger by ID; `None` if it does not exist.
pub fn get_by_id(pool: &Pool, id: i64) -> Result<Option<CommandTrigger>, TriggerError> {
    let conn = pool.get().map_err(DbError::from)?;
    let raw = conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM command_triggers WHERE id = ?1"),
            rusqlite::params![id],
            map_row,
        )
        .optional()
        .map_err(DbError::from)?;
    raw.map(decode).transpose()
}

/// Fetch a trigger by command, case-insensitively; `None` if it does not exist.
pub fn get_by_command(pool: &Pool, command: &str) -> Result<Option<CommandTrigger>, TriggerError> {
    let conn = pool.get().map_err(DbError::from)?;
    let raw = conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM command_triggers WHERE command = ?1"),
            rusqlite::params![command.to_lowercase()],
            map_row,
        )
        .optional()
        .map_err(DbError::from)?;
    raw.map(decode).transpose()
}

/// List all triggers ordered by command.
pub fn list(pool: &Pool) -> Result<Vec<CommandTrigger>, TriggerError> {
    let conn = pool.get().map_err(DbError::from)?;
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {COLUMNS} FROM command_triggers ORDER BY command"
        ))
        .map_err(DbError::from)?;
    let raws = stmt
        .query_map([], map_row)
        .map_err(DbError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(DbError::from)?;
    raws.into_iter().map(decode).collect()
}

/// Replace a trigger's fields; `None` if it does not exist.
pub fn update(
    pool: &Pool,
    id: i64,
    input: &TriggerInput<'_>,
) -> Result<Option<CommandTrigger>, TriggerError> {
    let command = input.command.to_lowercase();
    let rooms = serde_json::to_string(input.allowed_rooms)?;
    let users = serde_json::to_string(input.allowed_users)?;
    let conn = pool.get().map_err(DbError::from)?;
    let raw = conn
        .query_row(
            &format!(
                "UPDATE command_triggers
                 SET command = ?1, description = ?2, enabled = ?3, method = ?4, url = ?5,
                     response_template = ?6, access_profile_id = ?7, allowed_rooms = ?8,
                     allowed_users = ?9, updated_at = ?10
                 WHERE id = ?11
                 RETURNING {COLUMNS}"
            ),
            rusqlite::params![
                command,
                input.description,
                input.enabled,
                input.method,
                input.url,
                input.response_template,
                input.access_profile_id,
                rooms,
                users,
                now_unix(),
                id
            ],
            map_row,
        )
        .optional()
        .map_err(|e| map_write_error(e, &command))?;
    raw.map(decode).transpose()
}

/// Enable or disable a trigger; `None` if it does not exist.
pub fn set_enabled(
    pool: &Pool,
    id: i64,
    enabled: bool,
) -> Result<Option<CommandTrigger>, TriggerError> {
    let conn = pool.get().map_err(DbError::from)?;
    let raw = conn
        .query_row(
            &format!(
                "UPDATE command_triggers SET enabled = ?1, updated_at = ?2
                 WHERE id = ?3
                 RETURNING {COLUMNS}"
            ),
            rusqlite::params![enabled, now_unix(), id],
            map_row,
        )
        .optional()
        .map_err(DbError::from)?;
    raw.map(decode).transpose()
}

/// Delete a trigger; returns whether a row was removed.
pub fn delete(pool: &Pool, id: i64) -> Result<bool, TriggerError> {
    let conn = pool.get().map_err(DbError::from)?;
    let n = conn
        .execute(
            "DELETE FROM command_triggers WHERE id = ?1",
            rusqlite::params![id],
        )
        .map_err(DbError::from)?;
    Ok(n > 0)
}

/// Replace all of a trigger's headers atomically; an empty slice clears them.
pub fn set_headers(
    pool: &Pool,
    trigger_id: i64,
    headers: &[(String, String)],
) -> Result<(), TriggerError> {
    let mut conn = pool.get().map_err(DbError::from)?;
    let tx = conn.transaction().map_err(DbError::from)?;
    tx.execute(
        "DELETE FROM trigger_headers WHERE trigger_id = ?1",
        rusqlite::params![trigger_id],
    )
    .map_err(DbError::from)?;
    for (name, value) in headers {
        tx.execute(
            "INSERT INTO trigger_headers (trigger_id, name, value) VALUES (?1, ?2, ?3)",
            rusqlite::params![trigger_id, name, value],
        )
        .map_err(DbError::from)?;
    }
    tx.commit().map_err(DbError::from)?;
    Ok(())
}

/// List a trigger's headers ordered by name.
pub fn list_headers(pool: &Pool, trigger_id: i64) -> Result<Vec<TriggerHeader>, TriggerError> {
    let conn = pool.get().map_err(DbError::from)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, trigger_id, name, value FROM trigger_headers
             WHERE trigger_id = ?1 ORDER BY name",
        )
        .map_err(DbError::from)?;
    let headers = stmt
        .query_map(rusqlite::params![trigger_id], |r| {
            Ok(TriggerHeader {
                id: r.get(0)?,
                trigger_id: r.get(1)?,
                name: r.get(2)?,
                value: r.get(3)?,
            })
        })
        .map_err(DbError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(DbError::from)?;
    Ok(headers)
}

/// Insert a secret, or replace its ciphertext if `name` already exists for the trigger.
///
/// `ciphertext` must already be encrypted; no crypto happens here.
pub fn upsert_secret(
    pool: &Pool,
    trigger_id: i64,
    name: &str,
    ciphertext: &str,
) -> Result<(), TriggerError> {
    let conn = pool.get().map_err(DbError::from)?;
    conn.execute(
        "INSERT INTO trigger_secrets (trigger_id, name, ciphertext, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(trigger_id, name)
         DO UPDATE SET ciphertext = excluded.ciphertext, updated_at = excluded.updated_at",
        rusqlite::params![trigger_id, name, ciphertext, now_unix()],
    )
    .map_err(DbError::from)?;
    Ok(())
}

/// List a trigger's secrets (still encrypted) ordered by name.
pub fn list_secrets(pool: &Pool, trigger_id: i64) -> Result<Vec<TriggerSecret>, TriggerError> {
    let conn = pool.get().map_err(DbError::from)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, trigger_id, name, ciphertext, updated_at FROM trigger_secrets
             WHERE trigger_id = ?1 ORDER BY name",
        )
        .map_err(DbError::from)?;
    let secrets = stmt
        .query_map(rusqlite::params![trigger_id], |r| {
            Ok(TriggerSecret {
                id: r.get(0)?,
                trigger_id: r.get(1)?,
                name: r.get(2)?,
                ciphertext: r.get(3)?,
                updated_at: r.get(4)?,
            })
        })
        .map_err(DbError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(DbError::from)?;
    Ok(secrets)
}

/// Delete one secret; returns whether a row was removed.
pub fn delete_secret(pool: &Pool, trigger_id: i64, name: &str) -> Result<bool, TriggerError> {
    let conn = pool.get().map_err(DbError::from)?;
    let n = conn
        .execute(
            "DELETE FROM trigger_secrets WHERE trigger_id = ?1 AND name = ?2",
            rusqlite::params![trigger_id, name],
        )
        .map_err(DbError::from)?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{access_profiles, test_pool};

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    fn input<'a>(command: &'a str, rooms: &'a [String], users: &'a [String]) -> TriggerInput<'a> {
        TriggerInput {
            command,
            description: "desc",
            enabled: true,
            method: "POST",
            url: "https://example.com/hook",
            response_template: Some("ok {{status}}"),
            access_profile_id: None,
            allowed_rooms: rooms,
            allowed_users: users,
        }
    }

    #[test]
    fn create_then_get_returns_the_trigger() {
        // Arrange
        let pool = test_pool();
        let rooms = strings(&["!a:example.com"]);
        let users = strings(&["@alice:example.com"]);

        // Act
        let created = create(&pool, &input("Deploy", &rooms, &users)).unwrap();
        let fetched = get_by_id(&pool, created.id).unwrap().unwrap();

        // Assert
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.command, "deploy");
        assert_eq!(fetched.description, "desc");
        assert!(fetched.enabled);
        assert_eq!(fetched.method, "POST");
        assert_eq!(fetched.url, "https://example.com/hook");
        assert_eq!(fetched.response_template.as_deref(), Some("ok {{status}}"));
        assert_eq!(fetched.access_profile_id, None);
        assert_eq!(fetched.allowed_rooms, rooms);
        assert_eq!(fetched.allowed_users, users);
        assert!(fetched.created_at > 0);
        assert_eq!(fetched.created_at, fetched.updated_at);
    }

    #[test]
    fn get_by_id_returns_none_when_missing() {
        let pool = test_pool();
        assert!(get_by_id(&pool, 42).unwrap().is_none());
    }

    #[test]
    fn get_by_command_is_case_insensitive() {
        // Arrange
        let pool = test_pool();
        let created = create(&pool, &input("Deploy", &[], &[])).unwrap();

        // Act
        let upper = get_by_command(&pool, "DEPLOY").unwrap().unwrap();
        let mixed = get_by_command(&pool, "dEpLoY").unwrap().unwrap();

        // Assert
        assert_eq!(upper.id, created.id);
        assert_eq!(mixed.id, created.id);
        assert!(get_by_command(&pool, "other").unwrap().is_none());
    }

    #[test]
    fn list_returns_all_triggers_ordered_by_command() {
        // Arrange
        let pool = test_pool();
        create(&pool, &input("zeta", &[], &[])).unwrap();
        create(&pool, &input("alpha", &[], &[])).unwrap();

        // Act
        let triggers = list(&pool).unwrap();

        // Assert
        let commands: Vec<&str> = triggers.iter().map(|t| t.command.as_str()).collect();
        assert_eq!(commands, ["alpha", "zeta"]);
    }

    #[test]
    fn list_is_empty_without_triggers() {
        let pool = test_pool();
        assert!(list(&pool).unwrap().is_empty());
    }

    #[test]
    fn update_replaces_fields_and_returns_the_row() {
        // Arrange
        let pool = test_pool();
        let created = create(&pool, &input("deploy", &[], &[])).unwrap();
        let rooms = strings(&["!x:example.com"]);
        let mut changed = input("Release", &rooms, &[]);
        changed.description = "new";
        changed.enabled = false;
        changed.method = "GET";
        changed.url = "https://example.com/new";
        changed.response_template = None;

        // Act
        let updated = update(&pool, created.id, &changed).unwrap().unwrap();

        // Assert
        assert_eq!(updated.id, created.id);
        assert_eq!(updated.command, "release");
        assert_eq!(updated.description, "new");
        assert!(!updated.enabled);
        assert_eq!(updated.method, "GET");
        assert_eq!(updated.url, "https://example.com/new");
        assert_eq!(updated.response_template, None);
        assert_eq!(updated.allowed_rooms, rooms);
        assert_eq!(updated.created_at, created.created_at);
        assert_eq!(
            get_by_id(&pool, created.id).unwrap().unwrap().command,
            "release"
        );
    }

    #[test]
    fn update_returns_none_when_missing() {
        let pool = test_pool();
        assert!(update(&pool, 42, &input("x", &[], &[])).unwrap().is_none());
    }

    #[test]
    fn update_to_an_existing_command_is_rejected() {
        // Arrange
        let pool = test_pool();
        create(&pool, &input("one", &[], &[])).unwrap();
        let two = create(&pool, &input("two", &[], &[])).unwrap();

        // Act
        let result = update(&pool, two.id, &input("ONE", &[], &[]));

        // Assert
        assert!(matches!(result, Err(TriggerError::DuplicateCommand(c)) if c == "one"));
    }

    #[test]
    fn set_enabled_toggles_the_flag() {
        // Arrange
        let pool = test_pool();
        let created = create(&pool, &input("deploy", &[], &[])).unwrap();

        // Act
        let disabled = set_enabled(&pool, created.id, false).unwrap().unwrap();
        let fetched = get_by_id(&pool, created.id).unwrap().unwrap();
        let enabled = set_enabled(&pool, created.id, true).unwrap().unwrap();

        // Assert
        assert!(!disabled.enabled);
        assert!(!fetched.enabled);
        assert!(enabled.enabled);
    }

    #[test]
    fn set_enabled_returns_none_when_missing() {
        let pool = test_pool();
        assert!(set_enabled(&pool, 42, false).unwrap().is_none());
    }

    #[test]
    fn delete_removes_the_trigger() {
        // Arrange
        let pool = test_pool();
        let created = create(&pool, &input("deploy", &[], &[])).unwrap();

        // Act
        let deleted = delete(&pool, created.id).unwrap();

        // Assert
        assert!(deleted);
        assert!(get_by_id(&pool, created.id).unwrap().is_none());
    }

    #[test]
    fn delete_returns_false_when_missing() {
        let pool = test_pool();
        assert!(!delete(&pool, 42).unwrap());
    }

    #[test]
    fn create_with_duplicate_command_is_rejected_case_insensitively() {
        // Arrange
        let pool = test_pool();
        create(&pool, &input("deploy", &[], &[])).unwrap();

        // Act
        let result = create(&pool, &input("DEPLOY", &[], &[]));

        // Assert
        assert!(matches!(result, Err(TriggerError::DuplicateCommand(c)) if c == "deploy"));
    }

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn name_values(headers: &[TriggerHeader]) -> Vec<(&str, &str)> {
        headers
            .iter()
            .map(|h| (h.name.as_str(), h.value.as_str()))
            .collect()
    }

    #[test]
    fn set_headers_then_list_returns_headers_ordered_by_name() {
        // Arrange
        let pool = test_pool();
        let trigger = create(&pool, &input("deploy", &[], &[])).unwrap();

        // Act
        set_headers(
            &pool,
            trigger.id,
            &pairs(&[("X-Zeta", "z"), ("Authorization", "Bearer {{secret:T}}")]),
        )
        .unwrap();
        let headers = list_headers(&pool, trigger.id).unwrap();

        // Assert
        assert_eq!(
            name_values(&headers),
            [("Authorization", "Bearer {{secret:T}}"), ("X-Zeta", "z")]
        );
        assert!(headers.iter().all(|h| h.trigger_id == trigger.id));
    }

    #[test]
    fn set_headers_replaces_existing_headers() {
        // Arrange
        let pool = test_pool();
        let trigger = create(&pool, &input("deploy", &[], &[])).unwrap();
        let other = create(&pool, &input("other", &[], &[])).unwrap();
        set_headers(&pool, trigger.id, &pairs(&[("A", "1"), ("B", "2")])).unwrap();
        set_headers(&pool, other.id, &pairs(&[("A", "keep")])).unwrap();

        // Act
        set_headers(&pool, trigger.id, &pairs(&[("B", "3"), ("C", "4")])).unwrap();

        // Assert
        let headers = list_headers(&pool, trigger.id).unwrap();
        assert_eq!(name_values(&headers), [("B", "3"), ("C", "4")]);
        let untouched = list_headers(&pool, other.id).unwrap();
        assert_eq!(name_values(&untouched), [("A", "keep")]);
    }

    #[test]
    fn set_headers_with_empty_list_clears_headers() {
        // Arrange
        let pool = test_pool();
        let trigger = create(&pool, &input("deploy", &[], &[])).unwrap();
        set_headers(&pool, trigger.id, &pairs(&[("A", "1")])).unwrap();

        // Act
        set_headers(&pool, trigger.id, &[]).unwrap();

        // Assert
        assert!(list_headers(&pool, trigger.id).unwrap().is_empty());
    }

    #[test]
    fn set_headers_is_atomic_when_a_header_is_invalid() {
        // Arrange
        let pool = test_pool();
        let trigger = create(&pool, &input("deploy", &[], &[])).unwrap();
        set_headers(&pool, trigger.id, &pairs(&[("A", "1")])).unwrap();

        // Act: duplicate names violate UNIQUE(trigger_id, name)
        let result = set_headers(&pool, trigger.id, &pairs(&[("B", "2"), ("B", "3")]));

        // Assert
        assert!(matches!(result, Err(TriggerError::Database(_))));
        let headers = list_headers(&pool, trigger.id).unwrap();
        assert_eq!(name_values(&headers), [("A", "1")]);
    }

    #[test]
    fn deleting_a_trigger_cascades_to_its_headers() {
        // Arrange
        let pool = test_pool();
        let trigger = create(&pool, &input("deploy", &[], &[])).unwrap();
        set_headers(&pool, trigger.id, &pairs(&[("A", "1")])).unwrap();

        // Act
        delete(&pool, trigger.id).unwrap();

        // Assert
        let conn = pool.get().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM trigger_headers", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    fn name_ciphertexts(secrets: &[TriggerSecret]) -> Vec<(&str, &str)> {
        secrets
            .iter()
            .map(|s| (s.name.as_str(), s.ciphertext.as_str()))
            .collect()
    }

    #[test]
    fn upsert_secret_then_list_returns_the_secret() {
        // Arrange
        let pool = test_pool();
        let trigger = create(&pool, &input("deploy", &[], &[])).unwrap();

        // Act
        upsert_secret(&pool, trigger.id, "TOKEN", "aabbcc").unwrap();
        let secrets = list_secrets(&pool, trigger.id).unwrap();

        // Assert
        assert_eq!(name_ciphertexts(&secrets), [("TOKEN", "aabbcc")]);
        assert_eq!(secrets[0].trigger_id, trigger.id);
        assert!(secrets[0].updated_at > 0);
    }

    #[test]
    fn upsert_secret_replaces_the_ciphertext_for_an_existing_name() {
        // Arrange
        let pool = test_pool();
        let trigger = create(&pool, &input("deploy", &[], &[])).unwrap();
        upsert_secret(&pool, trigger.id, "TOKEN", "old").unwrap();
        let first_id = list_secrets(&pool, trigger.id).unwrap()[0].id;

        // Act
        upsert_secret(&pool, trigger.id, "TOKEN", "new").unwrap();

        // Assert
        let secrets = list_secrets(&pool, trigger.id).unwrap();
        assert_eq!(name_ciphertexts(&secrets), [("TOKEN", "new")]);
        assert_eq!(secrets[0].id, first_id);
    }

    #[test]
    fn list_secrets_is_ordered_by_name_and_scoped_to_the_trigger() {
        // Arrange
        let pool = test_pool();
        let trigger = create(&pool, &input("deploy", &[], &[])).unwrap();
        let other = create(&pool, &input("other", &[], &[])).unwrap();
        upsert_secret(&pool, trigger.id, "ZETA", "z").unwrap();
        upsert_secret(&pool, trigger.id, "ALPHA", "a").unwrap();
        upsert_secret(&pool, other.id, "MID", "m").unwrap();

        // Act
        let secrets = list_secrets(&pool, trigger.id).unwrap();

        // Assert
        assert_eq!(name_ciphertexts(&secrets), [("ALPHA", "a"), ("ZETA", "z")]);
    }

    #[test]
    fn delete_secret_removes_only_the_named_secret() {
        // Arrange
        let pool = test_pool();
        let trigger = create(&pool, &input("deploy", &[], &[])).unwrap();
        upsert_secret(&pool, trigger.id, "A", "1").unwrap();
        upsert_secret(&pool, trigger.id, "B", "2").unwrap();

        // Act
        let deleted = delete_secret(&pool, trigger.id, "A").unwrap();

        // Assert
        assert!(deleted);
        let secrets = list_secrets(&pool, trigger.id).unwrap();
        assert_eq!(name_ciphertexts(&secrets), [("B", "2")]);
    }

    #[test]
    fn delete_secret_returns_false_when_missing() {
        let pool = test_pool();
        let trigger = create(&pool, &input("deploy", &[], &[])).unwrap();
        assert!(!delete_secret(&pool, trigger.id, "NOPE").unwrap());
    }

    #[test]
    fn deleting_a_trigger_cascades_to_its_secrets() {
        // Arrange
        let pool = test_pool();
        let trigger = create(&pool, &input("deploy", &[], &[])).unwrap();
        upsert_secret(&pool, trigger.id, "A", "1").unwrap();

        // Act
        delete(&pool, trigger.id).unwrap();

        // Assert
        let conn = pool.get().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM trigger_secrets", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn deleting_a_profile_sets_access_profile_id_to_null() {
        // Arrange
        let pool = test_pool();
        let profile = access_profiles::create(&pool, "ops", &[], &[]).unwrap();
        let mut with_profile = input("deploy", &[], &[]);
        with_profile.access_profile_id = Some(profile.id);
        let created = create(&pool, &with_profile).unwrap();
        assert_eq!(created.access_profile_id, Some(profile.id));

        // Act
        access_profiles::delete(&pool, profile.id).unwrap();

        // Assert
        let fetched = get_by_id(&pool, created.id).unwrap().unwrap();
        assert_eq!(fetched.access_profile_id, None);
    }
}
