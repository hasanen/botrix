//! Database maintenance: crash recovery and history cleanup.
//!
//! - **Crash recovery** ([`recover_stuck_notifications`]): on startup, any
//!   notification left in the `processing` state by a crashed previous run is
//!   reset to `pending` so the worker can retry it (at-least-once semantics;
//!   see §11.4 of the implementation plan).
//! - **History cleanup** ([`cleanup_expired`] / [`spawn_cleanup_task`]): a
//!   periodic task deletes `notifications` past their `expires_at` and
//!   `log_events` older than the retention window (§5.3).

use std::time::Duration;

use super::{DbError, Pool};

/// Retention window for log events, in seconds (30 days).
pub const LOG_RETENTION_SECS: i64 = 30 * 24 * 60 * 60;

/// Interval between history cleanup runs (24 hours).
const CLEANUP_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Number of rows affected by a maintenance operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupStats {
    /// Expired notification rows deleted.
    pub notifications_deleted: u64,
    /// Expired log event rows deleted.
    pub log_events_deleted: u64,
}

/// Reset notifications stuck in the `processing` state back to `pending`.
///
/// Returns the number of rows updated. Intended to be called once at startup,
/// before any delivery worker begins polling.
pub async fn recover_stuck_notifications(pool: &Pool) -> Result<u64, DbError> {
    let result = sqlx::query(
        "UPDATE notifications
         SET status = 'pending',
             last_error = 'Recovered from processing state on startup'
         WHERE status = 'processing'",
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

/// Delete expired notifications and old log events.
///
/// `now` is the current Unix timestamp in seconds. Notifications are deleted
/// when `expires_at < now`; log events are deleted when
/// `created_at < now - LOG_RETENTION_SECS`.
pub async fn cleanup_expired(pool: &Pool, now: i64) -> Result<CleanupStats, DbError> {
    let notifications = sqlx::query("DELETE FROM notifications WHERE expires_at < ?")
        .bind(now)
        .execute(pool)
        .await?
        .rows_affected();

    let log_cutoff = now - LOG_RETENTION_SECS;
    let log_events = sqlx::query("DELETE FROM log_events WHERE created_at < ?")
        .bind(log_cutoff)
        .execute(pool)
        .await?
        .rows_affected();

    Ok(CleanupStats {
        notifications_deleted: notifications,
        log_events_deleted: log_events,
    })
}

/// Spawn a background task that runs [`cleanup_expired`] once immediately and
/// then every 24 hours for the lifetime of the process.
pub fn spawn_cleanup_task(pool: Pool) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CLEANUP_INTERVAL);
        loop {
            interval.tick().await;
            match cleanup_expired(&pool, now_unix()).await {
                Ok(stats) => {
                    if stats.notifications_deleted > 0 || stats.log_events_deleted > 0 {
                        tracing::info!(
                            notifications = stats.notifications_deleted,
                            log_events = stats.log_events_deleted,
                            "history cleanup removed expired rows"
                        );
                    }
                }
                Err(err) => {
                    tracing::error!(error = %err, "history cleanup failed");
                }
            }
        }
    })
}

/// Current Unix timestamp in seconds.
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create an in-memory SQLite pool with migrations applied.
    ///
    /// The pool is pinned to a single connection so every query in a test
    /// targets the same in-memory database (each SQLite `:memory:` connection
    /// otherwise gets its own isolated database).
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

    async fn insert_notification(pool: &Pool, status: &str, expires_at: i64) -> i64 {
        let result = sqlx::query(
            "INSERT INTO notifications
                (webhook_id, destination, rendered_message, status, created_at, expires_at)
             VALUES ('hook', 'dest', 'msg', ?, 0, ?)",
        )
        .bind(status)
        .bind(expires_at)
        .execute(pool)
        .await
        .expect("insert notification");
        result.last_insert_rowid()
    }

    async fn status_of(pool: &Pool, id: i64) -> String {
        sqlx::query_scalar::<_, String>("SELECT status FROM notifications WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .expect("fetch status")
    }

    #[tokio::test]
    async fn recovers_processing_notifications() {
        let pool = test_pool().await;
        let stuck = insert_notification(&pool, "processing", 9_999_999_999).await;
        let delivered = insert_notification(&pool, "delivered", 9_999_999_999).await;

        let count = recover_stuck_notifications(&pool).await.unwrap();

        assert_eq!(count, 1);
        assert_eq!(status_of(&pool, stuck).await, "pending");
        // Delivered rows are untouched.
        assert_eq!(status_of(&pool, delivered).await, "delivered");
    }

    #[tokio::test]
    async fn recovery_is_noop_when_nothing_stuck() {
        let pool = test_pool().await;
        insert_notification(&pool, "pending", 9_999_999_999).await;

        let count = recover_stuck_notifications(&pool).await.unwrap();

        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn cleanup_deletes_expired_notifications_only() {
        let pool = test_pool().await;
        let now = 1_000_000;
        let expired = insert_notification(&pool, "delivered", now - 1).await;
        let fresh = insert_notification(&pool, "delivered", now + 100).await;

        let stats = cleanup_expired(&pool, now).await.unwrap();

        assert_eq!(stats.notifications_deleted, 1);
        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notifications WHERE id = ?")
            .bind(expired)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(remaining, 0, "expired notification should be deleted");
        let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notifications WHERE id = ?")
            .bind(fresh)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(kept, 1, "fresh notification should be kept");
    }

    #[tokio::test]
    async fn cleanup_deletes_old_log_events() {
        let pool = test_pool().await;
        let now = LOG_RETENTION_SECS + 1_000;

        // One old, one recent.
        sqlx::query(
            "INSERT INTO log_events (level, component, message, created_at)
             VALUES ('INFO', 'test', 'old', ?)",
        )
        .bind(now - LOG_RETENTION_SECS - 1)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO log_events (level, component, message, created_at)
             VALUES ('INFO', 'test', 'recent', ?)",
        )
        .bind(now - 10)
        .execute(&pool)
        .await
        .unwrap();

        let stats = cleanup_expired(&pool, now).await.unwrap();

        assert_eq!(stats.log_events_deleted, 1);
        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM log_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(remaining, 1, "only the recent log event should remain");
    }

    #[tokio::test]
    async fn migrations_are_idempotent() {
        let pool = test_pool().await;
        // Running migrations a second time on the same pool must be a no-op.
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("re-running migrations should succeed");
    }

    #[tokio::test]
    async fn matrix_state_singleton_exists() {
        let pool = test_pool().await;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM matrix_state")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "migration should seed the matrix_state singleton");
        let status: String = sqlx::query_scalar("SELECT status FROM matrix_state WHERE id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(status, "disconnected");
    }
}
