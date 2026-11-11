//! HTTP integration tests for the config-defined trigger endpoints.
//!
//! Triggers come from the config; the API lists them and sets or clears their
//! secret values.

use std::collections::{BTreeMap, HashMap};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use botrix_common::config::{AccessProfileConfig, TriggerConfig};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

const SESSION_SECRET: &str = "0123456789abcdef0123456789abcdef";
/// `build_app` uses this passphrase for trigger secrets.
const CRYPTO_PASSPHRASE: &str = "test-passphrase";

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

fn deploy_trigger() -> TriggerConfig {
    TriggerConfig {
        description: "Deploy the app".to_owned(),
        enabled: true,
        method: "POST".to_owned(),
        url: "https://ci.example.com/hook".to_owned(),
        headers: BTreeMap::from([
            (
                "Authorization".to_owned(),
                "Bearer {{secret:TOKEN}}".to_owned(),
            ),
            ("X-Source".to_owned(), "botrix".to_owned()),
        ]),
        secrets: vec!["TOKEN".to_owned(), "SIGNING_KEY".to_owned()],
        response_template: Some("${status}".to_owned()),
        access_profile: Some("ops".to_owned()),
        allowed_rooms: Vec::new(),
        allowed_users: vec!["@alice:example.com".to_owned()],
    }
}

fn build_trigger() -> TriggerConfig {
    TriggerConfig {
        description: String::new(),
        enabled: false,
        method: "GET".to_owned(),
        url: "http://ci.internal/build".to_owned(),
        headers: BTreeMap::new(),
        secrets: Vec::new(),
        response_template: None,
        access_profile: None,
        allowed_rooms: vec!["!build:example.com".to_owned()],
        allowed_users: Vec::new(),
    }
}

async fn test_app() -> (Router, botrix_server::db::Pool) {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "botrix-apitrig-{}-{}.db",
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
        // Mixed-case key: the API reports and matches the lowercase command.
        triggers: HashMap::from([
            ("Deploy".to_owned(), deploy_trigger()),
            ("build".to_owned(), build_trigger()),
        ]),
        access_profiles: HashMap::from([(
            "ops".to_owned(),
            AccessProfileConfig {
                allowed_rooms: vec!["!ops:example.com".to_owned()],
                allowed_users: vec!["@bob:example.com".to_owned()],
            },
        )]),
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

async fn text(response: Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
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

fn secret_uri(command: &str, name: &str) -> String {
    format!("{BASE}/{command}/secrets/{name}")
}

/// Authenticate and fetch a CSRF token.
async fn session(app: &Router) -> (Jar, String) {
    let mut jar = authenticate(app).await;
    let csrf = get_csrf(app, &mut jar).await;
    (jar, csrf)
}

async fn put_secret(
    app: &Router,
    jar: &Jar,
    csrf: &str,
    command: &str,
    name: &str,
    value: &str,
) -> Response {
    send(
        app,
        Some(jar),
        "PUT",
        &secret_uri(command, name),
        Some(csrf),
        Some(serde_json::json!({ "value": value })),
    )
    .await
}

async fn list(app: &Router, jar: &Jar) -> Value {
    body_json(send(app, Some(jar), "GET", BASE, None, None).await).await
}

#[tokio::test]
async fn triggers_require_authentication() {
    // Arrange
    let (app, _pool) = test_app().await;

    // Act + Assert
    for (method, uri) in [
        ("GET", BASE.to_owned()),
        ("PUT", secret_uri("deploy", "TOKEN")),
        ("DELETE", secret_uri("deploy", "TOKEN")),
    ] {
        let body = (method == "PUT").then(|| serde_json::json!({ "value": PLAINTEXT }));
        let resp = send(&app, None, method, &uri, None, body).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{method} {uri}");
    }
}

#[tokio::test]
async fn secret_mutations_without_csrf_token_are_forbidden() {
    // Arrange
    let (app, pool) = test_app().await;
    let jar = authenticate(&app).await;

    // Act + Assert
    for (method, body) in [
        ("PUT", Some(serde_json::json!({ "value": PLAINTEXT }))),
        ("DELETE", None),
    ] {
        let resp = send(
            &app,
            Some(&jar),
            method,
            &secret_uri("deploy", "TOKEN"),
            None,
            body,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{method}");
    }
    assert!(botrix_server::db::triggers::list_secrets(&pool, "deploy")
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn list_shows_config_triggers_sorted_with_effective_access_and_secret_status() {
    // Arrange
    let (app, _pool) = test_app().await;
    let (jar, _csrf) = session(&app).await;

    // Act
    let resp = send(&app, Some(&jar), "GET", BASE, None, None).await;

    // Assert
    assert_eq!(resp.status(), StatusCode::OK);
    let listed: Value = serde_json::from_str(&text(resp).await).unwrap();
    let items = listed.as_array().unwrap();
    let commands: Vec<&str> = items
        .iter()
        .map(|i| i["command"].as_str().unwrap())
        .collect();
    assert_eq!(commands, ["build", "deploy"]);

    let build = &items[0];
    assert_eq!(build["enabled"], false);
    assert_eq!(build["method"], "GET");
    assert_eq!(build["url"], "http://ci.internal/build");
    assert_eq!(
        build["allowed_rooms"],
        serde_json::json!(["!build:example.com"])
    );
    assert_eq!(build["allowed_users"], serde_json::json!([]));
    assert_eq!(build["access_profile"], Value::Null);
    assert_eq!(build["secrets"], serde_json::json!([]));

    let deploy = &items[1];
    assert_eq!(deploy["description"], "Deploy the app");
    assert_eq!(deploy["enabled"], true);
    assert_eq!(deploy["access_profile"], "ops");
    assert_eq!(
        deploy["header_names"],
        serde_json::json!(["Authorization", "X-Source"])
    );
    // Rooms come from the profile, users from the inline override.
    assert_eq!(
        deploy["allowed_rooms"],
        serde_json::json!(["!ops:example.com"])
    );
    assert_eq!(
        deploy["allowed_users"],
        serde_json::json!(["@alice:example.com"])
    );
    assert_eq!(
        deploy["secrets"],
        serde_json::json!([
            { "name": "TOKEN", "is_set": false },
            { "name": "SIGNING_KEY", "is_set": false },
        ])
    );
}

#[tokio::test]
async fn list_never_returns_header_values() {
    // Arrange
    let (app, _pool) = test_app().await;
    let (jar, _csrf) = session(&app).await;

    // Act
    let body = text(send(&app, Some(&jar), "GET", BASE, None, None).await).await;

    // Assert
    assert!(!body.contains("{{secret:"), "header value leaked: {body}");
}

#[tokio::test]
async fn setting_a_secret_marks_it_set_and_stores_it_encrypted() {
    // Arrange
    let (app, pool) = test_app().await;
    let (jar, csrf) = session(&app).await;

    // Act
    let resp = put_secret(&app, &jar, &csrf, "deploy", "TOKEN", PLAINTEXT).await;

    // Assert
    assert_eq!(resp.status(), StatusCode::OK);
    let response_text = text(resp).await;
    assert!(!response_text.contains(PLAINTEXT));
    let status: Value = serde_json::from_str(&response_text).unwrap();
    assert_eq!(
        status,
        serde_json::json!({ "name": "TOKEN", "is_set": true })
    );

    let listed_text = text(send(&app, Some(&jar), "GET", BASE, None, None).await).await;
    assert!(!listed_text.contains(PLAINTEXT));
    let listed: Value = serde_json::from_str(&listed_text).unwrap();
    let deploy = &listed[1];
    assert_eq!(
        deploy["secrets"][0],
        serde_json::json!({ "name": "TOKEN", "is_set": true })
    );
    assert_eq!(
        deploy["secrets"][1],
        serde_json::json!({ "name": "SIGNING_KEY", "is_set": false })
    );

    let stored = botrix_server::db::triggers::list_secrets(&pool, "deploy").unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].name, "TOKEN");
    assert!(!stored[0].ciphertext.contains(PLAINTEXT));
    let decrypted = botrix_server::matrix::crypto::decrypt_trigger_secret(
        CRYPTO_PASSPHRASE,
        &stored[0].ciphertext,
    )
    .unwrap();
    assert_eq!(decrypted, PLAINTEXT);
}

#[tokio::test]
async fn setting_a_secret_again_replaces_the_value() {
    // Arrange
    let (app, pool) = test_app().await;
    let (jar, csrf) = session(&app).await;
    put_secret(&app, &jar, &csrf, "deploy", "TOKEN", PLAINTEXT).await;
    let before = botrix_server::db::triggers::list_secrets(&pool, "deploy").unwrap()[0]
        .ciphertext
        .clone();

    // Act
    let resp = put_secret(&app, &jar, &csrf, "deploy", "TOKEN", "another-value").await;

    // Assert
    assert_eq!(resp.status(), StatusCode::OK);
    let after = botrix_server::db::triggers::list_secrets(&pool, "deploy").unwrap();
    assert_eq!(after.len(), 1);
    assert_ne!(after[0].ciphertext, before);
    let decrypted = botrix_server::matrix::crypto::decrypt_trigger_secret(
        CRYPTO_PASSPHRASE,
        &after[0].ciphertext,
    )
    .unwrap();
    assert_eq!(decrypted, "another-value");
}

#[tokio::test]
async fn clearing_a_secret_removes_it_and_is_idempotent() {
    // Arrange
    let (app, pool) = test_app().await;
    let (jar, csrf) = session(&app).await;
    put_secret(&app, &jar, &csrf, "deploy", "TOKEN", PLAINTEXT).await;
    let uri = secret_uri("deploy", "TOKEN");

    // Act
    let cleared = send(&app, Some(&jar), "DELETE", &uri, Some(&csrf), None).await;
    let again = send(&app, Some(&jar), "DELETE", &uri, Some(&csrf), None).await;

    // Assert
    assert_eq!(cleared.status(), StatusCode::OK);
    assert_eq!(
        body_json(cleared).await,
        serde_json::json!({ "name": "TOKEN", "is_set": false })
    );
    assert_eq!(again.status(), StatusCode::OK);
    assert!(botrix_server::db::triggers::list_secrets(&pool, "deploy")
        .unwrap()
        .is_empty());
    assert_eq!(list(&app, &jar).await[1]["secrets"][0]["is_set"], false);
}

#[tokio::test]
async fn command_in_the_path_is_matched_case_insensitively() {
    // Arrange
    let (app, pool) = test_app().await;
    let (jar, csrf) = session(&app).await;

    // Act
    let resp = put_secret(&app, &jar, &csrf, "DEPLOY", "TOKEN", PLAINTEXT).await;

    // Assert
    assert_eq!(resp.status(), StatusCode::OK);
    let stored = botrix_server::db::triggers::list_secrets(&pool, "deploy").unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].command, "deploy");
}

#[tokio::test]
async fn unknown_command_or_undeclared_secret_is_not_found() {
    // Arrange
    let (app, pool) = test_app().await;
    let (jar, csrf) = session(&app).await;
    let cases = [
        ("nope", "TOKEN"),
        ("deploy", "NOT_DECLARED"),
        // `build` declares no secrets at all.
        ("build", "TOKEN"),
        // Secret names are case-sensitive.
        ("deploy", "token"),
    ];

    // Act + Assert
    for (command, name) in cases {
        let put = put_secret(&app, &jar, &csrf, command, name, PLAINTEXT).await;
        assert_eq!(put.status(), StatusCode::NOT_FOUND, "PUT {command}/{name}");
        let delete = send(
            &app,
            Some(&jar),
            "DELETE",
            &secret_uri(command, name),
            Some(&csrf),
            None,
        )
        .await;
        assert_eq!(
            delete.status(),
            StatusCode::NOT_FOUND,
            "DELETE {command}/{name}"
        );
    }
    for command in ["deploy", "build", "nope"] {
        assert!(botrix_server::db::triggers::list_secrets(&pool, command)
            .unwrap()
            .is_empty());
    }
}

#[tokio::test]
async fn empty_or_oversized_secret_values_are_rejected() {
    // Arrange
    let (app, pool) = test_app().await;
    let (jar, csrf) = session(&app).await;
    let too_long = "x".repeat(4097);

    // Act
    let empty = put_secret(&app, &jar, &csrf, "deploy", "TOKEN", "").await;
    let long = put_secret(&app, &jar, &csrf, "deploy", "TOKEN", &too_long).await;
    let at_limit = put_secret(
        &app,
        &jar,
        &csrf,
        "deploy",
        "SIGNING_KEY",
        &"y".repeat(4096),
    )
    .await;

    // Assert
    assert_eq!(empty.status(), StatusCode::BAD_REQUEST);
    assert_eq!(long.status(), StatusCode::BAD_REQUEST);
    assert_eq!(at_limit.status(), StatusCode::OK);
    let names: Vec<String> = botrix_server::db::triggers::list_secrets(&pool, "deploy")
        .unwrap()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(names, ["SIGNING_KEY"]);
}

#[tokio::test]
async fn secret_changes_are_logged_without_the_value() {
    // Arrange
    let (app, pool) = test_app().await;
    let (jar, csrf) = session(&app).await;

    // Act
    put_secret(&app, &jar, &csrf, "deploy", "TOKEN", PLAINTEXT).await;
    send(
        &app,
        Some(&jar),
        "DELETE",
        &secret_uri("deploy", "TOKEN"),
        Some(&csrf),
        None,
    )
    .await;

    // Assert
    let (items, _) = botrix_server::db::history::list_logs(&pool, 1, 50).unwrap();
    let trigger_logs: Vec<_> = items.iter().filter(|l| l.component == "trigger").collect();
    let messages: Vec<&str> = trigger_logs.iter().map(|l| l.message.as_str()).collect();
    assert!(messages.contains(&"secret set"), "{messages:?}");
    assert!(messages.contains(&"secret cleared"), "{messages:?}");
    for log in &items {
        assert!(!log.message.contains(PLAINTEXT));
        assert!(!log.details.clone().unwrap_or_default().contains(PLAINTEXT));
    }
    let set = trigger_logs
        .iter()
        .find(|l| l.message == "secret set")
        .unwrap();
    let details = set.details.clone().unwrap();
    assert!(details.contains("deploy") && details.contains("TOKEN"));
}

#[tokio::test]
async fn trigger_and_profile_crud_routes_no_longer_exist() {
    // Arrange
    let (app, _pool) = test_app().await;
    let (jar, csrf) = session(&app).await;
    let body = || Some(serde_json::json!({ "command": "x" }));

    // Act + Assert: the collection only supports GET now.
    let post = send(&app, Some(&jar), "POST", BASE, Some(&csrf), body()).await;
    assert_eq!(post.status(), StatusCode::METHOD_NOT_ALLOWED);

    for (method, uri) in [
        ("GET", format!("{BASE}/1")),
        ("PUT", format!("{BASE}/1")),
        ("DELETE", format!("{BASE}/1")),
        ("POST", format!("{BASE}/1/test")),
        ("GET", "/api/v1/access-profiles".to_owned()),
        ("POST", "/api/v1/access-profiles".to_owned()),
        ("PUT", "/api/v1/access-profiles/1".to_owned()),
        ("DELETE", "/api/v1/access-profiles/1".to_owned()),
    ] {
        let resp = send(&app, Some(&jar), method, &uri, Some(&csrf), body()).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

// ---- GET /api/v1/logs filters ----

async fn logs(app: &Router, jar: &Jar, query: &str) -> Value {
    let uri = format!("/api/v1/logs{query}");
    let resp = send(app, Some(jar), "GET", &uri, None, None).await;
    assert_eq!(resp.status(), StatusCode::OK, "{uri}");
    body_json(resp).await
}

fn messages(logs: &Value) -> Vec<&str> {
    logs["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["message"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn logs_can_be_filtered_by_command_and_component() {
    // Arrange: two events for `deploy` (secret set and cleared) and one for `build`.
    let (app, pool) = test_app().await;
    let (jar, csrf) = session(&app).await;
    put_secret(&app, &jar, &csrf, "deploy", "TOKEN", PLAINTEXT).await;
    send(
        &app,
        Some(&jar),
        "DELETE",
        &secret_uri("deploy", "TOKEN"),
        Some(&csrf),
        None,
    )
    .await;
    botrix_server::db::logs::record(
        &pool,
        botrix_common::types::LogLevel::Info,
        botrix_server::db::logs::Component::Trigger,
        "command succeeded",
        Some(&format!(
            "{}sender=@a:x room=!r:x",
            botrix_server::db::logs::command_tag("build")
        )),
    );

    // Act
    let deploy = logs(&app, &jar, "?command=deploy").await;
    let deploy_upper = logs(&app, &jar, "?command=DEPLOY&component=trigger").await;
    let build = logs(&app, &jar, "?command=build").await;
    let trigger_component = logs(&app, &jar, "?component=trigger").await;
    let none = logs(&app, &jar, "?command=nope").await;

    // Assert: newest first, and the total counts only the matches.
    assert_eq!(messages(&deploy), ["secret cleared", "secret set"]);
    assert_eq!(deploy["total"], 2);
    assert_eq!(messages(&deploy_upper), ["secret cleared", "secret set"]);
    assert_eq!(messages(&build), ["command succeeded"]);
    assert_eq!(trigger_component["total"], 3);
    assert!(messages(&none).is_empty());
    assert_eq!(none["total"], 0);
}

#[tokio::test]
async fn empty_log_filters_are_ignored_and_pagination_still_applies() {
    // Arrange
    let (app, _pool) = test_app().await;
    let (jar, csrf) = session(&app).await;
    for value in ["a", "b", "c"] {
        put_secret(&app, &jar, &csrf, "deploy", "TOKEN", value).await;
    }

    // Act
    let unfiltered = logs(&app, &jar, "").await;
    let blank = logs(&app, &jar, "?component=&command=").await;
    let paged = logs(&app, &jar, "?command=deploy&per_page=2&page=2").await;

    // Assert
    assert_eq!(blank["total"], unfiltered["total"]);
    assert_eq!(paged["total"], 3);
    assert_eq!(paged["items"].as_array().unwrap().len(), 1);
    assert_eq!(paged["page"], 2);
}

#[tokio::test]
async fn filtered_logs_still_require_authentication() {
    let (app, _pool) = test_app().await;

    let resp = send(&app, None, "GET", "/api/v1/logs?command=deploy", None, None).await;

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
