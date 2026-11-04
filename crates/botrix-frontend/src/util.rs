//! Pure view-model helpers shared by pages.
//!
//! Everything here is free of DOM and network access so it can be unit-tested
//! on the native target.

use botrix_common::types::{
    LogLevel, MatrixConnectionStatus, NotificationStatus, RoomMembershipState,
};
use thiserror::Error;

use crate::components::status_badge::BadgeStatus;

/// Errors from validating user-entered form values.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FormError {
    /// A numeric field was empty or not a whole number.
    #[error("{field} must be a whole number")]
    NotANumber {
        /// Human-readable field name.
        field: &'static str,
    },
    /// A numeric field was zero or negative.
    #[error("{field} must be greater than zero")]
    NotPositive {
        /// Human-readable field name.
        field: &'static str,
    },
    /// Idle timeout is longer than the absolute session lifetime.
    #[error("idle timeout must not exceed the absolute session lifetime")]
    IdleExceedsAbsolute,
}

/// Absolute URL a service should POST to for the given webhook identifier.
pub fn webhook_url(origin: &str, id: &str) -> String {
    format!(
        "{}/api/v1/webhooks/notify/{id}",
        origin.trim_end_matches('/')
    )
}

/// Query string path for a paginated list endpoint.
pub fn page_path(base: &str, page: u32) -> String {
    format!("{base}?page={page}")
}

/// Number of pages needed for `total` items (at least 1).
pub fn total_pages(total: i64, per_page: u32) -> u32 {
    if per_page == 0 || total <= 0 {
        return 1;
    }
    let per_page = i64::from(per_page);
    (((total + per_page - 1) / per_page) as u32).max(1)
}

/// Truncate `text` to at most `max` characters, appending `…` when cut.
pub fn preview(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

/// Format a Unix timestamp (seconds) as `YYYY-MM-DD HH:MM:SS UTC`.
///
/// Implemented with plain integer arithmetic (proleptic Gregorian calendar,
/// civil-from-days algorithm) to avoid pulling in a date/time crate for the
/// WASM bundle.
pub fn format_unix(ts: i64) -> String {
    let days = ts.div_euclid(86_400);
    let secs_of_day = ts.rem_euclid(86_400);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;

    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC")
}

/// Days-since-epoch to proleptic Gregorian (year, month, day).
///
/// Howard Hinnant's `civil_from_days` algorithm (public domain), which is
/// valid over the full `i64` range and correctly handles leap years.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

/// Parse and validate the session settings form.
///
/// Inputs are the raw idle timeout in minutes and absolute lifetime in hours;
/// returns `(idle_timeout_secs, absolute_lifetime_secs)`.
pub fn parse_session_settings(
    idle_minutes: &str,
    absolute_hours: &str,
) -> Result<(i64, i64), FormError> {
    let idle_minutes: i64 = idle_minutes
        .trim()
        .parse()
        .map_err(|_| FormError::NotANumber {
            field: "idle timeout",
        })?;
    let absolute_hours: i64 = absolute_hours
        .trim()
        .parse()
        .map_err(|_| FormError::NotANumber {
            field: "absolute lifetime",
        })?;

    if idle_minutes <= 0 {
        return Err(FormError::NotPositive {
            field: "idle timeout",
        });
    }
    if absolute_hours <= 0 {
        return Err(FormError::NotPositive {
            field: "absolute lifetime",
        });
    }

    let idle_secs = idle_minutes.checked_mul(60).ok_or(FormError::NotANumber {
        field: "idle timeout",
    })?;
    let absolute_secs = absolute_hours
        .checked_mul(3_600)
        .ok_or(FormError::NotANumber {
            field: "absolute lifetime",
        })?;

    if idle_secs > absolute_secs {
        return Err(FormError::IdleExceedsAbsolute);
    }

    Ok((idle_secs, absolute_secs))
}

/// Badge for a Matrix connection status.
pub fn connection_badge(status: &MatrixConnectionStatus) -> (BadgeStatus, &'static str) {
    match status {
        MatrixConnectionStatus::Connected => (BadgeStatus::Connected, "Connected"),
        MatrixConnectionStatus::Connecting => (BadgeStatus::Connecting, "Connecting"),
        MatrixConnectionStatus::Error => (BadgeStatus::Error, "Error"),
        MatrixConnectionStatus::Disconnected => (BadgeStatus::Disconnected, "Disconnected"),
    }
}

/// Badge for a notification delivery status.
pub fn notification_badge(status: &NotificationStatus) -> (BadgeStatus, &'static str) {
    match status {
        NotificationStatus::Delivered => (BadgeStatus::Connected, "Delivered"),
        NotificationStatus::Pending => (BadgeStatus::Connecting, "Pending"),
        NotificationStatus::Processing => (BadgeStatus::Connecting, "Processing"),
        NotificationStatus::Failed => (BadgeStatus::Error, "Failed"),
    }
}

/// Badge for a log severity level.
pub fn log_level_badge(level: &LogLevel) -> (BadgeStatus, &'static str) {
    match level {
        LogLevel::Info => (BadgeStatus::Disconnected, "INFO"),
        LogLevel::Warn => (BadgeStatus::Connecting, "WARN"),
        LogLevel::Error => (BadgeStatus::Error, "ERROR"),
    }
}

/// Badge for a room membership state.
pub fn membership_badge(state: &RoomMembershipState) -> (BadgeStatus, &'static str) {
    match state {
        RoomMembershipState::Joined => (BadgeStatus::Connected, "Joined"),
        RoomMembershipState::Invited => (BadgeStatus::Connecting, "Invited"),
        RoomMembershipState::Left => (BadgeStatus::Disconnected, "Left"),
        RoomMembershipState::Banned => (BadgeStatus::Error, "Banned"),
        RoomMembershipState::Unknown => (BadgeStatus::Disconnected, "Unknown"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_url_joins_origin_and_id() {
        assert_eq!(
            webhook_url("https://botrix.example.com", "wud"),
            "https://botrix.example.com/api/v1/webhooks/notify/wud"
        );
    }

    #[test]
    fn webhook_url_tolerates_trailing_slash_on_origin() {
        assert_eq!(
            webhook_url("https://botrix.example.com/", "wud"),
            "https://botrix.example.com/api/v1/webhooks/notify/wud"
        );
    }

    #[test]
    fn page_path_appends_page_query() {
        assert_eq!(page_path("/api/v1/logs", 3), "/api/v1/logs?page=3");
    }

    #[test]
    fn total_pages_rounds_up_and_is_at_least_one() {
        assert_eq!(total_pages(0, 25), 1);
        assert_eq!(total_pages(25, 25), 1);
        assert_eq!(total_pages(26, 25), 2);
        assert_eq!(total_pages(100, 0), 1);
    }

    #[test]
    fn preview_leaves_short_text_untouched() {
        assert_eq!(preview("hello", 10), "hello");
    }

    #[test]
    fn preview_truncates_on_char_boundaries() {
        assert_eq!(preview("héllo wörld", 5), "héllo…");
    }

    #[test]
    fn format_unix_epoch() {
        assert_eq!(format_unix(0), "1970-01-01 00:00:00 UTC");
    }

    #[test]
    fn format_unix_known_timestamp() {
        // 2026-11-04 12:34:56 UTC
        assert_eq!(format_unix(1_793_795_696), "2026-11-04 12:34:56 UTC");
    }

    #[test]
    fn format_unix_handles_leap_day() {
        // 2024-02-29 00:00:00 UTC
        assert_eq!(format_unix(1_709_164_800), "2024-02-29 00:00:00 UTC");
    }

    #[test]
    fn settings_form_converts_units() {
        assert_eq!(parse_session_settings("60", "24"), Ok((3_600, 86_400)));
    }

    #[test]
    fn settings_form_trims_whitespace() {
        assert_eq!(parse_session_settings(" 30 ", " 2 "), Ok((1_800, 7_200)));
    }

    #[test]
    fn settings_form_rejects_non_numbers() {
        assert_eq!(
            parse_session_settings("abc", "24"),
            Err(FormError::NotANumber {
                field: "idle timeout"
            })
        );
        assert_eq!(
            parse_session_settings("60", ""),
            Err(FormError::NotANumber {
                field: "absolute lifetime"
            })
        );
    }

    #[test]
    fn settings_form_rejects_zero_and_negative() {
        assert_eq!(
            parse_session_settings("0", "24"),
            Err(FormError::NotPositive {
                field: "idle timeout"
            })
        );
        assert_eq!(
            parse_session_settings("60", "-1"),
            Err(FormError::NotPositive {
                field: "absolute lifetime"
            })
        );
    }

    #[test]
    fn settings_form_rejects_idle_longer_than_absolute() {
        // 120 minutes idle vs 1 hour absolute.
        assert_eq!(
            parse_session_settings("120", "1"),
            Err(FormError::IdleExceedsAbsolute)
        );
    }

    #[test]
    fn settings_form_rejects_overflowing_values() {
        assert_eq!(
            parse_session_settings("60", "9223372036854775807"),
            Err(FormError::NotANumber {
                field: "absolute lifetime"
            })
        );
    }

    #[test]
    fn connection_badges_cover_every_status() {
        assert_eq!(
            connection_badge(&MatrixConnectionStatus::Connected),
            (BadgeStatus::Connected, "Connected")
        );
        assert_eq!(
            connection_badge(&MatrixConnectionStatus::Connecting),
            (BadgeStatus::Connecting, "Connecting")
        );
        assert_eq!(
            connection_badge(&MatrixConnectionStatus::Error),
            (BadgeStatus::Error, "Error")
        );
        assert_eq!(
            connection_badge(&MatrixConnectionStatus::Disconnected),
            (BadgeStatus::Disconnected, "Disconnected")
        );
    }

    #[test]
    fn notification_badges_cover_every_status() {
        assert_eq!(
            notification_badge(&NotificationStatus::Delivered),
            (BadgeStatus::Connected, "Delivered")
        );
        assert_eq!(
            notification_badge(&NotificationStatus::Pending),
            (BadgeStatus::Connecting, "Pending")
        );
        assert_eq!(
            notification_badge(&NotificationStatus::Processing),
            (BadgeStatus::Connecting, "Processing")
        );
        assert_eq!(
            notification_badge(&NotificationStatus::Failed),
            (BadgeStatus::Error, "Failed")
        );
    }

    #[test]
    fn log_level_badges_cover_every_level() {
        assert_eq!(
            log_level_badge(&LogLevel::Info),
            (BadgeStatus::Disconnected, "INFO")
        );
        assert_eq!(
            log_level_badge(&LogLevel::Warn),
            (BadgeStatus::Connecting, "WARN")
        );
        assert_eq!(
            log_level_badge(&LogLevel::Error),
            (BadgeStatus::Error, "ERROR")
        );
    }

    #[test]
    fn membership_badges_cover_every_state() {
        assert_eq!(
            membership_badge(&RoomMembershipState::Joined),
            (BadgeStatus::Connected, "Joined")
        );
        assert_eq!(
            membership_badge(&RoomMembershipState::Invited),
            (BadgeStatus::Connecting, "Invited")
        );
        assert_eq!(
            membership_badge(&RoomMembershipState::Left),
            (BadgeStatus::Disconnected, "Left")
        );
        assert_eq!(
            membership_badge(&RoomMembershipState::Banned),
            (BadgeStatus::Error, "Banned")
        );
        assert_eq!(
            membership_badge(&RoomMembershipState::Unknown),
            (BadgeStatus::Disconnected, "Unknown")
        );
    }
}
