//! HTTP integration tests for the Phase 7 read endpoints:
//! notification history, log events, and application info.

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

async fn test_app() -> (Router, botrix_server::db::Pool) {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "botrix-apiread-{}-{}.db",
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
    let config = botrix_server::config::Config {
        matrix: botrix_common::config::MatrixConfig {
            homeserver: "https://matrix.example.com".to_owned(),
            destinations,
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

async fn authenticate(app: &Router) -> Jar {
    let mut jar = Jar::default();
    let creds = serde_json::json!({ "username": "alice", "password": "hunter2pass" });
    let csrf = get_csrf(app, &mut jar).await;
    post_json(app, &mut jar, "/api/v1/setup", &csrf, creds.clone()).await;
    let csrf = get_csrf(app, &mut jar).await;
    let resp = post_json(app, &mut jar, "/api/v1/auth/login", &csrf, creds).await;
    assert_eq!(resp.status(), StatusCode::OK);
    jar
}

async fn get(app: &Router, jar: Option<&Jar>, uri: &str) -> Response {
    let mut builder = Request::builder().uri(uri);
    if let Some(jar) = jar {
        builder = builder.header(header::COOKIE, jar.header());
    }
    app.clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

fn insert_notification(pool: &botrix_server::db::Pool, n: i64) {
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO notifications
               (webhook_id, destination, rendered_message, status, created_at, expires_at)
             VALUES ('wud', 'containers', ?1, 'pending', ?2, ?3)",
            rusqlite::params![format!("msg {n}"), 1_000 + n, 1_000_000 + n],
        )
        .unwrap();
}

fn insert_log(pool: &botrix_server::db::Pool, n: i64, level: &str) {
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO log_events (level, component, message, created_at)
             VALUES (?1, 'queue', ?2, ?3)",
            rusqlite::params![level, format!("event {n}"), 1_000 + n],
        )
        .unwrap();
}

#[tokio::test]
async fn read_endpoints_require_authentication() {
    // Arrange
    let (app, _pool) = test_app().await;

    // Act + Assert
    for uri in [
        "/api/v1/notifications",
        "/api/v1/logs",
        "/api/v1/settings/info",
    ] {
        let resp = get(&app, None, uri).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{uri}");
    }
}

#[tokio::test]
async fn notifications_are_paginated_newest_first() {
    // Arrange
    let (app, pool) = test_app().await;
    for n in 1..=30 {
        insert_notification(&pool, n);
    }
    let jar = authenticate(&app).await;

    // Act
    let page1 = body_json(get(&app, Some(&jar), "/api/v1/notifications").await).await;
    let page2 =
        body_json(get(&app, Some(&jar), "/api/v1/notifications?page=2&per_page=25").await).await;

    // Assert
    assert_eq!(page1["total"], 30);
    assert_eq!(page1["page"], 1);
    assert_eq!(page1["per_page"], 25);
    assert_eq!(page1["items"].as_array().unwrap().len(), 25);
    assert_eq!(page1["items"][0]["rendered_message"], "msg 30");
    assert_eq!(page1["items"][0]["status"], "pending");
    assert_eq!(page2["items"].as_array().unwrap().len(), 5);
    assert_eq!(page2["items"][4]["rendered_message"], "msg 1");
}

#[tokio::test]
async fn notifications_per_page_is_clamped_and_page_floored() {
    // Arrange
    let (app, pool) = test_app().await;
    insert_notification(&pool, 1);
    let jar = authenticate(&app).await;

    // Act
    let json = body_json(
        get(
            &app,
            Some(&jar),
            "/api/v1/notifications?page=0&per_page=100000",
        )
        .await,
    )
    .await;

    // Assert
    assert_eq!(json["page"], 1);
    assert_eq!(json["per_page"], 100);
    assert_eq!(json["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn logs_are_paginated_newest_first_with_default_page_size_of_50() {
    // Arrange: authenticating itself records events (admin created, login),
    // so seed after it and account for that baseline.
    let (app, pool) = test_app().await;
    let jar = authenticate(&app).await;
    let baseline = body_json(get(&app, Some(&jar), "/api/v1/logs").await).await["total"]
        .as_i64()
        .unwrap();
    for n in 1..=55 {
        insert_log(&pool, n, if n % 2 == 0 { "WARN" } else { "INFO" });
    }

    // Act
    let page1 = body_json(get(&app, Some(&jar), "/api/v1/logs").await).await;
    let page2 = body_json(get(&app, Some(&jar), "/api/v1/logs?page=2").await).await;

    // Assert
    assert_eq!(page1["total"], baseline + 55);
    assert_eq!(page1["per_page"], 50);
    assert_eq!(page1["items"].as_array().unwrap().len(), 50);
    assert_eq!(page1["items"][0]["message"], "event 55");
    assert_eq!(page1["items"][0]["level"], "INFO");
    assert_eq!(page1["items"][1]["level"], "WARN");
    assert_eq!(
        page2["items"].as_array().unwrap().len() as i64,
        baseline + 55 - 50
    );
}

#[tokio::test]
async fn app_info_reports_version() {
    // Arrange
    let (app, _pool) = test_app().await;
    let jar = authenticate(&app).await;

    // Act
    let resp = get(&app, Some(&jar), "/api/v1/settings/info").await;

    // Assert
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
}

async fn join(app: &Router, jar: Option<&mut Jar>, name: &str) -> Response {
    let mut builder = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/matrix/rooms/{name}/join"));
    if let Some(jar) = jar {
        let csrf = get_csrf(app, jar).await;
        builder = builder
            .header("x-csrf-token", csrf)
            .header(header::COOKIE, jar.header());
    }
    app.clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn join_requires_authentication() {
    // Arrange
    let (app, _pool) = test_app().await;

    // Act
    let resp = join(&app, None, "containers").await;

    // Assert
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn join_unknown_destination_is_bad_request() {
    // Arrange
    let (app, _pool) = test_app().await;
    let mut jar = authenticate(&app).await;

    // Act
    let resp = join(&app, Some(&mut jar), "nope").await;

    // Assert
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn join_without_matrix_session_is_conflict() {
    // Arrange: test app is built without a live Matrix client.
    let (app, _pool) = test_app().await;
    let mut jar = authenticate(&app).await;

    // Act
    let resp = join(&app, Some(&mut jar), "containers").await;

    // Assert
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}
