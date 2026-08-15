//! Schema v20: per-Provider-account model pricing.
//!
//! `model_pricing` keeps its global `model_id` primary key and stays the
//! official reference catalogue. It prices subscription equivalents and is the
//! fallback for metered accounts the user has not priced.
//!
//! `provider_model_pricing` records what one Provider account actually charges
//! the user. It is scoped to `(provider_id, model_id)` because a relay may
//! resell the same model at a different rate than the vendor list price, and
//! two accounts of the same vendor may buy at different rates.
//!
//! `usage_events.pricing_origin` records which of the two catalogues produced a
//! stored estimate. It is nullable: upstream-reported and unavailable costs have
//! no pricing origin, and events written before v20 keep NULL.

use crate::database::Database;
use crate::error::AppError;
use rusqlite::Connection;

const SAVEPOINT: &str = "provider_pricing_v20";

pub fn migrate_v19_to_v20(conn: &Connection) -> Result<(), AppError> {
    conn.execute(&format!("SAVEPOINT {SAVEPOINT}"), [])?;
    let result = (|| {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS provider_model_pricing (
                provider_id TEXT NOT NULL,
                model_id TEXT NOT NULL,
                display_name TEXT NOT NULL DEFAULT '',
                input_cost_per_million TEXT NOT NULL,
                output_cost_per_million TEXT NOT NULL,
                cache_read_cost_per_million TEXT NOT NULL DEFAULT '0',
                cache_creation_cost_per_million TEXT NOT NULL DEFAULT '0',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (provider_id, model_id),
                FOREIGN KEY (provider_id) REFERENCES usage_providers(id) ON DELETE CASCADE
            )",
            [],
        )?;

        // A CHECK constraint cannot be attached by ALTER TABLE ADD COLUMN, and
        // usage_events is append-only behind immutability triggers, so the
        // 'user'/'official' domain is enforced by the single writer in
        // `usage::ingestion` instead of by the schema.
        if !Database::has_column(conn, "usage_events", "pricing_origin")? {
            conn.execute(
                "ALTER TABLE usage_events ADD COLUMN pricing_origin TEXT",
                [],
            )?;
        }

        Database::set_user_version(conn, 20)?;
        validate_schema_v20_complete(conn)
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
                    "schema v20 migration failed ({cause}); rollback failed and connection state is uncertain: {error}"
                )));
            }
            Err(cause)
        }
    }
}

pub fn validate_schema_v20_complete(conn: &Connection) -> Result<(), AppError> {
    if !Database::table_exists(conn, "provider_model_pricing")? {
        return Err(AppError::Database(
            "incomplete schema v20: provider_model_pricing is missing".to_string(),
        ));
    }
    for column in [
        "provider_id",
        "model_id",
        "input_cost_per_million",
        "output_cost_per_million",
        "cache_read_cost_per_million",
        "cache_creation_cost_per_million",
    ] {
        if !Database::has_column(conn, "provider_model_pricing", column)? {
            return Err(AppError::Database(format!(
                "incomplete schema v20: provider_model_pricing.{column} is missing"
            )));
        }
    }
    if !Database::has_column(conn, "usage_events", "pricing_origin")? {
        return Err(AppError::Database(
            "incomplete schema v20: usage_events.pricing_origin is missing".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v19_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE usage_providers (
                id TEXT PRIMARY KEY,
                billing_kind TEXT NOT NULL
             );
             CREATE TABLE usage_events (
                event_id TEXT PRIMARY KEY,
                provider_id TEXT NOT NULL,
                total_cost_usd TEXT,
                cost_source TEXT NOT NULL
                    CHECK (cost_source IN ('upstream','estimated','unavailable'))
             );
             CREATE TRIGGER usage_events_immutable_update
             BEFORE UPDATE ON usage_events
             BEGIN
                 SELECT RAISE(ABORT, 'usage_events are immutable');
             END;
             INSERT INTO usage_providers (id, billing_kind)
             VALUES ('relay-a', 'metered');
             INSERT INTO usage_events (event_id, provider_id, total_cost_usd, cost_source)
             VALUES ('event-1', 'relay-a', '0.42', 'estimated');
             PRAGMA user_version = 19;",
        )
        .unwrap();
        conn
    }

    #[test]
    fn migration_adds_provider_pricing_without_touching_existing_events() {
        let conn = v19_conn();

        migrate_v19_to_v20(&conn).unwrap();

        assert_eq!(Database::get_user_version(&conn).unwrap(), 20);
        validate_schema_v20_complete(&conn).unwrap();

        let (cost, origin): (String, Option<String>) = conn
            .query_row(
                "SELECT total_cost_usd, pricing_origin FROM usage_events
                 WHERE event_id = 'event-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(cost, "0.42");
        assert_eq!(
            origin, None,
            "pre-v20 events keep an unknown pricing origin"
        );
    }

    #[test]
    fn migration_is_idempotent_over_a_partially_upgraded_database() {
        let conn = v19_conn();
        conn.execute(
            "ALTER TABLE usage_events ADD COLUMN pricing_origin TEXT",
            [],
        )
        .unwrap();

        migrate_v19_to_v20(&conn).unwrap();

        assert_eq!(Database::get_user_version(&conn).unwrap(), 20);
        validate_schema_v20_complete(&conn).unwrap();
    }

    #[test]
    fn provider_pricing_rows_are_scoped_per_account_and_model() {
        let conn = v19_conn();
        conn.execute(
            "INSERT INTO usage_providers (id, billing_kind) VALUES ('relay-b', 'metered')",
            [],
        )
        .unwrap();
        migrate_v19_to_v20(&conn).unwrap();

        let insert = "INSERT INTO provider_model_pricing (
                provider_id, model_id, display_name,
                input_cost_per_million, output_cost_per_million,
                cache_read_cost_per_million, cache_creation_cost_per_million,
                created_at, updated_at
             ) VALUES (?1, ?2, '', ?3, ?4, '0', '0', 1, 1)";
        conn.execute(
            insert,
            rusqlite::params!["relay-a", "claude-sonnet-5", "1.5", "7.5"],
        )
        .unwrap();
        conn.execute(
            insert,
            rusqlite::params!["relay-b", "claude-sonnet-5", "2.1", "9"],
        )
        .unwrap();

        let price: String = conn
            .query_row(
                "SELECT input_cost_per_million FROM provider_model_pricing
                 WHERE provider_id = 'relay-b' AND model_id = 'claude-sonnet-5'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            price, "2.1",
            "two accounts of the same vendor keep independent prices"
        );

        assert!(
            conn.execute(
                insert,
                rusqlite::params!["relay-a", "claude-sonnet-5", "9", "9"]
            )
            .is_err(),
            "(provider_id, model_id) is the primary key"
        );
    }
}
