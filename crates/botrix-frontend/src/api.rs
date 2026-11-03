//! HTTP API client helpers for communicating with the `botrix-server` backend.

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

/// Perform a `PUT` request with a JSON body and deserialize the JSON response.
pub async fn put<B: Serialize, T: for<'de> Deserialize<'de>>(
    path: &str,
    body: &B,
) -> Result<T, ApiError> {
    let response = Request::put(path)
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

/// Perform a `DELETE` request and return the HTTP status code.
pub async fn delete(path: &str) -> Result<u16, ApiError> {
    let response = Request::delete(path).send().await?;

    if !response.ok() {
        let status = response.status();
        let message = response.text().await.unwrap_or_default();
        return Err(ApiError::Server { status, message });
    }

    Ok(response.status())
}
