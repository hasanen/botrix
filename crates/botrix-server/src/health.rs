//! Container health probe backing the `botrix healthcheck` command.
//!
//! Performs a minimal `GET /health` over plain TCP using only `std`, so the
//! runtime image needs no extra tools (`curl`, `wget`) and the crate needs no
//! HTTP client dependency. The probe targets the loopback interface because
//! it runs inside the same container as the server.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use thiserror::Error;

/// Reasons a health probe can fail.
#[derive(Debug, Error)]
pub enum HealthError {
    /// The server could not be reached.
    #[error("could not connect to {addr}: {source}")]
    Connect {
        /// Address that was probed.
        addr: SocketAddr,
        /// Underlying connection error.
        #[source]
        source: std::io::Error,
    },
    /// The request or response could not be transferred (including timeouts).
    #[error("health request failed: {0}")]
    Io(#[from] std::io::Error),
    /// The server answered with something other than HTTP 200.
    #[error("unhealthy: server responded with '{status_line}'")]
    Unhealthy {
        /// First line of the response, e.g. `HTTP/1.1 503 Service Unavailable`.
        status_line: String,
    },
}

/// Probe `GET /health` at `addr`; succeed only on an HTTP 200 response.
///
/// `timeout` bounds connecting, writing, and reading individually.
pub fn check(addr: SocketAddr, timeout: Duration) -> Result<(), HealthError> {
    let mut stream = TcpStream::connect_timeout(&addr, timeout)
        .map_err(|source| HealthError::Connect { addr, source })?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;

    // HTTP/1.0 + `Connection: close` so the server ends the response itself.
    stream.write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;

    // Only the status line is needed; read until the first line break.
    let mut head = Vec::with_capacity(128);
    let mut chunk = [0u8; 128];
    while !head.contains(&b'\n') && head.len() < 1024 {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        head.extend_from_slice(&chunk[..n]);
    }

    let text = String::from_utf8_lossy(&head);
    let status_line = text.lines().next().unwrap_or("").trim_end().to_owned();
    if is_http_200(&status_line) {
        Ok(())
    } else {
        Err(HealthError::Unhealthy { status_line })
    }
}

/// Whether `line` is an HTTP status line with code exactly `200`.
fn is_http_200(line: &str) -> bool {
    let mut parts = line.split_whitespace();
    matches!(
        (parts.next(), parts.next()),
        (Some(version), Some("200")) if version.starts_with("HTTP/")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    /// Serve one connection with `reply` (or hang if `None`), returning the
    /// address to probe.
    fn serve_once(reply: Option<&'static str>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                match reply {
                    Some(text) => {
                        let _ = stream.write_all(text.as_bytes());
                    }
                    // Hold the connection open without answering.
                    None => thread::sleep(Duration::from_secs(3)),
                }
            }
        });
        addr
    }

    const TIMEOUT: Duration = Duration::from_millis(500);

    #[test]
    fn http_200_is_healthy() {
        let addr = serve_once(Some("HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n"));
        assert!(check(addr, TIMEOUT).is_ok());
    }

    #[test]
    fn http_503_is_unhealthy_with_the_status_line() {
        let addr = serve_once(Some("HTTP/1.1 503 Service Unavailable\r\n\r\n"));
        match check(addr, TIMEOUT) {
            Err(HealthError::Unhealthy { status_line }) => {
                assert_eq!(status_line, "HTTP/1.1 503 Service Unavailable");
            }
            other => panic!("expected Unhealthy, got {other:?}"),
        }
    }

    #[test]
    fn a_status_that_merely_contains_200_is_unhealthy() {
        // "200" must be the status code, not any substring of the line.
        let addr = serve_once(Some("HTTP/1.1 500 Error 200\r\n\r\n"));
        assert!(matches!(
            check(addr, TIMEOUT),
            Err(HealthError::Unhealthy { .. })
        ));
    }

    #[test]
    fn garbage_response_is_unhealthy() {
        let addr = serve_once(Some("not http at all\r\n\r\n"));
        assert!(matches!(
            check(addr, TIMEOUT),
            Err(HealthError::Unhealthy { .. })
        ));
    }

    #[test]
    fn nothing_listening_is_a_connect_error() {
        // Bind then drop to obtain a port that is (almost certainly) closed.
        let addr = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        assert!(matches!(
            check(addr, TIMEOUT),
            Err(HealthError::Connect { .. })
        ));
    }

    #[test]
    fn a_server_that_never_answers_times_out() {
        let addr = serve_once(None);
        let started = std::time::Instant::now();
        let result = check(addr, TIMEOUT);
        assert!(matches!(result, Err(HealthError::Io(_))));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
