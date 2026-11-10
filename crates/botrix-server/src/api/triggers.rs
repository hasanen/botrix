//! Command trigger API handlers (authenticated).
//!
//! Triggers are defined in the YAML config and are read-only here, like the
//! webhook endpoints. Only secret values can be changed:
//!
//! - `GET    /api/v1/triggers`                        — list triggers + secret status
//! - `PUT    /api/v1/triggers/{command}/secrets/{name}` — set or replace a secret value
//! - `DELETE /api/v1/triggers/{command}/secrets/{name}` — clear a secret value
//!
//! Secret values are write-only: they are encrypted before storage and never
//! appear in any response or log.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use axum_csrf::CsrfToken;
use botrix_common::api::triggers::{SecretSetRequest, SecretStatusDto, TriggerInfo};
use botrix_common::config::TriggerConfig;
use botrix_common::types::LogLevel;
use tower_sessions::Session;

use super::csrf::verify_csrf;
use super::error::ApiError;
use crate::db::logs::{record, Component};
use crate::db::triggers as db_triggers;
use crate::matrix::crypto::encrypt_trigger_secret;
use crate::session;
use crate::state::AppState;
use crate::triggers::authz::effective_allowlists;
use crate::triggers::lookup::{find_trigger, profile_for};

/// Largest accepted secret value, in bytes.
const MAX_SECRET_BYTES: usize = 4096;

/// Require an authenticated session.
async fn require_user(session: &Session) -> Result<String, ApiError> {
    session::current_user(session)
        .await
        .ok_or(ApiError::Unauthorized)
}

/// Resolve `command` and a declared secret `name`, or a 404 naming what is missing.
fn find_declared_secret<'a>(
    state: &'a AppState,
    command: &str,
    name: &str,
) -> Result<(String, &'a TriggerConfig), ApiError> {
    let (key, trigger) = find_trigger(&state.config, command)
        .ok_or_else(|| ApiError::NotFound(format!("unknown trigger '{command}'")))?;
    if !trigger.secrets.iter().any(|s| s == name) {
        return Err(ApiError::NotFound(format!(
            "trigger '{command}' does not declare secret '{name}'"
        )));
    }
    Ok((key, trigger))
}

/// `GET /api/v1/triggers` — list all configured triggers with secret status.
pub async fn list(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<TriggerInfo>>, ApiError> {
    require_user(&session).await?;

    let mut items = Vec::with_capacity(state.config.triggers.len());
    for (command, trigger) in &state.config.triggers {
        let key = command.to_lowercase();
        let stored = db_triggers::list_secrets(&state.pool, &key)?;
        let secrets = trigger
            .secrets
            .iter()
            .map(|name| SecretStatusDto {
                name: name.clone(),
                is_set: stored.iter().any(|s| &s.name == name),
            })
            .collect();
        let (allowed_rooms, allowed_users) =
            effective_allowlists(trigger, profile_for(&state.config, trigger));
        items.push(TriggerInfo {
            command: key,
            description: trigger.description.clone(),
            enabled: trigger.enabled,
            method: trigger.method.clone(),
            url: trigger.url.clone(),
            header_names: trigger.headers.keys().cloned().collect(),
            access_profile: trigger.access_profile.clone(),
            allowed_rooms,
            allowed_users,
            secrets,
        });
    }
    // Stable ordering for a predictable UI.
    items.sort_by(|a, b| a.command.cmp(&b.command));
    Ok(Json(items))
}

/// `PUT /api/v1/triggers/{command}/secrets/{name}` — set or replace a secret value.
///
/// Only secret names declared on the trigger in the config are accepted.
pub async fn set_secret(
    State(state): State<AppState>,
    session: Session,
    token: CsrfToken,
    headers: HeaderMap,
    Path((command, name)): Path<(String, String)>,
    Json(body): Json<SecretSetRequest>,
) -> Result<Json<SecretStatusDto>, ApiError> {
    require_user(&session).await?;
    verify_csrf(&token, &headers)?;
    let (key, _) = find_declared_secret(&state, &command, &name)?;

    if body.value.is_empty() {
        return Err(ApiError::BadRequest(
            "secret value must not be empty".to_owned(),
        ));
    }
    if body.value.len() > MAX_SECRET_BYTES {
        return Err(ApiError::BadRequest(format!(
            "secret value must be at most {MAX_SECRET_BYTES} bytes"
        )));
    }

    let ciphertext = encrypt_trigger_secret(&state.crypto_passphrase, &body.value)?;
    db_triggers::upsert_secret(&state.pool, &key, &name, &ciphertext)?;
    // Only the command and secret name are logged, never the value.
    record(
        &state.pool,
        LogLevel::Info,
        Component::Trigger,
        "secret set",
        Some(&format!("command={key} secret={name}")),
    );

    Ok(Json(SecretStatusDto { name, is_set: true }))
}

/// `DELETE /api/v1/triggers/{command}/secrets/{name}` — clear a secret value.
///
/// Idempotent: clearing a secret that has no stored value succeeds.
pub async fn delete_secret(
    State(state): State<AppState>,
    session: Session,
    token: CsrfToken,
    headers: HeaderMap,
    Path((command, name)): Path<(String, String)>,
) -> Result<Json<SecretStatusDto>, ApiError> {
    require_user(&session).await?;
    verify_csrf(&token, &headers)?;
    let (key, _) = find_declared_secret(&state, &command, &name)?;

    if db_triggers::delete_secret(&state.pool, &key, &name)? {
        record(
            &state.pool,
            LogLevel::Info,
            Component::Trigger,
            "secret cleared",
            Some(&format!("command={key} secret={name}")),
        );
    }

    Ok(Json(SecretStatusDto {
        name,
        is_set: false,
    }))
}
