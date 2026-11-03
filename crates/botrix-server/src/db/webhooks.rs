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

/// A notification claimed by the worker for delivery.
#[derive(Debug, Clone)]
pub struct PendingNotification {
    /// Row ID.
    pub id: i64,
    /// Logical destination name.
    pub destination: String,
    /// Fully rendered message.
    pub rendered_message: String,
    /// Number of attempts made before this claim.
    pub attempts: i64,
}

/// Atomically claim up to `limit` pending notifications, moving them to
/// `processing`, and return them.
///
/// The claim uses a single `UPDATE ... WHERE id IN (SELECT ...)` with
/// `RETURNING` so no two workers (or loop iterations) claim the same row.
pub async fn claim_pending(pool: &Pool, limit: i64) -> Result<Vec<PendingNotification>, DbError> {
    let rows: Vec<(i64, String, String, i64)> = sqlx::query_as(
        "UPDATE notifications
         SET status = 'processing'
         WHERE id IN (
             SELECT id FROM notifications
             WHERE status = 'pending'
             ORDER BY id
             LIMIT ?
         )
         RETURNING id, destination, rendered_message, attempts",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(id, destination, rendered_message, attempts)| PendingNotification {
                id,
                destination,
                rendered_message,
                attempts,
            },
        )
        .collect())
}

/// Mark a notification delivered, recording the delivery timestamp.
pub async fn mark_delivered(pool: &Pool, id: i64) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE notifications
         SET status = 'delivered', attempts = attempts + 1, delivered_at = ?
         WHERE id = ?",
    )
    .bind(now_unix())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Mark a notification as failed (no further retries), recording the error.
pub async fn mark_failed(pool: &Pool, id: i64, error: &str) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE notifications
         SET status = 'failed', attempts = attempts + 1, last_error = ?
         WHERE id = ?",
    )
    .bind(error)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Return a notification to `pending` for a later retry, incrementing its
/// attempt count and recording the last error.
pub async fn mark_for_retry(pool: &Pool, id: i64, error: &str) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE notifications
         SET status = 'pending', attempts = attempts + 1, last_error = ?
         WHERE id = ?",
    )
    .bind(error)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
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

    async fn status_of(pool: &Pool, id: i64) -> String {
        sqlx::query_scalar("SELECT status FROM notifications WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn attempts_of(pool: &Pool, id: i64) -> i64 {
        sqlx::query_scalar("SELECT attempts FROM notifications WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn claim_pending_moves_rows_to_processing() {
        // Arrange
        let pool = test_pool().await;
        let id1 = insert_notification(&pool, "wud", "containers", "a")
            .await
            .unwrap();
        let id2 = insert_notification(&pool, "wud", "containers", "b")
            .await
            .unwrap();

        // Act
        let claimed = claim_pending(&pool, 10).await.unwrap();

        // Assert: both claimed, both now processing.
        assert_eq!(claimed.len(), 2);
        assert_eq!(status_of(&pool, id1).await, "processing");
        assert_eq!(status_of(&pool, id2).await, "processing");
    }

    #[tokio::test]
    async fn claim_pending_does_not_reclaim_processing_rows() {
        let pool = test_pool().await;
        insert_notification(&pool, "wud", "containers", "a")
            .await
            .unwrap();

        // First claim takes the row.
        let first = claim_pending(&pool, 10).await.unwrap();
        assert_eq!(first.len(), 1);
        // Second claim finds nothing (the row is already processing).
        let second = claim_pending(&pool, 10).await.unwrap();
        assert!(second.is_empty());
    }

    #[tokio::test]
    async fn claim_pending_respects_limit() {
        let pool = test_pool().await;
        for _ in 0..3 {
            insert_notification(&pool, "wud", "containers", "x")
                .await
                .unwrap();
        }
        let claimed = claim_pending(&pool, 2).await.unwrap();
        assert_eq!(claimed.len(), 2);
    }

    #[tokio::test]
    async fn mark_delivered_sets_status_and_timestamp() {
        let pool = test_pool().await;
        let id = insert_notification(&pool, "wud", "containers", "a")
            .await
            .unwrap();
        claim_pending(&pool, 10).await.unwrap();

        mark_delivered(&pool, id).await.unwrap();

        assert_eq!(status_of(&pool, id).await, "delivered");
        assert_eq!(attempts_of(&pool, id).await, 1);
        let delivered_at: Option<i64> =
            sqlx::query_scalar("SELECT delivered_at FROM notifications WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(delivered_at.is_some());
    }

    #[tokio::test]
    async fn mark_failed_sets_status_and_error() {
        let pool = test_pool().await;
        let id = insert_notification(&pool, "wud", "containers", "a")
            .await
            .unwrap();

        mark_failed(&pool, id, "boom").await.unwrap();

        assert_eq!(status_of(&pool, id).await, "failed");
        assert_eq!(attempts_of(&pool, id).await, 1);
        let err: Option<String> =
            sqlx::query_scalar("SELECT last_error FROM notifications WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(err.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn mark_for_retry_returns_to_pending_and_increments_attempts() {
        let pool = test_pool().await;
        let id = insert_notification(&pool, "wud", "containers", "a")
            .await
            .unwrap();
        claim_pending(&pool, 10).await.unwrap();

        mark_for_retry(&pool, id, "temporary").await.unwrap();

        assert_eq!(status_of(&pool, id).await, "pending");
        assert_eq!(attempts_of(&pool, id).await, 1);
        // It can be claimed again.
        let claimed = claim_pending(&pool, 10).await.unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].attempts, 1);
    }
}
