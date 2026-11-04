//! HTTP integration tests for the Phase 4 webhook system.
//!
//! Covers the inbound notify endpoint (token auth, size, JSON, template,
//! persistence, status codes) and the authenticated token management API.

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

const SESSION_SECRET: &str = "0123456789abcdef0123456789abcdef";

/// Monotonic counter for unique temp DB filenames across tests.
static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Default)]
struct Jar {
    cookies: HashMap<String, String>,
}

impl Jar {
    fn absorb(&mut self, response: &Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            if let Ok(s) = value.to_str() {
                if let Some(pair) = s.split(';').next() {
                    if let Some((name, val)) = pair.split_once('=') {
                        self.cookies
                            .insert(name.trim().to_owned(), val.trim().to_owned());
                    }
                }
            }
        }
    }

    fn header(&self) -> String {
        self.cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// Build an app whose config has a single `wud` webhook -> `containers`.
async fn test_app() -> (Router, botrix_server::db::Pool) {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "botrix-webhooktest-{}-{}.db",
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
    // A disabled webhook to assert 404 behavior.
    webhooks.insert(
        "disabled_hook".to_owned(),
        botrix_common::config::WebhookConfig {
            enabled: false,
            destination: "containers".to_owned(),
            msg_template: "nope".to_owned(),
        },
    );

    let config = botrix_server::config::Config {
        matrix: botrix_common::config::MatrixConfig {
            homeserver: "https://matrix.example.com".to_owned(),
            destinations,
        },
        webhooks,
        server: botrix_common::config::ServerConfig {
            host: "127.0.0.1".to_owned(),
            port: 8080,
            // Small limit to test 413 easily.
            max_body_bytes: 64,
            public_url: None,
        },
    };

    let app = botrix_server::server::build_app(pool.clone(), config, SESSION_SECRET, false)
        .await
        .expect("build app");
    (app, pool)
}

async fn body_json(response: Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

async fn get_csrf(app: &Router, jar: &mut Jar) -> String {
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/csrf")
        .header(header::COOKIE, jar.header())
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    jar.absorb(&resp);
    let json = body_json(resp).await;
    json["token"].as_str().unwrap().to_owned()
}

async fn post_json(app: &Router, jar: &mut Jar, uri: &str, csrf: &str, body: Value) -> Response {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-csrf-token", csrf)
        .header(header::COOKIE, jar.header())
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    jar.absorb(&resp);
    resp
}

/// Set up admin, log in, and return an authenticated jar.
async fn authenticate(app: &Router) -> Jar {
    let mut jar = Jar::default();
    let csrf = get_csrf(app, &mut jar).await;
    post_json(
        app,
        &mut jar,
        "/api/v1/setup",
        &csrf,
        serde_json::json!({ "username": "alice", "password": "hunter2pass" }),
    )
    .await;
    let csrf = get_csrf(app, &mut jar).await;
    let resp = post_json(
        app,
        &mut jar,
        "/api/v1/auth/login",
        &csrf,
        serde_json::json!({ "username": "alice", "password": "hunter2pass" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    jar
}

/// Generate a token for `wud` and return the plaintext.
async fn generate_token(app: &Router, jar: &mut Jar) -> String {
    let csrf = get_csrf(app, jar).await;
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/webhooks/wud/token")
        .header("x-csrf-token", csrf)
        .header(header::COOKIE, jar.header())
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    jar.absorb(&resp);
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    json["token"].as_str().unwrap().to_owned()
}

/// POST to the inbound notify endpoint with an optional bearer token.
async fn notify(app: &Router, token: Option<&str>, body: &str) -> Response {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/api/v1/webhooks/notify/wud")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(t) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let req = builder.body(Body::from(body.to_owned())).unwrap();
    app.clone().oneshot(req).await.unwrap()
}

#[tokio::test]
async fn notify_without_token_is_unauthorized() {
    let (app, _pool) = test_app().await;
    let mut jar = authenticate(&app).await;
    // Token must exist for the webhook, else we can't distinguish 401 from
    // "no token configured" — generate one first, then omit it on the call.
    generate_token(&app, &mut jar).await;

    let resp = notify(&app, None, r#"{"name":"nginx"}"#).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn notify_with_wrong_token_is_unauthorized() {
    let (app, _pool) = test_app().await;
    let mut jar = authenticate(&app).await;
    generate_token(&app, &mut jar).await;

    let resp = notify(&app, Some("not-the-real-token"), r#"{"name":"nginx"}"#).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn notify_unknown_webhook_is_not_found() {
    let (app, _pool) = test_app().await;

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/webhooks/notify/does_not_exist")
        .header(header::AUTHORIZATION, "Bearer whatever")
        .body(Body::from(r#"{}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn notify_disabled_webhook_is_not_found() {
    let (app, _pool) = test_app().await;

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/webhooks/notify/disabled_hook")
        .header(header::AUTHORIZATION, "Bearer whatever")
        .body(Body::from(r#"{}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn notify_valid_token_and_payload_is_accepted_and_persisted() {
    // Arrange
    let (app, pool) = test_app().await;
    let mut jar = authenticate(&app).await;
    let token = generate_token(&app, &mut jar).await;

    // Act
    let resp = notify(&app, Some(&token), r#"{"name":"nginx"}"#).await;

    // Assert: 202 and a pending notification row with the rendered message.
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let (status, dest, msg): (String, String, String) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT status, destination, rendered_message FROM notifications WHERE webhook_id = 'wud'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(status, "pending");
    assert_eq!(dest, "containers");
    assert_eq!(msg, "Update: nginx");
}

#[tokio::test]
async fn notify_invalid_json_is_bad_request() {
    let (app, _pool) = test_app().await;
    let mut jar = authenticate(&app).await;
    let token = generate_token(&app, &mut jar).await;

    let resp = notify(&app, Some(&token), "this is not json").await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn notify_missing_template_variable_is_bad_request() {
    let (app, _pool) = test_app().await;
    let mut jar = authenticate(&app).await;
    let token = generate_token(&app, &mut jar).await;

    // Template needs ${name}; payload omits it.
    let resp = notify(&app, Some(&token), r#"{"other":"value"}"#).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn notify_oversized_body_is_payload_too_large() {
    let (app, _pool) = test_app().await;
    let mut jar = authenticate(&app).await;
    let token = generate_token(&app, &mut jar).await;

    // max_body_bytes is 64 in the test config; build a larger valid JSON body.
    let big_value = "x".repeat(200);
    let body = format!(r#"{{"name":"{big_value}"}}"#);
    let resp = notify(&app, Some(&token), &body).await;
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn token_revoke_then_notify_is_unauthorized() {
    let (app, _pool) = test_app().await;
    let mut jar = authenticate(&app).await;
    let token = generate_token(&app, &mut jar).await;

    // Confirm it works first.
    let resp = notify(&app, Some(&token), r#"{"name":"nginx"}"#).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    // Revoke the token.
    let csrf = get_csrf(&app, &mut jar).await;
    let req = Request::builder()
        .method("DELETE")
        .uri("/api/v1/webhooks/wud/token")
        .header("x-csrf-token", csrf)
        .header(header::COOKIE, jar.header())
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // The old token no longer works.
    let resp = notify(&app, Some(&token), r#"{"name":"nginx"}"#).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn webhooks_list_requires_auth_and_reports_token_status() {
    let (app, _pool) = test_app().await;

    // Unauthenticated -> 401.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/webhooks")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Authenticated -> lists webhooks; wud has no token yet.
    let mut jar = authenticate(&app).await;
    let req = Request::builder()
        .uri("/api/v1/webhooks")
        .header(header::COOKIE, jar.header())
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let wud = json
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == "wud")
        .unwrap();
    assert_eq!(wud["has_token"], false);

    // After generating a token, has_token becomes true.
    generate_token(&app, &mut jar).await;
    let req = Request::builder()
        .uri("/api/v1/webhooks")
        .header(header::COOKIE, jar.header())
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    let wud = json
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == "wud")
        .unwrap();
    assert_eq!(wud["has_token"], true);
}
