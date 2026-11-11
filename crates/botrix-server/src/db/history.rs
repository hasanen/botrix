//! Read access to history tables: notification history and log events.
//!
//! Pages are 1-indexed and ordered newest first (`id DESC`, which follows
//! insertion order and is stable for rows created within the same second).

use botrix_common::api::logs::LogEntry;
use botrix_common::api::webhooks::NotificationEntry;
use botrix_common::types::{LogLevel, NotificationStatus};

use super::logs::command_tag;
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

/// Optional restrictions for [`list_logs_filtered`]; `None` means no restriction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LogFilter<'a> {
    /// Only events from this component, e.g. `trigger`.
    pub component: Option<&'a str>,
    /// Only events tagged with this command (case-insensitive). See
    /// [`command_tag`].
    pub command: Option<&'a str>,
}

/// Return one page of log events (newest first) and the total row count.
pub fn list_logs(pool: &Pool, page: u32, per_page: u32) -> Result<(Vec<LogEntry>, i64), DbError> {
    list_logs_filtered(pool, page, per_page, &LogFilter::default())
}

/// Like [`list_logs`], restricted by `filter`. The total counts only matches.
///
/// The command match is an exact, case-insensitive prefix comparison on the
/// event details, so `_` and `%` in a command are not treated as wildcards.
pub fn list_logs_filtered(
    pool: &Pool,
    page: u32,
    per_page: u32,
    filter: &LogFilter<'_>,
) -> Result<(Vec<LogEntry>, i64), DbError> {
    // `?1` is the component and `?2` the command tag; either may be NULL.
    const WHERE: &str = "WHERE (?1 IS NULL OR component = ?1)
           AND (?2 IS NULL OR substr(details, 1, length(?2)) = ?2)";

    let tag = filter.command.map(command_tag);
    let conn = pool.get()?;
    let total: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM log_events {WHERE}"),
        rusqlite::params![filter.component, tag],
        |r| r.get(0),
    )?;

    let mut stmt = conn.prepare(&format!(
        "SELECT id, level, component, message, details, created_at
         FROM log_events
         {WHERE}
         ORDER BY id DESC
         LIMIT ?3 OFFSET ?4"
    ))?;
    let rows = stmt.query_map(
        rusqlite::params![
            filter.component,
            tag,
            i64::from(per_page),
            offset(page, per_page)
        ],
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
    use crate::db::logs::{command_tag, record, Component};
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

    fn log(pool: &Pool, component: Component, message: &str, details: Option<&str>) {
        record(pool, LogLevel::Info, component, message, details);
    }

    /// Records a trigger event tagged for `command`, like the handler does.
    fn trigger_log(pool: &Pool, command: &str, message: &str) {
        let details = format!("{}sender=@a:x room=!r:x", command_tag(command));
        log(pool, Component::Trigger, message, Some(&details));
    }

    fn messages(items: &[LogEntry]) -> Vec<&str> {
        items.iter().map(|e| e.message.as_str()).collect()
    }

    #[test]
    fn default_filter_returns_everything_like_list_logs() {
        // Arrange
        let pool = test_pool();
        log(&pool, Component::System, "boot", None);
        trigger_log(&pool, "echo", "command succeeded");

        // Act
        let (all, total) = list_logs(&pool, 1, 10).unwrap();
        let (filtered, filtered_total) =
            list_logs_filtered(&pool, 1, 10, &LogFilter::default()).unwrap();

        // Assert
        assert_eq!(total, 2);
        assert_eq!(filtered_total, 2);
        assert_eq!(messages(&all), messages(&filtered));
    }

    #[test]
    fn component_filter_returns_only_that_component_and_counts_only_matches() {
        // Arrange
        let pool = test_pool();
        log(&pool, Component::System, "boot", None);
        log(&pool, Component::Webhook, "token generated", Some("wud"));
        trigger_log(&pool, "echo", "command succeeded");

        // Act
        let filter = LogFilter {
            component: Some("trigger"),
            command: None,
        };
        let (items, total) = list_logs_filtered(&pool, 1, 10, &filter).unwrap();

        // Assert
        assert_eq!(messages(&items), ["command succeeded"]);
        assert_eq!(total, 1);
    }

    #[test]
    fn unknown_component_matches_nothing() {
        let pool = test_pool();
        log(&pool, Component::System, "boot", None);

        let filter = LogFilter {
            component: Some("nope"),
            command: None,
        };
        let (items, total) = list_logs_filtered(&pool, 1, 10, &filter).unwrap();

        assert!(items.is_empty());
        assert_eq!(total, 0);
    }

    #[test]
    fn command_filter_matches_that_command_only_not_longer_ones() {
        // Arrange
        let pool = test_pool();
        trigger_log(&pool, "echo", "command succeeded");
        trigger_log(&pool, "echo2", "command failed");
        trigger_log(&pool, "other", "unknown command");
        // A secret change is tagged the same way.
        let details = format!("{}secret=TOKEN", command_tag("echo"));
        log(&pool, Component::Trigger, "secret set", Some(&details));

        // Act
        let filter = LogFilter {
            component: None,
            command: Some("echo"),
        };
        let (items, total) = list_logs_filtered(&pool, 1, 10, &filter).unwrap();

        // Assert: newest first.
        assert_eq!(messages(&items), ["secret set", "command succeeded"]);
        assert_eq!(total, 2);
    }

    #[test]
    fn command_filter_is_case_insensitive() {
        let pool = test_pool();
        trigger_log(&pool, "Echo", "command succeeded");

        let filter = LogFilter {
            component: None,
            command: Some("ECHO"),
        };
        let (items, _) = list_logs_filtered(&pool, 1, 10, &filter).unwrap();

        assert_eq!(items.len(), 1);
    }

    #[test]
    fn command_filter_treats_sql_wildcards_literally() {
        // Arrange: `_` and `%` are LIKE wildcards, but commands may contain `_`.
        let pool = test_pool();
        trigger_log(&pool, "a_b", "underscore");
        trigger_log(&pool, "axb", "other");

        // Act
        let underscore = LogFilter {
            component: None,
            command: Some("a_b"),
        };
        let percent = LogFilter {
            component: None,
            command: Some("%"),
        };
        let (matched, _) = list_logs_filtered(&pool, 1, 10, &underscore).unwrap();
        let (wild, wild_total) = list_logs_filtered(&pool, 1, 10, &percent).unwrap();

        // Assert
        assert_eq!(messages(&matched), ["underscore"]);
        assert!(wild.is_empty());
        assert_eq!(wild_total, 0);
    }

    #[test]
    fn command_filter_skips_events_without_details() {
        let pool = test_pool();
        log(&pool, Component::Trigger, "no details", None);
        trigger_log(&pool, "echo", "command succeeded");

        let filter = LogFilter {
            component: None,
            command: Some("echo"),
        };
        let (items, total) = list_logs_filtered(&pool, 1, 10, &filter).unwrap();

        assert_eq!(messages(&items), ["command succeeded"]);
        assert_eq!(total, 1);
    }

    #[test]
    fn component_and_command_filters_combine() {
        // Arrange: a non-trigger event whose details happen to carry the tag.
        let pool = test_pool();
        let details = format!("{}x", command_tag("echo"));
        log(&pool, Component::Webhook, "look-alike", Some(&details));
        trigger_log(&pool, "echo", "command succeeded");

        // Act
        let filter = LogFilter {
            component: Some("trigger"),
            command: Some("echo"),
        };
        let (items, total) = list_logs_filtered(&pool, 1, 10, &filter).unwrap();

        // Assert
        assert_eq!(messages(&items), ["command succeeded"]);
        assert_eq!(total, 1);
    }

    #[test]
    fn pagination_applies_to_the_filtered_results() {
        // Arrange
        let pool = test_pool();
        for n in 0..3 {
            trigger_log(&pool, "echo", &format!("run {n}"));
            log(&pool, Component::System, &format!("noise {n}"), None);
        }
        let filter = LogFilter {
            component: None,
            command: Some("echo"),
        };

        // Act
        let (first, total) = list_logs_filtered(&pool, 1, 2, &filter).unwrap();
        let (second, _) = list_logs_filtered(&pool, 2, 2, &filter).unwrap();

        // Assert
        assert_eq!(total, 3);
        assert_eq!(messages(&first), ["run 2", "run 1"]);
        assert_eq!(messages(&second), ["run 0"]);
    }

    #[test]
    fn empty_tables_yield_empty_pages() {
        let pool = test_pool();
        let (items, total) = list_notifications(&pool, 1, 25).unwrap();
        assert!(items.is_empty());
        assert_eq!(total, 0);
    }
}
