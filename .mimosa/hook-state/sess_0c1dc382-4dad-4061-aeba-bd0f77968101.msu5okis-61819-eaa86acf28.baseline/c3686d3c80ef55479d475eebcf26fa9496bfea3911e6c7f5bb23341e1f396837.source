use crate::error::AppError;
use crate::store::Database;
use rusqlite::Connection;
use rust_decimal::Decimal;
use std::str::FromStr;

const SAVEPOINT: &str = "usage_budget_v18";

pub fn canonicalize_daily_budget(raw: &str) -> Result<String, AppError> {
    let value = Decimal::from_str(raw.trim())
        .map_err(|_| AppError::Message("invalid_daily_budget".to_string()))?;
    if value <= Decimal::ZERO {
        return Err(AppError::Message("invalid_daily_budget".to_string()));
    }
    Ok(value.normalize().to_string())
}

fn validated_daily_budgets(conn: &Connection) -> Result<Vec<(String, String)>, AppError> {
    let mut statement = conn.prepare(
        "SELECT id, billing_kind, daily_budget_usd FROM usage_providers
         WHERE daily_budget_usd IS NOT NULL",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let rows = rows.collect::<Result<Vec<_>, _>>()?;

    rows.into_iter()
        .map(|(provider_id, billing_kind, raw)| {
            if billing_kind != "metered" {
                return Err(AppError::Database(
                    "invalid provider daily budget in database".to_string(),
                ));
            }
            canonicalize_daily_budget(&raw)
                .map(|canonical| (provider_id, canonical))
                .map_err(|_| {
                    AppError::Database("invalid provider daily budget in database".to_string())
                })
        })
        .collect()
}

pub fn normalize_daily_budgets(conn: &Connection) -> Result<(), AppError> {
    let budgets = validated_daily_budgets(conn)?;
    for (provider_id, canonical) in budgets {
        conn.execute(
            "UPDATE usage_providers SET daily_budget_usd = ?2 WHERE id = ?1",
            rusqlite::params![provider_id, canonical],
        )?;
    }
    Ok(())
}

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
    validated_daily_budgets(conn)?;
    Ok(())
}
