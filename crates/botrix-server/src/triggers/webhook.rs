//! Webhook payload for command triggers.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use async_trait::async_trait;
use serde::Serialize;
use thiserror::Error;

/// Errors raised while preparing a webhook call.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum WebhookError {
    /// A `{{secret:NAME}}` placeholder named a secret that is not defined.
    /// Carries only the secret name, never a value.
    #[error("missing secret: {0}")]
    MissingSecret(String),
    /// The webhook did not respond in time.
    #[error("webhook timed out")]
    Timeout,
    /// The response body exceeded the allowed size.
    #[error("webhook response too large")]
    ResponseTooLarge,
    /// The target resolved to an address that is not allowed.
    #[error("webhook address is blocked")]
    BlockedAddress,
    /// The webhook URL is invalid or not allowed.
    #[error("invalid webhook url")]
    InvalidUrl,
    /// A network or protocol failure. Carries no detail, since it could
    /// include secrets from the URL or headers.
    #[error("webhook transport error")]
    Transport,
}

/// An outgoing webhook HTTP request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookRequest {
    /// HTTP method, e.g. `POST`.
    pub method: String,
    /// Fully resolved target URL.
    pub url: String,
    /// Header name/value pairs.
    pub headers: Vec<(String, String)>,
    /// JSON body.
    pub body: serde_json::Value,
}

/// A webhook HTTP response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookResponse {
    /// HTTP status code.
    pub status: u16,
    /// Raw response body.
    pub body: Vec<u8>,
}

/// Performs webhook HTTP calls.
#[async_trait]
pub trait WebhookCaller: Send + Sync {
    /// Send `req` and return the response.
    async fn call(&self, req: WebhookRequest) -> Result<WebhookResponse, WebhookError>;
}

/// In-memory [`WebhookCaller`] that records requests and replays queued results.
#[derive(Debug, Default)]
pub struct FakeCaller {
    requests: Mutex<Vec<WebhookRequest>>,
    results: Mutex<VecDeque<Result<WebhookResponse, WebhookError>>>,
}

impl FakeCaller {
    /// Creates a fake with no queued results.
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues a successful response for the next call.
    pub fn push_response(&self, response: WebhookResponse) {
        self.results.lock().unwrap().push_back(Ok(response));
    }

    /// Queues an error for the next call.
    pub fn push_error(&self, error: WebhookError) {
        self.results.lock().unwrap().push_back(Err(error));
    }

    /// Returns all requests received so far, in order.
    pub fn requests(&self) -> Vec<WebhookRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl WebhookCaller for FakeCaller {
    async fn call(&self, req: WebhookRequest) -> Result<WebhookResponse, WebhookError> {
        self.requests.lock().unwrap().push(req);
        // With nothing queued, report a transport failure.
        self.results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(WebhookError::Transport))
    }
}

/// Replaces `{{secret:NAME}}` placeholders in `value` with entries from `secrets`.
pub fn resolve_secrets(
    value: &str,
    secrets: &HashMap<String, String>,
) -> Result<String, WebhookError> {
    const OPEN: &str = "{{secret:";
    const CLOSE: &str = "}}";

    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    // Single left-to-right pass over the input; substituted values are pushed
    // to `out` and never scanned again.
    while let Some(start) = rest.find(OPEN) {
        let after_open = &rest[start + OPEN.len()..];
        let Some(end) = after_open.find(CLOSE) else {
            // Unterminated placeholder: keep the remaining text as-is.
            break;
        };
        let name = &after_open[..end];
        let secret = secrets
            .get(name)
            .ok_or_else(|| WebhookError::MissingSecret(name.to_string()))?;
        out.push_str(&rest[..start]);
        out.push_str(secret);
        rest = &after_open[end + CLOSE.len()..];
    }
    out.push_str(rest);
    Ok(out)
}

/// JSON body sent to a command trigger's webhook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandPayload {
    /// Command name without the prefix.
    pub command: String,
    /// Everything after the command, as one string.
    pub arguments: String,
    /// Arguments split into words.
    pub args: Vec<String>,
    /// The original message text.
    pub raw_message: String,
    /// Matrix user ID of the sender.
    pub sender: String,
    /// Matrix room ID.
    pub room_id: String,
    /// Matrix event ID.
    pub event_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> WebhookRequest {
        WebhookRequest {
            method: "POST".into(),
            url: "https://example.org/hook".into(),
            headers: vec![("X-Test".into(), "1".into())],
            body: serde_json::json!({"a": 1}),
        }
    }

    #[tokio::test]
    async fn fake_caller_records_request_and_returns_queued_response() {
        // Arrange
        let fake = FakeCaller::new();
        fake.push_response(WebhookResponse {
            status: 200,
            body: b"ok".to_vec(),
        });

        // Act
        let res = fake.call(request()).await.unwrap();

        // Assert
        assert_eq!(res.status, 200);
        assert_eq!(res.body, b"ok".to_vec());
        assert_eq!(fake.requests(), vec![request()]);
    }

    #[tokio::test]
    async fn fake_caller_returns_queued_error() {
        // Arrange
        let fake = FakeCaller::new();
        fake.push_error(WebhookError::Timeout);

        // Act
        let res = fake.call(request()).await;

        // Assert
        assert_eq!(res, Err(WebhookError::Timeout));
        assert_eq!(fake.requests().len(), 1);
    }

    fn secrets(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn replaces_a_single_placeholder() {
        // Arrange
        let s = secrets(&[("TOKEN", "abc123")]);

        // Act
        let out = resolve_secrets("Bearer {{secret:TOKEN}}", &s);

        // Assert
        assert_eq!(out, Ok("Bearer abc123".to_string()));
    }

    #[test]
    fn replaces_multiple_placeholders() {
        // Arrange
        let s = secrets(&[("A", "1"), ("B", "2")]);

        // Act
        let out = resolve_secrets("{{secret:A}}-{{secret:B}}-{{secret:A}}", &s);

        // Assert
        assert_eq!(out, Ok("1-2-1".to_string()));
    }

    #[test]
    fn leaves_text_without_placeholders_unchanged() {
        // Arrange
        let s = secrets(&[("A", "1")]);

        // Act
        let out = resolve_secrets("plain {value} {{other}} {{secret:", &s);

        // Assert
        assert_eq!(out, Ok("plain {value} {{other}} {{secret:".to_string()));
    }

    #[test]
    fn missing_secret_returns_its_name() {
        // Arrange
        let s = secrets(&[("A", "1")]);

        // Act
        let out = resolve_secrets("x {{secret:NOPE}}", &s);

        // Assert
        assert_eq!(out, Err(WebhookError::MissingSecret("NOPE".to_string())));
    }

    #[test]
    fn error_output_never_contains_secret_values() {
        // Arrange
        let s = secrets(&[("A", "super-secret-value")]);

        // Act
        let err = resolve_secrets("{{secret:A}} {{secret:MISSING}}", &s).unwrap_err();

        // Assert
        let display = err.to_string();
        let debug = format!("{err:?}");
        assert!(display.contains("MISSING"));
        assert!(!display.contains("super-secret-value"));
        assert!(!debug.contains("super-secret-value"));
    }

    #[test]
    fn substituted_values_are_not_rescanned() {
        // Arrange
        let s = secrets(&[("A", "{{secret:B}}"), ("B", "leaked")]);

        // Act
        let out = resolve_secrets("{{secret:A}}", &s);

        // Assert
        assert_eq!(out, Ok("{{secret:B}}".to_string()));
    }

    #[test]
    fn serializes_with_exactly_the_expected_keys() {
        // Arrange
        let payload = CommandPayload {
            command: "deploy".into(),
            arguments: "prod now".into(),
            args: vec!["prod".into(), "now".into()],
            raw_message: "!deploy prod now".into(),
            sender: "@a:example.org".into(),
            room_id: "!r:example.org".into(),
            event_id: "$e".into(),
        };

        // Act
        let value = serde_json::to_value(&payload).unwrap();

        // Assert
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "args",
                "arguments",
                "command",
                "event_id",
                "raw_message",
                "room_id",
                "sender"
            ]
        );
        assert_eq!(value["args"], serde_json::json!(["prod", "now"]));
    }
}
