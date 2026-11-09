//! Persistence for access profiles (named room/user allow-lists).

use rusqlite::OptionalExtension;
use thiserror::Error;

use super::models::AccessProfile;
use super::{now_unix, DbError, Pool};

/// Errors that can occur during access profile operations.
#[derive(Debug, Error)]
pub enum AccessProfileError {
    /// A database operation failed.
    #[error("database error: {0}")]
    Database(#[from] DbError),
    /// A stored allow-list is not a valid JSON array of strings.
    #[error("invalid JSON in access profile: {0}")]
    Json(#[from] serde_json::Error),
    /// Another profile already uses this name.
    #[error("an access profile named '{0}' already exists")]
    DuplicateName(String),
}

/// Columns selected for every query; order matches [`map_row`].
const COLUMNS: &str = "id, name, allowed_rooms, allowed_users, created_at, updated_at";

/// Raw row with the JSON columns still encoded.
type RawRow = (i64, String, String, String, i64, i64);

fn map_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RawRow> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
    ))
}

fn decode(raw: RawRow) -> Result<AccessProfile, AccessProfileError> {
    let (id, name, rooms, users, created_at, updated_at) = raw;
    Ok(AccessProfile {
        id,
        name,
        allowed_rooms: serde_json::from_str(&rooms)?,
        allowed_users: serde_json::from_str(&users)?,
        created_at,
        updated_at,
    })
}

/// Map a UNIQUE violation on `name` to [`AccessProfileError::DuplicateName`].
fn map_write_error(e: rusqlite::Error, name: &str) -> AccessProfileError {
    match &e {
        rusqlite::Error::SqliteFailure(err, _)
            if err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
        {
            AccessProfileError::DuplicateName(name.to_owned())
        }
        _ => AccessProfileError::Database(DbError::from(e)),
    }
}

/// Create a profile and return the stored row.
pub fn create(
    pool: &Pool,
    name: &str,
    allowed_rooms: &[String],
    allowed_users: &[String],
) -> Result<AccessProfile, AccessProfileError> {
    let now = now_unix();
    let rooms = serde_json::to_string(allowed_rooms)?;
    let users = serde_json::to_string(allowed_users)?;
    let conn = pool.get().map_err(DbError::from)?;
    let raw = conn
        .query_row(
            &format!(
                "INSERT INTO access_profiles (name, allowed_rooms, allowed_users, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?4)
                 RETURNING {COLUMNS}"
            ),
            rusqlite::params![name, rooms, users, now],
            map_row,
        )
        .map_err(|e| map_write_error(e, name))?;
    decode(raw)
}

/// Fetch a profile by ID; `None` if it does not exist.
pub fn get_by_id(pool: &Pool, id: i64) -> Result<Option<AccessProfile>, AccessProfileError> {
    let conn = pool.get().map_err(DbError::from)?;
    let raw = conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM access_profiles WHERE id = ?1"),
            rusqlite::params![id],
            map_row,
        )
        .optional()
        .map_err(DbError::from)?;
    raw.map(decode).transpose()
}

/// List all profiles ordered by name.
pub fn list(pool: &Pool) -> Result<Vec<AccessProfile>, AccessProfileError> {
    let conn = pool.get().map_err(DbError::from)?;
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {COLUMNS} FROM access_profiles ORDER BY name"
        ))
        .map_err(DbError::from)?;
    let raws = stmt
        .query_map([], map_row)
        .map_err(DbError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(DbError::from)?;
    raws.into_iter().map(decode).collect()
}

/// Replace a profile's name and allow-lists; `None` if it does not exist.
pub fn update(
    pool: &Pool,
    id: i64,
    name: &str,
    allowed_rooms: &[String],
    allowed_users: &[String],
) -> Result<Option<AccessProfile>, AccessProfileError> {
    let rooms = serde_json::to_string(allowed_rooms)?;
    let users = serde_json::to_string(allowed_users)?;
    let conn = pool.get().map_err(DbError::from)?;
    let raw = conn
        .query_row(
            &format!(
                "UPDATE access_profiles
                 SET name = ?1, allowed_rooms = ?2, allowed_users = ?3, updated_at = ?4
                 WHERE id = ?5
                 RETURNING {COLUMNS}"
            ),
            rusqlite::params![name, rooms, users, now_unix(), id],
            map_row,
        )
        .optional()
        .map_err(|e| map_write_error(e, name))?;
    raw.map(decode).transpose()
}

/// Delete a profile; returns whether a row was removed.
pub fn delete(pool: &Pool, id: i64) -> Result<bool, AccessProfileError> {
    let conn = pool.get().map_err(DbError::from)?;
    let n = conn
        .execute(
            "DELETE FROM access_profiles WHERE id = ?1",
            rusqlite::params![id],
        )
        .map_err(DbError::from)?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_pool;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn create_then_get_returns_the_profile() {
        // Arrange
        let pool = test_pool();
        let rooms = strings(&["!a:example.com", "!b:example.com"]);
        let users = strings(&["@alice:example.com"]);

        // Act
        let created = create(&pool, "ops", &rooms, &users).unwrap();
        let fetched = get_by_id(&pool, created.id).unwrap().unwrap();

        // Assert
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.name, "ops");
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
    fn list_returns_all_profiles_ordered_by_name() {
        // Arrange
        let pool = test_pool();
        create(&pool, "zeta", &[], &[]).unwrap();
        create(&pool, "alpha", &strings(&["!r:example.com"]), &[]).unwrap();

        // Act
        let profiles = list(&pool).unwrap();

        // Assert
        let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["alpha", "zeta"]);
        assert_eq!(profiles[0].allowed_rooms, strings(&["!r:example.com"]));
    }

    #[test]
    fn list_is_empty_without_profiles() {
        let pool = test_pool();
        assert!(list(&pool).unwrap().is_empty());
    }

    #[test]
    fn update_replaces_fields_and_returns_the_row() {
        // Arrange
        let pool = test_pool();
        let created = create(&pool, "ops", &[], &[]).unwrap();
        let users = strings(&["@bob:example.com"]);

        // Act
        let updated = update(
            &pool,
            created.id,
            "ops2",
            &strings(&["!x:example.com"]),
            &users,
        )
        .unwrap()
        .unwrap();

        // Assert
        assert_eq!(updated.id, created.id);
        assert_eq!(updated.name, "ops2");
        assert_eq!(updated.allowed_rooms, strings(&["!x:example.com"]));
        assert_eq!(updated.allowed_users, users);
        assert_eq!(updated.created_at, created.created_at);
        assert_eq!(get_by_id(&pool, created.id).unwrap().unwrap().name, "ops2");
    }

    #[test]
    fn update_returns_none_when_missing() {
        let pool = test_pool();
        assert!(update(&pool, 42, "x", &[], &[]).unwrap().is_none());
    }

    #[test]
    fn update_to_an_existing_name_is_rejected() {
        // Arrange
        let pool = test_pool();
        create(&pool, "one", &[], &[]).unwrap();
        let two = create(&pool, "two", &[], &[]).unwrap();

        // Act
        let result = update(&pool, two.id, "one", &[], &[]);

        // Assert
        assert!(matches!(result, Err(AccessProfileError::DuplicateName(n)) if n == "one"));
    }

    #[test]
    fn delete_removes_the_profile() {
        // Arrange
        let pool = test_pool();
        let created = create(&pool, "ops", &[], &[]).unwrap();

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
    fn create_with_duplicate_name_is_rejected() {
        // Arrange
        let pool = test_pool();
        create(&pool, "ops", &[], &[]).unwrap();

        // Act
        let result = create(&pool, "ops", &[], &[]);

        // Assert
        assert!(matches!(result, Err(AccessProfileError::DuplicateName(n)) if n == "ops"));
    }
}
