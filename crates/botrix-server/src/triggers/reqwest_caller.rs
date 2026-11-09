//! `reqwest`-backed [`WebhookCaller`] with SSRF protection.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::Method;
use thiserror::Error;
use url::{Host, Url};

use super::ssrf;
use super::webhook::{WebhookCaller, WebhookError, WebhookRequest, WebhookResponse};

/// Default TCP connect timeout.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Default total request timeout (connect, send, and reading the whole body).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
/// Default cap on the response body size, in bytes.
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// Resolution failure inside the custom DNS resolver.
#[derive(Debug, Error)]
enum ResolveError {
    /// At least one resolved address is not allowed.
    #[error("blocked address")]
    Blocked,
    /// The lookup failed or returned nothing.
    #[error("dns lookup failed")]
    Lookup(#[from] std::io::Error),
}

/// Returns `true` if `ip` must not be contacted.
fn is_blocked(ip: IpAddr, allow_loopback: bool) -> bool {
    ssrf::is_blocked_ip(ip) && !(allow_loopback && ip.is_loopback())
}

/// DNS resolver that rejects a host if ANY of its addresses is blocked.
#[derive(Debug, Clone, Copy)]
struct SsrfResolver {
    allow_loopback: bool,
}

impl Resolve for SsrfResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        let allow_loopback = self.allow_loopback;
        Box::pin(async move {
            let addrs = lookup(&host, allow_loopback).await?;
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

async fn lookup(host: &str, allow_loopback: bool) -> Result<Vec<SocketAddr>, ResolveError> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, 0)).await?.collect();
    if addrs.is_empty() {
        return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
    }
    if addrs.iter().any(|a| is_blocked(a.ip(), allow_loopback)) {
        return Err(ResolveError::Blocked);
    }
    Ok(addrs)
}

/// Sends command-trigger webhooks over HTTP(S), refusing blocked addresses.
#[derive(Debug, Clone)]
pub struct ReqwestCaller {
    client: reqwest::Client,
    max_response_bytes: usize,
    allow_loopback: bool,
}

impl ReqwestCaller {
    /// Builds a caller with explicit limits. Blocked addresses are always rejected.
    pub fn new(
        connect_timeout: Duration,
        timeout: Duration,
        max_response_bytes: usize,
    ) -> Result<Self, WebhookError> {
        Self::build(connect_timeout, timeout, max_response_bytes, false)
    }

    /// Builds a caller using the default limits.
    pub fn with_defaults() -> Result<Self, WebhookError> {
        Self::new(
            DEFAULT_CONNECT_TIMEOUT,
            DEFAULT_TIMEOUT,
            DEFAULT_MAX_RESPONSE_BYTES,
        )
    }

    /// Test seam: like [`ReqwestCaller::new`] but allows loopback addresses so
    /// tests can reach local servers. Other blocked ranges stay blocked.
    #[cfg(test)]
    pub(crate) fn for_tests(
        connect_timeout: Duration,
        timeout: Duration,
        max_response_bytes: usize,
    ) -> Result<Self, WebhookError> {
        Self::build(connect_timeout, timeout, max_response_bytes, true)
    }

    fn build(
        connect_timeout: Duration,
        timeout: Duration,
        max_response_bytes: usize,
        allow_loopback: bool,
    ) -> Result<Self, WebhookError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(connect_timeout)
            .timeout(timeout)
            // Proxies would bypass the DNS resolver, so never use them.
            .no_proxy()
            .dns_resolver(Arc::new(SsrfResolver { allow_loopback }))
            .build()
            .map_err(|_| WebhookError::Transport)?;
        Ok(Self {
            client,
            max_response_bytes,
            allow_loopback,
        })
    }

    /// Parses `raw`, allowing only http/https and unblocked literal IPs.
    fn check_url(&self, raw: &str) -> Result<Url, WebhookError> {
        let url = Url::parse(raw).map_err(|_| WebhookError::InvalidUrl)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(WebhookError::InvalidUrl);
        }
        match url.host() {
            None => Err(WebhookError::InvalidUrl),
            Some(Host::Domain(_)) => Ok(url),
            Some(Host::Ipv4(ip)) if is_blocked(IpAddr::V4(ip), self.allow_loopback) => {
                Err(WebhookError::BlockedAddress)
            }
            Some(Host::Ipv6(ip)) if is_blocked(IpAddr::V6(ip), self.allow_loopback) => {
                Err(WebhookError::BlockedAddress)
            }
            Some(_) => Ok(url),
        }
    }
}

/// Maps a reqwest error to a [`WebhookError`] without exposing URL or headers.
fn map_error(err: &reqwest::Error) -> WebhookError {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = source {
        if let Some(ResolveError::Blocked) = e.downcast_ref::<ResolveError>() {
            return WebhookError::BlockedAddress;
        }
        source = e.source();
    }
    if err.is_timeout() {
        WebhookError::Timeout
    } else if err.is_builder() {
        WebhookError::InvalidUrl
    } else {
        WebhookError::Transport
    }
}

#[async_trait]
impl WebhookCaller for ReqwestCaller {
    async fn call(&self, req: WebhookRequest) -> Result<WebhookResponse, WebhookError> {
        let url = self.check_url(&req.url)?;
        let method =
            Method::from_bytes(req.method.as_bytes()).map_err(|_| WebhookError::Transport)?;

        let mut headers = HeaderMap::new();
        for (name, value) in &req.headers {
            let name =
                HeaderName::from_bytes(name.as_bytes()).map_err(|_| WebhookError::Transport)?;
            let mut value = HeaderValue::from_str(value).map_err(|_| WebhookError::Transport)?;
            value.set_sensitive(true);
            headers.append(name, value);
        }

        let mut builder = self.client.request(method.clone(), url).headers(headers);
        if method != Method::GET {
            let body = serde_json::to_vec(&req.body).map_err(|_| WebhookError::Transport)?;
            builder = builder
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body);
        }

        let mut response = builder.send().await.map_err(|e| map_error(&e))?;
        let status = response.status().as_u16();

        if response
            .content_length()
            .is_some_and(|len| len > self.max_response_bytes as u64)
        {
            return Err(WebhookError::ResponseTooLarge);
        }

        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| map_error(&e))? {
            if body.len().saturating_add(chunk.len()) > self.max_response_bytes {
                return Err(WebhookError::ResponseTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(WebhookResponse { status, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Starts a server that answers every connection with `respond`.
    /// Returns the bound port.
    async fn serve<F, Fut>(respond: F) -> u16
    where
        F: Fn(tokio::net::TcpStream) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(respond(stream));
            }
        });
        port
    }

    /// Reads the request head, then writes `response` and closes.
    async fn reply(mut stream: tokio::net::TcpStream, response: Vec<u8>) {
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf).await;
        let _ = stream.write_all(&response).await;
        let _ = stream.shutdown().await;
    }

    fn request(method: &str, url: String) -> WebhookRequest {
        WebhookRequest {
            method: method.into(),
            url,
            headers: vec![("X-Test".into(), "1".into())],
            body: serde_json::json!({"a": 1}),
        }
    }

    fn test_caller(max: usize) -> ReqwestCaller {
        ReqwestCaller::for_tests(Duration::from_secs(2), Duration::from_secs(2), max).unwrap()
    }

    #[tokio::test]
    async fn default_caller_blocks_literal_loopback_ip() {
        // Arrange
        let port =
            serve(|s| reply(s, b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec())).await;
        let caller = ReqwestCaller::with_defaults().unwrap();

        // Act
        let res = caller
            .call(request("POST", format!("http://127.0.0.1:{port}/")))
            .await;

        // Assert
        assert_eq!(res, Err(WebhookError::BlockedAddress));
    }

    #[tokio::test]
    async fn default_caller_blocks_localhost_name() {
        // Arrange
        let port =
            serve(|s| reply(s, b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec())).await;
        let caller = ReqwestCaller::with_defaults().unwrap();

        // Act
        let res = caller
            .call(request("POST", format!("http://localhost:{port}/")))
            .await;

        // Assert
        assert_eq!(res, Err(WebhookError::BlockedAddress));
    }

    #[tokio::test]
    async fn default_caller_blocks_private_and_metadata_literals() {
        // Arrange
        let caller = ReqwestCaller::with_defaults().unwrap();

        for url in [
            "http://10.0.0.1/",
            "http://169.254.169.254/latest/meta-data",
            "http://[::1]/",
        ] {
            // Act
            let res = caller.call(request("GET", url.into())).await;

            // Assert
            assert_eq!(res, Err(WebhookError::BlockedAddress), "{url}");
        }
    }

    #[tokio::test]
    async fn rejects_non_http_schemes_and_garbage() {
        // Arrange
        let caller = ReqwestCaller::with_defaults().unwrap();

        for url in ["ftp://example.org/", "file:///etc/passwd", "not a url"] {
            // Act
            let res = caller.call(request("GET", url.into())).await;

            // Assert
            assert_eq!(res, Err(WebhookError::InvalidUrl), "{url}");
        }
    }

    #[tokio::test]
    async fn success_returns_status_and_body() {
        // Arrange
        let port = serve(|s| {
            reply(
                s,
                b"HTTP/1.1 201 Created\r\nContent-Length: 5\r\n\r\nhello".to_vec(),
            )
        })
        .await;
        let caller = test_caller(1024);

        // Act
        let res = caller
            .call(request("POST", format!("http://127.0.0.1:{port}/hook")))
            .await;

        // Assert
        assert_eq!(
            res,
            Ok(WebhookResponse {
                status: 201,
                body: b"hello".to_vec()
            })
        );
    }

    #[tokio::test]
    async fn get_sends_no_body_and_post_sends_json() {
        // Arrange
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let port = serve(move |mut s| {
            let tx = tx.clone();
            async move {
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let _ = tx.send(String::from_utf8_lossy(&buf[..n]).to_string());
                let _ = s
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                    .await;
                let _ = s.shutdown().await;
            }
        })
        .await;
        let caller = test_caller(1024);

        // Act
        caller
            .call(request("GET", format!("http://127.0.0.1:{port}/")))
            .await
            .unwrap();
        let get_raw = rx.recv().await.unwrap().to_lowercase();
        caller
            .call(request("POST", format!("http://127.0.0.1:{port}/")))
            .await
            .unwrap();
        let post_raw = rx.recv().await.unwrap().to_lowercase();

        // Assert
        assert!(get_raw.starts_with("get "));
        assert!(!get_raw.contains("content-length: 7"));
        assert!(!get_raw.contains("{\"a\":1}"));
        assert!(get_raw.contains("x-test: 1"));
        assert!(post_raw.starts_with("post "));
        assert!(post_raw.contains("content-type: application/json"));
        assert!(post_raw.contains("{\"a\":1}"));
    }

    #[tokio::test]
    async fn redirects_are_not_followed() {
        // Arrange
        let port = serve(|s| {
            reply(
                s,
                b"HTTP/1.1 302 Found\r\nLocation: http://10.0.0.1/\r\nContent-Length: 0\r\n\r\n"
                    .to_vec(),
            )
        })
        .await;
        let caller = test_caller(1024);

        // Act
        let res = caller
            .call(request("GET", format!("http://127.0.0.1:{port}/")))
            .await
            .unwrap();

        // Assert
        assert_eq!(res.status, 302);
    }

    #[tokio::test]
    async fn oversize_body_with_content_length_is_rejected() {
        // Arrange
        let mut response = b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n".to_vec();
        response.extend(std::iter::repeat_n(b'x', 100));
        let port = serve(move |s| reply(s, response.clone())).await;
        let caller = test_caller(10);

        // Act
        let res = caller
            .call(request("GET", format!("http://127.0.0.1:{port}/")))
            .await;

        // Assert
        assert_eq!(res, Err(WebhookError::ResponseTooLarge));
    }

    #[tokio::test]
    async fn oversize_body_without_content_length_is_rejected() {
        // Arrange
        let mut response = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
        response.extend(std::iter::repeat_n(b'x', 100));
        let port = serve(move |s| reply(s, response.clone())).await;
        let caller = test_caller(10);

        // Act
        let res = caller
            .call(request("GET", format!("http://127.0.0.1:{port}/")))
            .await;

        // Assert
        assert_eq!(res, Err(WebhookError::ResponseTooLarge));
    }

    #[tokio::test]
    async fn slow_server_times_out() {
        // Arrange
        let port = serve(|s| async move {
            // Hold the connection open without ever answering.
            let _held = s;
            tokio::time::sleep(Duration::from_secs(10)).await;
        })
        .await;
        let caller =
            ReqwestCaller::for_tests(Duration::from_secs(1), Duration::from_millis(300), 1024)
                .unwrap();

        // Act
        let res = caller
            .call(request("GET", format!("http://127.0.0.1:{port}/")))
            .await;

        // Assert
        assert_eq!(res, Err(WebhookError::Timeout));
    }
}
