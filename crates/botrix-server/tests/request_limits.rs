//! Integration tests: request body size limits are enforced before parsing.

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

const SESSION_SECRET: &str = "0123456789abcdef0123456789abcdef";
/// The documented cap for authenticated API bodies (plan §16.6).
const API_LIMIT: usize = 64 * 1024;

static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

async fn test_app(max_body_bytes: usize) -> Router {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "botrix-limits-{}-{}.db",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let pool = botrix_server::db::init(path.to_str().unwrap()).expect("init db");
    let mut destinations = HashMap::new();
    destinations.insert(
        "containers".to_owned(),
        botrix_common::config::DestinationConfig {
            room_id: "!abc:example.com".to_owned(),
            enabled: true,
        },
    );
    let mut webhooks = HashMap::new();
    webhooks.insert(
        "wud".to_owned(),
        botrix_common::config::WebhookConfig {
            enabled: true,
            destination: "containers".to_owned(),
            msg_template: "Update: ${name}".to_owned(),
        },
    );
    let config = botrix_server::config::Config {
        matrix: botrix_common::config::MatrixConfig {
            homeserver: "https://matrix.example.com".to_owned(),
            destinations,
        },
        webhooks,
        triggers: Default::default(),
        access_profiles: Default::default(),
        server: botrix_common::config::ServerConfig {
            host: "127.0.0.1".to_owned(),
            port: 8080,
            max_body_bytes,
            public_url: None,
        },
    };
    botrix_server::server::build_app(pool, config, SESSION_SECRET, false)
        .await
        .expect("build app")
}

async fn csrf(app: &Router) -> (String, String) {
    let req = Request::builder()
        .uri("/api/v1/auth/csrf")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let cookies = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|s| s.split(';').next())
        .collect::<Vec<_>>()
        .join("; ");
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let token = serde_json::from_slice::<Value>(&bytes).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    (token, cookies)
}

async fn post_login(app: &Router, body: Vec<u8>) -> Response {
    let (token, cookies) = csrf(app).await;
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-csrf-token", token)
        .header(header::COOKIE, cookies)
        .body(Body::from(body))
        .unwrap();
    app.clone().oneshot(req).await.unwrap()
}

#[tokio::test]
async fn oversized_api_body_is_rejected_with_413_before_parsing() {
    // Arrange: not even valid JSON; a parse error (400/422) would mean the
    // body was read and parsed rather than rejected for its size.
    let app = test_app(262_144).await;
    let body = vec![b'x'; API_LIMIT + 1];

    // Act
    let resp = post_login(&app, body).await;

    // Assert
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn api_body_within_the_limit_is_still_processed() {
    // Arrange: ~60 KiB of valid JSON just under the cap.
    let app = test_app(262_144).await;
    let padding = "x".repeat(API_LIMIT - 1024);
    let body = serde_json::json!({ "username": padding, "password": "pw" })
        .to_string()
        .into_bytes();
    assert!(body.len() < API_LIMIT);

    // Act
    let resp = post_login(&app, body).await;

    // Assert: processed (bad credentials), not rejected for size.
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn webhook_body_limit_is_the_configured_value_not_the_api_limit() {
    // Arrange: a configured limit larger than the API cap must be honored by
    // the inbound webhook endpoint (default is 256 KiB).
    let app = test_app(256 * 1024).await;
    let body = vec![b'x'; API_LIMIT + 1024];

    // Act: no token, so the request is rejected as unauthorized, but crucially
    // not for size, because the webhook limit is larger than the API cap.
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/webhooks/notify/wud")
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();

    // Assert
    assert_ne!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}
