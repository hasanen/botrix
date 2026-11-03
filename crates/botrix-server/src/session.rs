//! Session helpers layered on top of `tower-sessions`.
//!
//! The authenticated admin's username is stored under [`USER_KEY`]. Handlers
//! read it to identify the current user, and the auth guard rejects requests
//! without it.

use tower_sessions::Session;

/// Session key under which the authenticated username is stored.
pub const USER_KEY: &str = "user";

/// Record the authenticated username in the session and rotate the session ID
/// to prevent session fixation.
pub async fn log_in(
    session: &Session,
    username: &str,
) -> Result<(), tower_sessions::session::Error> {
    // Rotate the ID on privilege change (login) to defend against fixation.
    session.cycle_id().await?;
    session.insert(USER_KEY, username.to_owned()).await?;
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
