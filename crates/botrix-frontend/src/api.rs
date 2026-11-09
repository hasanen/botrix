//! HTTP API client helpers for communicating with the `botrix-server` backend.

use crate::triggers_form::TriggerForm;
use botrix_common::api::triggers::{
    AccessProfileDto, AccessProfileInput, TriggerDto, TriggerInput, TriggerTestRequest,
    TriggerTestResponse,
};
use gloo_net::http::Request;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors that can occur during API calls.
#[derive(Debug, Error)]
pub enum ApiError {
    /// The HTTP request failed at the network/transport level.
    #[error("network error: {0}")]
    Network(String),
    /// The server returned a non-2xx status code.
    #[error("server error {status}: {message}")]
    Server { status: u16, message: String },
    /// The response body could not be deserialized.
    #[error("deserialization error: {0}")]
    Deserialize(String),
}

impl ApiError {
    /// A message suitable for showing to the user.
    ///
    /// For server errors this extracts the `error` field of the JSON body the
    /// API returns, falling back to the raw body (or the status code if empty).
    pub fn user_message(&self) -> String {
        match self {
            ApiError::Server { status, message } => {
                #[derive(Deserialize)]
                struct Body {
                    error: String,
                }
                if let Ok(body) = serde_json::from_str::<Body>(message) {
                    body.error
                } else if message.trim().is_empty() {
                    format!("request failed with status {status}")
                } else {
                    message.clone()
                }
            }
            other => other.to_string(),
        }
    }

    /// Whether this is an HTTP 401 (session missing or expired).
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, ApiError::Server { status: 401, .. })
    }
}

impl From<gloo_net::Error> for ApiError {
    fn from(err: gloo_net::Error) -> Self {
        ApiError::Network(err.to_string())
    }
}

/// Perform a `GET` request and deserialize the JSON response body.
pub async fn get<T: for<'de> Deserialize<'de>>(path: &str) -> Result<T, ApiError> {
    let response = Request::get(path).send().await?;

    if !response.ok() {
        let status = response.status();
        let message = response.text().await.unwrap_or_default();
        return Err(ApiError::Server { status, message });
    }

    response
        .json::<T>()
        .await
        .map_err(|e| ApiError::Deserialize(e.to_string()))
}

/// Perform a `POST` request with a JSON body and deserialize the JSON response.
pub async fn post<B: Serialize, T: for<'de> Deserialize<'de>>(
    path: &str,
    body: &B,
) -> Result<T, ApiError> {
    let response = Request::post(path)
        .json(body)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send()
        .await?;

    if !response.ok() {
        let status = response.status();
        let message = response.text().await.unwrap_or_default();
        return Err(ApiError::Server { status, message });
    }

    response
        .json::<T>()
        .await
        .map_err(|e| ApiError::Deserialize(e.to_string()))
}

/// Fetch a CSRF authenticity token, which also sets the CSRF cookie.
pub async fn fetch_csrf() -> Result<String, ApiError> {
    let resp: botrix_common::api::auth::CsrfResponse = get("/api/v1/auth/csrf").await?;
    Ok(resp.token)
}

/// Perform a state-mutating `POST` with a freshly fetched CSRF token.
///
/// Fetches `/api/v1/auth/csrf` first (setting the cookie), then submits the
/// request with the matching token in the `X-CSRF-Token` header.
pub async fn post_csrf<B: Serialize, T: for<'de> Deserialize<'de>>(
    path: &str,
    body: &B,
) -> Result<T, ApiError> {
    let csrf = fetch_csrf().await?;
    let response = Request::post(path)
        .header("x-csrf-token", &csrf)
        .json(body)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send()
        .await?;

    if !response.ok() {
        let status = response.status();
        let message = response.text().await.unwrap_or_default();
        return Err(ApiError::Server { status, message });
    }

    response
        .json::<T>()
        .await
        .map_err(|e| ApiError::Deserialize(e.to_string()))
}

/// Perform a `PUT` request with a JSON body and deserialize the JSON response.
///
/// Fetches a CSRF token first; the server requires one on all mutating calls.
pub async fn put_csrf<B: Serialize, T: for<'de> Deserialize<'de>>(
    path: &str,
    body: &B,
) -> Result<T, ApiError> {
    let csrf = fetch_csrf().await?;
    let response = Request::put(path)
        .header("x-csrf-token", &csrf)
        .json(body)
        .map_err(|e| ApiError::Network(e.to_string()))?
        .send()
        .await?;

    if !response.ok() {
        let status = response.status();
        let message = response.text().await.unwrap_or_default();
        return Err(ApiError::Server { status, message });
    }

    response
        .json::<T>()
        .await
        .map_err(|e| ApiError::Deserialize(e.to_string()))
}

/// Perform a `DELETE` request with a fresh CSRF token and deserialize the JSON response.
pub async fn delete_csrf<T: for<'de> Deserialize<'de>>(path: &str) -> Result<T, ApiError> {
    let csrf = fetch_csrf().await?;
    let response = Request::delete(path)
        .header("x-csrf-token", &csrf)
        .send()
        .await?;

    if !response.ok() {
        let status = response.status();
        let message = response.text().await.unwrap_or_default();
        return Err(ApiError::Server { status, message });
    }

    response
        .json::<T>()
        .await
        .map_err(|e| ApiError::Deserialize(e.to_string()))
}

/// List all command triggers.
pub async fn list_triggers() -> Result<Vec<TriggerDto>, ApiError> {
    get("/api/v1/triggers").await
}

/// Fetch one command trigger.
pub async fn get_trigger(id: i64) -> Result<TriggerDto, ApiError> {
    get(&format!("/api/v1/triggers/{id}")).await
}

/// Create a command trigger.
pub async fn create_trigger(input: &TriggerInput) -> Result<TriggerDto, ApiError> {
    post_csrf("/api/v1/triggers", input).await
}

/// Replace a command trigger.
pub async fn update_trigger(id: i64, input: &TriggerInput) -> Result<TriggerDto, ApiError> {
    put_csrf(&format!("/api/v1/triggers/{id}"), input).await
}

/// Delete a command trigger.
pub async fn delete_trigger(id: i64) -> Result<(), ApiError> {
    delete_csrf::<serde_json::Value>(&format!("/api/v1/triggers/{id}"))
        .await
        .map(|_| ())
}

/// Enable or disable a trigger via a full `PUT`; stored secrets stay unchanged.
pub async fn set_trigger_enabled(
    trigger: &TriggerDto,
    enabled: bool,
) -> Result<TriggerDto, ApiError> {
    let mut form = TriggerForm::from_dto(trigger);
    form.enabled = enabled;
    update_trigger(trigger.id, &form.to_input()).await
}

/// Dry-run a trigger against its webhook.
pub async fn test_trigger(id: i64, args: Option<String>) -> Result<TriggerTestResponse, ApiError> {
    post_csrf(
        &format!("/api/v1/triggers/{id}/test"),
        &TriggerTestRequest { args },
    )
    .await
}

/// List all access profiles.
pub async fn list_access_profiles() -> Result<Vec<AccessProfileDto>, ApiError> {
    get("/api/v1/access-profiles").await
}

/// Create an access profile.
pub async fn create_access_profile(
    input: &AccessProfileInput,
) -> Result<AccessProfileDto, ApiError> {
    post_csrf("/api/v1/access-profiles", input).await
}

/// Replace an access profile.
pub async fn update_access_profile(
    id: i64,
    input: &AccessProfileInput,
) -> Result<AccessProfileDto, ApiError> {
    put_csrf(&format!("/api/v1/access-profiles/{id}"), input).await
}

/// Delete an access profile.
pub async fn delete_access_profile(id: i64) -> Result<(), ApiError> {
    delete_csrf::<serde_json::Value>(&format!("/api/v1/access-profiles/{id}"))
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_message_extracts_json_error_field() {
        let err = ApiError::Server {
            status: 409,
            message: r#"{"error":"no active Matrix session"}"#.to_owned(),
        };
        assert_eq!(err.user_message(), "no active Matrix session");
    }

    #[test]
    fn user_message_falls_back_to_raw_body() {
        let err = ApiError::Server {
            status: 502,
            message: "Bad Gateway".to_owned(),
        };
        assert_eq!(err.user_message(), "Bad Gateway");
    }

    #[test]
    fn user_message_reports_status_for_empty_body() {
        let err = ApiError::Server {
            status: 500,
            message: "  ".to_owned(),
        };
        assert_eq!(err.user_message(), "request failed with status 500");
    }

    #[test]
    fn is_unauthorized_only_matches_401() {
        let unauthorized = ApiError::Server {
            status: 401,
            message: String::new(),
        };
        let forbidden = ApiError::Server {
            status: 403,
            message: String::new(),
        };
        assert!(unauthorized.is_unauthorized());
        assert!(!forbidden.is_unauthorized());
        assert!(!ApiError::Network("x".to_owned()).is_unauthorized());
    }
}
