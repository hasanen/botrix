//! Read access to history tables: notification history and log events.
//!
//! Pages are 1-indexed and ordered newest first (`id DESC`, which follows
//! insertion order and is stable for rows created within the same second).

use botrix_common::api::logs::LogEntry;
use botrix_common::api::webhooks::NotificationEntry;
use botrix_common::types::{LogLevel, NotificationStatus};

use super::{DbError, Pool};

/// Convert a 1-indexed page and page size into a SQL `OFFSET`.
fn offset(page: u32, per_page: u32) -> i64 {
    i64::from(page.saturating_sub(1)) * i64::from(per_page)
}

fn parse_status(raw: &str) -> Result<NotificationStatus, DbError> {
    match raw {
        "pending" => Ok(NotificationStatus::Pending),
        "processing" => Ok(NotificationStatus::Processing),
        "delivered" => Ok(NotificationStatus::Delivered),
        "failed" => Ok(NotificationStatus::Failed),
        other => Err(DbError::InvalidValue {
            column: "notifications.status",
            value: other.to_owned(),
        }),
    }
}

fn parse_level(raw: &str) -> Result<LogLevel, DbError> {
    match raw {
        "INFO" => Ok(LogLevel::Info),
        "WARN" => Ok(LogLevel::Warn),
        "ERROR" => Ok(LogLevel::Error),
        other => Err(DbError::InvalidValue {
            column: "log_events.level",
            value: other.to_owned(),
        }),
    }
}

/// Return one page of notifications (newest first) and the total row count.
pub fn list_notifications(
    pool: &Pool,
    page: u32,
    per_page: u32,
) -> Result<(Vec<NotificationEntry>, i64), DbError> {
    let conn = pool.get()?;
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM notifications", [], |r| r.get(0))?;

    let mut stmt = conn.prepare(
        "SELECT id, webhook_id, destination, rendered_message, status, attempts,
                last_error, created_at, delivered_at
         FROM notifications
         ORDER BY id DESC
         LIMIT ?1 OFFSET ?2",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![i64::from(per_page), offset(page, per_page)],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, Option<i64>>(8)?,
            ))
        },
    )?;

    let mut items = Vec::new();
    for row in rows {
        let (
            id,
            webhook_id,
            destination,
            rendered_message,
            status,
            attempts,
            last_error,
            created_at,
            delivered_at,
        ) = row?;
        items.push(NotificationEntry {
            id,
            webhook_id,
            destination,
            rendered_message,
            status: parse_status(&status)?,
            attempts,
            last_error,
            created_at,
            delivered_at,
        });
    }
    Ok((items, total))
}

/// Return one page of log events (newest first) and the total row count.
pub fn list_logs(pool: &Pool, page: u32, per_page: u32) -> Result<(Vec<LogEntry>, i64), DbError> {
    let conn = pool.get()?;
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM log_events", [], |r| r.get(0))?;

    let mut stmt = conn.prepare(
        "SELECT id, level, component, message, details, created_at
         FROM log_events
         ORDER BY id DESC
         LIMIT ?1 OFFSET ?2",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![i64::from(per_page), offset(page, per_page)],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, i64>(5)?,
            ))
        },
    )?;

    let mut items = Vec::new();
    for row in rows {
        let (id, level, component, message, details, created_at) = row?;
        items.push(LogEntry {
            id,
            level: parse_level(&level)?,
            component,
            message,
            details,
            created_at,
        });
    }
    Ok((items, total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_pool;

    #[test]
    fn offset_is_zero_for_first_page_and_never_underflows() {
        assert_eq!(offset(1, 25), 0);
        assert_eq!(offset(0, 25), 0);
        assert_eq!(offset(3, 25), 50);
    }

    #[test]
    fn unknown_log_level_is_a_typed_error() {
        // Arrange
        let pool = test_pool();
        pool.get()
            .unwrap()
            .execute(
                "INSERT INTO log_events (level, component, message, created_at)
                 VALUES ('BOGUS', 'queue', 'x', 1)",
                [],
            )
            .unwrap();

        // Act
        let result = list_logs(&pool, 1, 10);

        // Assert
        assert!(matches!(result, Err(DbError::InvalidValue { .. })));
    }

    #[test]
    fn empty_tables_yield_empty_pages() {
        let pool = test_pool();
        let (items, total) = list_notifications(&pool, 1, 25).unwrap();
        assert!(items.is_empty());
        assert_eq!(total, 0);
    }
}
