//! Admin account management: creation, login verification, and password changes.
//!
//! Passwords are hashed with Argon2id using the `argon2` crate's recommended
//! default parameters. Only the PHC-string hash is persisted — never the
//! plaintext. See §16.3 of the implementation plan.

use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use thiserror::Error;

use super::{DbError, Pool};

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
    /// Login or password-change verification failed (wrong username or password).
    #[error("invalid credentials")]
    InvalidCredentials,
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
///
/// Returns `Ok(true)` on match, `Ok(false)` on mismatch, and an error only if
/// the stored hash is malformed.
fn verify_password(password: &str, phc_hash: &str) -> Result<bool, AuthError> {
    let parsed = PasswordHash::new(phc_hash).map_err(|e| AuthError::Hash(e.to_string()))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

/// Current Unix timestamp in seconds.
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Return whether any admin account exists.
pub async fn admin_exists(pool: &Pool) -> Result<bool, AuthError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admins")
        .fetch_one(pool)
        .await
        .map_err(DbError::from)?;
    Ok(count > 0)
}

/// Create the initial admin account.
///
/// Fails with [`AuthError::AlreadyInitialized`] if an admin already exists, so
/// the first-run setup endpoint can only be used once.
pub async fn create_admin(pool: &Pool, username: &str, password: &str) -> Result<(), AuthError> {
    if admin_exists(pool).await? {
        return Err(AuthError::AlreadyInitialized);
    }

    let hash = hash_password(password)?;
    let now = now_unix();
    sqlx::query(
        "INSERT INTO admins (username, password_hash, created_at, updated_at)
         VALUES (?, ?, ?, ?)",
    )
    .bind(username)
    .bind(&hash)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .map_err(DbError::from)?;

    Ok(())
}

/// Verify login credentials.
///
/// Returns the admin's username on success, or [`AuthError::InvalidCredentials`]
/// if the username is unknown or the password does not match.
pub async fn verify_login(
    pool: &Pool,
    username: &str,
    password: &str,
) -> Result<String, AuthError> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT username, password_hash FROM admins WHERE username = ?")
            .bind(username)
            .fetch_optional(pool)
            .await
            .map_err(DbError::from)?;

    match row {
        Some((db_username, hash)) if verify_password(password, &hash)? => Ok(db_username),
        _ => Err(AuthError::InvalidCredentials),
    }
}

/// Change an admin's password after verifying the current one.
///
/// Returns [`AuthError::InvalidCredentials`] if the current password is wrong.
pub async fn change_password(
    pool: &Pool,
    username: &str,
    current_password: &str,
    new_password: &str,
) -> Result<(), AuthError> {
    // Verify the current password first.
    verify_login(pool, username, current_password).await?;

    let new_hash = hash_password(new_password)?;
    let now = now_unix();
    sqlx::query("UPDATE admins SET password_hash = ?, updated_at = ? WHERE username = ?")
        .bind(&new_hash)
        .bind(now)
        .bind(username)
        .execute(pool)
        .await
        .map_err(DbError::from)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> Pool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect in-memory sqlite");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations");
        pool
    }

    #[test]
    fn hash_then_verify_roundtrips() {
        // Arrange
        let password = "correct horse battery staple";
        // Act
        let hash = hash_password(password).unwrap();
        // Assert
        assert!(verify_password(password, &hash).unwrap());
        assert!(!verify_password("wrong password", &hash).unwrap());
    }

    #[test]
    fn hash_is_not_plaintext() {
        let password = "s3cr3t";
        let hash = hash_password(password).unwrap();
        assert!(!hash.contains(password));
        // PHC strings for Argon2id start with the algorithm identifier.
        assert!(hash.starts_with("$argon2id$"));
    }

    #[tokio::test]
    async fn no_admin_initially() {
        let pool = test_pool().await;
        assert!(!admin_exists(&pool).await.unwrap());
    }

    #[tokio::test]
    async fn create_admin_then_login_succeeds() {
        // Arrange
        let pool = test_pool().await;
        create_admin(&pool, "alice", "hunter2").await.unwrap();

        // Act / Assert
        assert!(admin_exists(&pool).await.unwrap());
        let username = verify_login(&pool, "alice", "hunter2").await.unwrap();
        assert_eq!(username, "alice");
    }

    #[tokio::test]
    async fn login_fails_with_wrong_password() {
        let pool = test_pool().await;
        create_admin(&pool, "alice", "hunter2").await.unwrap();

        let result = verify_login(&pool, "alice", "wrong").await;
        assert!(matches!(result, Err(AuthError::InvalidCredentials)));
    }

    #[tokio::test]
    async fn login_fails_for_unknown_user() {
        let pool = test_pool().await;
        create_admin(&pool, "alice", "hunter2").await.unwrap();

        let result = verify_login(&pool, "bob", "hunter2").await;
        assert!(matches!(result, Err(AuthError::InvalidCredentials)));
    }

    #[tokio::test]
    async fn second_create_admin_is_rejected() {
        let pool = test_pool().await;
        create_admin(&pool, "alice", "hunter2").await.unwrap();

        let result = create_admin(&pool, "bob", "sesame").await;
        assert!(matches!(result, Err(AuthError::AlreadyInitialized)));
    }

    #[tokio::test]
    async fn change_password_updates_and_old_password_stops_working() {
        // Arrange
        let pool = test_pool().await;
        create_admin(&pool, "alice", "old-pass").await.unwrap();

        // Act
        change_password(&pool, "alice", "old-pass", "new-pass")
            .await
            .unwrap();

        // Assert
        assert!(verify_login(&pool, "alice", "new-pass").await.is_ok());
        assert!(matches!(
            verify_login(&pool, "alice", "old-pass").await,
            Err(AuthError::InvalidCredentials)
        ));
    }

    #[tokio::test]
    async fn change_password_rejects_wrong_current_password() {
        let pool = test_pool().await;
        create_admin(&pool, "alice", "old-pass").await.unwrap();

        let result = change_password(&pool, "alice", "WRONG", "new-pass").await;
        assert!(matches!(result, Err(AuthError::InvalidCredentials)));
        // Original password still works.
        assert!(verify_login(&pool, "alice", "old-pass").await.is_ok());
    }
}
