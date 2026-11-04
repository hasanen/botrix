//! Black-box tests for `botrix status`.

use std::process::Command;

fn botrix_bin() -> &'static str {
    env!("CARGO_BIN_EXE_botrix")
}

/// Create a unique empty temp directory and return its path.
fn temp_data_dir(name: &str) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("botrix-status-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn status_reports_disconnected_and_queue_counts() {
    // Arrange
    let dir = temp_data_dir("ok");
    let db_path = dir.join("botrix.db");
    let pool = botrix_server::db::init(db_path.to_str().unwrap()).expect("init db");
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO notifications
               (webhook_id, destination, rendered_message, status, created_at, expires_at)
             VALUES ('w', 'd', 'm', 'pending', 1, 2)",
            [],
        )
        .unwrap();

    // Act
    let output = Command::new(botrix_bin())
        .args(["status", "--data-dir", dir.to_str().unwrap()])
        .output()
        .expect("run botrix");

    // Assert
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("Matrix status: disconnected"), "{stdout}");
    assert!(stdout.contains("Queued:        1"), "{stdout}");
}

#[test]
fn status_does_not_modify_in_flight_notifications() {
    // Arrange: a row in `processing` must NOT be reset by the status command.
    let dir = temp_data_dir("readonly");
    let db_path = dir.join("botrix.db");
    let pool = botrix_server::db::init(db_path.to_str().unwrap()).expect("init db");
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO notifications
               (webhook_id, destination, rendered_message, status, created_at, expires_at)
             VALUES ('w', 'd', 'm', 'processing', 1, 2)",
            [],
        )
        .unwrap();

    // Act
    let output = Command::new(botrix_bin())
        .args(["status", "--data-dir", dir.to_str().unwrap()])
        .output()
        .expect("run botrix");

    // Assert
    assert!(output.status.success());
    let status: String = pool
        .get()
        .unwrap()
        .query_row("SELECT status FROM notifications", [], |r| r.get(0))
        .unwrap();
    assert_eq!(status, "processing");
}

#[test]
fn status_fails_when_database_is_missing() {
    // Arrange
    let dir = temp_data_dir("missing");

    // Act
    let output = Command::new(botrix_bin())
        .args(["status", "--data-dir", dir.to_str().unwrap()])
        .output()
        .expect("run botrix");

    // Assert
    assert!(!output.status.success());
    assert!(
        !dir.join("botrix.db").exists(),
        "must not create the database"
    );
}
