//! Black-box CLI tests for `botrix config validate` and `botrix serve`
//! startup behavior.
//!
//! These exercise the Phase 1 acceptance criteria directly against the
//! compiled binary: exit codes and descriptive error messages for valid and
//! invalid configuration.

use std::io::Write;
use std::process::Command;

/// Path to the compiled `botrix` binary under test.
///
/// Cargo sets `CARGO_BIN_EXE_<name>` for integration tests of binary targets.
fn botrix_bin() -> &'static str {
    env!("CARGO_BIN_EXE_botrix")
}

/// Write `contents` to a uniquely named temp file and return its path.
fn write_temp_config(name: &str, contents: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("botrix-test-{}-{}.yaml", std::process::id(), name));
    let mut file = std::fs::File::create(&path).expect("create temp config");
    file.write_all(contents.as_bytes()).expect("write config");
    path
}

const VALID_CONFIG: &str = r#"
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
    msg_template: "Hello ${name}"
server:
  host: "127.0.0.1"
  port: 8080
"#;

#[test]
fn validate_accepts_valid_config() {
    let path = write_temp_config("valid", VALID_CONFIG);

    let output = Command::new(botrix_bin())
        .args(["config", "validate", "--config"])
        .arg(&path)
        .output()
        .expect("run botrix");

    let _ = std::fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected exit 0, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("valid"),
        "expected success message, got: {stdout}"
    );
}

#[test]
fn validate_rejects_missing_file() {
    let output = Command::new(botrix_bin())
        .args([
            "config",
            "validate",
            "--config",
            "/nonexistent/path/to/config.yaml",
        ])
        .output()
        .expect("run botrix");

    assert!(
        !output.status.success(),
        "expected non-zero exit for missing file"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("read config file"),
        "expected descriptive read error, got: {stderr}"
    );
}

#[test]
fn validate_rejects_malformed_yaml() {
    let path = write_temp_config("malformed", "matrix: [this is: not valid yaml");

    let output = Command::new(botrix_bin())
        .args(["config", "validate", "--config"])
        .arg(&path)
        .output()
        .expect("run botrix");

    let _ = std::fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected non-zero exit for malformed YAML"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("parse config file"),
        "expected descriptive parse error, got: {stderr}"
    );
}

#[test]
fn validate_rejects_missing_required_field() {
    // `matrix.homeserver` is required.
    let config = r#"
matrix:
  destinations:
    alerts:
      room_id: "!abc:example.com"
"#;
    let path = write_temp_config("missing-field", config);

    let output = Command::new(botrix_bin())
        .args(["config", "validate", "--config"])
        .arg(&path)
        .output()
        .expect("run botrix");

    let _ = std::fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected non-zero exit for missing required field"
    );
}

#[test]
fn validate_rejects_invalid_room_id() {
    let config = r#"
matrix:
  homeserver: https://matrix.example.com
  destinations:
    alerts:
      room_id: "not-a-room-id"
"#;
    let path = write_temp_config("bad-room-id", config);

    let output = Command::new(botrix_bin())
        .args(["config", "validate", "--config"])
        .arg(&path)
        .output()
        .expect("run botrix");

    let _ = std::fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected non-zero exit for invalid room_id"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("room_id"),
        "expected room_id error, got: {stderr}"
    );
}

#[test]
fn validate_rejects_unknown_destination() {
    let config = r#"
matrix:
  homeserver: https://matrix.example.com
  destinations:
    alerts:
      room_id: "!abc:example.com"
webhooks:
  hook:
    destination: does_not_exist
    msg_template: "hi"
"#;
    let path = write_temp_config("unknown-dest", config);

    let output = Command::new(botrix_bin())
        .args(["config", "validate", "--config"])
        .arg(&path)
        .output()
        .expect("run botrix");

    let _ = std::fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected non-zero exit for unknown destination"
    );
}

#[test]
fn serve_exits_before_binding_on_invalid_config() {
    // An invalid config must cause `serve` to exit before it binds a port.
    let path = write_temp_config("serve-invalid", "matrix: [broken yaml");

    let output = Command::new(botrix_bin())
        .args(["serve", "--config"])
        .arg(&path)
        .arg("--data-dir")
        .arg(std::env::temp_dir())
        .output()
        .expect("run botrix");

    let _ = std::fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected non-zero exit for invalid config on serve"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("parse config file") || stderr.contains("invalid configuration"),
        "expected config error before bind, got: {stderr}"
    );
}

#[test]
fn ships_example_config_that_validates() {
    // The checked-in example config must pass validation so users can copy it.
    let example = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config.example.yaml");

    let output = Command::new(botrix_bin())
        .args(["config", "validate", "--config", example])
        .output()
        .expect("run botrix");

    assert!(
        output.status.success(),
        "config.example.yaml must validate, got stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
