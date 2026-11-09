//! Command trigger management API handlers (authenticated).
//!
//! - `GET    /api/v1/triggers`      — list triggers
//! - `POST   /api/v1/triggers`      — create a trigger
//! - `GET    /api/v1/triggers/{id}` — fetch a trigger
//! - `PUT    /api/v1/triggers/{id}` — replace a trigger
//! - `DELETE /api/v1/triggers/{id}` — delete a trigger
//! - `POST   /api/v1/triggers/{id}/test` — dry-run a trigger against its webhook
//!
//! Secret values are write-only: they are encrypted before storage and never
//! appear in any response.
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use axum_csrf::CsrfToken;
use botrix_common::api::triggers::{
    HeaderDto, SecretInput, SecretStatusDto, TriggerDto, TriggerInput, TriggerTestRequest,
    TriggerTestResponse,
};
use tower_sessions::Session;

use super::csrf::verify_csrf;
use super::error::ApiError;
use crate::db::models::CommandTrigger;
use crate::db::triggers as db_triggers;
use crate::db::{access_profiles, Pool};
use crate::matrix::crypto::encrypt_trigger_secret;
use crate::session;
use crate::state::AppState;
use crate::triggers::handler::build_request;
use crate::triggers::respond::render_reply;
use crate::triggers::webhook::CommandPayload;

/// The built-in command that triggers may not shadow.
const RESERVED_COMMAND: &str = "help";

/// Allowed outbound HTTP methods.
const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

/// Require an authenticated session.
async fn require_user(session: &Session) -> Result<String, ApiError> {
    session::current_user(session)
        .await
        .ok_or(ApiError::Unauthorized)
}

/// Build the response DTO (headers and secret names only) for a trigger.
fn to_dto(pool: &Pool, t: CommandTrigger) -> Result<TriggerDto, ApiError> {
    let headers = db_triggers::list_headers(pool, t.id)?
        .into_iter()
        .map(|h| HeaderDto {
            name: h.name,
            value: h.value,
        })
        .collect();
    let secrets = db_triggers::list_secrets(pool, t.id)?
        .into_iter()
        .map(|s| SecretStatusDto {
            name: s.name,
            is_set: !s.ciphertext.is_empty(),
        })
        .collect();
    Ok(TriggerDto {
        id: t.id,
        command: t.command,
        description: t.description,
        enabled: t.enabled,
        method: t.method,
        url: t.url,
        response_template: t.response_template,
        access_profile_id: t.access_profile_id,
        allowed_rooms: t.allowed_rooms,
        allowed_users: t.allowed_users,
        headers,
        secrets,
    })
}

/// Validate the request and return the trimmed command and uppercased method.
fn validate(pool: &Pool, input: &TriggerInput) -> Result<(String, String), ApiError> {
    let command = input.command.trim();
    if command.is_empty() || command.chars().any(char::is_whitespace) {
        return Err(ApiError::BadRequest(
            "command must be a single non-empty word".to_owned(),
        ));
    }
    if command.to_lowercase() == RESERVED_COMMAND {
        return Err(ApiError::BadRequest(format!(
            "'{RESERVED_COMMAND}' is a reserved command"
        )));
    }
    let method = input.method.trim().to_uppercase();
    if !METHODS.contains(&method.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "method must be one of {}",
            METHODS.join(", ")
        )));
    }
    let url_ok = url::Url::parse(input.url.trim())
        .map(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
        .unwrap_or(false);
    if !url_ok {
        return Err(ApiError::BadRequest(
            "url must be a valid http or https URL".to_owned(),
        ));
    }
    if input.headers.iter().any(|h| h.name.trim().is_empty()) {
        return Err(ApiError::BadRequest(
            "header names must not be empty".to_owned(),
        ));
    }
    if input.secrets.iter().any(|s| s.name.trim().is_empty()) {
        return Err(ApiError::BadRequest(
            "secret names must not be empty".to_owned(),
        ));
    }
    if let Some(profile_id) = input.access_profile_id {
        if access_profiles::get_by_id(pool, profile_id)?.is_none() {
            return Err(ApiError::BadRequest(format!(
                "access profile {profile_id} does not exist"
            )));
        }
    }
    Ok((command.to_owned(), method))
}

/// Store headers (replacing all) and apply secret changes.
fn apply_children(state: &AppState, trigger_id: i64, input: &TriggerInput) -> Result<(), ApiError> {
    let headers: Vec<(String, String)> = input
        .headers
        .iter()
        .map(|h| (h.name.trim().to_owned(), h.value.clone()))
        .collect();
    db_triggers::set_headers(&state.pool, trigger_id, &headers)?;
    for SecretInput { name, value, clear } in &input.secrets {
        let name = name.trim();
        if *clear {
            db_triggers::delete_secret(&state.pool, trigger_id, name)?;
        } else if let Some(value) = value {
            let ciphertext = encrypt_trigger_secret(&state.crypto_passphrase, value)?;
            db_triggers::upsert_secret(&state.pool, trigger_id, name, &ciphertext)?;
        }
    }
    Ok(())
}

/// Borrow the writable fields of a validated request.
fn db_input<'a>(
    input: &'a TriggerInput,
    command: &'a str,
    method: &'a str,
) -> db_triggers::TriggerInput<'a> {
    db_triggers::TriggerInput {
        command,
        description: &input.description,
        enabled: input.enabled,
        method,
        url: input.url.trim(),
        response_template: input.response_template.as_deref(),
        access_profile_id: input.access_profile_id,
        allowed_rooms: &input.allowed_rooms,
        allowed_users: &input.allowed_users,
    }
}

/// `GET /api/v1/triggers` — list all triggers ordered by command.
pub async fn list(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<TriggerDto>>, ApiError> {
    require_user(&session).await?;
    let triggers = db_triggers::list(&state.pool)?;
    let dtos = triggers
        .into_iter()
        .map(|t| to_dto(&state.pool, t))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(dtos))
}

/// `GET /api/v1/triggers/{id}` — fetch one trigger.
pub async fn get(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> Result<Json<TriggerDto>, ApiError> {
    require_user(&session).await?;
    let trigger = db_triggers::get_by_id(&state.pool, id)?
        .ok_or_else(|| ApiError::NotFound(format!("trigger {id} not found")))?;
    Ok(Json(to_dto(&state.pool, trigger)?))
}

/// `POST /api/v1/triggers` — create a trigger.
pub async fn create(
    State(state): State<AppState>,
    session: Session,
    token: CsrfToken,
    headers: HeaderMap,
    Json(input): Json<TriggerInput>,
) -> Result<Json<TriggerDto>, ApiError> {
    require_user(&session).await?;
    verify_csrf(&token, &headers)?;
    let (command, method) = validate(&state.pool, &input)?;
    let trigger = db_triggers::create(&state.pool, &db_input(&input, &command, &method))?;
    if let Err(e) = apply_children(&state, trigger.id, &input) {
        // Do not leave a half-configured trigger behind.
        let _ = db_triggers::delete(&state.pool, trigger.id);
        return Err(e);
    }
    Ok(Json(to_dto(&state.pool, trigger)?))
}

/// `PUT /api/v1/triggers/{id}` — replace a trigger.
pub async fn update(
    State(state): State<AppState>,
    session: Session,
    token: CsrfToken,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<TriggerInput>,
) -> Result<Json<TriggerDto>, ApiError> {
    require_user(&session).await?;
    verify_csrf(&token, &headers)?;
    let (command, method) = validate(&state.pool, &input)?;
    let trigger = db_triggers::update(&state.pool, id, &db_input(&input, &command, &method))?
        .ok_or_else(|| ApiError::NotFound(format!("trigger {id} not found")))?;
    apply_children(&state, id, &input)?;
    Ok(Json(to_dto(&state.pool, trigger)?))
}

/// `DELETE /api/v1/triggers/{id}` — delete a trigger.
pub async fn delete(
    State(state): State<AppState>,
    session: Session,
    token: CsrfToken,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_user(&session).await?;
    verify_csrf(&token, &headers)?;
    if !db_triggers::delete(&state.pool, id)? {
        return Err(ApiError::NotFound(format!("trigger {id} not found")));
    }
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// Reply returned when a test call cannot be built or the webhook fails.
const REPLY_FAILED: &str = "Command failed";

/// Placeholder used for message fields in a test payload.
const TEST_PLACEHOLDER: &str = "test";

/// `POST /api/v1/triggers/{id}/test` — call the trigger's webhook with a
/// sample payload and return the reply that would be sent to the room.
///
/// Failures (unresolvable secrets, caller errors) yield a normal 200 response
/// with a generic reply and no status; details and secret values are never
/// returned.
pub async fn test(
    State(state): State<AppState>,
    session: Session,
    token: CsrfToken,
    headers: HeaderMap,
    Path(id): Path<i64>,
    body: Option<Json<TriggerTestRequest>>,
) -> Result<Json<TriggerTestResponse>, ApiError> {
    require_user(&session).await?;
    verify_csrf(&token, &headers)?;
    let trigger = db_triggers::get_by_id(&state.pool, id)?
        .ok_or_else(|| ApiError::NotFound(format!("trigger {id} not found")))?;

    let arguments = body
        .and_then(|Json(b)| b.args)
        .map(|a| a.trim().to_owned())
        .unwrap_or_default();
    let payload = CommandPayload {
        command: trigger.command.clone(),
        args: arguments.split_whitespace().map(str::to_owned).collect(),
        arguments,
        raw_message: TEST_PLACEHOLDER.to_owned(),
        sender: TEST_PLACEHOLDER.to_owned(),
        room_id: TEST_PLACEHOLDER.to_owned(),
        event_id: TEST_PLACEHOLDER.to_owned(),
    };

    let ctx = state.matrix.command_ctx();
    let failed = || TriggerTestResponse {
        status: None,
        reply: REPLY_FAILED.to_owned(),
    };
    let Ok(request) = build_request(ctx, &trigger, &payload) else {
        return Ok(Json(failed()));
    };
    let result = ctx.caller.call(request).await;
    let status = result.as_ref().ok().map(|r| r.status);
    let reply = render_reply(&result, trigger.response_template.as_deref());
    Ok(Json(TriggerTestResponse { status, reply }))
}
