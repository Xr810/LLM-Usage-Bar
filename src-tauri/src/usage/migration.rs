use crate::database::Database;
use crate::error::AppError;
use crate::usage::domain::{BillingKind, TokenSource};
use rusqlite::{params, Connection};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyProviderClassification {
    pub billing_kind: BillingKind,
    pub token_sources: Vec<TokenSource>,
    pub needs_review: bool,
}

pub fn classify_legacy_provider(
    meta: &Value,
    category: Option<&str>,
) -> LegacyProviderClassification {
    let provider_type = meta
        .get("providerType")
        .or_else(|| meta.get("provider_type"))
        .and_then(Value::as_str);
    let usage_script = meta.get("usage_script").or_else(|| meta.get("usageScript"));
    let enabled_template = usage_script
        .filter(|script| {
            script
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .and_then(|script| {
            script
                .get("templateType")
                .or_else(|| script.get("template_type"))
                .and_then(Value::as_str)
        });

    let proven_subscription = matches!(provider_type, Some("codex_oauth" | "github_copilot"))
        || matches!(
            enabled_template,
            Some("official_subscription" | "token_plan")
        );

    if proven_subscription {
        LegacyProviderClassification {
            billing_kind: BillingKind::Subscription,
            token_sources: vec![TokenSource::SessionLog],
            needs_review: false,
        }
    } else {
        LegacyProviderClassification {
            billing_kind: BillingKind::Metered,
            token_sources: vec![TokenSource::Proxy],
            needs_review: category == Some("official"),
        }
    }
}

struct LegacyProviderRow {
    id: String,
    app_type: String,
    name: String,
    settings_config: String,
    category: Option<String>,
    meta: String,
    created_at: Option<i64>,
}

pub(crate) fn migrate_v12_to_v13(conn: &Connection) -> Result<(), AppError> {
    create_v13_schema(conn)?;
    import_legacy_providers(conn)?;
    import_legacy_proxy_logs(conn)?;
    Ok(())
}

fn create_v13_schema(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS usage_providers (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            billing_kind TEXT NOT NULL CHECK (billing_kind IN ('subscription','metered')),
            product_group_id TEXT NOT NULL,
            token_sources TEXT NOT NULL,
            quota_source TEXT,
            quota_interval_seconds INTEGER,
            route_app_type TEXT,
            route_config TEXT,
            quota_config TEXT,
            enabled INTEGER NOT NULL DEFAULT 1,
            needs_review INTEGER NOT NULL DEFAULT 0,
            legacy_app_type TEXT,
            legacy_provider_id TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            UNIQUE (legacy_app_type, legacy_provider_id)
        );

        CREATE TABLE IF NOT EXISTS route_bindings (
            protocol TEXT PRIMARY KEY CHECK (protocol IN ('claude','codex','gemini')),
            provider_id TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            FOREIGN KEY (provider_id) REFERENCES usage_providers(id)
        );

        CREATE TABLE IF NOT EXISTS usage_source_bindings (
            source TEXT PRIMARY KEY CHECK (source IN ('claude','codex')),
            provider_id TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            FOREIGN KEY (provider_id) REFERENCES usage_providers(id)
        );

        CREATE TABLE IF NOT EXISTS usage_events (
            event_id TEXT PRIMARY KEY,
            source TEXT NOT NULL CHECK (source IN ('proxy','session_log')),
            provider_id TEXT NOT NULL,
            product_group_id TEXT NOT NULL,
            occurred_at INTEGER NOT NULL,
            model TEXT NOT NULL,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            request_id TEXT,
            session_id TEXT,
            upstream_correlation_id TEXT,
            input_cost_usd TEXT,
            output_cost_usd TEXT,
            cache_read_cost_usd TEXT,
            cache_creation_cost_usd TEXT,
            total_cost_usd TEXT,
            cost_source TEXT NOT NULL CHECK (cost_source IN ('upstream','estimated','unavailable')),
            legacy_request_id TEXT UNIQUE,
            created_at INTEGER NOT NULL,
            FOREIGN KEY (provider_id) REFERENCES usage_providers(id)
        );

        CREATE TABLE IF NOT EXISTS usage_event_links (
            canonical_event_id TEXT NOT NULL,
            duplicate_event_id TEXT NOT NULL,
            link_kind TEXT NOT NULL CHECK (
                link_kind IN ('request_id','session_id','upstream_correlation_id')
            ),
            link_value TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY (canonical_event_id, duplicate_event_id, link_kind, link_value),
            CHECK (canonical_event_id <> duplicate_event_id),
            FOREIGN KEY (canonical_event_id) REFERENCES usage_events(event_id),
            FOREIGN KEY (duplicate_event_id) REFERENCES usage_events(event_id)
        );

        CREATE TABLE IF NOT EXISTS quota_snapshots (
            snapshot_id TEXT PRIMARY KEY,
            provider_id TEXT NOT NULL,
            fetched_at INTEGER NOT NULL,
            five_hour_utilization_percent TEXT,
            five_hour_resets_at TEXT,
            seven_day_utilization_percent TEXT,
            seven_day_resets_at TEXT,
            manual_resets_remaining INTEGER,
            raw_payload TEXT NOT NULL DEFAULT '{}',
            created_at INTEGER NOT NULL,
            FOREIGN KEY (provider_id) REFERENCES usage_providers(id)
        );

        CREATE TABLE IF NOT EXISTS quota_fetch_state (
            provider_id TEXT PRIMARY KEY,
            last_attempt_at INTEGER,
            last_success_at INTEGER,
            last_error TEXT,
            stale INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (provider_id) REFERENCES usage_providers(id)
        );

        CREATE INDEX IF NOT EXISTS idx_usage_events_provider_occurred_at
            ON usage_events(provider_id, occurred_at DESC);
        CREATE INDEX IF NOT EXISTS idx_usage_events_product_group_occurred_at
            ON usage_events(product_group_id, occurred_at DESC);
        CREATE INDEX IF NOT EXISTS idx_usage_events_request_id
            ON usage_events(request_id) WHERE request_id IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_usage_events_session_id
            ON usage_events(session_id) WHERE session_id IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_usage_events_upstream_correlation_id
            ON usage_events(upstream_correlation_id)
            WHERE upstream_correlation_id IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_quota_snapshots_provider_fetched_at
            ON quota_snapshots(provider_id, fetched_at DESC);

        CREATE TRIGGER IF NOT EXISTS usage_events_immutable_update
        BEFORE UPDATE ON usage_events
        BEGIN
            SELECT RAISE(ABORT, 'usage_events are immutable');
        END;

        CREATE TRIGGER IF NOT EXISTS usage_events_immutable_delete
        BEFORE DELETE ON usage_events
        BEGIN
            SELECT RAISE(ABORT, 'usage_events are immutable');
        END;

        CREATE TRIGGER IF NOT EXISTS quota_snapshots_append_only_update
        BEFORE UPDATE ON quota_snapshots
        BEGIN
            SELECT RAISE(ABORT, 'quota_snapshots are append-only');
        END;

        CREATE TRIGGER IF NOT EXISTS quota_snapshots_append_only_delete
        BEFORE DELETE ON quota_snapshots
        BEGIN
            SELECT RAISE(ABORT, 'quota_snapshots are append-only');
        END;",
    )
    .map_err(|e| AppError::Database(format!("v12 -> v13 创建用量 schema 失败: {e}")))?;

    Ok(())
}

fn import_legacy_providers(conn: &Connection) -> Result<(), AppError> {
    if !has_legacy_columns(
        conn,
        "providers",
        &[
            "id",
            "app_type",
            "name",
            "settings_config",
            "category",
            "meta",
            "created_at",
        ],
    )? {
        return Ok(());
    }

    let providers = {
        let mut statement = conn
            .prepare(
                "SELECT id, app_type, name, settings_config, category, meta, created_at
                 FROM providers",
            )
            .map_err(|e| AppError::Database(format!("v12 -> v13 读取旧 Provider 失败: {e}")))?;

        let rows = statement
            .query_map([], |row| {
                Ok(LegacyProviderRow {
                    id: row.get(0)?,
                    app_type: row.get(1)?,
                    name: row.get(2)?,
                    settings_config: row.get(3)?,
                    category: row.get(4)?,
                    meta: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })
            .map_err(|e| AppError::Database(format!("v12 -> v13 查询旧 Provider 失败: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(format!("v12 -> v13 收集旧 Provider 失败: {e}")))?;
        rows
    };

    let now = unix_timestamp()?;
    for provider in providers {
        let meta = serde_json::from_str::<Value>(&provider.meta).unwrap_or(Value::Null);
        let classification = classify_legacy_provider(&meta, provider.category.as_deref());
        let global_id = format!("{}:{}", provider.app_type, provider.id);
        let billing_kind = match classification.billing_kind {
            BillingKind::Subscription => "subscription",
            BillingKind::Metered => "metered",
        };
        let token_sources = serde_json::to_string(&classification.token_sources).map_err(|e| {
            AppError::Database(format!("v12 -> v13 序列化 token sources 失败: {e}"))
        })?;

        conn.execute(
            "INSERT INTO usage_providers (
                id, name, billing_kind, product_group_id, token_sources,
                route_app_type, route_config, quota_config, enabled, needs_review,
                legacy_app_type, legacy_provider_id, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(legacy_app_type, legacy_provider_id) DO NOTHING",
            params![
                global_id,
                provider.name,
                billing_kind,
                provider.app_type,
                token_sources,
                provider.app_type,
                provider.settings_config,
                provider.meta,
                classification.needs_review,
                provider.app_type,
                provider.id,
                provider.created_at.unwrap_or(now),
                now,
            ],
        )
        .map_err(|e| AppError::Database(format!("v12 -> v13 导入旧 Provider 失败: {e}")))?;
    }

    Ok(())
}

fn import_legacy_proxy_logs(conn: &Connection) -> Result<(), AppError> {
    if !has_legacy_columns(
        conn,
        "proxy_request_logs",
        &[
            "request_id",
            "provider_id",
            "app_type",
            "model",
            "input_tokens",
            "output_tokens",
            "cache_read_tokens",
            "cache_creation_tokens",
            "session_id",
            "input_cost_usd",
            "output_cost_usd",
            "cache_read_cost_usd",
            "cache_creation_cost_usd",
            "total_cost_usd",
            "created_at",
            "data_source",
        ],
    )? {
        return Ok(());
    }

    let now = unix_timestamp()?;
    conn.execute(
        "INSERT INTO usage_events (
            event_id, source, provider_id, product_group_id, occurred_at, model,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
            request_id, session_id, upstream_correlation_id,
            input_cost_usd, output_cost_usd, cache_read_cost_usd,
            cache_creation_cost_usd, total_cost_usd, cost_source,
            legacy_request_id, created_at
         )
         SELECT
            'legacy:' || logs.request_id,
            'proxy',
            usage_providers.id,
            logs.app_type,
            logs.created_at,
            logs.model,
            logs.input_tokens,
            logs.output_tokens,
            logs.cache_read_tokens,
            logs.cache_creation_tokens,
            logs.request_id,
            logs.session_id,
            NULL,
            logs.input_cost_usd,
            logs.output_cost_usd,
            logs.cache_read_cost_usd,
            logs.cache_creation_cost_usd,
            logs.total_cost_usd,
            'estimated',
            logs.request_id,
            ?1
         FROM proxy_request_logs AS logs
         JOIN usage_providers
           ON usage_providers.legacy_app_type = logs.app_type
          AND usage_providers.legacy_provider_id = logs.provider_id
         WHERE (logs.data_source IS NULL OR logs.data_source = 'proxy')
           AND substr(logs.provider_id, 1, 1) <> '_'
         ON CONFLICT(legacy_request_id) DO NOTHING",
        params![now],
    )
    .map_err(|e| AppError::Database(format!("v12 -> v13 导入旧代理日志失败: {e}")))?;
    Ok(())
}

fn unix_timestamp() -> Result<i64, AppError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|e| AppError::Database(format!("系统时间早于 Unix epoch: {e}")))
}

fn has_legacy_columns(
    conn: &Connection,
    table: &str,
    required_columns: &[&str],
) -> Result<bool, AppError> {
    if !Database::table_exists(conn, table)? {
        return Ok(false);
    }
    for column in required_columns {
        if !Database::has_column(conn, table, column)? {
            return Ok(false);
        }
    }
    Ok(true)
}
