//! Session settings: the singleton `session_settings` row.

use botrix_common::api::settings::Settings;

use super::{now_unix, DbError, Pool};

/// Default idle timeout in seconds (1 hour).
pub const DEFAULT_IDLE_TIMEOUT_SECS: i64 = 3600;
/// Default absolute session lifetime in seconds (24 hours).
pub const DEFAULT_ABSOLUTE_LIFETIME_SECS: i64 = 86_400;
/// Largest absolute session lifetime accepted (30 days).
///
/// Also the backstop expiry given to the `tower-sessions` layer, which must
/// outlive any configurable limit so [`crate::session::enforce_timeouts`] is
/// what decides when a session ends.
pub const MAX_ABSOLUTE_LIFETIME_SECS: i64 = 30 * 86_400;

/// Read the session settings, inserting the default singleton row if absent.
pub fn get_settings(pool: &Pool) -> Result<Settings, DbError> {
    let conn = pool.get()?;
    let existing: Option<(i64, i64)> = match conn.query_row(
        "SELECT idle_timeout_secs, absolute_lifetime_secs FROM session_settings WHERE id = 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ) {
        Ok(v) => Some(v),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(e) => return Err(e.into()),
    };

    if let Some((idle, absolute)) = existing {
        return Ok(Settings {
            idle_timeout_secs: idle,
            absolute_lifetime_secs: absolute,
        });
    }

    conn.execute(
        "INSERT INTO session_settings
            (id, idle_timeout_secs, absolute_lifetime_secs, updated_at)
         VALUES (1, ?1, ?2, ?3)",
        rusqlite::params![
            DEFAULT_IDLE_TIMEOUT_SECS,
            DEFAULT_ABSOLUTE_LIFETIME_SECS,
            now_unix()
        ],
    )?;

    Ok(Settings {
        idle_timeout_secs: DEFAULT_IDLE_TIMEOUT_SECS,
        absolute_lifetime_secs: DEFAULT_ABSOLUTE_LIFETIME_SECS,
    })
}

/// Update the session settings singleton row.
pub fn update_settings(pool: &Pool, settings: &Settings) -> Result<(), DbError> {
    let conn = pool.get()?;
    conn.execute(
        "INSERT INTO session_settings
            (id, idle_timeout_secs, absolute_lifetime_secs, updated_at)
         VALUES (1, ?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET
            idle_timeout_secs = excluded.idle_timeout_secs,
            absolute_lifetime_secs = excluded.absolute_lifetime_secs,
            updated_at = excluded.updated_at",
        rusqlite::params![
            settings.idle_timeout_secs,
            settings.absolute_lifetime_secs,
            now_unix()
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_pool;

    #[test]
    fn first_read_returns_defaults() {
        let pool = test_pool();
        let s = get_settings(&pool).unwrap();
        assert_eq!(s.idle_timeout_secs, DEFAULT_IDLE_TIMEOUT_SECS);
        assert_eq!(s.absolute_lifetime_secs, DEFAULT_ABSOLUTE_LIFETIME_SECS);
    }

    #[test]
    fn update_then_read_returns_new_values() {
        let pool = test_pool();
        update_settings(
            &pool,
            &Settings {
                idle_timeout_secs: 900,
                absolute_lifetime_secs: 7200,
            },
        )
        .unwrap();
        let s = get_settings(&pool).unwrap();
        assert_eq!(s.idle_timeout_secs, 900);
        assert_eq!(s.absolute_lifetime_secs, 7200);
    }

    #[test]
    fn settings_remain_a_singleton() {
        let pool = test_pool();
        get_settings(&pool).unwrap();
        update_settings(
            &pool,
            &Settings {
                idle_timeout_secs: 100,
                absolute_lifetime_secs: 200,
            },
        )
        .unwrap();
        let conn = pool.get().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM session_settings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}
