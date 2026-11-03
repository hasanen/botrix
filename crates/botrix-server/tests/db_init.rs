//! Integration tests for the file-based database initialization path.

use botrix_server::db;

fn temp_db_path(label: &str) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("botrix-dbtest-{}-{}", std::process::id(), label));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir.push("botrix.db");
    dir
}

fn cleanup(path: &std::path::Path) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
}

#[test]
fn init_creates_schema_and_is_repeatable() {
    let path = temp_db_path("schema");
    let path_str = path.to_str().unwrap().to_owned();

    let pool = db::init(&path_str).expect("first init");
    let count: i64 = pool
        .get()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM matrix_state", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
    drop(pool);

    // Second init (simulating restart) must not re-apply migrations.
    let pool2 = db::init(&path_str).expect("second init");
    let count2: i64 = pool2
        .get()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM matrix_state", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count2, 1);
    drop(pool2);

    cleanup(&path);
}

#[test]
fn crash_recovery_runs_on_init() {
    let path = temp_db_path("recovery");
    let path_str = path.to_str().unwrap().to_owned();

    {
        let pool = db::init(&path_str).expect("first init");
        pool.get()
            .unwrap()
            .execute(
                "INSERT INTO notifications
                    (webhook_id, destination, rendered_message, status, created_at, expires_at)
                 VALUES ('hook', 'dest', 'msg', 'processing', 0, 9999999999)",
                [],
            )
            .unwrap();
        drop(pool);
    }

    let pool = db::init(&path_str).expect("second init");
    let status: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT status FROM notifications WHERE webhook_id = 'hook'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "pending");
    drop(pool);

    cleanup(&path);
}
