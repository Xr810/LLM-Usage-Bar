use crate::database::Database;
use crate::error::AppError;
use rusqlite::{Connection, OptionalExtension};

const SAVEPOINT: &str = "usage_light_predictions_v24";

pub(crate) fn migrate_v23_to_v24(conn: &Connection) -> Result<(), AppError> {
    conn.execute(&format!("SAVEPOINT {SAVEPOINT}"), [])?;
    let result = (|| {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS usage_light_predictions (
                prediction_id       TEXT PRIMARY KEY,
                provider_id         TEXT NOT NULL,
                window_kind         TEXT NOT NULL,
                window_resets_at    INTEGER NOT NULL,
                predicted_at        INTEGER NOT NULL,
                status              TEXT NOT NULL,
                used_percent        TEXT,
                headroom_ratio      TEXT,
                burn_rate           TEXT,
                pace_basis          TEXT NOT NULL,
                rhythm_adjustment   TEXT,
                source              TEXT NOT NULL,
                outcome             TEXT,
                outcome_peak_used   TEXT,
                outcome_resolved_at INTEGER
             );
             CREATE INDEX IF NOT EXISTS idx_usage_light_predictions_window
                 ON usage_light_predictions(provider_id, window_kind, window_resets_at);
             CREATE INDEX IF NOT EXISTS idx_usage_light_predictions_outcome_reset
                 ON usage_light_predictions(outcome, window_resets_at);",
        )?;
        Database::set_user_version(conn, 24)?;
        validate_schema_v24_complete(conn)
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
                    "schema v24 migration failed ({cause}); rollback failed and connection state is uncertain: {error}"
                )));
            }
            Err(cause)
        }
    }
}

pub(crate) fn validate_schema_v24_complete(conn: &Connection) -> Result<(), AppError> {
    if !Database::table_exists(conn, "usage_light_predictions")? {
        return Err(AppError::Database(
            "incomplete schema v24: usage_light_predictions is missing".to_string(),
        ));
    }
    for column in [
        "prediction_id",
        "provider_id",
        "window_kind",
        "window_resets_at",
        "predicted_at",
        "status",
        "used_percent",
        "headroom_ratio",
        "burn_rate",
        "pace_basis",
        "rhythm_adjustment",
        "source",
        "outcome",
        "outcome_peak_used",
        "outcome_resolved_at",
    ] {
        if !Database::has_column(conn, "usage_light_predictions", column)? {
            return Err(AppError::Database(format!(
                "incomplete schema v24: usage_light_predictions.{column} is missing"
            )));
        }
    }
    for index in [
        "idx_usage_light_predictions_window",
        "idx_usage_light_predictions_outcome_reset",
    ] {
        let exists = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1",
                [index],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Err(AppError::Database(format!(
                "incomplete schema v24: index {index} is missing"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_v23_to_v24_is_idempotent_and_preserves_existing_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE existing_rows (id TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO existing_rows (id, value) VALUES ('kept', 'unchanged');
             PRAGMA user_version = 23;",
        )
        .unwrap();

        migrate_v23_to_v24(&conn).unwrap();
        migrate_v23_to_v24(&conn).unwrap();

        assert_eq!(Database::get_user_version(&conn).unwrap(), 24);
        validate_schema_v24_complete(&conn).unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT value FROM existing_rows WHERE id = 'kept'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            "unchanged"
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM usage_light_predictions", [], |row| {
                row.get::<_, i64>(0)
            },)
                .unwrap(),
            0
        );
    }
}
