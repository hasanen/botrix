//! HTTP integration tests for the command trigger CRUD endpoints.

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
    let (app, pool, _fake) = test_app_with_fake().await;
    (app, pool)
}

async fn test_app_with_fake() -> (
    Router,
    botrix_server::db::Pool,
    std::sync::Arc<botrix_server::triggers::webhook::FakeCaller>,
) {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "botrix-apitrig-{}-{}.db",
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
        triggers: HashMap::new(),
        access_profiles: HashMap::new(),
        server: botrix_common::config::ServerConfig {
            host: "127.0.0.1".to_owned(),
            port: 8080,
            max_body_bytes: 1024,
            public_url: None,
        },
    };
    let fake = std::sync::Arc::new(botrix_server::triggers::webhook::FakeCaller::new());
    let app = botrix_server::server::build_app_with_caller(
        pool.clone(),
        config,
        SESSION_SECRET,
        false,
        fake.clone(),
    )
    .await
    .expect("build app");
    (app, pool, fake)
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

async fn send(
    app: &Router,
    jar: Option<&Jar>,
    method: &str,
    uri: &str,
    csrf: Option<&str>,
    body: Option<Value>,
) -> Response {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(jar) = jar {
        builder = builder.header(header::COOKIE, jar.header());
    }
    if let Some(csrf) = csrf {
        builder = builder.header("x-csrf-token", csrf);
    }
    let body = match body {
        Some(b) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    app.clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap()
}

const BASE: &str = "/api/v1/triggers";
const PLAINTEXT: &str = "s3cr3t-plaintext-value-xyz";

fn trigger(command: &str) -> Value {
    serde_json::json!({
        "command": command,
        "description": "Deploy the app",
        "enabled": true,
        "method": "POST",
        "url": "https://example.com/hook",
        "response_template": "{{status}}",
        "access_profile_id": null,
        "allowed_rooms": ["!a:example.com"],
        "allowed_users": ["@alice:example.com"],
        "headers": [{ "name": "Authorization", "value": "Bearer {{secret:TOKEN}}" }],
        "secrets": [{ "name": "TOKEN", "value": PLAINTEXT }],
    })
}

/// Authenticate and fetch a CSRF token.
async fn session(app: &Router) -> (Jar, String) {
    let mut jar = authenticate(app).await;
    let csrf = get_csrf(app, &mut jar).await;
    (jar, csrf)
}

async fn create(app: &Router, jar: &Jar, csrf: &str, body: Value) -> Response {
    send(app, Some(jar), "POST", BASE, Some(csrf), Some(body)).await
}

async fn text(response: Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn triggers_require_authentication() {
    // Arrange
    let (app, _pool) = test_app().await;
    // Act + Assert
    for (method, uri) in [
        ("GET", BASE.to_owned()),
        ("POST", BASE.to_owned()),
        ("GET", format!("{BASE}/1")),
        ("PUT", format!("{BASE}/1")),
        ("DELETE", format!("{BASE}/1")),
    ] {
        let body = (method == "POST" || method == "PUT").then(|| trigger("deploy"));
        let resp = send(&app, None, method, &uri, None, body).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{method} {uri}");
    }
}

#[tokio::test]
async fn mutations_without_csrf_token_are_forbidden() {
    // Arrange
    let (app, _pool) = test_app().await;
    let jar = authenticate(&app).await;
    // Act + Assert
    for (method, uri) in [
        ("POST", BASE.to_owned()),
        ("PUT", format!("{BASE}/1")),
        ("DELETE", format!("{BASE}/1")),
    ] {
        let body = (method != "DELETE").then(|| trigger("deploy"));
        let resp = send(&app, Some(&jar), method, &uri, None, body).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{method} {uri}");
    }
}

#[tokio::test]
async fn create_and_get_round_trip_without_secret_value() {
    // Arrange
    let (app, _pool) = test_app().await;
    let (jar, csrf) = session(&app).await;

    // Act
    let resp = create(&app, &jar, &csrf, trigger("Deploy")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let created_text = text(resp).await;
    let created: Value = serde_json::from_str(&created_text).unwrap();
    let id = created["id"].as_i64().unwrap();
    let get_resp = send(&app, Some(&jar), "GET", &format!("{BASE}/{id}"), None, None).await;
    assert_eq!(get_resp.status(), StatusCode::OK);
    let get_text = text(get_resp).await;
    let fetched: Value = serde_json::from_str(&get_text).unwrap();
    let list_text = text(send(&app, Some(&jar), "GET", BASE, None, None).await).await;
    let listed: Value = serde_json::from_str(&list_text).unwrap();

    // Assert
    assert_eq!(created, fetched);
    assert_eq!(fetched["command"], "deploy");
    assert_eq!(fetched["method"], "POST");
    assert_eq!(fetched["url"], "https://example.com/hook");
    assert_eq!(fetched["headers"][0]["name"], "Authorization");
    assert_eq!(fetched["headers"][0]["value"], "Bearer {{secret:TOKEN}}");
    assert_eq!(fetched["secrets"][0]["name"], "TOKEN");
    assert_eq!(fetched["secrets"][0]["is_set"], true);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    for body in [&created_text, &get_text, &list_text] {
        assert!(!body.contains(PLAINTEXT), "secret leaked: {body}");
    }
}

#[tokio::test]
async fn secret_is_stored_encrypted() {
    // Arrange
    let (app, pool) = test_app().await;
    let (jar, csrf) = session(&app).await;

    // Act
    let created = body_json(create(&app, &jar, &csrf, trigger("deploy")).await).await;
    let id = created["id"].as_i64().unwrap();

    // Assert
    let secrets = botrix_server::db::triggers::list_secrets(&pool, id).unwrap();
    assert_eq!(secrets.len(), 1);
    assert_eq!(secrets[0].name, "TOKEN");
    assert_ne!(secrets[0].ciphertext, PLAINTEXT);
    assert!(!secrets[0].ciphertext.contains(PLAINTEXT));
}

#[tokio::test]
async fn put_replaces_headers_and_leaves_omitted_secret_unchanged() {
    // Arrange
    let (app, pool) = test_app().await;
    let (jar, csrf) = session(&app).await;
    let created = body_json(create(&app, &jar, &csrf, trigger("deploy")).await).await;
    let id = created["id"].as_i64().unwrap();
    let before = botrix_server::db::triggers::list_secrets(&pool, id).unwrap()[0]
        .ciphertext
        .clone();
    let mut update = trigger("deploy");
    update["headers"] = serde_json::json!([{ "name": "X-Other", "value": "1" }]);
    update["secrets"] = serde_json::json!([]);

    // Act
    let resp = send(
        &app,
        Some(&jar),
        "PUT",
        &format!("{BASE}/{id}"),
        Some(&csrf),
        Some(update),
    )
    .await;

    // Assert
    assert_eq!(resp.status(), StatusCode::OK);
    let updated = body_json(resp).await;
    assert_eq!(updated["headers"].as_array().unwrap().len(), 1);
    assert_eq!(updated["headers"][0]["name"], "X-Other");
    assert_eq!(updated["secrets"][0]["name"], "TOKEN");
    assert_eq!(updated["secrets"][0]["is_set"], true);
    let after = botrix_server::db::triggers::list_secrets(&pool, id).unwrap();
    assert_eq!(after[0].ciphertext, before);
}

#[tokio::test]
async fn put_secret_without_value_leaves_it_unchanged_and_with_value_replaces_it() {
    // Arrange
    let (app, pool) = test_app().await;
    let (jar, csrf) = session(&app).await;
    let created = body_json(create(&app, &jar, &csrf, trigger("deploy")).await).await;
    let id = created["id"].as_i64().unwrap();
    let before = botrix_server::db::triggers::list_secrets(&pool, id).unwrap()[0]
        .ciphertext
        .clone();
    let uri = format!("{BASE}/{id}");

    // Act: no value, clear false
    let mut update = trigger("deploy");
    update["secrets"] = serde_json::json!([{ "name": "TOKEN" }]);
    let resp = send(&app, Some(&jar), "PUT", &uri, Some(&csrf), Some(update)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let unchanged = botrix_server::db::triggers::list_secrets(&pool, id).unwrap()[0]
        .ciphertext
        .clone();

    // Act: new value
    let mut update = trigger("deploy");
    update["secrets"] = serde_json::json!([{ "name": "TOKEN", "value": "another-value" }]);
    let resp = send(&app, Some(&jar), "PUT", &uri, Some(&csrf), Some(update)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let replaced = botrix_server::db::triggers::list_secrets(&pool, id).unwrap()[0]
        .ciphertext
        .clone();

    // Assert
    assert_eq!(unchanged, before);
    assert_ne!(replaced, before);
}

#[tokio::test]
async fn put_clear_removes_secret() {
    // Arrange
    let (app, pool) = test_app().await;
    let (jar, csrf) = session(&app).await;
    let created = body_json(create(&app, &jar, &csrf, trigger("deploy")).await).await;
    let id = created["id"].as_i64().unwrap();
    let mut update = trigger("deploy");
    update["secrets"] = serde_json::json!([{ "name": "TOKEN", "clear": true }]);

    // Act
    let resp = send(
        &app,
        Some(&jar),
        "PUT",
        &format!("{BASE}/{id}"),
        Some(&csrf),
        Some(update),
    )
    .await;

    // Assert
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(body_json(resp).await["secrets"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(botrix_server::db::triggers::list_secrets(&pool, id)
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn invalid_triggers_are_rejected() {
    // Arrange
    let (app, _pool) = test_app().await;
    let (jar, csrf) = session(&app).await;
    let with = |key: &str, value: Value| {
        let mut t = trigger("other");
        t[key] = value;
        t
    };
    let cases = [
        ("help", with("command", "help".into())),
        ("HELP", with("command", "HeLp".into())),
        ("empty", with("command", "".into())),
        ("space", with("command", "two words".into())),
        ("method", with("method", "TRACE".into())),
        ("url scheme", with("url", "ftp://example.com/x".into())),
        ("url junk", with("url", "not a url".into())),
        ("profile", with("access_profile_id", 9999.into())),
    ];

    // Act + Assert
    for (label, body) in cases {
        let resp = create(&app, &jar, &csrf, body).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{label}");
    }
    let listed = body_json(send(&app, Some(&jar), "GET", BASE, None, None).await).await;
    assert!(listed.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn update_validates_input_too() {
    // Arrange
    let (app, _pool) = test_app().await;
    let (jar, csrf) = session(&app).await;
    let created = body_json(create(&app, &jar, &csrf, trigger("deploy")).await).await;
    let mut update = trigger("help");
    update["secrets"] = serde_json::json!([]);

    // Act
    let resp = send(
        &app,
        Some(&jar),
        "PUT",
        &format!("{BASE}/{}", created["id"]),
        Some(&csrf),
        Some(update),
    )
    .await;

    // Assert
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn duplicate_command_is_conflict() {
    // Arrange
    let (app, _pool) = test_app().await;
    let (jar, csrf) = session(&app).await;
    create(&app, &jar, &csrf, trigger("deploy")).await;
    let other = body_json(create(&app, &jar, &csrf, trigger("build")).await).await;

    // Act
    let dup_create = create(&app, &jar, &csrf, trigger("DEPLOY")).await;
    let dup_update = send(
        &app,
        Some(&jar),
        "PUT",
        &format!("{BASE}/{}", other["id"]),
        Some(&csrf),
        Some(trigger("deploy")),
    )
    .await;

    // Assert
    assert_eq!(dup_create.status(), StatusCode::CONFLICT);
    assert_eq!(dup_update.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn known_access_profile_is_accepted() {
    // Arrange
    let (app, pool) = test_app().await;
    let (jar, csrf) = session(&app).await;
    let profile = botrix_server::db::access_profiles::create(&pool, "ops", &[], &[]).unwrap();
    let mut body = trigger("deploy");
    body["access_profile_id"] = profile.id.into();

    // Act
    let resp = create(&app, &jar, &csrf, body).await;

    // Assert
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["access_profile_id"], profile.id);
}

#[tokio::test]
async fn delete_then_get_is_not_found() {
    // Arrange
    let (app, _pool) = test_app().await;
    let (jar, csrf) = session(&app).await;
    let created = body_json(create(&app, &jar, &csrf, trigger("deploy")).await).await;
    let uri = format!("{BASE}/{}", created["id"]);

    // Act
    let deleted = send(&app, Some(&jar), "DELETE", &uri, Some(&csrf), None).await;
    let get = send(&app, Some(&jar), "GET", &uri, None, None).await;
    let delete_again = send(&app, Some(&jar), "DELETE", &uri, Some(&csrf), None).await;
    let put = send(
        &app,
        Some(&jar),
        "PUT",
        &uri,
        Some(&csrf),
        Some(trigger("deploy")),
    )
    .await;

    // Assert
    assert_eq!(deleted.status(), StatusCode::OK);
    assert_eq!(get.status(), StatusCode::NOT_FOUND);
    assert_eq!(delete_again.status(), StatusCode::NOT_FOUND);
    assert_eq!(put.status(), StatusCode::NOT_FOUND);
}

// ---- POST /api/v1/triggers/{id}/test ----

use botrix_server::triggers::webhook::{WebhookError, WebhookResponse};

fn test_uri(id: i64) -> String {
    format!("{BASE}/{id}/test")
}

async fn create_id(app: &Router, jar: &Jar, csrf: &str, body: Value) -> i64 {
    body_json(create(app, jar, csrf, body).await).await["id"]
        .as_i64()
        .unwrap()
}

#[tokio::test]
async fn test_endpoint_requires_authentication_and_csrf() {
    // Arrange
    let (app, _pool) = test_app().await;
    let anon = send(
        &app,
        None,
        "POST",
        &test_uri(1),
        None,
        Some(serde_json::json!({})),
    )
    .await;
    let jar = authenticate(&app).await;

    // Act
    let no_csrf = send(
        &app,
        Some(&jar),
        "POST",
        &test_uri(1),
        None,
        Some(serde_json::json!({})),
    )
    .await;

    // Assert
    assert_eq!(anon.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(no_csrf.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_endpoint_unknown_id_is_not_found() {
    // Arrange
    let (app, _pool, fake) = test_app_with_fake().await;
    let (jar, csrf) = session(&app).await;

    // Act
    let resp = send(
        &app,
        Some(&jar),
        "POST",
        &test_uri(9999),
        Some(&csrf),
        Some(serde_json::json!({})),
    )
    .await;

    // Assert
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert!(fake.requests().is_empty());
}

#[tokio::test]
async fn test_endpoint_returns_rendered_preview_and_sends_resolved_secret() {
    // Arrange
    let (app, _pool, fake) = test_app_with_fake().await;
    let (jar, csrf) = session(&app).await;
    let mut body = trigger("deploy");
    body["response_template"] = "Got ${a.b}".into();
    let id = create_id(&app, &jar, &csrf, body).await;
    fake.push_response(WebhookResponse {
        status: 200,
        body: br#"{"a":{"b":"hello"}}"#.to_vec(),
    });

    // Act
    let resp = send(
        &app,
        Some(&jar),
        "POST",
        &test_uri(id),
        Some(&csrf),
        Some(serde_json::json!({ "args": "prod now" })),
    )
    .await;

    // Assert
    assert_eq!(resp.status(), StatusCode::OK);
    let response_text = text(resp).await;
    let json: Value = serde_json::from_str(&response_text).unwrap();
    assert_eq!(json["status"], 200);
    assert_eq!(json["reply"], "Got hello");
    assert!(!response_text.contains(PLAINTEXT));
    let reqs = fake.requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].method, "POST");
    assert_eq!(reqs[0].url, "https://example.com/hook");
    assert_eq!(
        reqs[0].headers,
        vec![("Authorization".to_string(), format!("Bearer {PLAINTEXT}"))]
    );
    assert_eq!(reqs[0].body["command"], "deploy");
    assert_eq!(reqs[0].body["arguments"], "prod now");
    assert_eq!(reqs[0].body["args"], serde_json::json!(["prod", "now"]));
    for key in ["raw_message", "sender", "room_id", "event_id"] {
        assert_eq!(reqs[0].body[key], "test", "{key}");
    }
}

#[tokio::test]
async fn test_endpoint_accepts_empty_body_object() {
    // Arrange
    let (app, _pool, fake) = test_app_with_fake().await;
    let (jar, csrf) = session(&app).await;
    let id = create_id(&app, &jar, &csrf, trigger("deploy")).await;
    fake.push_response(WebhookResponse {
        status: 200,
        body: b"ok".to_vec(),
    });

    // Act
    let resp = send(
        &app,
        Some(&jar),
        "POST",
        &test_uri(id),
        Some(&csrf),
        Some(serde_json::json!({})),
    )
    .await;

    // Assert
    assert_eq!(resp.status(), StatusCode::OK);
    let reqs = fake.requests();
    assert_eq!(reqs[0].body["arguments"], "");
    assert_eq!(reqs[0].body["args"], serde_json::json!([]));
}

#[tokio::test]
async fn test_endpoint_caller_error_returns_command_failed_without_secret() {
    // Arrange
    let (app, _pool, fake) = test_app_with_fake().await;
    let (jar, csrf) = session(&app).await;
    let id = create_id(&app, &jar, &csrf, trigger("deploy")).await;
    fake.push_error(WebhookError::Timeout);

    // Act
    let resp = send(
        &app,
        Some(&jar),
        "POST",
        &test_uri(id),
        Some(&csrf),
        Some(serde_json::json!({})),
    )
    .await;

    // Assert
    assert_eq!(resp.status(), StatusCode::OK);
    let response_text = text(resp).await;
    let json: Value = serde_json::from_str(&response_text).unwrap();
    assert_eq!(json["reply"], "Command failed");
    assert!(json["status"].is_null());
    assert!(!response_text.contains(PLAINTEXT));
}

#[tokio::test]
async fn test_endpoint_missing_secret_returns_command_failed() {
    // Arrange
    let (app, _pool, fake) = test_app_with_fake().await;
    let (jar, csrf) = session(&app).await;
    let mut body = trigger("deploy");
    body["secrets"] = serde_json::json!([]);
    let id = create_id(&app, &jar, &csrf, body).await;

    // Act
    let resp = send(
        &app,
        Some(&jar),
        "POST",
        &test_uri(id),
        Some(&csrf),
        Some(serde_json::json!({})),
    )
    .await;

    // Assert
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["reply"], "Command failed");
    assert!(json["status"].is_null());
    assert!(fake.requests().is_empty());
}
