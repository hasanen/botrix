//! Log API types.

use serde::{Deserialize, Serialize};

use crate::types::LogLevel;

/// A single log event entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// Database row ID.
    pub id: i64,
    /// Log severity level.
    pub level: LogLevel,
    /// Component that produced the log entry.
    pub component: String,
    /// Human-readable log message.
    pub message: String,
    /// Optional structured details (JSON-encoded, no secrets).
    pub details: Option<String>,
    /// Unix timestamp when the event was recorded.
    pub created_at: i64,
}

/// Paginated list of log events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogListResponse {
    /// Items on this page.
    pub items: Vec<LogEntry>,
    /// Current page number (1-indexed).
    pub page: u32,
    /// Items per page.
    pub per_page: u32,
    /// Total number of log entries.
    pub total: i64,
}
