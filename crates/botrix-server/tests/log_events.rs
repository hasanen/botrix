//! Integration tests: operational events are written to `log_events` and
//! surfaced through `GET /api/v1/logs`, without leaking secrets.

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

const SESSION_SECRET: &str = "0123456789abcdef0123456789abcdef";
const PASSWORD: &str = "hunter2pass";

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

async fn test_app() -> Router {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "botrix-logevents-{}-{}.db",
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
        server: botrix_common::config::ServerConfig {
            host: "127.0.0.1".to_owned(),
            port: 8080,
            max_body_bytes: 1024,
            public_url: None,
        },
    };
    botrix_server::server::build_app(pool, config, SESSION_SECRET, false)
        .await
        .expect("build app")
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

async fn send(
    app: &Router,
    jar: &mut Jar,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> Response {
    let csrf = get_csrf(app, jar).await;
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-csrf-token", csrf)
        .header(header::COOKIE, jar.header());
    let body = match body {
        Some(json) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    let resp = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    jar.absorb(&resp);
    resp
}

fn creds(username: &str, password: &str) -> Value {
    serde_json::json!({ "username": username, "password": password })
}

/// Create the admin and log in; the returned jar holds the session.
async fn authenticate(app: &Router) -> Jar {
    let mut jar = Jar::default();
    send(
        app,
        &mut jar,
        "POST",
        "/api/v1/setup",
        Some(creds("alice", PASSWORD)),
    )
    .await;
    let resp = send(
        app,
        &mut jar,
        "POST",
        "/api/v1/auth/login",
        Some(creds("alice", PASSWORD)),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    jar
}

/// Fetch every log event (newest first) as JSON objects.
async fn events(app: &Router, jar: &Jar) -> Vec<Value> {
    let req = Request::builder()
        .uri("/api/v1/logs?per_page=100")
        .header(header::COOKIE, jar.header())
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    body_json(resp).await["items"].as_array().unwrap().clone()
}

fn find<'a>(events: &'a [Value], component: &str, message_part: &str) -> Option<&'a Value> {
    events.iter().find(|e| {
        e["component"] == component && e["message"].as_str().unwrap().contains(message_part)
    })
}

/// No event may contain any of the given secrets anywhere.
fn assert_no_secret(events: &[Value], secrets: &[&str]) {
    let dump = serde_json::to_string(events).unwrap();
    for secret in secrets {
        assert!(
            !dump.contains(secret),
            "log events leaked '{secret}': {dump}"
        );
    }
}

#[tokio::test]
async fn admin_creation_login_and_logout_are_logged() {
    // Arrange + Act
    let app = test_app().await;
    let mut jar = authenticate(&app).await;
    send(&app, &mut jar, "POST", "/api/v1/auth/logout", None).await;
    let mut viewer = Jar::default();
    send(
        &app,
        &mut viewer,
        "POST",
        "/api/v1/auth/login",
        Some(creds("alice", PASSWORD)),
    )
    .await;
    let log = events(&app, &viewer).await;

    // Assert
    let created = find(&log, "auth", "admin account created").expect("setup event");
    assert_eq!(created["level"], "INFO");
    let login = find(&log, "auth", "login succeeded").expect("login event");
    assert_eq!(login["level"], "INFO");
    assert!(find(&log, "auth", "logged out").is_some());
}

#[tokio::test]
async fn failed_login_is_logged_without_the_attempted_username_or_password() {
    // Arrange
    let app = test_app().await;
    let admin = authenticate(&app).await;

    // Act
    let mut attacker = Jar::default();
    let resp = send(
        &app,
        &mut attacker,
        "POST",
        "/api/v1/auth/login",
        Some(creds("mallory", "wrong-password-123")),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let log = events(&app, &admin).await;

    // Assert
    let failed = find(&log, "auth", "login failed").expect("failed login event");
    assert_eq!(failed["level"], "WARN");
    assert_no_secret(&log, &["mallory", "wrong-password-123", PASSWORD]);
}

#[tokio::test]
async fn password_and_settings_changes_are_logged_without_secrets() {
    // Arrange
    let app = test_app().await;
    let mut jar = authenticate(&app).await;

    // Act
    let resp = send(
        &app,
        &mut jar,
        "POST",
        "/api/v1/settings/password",
        Some(serde_json::json!({
            "current_password": PASSWORD,
            "new_password": "brand-new-secret-9"
        })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let resp = send(
        &app,
        &mut jar,
        "PUT",
        "/api/v1/settings",
        Some(serde_json::json!({ "idle_timeout_secs": 1800, "absolute_lifetime_secs": 7200 })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let log = events(&app, &jar).await;

    // Assert
    assert!(find(&log, "auth", "password changed").is_some());
    assert!(find(&log, "auth", "session settings updated").is_some());
    assert_no_secret(&log, &[PASSWORD, "brand-new-secret-9"]);
}

#[tokio::test]
async fn token_generation_and_revocation_are_logged_without_the_token() {
    // Arrange
    let app = test_app().await;
    let mut jar = authenticate(&app).await;

    // Act
    let resp = send(&app, &mut jar, "POST", "/api/v1/webhooks/wud/token", None).await;
    let token = body_json(resp).await["token"].as_str().unwrap().to_owned();
    send(&app, &mut jar, "DELETE", "/api/v1/webhooks/wud/token", None).await;
    let log = events(&app, &jar).await;

    // Assert
    let generated = find(&log, "webhook", "token generated").expect("generate event");
    assert!(generated["details"].as_str().unwrap().contains("wud"));
    assert!(find(&log, "webhook", "token revoked").is_some());
    assert_no_secret(&log, &[&token]);
}

async fn notify(app: &Router, token: Option<&str>, body: &str) -> StatusCode {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/api/v1/webhooks/notify/wud")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(t) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    app.clone()
        .oneshot(builder.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn rejected_payload_from_an_authenticated_sender_is_logged() {
    // Arrange
    let app = test_app().await;
    let mut jar = authenticate(&app).await;
    let resp = send(&app, &mut jar, "POST", "/api/v1/webhooks/wud/token", None).await;
    let token = body_json(resp).await["token"].as_str().unwrap().to_owned();

    // Act: valid token, but the payload lacks the template's ${name}.
    let status = notify(&app, Some(&token), r#"{"other":"x"}"#).await;
    let log = events(&app, &jar).await;

    // Assert
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let rejected = find(&log, "webhook", "payload rejected").expect("rejection event");
    assert_eq!(rejected["level"], "WARN");
    assert!(rejected["details"].as_str().unwrap().contains("wud"));
}

#[tokio::test]
async fn unauthenticated_webhook_calls_are_not_written_to_the_database() {
    // Arrange: a token exists, so wrong/missing tokens are real 401s.
    let app = test_app().await;
    let mut jar = authenticate(&app).await;
    send(&app, &mut jar, "POST", "/api/v1/webhooks/wud/token", None).await;
    let before = events(&app, &jar).await.len();

    // Act: anyone on the network can do this, so it must not fill the log.
    for _ in 0..5 {
        assert_eq!(notify(&app, None, "{}").await, StatusCode::UNAUTHORIZED);
        assert_eq!(
            notify(&app, Some("not-the-token"), "{}").await,
            StatusCode::UNAUTHORIZED
        );
    }

    // Assert
    assert_eq!(events(&app, &jar).await.len(), before);
}

#[tokio::test]
async fn disconnect_and_failed_test_message_are_logged() {
    // Arrange
    let app = test_app().await;
    let mut jar = authenticate(&app).await;

    // Act
    send(
        &app,
        &mut jar,
        "POST",
        "/api/v1/matrix/disconnect",
        Some(serde_json::json!({ "confirm": true })),
    )
    .await;
    send(
        &app,
        &mut jar,
        "POST",
        "/api/v1/matrix/test-message",
        Some(serde_json::json!({ "destination": "containers" })),
    )
    .await;
    let log = events(&app, &jar).await;

    // Assert: the test app has no Matrix session, so the send fails.
    assert!(find(&log, "matrix", "disconnected by admin").is_some());
    let failed = find(&log, "matrix", "test message failed").expect("test failure event");
    assert_eq!(failed["level"], "WARN");
}

#[tokio::test]
async fn wrong_current_password_is_a_400_not_a_session_401_and_is_logged() {
    // Arrange: a 401 here would make the UI think the session expired and
    // redirect to the login page on a simple typo.
    let app = test_app().await;
    let mut jar = authenticate(&app).await;

    // Act
    let resp = send(
        &app,
        &mut jar,
        "POST",
        "/api/v1/settings/password",
        Some(serde_json::json!({
            "current_password": "not-my-password",
            "new_password": "brand-new-secret-9"
        })),
    )
    .await;
    let log = events(&app, &jar).await;

    // Assert
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let failed = find(&log, "auth", "password change failed").expect("failure event");
    assert_eq!(failed["level"], "WARN");
    assert_no_secret(&log, &["not-my-password", "brand-new-secret-9"]);
}
