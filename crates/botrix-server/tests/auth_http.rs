//! HTTP integration tests for the Phase 3 authentication and session API.
//!
//! Drives the fully-layered router (session + CSRF) via `tower::ServiceExt`,
//! threading cookies between requests with a minimal in-test cookie jar.

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

const SESSION_SECRET: &str = "0123456789abcdef0123456789abcdef";

/// A tiny cookie jar: maps cookie name -> value, updated from `Set-Cookie`.
#[derive(Default)]
struct Jar {
    cookies: HashMap<String, String>,
}

impl Jar {
    /// Absorb all `Set-Cookie` headers from a response.
    fn absorb(&mut self, response: &Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            if let Ok(s) = value.to_str() {
                // Cookie format: "name=value; Attr=...; ..." — take the first pair.
                if let Some(pair) = s.split(';').next() {
                    if let Some((name, val)) = pair.split_once('=') {
                        self.cookies
                            .insert(name.trim().to_owned(), val.trim().to_owned());
                    }
                }
            }
        }
    }

    /// Render the `Cookie` header value for the next request.
    fn header(&self) -> String {
        self.cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// Build an in-memory, migrated app router for tests.
async fn test_app() -> (Router, sqlx::SqlitePool) {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("run migrations");

    // A minimal valid config.
    let config = botrix_server::config::Config {
        matrix: botrix_common::config::MatrixConfig {
            homeserver: "https://matrix.example.com".to_owned(),
            destinations: Default::default(),
        },
        webhooks: Default::default(),
        server: botrix_common::config::ServerConfig::default(),
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

/// Fetch a CSRF token, returning (token, jar) with the CSRF cookie stored.
async fn get_csrf(app: &Router, jar: &mut Jar) -> String {
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/csrf")
        .header(header::COOKIE, jar.header())
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    jar.absorb(&resp);
    let json = body_json(resp).await;
    json["token"].as_str().unwrap().to_owned()
}

/// Issue a JSON POST with the current jar and CSRF token.
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

#[tokio::test]
async fn health_is_public() {
    let (app, _pool) = test_app().await;
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn me_is_unauthorized_without_session() {
    let (app, _pool) = test_app().await;
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/me")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn setup_creates_admin_then_rejects_second_call() {
    let (app, _pool) = test_app().await;
    let mut jar = Jar::default();

    // Arrange: obtain a CSRF token.
    let csrf = get_csrf(&app, &mut jar).await;

    // Act: first setup succeeds.
    let resp = post_json(
        &app,
        &mut jar,
        "/api/v1/setup",
        &csrf,
        serde_json::json!({ "username": "alice", "password": "hunter2pass" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Act: second setup is rejected with 409.
    let csrf2 = get_csrf(&app, &mut jar).await;
    let resp2 = post_json(
        &app,
        &mut jar,
        "/api/v1/setup",
        &csrf2,
        serde_json::json!({ "username": "bob", "password": "sesame12" }),
    )
    .await;
    assert_eq!(resp2.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn setup_without_csrf_is_forbidden() {
    let (app, _pool) = test_app().await;

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/setup")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({ "username": "alice", "password": "hunter2pass" }).to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn login_logout_flow() {
    let (app, _pool) = test_app().await;
    let mut jar = Jar::default();

    // Arrange: create the admin.
    let csrf = get_csrf(&app, &mut jar).await;
    let resp = post_json(
        &app,
        &mut jar,
        "/api/v1/setup",
        &csrf,
        serde_json::json!({ "username": "alice", "password": "hunter2pass" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Act: log in with correct credentials.
    let csrf = get_csrf(&app, &mut jar).await;
    let resp = post_json(
        &app,
        &mut jar,
        "/api/v1/auth/login",
        &csrf,
        serde_json::json!({ "username": "alice", "password": "hunter2pass" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Assert: /me now returns the username.
    let req = Request::builder()
        .uri("/api/v1/auth/me")
        .header(header::COOKIE, jar.header())
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["username"], "alice");

    // Act: log out.
    let csrf = get_csrf(&app, &mut jar).await;
    let resp = post_json(
        &app,
        &mut jar,
        "/api/v1/auth/logout",
        &csrf,
        serde_json::json!({}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Assert: /me is unauthorized again.
    let req = Request::builder()
        .uri("/api/v1/auth/me")
        .header(header::COOKIE, jar.header())
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_with_wrong_password_is_unauthorized() {
    let (app, _pool) = test_app().await;
    let mut jar = Jar::default();

    let csrf = get_csrf(&app, &mut jar).await;
    post_json(
        &app,
        &mut jar,
        "/api/v1/setup",
        &csrf,
        serde_json::json!({ "username": "alice", "password": "hunter2pass" }),
    )
    .await;

    let csrf = get_csrf(&app, &mut jar).await;
    let resp = post_json(
        &app,
        &mut jar,
        "/api/v1/auth/login",
        &csrf,
        serde_json::json!({ "username": "alice", "password": "wrong-password" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn settings_require_authentication() {
    let (app, _pool) = test_app().await;
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/settings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
