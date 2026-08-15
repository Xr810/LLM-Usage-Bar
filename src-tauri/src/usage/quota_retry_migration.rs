use crate::error::AppError;
use crate::store::Database;
use rusqlite::Connection;

const SAVEPOINT: &str = "quota_retry_v19";

pub fn migrate_v18_to_v19(conn: &Connection) -> Result<(), AppError> {
    conn.execute(&format!("SAVEPOINT {SAVEPOINT}"), [])?;
    let result = (|| {
        conn.execute(
            "ALTER TABLE quota_fetch_state
             ADD COLUMN consecutive_failures INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
        conn.execute(
            "UPDATE quota_fetch_state
             SET consecutive_failures = CASE WHEN stale = 1 THEN 1 ELSE 0 END,
                 stale = 0",
            [],
        )?;
        Database::set_user_version(conn, 19)?;
        validate_schema_v19_complete(conn)
    })();

    match result {
        Ok(()) => {
            conn.execute(&format!("RELEASE {SAVEPOINT}"), [])?;
            Ok(())
        }
        Err(cause) => {
            if let Err(error) =
                conn.execute_batch(&format!("ROLLBACK TO {SAVEPOINT}; RELEASE {SAVEPOINT};"))
            {
                return Err(AppError::Database(format!(
                    "schema v19 migration failed ({cause}); rollback failed and connection state is uncertain: {error}"
                )));
            }
            Err(cause)
        }
    }
}

pub fn validate_schema_v19_complete(conn: &Connection) -> Result<(), AppError> {
    if !Database::has_column(conn, "quota_fetch_state", "consecutive_failures")? {
        return Err(AppError::Database(
            "incomplete schema v19: quota_fetch_state.consecutive_failures is missing".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_converts_old_immediate_stale_state_into_first_retry() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE quota_fetch_state (
                provider_id TEXT PRIMARY KEY,
                last_attempt_at INTEGER,
                last_success_at INTEGER,
                last_error TEXT,
                stale INTEGER NOT NULL DEFAULT 0
             );
             INSERT INTO quota_fetch_state (
                provider_id, last_attempt_at, last_success_at, last_error, stale
             ) VALUES ('provider-a', 200, 100, 'timeout', 1);
             PRAGMA user_version = 18;",
        )
        .unwrap();

        migrate_v18_to_v19(&conn).unwrap();

        assert_eq!(Database::get_user_version(&conn).unwrap(), 19);
        assert_eq!(
            conn.query_row(
                "SELECT consecutive_failures, stale FROM quota_fetch_state
                 WHERE provider_id = 'provider-a'",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, bool>(1)?)),
            )
            .unwrap(),
            (1, false)
        );
        validate_schema_v19_complete(&conn).unwrap();
    }
}
