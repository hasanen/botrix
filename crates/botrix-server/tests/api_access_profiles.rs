//! HTTP integration tests for the access profile CRUD endpoints.

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
        "botrix-apiprof-{}-{}.db",
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

fn profile(name: &str) -> Value {
    serde_json::json!({
        "name": name,
        "allowed_rooms": ["!a:example.com"],
        "allowed_users": ["@alice:example.com"],
    })
}

const BASE: &str = "/api/v1/access-profiles";

#[tokio::test]
async fn access_profiles_require_authentication() {
    // Arrange
    let (app, _pool) = test_app().await;
    // Act + Assert
    for (method, uri) in [
        ("GET", BASE.to_owned()),
        ("POST", BASE.to_owned()),
        ("PUT", format!("{BASE}/1")),
        ("DELETE", format!("{BASE}/1")),
    ] {
        let body = (method == "POST" || method == "PUT").then(|| profile("ops"));
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
        let body = (method != "DELETE").then(|| profile("ops"));
        let resp = send(&app, Some(&jar), method, &uri, None, body).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{method} {uri}");
    }
}

#[tokio::test]
async fn create_list_update_delete_round_trip() {
    // Arrange
    let (app, _pool) = test_app().await;
    let mut jar = authenticate(&app).await;
    let csrf = get_csrf(&app, &mut jar).await;

    // Act: create
    let resp = send(
        &app,
        Some(&jar),
        "POST",
        BASE,
        Some(&csrf),
        Some(profile("ops")),
    )
    .await;
    // Assert
    assert_eq!(resp.status(), StatusCode::OK);
    let created = body_json(resp).await;
    let id = created["id"].as_i64().unwrap();
    assert_eq!(created["name"], "ops");
    assert_eq!(created["allowed_rooms"][0], "!a:example.com");
    assert_eq!(created["allowed_users"][0], "@alice:example.com");

    // Act: list
    let listed = body_json(send(&app, Some(&jar), "GET", BASE, None, None).await).await;
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["id"], id);

    // Act: update (name is trimmed)
    let update = serde_json::json!({
        "name": "  ops2  ",
        "allowed_rooms": [],
        "allowed_users": ["@bob:example.com"],
    });
    let resp = send(
        &app,
        Some(&jar),
        "PUT",
        &format!("{BASE}/{id}"),
        Some(&csrf),
        Some(update),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let updated = body_json(resp).await;
    assert_eq!(updated["name"], "ops2");
    assert_eq!(updated["allowed_rooms"].as_array().unwrap().len(), 0);
    assert_eq!(updated["allowed_users"][0], "@bob:example.com");

    // Act: delete
    let resp = send(
        &app,
        Some(&jar),
        "DELETE",
        &format!("{BASE}/{id}"),
        Some(&csrf),
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let listed = body_json(send(&app, Some(&jar), "GET", BASE, None, None).await).await;
    assert!(listed.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn duplicate_name_is_rejected_with_conflict() {
    // Arrange
    let (app, _pool) = test_app().await;
    let mut jar = authenticate(&app).await;
    let csrf = get_csrf(&app, &mut jar).await;
    send(
        &app,
        Some(&jar),
        "POST",
        BASE,
        Some(&csrf),
        Some(profile("ops")),
    )
    .await;
    let other = body_json(
        send(
            &app,
            Some(&jar),
            "POST",
            BASE,
            Some(&csrf),
            Some(profile("dev")),
        )
        .await,
    )
    .await;

    // Act
    let create = send(
        &app,
        Some(&jar),
        "POST",
        BASE,
        Some(&csrf),
        Some(profile("ops")),
    )
    .await;
    let update = send(
        &app,
        Some(&jar),
        "PUT",
        &format!("{BASE}/{}", other["id"]),
        Some(&csrf),
        Some(profile("ops")),
    )
    .await;

    // Assert
    assert_eq!(create.status(), StatusCode::CONFLICT);
    assert_eq!(update.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn blank_name_is_rejected() {
    // Arrange
    let (app, _pool) = test_app().await;
    let mut jar = authenticate(&app).await;
    let csrf = get_csrf(&app, &mut jar).await;

    // Act
    let resp = send(
        &app,
        Some(&jar),
        "POST",
        BASE,
        Some(&csrf),
        Some(profile("   ")),
    )
    .await;

    // Assert
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn unknown_id_returns_not_found() {
    // Arrange
    let (app, _pool) = test_app().await;
    let mut jar = authenticate(&app).await;
    let csrf = get_csrf(&app, &mut jar).await;

    // Act
    let put = send(
        &app,
        Some(&jar),
        "PUT",
        &format!("{BASE}/999"),
        Some(&csrf),
        Some(profile("ops")),
    )
    .await;
    let delete = send(
        &app,
        Some(&jar),
        "DELETE",
        &format!("{BASE}/999"),
        Some(&csrf),
        None,
    )
    .await;

    // Assert
    assert_eq!(put.status(), StatusCode::NOT_FOUND);
    assert_eq!(delete.status(), StatusCode::NOT_FOUND);
}
