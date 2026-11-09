//! Admin account management: creation, login verification, and password changes.
//!
//! Passwords are hashed with Argon2id. Only the PHC-string hash is persisted —
//! never the plaintext (§16.3).

use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use thiserror::Error;

use super::{now_unix, DbError, Pool};

/// Errors that can occur during admin authentication operations.
#[derive(Debug, Error)]
pub enum AuthError {
    /// A database operation failed.
    #[error("database error: {0}")]
    Database(#[from] DbError),
    /// Password hashing or verification failed internally.
    #[error("password hashing failed: {0}")]
    Hash(String),
    /// Setup was attempted when an admin account already exists.
    #[error("an admin account already exists")]
    AlreadyInitialized,
    /// Login or password-change verification failed.
    #[error("invalid credentials")]
    InvalidCredentials,
}

impl From<rusqlite::Error> for AuthError {
    fn from(e: rusqlite::Error) -> Self {
        AuthError::Database(DbError::from(e))
    }
}

impl From<r2d2::Error> for AuthError {
    fn from(e: r2d2::Error) -> Self {
        AuthError::Database(DbError::from(e))
    }
}

/// Hash a plaintext password with Argon2id, returning a PHC-format string.
fn hash_password(password: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| AuthError::Hash(e.to_string()))
}

/// Verify a plaintext password against a stored PHC hash.
fn verify_password(password: &str, phc_hash: &str) -> Result<bool, AuthError> {
    let parsed = PasswordHash::new(phc_hash).map_err(|e| AuthError::Hash(e.to_string()))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

/// Return whether any admin account exists.
pub fn admin_exists(pool: &Pool) -> Result<bool, AuthError> {
    let conn = pool.get()?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM admins", [], |r| r.get(0))?;
    Ok(count > 0)
}

/// Create the initial admin account; fails if one already exists.
pub fn create_admin(pool: &Pool, username: &str, password: &str) -> Result<(), AuthError> {
    if admin_exists(pool)? {
        return Err(AuthError::AlreadyInitialized);
    }
    let hash = hash_password(password)?;
    let now = now_unix();
    let conn = pool.get()?;
    conn.execute(
        "INSERT INTO admins (username, password_hash, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?3)",
        rusqlite::params![username, hash, now],
    )?;
    Ok(())
}

/// Verify login credentials; returns the username on success.
pub fn verify_login(pool: &Pool, username: &str, password: &str) -> Result<String, AuthError> {
    let conn = pool.get()?;
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT username, password_hash FROM admins WHERE username = ?1",
            rusqlite::params![username],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional_row()?;

    match row {
        Some((db_username, hash)) if verify_password(password, &hash)? => Ok(db_username),
        Some(_) => Err(AuthError::InvalidCredentials),
        None => {
            // Unknown user: spend the same time hashing against a throwaway
            // hash, so response time does not reveal which usernames exist.
            let _ = verify_password(password, dummy_hash());
            Err(AuthError::InvalidCredentials)
        }
    }
}

/// A valid Argon2 hash (of a random, discarded password) generated once per
/// process with the same parameters as real hashes, used to equalize timing.
fn dummy_hash() -> &'static str {
    static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    DUMMY.get_or_init(|| {
        // Falling back to an empty string only skips the timing equalization
        // (verification of an unparsable hash fails fast); it never grants access.
        hash_password(&random_throwaway_secret()).unwrap_or_default()
    })
}

/// A random throwaway value for [`dummy_hash`]; never stored or compared.
fn random_throwaway_secret() -> String {
    let mut bytes = [0u8; 16];
    rand::fill(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Change an admin's password after verifying the current one.
pub fn change_password(
    pool: &Pool,
    username: &str,
    current_password: &str,
    new_password: &str,
) -> Result<(), AuthError> {
    verify_login(pool, username, current_password)?;
    let new_hash = hash_password(new_password)?;
    let now = now_unix();
    let conn = pool.get()?;
    conn.execute(
        "UPDATE admins SET password_hash = ?1, updated_at = ?2 WHERE username = ?3",
        rusqlite::params![new_hash, now, username],
    )?;
    Ok(())
}

/// Helper: turn a `QueryReturnedNoRows` error into `Ok(None)`.
trait OptionalRow<T> {
    fn optional_row(self) -> Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalRow<T> for Result<T, rusqlite::Error> {
    fn optional_row(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_pool;

    #[test]
    fn hash_then_verify_roundtrips() {
        let password = "correct horse battery staple";
        let hash = hash_password(password).unwrap();
        assert!(verify_password(password, &hash).unwrap());
        assert!(!verify_password("wrong password", &hash).unwrap());
    }

    #[test]
    fn hash_is_not_plaintext() {
        let hash = hash_password("s3cr3t").unwrap();
        assert!(!hash.contains("s3cr3t"));
        assert!(hash.starts_with("$argon2id$"));
    }

    #[test]
    fn unknown_user_costs_about_as_much_as_a_wrong_password() {
        // Arrange: if an unknown username returned without hashing, response
        // time would reveal which usernames exist.
        let pool = test_pool();
        create_admin(&pool, "alice", "hunter2pass").unwrap();
        let time_min = |user: &str| {
            (0..3)
                .map(|_| {
                    let start = std::time::Instant::now();
                    let _ = verify_login(&pool, user, "wrong-password");
                    start.elapsed()
                })
                .min()
                .unwrap()
        };
        let _ = verify_login(&pool, "alice", "warm-up");

        // Act
        let known_user = time_min("alice");
        let unknown_user = time_min("nobody");

        // Assert: same order of magnitude (generous bound to avoid flakiness;
        // the unfixed code is several hundred times faster).
        assert!(
            unknown_user * 2 >= known_user,
            "unknown user took {unknown_user:?} vs {known_user:?} for a known user"
        );
    }

    #[test]
    fn unknown_user_and_wrong_password_return_the_same_error() {
        let pool = test_pool();
        create_admin(&pool, "alice", "hunter2pass").unwrap();
        assert!(matches!(
            verify_login(&pool, "nobody", "x"),
            Err(AuthError::InvalidCredentials)
        ));
        assert!(matches!(
            verify_login(&pool, "alice", "x"),
            Err(AuthError::InvalidCredentials)
        ));
    }

    #[test]
    fn no_admin_initially() {
        let pool = test_pool();
        assert!(!admin_exists(&pool).unwrap());
    }

    #[test]
    fn create_admin_then_login_succeeds() {
        let pool = test_pool();
        create_admin(&pool, "alice", "hunter2").unwrap();
        assert!(admin_exists(&pool).unwrap());
        assert_eq!(verify_login(&pool, "alice", "hunter2").unwrap(), "alice");
    }

    #[test]
    fn login_fails_with_wrong_password() {
        let pool = test_pool();
        create_admin(&pool, "alice", "hunter2").unwrap();
        assert!(matches!(
            verify_login(&pool, "alice", "wrong"),
            Err(AuthError::InvalidCredentials)
        ));
    }

    #[test]
    fn login_fails_for_unknown_user() {
        let pool = test_pool();
        create_admin(&pool, "alice", "hunter2").unwrap();
        assert!(matches!(
            verify_login(&pool, "bob", "hunter2"),
            Err(AuthError::InvalidCredentials)
        ));
    }

    #[test]
    fn second_create_admin_is_rejected() {
        let pool = test_pool();
        create_admin(&pool, "alice", "hunter2").unwrap();
        assert!(matches!(
            create_admin(&pool, "bob", "sesame"),
            Err(AuthError::AlreadyInitialized)
        ));
    }

    #[test]
    fn change_password_updates_and_old_password_stops_working() {
        let pool = test_pool();
        create_admin(&pool, "alice", "old-pass").unwrap();
        change_password(&pool, "alice", "old-pass", "new-pass").unwrap();
        assert!(verify_login(&pool, "alice", "new-pass").is_ok());
        assert!(matches!(
            verify_login(&pool, "alice", "old-pass"),
            Err(AuthError::InvalidCredentials)
        ));
    }

    #[test]
    fn change_password_rejects_wrong_current_password() {
        let pool = test_pool();
        create_admin(&pool, "alice", "old-pass").unwrap();
        assert!(matches!(
            change_password(&pool, "alice", "WRONG", "new-pass"),
            Err(AuthError::InvalidCredentials)
        ));
        assert!(verify_login(&pool, "alice", "old-pass").is_ok());
    }
}
