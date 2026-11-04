//! Status report backing the `botrix status` CLI command.
//!
//! Reads the application database only; it never contacts the homeserver and
//! never touches secrets (token columns are not selected).

use std::fmt;

use crate::db::{matrix as db_matrix, DbError, Pool};

/// A point-in-time summary of Matrix connectivity and the delivery queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusReport {
    /// Persisted Matrix connection status (one of [`db_matrix::status`]).
    pub matrix_status: String,
    /// Matrix user ID, if a session exists.
    pub user_id: Option<String>,
    /// Homeserver URL, if a session exists.
    pub homeserver: Option<String>,
    /// Unix timestamp of the last successful connection, if any.
    pub connected_at: Option<i64>,
    /// Notifications waiting for delivery (`pending` or `processing`).
    pub queued: i64,
    /// Notifications whose delivery attempts were exhausted.
    pub failed: i64,
}

impl StatusReport {
    /// Build a report from the application database.
    pub fn load(pool: &Pool) -> Result<Self, DbError> {
        let state = db_matrix::get_state(pool)?;
        let conn = pool.get()?;
        let queued: i64 = conn.query_row(
            "SELECT COUNT(*) FROM notifications WHERE status IN ('pending', 'processing')",
            [],
            |r| r.get(0),
        )?;
        let failed: i64 = conn.query_row(
            "SELECT COUNT(*) FROM notifications WHERE status = 'failed'",
            [],
            |r| r.get(0),
        )?;
        Ok(Self {
            matrix_status: state.status,
            user_id: state.user_id,
            homeserver: state.homeserver,
            connected_at: state.connected_at,
            queued,
            failed,
        })
    }
}

impl fmt::Display for StatusReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Matrix status: {}", self.matrix_status)?;
        if let Some(user) = &self.user_id {
            writeln!(f, "Matrix user:   {user}")?;
        }
        if let Some(hs) = &self.homeserver {
            writeln!(f, "Homeserver:    {hs}")?;
        }
        if let Some(ts) = self.connected_at {
            writeln!(f, "Connected at:  {ts} (unix)")?;
        }
        writeln!(f, "Queued:        {}", self.queued)?;
        write!(f, "Failed:        {}", self.failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_pool;

    fn insert(pool: &Pool, status: &str) {
        pool.get()
            .unwrap()
            .execute(
                "INSERT INTO notifications
                   (webhook_id, destination, rendered_message, status, created_at, expires_at)
                 VALUES ('w', 'd', 'm', ?1, 1, 2)",
                [status],
            )
            .unwrap();
    }

    #[test]
    fn fresh_database_reports_disconnected_and_empty_queue() {
        // Arrange
        let pool = test_pool();

        // Act
        let report = StatusReport::load(&pool).unwrap();

        // Assert
        assert_eq!(report.matrix_status, "disconnected");
        assert_eq!(report.user_id, None);
        assert_eq!((report.queued, report.failed), (0, 0));
    }

    #[test]
    fn counts_queued_and_failed_notifications_separately() {
        // Arrange
        let pool = test_pool();
        for status in ["pending", "processing", "delivered", "failed", "failed"] {
            insert(&pool, status);
        }

        // Act
        let report = StatusReport::load(&pool).unwrap();

        // Assert
        assert_eq!(report.queued, 2);
        assert_eq!(report.failed, 2);
    }

    #[test]
    fn display_omits_absent_session_fields() {
        // Arrange
        let pool = test_pool();
        let report = StatusReport::load(&pool).unwrap();

        // Act
        let text = report.to_string();

        // Assert
        assert!(text.contains("Matrix status: disconnected"));
        assert!(!text.contains("Matrix user"));
        assert!(text.contains("Queued:        0"));
    }
}
