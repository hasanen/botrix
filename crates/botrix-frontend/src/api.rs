//! HTTP API client helpers for communicating with the `botrix-server` backend.

use crate::util::trigger_secret_path;
use botrix_common::api::triggers::{SecretSetRequest, SecretStatusDto, TriggerInfo};
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

/// List the config-defined command triggers with their secret status.
pub async fn list_triggers() -> Result<Vec<TriggerInfo>, ApiError> {
    get("/api/v1/triggers").await
}

/// Set or replace one secret value of a trigger.
pub async fn set_trigger_secret(
    command: &str,
    name: &str,
    value: &str,
) -> Result<SecretStatusDto, ApiError> {
    put_csrf(
        &trigger_secret_path(command, name),
        &SecretSetRequest {
            value: value.to_owned(),
        },
    )
    .await
}

/// Clear one secret value of a trigger.
pub async fn clear_trigger_secret(command: &str, name: &str) -> Result<SecretStatusDto, ApiError> {
    delete_csrf(&trigger_secret_path(command, name)).await
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
