//! Schema 定义和迁移
//!
//! 负责数据库表结构的创建和版本迁移。

use super::{lock_conn, Database, SCHEMA_VERSION};
use crate::error::AppError;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize)]
struct LegacySkillMigrationRow {
    directory: String,
    app_type: String,
}

/// Schema-v14 identity boundary for app-owned discriminators.
///
/// The real v13 schema has no product origin/provenance column and stores the
/// writable Skill storage discriminator in `settings.json`, not SQLite. This
/// function therefore validates the expected tables and deliberately performs
/// no data rewrite. Future product-owned columns must be enumerated here rather
/// than introduced through a global text/JSON replacement.
pub(crate) fn migrate_app_owned_identity_v14(conn: &Connection) -> Result<(), AppError> {
    for table in ["providers", "settings"] {
        if !Database::table_exists(conn, table)? {
            return Err(AppError::Database(format!(
                "schema v14 identity boundary is missing expected v13 table: {table}"
            )));
        }
    }
    let version = Database::get_user_version(conn)?;
    if version != crate::product_identity::DATABASE_IDENTITY_SOURCE_SCHEMA_VERSION {
        return Err(AppError::Database(format!(
            "schema v14 identity boundary expected v{}, found v{version}",
            crate::product_identity::DATABASE_IDENTITY_SOURCE_SCHEMA_VERSION
        )));
    }
    Ok(())
}

impl Database {
    /// 创建所有数据库表
    pub(crate) fn create_tables(&self) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        Self::create_tables_on_conn(&conn)
    }

    /// 在指定连接上创建表（供迁移和测试使用）
    pub(crate) fn create_tables_on_conn(conn: &Connection) -> Result<(), AppError> {
        let stored_version = Self::get_user_version(conn)?;
        // 1. Providers 表
        conn.execute(
            "CREATE TABLE IF NOT EXISTS providers (
                id TEXT NOT NULL,
                app_type TEXT NOT NULL,
                name TEXT NOT NULL,
                settings_config TEXT NOT NULL,
                website_url TEXT,
                category TEXT,
                created_at INTEGER,
                sort_index INTEGER,
                notes TEXT,
                icon TEXT,
                icon_color TEXT,
                meta TEXT NOT NULL DEFAULT '{}',
                is_current BOOLEAN NOT NULL DEFAULT 0,
                in_failover_queue BOOLEAN NOT NULL DEFAULT 0,
                PRIMARY KEY (id, app_type)
            )",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

        // 2. Provider Endpoints 表
        conn.execute(
            "CREATE TABLE IF NOT EXISTS provider_endpoints (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                provider_id TEXT NOT NULL,
                app_type TEXT NOT NULL,
                url TEXT NOT NULL,
                added_at INTEGER,
                FOREIGN KEY (provider_id, app_type) REFERENCES providers(id, app_type) ON DELETE CASCADE
            )",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

        // 7. Settings 表
        conn.execute(
            "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT)",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

        // 10. Proxy Request Logs 表
        // pricing_model = 写入时实际用于计价的模型名（pricing_model_source 解析结果），
        // 回填按它重算；NULL 表示 v11 之前的历史行，'' 表示未计价的错误行。
        conn.execute("CREATE TABLE IF NOT EXISTS proxy_request_logs (
            request_id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, app_type TEXT NOT NULL, model TEXT NOT NULL,
            request_model TEXT,
            pricing_model TEXT,
            input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0, cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            input_cost_usd TEXT NOT NULL DEFAULT '0', output_cost_usd TEXT NOT NULL DEFAULT '0',
            cache_read_cost_usd TEXT NOT NULL DEFAULT '0', cache_creation_cost_usd TEXT NOT NULL DEFAULT '0',
            total_cost_usd TEXT NOT NULL DEFAULT '0', latency_ms INTEGER NOT NULL, first_token_ms INTEGER,
            duration_ms INTEGER, status_code INTEGER NOT NULL, error_message TEXT, session_id TEXT,
            provider_type TEXT, is_streaming INTEGER NOT NULL DEFAULT 0,
            cost_multiplier TEXT NOT NULL DEFAULT '1.0', created_at INTEGER NOT NULL,
            data_source TEXT NOT NULL DEFAULT 'proxy'
        )", []).map_err(|e| AppError::Database(e.to_string()))?;

        conn.execute("CREATE INDEX IF NOT EXISTS idx_request_logs_provider ON proxy_request_logs(provider_id, app_type)", [])
            .map_err(|e| AppError::Database(e.to_string()))?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_request_logs_created_at ON proxy_request_logs(created_at)", [])
            .map_err(|e| AppError::Database(e.to_string()))?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_request_logs_model ON proxy_request_logs(model)",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_request_logs_session ON proxy_request_logs(session_id)",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_request_logs_status ON proxy_request_logs(status_code)",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        Self::create_request_logs_usage_indexes_if_supported(conn)?;

        // 11. Model Pricing 表
        conn.execute(
            "CREATE TABLE IF NOT EXISTS model_pricing (
            model_id TEXT PRIMARY KEY, display_name TEXT NOT NULL,
            input_cost_per_million TEXT NOT NULL, output_cost_per_million TEXT NOT NULL,
            cache_read_cost_per_million TEXT NOT NULL DEFAULT '0',
            cache_creation_cost_per_million TEXT NOT NULL DEFAULT '0'
        )",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

        // 17. Usage Daily Rollups 表 (日聚合统计)
        // request_model 保留路由接管的「客户端别名 → 真实模型」映射维度，
        // pricing_model 保留写入时的计价基准（request 计价模式下与 model 分叉），
        // 否则明细被 prune 后接管计费不可审计；历史行迁移时填 ''（未知）。
        conn.execute(
            "CREATE TABLE IF NOT EXISTS usage_daily_rollups (
                date TEXT NOT NULL,
                app_type TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                model TEXT NOT NULL,
                request_model TEXT NOT NULL DEFAULT '',
                pricing_model TEXT NOT NULL DEFAULT '',
                request_count INTEGER NOT NULL DEFAULT 0,
                success_count INTEGER NOT NULL DEFAULT 0,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
                total_cost_usd TEXT NOT NULL DEFAULT '0',
                avg_latency_ms INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (date, app_type, provider_id, model, request_model, pricing_model)
            )",
            [],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

        // 18. v13 session line state. Fresh/pre-v14 databases need this table
        // so the normal migration chain can archive it. A v14 database must
        // never recreate the retired writable table on startup.
        if stored_version < 14 && !Self::table_exists(conn, "session_log_sync_v13_archive")? {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS session_log_sync (
                    file_path TEXT PRIMARY KEY,
                    last_modified INTEGER NOT NULL,
                    last_line_offset INTEGER NOT NULL DEFAULT 0,
                    last_synced_at INTEGER NOT NULL
                )",
                [],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        }

        // 确保 in_failover_queue 列存在（对于已存在的 v2 数据库）
        Self::add_column_if_missing(
            conn,
            "providers",
            "in_failover_queue",
            "BOOLEAN NOT NULL DEFAULT 0",
        )?;

        // 删除旧的 failover_queue 表（如果存在）
        let _ = conn.execute("DROP INDEX IF EXISTS idx_failover_queue_order", []);
        let _ = conn.execute("DROP TABLE IF EXISTS failover_queue", []);

        // 为故障转移队列创建索引（基于 providers 表）
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_providers_failover
             ON providers(app_type, in_failover_queue, sort_index)",
            [],
        );

        Ok(())
    }

    /// 应用 Schema 迁移
    pub(crate) fn apply_schema_migrations(&self) -> Result<(), AppError> {
        let roots = crate::usage::source_roots::UsageSourceRoots::resolve_runtime();
        self.apply_schema_migrations_with_roots(&roots)
    }

    pub(crate) fn apply_schema_migrations_with_roots(
        &self,
        roots: &crate::usage::source_roots::UsageSourceRoots,
    ) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        Self::apply_schema_migrations_on_conn_with_roots(&conn, roots)
    }

    /// 在指定连接上应用 Schema 迁移
    pub(crate) fn apply_schema_migrations_on_conn(conn: &Connection) -> Result<(), AppError> {
        let roots = crate::usage::source_roots::UsageSourceRoots::resolve_runtime();
        Self::apply_schema_migrations_on_conn_with_roots(conn, &roots)
    }

    pub(crate) fn apply_schema_migrations_on_conn_with_roots(
        conn: &Connection,
        roots: &crate::usage::source_roots::UsageSourceRoots,
    ) -> Result<(), AppError> {
        conn.execute("SAVEPOINT schema_migration;", [])
            .map_err(|e| AppError::Database(format!("开启迁移 savepoint 失败: {e}")))?;

        let result = (|| {
            let mut version = Self::get_user_version(conn)?;
            if version > SCHEMA_VERSION {
                return Err(AppError::Database(format!(
                    "数据库版本过新（{version}），当前应用仅支持 {SCHEMA_VERSION}，请升级应用后再尝试。"
                )));
            }
            // The built-in Provider catalog can grow without a schema-version
            // bump. Reconcile it before versioned validators inspect the
            // protected catalog so an existing v17+ database can open safely.
            if version >= 17 {
                crate::usage::system_provider_migration::reconcile_system_provider_catalog(conn)?;
            }
            while version < SCHEMA_VERSION {
                match version {
                    0 => {
                        log::info!("检测到 user_version=0，迁移到 1（补齐缺失列并设置版本）");
                        Self::migrate_v0_to_v1(conn)?;
                        Self::set_user_version(conn, 1)?;
                    }
                    1 => {
                        log::info!(
                            "迁移数据库从 v1 到 v2（添加使用统计表和完整字段，重构 skills 表）"
                        );
                        Self::migrate_v1_to_v2(conn)?;
                        Self::set_user_version(conn, 2)?;
                    }
                    2 => {
                        log::info!("迁移数据库从 v2 到 v3（Skills 统一管理架构）");
                        Self::migrate_v2_to_v3(conn)?;
                        Self::set_user_version(conn, 3)?;
                    }
                    3 => {
                        log::info!("迁移数据库从 v3 到 v4（OpenCode 支持）");
                        Self::migrate_v3_to_v4(conn)?;
                        Self::set_user_version(conn, 4)?;
                    }
                    4 => {
                        log::info!("迁移数据库从 v4 到 v5（计费模式支持）");
                        Self::migrate_v4_to_v5(conn)?;
                        Self::set_user_version(conn, 5)?;
                    }
                    5 => {
                        log::info!("迁移数据库从 v5 到 v6（使用量聚合表 + Copilot 模板类型统一）");
                        Self::migrate_v5_to_v6(conn)?;
                        Self::set_user_version(conn, 6)?;
                    }
                    6 => {
                        log::info!("迁移数据库从 v6 到 v7（Skills 更新检测支持）");
                        Self::migrate_v6_to_v7(conn)?;
                        Self::set_user_version(conn, 7)?;
                    }
                    7 => {
                        log::info!("迁移数据库从 v7 到 v8（会话日志使用追踪 + 修正模型定价）");
                        Self::migrate_v7_to_v8(conn)?;
                        Self::set_user_version(conn, 8)?;
                    }
                    8 => {
                        log::info!("迁移数据库从 v8 到 v9（全面补充模型定价）");
                        Self::migrate_v8_to_v9(conn)?;
                        Self::set_user_version(conn, 9)?;
                    }
                    9 => {
                        log::info!("迁移数据库从 v9 到 v10（添加 Hermes Agent 支持）");
                        Self::migrate_v9_to_v10(conn)?;
                        Self::set_user_version(conn, 10)?;
                    }
                    10 => {
                        log::info!("迁移数据库从 v10 到 v11（usage_daily_rollups 保留 request_model 维度）");
                        Self::migrate_v10_to_v11(conn)?;
                        Self::set_user_version(conn, 11)?;
                    }
                    11 => {
                        log::info!("迁移数据库从 v11 到 v12（添加项目 Profiles 表）");
                        Self::migrate_v11_to_v12(conn)?;
                        Self::set_user_version(conn, 12)?;
                    }
                    12 => {
                        log::info!("迁移数据库从 v12 到 v13（添加供应商感知的用量表）");
                        crate::usage::migration::migrate_v12_to_v13(conn)?;
                        Self::set_user_version(conn, 13)?;
                    }
                    13 => {
                        log::info!("迁移数据库从 v13 到 v14（归档行游标并建立来源感知字节游标）");
                        crate::usage::cursor_migration::migrate_v13_to_v14(conn, roots)?;
                        migrate_app_owned_identity_v14(conn)?;
                        Self::set_user_version(conn, 14)?;
                    }
                    14 => {
                        log::info!("迁移数据库从 v14 到 v15（添加动态用量模块和 Provider 归属）");
                        Self::validate_schema_v14_complete(conn)?;
                        crate::usage::module_migration::migrate_v14_to_v15(conn)?;
                        Self::set_user_version(conn, 15)?;
                    }
                    15 => {
                        log::info!(
                            "迁移数据库从 v15 到 v16（添加 Agent 模块、Provider 绑定和历史归属）"
                        );
                        Self::validate_schema_v15_complete(conn)?;
                        crate::usage::agent_module_migration::migrate_v15_to_v16(conn)?;
                        Self::set_user_version(conn, 16)?;
                    }
                    16 => {
                        log::info!(
                            "迁移数据库从 v16 到 v17（添加固定系统 Provider 和分层凭据元数据）"
                        );
                        Self::validate_schema_v15_complete(conn)?;
                        crate::usage::agent_module_migration::validate_schema_v16_complete(conn)?;
                        crate::usage::system_provider_migration::migrate_v16_to_v17(conn)?;
                        Self::set_user_version(conn, 17)?;
                    }
                    17 => {
                        log::info!("迁移数据库从 v17 到 v18（添加 Provider 每日用量预算）");
                        crate::usage::system_provider_migration::validate_schema_v17_complete(
                            conn,
                        )?;
                        crate::usage::budget_migration::migrate_v17_to_v18(conn)?;
                    }
                    18 => {
                        log::info!("迁移数据库从 v18 到 v19（持久化配额刷新失败退避状态）");
                        crate::usage::budget_migration::validate_schema_v18_complete(conn)?;
                        crate::usage::quota_retry_migration::migrate_v18_to_v19(conn)?;
                    }
                    19 => {
                        log::info!("迁移数据库从 v19 到 v20（Provider 维度的自定义模型定价）");
                        crate::usage::quota_retry_migration::validate_schema_v19_complete(conn)?;
                        crate::usage::provider_pricing_migration::migrate_v19_to_v20(conn)?;
                    }
                    20 => {
                        log::info!("迁移数据库从 v20 到 v21（删除已退役的功能表）");
                        crate::usage::provider_pricing_migration::validate_schema_v20_complete(
                            conn,
                        )?;
                        Self::migrate_v20_to_v21(conn)?;
                        Self::set_user_version(conn, 21)?;
                    }
                    21 => {
                        log::info!("迁移数据库从 v21 到 v22（补算缺失的历史用量成本）");
                        Self::validate_schema_v21_complete(conn)?;
                        crate::usage::cost_backfill_migration::migrate_v21_to_v22(conn)?;
                        Self::set_user_version(conn, 22)?;
                    }
                    22 => {
                        log::info!(
                            "迁移数据库从 v22 到 v23（Provider 自定义价格支持逐项回落官方价）"
                        );
                        crate::usage::cost_backfill_migration::validate_schema_v22_complete(conn)?;
                        Self::migrate_v22_to_v23(conn)?;
                        Self::set_user_version(conn, 23)?;
                    }
                    23 => {
                        log::info!("迁移数据库从 v23 到 v24（添加用量灯预测与结果日志）");
                        Self::validate_schema_v23_complete(conn)?;
                        crate::usage::usage_light_prediction_migration::migrate_v23_to_v24(conn)?;
                    }
                    24 => {
                        log::info!("迁移数据库从 v24 到 v25（添加 Provider API Key 用量快照）");
                        crate::usage::usage_light_prediction_migration::validate_schema_v24_complete(
                            conn,
                        )?;
                        Self::migrate_v24_to_v25(conn)?;
                        Self::set_user_version(conn, 25)?;
                    }
                    25 => {
                        log::info!("迁移数据库从 v25 到 v26（每个 Provider 支持多把 API Key）");
                        Self::validate_schema_v25_complete(conn)?;
                        Self::migrate_v25_to_v26(conn)?;
                        Self::set_user_version(conn, 26)?;
                    }
                    26 => {
                        log::info!("迁移数据库从 v26 到 v27（添加本地路由存储表）");
                        Self::validate_schema_v26_complete(conn)?;
                        Self::migrate_v26_to_v27(conn)?;
                        Self::set_user_version(conn, 27)?;
                    }
                    27 => {
                        log::info!("迁移数据库从 v27 到 v28（router_providers 增加凭据来源列）");
                        Self::migrate_v27_to_v28(conn)?;
                        Self::set_user_version(conn, 28)?;
                    }
                    _ => {
                        return Err(AppError::Database(format!(
                            "未知的数据库版本 {version}，无法迁移到 {SCHEMA_VERSION}"
                        )));
                    }
                }
                version = Self::get_user_version(conn)?;
            }
            if version >= 16 {
                Self::validate_schema_v15_complete(conn)?;
                crate::usage::agent_module_migration::validate_schema_v16_complete(conn)?;
            }
            if version >= 17 {
                crate::usage::system_provider_migration::validate_schema_v17_complete(conn)?;
            }
            if version >= 18 {
                crate::usage::budget_migration::validate_schema_v18_complete(conn)?;
            }
            if version >= 19 {
                crate::usage::quota_retry_migration::validate_schema_v19_complete(conn)?;
            }
            if version >= 20 {
                crate::usage::provider_pricing_migration::validate_schema_v20_complete(conn)?;
            }
            if version >= 21 {
                Self::validate_schema_v21_complete(conn)?;
            }
            if version >= 22 {
                crate::usage::cost_backfill_migration::validate_schema_v22_complete(conn)?;
            }
            if version >= 23 {
                Self::validate_schema_v23_complete(conn)?;
            }
            if version >= 24 {
                crate::usage::usage_light_prediction_migration::validate_schema_v24_complete(conn)?;
            }
            // v25's snapshot table is Provider-scoped; v26 rebuilds it around
            // key_id, so its validator only applies at exactly 25.
            if version == 25 {
                Self::validate_schema_v25_complete(conn)?;
            }
            if version >= 26 {
                Self::validate_schema_v26_complete(conn)?;
            }
            Ok(())
        })();

        match result {
            Ok(_) => {
                conn.execute("RELEASE schema_migration;", [])
                    .map_err(|error| {
                        AppError::Database(format!(
                            "提交迁移 savepoint 失败，连接状态不确定: {error}"
                        ))
                    })?;
                Ok(())
            }
            Err(error) => Err(Self::rollback_schema_migration_error(conn, error)),
        }
    }

    pub(crate) fn rollback_schema_migration_error(conn: &Connection, cause: AppError) -> AppError {
        if let Err(error) = conn.execute("ROLLBACK TO schema_migration;", []) {
            return AppError::Database(format!(
                "schema migration failed ({cause}); rollback failed and connection state is uncertain: rollback={error}; release=not attempted"
            ));
        }
        if let Err(error) = conn.execute("RELEASE schema_migration;", []) {
            return AppError::Database(format!(
                "schema migration failed ({cause}); rollback succeeded but release failed and connection state is uncertain: rollback=ok; release={error}"
            ));
        }
        cause
    }

    pub(crate) fn validate_schema_v14_complete(conn: &Connection) -> Result<(), AppError> {
        let mut missing = Vec::new();
        for table in ["usage_sync_cursors", "session_log_sync_v13_archive"] {
            if !Self::table_exists(conn, table)? {
                missing.push(table);
            }
        }
        let legacy_live = Self::table_exists(conn, "session_log_sync")?;
        if !missing.is_empty() || legacy_live {
            let mut details = Vec::new();
            if !missing.is_empty() {
                details.push(format!("missing {}", missing.join(", ")));
            }
            if legacy_live {
                details.push("retired session_log_sync is still writable".to_string());
            }
            return Err(AppError::Database(format!(
                "incomplete schema v14: {}",
                details.join("; ")
            )));
        }
        Self::validate_v14_cursor_table_shape(conn)?;
        Self::validate_v14_archive_table_shape(conn)?;
        Ok(())
    }

    pub(crate) fn validate_schema_v15_complete(conn: &Connection) -> Result<(), AppError> {
        Self::validate_schema_v14_complete(conn)?;
        if !Self::table_exists(conn, "dashboard_modules")? {
            return Err(AppError::Database(
                "incomplete schema v15: missing dashboard_modules".to_string(),
            ));
        }
        if !Self::has_column(conn, "usage_providers", "dashboard_module_id")? {
            return Err(AppError::Database(
                "incomplete schema v15: usage_providers.dashboard_module_id is missing".to_string(),
            ));
        }
        let api_modules: i64 = conn.query_row(
            "SELECT COUNT(*) FROM dashboard_modules WHERE kind = 'api' AND is_system = 1",
            [],
            |row| row.get(0),
        )?;
        if api_modules != 1 {
            return Err(AppError::Database(format!(
                "incomplete schema v15: expected one system API module, found {api_modules}"
            )));
        }
        Ok(())
    }

    fn validate_v14_cursor_table_shape(conn: &Connection) -> Result<(), AppError> {
        let columns = Self::table_column_primary_keys(conn, "usage_sync_cursors")?;
        let required = [
            "source",
            "cursor_key",
            "resource_path",
            "resource_identity",
            "modified_at_ns",
            "size_bytes",
            "byte_offset",
            "line_offset",
            "parser_state_json",
            "last_success_at",
        ];
        let missing = required
            .into_iter()
            .filter(|column| !columns.contains_key(*column))
            .collect::<Vec<_>>();
        let primary_key_is_exact = columns.get("source") == Some(&1)
            && columns.get("cursor_key") == Some(&2)
            && columns.iter().all(|(column, position)| {
                matches!(column.as_str(), "source" | "cursor_key") || *position == 0
            });
        if !missing.is_empty() || !primary_key_is_exact {
            return Err(AppError::Database(format!(
                "incomplete schema v14: malformed usage_sync_cursors (missing columns: {}; expected PRIMARY KEY(source, cursor_key))",
                if missing.is_empty() {
                    "none".to_string()
                } else {
                    missing.join(", ")
                }
            )));
        }
        Ok(())
    }

    fn validate_v14_archive_table_shape(conn: &Connection) -> Result<(), AppError> {
        let columns = Self::table_column_primary_keys(conn, "session_log_sync_v13_archive")?;
        let required = [
            "file_path",
            "last_modified",
            "last_line_offset",
            "last_synced_at",
        ];
        let missing = required
            .into_iter()
            .filter(|column| !columns.contains_key(*column))
            .collect::<Vec<_>>();
        let primary_key_is_exact = columns.get("file_path") == Some(&1)
            && columns
                .iter()
                .all(|(column, position)| column == "file_path" || *position == 0);
        if !missing.is_empty() || !primary_key_is_exact {
            return Err(AppError::Database(format!(
                "incomplete schema v14: malformed session_log_sync_v13_archive (missing columns: {}; expected file_path primary key)",
                if missing.is_empty() {
                    "none".to_string()
                } else {
                    missing.join(", ")
                }
            )));
        }
        Ok(())
    }

    fn table_column_primary_keys(
        conn: &Connection,
        table: &str,
    ) -> Result<HashMap<String, i64>, AppError> {
        let sql = format!("PRAGMA table_info('{table}')");
        let mut statement = conn
            .prepare(&sql)
            .map_err(|error| AppError::Database(format!("inspect {table} schema: {error}")))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
            })
            .map_err(|error| AppError::Database(format!("query {table} schema: {error}")))?;
        rows.collect::<Result<HashMap<_, _>, _>>()
            .map_err(|error| AppError::Database(format!("read {table} schema: {error}")))
    }

    /// v0 -> v1 迁移：补齐所有缺失列
    fn migrate_v0_to_v1(conn: &Connection) -> Result<(), AppError> {
        // providers 表
        Self::add_column_if_missing(conn, "providers", "category", "TEXT")?;
        Self::add_column_if_missing(conn, "providers", "created_at", "INTEGER")?;
        Self::add_column_if_missing(conn, "providers", "sort_index", "INTEGER")?;
        Self::add_column_if_missing(conn, "providers", "notes", "TEXT")?;
        Self::add_column_if_missing(conn, "providers", "icon", "TEXT")?;
        Self::add_column_if_missing(conn, "providers", "icon_color", "TEXT")?;
        Self::add_column_if_missing(conn, "providers", "meta", "TEXT NOT NULL DEFAULT '{}'")?;
        Self::add_column_if_missing(
            conn,
            "providers",
            "is_current",
            "BOOLEAN NOT NULL DEFAULT 0",
        )?;

        // provider_endpoints 表
        Self::add_column_if_missing(conn, "provider_endpoints", "added_at", "INTEGER")?;

        if Self::table_exists(conn, "skills")? {
            Self::add_column_if_missing(
                conn,
                "skills",
                "installed_at",
                "INTEGER NOT NULL DEFAULT 0",
            )?;
        }

        Ok(())
    }

    /// v1 -> v2 迁移：添加使用统计表和完整字段，重构 skills 表
    fn migrate_v1_to_v2(conn: &Connection) -> Result<(), AppError> {
        // providers 表字段
        Self::add_column_if_missing(
            conn,
            "providers",
            "cost_multiplier",
            "TEXT NOT NULL DEFAULT '1.0'",
        )?;
        Self::add_column_if_missing(conn, "providers", "limit_daily_usd", "TEXT")?;
        Self::add_column_if_missing(conn, "providers", "limit_monthly_usd", "TEXT")?;
        Self::add_column_if_missing(conn, "providers", "provider_type", "TEXT")?;
        Self::add_column_if_missing(
            conn,
            "providers",
            "in_failover_queue",
            "BOOLEAN NOT NULL DEFAULT 0",
        )?;

        // 添加代理超时配置字段
        if Self::table_exists(conn, "proxy_config")? {
            // 兼容旧版本缺失的基础字段
            Self::add_column_if_missing(
                conn,
                "proxy_config",
                "proxy_enabled",
                "INTEGER NOT NULL DEFAULT 0",
            )?;
            Self::add_column_if_missing(
                conn,
                "proxy_config",
                "listen_address",
                "TEXT NOT NULL DEFAULT '127.0.0.1'",
            )?;
            Self::add_column_if_missing(
                conn,
                "proxy_config",
                "listen_port",
                "INTEGER NOT NULL DEFAULT 15722",
            )?;
            Self::add_column_if_missing(
                conn,
                "proxy_config",
                "enable_logging",
                "INTEGER NOT NULL DEFAULT 1",
            )?;

            Self::add_column_if_missing(
                conn,
                "proxy_config",
                "streaming_first_byte_timeout",
                "INTEGER NOT NULL DEFAULT 60",
            )?;
            Self::add_column_if_missing(
                conn,
                "proxy_config",
                "streaming_idle_timeout",
                "INTEGER NOT NULL DEFAULT 120",
            )?;
            Self::add_column_if_missing(
                conn,
                "proxy_config",
                "non_streaming_timeout",
                "INTEGER NOT NULL DEFAULT 600",
            )?;
        }

        // 删除旧的 failover_queue 表（如果存在）
        conn.execute("DROP INDEX IF EXISTS idx_failover_queue_order", [])
            .map_err(|e| AppError::Database(format!("删除 failover_queue 索引失败: {e}")))?;
        conn.execute("DROP TABLE IF EXISTS failover_queue", [])
            .map_err(|e| AppError::Database(format!("删除 failover_queue 表失败: {e}")))?;

        // 创建 failover 索引
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_providers_failover
             ON providers(app_type, in_failover_queue, sort_index)",
            [],
        )
        .map_err(|e| AppError::Database(format!("创建 failover 索引失败: {e}")))?;

        // proxy_request_logs 表
        conn.execute("CREATE TABLE IF NOT EXISTS proxy_request_logs (
            request_id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, app_type TEXT NOT NULL, model TEXT NOT NULL,
            request_model TEXT,
            input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0, cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            input_cost_usd TEXT NOT NULL DEFAULT '0', output_cost_usd TEXT NOT NULL DEFAULT '0',
            cache_read_cost_usd TEXT NOT NULL DEFAULT '0', cache_creation_cost_usd TEXT NOT NULL DEFAULT '0',
            total_cost_usd TEXT NOT NULL DEFAULT '0', latency_ms INTEGER NOT NULL, first_token_ms INTEGER,
            duration_ms INTEGER, status_code INTEGER NOT NULL, error_message TEXT, session_id TEXT,
            provider_type TEXT, is_streaming INTEGER NOT NULL DEFAULT 0,
            cost_multiplier TEXT NOT NULL DEFAULT '1.0', created_at INTEGER NOT NULL
        )", [])?;

        // 为已存在的表添加新字段
        Self::add_column_if_missing(conn, "proxy_request_logs", "provider_type", "TEXT")?;
        Self::add_column_if_missing(
            conn,
            "proxy_request_logs",
            "is_streaming",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        Self::add_column_if_missing(
            conn,
            "proxy_request_logs",
            "cost_multiplier",
            "TEXT NOT NULL DEFAULT '1.0'",
        )?;
        Self::add_column_if_missing(conn, "proxy_request_logs", "first_token_ms", "INTEGER")?;
        Self::add_column_if_missing(conn, "proxy_request_logs", "duration_ms", "INTEGER")?;

        // model_pricing 表
        conn.execute(
            "CREATE TABLE IF NOT EXISTS model_pricing (
            model_id TEXT PRIMARY KEY, display_name TEXT NOT NULL,
            input_cost_per_million TEXT NOT NULL, output_cost_per_million TEXT NOT NULL,
            cache_read_cost_per_million TEXT NOT NULL DEFAULT '0',
            cache_creation_cost_per_million TEXT NOT NULL DEFAULT '0'
        )",
            [],
        )?;

        // 清空并重新插入模型定价
        conn.execute("DELETE FROM model_pricing", [])
            .map_err(|e| AppError::Database(format!("清空模型定价失败: {e}")))?;
        Self::seed_model_pricing(conn)?;

        // 重构 skills 表（添加 app_type 字段）
        Self::migrate_skills_table(conn)?;

        // 重构 proxy_config 为三行结构（每应用独立配置）
        Self::migrate_proxy_config_to_per_app(conn)?;

        Ok(())
    }

    /// 将 proxy_config 迁移为三行结构（每应用独立配置）
    fn migrate_proxy_config_to_per_app(conn: &Connection) -> Result<(), AppError> {
        // 检查是否已经是新表结构（幂等性）
        if !Self::table_exists(conn, "proxy_config")? {
            // 表不存在，跳过迁移（新安装）
            return Ok(());
        }

        if Self::has_column(conn, "proxy_config", "app_type")? {
            // 已经是三行结构，跳过迁移
            log::info!("proxy_config 已经是三行结构，跳过迁移");
            return Ok(());
        }

        // 读取旧配置
        let old_config = conn
            .query_row(
                "SELECT listen_address, listen_port, max_retries, enable_logging,
                    streaming_first_byte_timeout, streaming_idle_timeout, non_streaming_timeout
             FROM proxy_config WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i32>(1)?,
                        row.get::<_, i32>(2)?,
                        row.get::<_, i32>(3)?,
                        row.get::<_, i32>(4).unwrap_or(30),
                        row.get::<_, i32>(5).unwrap_or(60),
                        row.get::<_, i32>(6).unwrap_or(300),
                    ))
                },
            )
            .unwrap_or_else(|_| ("127.0.0.1".to_string(), 5000, 3, 1, 30, 60, 300));

        let old_cb = conn.query_row(
            "SELECT failure_threshold, success_threshold, timeout_seconds, error_rate_threshold, min_requests
             FROM circuit_breaker_config WHERE id = 1", [],
            |row| Ok((row.get::<_, i32>(0)?, row.get::<_, i32>(1)?, row.get::<_, i64>(2)?,
                      row.get::<_, f64>(3)?, row.get::<_, i32>(4)?))
        ).unwrap_or((5, 2, 60, 0.5, 10));

        let get_bool = |key: &str| -> bool {
            conn.query_row("SELECT value FROM settings WHERE key = ?", [key], |r| {
                r.get::<_, String>(0)
            })
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false)
        };

        let apps = [
            (
                "claude",
                get_bool("proxy_takeover_claude"),
                get_bool("auto_failover_enabled_claude"),
                6,
                45,
                90,
                8,
                3,
                90,
                0.6,
                15,
            ),
            (
                "codex",
                get_bool("proxy_takeover_codex"),
                get_bool("auto_failover_enabled_codex"),
                3,
                old_config.4,
                old_config.5,
                old_cb.0,
                old_cb.1,
                old_cb.2,
                old_cb.3,
                old_cb.4,
            ),
            (
                "gemini",
                get_bool("proxy_takeover_gemini"),
                get_bool("auto_failover_enabled_gemini"),
                5,
                old_config.4,
                old_config.5,
                old_cb.0,
                old_cb.1,
                old_cb.2,
                old_cb.3,
                old_cb.4,
            ),
        ];

        // 创建新表
        conn.execute("DROP TABLE IF EXISTS proxy_config_new", [])?;
        conn.execute("CREATE TABLE proxy_config_new (
            app_type TEXT PRIMARY KEY CHECK (app_type IN ('claude','codex','gemini')),
            proxy_enabled INTEGER NOT NULL DEFAULT 0, listen_address TEXT NOT NULL DEFAULT '127.0.0.1',
            listen_port INTEGER NOT NULL DEFAULT 15722, enable_logging INTEGER NOT NULL DEFAULT 1,
            enabled INTEGER NOT NULL DEFAULT 0, auto_failover_enabled INTEGER NOT NULL DEFAULT 0,
            max_retries INTEGER NOT NULL DEFAULT 3, streaming_first_byte_timeout INTEGER NOT NULL DEFAULT 60,
            streaming_idle_timeout INTEGER NOT NULL DEFAULT 120, non_streaming_timeout INTEGER NOT NULL DEFAULT 600,
            circuit_failure_threshold INTEGER NOT NULL DEFAULT 4, circuit_success_threshold INTEGER NOT NULL DEFAULT 2,
            circuit_timeout_seconds INTEGER NOT NULL DEFAULT 60, circuit_error_rate_threshold REAL NOT NULL DEFAULT 0.6,
            circuit_min_requests INTEGER NOT NULL DEFAULT 10,
            default_cost_multiplier TEXT NOT NULL DEFAULT '1',
            pricing_model_source TEXT NOT NULL DEFAULT 'response',
            created_at TEXT NOT NULL DEFAULT (datetime('now')), updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )", [])?;

        // 插入三行配置
        for (app, takeover, failover, retries, fb, idle, cb_f, cb_s, cb_t, cb_r, cb_m) in apps {
            conn.execute(
                "INSERT INTO proxy_config_new (app_type, proxy_enabled, listen_address, listen_port, enable_logging,
                 enabled, auto_failover_enabled, max_retries, streaming_first_byte_timeout, streaming_idle_timeout,
                 non_streaming_timeout, circuit_failure_threshold, circuit_success_threshold, circuit_timeout_seconds,
                 circuit_error_rate_threshold, circuit_min_requests)
                 VALUES (?1, 0, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                rusqlite::params![app, old_config.0, old_config.1, old_config.3,
                    if takeover { 1 } else { 0 }, if failover { 1 } else { 0 },
                    retries, fb, idle, old_config.6, cb_f, cb_s, cb_t, cb_r, cb_m]
            ).map_err(|e| AppError::Database(format!("插入 {app} 配置失败: {e}")))?;
        }

        // 替换表并清理
        conn.execute("DROP TABLE IF EXISTS proxy_config", [])?;
        conn.execute("ALTER TABLE proxy_config_new RENAME TO proxy_config", [])?;
        conn.execute("DROP TABLE IF EXISTS circuit_breaker_config", [])?;
        conn.execute("DELETE FROM settings WHERE key LIKE 'proxy_takeover_%'", [])?;
        conn.execute(
            "DELETE FROM settings WHERE key LIKE 'auto_failover_enabled_%'",
            [],
        )?;

        log::info!("proxy_config 已迁移为三行结构");
        Ok(())
    }

    /// 迁移 skills 表：从单 key 主键改为 (directory, app_type) 复合主键
    fn migrate_skills_table(conn: &Connection) -> Result<(), AppError> {
        if !Self::table_exists(conn, "skills")? {
            return Ok(());
        }

        // v3 结构（统一管理架构）已经是更高版本的 skills 表：
        // - 主键为 id
        // - 包含 enabled_claude / enabled_codex / enabled_gemini 等列
        // 在这种情况下，不应再执行 v1 -> v2 的迁移逻辑，否则会因列不匹配而失败。
        if Self::has_column(conn, "skills", "enabled_claude")?
            || Self::has_column(conn, "skills", "id")?
        {
            log::info!("skills 表已经是 v3 结构，跳过 v1 -> v2 迁移");
            return Ok(());
        }

        // 检查是否已经是新表结构
        if Self::has_column(conn, "skills", "app_type")? {
            log::info!("skills 表已经包含 app_type 字段，跳过迁移");
            return Ok(());
        }

        log::info!("开始迁移 skills 表...");

        // 1. 重命名旧表
        conn.execute("ALTER TABLE skills RENAME TO skills_old", [])
            .map_err(|e| AppError::Database(format!("重命名旧 skills 表失败: {e}")))?;

        // 2. 创建新表
        conn.execute(
            "CREATE TABLE skills (
                directory TEXT NOT NULL,
                app_type TEXT NOT NULL,
                installed BOOLEAN NOT NULL DEFAULT 0,
                installed_at INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (directory, app_type)
            )",
            [],
        )
        .map_err(|e| AppError::Database(format!("创建新 skills 表失败: {e}")))?;

        // 3. 迁移数据：解析 key 格式（如 "claude:my-skill" 或 "codex:foo"）
        //    旧数据如果没有前缀，默认为 claude
        let mut stmt = conn
            .prepare("SELECT key, installed, installed_at FROM skills_old")
            .map_err(|e| AppError::Database(format!("查询旧 skills 数据失败: {e}")))?;

        let old_skills: Vec<(String, bool, i64)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|e| AppError::Database(format!("读取旧 skills 数据失败: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(format!("解析旧 skills 数据失败: {e}")))?;

        let count = old_skills.len();

        for (key, installed, installed_at) in old_skills {
            // 解析 key: "app:directory" 或 "directory"（默认 claude）
            let (app_type, directory) = if let Some(idx) = key.find(':') {
                let (app, dir) = key.split_at(idx);
                (app.to_string(), dir[1..].to_string()) // 跳过冒号
            } else {
                ("claude".to_string(), key.clone())
            };

            conn.execute(
                "INSERT INTO skills (directory, app_type, installed, installed_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![directory, app_type, installed, installed_at],
            )
            .map_err(|e| {
                AppError::Database(format!("迁移 skill {key} 到新表失败: {e}"))
            })?;
        }

        // 4. 删除旧表
        conn.execute("DROP TABLE skills_old", [])
            .map_err(|e| AppError::Database(format!("删除旧 skills 表失败: {e}")))?;

        log::info!("skills 表迁移完成，共迁移 {count} 条记录");
        Ok(())
    }

    /// v2 -> v3 迁移：Skills 统一管理架构
    ///
    /// 将 skills 表从 (directory, app_type) 复合主键结构迁移到统一的 id 主键结构，
    /// 支持三应用启用标志（enabled_claude, enabled_codex, enabled_gemini）。
    ///
    /// 迁移策略：
    /// 1. 旧数据库只存储安装记录，真正的 skill 文件在文件系统
    /// 2. 直接重建新表结构，后续由 SkillService 在首次启动时扫描文件系统重建数据
    fn migrate_v2_to_v3(conn: &Connection) -> Result<(), AppError> {
        if !Self::table_exists(conn, "skills")? {
            return Ok(());
        }

        // 检查是否已经是新结构（通过检查是否有 enabled_claude 列）
        if Self::has_column(conn, "skills", "enabled_claude")? {
            log::info!("skills 表已经是 v3 结构，跳过迁移");
            return Ok(());
        }

        log::info!("开始迁移 skills 表到 v3 结构（统一管理架构）...");

        // 1. 备份旧数据（用于日志和后续启动迁移）
        let old_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM skills", [], |row| row.get(0))
            .unwrap_or(0);
        log::info!("旧 skills 表有 {old_count} 条记录");

        let mut stmt = conn
            .prepare(
                "SELECT directory, app_type FROM skills
                 WHERE installed = 1",
            )
            .map_err(|e| AppError::Database(format!("查询旧 skills 快照失败: {e}")))?;
        let snapshot_rows: Vec<LegacySkillMigrationRow> = stmt
            .query_map([], |row| {
                Ok(LegacySkillMigrationRow {
                    directory: row.get(0)?,
                    app_type: row.get(1)?,
                })
            })
            .map_err(|e| AppError::Database(format!("读取旧 skills 快照失败: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(format!("解析旧 skills 快照失败: {e}")))?;
        let snapshot_json = serde_json::to_string(&snapshot_rows)
            .map_err(|e| AppError::Database(format!("序列化旧 skills 快照失败: {e}")))?;

        // 标记：需要在启动后从文件系统扫描并重建 Skills 数据
        // 说明：v3 结构将 Skills 的 SSOT 迁移到 ~/.llm-usage-bar/skills/，
        // 旧表只存“安装记录”，无法直接无损迁移到新结构，因此改为启动后扫描 app 目录导入。
        let _ = conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('skills_ssot_migration_pending', 'true')",
            [],
        );
        let _ = conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('skills_ssot_migration_snapshot', ?1)",
            [snapshot_json],
        );

        // 2. 删除旧表
        conn.execute("DROP TABLE IF EXISTS skills", [])
            .map_err(|e| AppError::Database(format!("删除旧 skills 表失败: {e}")))?;

        // 3. 创建新表
        conn.execute(
            "CREATE TABLE skills (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT,
                directory TEXT NOT NULL,
                repo_owner TEXT,
                repo_name TEXT,
                repo_branch TEXT DEFAULT 'main',
                readme_url TEXT,
                enabled_claude BOOLEAN NOT NULL DEFAULT 0,
                enabled_codex BOOLEAN NOT NULL DEFAULT 0,
                enabled_gemini BOOLEAN NOT NULL DEFAULT 0,
                installed_at INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )
        .map_err(|e| AppError::Database(format!("创建新 skills 表失败: {e}")))?;

        log::info!(
            "skills 表已迁移到 v3 结构。\n\
             注意：旧的安装记录已清除，首次启动时将自动扫描文件系统重建数据。"
        );

        Ok(())
    }

    /// v3 -> v4 迁移：添加 OpenCode 支持
    ///
    /// 为 mcp_servers 和 skills 表添加 enabled_opencode 列。
    fn migrate_v3_to_v4(conn: &Connection) -> Result<(), AppError> {
        if Self::table_exists(conn, "mcp_servers")? {
            Self::add_column_if_missing(
                conn,
                "mcp_servers",
                "enabled_opencode",
                "BOOLEAN NOT NULL DEFAULT 0",
            )?;
        }

        if Self::table_exists(conn, "skills")? {
            Self::add_column_if_missing(
                conn,
                "skills",
                "enabled_opencode",
                "BOOLEAN NOT NULL DEFAULT 0",
            )?;
        }

        log::info!("v3 -> v4 迁移完成：已添加 OpenCode 支持");
        Ok(())
    }

    /// v4 -> v5 迁移：新增计费模式配置与请求模型字段
    pub(crate) fn migrate_v4_to_v5(conn: &Connection) -> Result<(), AppError> {
        if Self::table_exists(conn, "proxy_config")? {
            Self::add_column_if_missing(
                conn,
                "proxy_config",
                "default_cost_multiplier",
                "TEXT NOT NULL DEFAULT '1'",
            )?;
            Self::add_column_if_missing(
                conn,
                "proxy_config",
                "pricing_model_source",
                "TEXT NOT NULL DEFAULT 'response'",
            )?;
        }
        if Self::table_exists(conn, "proxy_request_logs")? {
            Self::add_column_if_missing(conn, "proxy_request_logs", "request_model", "TEXT")?;
        }

        log::info!("v4 -> v5 迁移完成：已添加计费模式与请求模型字段");
        Ok(())
    }

    /// v5 -> v6 迁移：添加使用量日聚合表 + 统一 Copilot 模板类型
    fn migrate_v5_to_v6(conn: &Connection) -> Result<(), AppError> {
        // 1. 添加使用量日聚合表
        conn.execute(
            "CREATE TABLE IF NOT EXISTS usage_daily_rollups (
                date TEXT NOT NULL,
                app_type TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                model TEXT NOT NULL,
                request_count INTEGER NOT NULL DEFAULT 0,
                success_count INTEGER NOT NULL DEFAULT 0,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
                total_cost_usd TEXT NOT NULL DEFAULT '0',
                avg_latency_ms INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (date, app_type, provider_id, model)
            )",
            [],
        )
        .map_err(|e| AppError::Database(format!("创建 usage_daily_rollups 表失败: {e}")))?;

        // 2. 统一 Copilot 模板类型为 github_copilot
        let mut stmt = conn
            .prepare("SELECT id, app_type, meta FROM providers")
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut updates = Vec::new();
        for row in rows {
            let (id, app_type, meta_str) = row.map_err(|e| AppError::Database(e.to_string()))?;

            if let Ok(mut meta) = serde_json::from_str::<serde_json::Value>(&meta_str) {
                let mut updated = false;

                if let Some(usage_script) = meta.get_mut("usage_script") {
                    if let Some(template_type) = usage_script.get_mut("template_type") {
                        if template_type == "copilot" {
                            *template_type =
                                serde_json::Value::String("github_copilot".to_string());
                            updated = true;
                        }
                    }
                }

                if updated {
                    let new_meta_str = serde_json::to_string(&meta)
                        .map_err(|e| AppError::Database(e.to_string()))?;
                    updates.push((id, app_type, new_meta_str));
                }
            }
        }

        for (id, app_type, new_meta) in updates {
            conn.execute(
                "UPDATE providers SET meta = ?1 WHERE id = ?2 AND app_type = ?3",
                params![new_meta, id, app_type],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        }

        log::info!("v5 -> v6 迁移完成：已添加使用量日聚合表，统一 copilot 模板类型");
        Ok(())
    }

    /// v6 -> v7: Skills 更新检测支持（content_hash + updated_at）
    fn migrate_v6_to_v7(conn: &Connection) -> Result<(), AppError> {
        if Self::table_exists(conn, "skills")? {
            Self::add_column_if_missing(conn, "skills", "content_hash", "TEXT")?;
            Self::add_column_if_missing(
                conn,
                "skills",
                "updated_at",
                "INTEGER NOT NULL DEFAULT 0",
            )?;
        }
        log::info!("v6 -> v7 迁移完成：已添加 content_hash 和 updated_at 列");
        Ok(())
    }

    /// v7 -> v8: 会话日志使用追踪（无代理模式统计支持）
    fn migrate_v7_to_v8(conn: &Connection) -> Result<(), AppError> {
        // 1. 为 proxy_request_logs 添加 data_source 列，区分数据来源
        if Self::table_exists(conn, "proxy_request_logs")? {
            Self::add_column_if_missing(
                conn,
                "proxy_request_logs",
                "data_source",
                "TEXT NOT NULL DEFAULT 'proxy'",
            )?;
            Self::create_request_logs_usage_indexes_if_supported(conn)?;
        }

        // 2. 创建会话日志同步状态表
        conn.execute(
            "CREATE TABLE IF NOT EXISTS session_log_sync (
                file_path TEXT PRIMARY KEY,
                last_modified INTEGER NOT NULL,
                last_line_offset INTEGER NOT NULL DEFAULT 0,
                last_synced_at INTEGER NOT NULL
            )",
            [],
        )
        .map_err(|e| AppError::Database(format!("创建 session_log_sync 表失败: {e}")))?;

        // 3. 修正国产模型定价：之前误将 CNY 值存为 USD 字段，统一转换为 USD
        if Self::table_exists(conn, "model_pricing")? {
            let pricing_fixes: &[(&str, &str, &str, &str, &str)] = &[
                ("deepseek-v3.2", "0.28", "0.42", "0.028", "0"),
                ("deepseek-v3.1", "0.55", "1.67", "0.055", "0"),
                ("deepseek-v3", "0.28", "1.11", "0.028", "0"),
                ("doubao-seed-code", "0.17", "1.11", "0.02", "0"),
                ("kimi-k2-thinking", "0.55", "2.20", "0.10", "0"),
                ("kimi-k2-0905", "0.55", "2.20", "0.10", "0"),
                ("kimi-k2-turbo", "1.11", "8.06", "0.14", "0"),
                ("minimax-m2.1", "0.27", "0.95", "0.03", "0"),
                ("minimax-m2.1-lightning", "0.27", "2.33", "0.03", "0"),
                ("minimax-m2", "0.27", "0.95", "0.03", "0"),
                ("glm-4.7", "0.39", "1.75", "0.04", "0"),
                ("glm-4.6", "0.28", "1.11", "0.03", "0"),
                ("mimo-v2-flash", "0.09", "0.29", "0.009", "0"),
            ];
            for (model_id, input, output, cache_read, cache_creation) in pricing_fixes {
                conn.execute(
                    "UPDATE model_pricing SET
                        input_cost_per_million = ?2,
                        output_cost_per_million = ?3,
                        cache_read_cost_per_million = ?4,
                        cache_creation_cost_per_million = ?5
                     WHERE model_id = ?1",
                    rusqlite::params![model_id, input, output, cache_read, cache_creation],
                )
                .map_err(|e| AppError::Database(format!("更新模型 {model_id} 定价失败: {e}")))?;
            }
        }

        log::info!("v7 -> v8 迁移完成：data_source 列、session_log_sync 表、修正 13 个模型定价");
        Ok(())
    }

    /// v8 → v9: 全面补充模型定价（清空 + 重新 seed）
    fn migrate_v8_to_v9(conn: &Connection) -> Result<(), AppError> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS model_pricing (
                model_id TEXT PRIMARY KEY, display_name TEXT NOT NULL,
                input_cost_per_million TEXT NOT NULL, output_cost_per_million TEXT NOT NULL,
                cache_read_cost_per_million TEXT NOT NULL DEFAULT '0',
                cache_creation_cost_per_million TEXT NOT NULL DEFAULT '0'
            )",
            [],
        )
        .map_err(|e| AppError::Database(format!("创建 model_pricing 表失败: {e}")))?;
        conn.execute("DELETE FROM model_pricing", [])
            .map_err(|e| AppError::Database(format!("清空模型定价失败: {e}")))?;
        Self::seed_model_pricing(conn)?;
        log::info!("v8 -> v9 迁移完成：已刷新全部模型定价数据");
        Ok(())
    }

    /// v9 -> v10 迁移：添加 Hermes Agent 支持
    fn migrate_v9_to_v10(conn: &Connection) -> Result<(), AppError> {
        if Self::table_exists(conn, "mcp_servers")? {
            Self::add_column_if_missing(
                conn,
                "mcp_servers",
                "enabled_hermes",
                "BOOLEAN NOT NULL DEFAULT 0",
            )?;
        }

        // skills table may not exist in databases migrated from very old versions
        if Self::table_exists(conn, "skills")? {
            Self::add_column_if_missing(
                conn,
                "skills",
                "enabled_hermes",
                "BOOLEAN NOT NULL DEFAULT 0",
            )?;
        }

        log::info!("v9 -> v10 迁移完成：已添加 Hermes Agent 支持");
        Ok(())
    }

    /// v10 -> v11：usage_daily_rollups 增加 request_model 维度（进入主键），
    /// proxy_request_logs 增加 pricing_model 列（写入时的计价基准，回填依据）。
    ///
    /// 路由接管下 model（真实上游模型）≠ request_model（客户端别名），
    /// 旧 rollup 只按 model 聚合，明细 prune 后映射关系永久丢失、计费不可审计。
    /// SQLite 改主键必须重建表；历史行的 request_model 已不可知，填 ''。
    pub(crate) fn migrate_v10_to_v11(conn: &Connection) -> Result<(), AppError> {
        // proxy_request_logs.pricing_model：NULL = v11 前的历史行（回填走
        // model → 占位符回退 request_model 的旧逻辑），'' = 未计价的错误行
        if Self::table_exists(conn, "proxy_request_logs")? {
            Self::add_column_if_missing(conn, "proxy_request_logs", "pricing_model", "TEXT")?;
        }

        if !Self::table_exists(conn, "usage_daily_rollups")? {
            log::info!("v10 -> v11：usage_daily_rollups 不存在，跳过重建");
            return Ok(());
        }

        conn.execute_batch(
            "ALTER TABLE usage_daily_rollups RENAME TO usage_daily_rollups_v10;
             CREATE TABLE usage_daily_rollups (
                 date TEXT NOT NULL,
                 app_type TEXT NOT NULL,
                 provider_id TEXT NOT NULL,
                 model TEXT NOT NULL,
                 request_model TEXT NOT NULL DEFAULT '',
                 pricing_model TEXT NOT NULL DEFAULT '',
                 request_count INTEGER NOT NULL DEFAULT 0,
                 success_count INTEGER NOT NULL DEFAULT 0,
                 input_tokens INTEGER NOT NULL DEFAULT 0,
                 output_tokens INTEGER NOT NULL DEFAULT 0,
                 cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                 cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
                 total_cost_usd TEXT NOT NULL DEFAULT '0',
                 avg_latency_ms INTEGER NOT NULL DEFAULT 0,
                 PRIMARY KEY (date, app_type, provider_id, model, request_model, pricing_model)
             );
             INSERT INTO usage_daily_rollups
                 (date, app_type, provider_id, model, request_model, pricing_model,
                  request_count, success_count, input_tokens, output_tokens,
                  cache_read_tokens, cache_creation_tokens, total_cost_usd, avg_latency_ms)
             SELECT date, app_type, provider_id, model, '', '',
                  request_count, success_count, input_tokens, output_tokens,
                  cache_read_tokens, cache_creation_tokens, total_cost_usd, avg_latency_ms
             FROM usage_daily_rollups_v10;
             DROP TABLE usage_daily_rollups_v10;",
        )
        .map_err(|e| {
            AppError::Database(format!("v10 -> v11 重建 usage_daily_rollups 失败: {e}"))
        })?;

        log::info!(
            "v10 -> v11 迁移完成：usage_daily_rollups 已保留 request_model/pricing_model 维度"
        );
        Ok(())
    }

    fn migrate_v11_to_v12(_conn: &Connection) -> Result<(), AppError> {
        Ok(())
    }

    fn migrate_v20_to_v21(conn: &Connection) -> Result<(), AppError> {
        conn.execute_batch(
            "DROP TABLE IF EXISTS mcp_servers;
             DROP TABLE IF EXISTS prompts;
             DROP TABLE IF EXISTS profiles;
             DROP TABLE IF EXISTS provider_health;
             DROP TABLE IF EXISTS skills;
             DROP TABLE IF EXISTS skill_repos;
             DROP TABLE IF EXISTS proxy_config;
             DROP TABLE IF EXISTS proxy_live_backup;
             DROP TABLE IF EXISTS stream_check_logs;",
        )
        .map_err(|e| AppError::Database(format!("v20 -> v21 删除退役表失败: {e}")))?;
        Ok(())
    }

    fn validate_schema_v21_complete(conn: &Connection) -> Result<(), AppError> {
        for table in [
            "mcp_servers",
            "prompts",
            "profiles",
            "provider_health",
            "skills",
            "skill_repos",
            "proxy_config",
            "proxy_live_backup",
            "stream_check_logs",
        ] {
            if Self::table_exists(conn, table)? {
                return Err(AppError::Database(format!(
                    "incomplete schema v21: retired table still exists: {table}"
                )));
            }
        }
        Ok(())
    }

    /// SQLite cannot remove a NOT NULL constraint in place. Rebuild only the
    /// custom-pricing table; the outer `schema_migration` savepoint makes the
    /// copy/drop/rename sequence atomic.
    fn migrate_v22_to_v23(conn: &Connection) -> Result<(), AppError> {
        conn.execute_batch(
            "CREATE TABLE provider_model_pricing_v23 (
                provider_id TEXT NOT NULL,
                model_id TEXT NOT NULL,
                display_name TEXT NOT NULL DEFAULT '',
                input_cost_per_million TEXT,
                output_cost_per_million TEXT,
                cache_read_cost_per_million TEXT,
                cache_creation_cost_per_million TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (provider_id, model_id),
                FOREIGN KEY (provider_id) REFERENCES usage_providers(id) ON DELETE CASCADE
             );
             INSERT INTO provider_model_pricing_v23 (
                provider_id, model_id, display_name,
                input_cost_per_million, output_cost_per_million,
                cache_read_cost_per_million, cache_creation_cost_per_million,
                created_at, updated_at
             )
             SELECT provider_id, model_id, display_name,
                    input_cost_per_million, output_cost_per_million,
                    cache_read_cost_per_million, cache_creation_cost_per_million,
                    created_at, updated_at
             FROM provider_model_pricing;
             DROP TABLE provider_model_pricing;
             ALTER TABLE provider_model_pricing_v23 RENAME TO provider_model_pricing;",
        )
        .map_err(|e| {
            AppError::Database(format!("v22 -> v23 重建 provider_model_pricing 失败: {e}"))
        })?;
        Ok(())
    }

    fn validate_schema_v23_complete(conn: &Connection) -> Result<(), AppError> {
        let nullable_rate_columns: i64 = conn.query_row(
            "SELECT COUNT(*)
             FROM pragma_table_info('provider_model_pricing')
             WHERE name IN (
                'input_cost_per_million', 'output_cost_per_million',
                'cache_read_cost_per_million', 'cache_creation_cost_per_million'
             ) AND \"notnull\" = 0",
            [],
            |row| row.get(0),
        )?;
        if nullable_rate_columns != 4 {
            return Err(AppError::Database(
                "incomplete schema v23: provider_model_pricing rates must be nullable".to_string(),
            ));
        }
        Ok(())
    }

    fn migrate_v24_to_v25(conn: &Connection) -> Result<(), AppError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS provider_key_usage_snapshots (
                provider_id TEXT NOT NULL PRIMARY KEY
                    REFERENCES usage_providers(id) ON DELETE RESTRICT,
                credential_version INTEGER NOT NULL CHECK (credential_version > 0),
                usage_total_usd TEXT,
                usage_daily_usd TEXT,
                usage_weekly_usd TEXT,
                usage_monthly_usd TEXT,
                limit_usd TEXT,
                limit_remaining_usd TEXT,
                is_free_tier INTEGER,
                fetched_at INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
             );",
        )
        .map_err(|error| {
            AppError::Database(format!(
                "v24 -> v25 创建 provider_key_usage_snapshots 失败: {error}"
            ))
        })?;
        Self::validate_schema_v25_complete(conn)
    }

    /// v25 -> v26 turns the one-credential-per-Provider model into a list of
    /// named keys per Provider, so a Provider's spend can be broken down by key.
    ///
    /// Two invariants make this migration safe to run on a live database:
    ///
    /// 1. **Keychain slots are carried over verbatim.** The slot string is the
    ///    lookup key for the secret in the OS keychain; renaming it here would
    ///    orphan every stored secret. New slots use the key-scoped naming, old
    ///    ones keep theirs until the key is next replaced.
    /// 2. **Only rows that actually hold a credential become keys.** Every
    ///    Provider currently has a `provider_api_credentials` row, most of them
    ///    empty placeholders at version 0. In the list model "no credential" is
    ///    an empty list, so importing those would create phantom unnamed keys.
    fn migrate_v25_to_v26(conn: &Connection) -> Result<(), AppError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS provider_api_keys (
                 id TEXT NOT NULL PRIMARY KEY CHECK (length(trim(id)) > 0),
                 provider_id TEXT NOT NULL
                     REFERENCES usage_providers(id) ON DELETE RESTRICT,
                 label TEXT NOT NULL CHECK (length(trim(label)) > 0),
                 api_key_fingerprint BLOB,
                 credential_slot TEXT,
                 credential_version INTEGER NOT NULL DEFAULT 0
                     CHECK (credential_version >= 0),
                 last_test_at INTEGER,
                 last_test_status TEXT CHECK (
                     last_test_status IS NULL OR last_test_status IN ('success','failed')
                 ),
                 last_test_error_code TEXT,
                 sort_order INTEGER NOT NULL DEFAULT 0,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 CHECK (
                     (api_key_fingerprint IS NULL AND credential_slot IS NULL)
                     OR
                     (api_key_fingerprint IS NOT NULL
                      AND typeof(api_key_fingerprint) = 'blob'
                      AND length(api_key_fingerprint) = 32
                      AND credential_slot IS NOT NULL
                      AND length(trim(credential_slot)) > 0
                      AND credential_version > 0)
                 )
             );
             CREATE UNIQUE INDEX IF NOT EXISTS idx_provider_api_keys_fingerprint
                 ON provider_api_keys(api_key_fingerprint)
                 WHERE api_key_fingerprint IS NOT NULL;
             CREATE UNIQUE INDEX IF NOT EXISTS idx_provider_api_keys_slot
                 ON provider_api_keys(credential_slot)
                 WHERE credential_slot IS NOT NULL;
             CREATE UNIQUE INDEX IF NOT EXISTS idx_provider_api_keys_label
                 ON provider_api_keys(provider_id, label);
             CREATE INDEX IF NOT EXISTS idx_provider_api_keys_provider
                 ON provider_api_keys(provider_id);",
        )
        .map_err(|error| {
            AppError::Database(format!("v25 -> v26 创建 provider_api_keys 失败: {error}"))
        })?;

        // Invariant 2: only configured credentials migrate. The label falls back
        // to the Provider's own name, which is what a single-key setup reads as.
        conn.execute(
            "INSERT OR IGNORE INTO provider_api_keys (
                 id, provider_id, label, api_key_fingerprint, credential_slot,
                 credential_version, last_test_at, last_test_status,
                 last_test_error_code, sort_order, created_at, updated_at
             )
             SELECT credential.provider_id, credential.provider_id, provider.name,
                    credential.api_key_fingerprint, credential.credential_slot,
                    credential.credential_version, credential.last_test_at,
                    credential.last_test_status, credential.last_test_error_code,
                    0, credential.created_at, credential.updated_at
             FROM provider_api_credentials credential
             JOIN usage_providers provider ON provider.id = credential.provider_id
             WHERE credential.api_key_fingerprint IS NOT NULL
               AND credential.credential_slot IS NOT NULL",
            [],
        )
        .map_err(|error| AppError::Database(format!("v25 -> v26 迁移已配置凭据失败: {error}")))?;

        Self::migrate_v25_to_v26_snapshots(conn)?;
        Self::migrate_v25_to_v26_journal(conn)?;
        Self::validate_schema_v26_complete(conn)
    }

    /// Snapshots move from Provider-scoped to key-scoped. Rows whose Provider
    /// produced no key (an unconfigured placeholder) describe spend on a
    /// credential that no longer has an owner, so they are dropped rather than
    /// re-pointed at an arbitrary key.
    fn migrate_v25_to_v26_snapshots(conn: &Connection) -> Result<(), AppError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS provider_key_usage_snapshots_v26 (
                 key_id TEXT NOT NULL PRIMARY KEY
                     REFERENCES provider_api_keys(id) ON DELETE CASCADE,
                 credential_version INTEGER NOT NULL CHECK (credential_version > 0),
                 usage_total_usd TEXT,
                 usage_daily_usd TEXT,
                 usage_weekly_usd TEXT,
                 usage_monthly_usd TEXT,
                 limit_usd TEXT,
                 limit_remaining_usd TEXT,
                 is_free_tier INTEGER,
                 fetched_at INTEGER NOT NULL,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             INSERT OR IGNORE INTO provider_key_usage_snapshots_v26 (
                 key_id, credential_version, usage_total_usd, usage_daily_usd,
                 usage_weekly_usd, usage_monthly_usd, limit_usd,
                 limit_remaining_usd, is_free_tier, fetched_at, created_at,
                 updated_at
             )
             SELECT snapshot.provider_id, snapshot.credential_version,
                    snapshot.usage_total_usd, snapshot.usage_daily_usd,
                    snapshot.usage_weekly_usd, snapshot.usage_monthly_usd,
                    snapshot.limit_usd, snapshot.limit_remaining_usd,
                    snapshot.is_free_tier, snapshot.fetched_at,
                    snapshot.created_at, snapshot.updated_at
             FROM provider_key_usage_snapshots snapshot
             JOIN provider_api_keys key ON key.id = snapshot.provider_id;
             DROP TABLE provider_key_usage_snapshots;
             ALTER TABLE provider_key_usage_snapshots_v26
                 RENAME TO provider_key_usage_snapshots;",
        )
        .map_err(|error| AppError::Database(format!("v25 -> v26 迁移用量快照失败: {error}")))
    }

    /// The credential journal is rebuilt around `key_id`. Any operation still
    /// pending across the upgrade is dropped: its staging slot is reconciled at
    /// startup, and replaying a half-finished Provider-scoped operation against
    /// the new key-scoped table would target the wrong row.
    fn migrate_v25_to_v26_journal(conn: &Connection) -> Result<(), AppError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS provider_credential_operations_v26 (
                 operation_id TEXT NOT NULL PRIMARY KEY
                     CHECK (length(trim(operation_id)) > 0),
                 key_id TEXT NOT NULL,
                 generation INTEGER NOT NULL CHECK (generation > 0),
                 operation_kind TEXT NOT NULL
                     CHECK (operation_kind IN ('set','replace','clear')),
                 status TEXT NOT NULL CHECK (status IN ('pending','committed','cleanup')),
                 staging_slot TEXT,
                 previous_slot TEXT,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 FOREIGN KEY (key_id) REFERENCES provider_api_keys(id)
                     ON DELETE RESTRICT,
                 UNIQUE (key_id, generation),
                 CHECK (staging_slot IS NULL OR length(trim(staging_slot)) > 0),
                 CHECK (previous_slot IS NULL OR length(trim(previous_slot)) > 0),
                 CHECK (operation_kind = 'clear' OR staging_slot IS NOT NULL)
             );
             INSERT OR IGNORE INTO provider_credential_operations_v26 (
                 operation_id, key_id, generation, operation_kind, status,
                 staging_slot, previous_slot, created_at, updated_at
             )
             SELECT operation.operation_id, operation.provider_id,
                    operation.generation, operation.operation_kind,
                    operation.status, operation.staging_slot,
                    operation.previous_slot, operation.created_at,
                    operation.updated_at
             FROM provider_credential_operations operation
             JOIN provider_api_keys key ON key.id = operation.provider_id
             WHERE operation.status <> 'pending';
             DROP TABLE provider_credential_operations;
             ALTER TABLE provider_credential_operations_v26
                 RENAME TO provider_credential_operations;
             CREATE UNIQUE INDEX IF NOT EXISTS
                 idx_provider_credential_operations_pending_key
                 ON provider_credential_operations(key_id)
                 WHERE status = 'pending';",
        )
        .map_err(|error| AppError::Database(format!("v25 -> v26 迁移凭据日志失败: {error}")))
    }

    fn validate_schema_v26_complete(conn: &Connection) -> Result<(), AppError> {
        if !Self::table_exists(conn, "provider_api_keys")? {
            return Err(AppError::Database(
                "incomplete schema v26: provider_api_keys is missing".to_string(),
            ));
        }
        for column in [
            "id",
            "provider_id",
            "label",
            "api_key_fingerprint",
            "credential_slot",
            "credential_version",
            "last_test_at",
            "last_test_status",
            "last_test_error_code",
            "sort_order",
        ] {
            if !Self::has_column(conn, "provider_api_keys", column)? {
                return Err(AppError::Database(format!(
                    "incomplete schema v26: provider_api_keys.{column} is missing"
                )));
            }
        }
        if !Self::has_column(conn, "provider_key_usage_snapshots", "key_id")? {
            return Err(AppError::Database(
                "incomplete schema v26: provider_key_usage_snapshots.key_id is missing".to_string(),
            ));
        }
        if !Self::has_column(conn, "provider_credential_operations", "key_id")? {
            return Err(AppError::Database(
                "incomplete schema v26: provider_credential_operations.key_id is missing"
                    .to_string(),
            ));
        }
        Ok(())
    }

    fn validate_schema_v25_complete(conn: &Connection) -> Result<(), AppError> {
        if !Self::table_exists(conn, "provider_key_usage_snapshots")? {
            return Err(AppError::Database(
                "incomplete schema v25: provider_key_usage_snapshots is missing".to_string(),
            ));
        }
        for column in [
            "provider_id",
            "credential_version",
            "usage_total_usd",
            "usage_daily_usd",
            "usage_weekly_usd",
            "usage_monthly_usd",
            "limit_usd",
            "limit_remaining_usd",
            "is_free_tier",
            "fetched_at",
            "created_at",
            "updated_at",
        ] {
            if !Self::has_column(conn, "provider_key_usage_snapshots", column)? {
                return Err(AppError::Database(format!(
                    "incomplete schema v25: provider_key_usage_snapshots.{column} is missing"
                )));
            }
        }
        Ok(())
    }

    /// v26 -> v27 迁移:添加本地路由的三张存储表。
    ///
    /// 只负责建表,不做任何路由逻辑(决策在 T4、转发在 T6)。
    /// 新库与内存库都从 user_version=0 走完整条迁移链,所以三张表只在这里
    /// 定义一次,不往 create_tables_on_conn 里抄第二份。
    fn migrate_v26_to_v27(conn: &Connection) -> Result<(), AppError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS router_providers (
                 id            TEXT PRIMARY KEY,      -- 稳定标识，如 packyapi、official
                 display_name  TEXT NOT NULL,
                 base_url      TEXT NOT NULL,
                 wire_api      TEXT NOT NULL,         -- responses | chat_completions
                 priority      INTEGER NOT NULL,      -- 越小越优先，全局顺序（v1 只有全局）
                 enabled       INTEGER NOT NULL DEFAULT 1,
                 created_at    INTEGER NOT NULL,
                 updated_at    INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS router_model_map (
                 provider_id     TEXT NOT NULL,
                 logical_model   TEXT NOT NULL,       -- Codex 发来的那个名字，如 gpt-5.6-sol
                 upstream_model  TEXT NOT NULL,       -- 该家的真实 ID，可能与 logical 相同
                 created_at      INTEGER NOT NULL,
                 PRIMARY KEY (provider_id, logical_model),
                 FOREIGN KEY (provider_id) REFERENCES router_providers(id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS router_attempts (
                 id             INTEGER PRIMARY KEY AUTOINCREMENT,
                 started_at     INTEGER NOT NULL,     -- epoch 毫秒
                 logical_model  TEXT NOT NULL,
                 provider_id    TEXT NOT NULL,
                 outcome        TEXT NOT NULL,        -- success | failed | skipped
                 failure_kind   TEXT,                 -- 失败时填，取值见 T5
                 http_status    INTEGER,
                 input_tokens   INTEGER,
                 output_tokens  INTEGER,
                 duration_ms    INTEGER
             );
             CREATE INDEX IF NOT EXISTS idx_router_attempts_started
                 ON router_attempts(started_at);
             CREATE INDEX IF NOT EXISTS idx_router_attempts_provider
                 ON router_attempts(provider_id, started_at);",
        )
        .map_err(|error| AppError::Database(format!("v26 -> v27 创建路由存储表失败: {error}")))
    }

    /// v27 -> v28 迁移:给 router_providers 加凭据来源两列。
    ///
    /// `auth_kind` 只记「凭据从哪来」,`credential_key_id` 只记「去哪把凭据取出来」
    /// (provider_api_keys.id 的引用)——两列都不承载凭据本身,不破坏 T3
    /// 「这三张表不存凭据」的规矩。
    ///
    /// 与 v26 -> v27 一样保持可重入:回卷 user_version 的迁移测试会把整条链
    /// 重跑一遍,而内存库建库时已经带上了这两列,再 ALTER 会报重复列名,
    /// 所以列已存在时跳过。
    fn migrate_v27_to_v28(conn: &Connection) -> Result<(), AppError> {
        if Self::has_column(conn, "router_providers", "auth_kind")? {
            return Ok(());
        }
        conn.execute_batch(
            "ALTER TABLE router_providers ADD COLUMN auth_kind TEXT NOT NULL DEFAULT 'none';
             ALTER TABLE router_providers ADD COLUMN credential_key_id TEXT;",
        )
        .map_err(|error| AppError::Database(format!("v27 -> v28 添加路由凭据列失败: {error}")))
    }

    /// 插入默认模型定价数据
    /// 格式: (model_id, display_name, input, output, cache_read, cache_creation)
    /// 注意: model_id 使用短横线格式（如 claude-haiku-4-5），与 API 返回的模型名称标准化后一致
    fn seed_model_pricing(conn: &Connection) -> Result<(), AppError> {
        let pricing_data = [
            // Claude Fable 5（Opus 之上的新档）
            (
                "claude-fable-5",
                "Claude Fable 5",
                "10",
                "50",
                "1.00",
                "12.50",
            ),
            (
                "claude-mythos-5",
                "Claude Mythos 5",
                "10",
                "50",
                "1.00",
                "12.50",
            ),
            // Claude 4.8 系列
            (
                "claude-opus-4-8",
                "Claude Opus 4.8",
                "5",
                "25",
                "0.50",
                "6.25",
            ),
            // Claude Sonnet 5（list 价，与 Sonnet 4.6 一致；促销 $2/$10 至 2026-08-31 不入表）
            (
                "claude-sonnet-5",
                "Claude Sonnet 5",
                "3",
                "15",
                "0.30",
                "3.75",
            ),
            // Claude 4.7 系列
            (
                "claude-opus-4-7",
                "Claude Opus 4.7",
                "5",
                "25",
                "0.50",
                "6.25",
            ),
            // Claude 4.6 系列
            (
                "claude-opus-4-6-20260206",
                "Claude Opus 4.6",
                "5",
                "25",
                "0.50",
                "6.25",
            ),
            (
                "claude-sonnet-4-6-20260217",
                "Claude Sonnet 4.6",
                "3",
                "15",
                "0.30",
                "3.75",
            ),
            // Claude 4.5 系列
            (
                "claude-opus-4-5-20251101",
                "Claude Opus 4.5",
                "5",
                "25",
                "0.50",
                "6.25",
            ),
            (
                "claude-sonnet-4-5-20250929",
                "Claude Sonnet 4.5",
                "3",
                "15",
                "0.30",
                "3.75",
            ),
            (
                "claude-haiku-4-5-20251001",
                "Claude Haiku 4.5",
                "1",
                "5",
                "0.10",
                "1.25",
            ),
            // Claude 4 系列 (Legacy Models)
            (
                "claude-opus-4-20250514",
                "Claude Opus 4",
                "15",
                "75",
                "1.50",
                "18.75",
            ),
            (
                "claude-opus-4-1-20250805",
                "Claude Opus 4.1",
                "15",
                "75",
                "1.50",
                "18.75",
            ),
            (
                "claude-sonnet-4-20250514",
                "Claude Sonnet 4",
                "3",
                "15",
                "0.30",
                "3.75",
            ),
            // Claude 3.5 系列
            (
                "claude-3-5-haiku-20241022",
                "Claude 3.5 Haiku",
                "0.80",
                "4",
                "0.08",
                "1",
            ),
            (
                "claude-3-5-sonnet-20241022",
                "Claude 3.5 Sonnet",
                "3",
                "15",
                "0.30",
                "3.75",
            ),
            // GPT-5.5 系列
            ("gpt-5.5", "GPT-5.5", "5", "30", "0.50", "0"),
            ("gpt-5.5-low", "GPT-5.5", "5", "30", "0.50", "0"),
            ("gpt-5.5-medium", "GPT-5.5", "5", "30", "0.50", "0"),
            ("gpt-5.5-high", "GPT-5.5", "5", "30", "0.50", "0"),
            ("gpt-5.5-xhigh", "GPT-5.5", "5", "30", "0.50", "0"),
            ("gpt-5.5-minimal", "GPT-5.5", "5", "30", "0.50", "0"),
            // GPT-5.4 系列
            ("gpt-5.4", "GPT-5.4", "2.50", "15", "0.25", "0"),
            ("gpt-5.4-mini", "GPT-5.4 Mini", "0.75", "4.50", "0.075", "0"),
            ("gpt-5.4-nano", "GPT-5.4 Nano", "0.20", "1.25", "0.02", "0"),
            // GPT-5.2 系列
            ("gpt-5.2", "GPT-5.2", "1.75", "14", "0.175", "0"),
            ("gpt-5.2-low", "GPT-5.2", "1.75", "14", "0.175", "0"),
            ("gpt-5.2-medium", "GPT-5.2", "1.75", "14", "0.175", "0"),
            ("gpt-5.2-high", "GPT-5.2", "1.75", "14", "0.175", "0"),
            ("gpt-5.2-xhigh", "GPT-5.2", "1.75", "14", "0.175", "0"),
            ("gpt-5.2-codex", "GPT-5.2 Codex", "1.75", "14", "0.175", "0"),
            (
                "gpt-5.2-codex-low",
                "GPT-5.2 Codex",
                "1.75",
                "14",
                "0.175",
                "0",
            ),
            (
                "gpt-5.2-codex-medium",
                "GPT-5.2 Codex",
                "1.75",
                "14",
                "0.175",
                "0",
            ),
            (
                "gpt-5.2-codex-high",
                "GPT-5.2 Codex",
                "1.75",
                "14",
                "0.175",
                "0",
            ),
            (
                "gpt-5.2-codex-xhigh",
                "GPT-5.2 Codex",
                "1.75",
                "14",
                "0.175",
                "0",
            ),
            // GPT-5.3 Codex 系列
            ("gpt-5.3-codex", "GPT-5.3 Codex", "1.75", "14", "0.175", "0"),
            (
                "gpt-5.3-codex-low",
                "GPT-5.3 Codex",
                "1.75",
                "14",
                "0.175",
                "0",
            ),
            (
                "gpt-5.3-codex-medium",
                "GPT-5.3 Codex",
                "1.75",
                "14",
                "0.175",
                "0",
            ),
            (
                "gpt-5.3-codex-high",
                "GPT-5.3 Codex",
                "1.75",
                "14",
                "0.175",
                "0",
            ),
            (
                "gpt-5.3-codex-xhigh",
                "GPT-5.3 Codex",
                "1.75",
                "14",
                "0.175",
                "0",
            ),
            // GPT-5.1 系列
            ("gpt-5.1", "GPT-5.1", "1.25", "10", "0.125", "0"),
            ("gpt-5.1-low", "GPT-5.1", "1.25", "10", "0.125", "0"),
            ("gpt-5.1-medium", "GPT-5.1", "1.25", "10", "0.125", "0"),
            ("gpt-5.1-high", "GPT-5.1", "1.25", "10", "0.125", "0"),
            ("gpt-5.1-minimal", "GPT-5.1", "1.25", "10", "0.125", "0"),
            ("gpt-5.1-codex", "GPT-5.1 Codex", "1.25", "10", "0.125", "0"),
            (
                "gpt-5.1-codex-mini",
                "GPT-5.1 Codex",
                "1.25",
                "10",
                "0.125",
                "0",
            ),
            (
                "gpt-5.1-codex-max",
                "GPT-5.1 Codex",
                "1.25",
                "10",
                "0.125",
                "0",
            ),
            (
                "gpt-5.1-codex-max-high",
                "GPT-5.1 Codex",
                "1.25",
                "10",
                "0.125",
                "0",
            ),
            (
                "gpt-5.1-codex-max-xhigh",
                "GPT-5.1 Codex",
                "1.25",
                "10",
                "0.125",
                "0",
            ),
            // GPT-5 系列
            ("gpt-5", "GPT-5", "1.25", "10", "0.125", "0"),
            ("gpt-5-low", "GPT-5", "1.25", "10", "0.125", "0"),
            ("gpt-5-medium", "GPT-5", "1.25", "10", "0.125", "0"),
            ("gpt-5-high", "GPT-5", "1.25", "10", "0.125", "0"),
            ("gpt-5-minimal", "GPT-5", "1.25", "10", "0.125", "0"),
            ("gpt-5-codex", "GPT-5 Codex", "1.25", "10", "0.125", "0"),
            ("gpt-5-codex-low", "GPT-5 Codex", "1.25", "10", "0.125", "0"),
            (
                "gpt-5-codex-medium",
                "GPT-5 Codex",
                "1.25",
                "10",
                "0.125",
                "0",
            ),
            (
                "gpt-5-codex-high",
                "GPT-5 Codex",
                "1.25",
                "10",
                "0.125",
                "0",
            ),
            (
                "gpt-5-codex-mini",
                "GPT-5 Codex",
                "1.25",
                "10",
                "0.125",
                "0",
            ),
            (
                "gpt-5-codex-mini-medium",
                "GPT-5 Codex",
                "1.25",
                "10",
                "0.125",
                "0",
            ),
            (
                "gpt-5-codex-mini-high",
                "GPT-5 Codex",
                "1.25",
                "10",
                "0.125",
                "0",
            ),
            // OpenAI Reasoning 系列
            ("o3", "OpenAI o3", "2", "8", "0.50", "0"),
            ("o4-mini", "OpenAI o4-mini", "1.10", "4.40", "0.275", "0"),
            // GPT-4.1 系列
            ("gpt-4.1", "GPT-4.1", "2", "8", "0.50", "0"),
            ("gpt-4.1-mini", "GPT-4.1 Mini", "0.40", "1.60", "0.10", "0"),
            ("gpt-4.1-nano", "GPT-4.1 Nano", "0.10", "0.40", "0.025", "0"),
            // Gemini 3.5 系列
            (
                "gemini-3.5-flash",
                "Gemini 3.5 Flash",
                "1.50",
                "9.00",
                "0.15",
                "0",
            ),
            // Gemini 3.1 系列
            (
                "gemini-3.1-pro-preview",
                "Gemini 3.1 Pro Preview",
                "2",
                "12",
                "0.20",
                "0",
            ),
            (
                "gemini-3.1-flash-lite",
                "Gemini 3.1 Flash Lite",
                "0.25",
                "1.50",
                "0.025",
                "0",
            ),
            (
                "gemini-3.1-flash-lite-preview",
                "Gemini 3.1 Flash Lite Preview",
                "0.25",
                "1.50",
                "0.025",
                "0",
            ),
            // Gemini 3 系列
            (
                "gemini-3-pro-preview",
                "Gemini 3 Pro Preview",
                "2",
                "12",
                "0.2",
                "0",
            ),
            (
                "gemini-3-flash-preview",
                "Gemini 3 Flash Preview",
                "0.5",
                "3",
                "0.05",
                "0",
            ),
            // Gemini 2.5 系列
            (
                "gemini-2.5-pro",
                "Gemini 2.5 Pro",
                "1.25",
                "10",
                "0.125",
                "0",
            ),
            (
                "gemini-2.5-flash",
                "Gemini 2.5 Flash",
                "0.3",
                "2.5",
                "0.03",
                "0",
            ),
            (
                "gemini-2.5-flash-lite",
                "Gemini 2.5 Flash Lite",
                "0.10",
                "0.40",
                "0.01",
                "0",
            ),
            // Gemini 2.0 系列
            (
                "gemini-2.0-flash",
                "Gemini 2.0 Flash",
                "0.10",
                "0.40",
                "0.025",
                "0",
            ),
            // StepFun 系列
            (
                "step-3.7-flash",
                "Step 3.7 Flash",
                "0.19",
                "1.13",
                "0.04",
                "0",
            ),
            (
                "step-3.5-flash",
                "Step 3.5 Flash",
                "0.10",
                "0.30",
                "0.02",
                "0",
            ),
            (
                "step-3.5-flash-2603",
                "Step 3.5 Flash 2603",
                "0.10",
                "0.30",
                "0.02",
                "0",
            ),
            // ====== 国产模型 (USD/1M tokens) ======
            // Doubao (字节跳动)
            // Seed 2.1 系列（2026-06 火山引擎官方 list 价，CNY 按 ~7.14 折算）：
            //   pro   输入 6 元 / 输出 30 元 / 命中 1.2 元
            //   turbo 输入 3 元 / 输出 15 元 / 命中 0.6 元
            // 「缓存存储 0.017 元/M/小时」是按时长计费的存储费，与本表 cache_creation（按 token 写入价）口径不同，置 0。
            (
                "doubao-seed-2-1-pro",
                "Doubao Seed 2.1 Pro",
                "0.84",
                "4.2",
                "0.17",
                "0",
            ),
            (
                "doubao-seed-2-1-turbo",
                "Doubao Seed 2.1 Turbo",
                "0.42",
                "2.1",
                "0.08",
                "0",
            ),
            (
                "doubao-seed-code",
                "Doubao Seed Code",
                "0.17",
                "1.11",
                "0.02",
                "0",
            ),
            (
                "doubao-seed-2-0-pro",
                "Doubao Seed 2.0 Pro",
                "0.47",
                "2.37",
                "0.09",
                "0",
            ),
            (
                "doubao-seed-2-0-code",
                "Doubao Seed 2.0 Code",
                "0.47",
                "2.37",
                "0.09",
                "0",
            ),
            (
                "doubao-seed-2-0-code-preview-latest",
                "Doubao Seed 2.0 Code Preview",
                "0.47",
                "2.37",
                "0.09",
                "0",
            ),
            (
                "doubao-seed-2-0-lite",
                "Doubao Seed 2.0 Lite",
                "0.08",
                "0.50",
                "0.017",
                "0",
            ),
            (
                "doubao-seed-2-0-mini",
                "Doubao Seed 2.0 Mini",
                "0.03",
                "0.31",
                "0.0056",
                "0",
            ),
            // DeepSeek 系列
            (
                "deepseek-v3.2",
                "DeepSeek V3.2",
                "0.28",
                "0.42",
                "0.028",
                "0",
            ),
            (
                "deepseek-v3.1",
                "DeepSeek V3.1",
                "0.55",
                "1.67",
                "0.055",
                "0",
            ),
            ("deepseek-v3", "DeepSeek V3", "0.28", "1.11", "0.028", "0"),
            (
                "deepseek-chat",
                "DeepSeek Chat",
                "0.27",
                "1.10",
                "0.07",
                "0",
            ),
            (
                "deepseek-reasoner",
                "DeepSeek Reasoner",
                "0.55",
                "2.19",
                "0.14",
                "0",
            ),
            // DeepSeek V4 系列（官方 CNY 按 1 USD ≈ 7.14 折算）
            (
                "deepseek-v4-flash",
                "DeepSeek V4 Flash",
                "0.14",
                "0.28",
                "0.0028",
                "0",
            ),
            (
                "deepseek-v4-pro",
                "DeepSeek V4 Pro",
                "0.435",
                "0.87",
                "0.003625",
                "0",
            ),
            // Kimi (月之暗面)
            (
                "kimi-k2-thinking",
                "Kimi K2 Thinking",
                "0.55",
                "2.20",
                "0.10",
                "0",
            ),
            ("kimi-k2-0905", "Kimi K2", "0.55", "2.20", "0.10", "0"),
            (
                "kimi-k2-turbo",
                "Kimi K2 Turbo",
                "1.11",
                "8.06",
                "0.14",
                "0",
            ),
            ("kimi-k2.5", "Kimi K2.5", "0.60", "3.00", "0.10", "0"),
            ("kimi-k2.6", "Kimi K2.6", "0.95", "4.00", "0.16", "0"),
            (
                "kimi-k2.7-code",
                "Kimi K2.7 Code",
                "0.95",
                "4.00",
                "0.19",
                "0",
            ),
            // MiniMax 系列
            ("minimax-m2.1", "MiniMax M2.1", "0.27", "0.95", "0.03", "0"),
            (
                "minimax-m2.1-lightning",
                "MiniMax M2.1 Lightning",
                "0.27",
                "2.33",
                "0.03",
                "0",
            ),
            ("minimax-m2", "MiniMax M2", "0.27", "0.95", "0.03", "0"),
            ("minimax-m2.5", "MiniMax M2.5", "0.15", "0.95", "0.03", "0"),
            (
                "minimax-m2.5-lightning",
                "MiniMax M2.5 Lightning",
                "0.30",
                "2.40",
                "0.03",
                "0",
            ),
            (
                "minimax-m2.7",
                "MiniMax M2.7",
                "0.30",
                "1.20",
                "0.06",
                "0.375",
            ),
            (
                "minimax-m2.7-highspeed",
                "MiniMax M2.7 Highspeed",
                "0.60",
                "2.40",
                "0.06",
                "0.375",
            ),
            ("minimax-m3", "MiniMax M3", "0.60", "2.40", "0.12", "0"),
            // GLM (智谱)
            ("glm-4.7", "GLM-4.7", "0.6", "2.2", "0.11", "0"),
            ("glm-4.6", "GLM-4.6", "0.6", "2.2", "0.11", "0"),
            ("glm-5", "GLM-5", "1", "3.2", "0.2", "0"),
            ("glm-5.1", "GLM-5.1", "1.4", "4.4", "0.26", "0"),
            ("glm-5.2", "GLM-5.2", "1.4", "4.4", "0.26", "0"),
            // MiMo (小米)
            (
                "mimo-v2-flash",
                "MiMo V2 Flash",
                "0.09",
                "0.29",
                "0.009",
                "0",
            ),
            ("mimo-v2-pro", "MiMo V2 Pro", "0.435", "0.87", "0.0036", "0"),
            ("mimo-v2.5", "MiMo V2.5", "0.14", "0.29", "0.0028", "0"),
            (
                "mimo-v2.5-pro",
                "MiMo V2.5 Pro",
                "0.435",
                "0.87",
                "0.0036",
                "0",
            ),
            // Qwen 系列 (阿里巴巴)
            ("qwen3.7-max", "Qwen3.7 Max", "2.50", "7.50", "0.25", "0"),
            ("qwen3.7-plus", "Qwen3.7 Plus", "0.40", "1.60", "0.08", "0"),
            (
                "qwen3.6-plus",
                "Qwen3.6 Plus",
                "0.325",
                "1.95",
                "0.065",
                "0",
            ),
            ("qwen3.5-plus", "Qwen3.5 Plus", "0.26", "1.56", "0.052", "0"),
            ("qwen3-max", "Qwen3 Max", "0.78", "3.90", "0", "0"),
            (
                "qwen3-235b-a22b",
                "Qwen3 235B-A22B",
                "0.70",
                "8.40",
                "0",
                "0",
            ),
            (
                "qwen3-coder-plus",
                "Qwen3 Coder Plus",
                "0.65",
                "3.25",
                "0.13",
                "0",
            ),
            (
                "qwen3-coder-480b",
                "Qwen3 Coder 480B",
                "0.65",
                "3.25",
                "0",
                "0",
            ),
            (
                "qwen3-coder-480b-a35b-instruct",
                "Qwen3 Coder 480B-A35B Instruct",
                "0.65",
                "3.25",
                "0",
                "0",
            ),
            (
                "qwen3-coder-flash",
                "Qwen3 Coder Flash",
                "0.195",
                "0.975",
                "0.039",
                "0",
            ),
            (
                "qwen3-coder-next",
                "Qwen3 Coder Next",
                "0.12",
                "0.75",
                "0",
                "0",
            ),
            ("qwq-plus", "QwQ Plus", "0.80", "2.40", "0", "0"),
            ("qwq-32b", "QwQ 32B", "0.20", "0.60", "0", "0"),
            ("qwen3-32b", "Qwen3 32B", "0.16", "0.64", "0", "0"),
            // Grok 系列 (xAI)
            ("grok-4.3", "Grok 4.3", "1.25", "2.50", "0.20", "0"),
            (
                "grok-4.20-0309-reasoning",
                "Grok 4.20 Reasoning",
                "1.25",
                "2.50",
                "0.20",
                "0",
            ),
            (
                "grok-4.20-0309-non-reasoning",
                "Grok 4.20",
                "1.25",
                "2.50",
                "0.20",
                "0",
            ),
            (
                "grok-4-1-fast-reasoning",
                "Grok 4.1 Fast Reasoning",
                "0.20",
                "0.50",
                "0.05",
                "0",
            ),
            (
                "grok-4-1-fast-non-reasoning",
                "Grok 4.1 Fast",
                "0.20",
                "0.50",
                "0.05",
                "0",
            ),
            ("grok-4", "Grok 4", "3", "15", "0.75", "0"),
            (
                "grok-code-fast-1",
                "Grok Build 0.1 (Code Fast Alias)",
                "1",
                "2",
                "0.20",
                "0",
            ),
            ("grok-build-0.1", "Grok Build 0.1", "1", "2", "0.20", "0"),
            ("grok-3", "Grok 3", "3", "15", "0.75", "0"),
            ("grok-3-mini", "Grok 3 Mini", "0.25", "0.50", "0.075", "0"),
            // Mistral 系列
            (
                "mistral-medium-3.5",
                "Mistral Medium 3.5",
                "1.50",
                "7.50",
                "0",
                "0",
            ),
            (
                "mistral-small-4",
                "Mistral Small 4",
                "0.10",
                "0.30",
                "0.01",
                "0",
            ),
            (
                "devstral-small-2-2512",
                "Devstral Small 2",
                "0.10",
                "0.30",
                "0.01",
                "0",
            ),
            (
                "magistral-small",
                "Magistral Small",
                "0.50",
                "1.50",
                "0",
                "0",
            ),
            ("codestral-2508", "Codestral", "0.30", "0.90", "0.03", "0"),
            (
                "devstral-small-1.1",
                "Devstral Small 1.1",
                "0.07",
                "0.28",
                "0.01",
                "0",
            ),
            ("devstral-2-2512", "Devstral 2", "0.40", "2", "0.04", "0"),
            (
                "devstral-medium",
                "Devstral Medium",
                "0.40",
                "2",
                "0.04",
                "0",
            ),
            (
                "mistral-large-3-2512",
                "Mistral Large 3",
                "0.50",
                "1.50",
                "0.05",
                "0",
            ),
            (
                "mistral-medium-3.1",
                "Mistral Medium 3.1",
                "0.40",
                "2",
                "0.04",
                "0",
            ),
            (
                "mistral-small-3.2-24b",
                "Mistral Small 3.2",
                "0.075",
                "0.20",
                "0.01",
                "0",
            ),
            ("magistral-medium", "Magistral Medium", "2", "5", "0", "0"),
            // Cohere 系列
            ("command-a", "Cohere Command A", "2.50", "10", "0", "0"),
            (
                "command-r-plus",
                "Cohere Command R+",
                "2.50",
                "10",
                "0",
                "0",
            ),
            ("command-r", "Cohere Command R", "0.15", "0.60", "0", "0"),
            // OpenAI 补充
            ("o3-pro", "OpenAI o3-pro", "20", "80", "0", "0"),
            ("o3-mini", "OpenAI o3-mini", "0.55", "2.20", "0.55", "0"),
            ("o1", "OpenAI o1", "15", "60", "7.50", "0"),
            ("o1-mini", "OpenAI o1-mini", "0.55", "2.20", "0.55", "0"),
            ("codex-mini", "Codex Mini", "0.75", "3", "0.025", "0"),
            ("gpt-5-mini", "GPT-5 Mini", "0.25", "2", "0.025", "0"),
            ("gpt-5-nano", "GPT-5 Nano", "0.05", "0.40", "0.005", "0"),
        ];

        let mut stmt = conn
            .prepare(
                "INSERT OR IGNORE INTO model_pricing (
                    model_id, display_name, input_cost_per_million, output_cost_per_million,
                    cache_read_cost_per_million, cache_creation_cost_per_million
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(|e| AppError::Database(format!("准备模型定价语句失败: {e}")))?;
        for (model_id, display_name, input, output, cache_read, cache_creation) in pricing_data {
            stmt.execute(rusqlite::params![
                model_id,
                display_name,
                input,
                output,
                cache_read,
                cache_creation
            ])
            .map_err(|e| AppError::Database(format!("插入模型定价失败: {e}")))?;
        }

        log::info!("已插入 {} 条默认模型定价数据", pricing_data.len());
        Ok(())
    }

    fn repair_current_model_pricing(conn: &Connection) -> Result<(), AppError> {
        let pricing_fixes = [
            // 2026-06-10 全量核价（厂商官方 list 价；CNY 按 ~7.14 折算）
            // GLM 4.6/4.7：旧值是中转/OpenRouter 折扣价，统一到 Z.ai 官方（与 glm-5/5.1 一致）
            (
                "glm-4.7", "GLM-4.7", "0.6", "2.2", "0.11", "0", "0.39", "1.75", "0.04", "0",
            ),
            (
                "glm-4.6", "GLM-4.6", "0.6", "2.2", "0.11", "0", "0.28", "1.11", "0.03", "0",
            ),
            // Grok 4.20：xAI 已降价 2/6 → 1.25/2.50
            (
                "grok-4.20-0309-reasoning",
                "Grok 4.20 Reasoning",
                "1.25",
                "2.50",
                "0.20",
                "0",
                "2",
                "6",
                "0.20",
                "0",
            ),
            (
                "grok-4.20-0309-non-reasoning",
                "Grok 4.20",
                "1.25",
                "2.50",
                "0.20",
                "0",
                "2",
                "6",
                "0.20",
                "0",
            ),
            // Kimi K2.5 官方 output 3.00
            (
                "kimi-k2.5",
                "Kimi K2.5",
                "0.60",
                "3.00",
                "0.10",
                "0",
                "0.60",
                "2.50",
                "0.10",
                "0",
            ),
            // MiniMax M2.5 input 0.15
            (
                "minimax-m2.5",
                "MiniMax M2.5",
                "0.15",
                "0.95",
                "0.03",
                "0",
                "0.12",
                "0.95",
                "0.03",
                "0",
            ),
            // Mistral Devstral 2 output 0.90 → 2（与同表 devstral-medium 一致）
            (
                "devstral-2-2512",
                "Devstral 2",
                "0.40",
                "2",
                "0.04",
                "0",
                "0.40",
                "0.90",
                "0.04",
                "0",
            ),
            // Doubao Seed 2.0：lite 旧价贵 3-4 倍 + 全系补 cache 命中价
            (
                "doubao-seed-2-0-lite",
                "Doubao Seed 2.0 Lite",
                "0.08",
                "0.50",
                "0.017",
                "0",
                "0.25",
                "2",
                "0",
                "0",
            ),
            (
                "doubao-seed-2-0-pro",
                "Doubao Seed 2.0 Pro",
                "0.47",
                "2.37",
                "0.09",
                "0",
                "0.47",
                "2.37",
                "0",
                "0",
            ),
            (
                "doubao-seed-2-0-code",
                "Doubao Seed 2.0 Code",
                "0.47",
                "2.37",
                "0.09",
                "0",
                "0.47",
                "2.37",
                "0",
                "0",
            ),
            (
                "doubao-seed-2-0-code-preview-latest",
                "Doubao Seed 2.0 Code Preview",
                "0.47",
                "2.37",
                "0.09",
                "0",
                "0.47",
                "2.37",
                "0",
                "0",
            ),
            (
                "doubao-seed-2-0-mini",
                "Doubao Seed 2.0 Mini",
                "0.03",
                "0.31",
                "0.0056",
                "0",
                "0.03",
                "0.31",
                "0",
                "0",
            ),
            // MiMo：5/27 永久降价，旧值是旧价
            (
                "mimo-v2-pro",
                "MiMo V2 Pro",
                "0.435",
                "0.87",
                "0.0036",
                "0",
                "1",
                "3",
                "0",
                "0",
            ),
            (
                "mimo-v2.5",
                "MiMo V2.5",
                "0.14",
                "0.29",
                "0.0028",
                "0",
                "0.09",
                "0.29",
                "0.009",
                "0",
            ),
            (
                "mimo-v2.5-pro",
                "MiMo V2.5 Pro",
                "0.435",
                "0.87",
                "0.0036",
                "0",
                "1",
                "3",
                "0",
                "0",
            ),
            // Qwen：官方"隐式缓存 = 输入 20%"补 cache 命中价
            (
                "qwen3.6-plus",
                "Qwen3.6 Plus",
                "0.325",
                "1.95",
                "0.065",
                "0",
                "0.325",
                "1.95",
                "0",
                "0",
            ),
            (
                "qwen3.5-plus",
                "Qwen3.5 Plus",
                "0.26",
                "1.56",
                "0.052",
                "0",
                "0.26",
                "1.56",
                "0",
                "0",
            ),
            (
                "qwen3-coder-plus",
                "Qwen3 Coder Plus",
                "0.65",
                "3.25",
                "0.13",
                "0",
                "0.65",
                "3.25",
                "0",
                "0",
            ),
            (
                "qwen3-coder-flash",
                "Qwen3 Coder Flash",
                "0.195",
                "0.975",
                "0.039",
                "0",
                "0.195",
                "0.975",
                "0",
                "0",
            ),
            (
                "deepseek-v4-flash",
                "DeepSeek V4 Flash",
                "0.14",
                "0.28",
                "0.0028",
                "0",
                "0.14",
                "0.28",
                "0.028",
                "0",
            ),
            (
                "deepseek-v4-pro",
                "DeepSeek V4 Pro",
                "0.435",
                "0.87",
                "0.003625",
                "0",
                "1.68",
                "3.36",
                "0.14",
                "0",
            ),
            (
                "glm-5", "GLM-5", "1", "3.2", "0.2", "0", "0.72", "2.30", "0", "0",
            ),
            (
                "glm-5.1", "GLM-5.1", "1.4", "4.4", "0.26", "0", "0.95", "3.15", "0", "0",
            ),
            (
                "grok-code-fast-1",
                "Grok Build 0.1 (Code Fast Alias)",
                "1",
                "2",
                "0.20",
                "0",
                "0.20",
                "1.50",
                "0.02",
                "0",
            ),
        ];

        for (
            model_id,
            display_name,
            input,
            output,
            cache_read,
            cache_creation,
            old_input,
            old_output,
            old_cache_read,
            old_cache_creation,
        ) in pricing_fixes
        {
            conn.execute(
                "UPDATE model_pricing SET
                    display_name = ?2,
                    input_cost_per_million = ?3,
                    output_cost_per_million = ?4,
                    cache_read_cost_per_million = ?5,
                    cache_creation_cost_per_million = ?6
                 WHERE model_id = ?1
                   AND input_cost_per_million = ?7
                   AND output_cost_per_million = ?8
                   AND cache_read_cost_per_million = ?9
                   AND cache_creation_cost_per_million = ?10",
                rusqlite::params![
                    model_id,
                    display_name,
                    input,
                    output,
                    cache_read,
                    cache_creation,
                    old_input,
                    old_output,
                    old_cache_read,
                    old_cache_creation
                ],
            )
            .map_err(|e| AppError::Database(format!("修复模型 {model_id} 定价失败: {e}")))?;
        }

        Ok(())
    }

    /// 确保模型定价表具备默认数据
    pub fn ensure_model_pricing_seeded(&self) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        Self::ensure_model_pricing_seeded_on_conn(&conn)
    }

    fn ensure_model_pricing_seeded_on_conn(conn: &Connection) -> Result<(), AppError> {
        // 每次启动都执行 INSERT OR IGNORE，增量追加新模型；仅修复仍等于旧内置值的定价。
        Self::seed_model_pricing(conn)?;
        Self::repair_current_model_pricing(conn)
    }

    // --- 辅助方法 ---

    pub(crate) fn get_user_version(conn: &Connection) -> Result<i32, AppError> {
        conn.query_row("PRAGMA user_version;", [], |row| row.get(0))
            .map_err(|e| AppError::Database(format!("读取 user_version 失败: {e}")))
    }

    pub(crate) fn set_user_version(conn: &Connection, version: i32) -> Result<(), AppError> {
        if version < 0 {
            return Err(AppError::Database("user_version 不能为负数".to_string()));
        }
        let sql = format!("PRAGMA user_version = {version};");
        conn.execute(&sql, [])
            .map_err(|e| AppError::Database(format!("写入 user_version 失败: {e}")))?;
        Ok(())
    }

    fn create_request_logs_usage_indexes_if_supported(conn: &Connection) -> Result<(), AppError> {
        if !Self::table_exists(conn, "proxy_request_logs")? {
            return Ok(());
        }

        let has_app_type = Self::has_column(conn, "proxy_request_logs", "app_type")?;
        let has_created_at = Self::has_column(conn, "proxy_request_logs", "created_at")?;
        if has_app_type && has_created_at {
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_request_logs_app_created_at
                 ON proxy_request_logs(app_type, created_at DESC)",
                [],
            )
            .map_err(|e| AppError::Database(format!("创建使用量应用时间索引失败: {e}")))?;
        }

        let required_columns = [
            "app_type",
            "data_source",
            "input_tokens",
            "output_tokens",
            "cache_read_tokens",
            "created_at",
            "cache_creation_tokens",
        ];
        for column in required_columns {
            if !Self::has_column(conn, "proxy_request_logs", column)? {
                return Ok(());
            }
        }

        conn.execute("DROP INDEX IF EXISTS idx_request_logs_dedup_lookup", [])
            .map_err(|e| AppError::Database(format!("删除旧使用量去重索引失败: {e}")))?;

        // 查询层为了兼容历史 NULL data_source 行，会使用
        // COALESCE(data_source, 'proxy')。普通 data_source 索引无法匹配该表达式，
        // 会让跨源去重子查询退化成大量扫描；表达式索引让 SQLite 能按同一表达式查找。
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_request_logs_dedup_lookup_expr
             ON proxy_request_logs(app_type, COALESCE(data_source, 'proxy'), input_tokens,
                                   output_tokens, cache_read_tokens, created_at,
                                   cache_creation_tokens)",
            [],
        )
        .map_err(|e| AppError::Database(format!("创建使用量去重表达式索引失败: {e}")))?;
        Ok(())
    }

    fn validate_identifier(s: &str, kind: &str) -> Result<(), AppError> {
        if s.is_empty() {
            return Err(AppError::Database(format!("{kind} 不能为空")));
        }
        if !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(AppError::Database(format!(
                "非法{kind}: {s}，仅允许字母、数字和下划线"
            )));
        }
        Ok(())
    }

    pub(crate) fn table_exists(conn: &Connection, table: &str) -> Result<bool, AppError> {
        Self::validate_identifier(table, "表名")?;

        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .map_err(|e| AppError::Database(format!("读取表名失败: {e}")))?;
        let mut rows = stmt
            .query([])
            .map_err(|e| AppError::Database(format!("查询表名失败: {e}")))?;
        while let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
            let name: String = row
                .get(0)
                .map_err(|e| AppError::Database(format!("解析表名失败: {e}")))?;
            if name.eq_ignore_ascii_case(table) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn has_column(
        conn: &Connection,
        table: &str,
        column: &str,
    ) -> Result<bool, AppError> {
        Self::validate_identifier(table, "表名")?;
        Self::validate_identifier(column, "列名")?;

        let sql = format!("PRAGMA table_info(\"{table}\");");
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| AppError::Database(format!("读取表结构失败: {e}")))?;
        let mut rows = stmt
            .query([])
            .map_err(|e| AppError::Database(format!("查询表结构失败: {e}")))?;
        while let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
            let name: String = row
                .get(1)
                .map_err(|e| AppError::Database(format!("读取列名失败: {e}")))?;
            if name.eq_ignore_ascii_case(column) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn add_column_if_missing(
        conn: &Connection,
        table: &str,
        column: &str,
        definition: &str,
    ) -> Result<bool, AppError> {
        Self::validate_identifier(table, "表名")?;
        Self::validate_identifier(column, "列名")?;

        if !Self::table_exists(conn, table)? {
            return Err(AppError::Database(format!(
                "表 {table} 不存在，无法添加列 {column}"
            )));
        }
        if Self::has_column(conn, table, column)? {
            return Ok(false);
        }

        let sql = format!("ALTER TABLE \"{table}\" ADD COLUMN \"{column}\" {definition};");
        conn.execute(&sql, [])
            .map_err(|e| AppError::Database(format!("为表 {table} 添加列 {column} 失败: {e}")))?;
        log::info!("已为表 {table} 添加缺失列 {column}");
        Ok(true)
    }
}
