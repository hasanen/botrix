//! Persistence for the singleton `matrix_state` row (plan §5.1).
//!
//! `matrix_state` holds the bot's OAuth session: user ID, homeserver, device
//! ID, OAuth client ID, the connection status, and the (encrypted) access and
//! refresh tokens. The table is seeded with a single row (`id = 1`) by the
//! initial migration, so every write here is an `UPDATE` of that row.
//!
//! Token columns store ciphertext produced by [`crate::matrix::crypto`]; this
//! module never performs encryption itself — callers pass already-encrypted
//! strings and receive ciphertext back, keeping the key-handling in one place.

use super::{now_unix, DbError, Pool};

/// Allowed values of the `status` column, mirroring the `CHECK` constraint.
pub mod status {
    /// No session configured.
    pub const DISCONNECTED: &str = "disconnected";
    /// OAuth flow in progress.
    pub const CONNECTING: &str = "connecting";
    /// Session active.
    pub const CONNECTED: &str = "connected";
    /// A session error requiring intervention.
    pub const ERROR: &str = "error";
}

/// A snapshot of the `matrix_state` singleton row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixState {
    /// Matrix user ID (`@bot:server`), if a session exists.
    pub user_id: Option<String>,
    /// Homeserver URL associated with the session.
    pub homeserver: Option<String>,
    /// Encrypted access token (see [`crate::matrix::crypto`]).
    pub access_token: Option<String>,
    /// Encrypted refresh token.
    pub refresh_token: Option<String>,
    /// Device ID issued during login.
    pub device_id: Option<String>,
    /// OAuth client ID registered with the homeserver.
    pub oauth_client_id: Option<String>,
    /// Unix timestamp of the last successful connection.
    pub connected_at: Option<i64>,
    /// Unix timestamp of the last token refresh.
    pub last_token_refresh: Option<i64>,
    /// Current connection status (one of [`status`]).
    pub status: String,
}

impl MatrixState {
    /// Whether the row represents an active, restorable session: status is
    /// `connected` and both tokens plus identifying fields are present.
    pub fn is_connected(&self) -> bool {
        self.status == status::CONNECTED && self.access_token.is_some() && self.user_id.is_some()
    }
}

/// Read the singleton `matrix_state` row.
pub fn get_state(pool: &Pool) -> Result<MatrixState, DbError> {
    let conn = pool.get()?;
    let state = conn.query_row(
        "SELECT user_id, homeserver, access_token, refresh_token, device_id,
                oauth_client_id, connected_at, last_token_refresh, status
         FROM matrix_state WHERE id = 1",
        [],
        |r| {
            Ok(MatrixState {
                user_id: r.get(0)?,
                homeserver: r.get(1)?,
                access_token: r.get(2)?,
                refresh_token: r.get(3)?,
                device_id: r.get(4)?,
                oauth_client_id: r.get(5)?,
                connected_at: r.get(6)?,
                last_token_refresh: r.get(7)?,
                status: r.get(8)?,
            })
        },
    )?;
    Ok(state)
}

/// Set only the `status` column.
pub fn set_status(pool: &Pool, status: &str) -> Result<(), DbError> {
    let conn = pool.get()?;
    conn.execute(
        "UPDATE matrix_state SET status = ?1 WHERE id = 1",
        rusqlite::params![status],
    )?;
    Ok(())
}

/// Fields required to persist a freshly established or restored session.
#[derive(Debug, Clone)]
pub struct SessionRecord {
    /// Matrix user ID.
    pub user_id: String,
    /// Homeserver URL.
    pub homeserver: String,
    /// Device ID.
    pub device_id: String,
    /// OAuth client ID, if known.
    pub oauth_client_id: Option<String>,
    /// Encrypted access token.
    pub access_token: String,
    /// Encrypted refresh token, if the server issued one.
    pub refresh_token: Option<String>,
}

/// Persist a connected session: identity, encrypted tokens, `connected_at`,
/// and status `connected`.
///
/// Tokens must already be encrypted by [`crate::matrix::crypto`].
pub fn persist_session(pool: &Pool, record: &SessionRecord) -> Result<(), DbError> {
    let now = now_unix();
    let conn = pool.get()?;
    conn.execute(
        "UPDATE matrix_state SET
            user_id = ?1,
            homeserver = ?2,
            device_id = ?3,
            oauth_client_id = ?4,
            access_token = ?5,
            refresh_token = ?6,
            connected_at = ?7,
            last_token_refresh = ?7,
            status = ?8
         WHERE id = 1",
        rusqlite::params![
            record.user_id,
            record.homeserver,
            record.device_id,
            record.oauth_client_id,
            record.access_token,
            record.refresh_token,
            now,
            status::CONNECTED,
        ],
    )?;
    Ok(())
}

/// Replace the stored (encrypted) tokens after a token refresh, updating
/// `last_token_refresh`. Leaves identity fields and status untouched.
pub fn update_tokens(
    pool: &Pool,
    access_token: &str,
    refresh_token: Option<&str>,
) -> Result<(), DbError> {
    let conn = pool.get()?;
    conn.execute(
        "UPDATE matrix_state SET
            access_token = ?1,
            refresh_token = ?2,
            last_token_refresh = ?3
         WHERE id = 1",
        rusqlite::params![access_token, refresh_token, now_unix()],
    )?;
    Ok(())
}

/// Clear all session fields and set status `disconnected`.
///
/// Used on explicit disconnect (§9.8) and when session restore fails (§9.4).
/// Does not touch webhook tokens or notification history.
pub fn clear_session(pool: &Pool) -> Result<(), DbError> {
    let conn = pool.get()?;
    conn.execute(
        "UPDATE matrix_state SET
            user_id = NULL,
            homeserver = NULL,
            access_token = NULL,
            refresh_token = NULL,
            device_id = NULL,
            oauth_client_id = NULL,
            connected_at = NULL,
            last_token_refresh = NULL,
            status = ?1
         WHERE id = 1",
        rusqlite::params![status::DISCONNECTED],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_pool;

    fn sample_record() -> SessionRecord {
        SessionRecord {
            user_id: "@botrix:example.com".to_owned(),
            homeserver: "https://matrix.example.com".to_owned(),
            device_id: "DEVICEID".to_owned(),
            oauth_client_id: Some("client-123".to_owned()),
            access_token: "enc-access".to_owned(),
            refresh_token: Some("enc-refresh".to_owned()),
        }
    }

    #[test]
    fn initial_state_is_disconnected_and_empty() {
        let pool = test_pool();
        let state = get_state(&pool).unwrap();
        assert_eq!(state.status, status::DISCONNECTED);
        assert_eq!(state.user_id, None);
        assert_eq!(state.access_token, None);
        assert!(!state.is_connected());
    }

    #[test]
    fn set_status_updates_only_status() {
        let pool = test_pool();
        set_status(&pool, status::CONNECTING).unwrap();
        let state = get_state(&pool).unwrap();
        assert_eq!(state.status, status::CONNECTING);
        assert_eq!(state.user_id, None);
    }

    #[test]
    fn persist_session_stores_all_fields_and_marks_connected() {
        let pool = test_pool();
        persist_session(&pool, &sample_record()).unwrap();

        let state = get_state(&pool).unwrap();
        assert_eq!(state.status, status::CONNECTED);
        assert_eq!(state.user_id.as_deref(), Some("@botrix:example.com"));
        assert_eq!(
            state.homeserver.as_deref(),
            Some("https://matrix.example.com")
        );
        assert_eq!(state.device_id.as_deref(), Some("DEVICEID"));
        assert_eq!(state.oauth_client_id.as_deref(), Some("client-123"));
        assert_eq!(state.access_token.as_deref(), Some("enc-access"));
        assert_eq!(state.refresh_token.as_deref(), Some("enc-refresh"));
        assert!(state.connected_at.is_some());
        assert!(state.last_token_refresh.is_some());
        assert!(state.is_connected());
    }

    #[test]
    fn update_tokens_replaces_tokens_and_bumps_refresh_time() {
        let pool = test_pool();
        persist_session(&pool, &sample_record()).unwrap();

        update_tokens(&pool, "enc-access-2", Some("enc-refresh-2")).unwrap();

        let state = get_state(&pool).unwrap();
        assert_eq!(state.access_token.as_deref(), Some("enc-access-2"));
        assert_eq!(state.refresh_token.as_deref(), Some("enc-refresh-2"));
        // Identity untouched.
        assert_eq!(state.user_id.as_deref(), Some("@botrix:example.com"));
        assert_eq!(state.status, status::CONNECTED);
    }

    #[test]
    fn update_tokens_accepts_absent_refresh_token() {
        let pool = test_pool();
        persist_session(&pool, &sample_record()).unwrap();
        update_tokens(&pool, "enc-access-2", None).unwrap();
        let state = get_state(&pool).unwrap();
        assert_eq!(state.refresh_token, None);
    }

    #[test]
    fn clear_session_nulls_everything_and_disconnects() {
        let pool = test_pool();
        persist_session(&pool, &sample_record()).unwrap();

        clear_session(&pool).unwrap();

        let state = get_state(&pool).unwrap();
        assert_eq!(state.status, status::DISCONNECTED);
        assert_eq!(state.user_id, None);
        assert_eq!(state.homeserver, None);
        assert_eq!(state.access_token, None);
        assert_eq!(state.refresh_token, None);
        assert_eq!(state.device_id, None);
        assert_eq!(state.oauth_client_id, None);
        assert_eq!(state.connected_at, None);
        assert_eq!(state.last_token_refresh, None);
        assert!(!state.is_connected());
    }

    #[test]
    fn singleton_row_is_never_duplicated() {
        let pool = test_pool();
        persist_session(&pool, &sample_record()).unwrap();
        set_status(&pool, status::ERROR).unwrap();
        clear_session(&pool).unwrap();
        let count: i64 = pool
            .get()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM matrix_state", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}
