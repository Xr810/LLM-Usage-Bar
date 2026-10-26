//! Schema v29 (BE-15).
//!
//! One transaction adds the ADR-021 request columns, the hourly rollup tables,
//! the C12 counter table, and the v7.7.1 `model_pricing` rebuild. Existing
//! cache prices of `0` stay `0`. Pending columns start NULL. Any failure rolls
//! the whole step back.

use crate::error::AppError;
use crate::store::dao::hourly_rollup::rebuild_usage_hourly_rollups;
use crate::store::Database;
use rusqlite::Connection;

const SAVEPOINT: &str = "schema_v29";

const REQUEST_LOG_COLUMNS: &[(&str, &str)] = &[
    ("billing_identity", "TEXT"),
    ("key_fingerprint", "TEXT"),
    ("client_tag_id", "TEXT"),
    ("project_alias", "TEXT"),
    ("end_reason", "TEXT"),
    ("cost_source", "TEXT"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailurePoint {
    AfterPricingRebuild,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum V29FailurePoint {
    AfterPricingRebuild,
}

pub(crate) fn migrate_v28_to_v29(conn: &Connection) -> Result<(), AppError> {
    migrate_v28_to_v29_inner(conn, None)
}

#[cfg(test)]
pub(crate) fn migrate_v28_to_v29_with_failure(
    conn: &Connection,
    point: V29FailurePoint,
) -> Result<(), AppError> {
    let point = match point {
        V29FailurePoint::AfterPricingRebuild => FailurePoint::AfterPricingRebuild,
    };
    migrate_v28_to_v29_inner(conn, Some(point))
}

fn migrate_v28_to_v29_inner(
    conn: &Connection,
    failure: Option<FailurePoint>,
) -> Result<(), AppError> {
    conn.execute(&format!("SAVEPOINT {SAVEPOINT}"), [])
        .map_err(|error| AppError::Database(format!("v28 -> v29 开启 savepoint 失败: {error}")))?;

    let result = (|| {
        add_request_log_columns(conn)?;
        create_request_log_indexes(conn)?;
        create_v29_tables(conn)?;
        rebuild_model_pricing(conn)?;
        if failure == Some(FailurePoint::AfterPricingRebuild) {
            return Err(AppError::Database(
                "injected v29 migration failure".to_string(),
            ));
        }
        rebuild_usage_hourly_rollups(conn)?;
        Database::set_user_version(conn, 29)?;
        validate_schema_v29_complete(conn)
    })();

    match result {
        Ok(()) => {
            conn.execute(&format!("RELEASE {SAVEPOINT}"), [])
                .map_err(|error| {
                    AppError::Database(format!("v28 -> v29 提交 savepoint 失败: {error}"))
                })?;
            Ok(())
        }
        Err(cause) => {
            if let Err(error) =
                conn.execute_batch(&format!("ROLLBACK TO {SAVEPOINT}; RELEASE {SAVEPOINT};"))
            {
                return Err(AppError::Database(format!(
                    "schema v29 migration failed ({cause}); rollback failed and connection state is uncertain: {error}"
                )));
            }
            Err(cause)
        }
    }
}

fn add_request_log_columns(conn: &Connection) -> Result<(), AppError> {
    if !Database::table_exists(conn, "proxy_request_logs")? {
        return Err(AppError::Database(
            "v28 -> v29 找不到 proxy_request_logs".to_string(),
        ));
    }
    for (column, definition) in REQUEST_LOG_COLUMNS {
        if Database::has_column(conn, "proxy_request_logs", column)? {
            continue;
        }
        let sql = format!("ALTER TABLE proxy_request_logs ADD COLUMN \"{column}\" {definition}");
        conn.execute(&sql, [])
            .map_err(|error| AppError::Database(format!("v28 -> v29 添加请求列失败: {error}")))?;
    }
    Ok(())
}

fn create_request_log_indexes(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_request_logs_created_billing
             ON proxy_request_logs(created_at, billing_identity);
         CREATE INDEX IF NOT EXISTS idx_request_logs_client_tag_created
             ON proxy_request_logs(client_tag_id, created_at);",
    )
    .map_err(|error| AppError::Database(format!("v28 -> v29 添加请求索引失败: {error}")))
}

fn create_v29_tables(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS usage_hourly_rollups (
             hour_utc INTEGER NOT NULL,
             app_type TEXT NOT NULL,
             provider_id TEXT NOT NULL,
             model TEXT NOT NULL,
             billing_identity TEXT NOT NULL,
             data_source TEXT NOT NULL,
             request_count INTEGER NOT NULL DEFAULT 0,
             success_count INTEGER NOT NULL DEFAULT 0,
             blocked_count INTEGER NOT NULL DEFAULT 0,
             abort_count INTEGER NOT NULL DEFAULT 0,
             input_tokens INTEGER NOT NULL DEFAULT 0,
             output_tokens INTEGER NOT NULL DEFAULT 0,
             cache_read_tokens INTEGER NOT NULL DEFAULT 0,
             cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
             cost_micro_usd INTEGER NOT NULL DEFAULT 0,
             latency_ms_sum INTEGER NOT NULL DEFAULT 0,
             PRIMARY KEY (
                 hour_utc, app_type, provider_id, model, billing_identity, data_source
             )
         );
         CREATE INDEX IF NOT EXISTS idx_hourly_rollups_provider_hour
             ON usage_hourly_rollups(provider_id, hour_utc);
         CREATE TABLE IF NOT EXISTS rollup_dirty_hours (
             hour_utc INTEGER NOT NULL PRIMARY KEY
         );
         CREATE TABLE IF NOT EXISTS guard_counters (
             local_date TEXT NOT NULL PRIMARY KEY,
             blocked_count INTEGER NOT NULL DEFAULT 0,
             allowed_count INTEGER NOT NULL DEFAULT 0,
             allowed_micro_usd INTEGER NOT NULL DEFAULT 0
         );",
    )
    .map_err(|error| AppError::Database(format!("v28 -> v29 创建聚合表失败: {error}")))
}

fn rebuild_model_pricing(conn: &Connection) -> Result<(), AppError> {
    if !Database::table_exists(conn, "model_pricing")? {
        return Err(AppError::Database(
            "v28 -> v29 找不到 model_pricing".to_string(),
        ));
    }
    if Database::has_column(conn, "model_pricing", "pending_json")? {
        return Ok(());
    }
    conn.execute_batch(
        "DROP TABLE IF EXISTS model_pricing_v29;
         CREATE TABLE model_pricing_v29 (
             model_id TEXT PRIMARY KEY,
             display_name TEXT NOT NULL,
             input_cost_per_million TEXT NOT NULL,
             output_cost_per_million TEXT NOT NULL,
             cache_read_cost_per_million TEXT,
             cache_creation_cost_per_million TEXT,
             pending_json TEXT,
             pending_first_seen_at INTEGER,
             pending_seen_count INTEGER
         );
         INSERT INTO model_pricing_v29 (
             model_id, display_name, input_cost_per_million, output_cost_per_million,
             cache_read_cost_per_million, cache_creation_cost_per_million,
             pending_json, pending_first_seen_at, pending_seen_count
         )
         SELECT
             model_id, display_name, input_cost_per_million, output_cost_per_million,
             cache_read_cost_per_million, cache_creation_cost_per_million,
             NULL, NULL, NULL
         FROM model_pricing;
         DROP TABLE model_pricing;
         ALTER TABLE model_pricing_v29 RENAME TO model_pricing;",
    )
    .map_err(|error| AppError::Database(format!("v28 -> v29 重建 model_pricing 失败: {error}")))
}

pub(crate) fn validate_schema_v29_complete(conn: &Connection) -> Result<(), AppError> {
    for (column, _) in REQUEST_LOG_COLUMNS {
        if !Database::has_column(conn, "proxy_request_logs", column)? {
            return Err(AppError::Database(format!(
                "incomplete schema v29: proxy_request_logs.{column} is missing"
            )));
        }
    }
    for table in [
        "usage_hourly_rollups",
        "rollup_dirty_hours",
        "guard_counters",
    ] {
        if !Database::table_exists(conn, table)? {
            return Err(AppError::Database(format!(
                "incomplete schema v29: {table} is missing"
            )));
        }
    }
    for index in [
        "idx_request_logs_created_billing",
        "idx_request_logs_client_tag_created",
        "idx_hourly_rollups_provider_hour",
    ] {
        if !index_exists(conn, index)? {
            return Err(AppError::Database(format!(
                "incomplete schema v29: index {index} is missing"
            )));
        }
    }
    if nullable_column_count(
        conn,
        "model_pricing",
        &[
            "cache_read_cost_per_million",
            "cache_creation_cost_per_million",
        ],
    )? != 2
    {
        return Err(AppError::Database(
            "incomplete schema v29: model_pricing cache rates must be nullable".to_string(),
        ));
    }
    if nullable_column_count(
        conn,
        "model_pricing",
        &[
            "pending_json",
            "pending_first_seen_at",
            "pending_seen_count",
        ],
    )? != 3
    {
        return Err(AppError::Database(
            "incomplete schema v29: model_pricing pending columns must exist and be nullable"
                .to_string(),
        ));
    }
    let required_rates: i64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM pragma_table_info('model_pricing')
             WHERE name IN ('input_cost_per_million', 'output_cost_per_million')
               AND \"notnull\" = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| AppError::Database(format!("读取 model_pricing 列约束失败: {error}")))?;
    if required_rates != 2 {
        return Err(AppError::Database(
            "incomplete schema v29: model_pricing input and output rates must stay required"
                .to_string(),
        ));
    }
    Ok(())
}

fn index_exists(conn: &Connection, name: &str) -> Result<bool, AppError> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
            [name],
            |row| row.get(0),
        )
        .map_err(|error| AppError::Database(format!("读取索引失败: {error}")))?;
    Ok(count == 1)
}

fn nullable_column_count(
    conn: &Connection,
    table: &str,
    columns: &[&str],
) -> Result<i64, AppError> {
    let placeholders = columns.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT COUNT(*)
         FROM pragma_table_info('{table}')
         WHERE name IN ({placeholders}) AND \"notnull\" = 0"
    );
    let mut statement = conn
        .prepare(&sql)
        .map_err(|error| AppError::Database(format!("读取列约束失败: {error}")))?;
    let count = statement
        .query_row(rusqlite::params_from_iter(columns.iter()), |row| row.get(0))
        .map_err(|error| AppError::Database(format!("读取列约束失败: {error}")))?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::dao::hourly_rollup::usd_text_to_micro;

    fn v28_fixture() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE model_pricing (
                 model_id TEXT PRIMARY KEY,
                 display_name TEXT NOT NULL,
                 input_cost_per_million TEXT NOT NULL,
                 output_cost_per_million TEXT NOT NULL,
                 cache_read_cost_per_million TEXT NOT NULL DEFAULT '0',
                 cache_creation_cost_per_million TEXT NOT NULL DEFAULT '0'
             );
             INSERT INTO model_pricing (
                 model_id, display_name, input_cost_per_million, output_cost_per_million,
                 cache_read_cost_per_million, cache_creation_cost_per_million
             ) VALUES
                 ('zero-cache', 'Zero', '1', '2', '0', '0'),
                 ('priced-cache', 'Priced', '3', '15', '0.30', '3.75');
             CREATE TABLE proxy_request_logs (
                 request_id TEXT PRIMARY KEY,
                 provider_id TEXT NOT NULL,
                 app_type TEXT NOT NULL,
                 model TEXT NOT NULL,
                 input_tokens INTEGER NOT NULL DEFAULT 0,
                 output_tokens INTEGER NOT NULL DEFAULT 0,
                 cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                 cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
                 total_cost_usd TEXT NOT NULL DEFAULT '0',
                 latency_ms INTEGER NOT NULL,
                 status_code INTEGER NOT NULL,
                 created_at INTEGER NOT NULL,
                 data_source TEXT NOT NULL DEFAULT 'proxy'
             );
             INSERT INTO proxy_request_logs (
                 request_id, provider_id, app_type, model, input_tokens, output_tokens,
                 total_cost_usd, latency_ms, status_code, created_at
             ) VALUES (
                 'req-1', 'anthropic', 'claude', 'priced-cache', 11, 7, '0.10', 15, 200, 1700000000
             );
             PRAGMA user_version = 28;",
        )
        .unwrap();
        conn
    }

    fn cache_notnull(conn: &Connection, column: &str) -> i64 {
        conn.query_row(
            "SELECT \"notnull\" FROM pragma_table_info('model_pricing') WHERE name = ?1",
            [column],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn migration_from_v28_preserves_rows_keeps_zero_and_reaches_v29() {
        let conn = v28_fixture();

        migrate_v28_to_v29(&conn).unwrap();

        assert_eq!(Database::get_user_version(&conn).unwrap(), 29);
        validate_schema_v29_complete(&conn).unwrap();
        assert_eq!(cache_notnull(&conn, "cache_read_cost_per_million"), 0);
        assert_eq!(cache_notnull(&conn, "cache_creation_cost_per_million"), 0);

        let (cache_read, cache_creation, pending_json, pending_seen): (
            String,
            String,
            Option<String>,
            Option<i64>,
        ) = conn
            .query_row(
                "SELECT cache_read_cost_per_million, cache_creation_cost_per_million,
                        pending_json, pending_seen_count
                 FROM model_pricing WHERE model_id = 'zero-cache'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(cache_read, "0");
        assert_eq!(cache_creation, "0");
        assert_eq!(pending_json, None);
        assert_eq!(pending_seen, None);

        let priced: String = conn
            .query_row(
                "SELECT cache_read_cost_per_million FROM model_pricing WHERE model_id = 'priced-cache'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(priced, "0.30");
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM model_pricing", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            2i64
        );

        let (input_tokens, cost_micro, success_count): (i64, i64, i64) = conn
            .query_row(
                "SELECT input_tokens, cost_micro_usd, success_count FROM usage_hourly_rollups",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(input_tokens, 11);
        assert_eq!(cost_micro, usd_text_to_micro("0.10"));
        assert_eq!(success_count, 1);

        let log_cost: String = conn
            .query_row(
                "SELECT total_cost_usd FROM proxy_request_logs WHERE request_id = 'req-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(log_cost, "0.10");
    }

    #[test]
    fn migration_is_idempotent_and_does_not_double_count_hourly_rows() {
        let conn = v28_fixture();
        migrate_v28_to_v29(&conn).unwrap();
        Database::set_user_version(&conn, 28).unwrap();

        migrate_v28_to_v29(&conn).unwrap();

        assert_eq!(Database::get_user_version(&conn).unwrap(), 29);
        let (rows, micro): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(cost_micro_usd), 0) FROM usage_hourly_rollups",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(rows, 1);
        assert_eq!(micro, usd_text_to_micro("0.10"));
        let zero: String = conn
            .query_row(
                "SELECT cache_read_cost_per_million FROM model_pricing WHERE model_id = 'zero-cache'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(zero, "0");
    }

    #[test]
    fn failed_migration_rolls_back_columns_pricing_and_version() {
        let conn = v28_fixture();

        let error = migrate_v28_to_v29_with_failure(&conn, V29FailurePoint::AfterPricingRebuild)
            .expect_err("injected failure");
        assert!(error.to_string().contains("injected v29 migration failure"));

        assert_eq!(Database::get_user_version(&conn).unwrap(), 28);
        assert!(!Database::has_column(&conn, "proxy_request_logs", "billing_identity").unwrap());
        assert!(!Database::table_exists(&conn, "usage_hourly_rollups").unwrap());
        assert!(!Database::table_exists(&conn, "guard_counters").unwrap());
        assert!(!Database::has_column(&conn, "model_pricing", "pending_json").unwrap());
        assert_eq!(cache_notnull(&conn, "cache_read_cost_per_million"), 1);
        let (cache_read, log_cost): (String, String) = conn
            .query_row(
                "SELECT
                    (SELECT cache_read_cost_per_million FROM model_pricing WHERE model_id = 'zero-cache'),
                    (SELECT total_cost_usd FROM proxy_request_logs WHERE request_id = 'req-1')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(cache_read, "0");
        assert_eq!(log_cost, "0.10");
    }
}
