//! Matrix API handlers (authenticated).
//!
//! - `GET  /api/v1/matrix/status`        — current connection status
//! - `GET  /api/v1/matrix/rooms`         — configured destinations + room state
//! - `POST /api/v1/matrix/connect`       — begin OAuth device grant (see TODO)
//! - `GET  /api/v1/matrix/connect/poll`  — poll OAuth outcome (see TODO)
//! - `POST /api/v1/matrix/disconnect`    — clear the session
//! - `POST /api/v1/matrix/test-message`  — deliver a test message
//! - `POST /api/v1/matrix/recovery-key`  — submit the E2EE recovery key (TODO)
//!
//! ## Interactive OAuth / E2EE are stubbed
//!
//! `connect`, `connect/poll`, and `recovery-key` drive the OAuth Device
//! Authorization Grant and the E2EE key-backup bootstrap. Both require a live
//! homeserver (Pikaviestin.fi MAS) to exercise and cannot be verified in this
//! environment, so they return a clear "not yet implemented" result rather
//! than faking success. See the implementation plan §9.3/§9.5 for the intended
//! flow and §19.2/§19.4 for the open risks that gate this work. The `status`,
//! `rooms`, `disconnect`, and `test-message` handlers are fully implemented.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use axum_csrf::CsrfToken;
use botrix_common::api::matrix::{
    ConnectPollResponse, ConnectPollStatus, ConnectResponse, DisconnectRequest, MatrixStatus,
    RecoveryKeyRequest, RoomInfo, TestMessageRequest,
};
use botrix_common::types::{MatrixConnectionStatus, RoomMembershipState};
use tower_sessions::Session;

use super::csrf::verify_csrf;
use super::error::ApiError;
use crate::db::matrix as db_matrix;
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
        // E2EE bootstrap tracking is part of the stubbed work; always false
        // until the recovery-key flow is implemented (§9.5).
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

        if let Some(client) = &state.matrix_client {
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

/// `POST /api/v1/matrix/connect` — begin the OAuth Device Authorization Grant.
///
/// TODO(plan §9.3, §19.2): Implement the Device Authorization Grant against the
/// homeserver's MAS. `matrix-sdk`'s `OAuth` initiates the grant and returns a
/// `verification_uri` / `user_code`; the device code is held in memory with a
/// TTL and polled by [`connect_poll`]. This requires a live homeserver (and a
/// MAS that supports the device grant without a QR code), so it is not
/// verifiable here. Returning an error keeps the contract honest instead of
/// fabricating a `ConnectResponse`.
pub async fn connect(
    State(_state): State<AppState>,
    session: Session,
    token: CsrfToken,
    headers: HeaderMap,
) -> Result<Json<ConnectResponse>, ApiError> {
    require_user(&session).await?;
    verify_csrf(&token, &headers)?;

    Err(ApiError::Conflict(
        "interactive OAuth connect is not yet implemented (requires a live homeserver; see plan §9.3/§19.2)"
            .to_owned(),
    ))
}

/// `GET /api/v1/matrix/connect/poll` — poll the OAuth outcome.
///
/// TODO(plan §9.3, §19.2): Return the status of the in-flight device grant
/// (`pending` / `connected` / `expired` / `error`). Until [`connect`] is
/// implemented there is never a flow in progress, so this reports an error
/// with an explanatory message rather than `pending`.
pub async fn connect_poll(
    State(_state): State<AppState>,
    session: Session,
) -> Result<Json<ConnectPollResponse>, ApiError> {
    require_user(&session).await?;

    Ok(Json(ConnectPollResponse {
        status: ConnectPollStatus::Error,
        message: Some(
            "interactive OAuth is not yet implemented (requires a live homeserver; see plan §9.3/§19.2)"
                .to_owned(),
        ),
    }))
}

/// `POST /api/v1/matrix/disconnect` — clear the persisted session (§9.8).
///
/// Clears `matrix_state` tokens and identity and sets status `disconnected`.
/// Webhook tokens and notification history are left untouched. Requires
/// `confirm: true`.
///
/// Note: the live-client side effects of §9.8 (calling `OAuth::logout()` and
/// deleting the SDK state/crypto stores) are deferred with the OAuth work,
/// since logout requires a live session. Clearing local session state here is
/// fully implemented and is what gates session restore on the next start.
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

    db_matrix::clear_session(&state.pool)?;
    tracing::info!("Matrix session disconnected by admin");
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
        .matrix_client
        .as_ref()
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

    Ok(Json(serde_json::json!({ "status": "joined" })))
}

/// `POST /api/v1/matrix/test-message` — send a test message to a destination.
///
/// Delivers synchronously through the shared [`MessageSender`], so it exercises
/// the exact path queued notifications take. Returns 400 for an unknown
/// destination or a permanent send failure, and 409 when delivery fails
/// transiently (e.g. Matrix is not connected).
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
    match state.sender.send(&req.destination, body).await {
        Ok(()) => Ok(Json(serde_json::json!({ "status": "sent" }))),
        Err(crate::queue::sender::SendError::Permanent(msg)) => Err(ApiError::BadRequest(msg)),
        Err(crate::queue::sender::SendError::Transient(msg)) => Err(ApiError::Conflict(msg)),
    }
}

/// `POST /api/v1/matrix/recovery-key` — submit the E2EE recovery key (§9.5).
///
/// TODO(plan §9.5, §19.4): Pass the recovery key to the SDK's key-backup
/// restore (`Encryption::recover_encryption_state()` or equivalent), then mark
/// E2EE bootstrapped and verify an encrypted test send. This bootstrap is the
/// high-risk, live-homeserver-dependent part of the phase and cannot be
/// verified here, so the key is accepted syntactically but the operation
/// reports "not yet implemented". The key is never persisted (§16.11).
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
        "E2EE recovery-key bootstrap is not yet implemented (requires a live homeserver; see plan §9.5/§19.4)"
            .to_owned(),
    ))
}
