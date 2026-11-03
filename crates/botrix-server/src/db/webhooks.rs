//! Persistence for webhook tokens and notifications.

use super::{now_unix, DbError, Pool};

/// Retention window for notifications, in seconds (30 days, §5.3).
pub const NOTIFICATION_RETENTION_SECS: i64 = 30 * 24 * 60 * 60;

/// Store (or replace) the token hash for a webhook identifier.
pub fn set_token_hash(pool: &Pool, webhook_id: &str, token_hash: &str) -> Result<(), DbError> {
    let now = now_unix();
    let conn = pool.get()?;
    conn.execute(
        "INSERT INTO webhook_tokens (webhook_id, token_hash, enabled, created_at, updated_at)
         VALUES (?1, ?2, 1, ?3, ?3)
         ON CONFLICT(webhook_id) DO UPDATE SET
            token_hash = excluded.token_hash,
            enabled = 1,
            updated_at = excluded.updated_at",
        rusqlite::params![webhook_id, token_hash, now],
    )?;
    Ok(())
}

/// Revoke a webhook's token; returns whether a token was present.
pub fn revoke_token(pool: &Pool, webhook_id: &str) -> Result<bool, DbError> {
    let conn = pool.get()?;
    let n = conn.execute(
        "UPDATE webhook_tokens
         SET token_hash = NULL, enabled = 0, updated_at = ?1
         WHERE webhook_id = ?2 AND token_hash IS NOT NULL",
        rusqlite::params![now_unix(), webhook_id],
    )?;
    Ok(n > 0)
}

/// Fetch the stored token hash for an enabled webhook with a token set.
pub fn get_token_hash(pool: &Pool, webhook_id: &str) -> Result<Option<String>, DbError> {
    let conn = pool.get()?;
    let hash: Option<String> = match conn.query_row(
        "SELECT token_hash FROM webhook_tokens
         WHERE webhook_id = ?1 AND enabled = 1 AND token_hash IS NOT NULL",
        rusqlite::params![webhook_id],
        |r| r.get(0),
    ) {
        Ok(v) => Some(v),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(e) => return Err(e.into()),
    };
    Ok(hash)
}

/// Return whether a webhook has an active token set.
pub fn has_token(pool: &Pool, webhook_id: &str) -> Result<bool, DbError> {
    Ok(get_token_hash(pool, webhook_id)?.is_some())
}

/// Insert a rendered notification as `pending`; returns its row ID.
pub fn insert_notification(
    pool: &Pool,
    webhook_id: &str,
    destination: &str,
    rendered_message: &str,
) -> Result<i64, DbError> {
    let now = now_unix();
    let expires_at = now + NOTIFICATION_RETENTION_SECS;
    let conn = pool.get()?;
    conn.execute(
        "INSERT INTO notifications
            (webhook_id, destination, rendered_message, status, attempts, created_at, expires_at)
         VALUES (?1, ?2, ?3, 'pending', 0, ?4, ?5)",
        rusqlite::params![webhook_id, destination, rendered_message, now, expires_at],
    )?;
    Ok(conn.last_insert_rowid())
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

/// Atomically claim up to `limit` pending notifications → `processing`.
pub fn claim_pending(pool: &Pool, limit: i64) -> Result<Vec<PendingNotification>, DbError> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "UPDATE notifications
         SET status = 'processing'
         WHERE id IN (
             SELECT id FROM notifications WHERE status = 'pending' ORDER BY id LIMIT ?1
         )
         RETURNING id, destination, rendered_message, attempts",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![limit], |r| {
            Ok(PendingNotification {
                id: r.get(0)?,
                destination: r.get(1)?,
                rendered_message: r.get(2)?,
                attempts: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Mark a notification delivered.
pub fn mark_delivered(pool: &Pool, id: i64) -> Result<(), DbError> {
    let conn = pool.get()?;
    conn.execute(
        "UPDATE notifications
         SET status = 'delivered', attempts = attempts + 1, delivered_at = ?1
         WHERE id = ?2",
        rusqlite::params![now_unix(), id],
    )?;
    Ok(())
}

/// Mark a notification failed (no further retries).
pub fn mark_failed(pool: &Pool, id: i64, error: &str) -> Result<(), DbError> {
    let conn = pool.get()?;
    conn.execute(
        "UPDATE notifications
         SET status = 'failed', attempts = attempts + 1, last_error = ?1
         WHERE id = ?2",
        rusqlite::params![error, id],
    )?;
    Ok(())
}

/// Return a notification to `pending` for retry, incrementing attempts.
pub fn mark_for_retry(pool: &Pool, id: i64, error: &str) -> Result<(), DbError> {
    let conn = pool.get()?;
    conn.execute(
        "UPDATE notifications
         SET status = 'pending', attempts = attempts + 1, last_error = ?1
         WHERE id = ?2",
        rusqlite::params![error, id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_pool;

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

    fn attempts_of(pool: &Pool, id: i64) -> i64 {
        pool.get()
            .unwrap()
            .query_row(
                "SELECT attempts FROM notifications WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[test]
    fn no_token_initially() {
        let pool = test_pool();
        assert!(!has_token(&pool, "wud").unwrap());
        assert_eq!(get_token_hash(&pool, "wud").unwrap(), None);
    }

    #[test]
    fn set_then_get_token_hash() {
        let pool = test_pool();
        set_token_hash(&pool, "wud", "deadbeef").unwrap();
        assert!(has_token(&pool, "wud").unwrap());
        assert_eq!(
            get_token_hash(&pool, "wud").unwrap(),
            Some("deadbeef".to_owned())
        );
    }

    #[test]
    fn setting_token_again_replaces_it() {
        let pool = test_pool();
        set_token_hash(&pool, "wud", "first").unwrap();
        set_token_hash(&pool, "wud", "second").unwrap();
        assert_eq!(
            get_token_hash(&pool, "wud").unwrap(),
            Some("second".to_owned())
        );
        let count: i64 = pool
            .get()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM webhook_tokens", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn revoke_clears_token() {
        let pool = test_pool();
        set_token_hash(&pool, "wud", "deadbeef").unwrap();
        assert!(revoke_token(&pool, "wud").unwrap());
        assert!(!has_token(&pool, "wud").unwrap());
        assert_eq!(get_token_hash(&pool, "wud").unwrap(), None);
    }

    #[test]
    fn revoke_without_token_returns_false() {
        let pool = test_pool();
        assert!(!revoke_token(&pool, "wud").unwrap());
    }

    #[test]
    fn insert_notification_creates_pending_row() {
        let pool = test_pool();
        let id = insert_notification(&pool, "wud", "containers", "hello").unwrap();
        assert!(id > 0);
        assert_eq!(status_of(&pool, id), "pending");
    }

    #[test]
    fn insert_notification_sets_expiry_30_days_out() {
        let pool = test_pool();
        let id = insert_notification(&pool, "wud", "containers", "hello").unwrap();
        let (created, expires): (i64, i64) = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT created_at, expires_at FROM notifications WHERE id = ?1",
                rusqlite::params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(expires - created, NOTIFICATION_RETENTION_SECS);
    }

    #[test]
    fn claim_pending_moves_rows_to_processing() {
        let pool = test_pool();
        let id1 = insert_notification(&pool, "wud", "containers", "a").unwrap();
        let id2 = insert_notification(&pool, "wud", "containers", "b").unwrap();
        let claimed = claim_pending(&pool, 10).unwrap();
        assert_eq!(claimed.len(), 2);
        assert_eq!(status_of(&pool, id1), "processing");
        assert_eq!(status_of(&pool, id2), "processing");
    }

    #[test]
    fn claim_pending_does_not_reclaim_processing_rows() {
        let pool = test_pool();
        insert_notification(&pool, "wud", "containers", "a").unwrap();
        assert_eq!(claim_pending(&pool, 10).unwrap().len(), 1);
        assert!(claim_pending(&pool, 10).unwrap().is_empty());
    }

    #[test]
    fn claim_pending_respects_limit() {
        let pool = test_pool();
        for _ in 0..3 {
            insert_notification(&pool, "wud", "containers", "x").unwrap();
        }
        assert_eq!(claim_pending(&pool, 2).unwrap().len(), 2);
    }

    #[test]
    fn mark_delivered_sets_status_and_timestamp() {
        let pool = test_pool();
        let id = insert_notification(&pool, "wud", "containers", "a").unwrap();
        claim_pending(&pool, 10).unwrap();
        mark_delivered(&pool, id).unwrap();
        assert_eq!(status_of(&pool, id), "delivered");
        assert_eq!(attempts_of(&pool, id), 1);
        let delivered_at: Option<i64> = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT delivered_at FROM notifications WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(delivered_at.is_some());
    }

    #[test]
    fn mark_failed_sets_status_and_error() {
        let pool = test_pool();
        let id = insert_notification(&pool, "wud", "containers", "a").unwrap();
        mark_failed(&pool, id, "boom").unwrap();
        assert_eq!(status_of(&pool, id), "failed");
        assert_eq!(attempts_of(&pool, id), 1);
    }

    #[test]
    fn mark_for_retry_returns_to_pending_and_increments_attempts() {
        let pool = test_pool();
        let id = insert_notification(&pool, "wud", "containers", "a").unwrap();
        claim_pending(&pool, 10).unwrap();
        mark_for_retry(&pool, id, "temporary").unwrap();
        assert_eq!(status_of(&pool, id), "pending");
        assert_eq!(attempts_of(&pool, id), 1);
        let claimed = claim_pending(&pool, 10).unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].attempts, 1);
    }
}
