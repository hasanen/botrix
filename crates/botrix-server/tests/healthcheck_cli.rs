//! Black-box tests for `botrix healthcheck` (the container HEALTHCHECK).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::thread;

fn botrix_bin() -> &'static str {
    env!("CARGO_BIN_EXE_botrix")
}

/// Write a config whose server listens on `port`; returns its path.
fn write_config(name: &str, port: u16) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "botrix-health-{}-{}.yaml",
        std::process::id(),
        name
    ));
    let yaml = format!(
        r#"
matrix:
  homeserver: https://matrix.example.com
  destinations:
    alerts:
      room_id: "!abc123:example.com"
      enabled: true
webhooks:
  hook:
    enabled: true
    destination: alerts
    msg_template: "Hello ${{name}}"
server:
  host: "0.0.0.0"
  port: {port}
"#
    );
    std::fs::write(&path, yaml).expect("write config");
    path
}

/// Answer one request on `listener` with the given status line.
fn answer_once(listener: TcpListener, status_line: &'static str) {
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let _ =
                stream.write_all(format!("{status_line}\r\ncontent-length: 0\r\n\r\n").as_bytes());
        }
    });
}

fn run(config: &std::path::Path) -> std::process::Output {
    Command::new(botrix_bin())
        .args(["healthcheck", "--config"])
        .arg(config)
        .output()
        .expect("run botrix")
}

#[test]
fn healthy_server_exits_zero() {
    // Arrange: a listener on the configured port answering 200.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    answer_once(listener, "HTTP/1.1 200 OK");
    let config = write_config("ok", port);

    // Act
    let output = run(&config);
    let _ = std::fs::remove_file(&config);

    // Assert
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn unhealthy_server_exits_nonzero() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    answer_once(listener, "HTTP/1.1 503 Service Unavailable");
    let config = write_config("bad", port);

    let output = run(&config);
    let _ = std::fs::remove_file(&config);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("503"));
}

#[test]
fn nothing_listening_exits_nonzero() {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config = write_config("closed", port);

    let output = run(&config);
    let _ = std::fs::remove_file(&config);

    assert!(!output.status.success());
}

#[test]
fn invalid_config_exits_nonzero() {
    let mut path = std::env::temp_dir();
    path.push(format!("botrix-health-{}-invalid.yaml", std::process::id()));
    std::fs::write(&path, "not: [valid").unwrap();

    let output = run(&path);
    let _ = std::fs::remove_file(&path);

    assert!(!output.status.success());
}
