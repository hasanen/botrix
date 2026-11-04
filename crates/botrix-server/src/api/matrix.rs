//! Matrix API handlers (authenticated).
//!
//! - `GET  /api/v1/matrix/status`       — current connection status
//! - `GET  /api/v1/matrix/rooms`        — configured destinations + room state
//! - `POST /api/v1/matrix/connect`      — begin OAuth Authorization Code login
//! - `GET  /api/v1/matrix/callback`     — OAuth redirect target; finishes login
//! - `POST /api/v1/matrix/disconnect`   — clear the session
//! - `POST /api/v1/matrix/rooms/:name/join` — accept a room invitation
//! - `POST /api/v1/matrix/test-message` — deliver a test message
//! - `POST /api/v1/matrix/recovery-key` — submit the E2EE recovery key (TODO)
//!
//! Login uses the OAuth 2.0 Authorization Code flow (plan §9.3, redirect
//! variant): `connect` returns the homeserver authorization URL; the operator
//! logs in there; the homeserver redirects to `callback`, which completes the
//! login and persists the session. E2EE recovery (`recovery-key`) is still a
//! stub pending a live homeserver (§9.5).

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use axum_csrf::CsrfToken;
use botrix_common::api::matrix::{
    ConnectResponse, DisconnectRequest, MatrixStatus, RecoveryKeyRequest, RoomInfo,
    TestMessageRequest,
};
use botrix_common::config::oauth_redirect_uri_for_origin;
use botrix_common::types::{LogLevel, MatrixConnectionStatus, RoomMembershipState};
use serde::Deserialize;
use tower_sessions::Session;

use super::csrf::verify_csrf;
use super::error::ApiError;
use crate::db::logs::{record, Component};
use crate::db::matrix as db_matrix;
use crate::matrix::client;
use crate::session;
use crate::state::AppState;

/// Require an authenticated session.
async fn require_user(session: &Session) -> Result<String, ApiError> {
    session::current_user(session)
        .await
        .ok_or(ApiError::Unauthorized)
}

/// Map a stored `matrix_state.status` string to the API enum.
fn status_enum(status: &str) -> MatrixConnectionStatus {
    match status {
        db_matrix::status::CONNECTING => MatrixConnectionStatus::Connecting,
        db_matrix::status::CONNECTED => MatrixConnectionStatus::Connected,
        db_matrix::status::ERROR => MatrixConnectionStatus::Error,
        // `disconnected` and any unexpected value fall back to disconnected.
        _ => MatrixConnectionStatus::Disconnected,
    }
}

/// `GET /api/v1/matrix/status` — report the persisted connection status.
pub async fn status(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<MatrixStatus>, ApiError> {
    require_user(&session).await?;

    let s = db_matrix::get_state(&state.pool)?;
    Ok(Json(MatrixStatus {
        status: status_enum(&s.status),
        user_id: s.user_id,
        homeserver: s.homeserver,
        // E2EE bootstrap tracking is part of the stubbed recovery-key work;
        // always false until that is implemented (§9.5).
        e2ee_bootstrapped: false,
        connected_at: s.connected_at,
    }))
}

/// `GET /api/v1/matrix/rooms` — list configured destinations with room state.
///
/// Membership and encryption are resolved best-effort from the live client. If
/// no client is connected, both are reported as unknown/false.
pub async fn rooms(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<RoomInfo>>, ApiError> {
    require_user(&session).await?;

    let client = state.matrix.client().await;
    let mut out = Vec::with_capacity(state.config.matrix.destinations.len());
    for (name, dest) in &state.config.matrix.destinations {
        let mut info = RoomInfo {
            name: name.clone(),
            room_id: dest.room_id.clone(),
            display_name: None,
            membership: RoomMembershipState::Unknown,
            encrypted: false,
            enabled: dest.enabled,
        };

        if let Some(client) = &client {
            if let Ok(room_id) = matrix_sdk::ruma::RoomId::parse(&dest.room_id) {
                if let Some(room) = client.get_room(&room_id) {
                    info.display_name = room.name();
                    info.membership = membership_of(room.state());
                    info.encrypted = room.encryption_state().is_encrypted();
                }
            }
        }

        out.push(info);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(out))
}

/// Translate the SDK's room state into the shared membership enum.
fn membership_of(state: matrix_sdk::RoomState) -> RoomMembershipState {
    match state {
        matrix_sdk::RoomState::Joined => RoomMembershipState::Joined,
        matrix_sdk::RoomState::Invited => RoomMembershipState::Invited,
        matrix_sdk::RoomState::Left => RoomMembershipState::Left,
        matrix_sdk::RoomState::Banned => RoomMembershipState::Banned,
        // Knocked has no shared-enum equivalent; surface as unknown.
        matrix_sdk::RoomState::Knocked => RoomMembershipState::Unknown,
    }
}

/// Determine the OAuth redirect URI: `public_url` if configured, else derived
/// from the request's forwarded/host headers.
fn redirect_uri(state: &AppState, headers: &HeaderMap) -> Result<String, ApiError> {
    if let Some(uri) = state.config.server.oauth_redirect_uri() {
        return Ok(uri);
    }
    let origin = request_origin(headers).ok_or_else(|| {
        ApiError::BadRequest(
            "cannot determine the callback URL; set server.public_url (or BOTRIX_PUBLIC_URL)"
                .to_owned(),
        )
    })?;
    Ok(oauth_redirect_uri_for_origin(&origin))
}

/// Best-effort request origin (`scheme://host`) from proxy/host headers.
fn request_origin(headers: &HeaderMap) -> Option<String> {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let host = header("x-forwarded-host").or_else(|| header("host"))?;
    let scheme = header("x-forwarded-proto").unwrap_or("http");
    Some(format!("{scheme}://{host}"))
}

/// `POST /api/v1/matrix/connect` — begin the OAuth Authorization Code login.
///
/// Builds (or reuses) the Matrix client, registers client metadata with the
/// homeserver, and returns the authorization URL for the browser to open. The
/// homeserver redirects back to [`callback`] on the same client instance.
pub async fn connect(
    State(state): State<AppState>,
    session: Session,
    token: CsrfToken,
    headers: HeaderMap,
) -> Result<Json<ConnectResponse>, ApiError> {
    require_user(&session).await?;
    verify_csrf(&token, &headers)?;

    if state.matrix.is_connected().await {
        return Err(ApiError::Conflict(
            "a Matrix account is already connected; disconnect first".to_owned(),
        ));
    }

    let redirect = redirect_uri(&state, &headers)?;
    let public_base = redirect
        .strip_suffix(botrix_common::config::OAUTH_CALLBACK_PATH)
        .unwrap_or(&redirect)
        .to_owned();

    // Build a fresh client and keep it in the shared handle: the SDK stores
    // PKCE/CSRF state in it, so the callback must use this same instance.
    let homeserver = state.config.matrix.homeserver.clone();
    let client = client::build_client(&state.data_dir, &homeserver, &state.crypto_passphrase)
        .await
        .map_err(|e| connect_error(&state, "building the Matrix client", e))?;
    state.matrix.set_client(client.clone()).await;

    let url = client::begin_login(&client, &redirect, &public_base)
        .await
        .map_err(|e| connect_error(&state, "starting the OAuth login", e))?;
    db_matrix::set_status(&state.pool, db_matrix::status::CONNECTING)?;
    record(
        &state.pool,
        LogLevel::Info,
        Component::Matrix,
        "OAuth login started",
        None,
    );

    Ok(Json(ConnectResponse {
        authorization_url: url,
    }))
}

/// Log a connect-flow failure and map it to a 502 with a readable message.
///
/// Connect failures are almost always configuration or homeserver problems
/// the operator needs to see (wrong homeserver URL, a server without MAS, a
/// redirect URI the server rejects), so surface the cause rather than a bare
/// 500. Returns [`ApiError::Upstream`].
fn connect_error(state: &AppState, during: &str, err: crate::matrix::MatrixError) -> ApiError {
    tracing::error!(error = %err, during, "Matrix connect failed");
    record(
        &state.pool,
        LogLevel::Error,
        Component::Matrix,
        "connect failed",
        Some(&format!("while {during}: {err}")),
    );

    // Homeservers running MAS reject dynamic client registration when the
    // redirect/client URL is not HTTPS (a plain http://localhost will fail).
    // Point the operator at the actual fix rather than the raw OAuth error.
    let text = err.to_string();
    let message = if text.contains("invalid_redirect_uri") || text.contains("invalid client_uri") {
        format!(
            "{err}. The homeserver rejected Botrix's callback URL. Set server.public_url \
             (or BOTRIX_PUBLIC_URL) to the HTTPS URL where Botrix is reachable; most \
             homeservers refuse plain http:// or localhost redirect URLs."
        )
    } else {
        format!("Matrix connect failed while {during}: {err}")
    };
    ApiError::Upstream(message)
}

/// Query parameters on the OAuth redirect callback.
#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    /// Full raw query string is reconstructed from these; the SDK validates
    /// `state` and exchanges `code`.
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

/// `GET /api/v1/matrix/callback` — OAuth redirect target.
///
/// The homeserver redirects the operator's browser here with `code`/`state`.
/// This finishes the login on the shared client, persists the (encrypted)
/// session, activates live delivery, and redirects the browser back to the
/// Matrix page. Requires an authenticated admin session so a stray `code`
/// cannot be replayed by an unauthenticated caller.
pub async fn callback(
    State(state): State<AppState>,
    session: Session,
    Query(params): Query<CallbackQuery>,
) -> Result<Response, ApiError> {
    require_user(&session).await?;

    // Explicit error from the authorization server.
    if let Some(error) = params.error {
        let detail = params.error_description.unwrap_or_default();
        db_matrix::set_status(&state.pool, db_matrix::status::ERROR)?;
        record(
            &state.pool,
            LogLevel::Warn,
            Component::Matrix,
            "OAuth login denied by authorization server",
            Some(&format!("{error}: {detail}")),
        );
        return Ok(Redirect::to("/matrix?error=denied").into_response());
    }

    let (Some(code), Some(csrf_state)) = (params.code, params.state) else {
        return Err(ApiError::BadRequest(
            "callback missing code or state".to_owned(),
        ));
    };

    let client = state.matrix.client().await.ok_or_else(|| {
        ApiError::Conflict("no OAuth login is in progress; start again".to_owned())
    })?;

    // Reconstruct the query the SDK expects (it reads code + state).
    let query = format!(
        "code={}&state={}",
        urlencoding_encode(&code),
        urlencoding_encode(&csrf_state)
    );
    let homeserver = state.config.matrix.homeserver.clone();

    let record_fields = match client::complete_login(
        &client,
        &query,
        &homeserver,
        &state.crypto_passphrase,
    )
    .await
    {
        Ok(record_fields) => record_fields,
        Err(err) => {
            tracing::error!(error = %err, "OAuth login failed in callback");
            db_matrix::set_status(&state.pool, db_matrix::status::ERROR)?;
            record(
                &state.pool,
                LogLevel::Error,
                Component::Matrix,
                "OAuth login failed",
                Some(&err.to_string()),
            );
            return Ok(Redirect::to("/matrix?error=failed").into_response());
        }
    };

    let user_id = record_fields.user_id.clone();
    db_matrix::persist_session(&state.pool, &record_fields)?;
    state.matrix.activate(client).await;
    record(
        &state.pool,
        LogLevel::Info,
        Component::Matrix,
        "connected",
        Some(&user_id),
    );

    Ok(Redirect::to("/matrix?connected=1").into_response())
}

/// Percent-encode a value for use in the callback query string.
fn urlencoding_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// `POST /api/v1/matrix/disconnect` — clear the session (§9.8).
///
/// Logs out of the homeserver if a live client exists, clears `matrix_state`
/// tokens and identity, sets status `disconnected`, and stops delivery.
/// Webhook tokens and notification history are left untouched. Requires
/// `confirm: true`.
pub async fn disconnect(
    State(state): State<AppState>,
    session: Session,
    token: CsrfToken,
    headers: HeaderMap,
    Json(req): Json<DisconnectRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_user(&session).await?;
    verify_csrf(&token, &headers)?;

    if !req.confirm {
        return Err(ApiError::BadRequest(
            "disconnect requires confirm = true".to_owned(),
        ));
    }

    // Best-effort server-side logout before dropping local state.
    if let Some(client) = state.matrix.client().await {
        if let Err(e) = client.oauth().logout().await {
            tracing::warn!(error = %e, "OAuth logout failed; clearing local session anyway");
        }
    }

    state.matrix.deactivate().await;
    db_matrix::clear_session(&state.pool)?;
    tracing::info!("Matrix session disconnected by admin");
    record(
        &state.pool,
        LogLevel::Info,
        Component::Matrix,
        "disconnected by admin",
        None,
    );
    Ok(Json(serde_json::json!({ "status": "disconnected" })))
}

/// `POST /api/v1/matrix/rooms/:name/join` — accept a room invitation (§9.6).
///
/// `:name` is the logical destination name from config. Returns 400 for an
/// unknown destination, 409 when there is no live Matrix client or the room
/// cannot be found/joined, and 200 on success.
pub async fn join_room(
    State(state): State<AppState>,
    session: Session,
    token: CsrfToken,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_user(&session).await?;
    verify_csrf(&token, &headers)?;

    let dest = state
        .config
        .matrix
        .destinations
        .get(&name)
        .ok_or_else(|| ApiError::BadRequest(format!("unknown destination '{name}'")))?;

    let client = state
        .matrix
        .client()
        .await
        .ok_or_else(|| ApiError::Conflict("no active Matrix session".to_owned()))?;

    let room_id = matrix_sdk::ruma::RoomId::parse(&dest.room_id)
        .map_err(|e| ApiError::BadRequest(format!("invalid room ID '{}': {e}", dest.room_id)))?;

    let room = client.get_room(&room_id).ok_or_else(|| {
        ApiError::Conflict(format!(
            "room '{}' is not known to the client",
            dest.room_id
        ))
    })?;

    room.join().await.map_err(|e| {
        tracing::error!(error = %e, room_id = %dest.room_id, "failed to join room");
        // Details stay in the server log; don't leak SDK internals to the UI.
        ApiError::Conflict("failed to join room; see server logs".to_owned())
    })?;

    record(
        &state.pool,
        LogLevel::Info,
        Component::Matrix,
        "joined room",
        Some(&name),
    );
    Ok(Json(serde_json::json!({ "status": "joined" })))
}

/// `POST /api/v1/matrix/test-message` — send a test message to a destination.
///
/// Delivers synchronously through the shared sender, so it exercises the exact
/// path queued notifications take. Returns 400 for an unknown destination or a
/// permanent send failure, and 409 when delivery fails transiently (e.g.
/// Matrix is not connected).
pub async fn test_message(
    State(state): State<AppState>,
    session: Session,
    token: CsrfToken,
    headers: HeaderMap,
    Json(req): Json<TestMessageRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_user(&session).await?;
    verify_csrf(&token, &headers)?;

    // Reject unknown destinations up front with a clear 400.
    if !state
        .config
        .matrix
        .destinations
        .contains_key(&req.destination)
    {
        return Err(ApiError::BadRequest(format!(
            "unknown destination '{}'",
            req.destination
        )));
    }

    let body = "Botrix test message — if you can read this, delivery works.";
    match state.matrix.sender().send(&req.destination, body).await {
        Ok(()) => {
            record(
                &state.pool,
                LogLevel::Info,
                Component::Matrix,
                "test message sent",
                Some(&req.destination),
            );
            Ok(Json(serde_json::json!({ "status": "sent" })))
        }
        Err(err) => {
            let (msg, api_err) = match err {
                crate::queue::sender::SendError::Permanent(msg) => {
                    (msg.clone(), ApiError::BadRequest(msg))
                }
                crate::queue::sender::SendError::Transient(msg) => {
                    (msg.clone(), ApiError::Conflict(msg))
                }
            };
            record(
                &state.pool,
                LogLevel::Warn,
                Component::Matrix,
                "test message failed",
                Some(&format!("{}: {msg}", req.destination)),
            );
            Err(api_err)
        }
    }
}

/// `POST /api/v1/matrix/recovery-key` — submit the E2EE recovery key (§9.5).
///
/// TODO(plan §9.5): Pass the recovery key to the SDK's key-backup restore, then
/// mark E2EE bootstrapped and verify an encrypted test send. This bootstrap
/// needs a live homeserver to exercise, so the key is accepted syntactically
/// but the operation reports "not yet implemented". The key is never persisted
/// (§16.11).
pub async fn recovery_key(
    State(_state): State<AppState>,
    session: Session,
    token: CsrfToken,
    headers: HeaderMap,
    Json(req): Json<RecoveryKeyRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_user(&session).await?;
    verify_csrf(&token, &headers)?;

    if req.recovery_key.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "recovery key must not be empty".to_owned(),
        ));
    }

    Err(ApiError::Conflict(
        "E2EE recovery-key bootstrap is not yet implemented (requires a live homeserver; see plan §9.5)"
            .to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    #[test]
    fn origin_prefers_forwarded_headers() {
        let h = headers(&[
            ("host", "internal:8080"),
            ("x-forwarded-host", "botrix.example.com"),
            ("x-forwarded-proto", "https"),
        ]);
        assert_eq!(
            request_origin(&h).as_deref(),
            Some("https://botrix.example.com")
        );
    }

    #[test]
    fn origin_falls_back_to_host_and_http() {
        let h = headers(&[("host", "localhost:8080")]);
        assert_eq!(request_origin(&h).as_deref(), Some("http://localhost:8080"));
    }

    #[test]
    fn origin_is_none_without_host() {
        assert_eq!(request_origin(&HeaderMap::new()), None);
    }

    #[test]
    fn query_encoding_escapes_reserved_characters() {
        assert_eq!(urlencoding_encode("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(urlencoding_encode("keep-._~"), "keep-._~");
    }
}
