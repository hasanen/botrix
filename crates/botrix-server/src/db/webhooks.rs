//! Persistence for webhook tokens and notifications.
//!
//! Token hashes live in `webhook_tokens` (one row per webhook identifier).
//! Accepted webhook payloads are rendered and inserted into `notifications`
//! with status `pending` for the background worker to deliver.

use super::{DbError, Pool};

/// Retention window for notifications, in seconds (30 days, §5.3).
pub const NOTIFICATION_RETENTION_SECS: i64 = 30 * 24 * 60 * 60;

/// Current Unix timestamp in seconds.
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Store (or replace) the token hash for a webhook identifier.
///
/// Upserts the `webhook_tokens` row, enabling it and recording `created_at`
/// on first insert.
pub async fn set_token_hash(
    pool: &Pool,
    webhook_id: &str,
    token_hash: &str,
) -> Result<(), DbError> {
    let now = now_unix();
    sqlx::query(
        "INSERT INTO webhook_tokens (webhook_id, token_hash, enabled, created_at, updated_at)
         VALUES (?, ?, 1, ?, ?)
         ON CONFLICT(webhook_id) DO UPDATE SET
            token_hash = excluded.token_hash,
            enabled = 1,
            updated_at = excluded.updated_at",
    )
    .bind(webhook_id)
    .bind(token_hash)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Revoke a webhook's token by clearing its hash and disabling the row.
///
/// Returns `true` if a token was present and cleared, `false` if there was
/// nothing to revoke.
pub async fn revoke_token(pool: &Pool, webhook_id: &str) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE webhook_tokens
         SET token_hash = NULL, enabled = 0, updated_at = ?
         WHERE webhook_id = ? AND token_hash IS NOT NULL",
    )
    .bind(now_unix())
    .bind(webhook_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Fetch the stored token hash for a webhook, if a token is set and enabled.
pub async fn get_token_hash(pool: &Pool, webhook_id: &str) -> Result<Option<String>, DbError> {
    let hash: Option<String> = sqlx::query_scalar(
        "SELECT token_hash FROM webhook_tokens
         WHERE webhook_id = ? AND enabled = 1 AND token_hash IS NOT NULL",
    )
    .bind(webhook_id)
    .fetch_optional(pool)
    .await?
    .flatten();
    Ok(hash)
}

/// Return whether a webhook has an active token set.
pub async fn has_token(pool: &Pool, webhook_id: &str) -> Result<bool, DbError> {
    Ok(get_token_hash(pool, webhook_id).await?.is_some())
}

/// Insert a rendered notification in the `pending` state and return its row ID.
pub async fn insert_notification(
    pool: &Pool,
    webhook_id: &str,
    destination: &str,
    rendered_message: &str,
) -> Result<i64, DbError> {
    let now = now_unix();
    let expires_at = now + NOTIFICATION_RETENTION_SECS;
    let result = sqlx::query(
        "INSERT INTO notifications
            (webhook_id, destination, rendered_message, status, attempts, created_at, expires_at)
         VALUES (?, ?, ?, 'pending', 0, ?, ?)",
    )
    .bind(webhook_id)
    .bind(destination)
    .bind(rendered_message)
    .bind(now)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
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

    #[tokio::test]
    async fn no_token_initially() {
        let pool = test_pool().await;
        assert!(!has_token(&pool, "wud").await.unwrap());
        assert_eq!(get_token_hash(&pool, "wud").await.unwrap(), None);
    }

    #[tokio::test]
    async fn set_then_get_token_hash() {
        // Arrange
        let pool = test_pool().await;
        // Act
        set_token_hash(&pool, "wud", "deadbeef").await.unwrap();
        // Assert
        assert!(has_token(&pool, "wud").await.unwrap());
        assert_eq!(
            get_token_hash(&pool, "wud").await.unwrap(),
            Some("deadbeef".to_owned())
        );
    }

    #[tokio::test]
    async fn setting_token_again_replaces_it() {
        let pool = test_pool().await;
        set_token_hash(&pool, "wud", "first").await.unwrap();
        set_token_hash(&pool, "wud", "second").await.unwrap();
        assert_eq!(
            get_token_hash(&pool, "wud").await.unwrap(),
            Some("second".to_owned())
        );
        // Still a single row.
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM webhook_tokens")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn revoke_clears_token() {
        // Arrange
        let pool = test_pool().await;
        set_token_hash(&pool, "wud", "deadbeef").await.unwrap();

        // Act
        let revoked = revoke_token(&pool, "wud").await.unwrap();

        // Assert
        assert!(revoked);
        assert!(!has_token(&pool, "wud").await.unwrap());
        assert_eq!(get_token_hash(&pool, "wud").await.unwrap(), None);
    }

    #[tokio::test]
    async fn revoke_without_token_returns_false() {
        let pool = test_pool().await;
        assert!(!revoke_token(&pool, "wud").await.unwrap());
    }

    #[tokio::test]
    async fn insert_notification_creates_pending_row() {
        // Arrange
        let pool = test_pool().await;

        // Act
        let id = insert_notification(&pool, "wud", "containers", "hello")
            .await
            .unwrap();

        // Assert
        assert!(id > 0);
        let (status, dest, msg): (String, String, String) = sqlx::query_as(
            "SELECT status, destination, rendered_message FROM notifications WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(status, "pending");
        assert_eq!(dest, "containers");
        assert_eq!(msg, "hello");
    }

    #[tokio::test]
    async fn insert_notification_sets_expiry_30_days_out() {
        let pool = test_pool().await;
        let id = insert_notification(&pool, "wud", "containers", "hello")
            .await
            .unwrap();
        let (created, expires): (i64, i64) =
            sqlx::query_as("SELECT created_at, expires_at FROM notifications WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(expires - created, NOTIFICATION_RETENTION_SECS);
    }
}
