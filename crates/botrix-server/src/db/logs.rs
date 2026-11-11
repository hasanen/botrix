//! Writing operational events to the `log_events` table.
//!
//! Events are what the operator sees on the Logs page. They are deliberately
//! limited to security-relevant and operational facts, and must **never**
//! contain secrets (passwords, tokens, recovery keys) or unauthenticated
//! input an attacker could use to flood the table.
//!
//! Recording is best-effort: a failure to write an event is reported through
//! `tracing` but never fails the operation being logged.

use botrix_common::types::LogLevel;

use super::{now_unix, DbError, Pool};

/// Upper bound on stored events. Older events beyond this are dropped on
/// insert, in addition to the 30-day expiry sweep, so the table cannot grow
/// without bound even within the retention window.
pub const MAX_EVENTS: i64 = 10_000;

/// The part of the application an event came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    /// Admin authentication and account changes.
    Auth,
    /// Webhook tokens and inbound payloads.
    Webhook,
    /// The background delivery queue.
    Queue,
    /// The Matrix connection.
    Matrix,
    /// Process lifecycle: startup, crash recovery, maintenance.
    System,
    /// Command triggers.
    Trigger,
}

impl Component {
    /// The value stored in the `component` column.
    pub fn as_str(self) -> &'static str {
        match self {
            Component::Auth => "auth",
            Component::Webhook => "webhook",
            Component::Queue => "queue",
            Component::Matrix => "matrix",
            Component::System => "system",
            Component::Trigger => "trigger",
        }
    }
}

/// The prefix that ties a trigger event to a command.
///
/// Trigger events start their `details` with this tag, and the Logs page
/// filters on it, so the writer and the filter share one definition. The
/// command is lowercased (commands are case-insensitive) and the trailing
/// space stops `echo` from matching `echo2`.
pub fn command_tag(command: &str) -> String {
    format!("command={} ", command.to_lowercase())
}

/// The value stored in the `level` column (matches the API's serialization).
fn level_str(level: &LogLevel) -> &'static str {
    match level {
        LogLevel::Info => "INFO",
        LogLevel::Warn => "WARN",
        LogLevel::Error => "ERROR",
    }
}

/// Insert an event and enforce [`MAX_EVENTS`], returning any database error.
///
/// Prefer [`record`] in application code; this exists so tests can observe
/// failures.
pub fn try_record(
    pool: &Pool,
    level: LogLevel,
    component: Component,
    message: &str,
    details: Option<&str>,
) -> Result<(), DbError> {
    let conn = pool.get()?;
    conn.execute(
        "INSERT INTO log_events (level, component, message, details, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            level_str(&level),
            component.as_str(),
            message,
            details,
            now_unix()
        ],
    )?;
    // Ids only increase, so everything at or below (newest - MAX) is the
    // oldest surplus. An indexed primary-key range delete, effectively free.
    conn.execute(
        "DELETE FROM log_events WHERE id <= last_insert_rowid() - ?1",
        [MAX_EVENTS],
    )?;
    Ok(())
}

/// Record an operational event; never fails the caller.
///
/// `details` is optional free text (for example a webhook id) and must not
/// contain secrets.
pub fn record(
    pool: &Pool,
    level: LogLevel,
    component: Component,
    message: &str,
    details: Option<&str>,
) {
    if let Err(err) = try_record(pool, level, component, message, details) {
        tracing::error!(error = %err, message, "failed to record log event");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::history::list_logs;
    use crate::db::test_pool;

    #[test]
    fn recorded_event_round_trips_through_the_list_query() {
        // Arrange
        let pool = test_pool();

        // Act
        record(
            &pool,
            LogLevel::Warn,
            Component::Webhook,
            "payload rejected",
            Some("wud"),
        );
        let (items, total) = list_logs(&pool, 1, 10).unwrap();

        // Assert
        assert_eq!(total, 1);
        assert_eq!(items[0].level, LogLevel::Warn);
        assert_eq!(items[0].component, "webhook");
        assert_eq!(items[0].message, "payload rejected");
        assert_eq!(items[0].details.as_deref(), Some("wud"));
    }

    #[test]
    fn details_are_optional() {
        let pool = test_pool();
        record(&pool, LogLevel::Info, Component::System, "started", None);
        let (items, _) = list_logs(&pool, 1, 10).unwrap();
        assert_eq!(items[0].details, None);
    }

    #[test]
    fn every_level_and_component_is_readable_back() {
        // Arrange
        let pool = test_pool();
        let levels = [LogLevel::Info, LogLevel::Warn, LogLevel::Error];
        let components = [
            Component::Auth,
            Component::Webhook,
            Component::Queue,
            Component::Matrix,
            Component::System,
            Component::Trigger,
        ];

        // Act: list_logs errors on any level string it cannot parse.
        for level in &levels {
            for component in components {
                record(&pool, level.clone(), component, "m", None);
            }
        }

        // Assert
        let (items, total) = list_logs(&pool, 1, 100).unwrap();
        assert_eq!(total, 18);
        assert_eq!(items.len(), 18);
    }

    #[test]
    fn command_tag_is_lowercase_and_ends_with_a_separator() {
        assert_eq!(command_tag("Deploy"), "command=deploy ");
        assert_eq!(command_tag("a_b"), "command=a_b ");
    }

    #[test]
    fn command_tag_does_not_match_a_longer_command_with_the_same_prefix() {
        // The trailing separator is what keeps `echo` apart from `echo2`.
        assert!(!"command=echo2 sender=x".starts_with(&command_tag("echo")));
        assert!("command=echo sender=x".starts_with(&command_tag("echo")));
    }

    #[test]
    fn trigger_component_is_stored_and_read_back_as_trigger() {
        // Arrange
        let pool = test_pool();

        // Act
        record(&pool, LogLevel::Info, Component::Trigger, "fired", None);
        let (items, _) = list_logs(&pool, 1, 10).unwrap();

        // Assert
        assert_eq!(Component::Trigger.as_str(), "trigger");
        assert_eq!(items[0].component, "trigger");
    }

    #[test]
    fn table_is_capped_dropping_the_oldest_events() {
        // Arrange: fill past the cap.
        let pool = test_pool();
        for n in 0..(MAX_EVENTS + 25) {
            try_record(
                &pool,
                LogLevel::Info,
                Component::System,
                &format!("event {n}"),
                None,
            )
            .unwrap();
        }

        // Act
        let (items, total) = list_logs(&pool, 1, 1).unwrap();
        let oldest: String = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT message FROM log_events ORDER BY id LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();

        // Assert: exactly MAX_EVENTS remain, newest kept, oldest dropped.
        assert_eq!(total, MAX_EVENTS);
        assert_eq!(items[0].message, format!("event {}", MAX_EVENTS + 24));
        assert_eq!(oldest, "event 25");
    }

    #[test]
    fn record_swallows_database_errors() {
        // Arrange: break the table so the insert must fail.
        let pool = test_pool();
        pool.get()
            .unwrap()
            .execute("DROP TABLE log_events", [])
            .unwrap();

        // Act + Assert: no panic, and try_record reports the failure.
        record(&pool, LogLevel::Info, Component::Auth, "x", None);
        assert!(try_record(&pool, LogLevel::Info, Component::Auth, "x", None).is_err());
    }
}
