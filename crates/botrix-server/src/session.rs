//! Session helpers layered on top of `tower-sessions`.
//!
//! The authenticated admin's username is stored under [`USER_KEY`]. Handlers
//! read it to identify the current user, and the auth guard rejects requests
//! without it.
//!
//! ## Timeouts
//!
//! Two limits apply to every authenticated session, read from the
//! `session_settings` table **on each request** so changes take effect
//! immediately without a restart:
//!
//! - *idle timeout*: seconds since the session's previous request;
//! - *absolute lifetime*: seconds since login, regardless of activity.
//!
//! [`enforce_timeouts`] checks both and destroys the session when either is
//! exceeded. The `tower-sessions` layer's own expiry is only a backstop
//! (see [`crate::db::settings::MAX_ABSOLUTE_LIFETIME_SECS`]).

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use botrix_common::api::settings::Settings;
use tower_sessions::Session;

use crate::state::AppState;

/// Session key under which the authenticated username is stored.
pub const USER_KEY: &str = "user";
/// Session key holding the Unix time of login.
pub const LOGIN_AT_KEY: &str = "login_at";
/// Session key holding the Unix time of the previous authenticated request.
pub const LAST_SEEN_KEY: &str = "last_seen";

/// Record the authenticated username in the session and rotate the session ID
/// to prevent session fixation.
pub async fn log_in(
    session: &Session,
    username: &str,
) -> Result<(), tower_sessions::session::Error> {
    // Rotate the ID on privilege change (login) to defend against fixation.
    session.cycle_id().await?;
    let now = crate::db::now_unix();
    session.insert(USER_KEY, username.to_owned()).await?;
    session.insert(LOGIN_AT_KEY, now).await?;
    session.insert(LAST_SEEN_KEY, now).await?;
    Ok(())
}

/// Clear the session, logging the user out.
pub async fn log_out(session: &Session) -> Result<(), tower_sessions::session::Error> {
    session.flush().await
}

/// Return the authenticated username, if any.
pub async fn current_user(session: &Session) -> Option<String> {
    session.get::<String>(USER_KEY).await.ok().flatten()
}

/// Outcome of checking a session against the configured timeouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The session is within both limits.
    Valid,
    /// More than the idle timeout has passed since the previous request.
    IdleExpired,
    /// More than the absolute lifetime has passed since login.
    AbsoluteExpired,
    /// Timestamps are missing (e.g. a session from before timeouts were
    /// tracked); treated as expired so the user must log in again.
    Untracked,
}

/// Decide whether a session is still valid at time `now` (Unix seconds).
pub fn check(
    now: i64,
    login_at: Option<i64>,
    last_seen: Option<i64>,
    limits: &Settings,
) -> Verdict {
    let (Some(login_at), Some(last_seen)) = (login_at, last_seen) else {
        return Verdict::Untracked;
    };
    if now.saturating_sub(login_at) > limits.absolute_lifetime_secs {
        return Verdict::AbsoluteExpired;
    }
    if now.saturating_sub(last_seen) > limits.idle_timeout_secs {
        return Verdict::IdleExpired;
    }
    Verdict::Valid
}

/// Middleware: enforce the idle and absolute session timeouts.
///
/// Must be layered *inside* the session layer. Expired sessions are flushed
/// (destroyed) so downstream handlers see an unauthenticated request and
/// answer 401; valid ones have their last-seen time refreshed. Requests
/// without an authenticated session pass through untouched. If the settings
/// cannot be read the request fails closed with a 500.
pub async fn enforce_timeouts(
    State(state): State<AppState>,
    session: Session,
    request: Request,
    next: Next,
) -> Response {
    if current_user(&session).await.is_some() {
        let limits = match crate::db::settings::get_settings(&state.pool) {
            Ok(limits) => limits,
            Err(err) => {
                tracing::error!(error = %err, "failed to read session settings");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };

        let now = crate::db::now_unix();
        let login_at = session.get::<i64>(LOGIN_AT_KEY).await.ok().flatten();
        let last_seen = session.get::<i64>(LAST_SEEN_KEY).await.ok().flatten();

        match check(now, login_at, last_seen, &limits) {
            Verdict::Valid => {
                if let Err(err) = session.insert(LAST_SEEN_KEY, now).await {
                    tracing::error!(error = %err, "failed to refresh session activity");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            }
            expired => {
                tracing::info!(reason = ?expired, "session expired; destroying");
                if let Err(err) = session.flush().await {
                    tracing::error!(error = %err, "failed to destroy expired session");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            }
        }
    }

    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(idle: i64, absolute: i64) -> Settings {
        Settings {
            idle_timeout_secs: idle,
            absolute_lifetime_secs: absolute,
        }
    }

    #[test]
    fn within_both_limits_is_valid() {
        assert_eq!(
            check(1_000, Some(900), Some(990), &limits(60, 3_600)),
            Verdict::Valid
        );
    }

    #[test]
    fn exactly_at_a_limit_is_still_valid() {
        // Idle: 60s elapsed with a 60s limit; absolute: 3600s with 3600s.
        assert_eq!(
            check(3_600, Some(0), Some(3_540), &limits(60, 3_600)),
            Verdict::Valid
        );
    }

    #[test]
    fn beyond_idle_timeout_is_idle_expired() {
        assert_eq!(
            check(1_000, Some(900), Some(900), &limits(60, 3_600)),
            Verdict::IdleExpired
        );
    }

    #[test]
    fn beyond_absolute_lifetime_is_absolute_expired_even_if_active() {
        assert_eq!(
            check(10_000, Some(0), Some(9_999), &limits(60, 3_600)),
            Verdict::AbsoluteExpired
        );
    }

    #[test]
    fn absolute_takes_precedence_when_both_exceeded() {
        assert_eq!(
            check(10_000, Some(0), Some(0), &limits(60, 3_600)),
            Verdict::AbsoluteExpired
        );
    }

    #[test]
    fn missing_timestamps_are_untracked() {
        assert_eq!(
            check(1_000, None, Some(990), &limits(60, 3_600)),
            Verdict::Untracked
        );
        assert_eq!(
            check(1_000, Some(900), None, &limits(60, 3_600)),
            Verdict::Untracked
        );
    }

    #[test]
    fn clock_going_backwards_is_not_treated_as_expired() {
        // `last_seen` in the future (clock adjusted) yields a negative age.
        assert_eq!(
            check(1_000, Some(900), Some(2_000), &limits(60, 3_600)),
            Verdict::Valid
        );
    }
}
