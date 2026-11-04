//! Integration tests: session idle and absolute timeouts are enforced and
//! changes to them apply immediately, without a server restart.

use std::collections::HashMap;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use botrix_common::api::settings::Settings;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

const SESSION_SECRET: &str = "0123456789abcdef0123456789abcdef";

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

async fn test_app() -> (Router, botrix_server::db::Pool) {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "botrix-timeouts-{}-{}.db",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let pool = botrix_server::db::init(path.to_str().unwrap()).expect("init db");
    let config = botrix_server::config::Config {
        matrix: botrix_common::config::MatrixConfig {
            homeserver: "https://matrix.example.com".to_owned(),
            destinations: HashMap::new(),
        },
        webhooks: HashMap::new(),
        server: botrix_common::config::ServerConfig {
            host: "127.0.0.1".to_owned(),
            port: 8080,
            max_body_bytes: 1024,
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
        .uri("/api/v1/auth/csrf")
        .header(header::COOKIE, jar.header())
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    jar.absorb(&resp);
    body_json(resp).await["token"].as_str().unwrap().to_owned()
}

async fn send_json(app: &Router, jar: &mut Jar, method: &str, uri: &str, body: Value) -> Response {
    let csrf = get_csrf(app, jar).await;
    let req = Request::builder()
        .method(method)
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

async fn authenticate(app: &Router) -> Jar {
    let mut jar = Jar::default();
    let creds = serde_json::json!({ "username": "alice", "password": "hunter2pass" });
    send_json(app, &mut jar, "POST", "/api/v1/setup", creds.clone()).await;
    let resp = send_json(app, &mut jar, "POST", "/api/v1/auth/login", creds).await;
    assert_eq!(resp.status(), StatusCode::OK);
    jar
}

async fn me_status(app: &Router, jar: &mut Jar) -> StatusCode {
    let req = Request::builder()
        .uri("/api/v1/auth/me")
        .header(header::COOKIE, jar.header())
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    jar.absorb(&resp);
    resp.status()
}

fn set_timeouts(pool: &botrix_server::db::Pool, idle: i64, absolute: i64) {
    botrix_server::db::settings::update_settings(
        pool,
        &Settings {
            idle_timeout_secs: idle,
            absolute_lifetime_secs: absolute,
        },
    )
    .expect("update settings");
}

#[tokio::test]
async fn session_is_valid_within_both_limits() {
    // Arrange
    let (app, _pool) = test_app().await;
    let mut jar = authenticate(&app).await;

    // Act + Assert
    assert_eq!(me_status(&app, &mut jar).await, StatusCode::OK);
}

#[tokio::test]
async fn idle_timeout_change_applies_without_restart() {
    // Arrange: log in under the default (1h) timeout, then shorten it live.
    let (app, pool) = test_app().await;
    let mut jar = authenticate(&app).await;
    assert_eq!(me_status(&app, &mut jar).await, StatusCode::OK);
    set_timeouts(&pool, 1, 3600);

    // Act: stay idle longer than the new limit.
    tokio::time::sleep(Duration::from_millis(2100)).await;

    // Assert
    assert_eq!(me_status(&app, &mut jar).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn activity_keeps_a_session_alive_past_the_idle_timeout() {
    // Arrange
    let (app, pool) = test_app().await;
    let mut jar = authenticate(&app).await;
    set_timeouts(&pool, 2, 3600);

    // Act: make a request every second for 3s (> idle timeout in total).
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(1000)).await;
        // Assert: each request is inside the idle window, so still valid.
        assert_eq!(me_status(&app, &mut jar).await, StatusCode::OK);
    }
}

#[tokio::test]
async fn absolute_lifetime_ends_a_session_even_while_active() {
    // Arrange: 1s idle, 2s absolute.
    let (app, pool) = test_app().await;
    let mut jar = authenticate(&app).await;
    set_timeouts(&pool, 1, 2);

    // Act: stay active (every 0.7s) until past the absolute lifetime.
    let mut last = StatusCode::OK;
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(700)).await;
        last = me_status(&app, &mut jar).await;
    }

    // Assert: ~3.5s elapsed > 2s absolute, despite constant activity.
    assert_eq!(last, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn expired_session_cannot_be_revived_by_a_later_request() {
    // Arrange
    let (app, pool) = test_app().await;
    let mut jar = authenticate(&app).await;
    set_timeouts(&pool, 1, 3600);
    tokio::time::sleep(Duration::from_millis(2100)).await;
    assert_eq!(me_status(&app, &mut jar).await, StatusCode::UNAUTHORIZED);

    // Act: restore a generous timeout and retry with the same cookie.
    set_timeouts(&pool, 3600, 86_400);

    // Assert: the session was destroyed, not merely rejected.
    assert_eq!(me_status(&app, &mut jar).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn settings_update_rejects_lifetimes_above_the_maximum() {
    // Arrange
    let (app, _pool) = test_app().await;
    let mut jar = authenticate(&app).await;
    let too_long = 31 * 86_400;

    // Act
    let resp = send_json(
        &app,
        &mut jar,
        "PUT",
        "/api/v1/settings",
        serde_json::json!({ "idle_timeout_secs": 60, "absolute_lifetime_secs": too_long }),
    )
    .await;

    // Assert
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
