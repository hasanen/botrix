//! Session settings: the singleton `session_settings` row.
//!
//! Stores the idle timeout and absolute session lifetime used by the session
//! middleware. Reading applies defaults on first access by upserting the
//! singleton row (`id = 1`).

use botrix_common::api::settings::Settings;

use super::{DbError, Pool};

/// Default idle timeout in seconds (1 hour).
pub const DEFAULT_IDLE_TIMEOUT_SECS: i64 = 3600;
/// Default absolute session lifetime in seconds (24 hours).
pub const DEFAULT_ABSOLUTE_LIFETIME_SECS: i64 = 86_400;

/// Current Unix timestamp in seconds.
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Read the session settings, inserting the default singleton row if absent.
pub async fn get_settings(pool: &Pool) -> Result<Settings, DbError> {
    let existing: Option<(i64, i64)> = sqlx::query_as(
        "SELECT idle_timeout_secs, absolute_lifetime_secs FROM session_settings WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;

    if let Some((idle, absolute)) = existing {
        return Ok(Settings {
            idle_timeout_secs: idle,
            absolute_lifetime_secs: absolute,
        });
    }

    // Seed defaults on first read.
    sqlx::query(
        "INSERT INTO session_settings
            (id, idle_timeout_secs, absolute_lifetime_secs, updated_at)
         VALUES (1, ?, ?, ?)",
    )
    .bind(DEFAULT_IDLE_TIMEOUT_SECS)
    .bind(DEFAULT_ABSOLUTE_LIFETIME_SECS)
    .bind(now_unix())
    .execute(pool)
    .await?;

    Ok(Settings {
        idle_timeout_secs: DEFAULT_IDLE_TIMEOUT_SECS,
        absolute_lifetime_secs: DEFAULT_ABSOLUTE_LIFETIME_SECS,
    })
}

/// Update the session settings singleton row.
pub async fn update_settings(pool: &Pool, settings: &Settings) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO session_settings
            (id, idle_timeout_secs, absolute_lifetime_secs, updated_at)
         VALUES (1, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            idle_timeout_secs = excluded.idle_timeout_secs,
            absolute_lifetime_secs = excluded.absolute_lifetime_secs,
            updated_at = excluded.updated_at",
    )
    .bind(settings.idle_timeout_secs)
    .bind(settings.absolute_lifetime_secs)
    .bind(now_unix())
    .execute(pool)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> Pool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect in-memory sqlite");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations");
        pool
    }

    #[tokio::test]
    async fn first_read_returns_defaults() {
        // Arrange
        let pool = test_pool().await;
        // Act
        let settings = get_settings(&pool).await.unwrap();
        // Assert
        assert_eq!(settings.idle_timeout_secs, DEFAULT_IDLE_TIMEOUT_SECS);
        assert_eq!(
            settings.absolute_lifetime_secs,
            DEFAULT_ABSOLUTE_LIFETIME_SECS
        );
    }

    #[tokio::test]
    async fn update_then_read_returns_new_values() {
        // Arrange
        let pool = test_pool().await;
        let new = Settings {
            idle_timeout_secs: 900,
            absolute_lifetime_secs: 7200,
        };

        // Act
        update_settings(&pool, &new).await.unwrap();
        let read = get_settings(&pool).await.unwrap();

        // Assert
        assert_eq!(read.idle_timeout_secs, 900);
        assert_eq!(read.absolute_lifetime_secs, 7200);
    }

    #[tokio::test]
    async fn settings_remain_a_singleton() {
        let pool = test_pool().await;
        get_settings(&pool).await.unwrap();
        update_settings(
            &pool,
            &Settings {
                idle_timeout_secs: 100,
                absolute_lifetime_secs: 200,
            },
        )
        .await
        .unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM session_settings")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }
}
