use crate::database::Database;
use crate::error::AppError;
use rusqlite::Connection;

const SAVEPOINT: &str = "usage_budget_v18";

pub fn migrate_v17_to_v18(conn: &Connection) -> Result<(), AppError> {
    conn.execute(&format!("SAVEPOINT {SAVEPOINT}"), [])?;
    let result = (|| {
        conn.execute(
            "ALTER TABLE usage_providers ADD COLUMN daily_budget_usd TEXT",
            [],
        )?;
        Database::set_user_version(conn, 18)?;
        validate_schema_v18_complete(conn)
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
                    "schema v18 migration failed ({cause}); rollback failed and connection state is uncertain: {error}"
                )));
            }
            Err(cause)
        }
    }
}

pub fn validate_schema_v18_complete(conn: &Connection) -> Result<(), AppError> {
    if !Database::has_column(conn, "usage_providers", "daily_budget_usd")? {
        return Err(AppError::Database(
            "incomplete schema v18: usage_providers.daily_budget_usd is missing".to_string(),
        ));
    }
    Ok(())
}
