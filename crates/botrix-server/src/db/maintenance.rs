//! Database maintenance: crash recovery and history cleanup.

use std::time::Duration;

use super::{now_unix, DbError, Pool};

/// Retention window for log events, in seconds (30 days).
pub const LOG_RETENTION_SECS: i64 = 30 * 24 * 60 * 60;

/// Interval between history cleanup runs (24 hours).
const CLEANUP_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Rows affected by a cleanup run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupStats {
    /// Expired notification rows deleted.
    pub notifications_deleted: u64,
    /// Expired log event rows deleted.
    pub log_events_deleted: u64,
}

/// Reset notifications stuck in `processing` back to `pending`.
pub fn recover_stuck_notifications(pool: &Pool) -> Result<u64, DbError> {
    let conn = pool.get()?;
    let n = conn.execute(
        "UPDATE notifications
         SET status = 'pending',
             last_error = 'Recovered from processing state on startup'
         WHERE status = 'processing'",
        [],
    )?;
    Ok(n as u64)
}

/// Delete expired notifications and old log events.
pub fn cleanup_expired(pool: &Pool, now: i64) -> Result<CleanupStats, DbError> {
    let conn = pool.get()?;
    let notifications = conn.execute(
        "DELETE FROM notifications WHERE expires_at < ?1",
        rusqlite::params![now],
    )?;
    let log_cutoff = now - LOG_RETENTION_SECS;
    let log_events = conn.execute(
        "DELETE FROM log_events WHERE created_at < ?1",
        rusqlite::params![log_cutoff],
    )?;
    Ok(CleanupStats {
        notifications_deleted: notifications as u64,
        log_events_deleted: log_events as u64,
    })
}

/// Spawn a background task running [`cleanup_expired`] immediately and daily.
pub fn spawn_cleanup_task(pool: Pool) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CLEANUP_INTERVAL);
        loop {
            interval.tick().await;
            let pool_for_log = pool.clone();
            let pool = pool.clone();
            let result =
                tokio::task::spawn_blocking(move || cleanup_expired(&pool, now_unix())).await;
            match result {
                Ok(Ok(stats)) => {
                    if stats.notifications_deleted > 0 || stats.log_events_deleted > 0 {
                        tracing::info!(
                            notifications = stats.notifications_deleted,
                            log_events = stats.log_events_deleted,
                            "history cleanup removed expired rows"
                        );
                        crate::db::logs::record(
                            &pool_for_log,
                            botrix_common::types::LogLevel::Info,
                            crate::db::logs::Component::System,
                            "history cleanup removed expired rows",
                            Some(&format!(
                                "{} notification(s), {} log event(s)",
                                stats.notifications_deleted, stats.log_events_deleted
                            )),
                        );
                    }
                }
                Ok(Err(err)) => tracing::error!(error = %err, "history cleanup failed"),
                Err(err) => tracing::error!(error = %err, "history cleanup task panicked"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_pool;

    fn insert_notification(pool: &Pool, status: &str, expires_at: i64) -> i64 {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO notifications
                (webhook_id, destination, rendered_message, status, created_at, expires_at)
             VALUES ('hook', 'dest', 'msg', ?1, 0, ?2)",
            rusqlite::params![status, expires_at],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn status_of(pool: &Pool, id: i64) -> String {
        pool.get()
            .unwrap()
            .query_row(
                "SELECT status FROM notifications WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[test]
    fn recovers_processing_notifications() {
        let pool = test_pool();
        let stuck = insert_notification(&pool, "processing", 9_999_999_999);
        let delivered = insert_notification(&pool, "delivered", 9_999_999_999);
        let n = recover_stuck_notifications(&pool).unwrap();
        assert_eq!(n, 1);
        assert_eq!(status_of(&pool, stuck), "pending");
        assert_eq!(status_of(&pool, delivered), "delivered");
    }

    #[test]
    fn recovery_is_noop_when_nothing_stuck() {
        let pool = test_pool();
        insert_notification(&pool, "pending", 9_999_999_999);
        assert_eq!(recover_stuck_notifications(&pool).unwrap(), 0);
    }

    #[test]
    fn cleanup_deletes_expired_notifications_only() {
        let pool = test_pool();
        let now = 1_000_000;
        let expired = insert_notification(&pool, "delivered", now - 1);
        let fresh = insert_notification(&pool, "delivered", now + 100);
        let stats = cleanup_expired(&pool, now).unwrap();
        assert_eq!(stats.notifications_deleted, 1);
        let conn = pool.get().unwrap();
        let gone: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM notifications WHERE id = ?1",
                rusqlite::params![expired],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(gone, 0);
        let kept: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM notifications WHERE id = ?1",
                rusqlite::params![fresh],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept, 1);
    }

    #[test]
    fn cleanup_deletes_old_log_events() {
        let pool = test_pool();
        let now = LOG_RETENTION_SECS + 1_000;
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO log_events (level, component, message, created_at)
             VALUES ('INFO', 'test', 'old', ?1)",
            rusqlite::params![now - LOG_RETENTION_SECS - 1],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO log_events (level, component, message, created_at)
             VALUES ('INFO', 'test', 'recent', ?1)",
            rusqlite::params![now - 10],
        )
        .unwrap();
        drop(conn);
        let stats = cleanup_expired(&pool, now).unwrap();
        assert_eq!(stats.log_events_deleted, 1);
        let remaining: i64 = pool
            .get()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM log_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn matrix_state_singleton_exists() {
        let pool = test_pool();
        let conn = pool.get().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM matrix_state", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        let status: String = conn
            .query_row("SELECT status FROM matrix_state WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(status, "disconnected");
    }
}
