//! Serving of the compiled frontend (single-page app).
//!
//! In release builds the Trunk output (`crates/botrix-frontend/dist`) is
//! embedded into the binary with `rust-embed`, so the container needs no
//! asset volume. Setting `BOTRIX_FRONTEND_DIR` serves from disk instead, which
//! is convenient for local development.
//!
//! Routing rules (see [`respond`]):
//! - an exact file match is served as that file;
//! - paths under `/api/` that match nothing are a real `404` (never HTML);
//! - any other unknown path gets `index.html`, so client-side routes such as
//!   `/webhooks` work on a hard refresh.

use std::borrow::Cow;

use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

/// The frontend build output embedded at compile time.
///
/// `allow_missing` lets the crate compile (and tests run) without a frontend
/// build; the embedded set is then empty and requests get a `404`.
#[derive(RustEmbed)]
#[folder = "../botrix-frontend/dist"]
#[allow_missing = true]
struct Embedded;

/// A static file ready to be served.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// File contents.
    pub body: Cow<'static, [u8]>,
}

/// Name of the SPA entry point, served for unknown client-side routes.
const INDEX: &str = "index.html";

/// `Cache-Control` for `index.html`: always revalidate so a new deploy is
/// picked up immediately.
const CACHE_INDEX: &str = "no-cache";
/// `Cache-Control` for other assets (hashed by Trunk, except `main.css`).
const CACHE_ASSET: &str = "public, max-age=3600";

/// Content type for a file name, from its extension.
pub fn content_type(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

/// Whether `name` looks like a static asset request rather than a
/// client-side route (i.e. it has a file extension).
fn looks_like_asset(name: &str) -> bool {
    name.rsplit('/').next().unwrap_or("").contains('.')
}

/// Build the response for a request `path` using `lookup` to find files.
///
/// `path` is the URI path (leading `/` optional). Generic over `lookup` so the
/// routing rules can be tested without embedding anything.
pub fn respond(path: &str, lookup: impl Fn(&str) -> Option<Asset>) -> Response {
    let name = path.trim_start_matches('/');

    // Axum normalizes `..` segments before routing reaches a fallback
    // handler, but this function is also unit-tested directly, so reject
    // traversal defensively rather than relying on that upstream behavior.
    if name.split('/').any(|segment| segment == "..") {
        return StatusCode::NOT_FOUND.into_response();
    }

    if let Some(asset) = lookup(name) {
        return serve(name, asset);
    }

    // Never fall back to the SPA shell for API paths or anything that looks
    // like a missing static asset (e.g. a stale hashed JS/WASM URL) — both
    // must be a real 404, not a confusing 200 of HTML.
    if path.starts_with("/api/") || looks_like_asset(name) {
        return StatusCode::NOT_FOUND.into_response();
    }

    match lookup(INDEX) {
        Some(asset) => serve(INDEX, asset),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Build the final response for a resolved asset.
fn serve(name: &str, asset: Asset) -> Response {
    let cache = if name == INDEX {
        CACHE_INDEX
    } else {
        CACHE_ASSET
    };
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type(name)),
            (header::CACHE_CONTROL, cache),
        ],
        asset.body,
    )
        .into_response()
}

/// Axum fallback handler serving the embedded frontend.
pub async fn embedded_handler(uri: Uri) -> Response {
    respond(uri.path(), |name| {
        Embedded::get(name).map(|f| Asset { body: f.data })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    fn files(name: &str) -> Option<Asset> {
        let body: &'static [u8] = match name {
            "index.html" => b"<html>app</html>",
            "app.js" => b"console.log(1)",
            "app_bg.wasm" => b"\0asm",
            "main.css" => b"body{}",
            _ => return None,
        };
        Some(Asset {
            body: Cow::Borrowed(body),
        })
    }

    async fn body_of(resp: Response) -> Vec<u8> {
        to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec()
    }

    fn header_of(resp: &Response, name: header::HeaderName) -> &str {
        resp.headers().get(name).unwrap().to_str().unwrap()
    }

    #[test]
    fn content_types_cover_trunk_output() {
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(content_type("a.js"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("a.wasm"), "application/wasm");
        assert_eq!(content_type("main.css"), "text/css; charset=utf-8");
        assert_eq!(content_type("logo.svg"), "image/svg+xml");
        assert_eq!(content_type("logo.png"), "image/png");
        assert_eq!(content_type("favicon.ico"), "image/x-icon");
        assert_eq!(content_type("data.json"), "application/json");
    }

    #[test]
    fn unknown_extension_is_octet_stream() {
        assert_eq!(content_type("blob.xyz"), "application/octet-stream");
        assert_eq!(content_type("noextension"), "application/octet-stream");
    }

    #[test]
    fn extension_matching_ignores_case() {
        assert_eq!(content_type("APP.JS"), "text/javascript; charset=utf-8");
    }

    #[tokio::test]
    async fn exact_file_is_served_with_its_content_type() {
        let resp = respond("/app.js", files);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            header_of(&resp, header::CONTENT_TYPE),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(body_of(resp).await, b"console.log(1)");
    }

    #[tokio::test]
    async fn wasm_is_served_as_application_wasm() {
        let resp = respond("/app_bg.wasm", files);
        assert_eq!(header_of(&resp, header::CONTENT_TYPE), "application/wasm");
    }

    #[tokio::test]
    async fn root_serves_index() {
        let resp = respond("/", files);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_of(resp).await, b"<html>app</html>");
    }

    #[tokio::test]
    async fn unknown_client_route_falls_back_to_index() {
        let resp = respond("/webhooks", files);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            header_of(&resp, header::CONTENT_TYPE),
            "text/html; charset=utf-8"
        );
        assert_eq!(body_of(resp).await, b"<html>app</html>");
    }

    #[tokio::test]
    async fn unknown_api_path_is_a_real_404_not_html() {
        let resp = respond("/api/v1/nope", files);
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn missing_asset_with_extension_is_404_not_index() {
        // A stale hashed bundle URL must not return HTML for a .js request.
        let resp = respond("/old-hash.js", files);
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn missing_index_is_404() {
        let resp = respond("/webhooks", |_| None);
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn index_is_never_cached_but_assets_are() {
        let index = respond("/", files);
        let asset = respond("/app.js", files);
        assert_eq!(header_of(&index, header::CACHE_CONTROL), "no-cache");
        assert_eq!(
            header_of(&asset, header::CACHE_CONTROL),
            "public, max-age=3600"
        );
    }

    #[tokio::test]
    async fn dot_dot_segments_cannot_escape_the_asset_set() {
        // Lookup is by exact key, so traversal can only miss; it must not
        // panic and must not leak anything.
        let resp = respond("/../../etc/passwd", files);
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
