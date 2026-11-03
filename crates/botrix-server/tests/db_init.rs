//! Integration tests for the file-based database initialization path.
//!
//! These exercise `botrix_server::db::init` against a real on-disk SQLite
//! database, verifying migrations apply, startup is idempotent across
//! "restarts", and crash recovery runs automatically on init.

use botrix_server::db;

/// A temp database path unique to this test process + a label.
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

#[tokio::test]
async fn init_creates_schema_and_is_repeatable() {
    let path = temp_db_path("schema");
    let path_str = path.to_str().unwrap().to_owned();

    // First init creates the database and applies migrations.
    let pool = db::init(&path_str).await.expect("first init");

    // The matrix_state singleton should be seeded.
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM matrix_state")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    drop(pool);

    // Second init on the same path (simulating a restart) must succeed and
    // not re-apply migrations.
    let pool2 = db::init(&path_str).await.expect("second init");
    let count2: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM matrix_state")
        .fetch_one(&pool2)
        .await
        .unwrap();
    assert_eq!(count2, 1, "restart must not duplicate the singleton");
    drop(pool2);

    cleanup(&path);
}

#[tokio::test]
async fn crash_recovery_runs_on_init() {
    let path = temp_db_path("recovery");
    let path_str = path.to_str().unwrap().to_owned();

    // First run: create the DB and insert a notification stuck in 'processing',
    // as if the process crashed mid-delivery.
    {
        let pool = db::init(&path_str).await.expect("first init");
        sqlx::query(
            "INSERT INTO notifications
                (webhook_id, destination, rendered_message, status, created_at, expires_at)
             VALUES ('hook', 'dest', 'msg', 'processing', 0, 9999999999)",
        )
        .execute(&pool)
        .await
        .expect("insert stuck notification");
        drop(pool);
    }

    // Second run: init should reset the stuck row to 'pending'.
    let pool = db::init(&path_str).await.expect("second init");
    let status: String =
        sqlx::query_scalar("SELECT status FROM notifications WHERE webhook_id = 'hook'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "pending", "stuck notification recovered on restart");
    drop(pool);

    cleanup(&path);
}
