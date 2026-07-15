use super::domain::{BillingKind, SystemProviderAuthKind};
use super::system_providers::{
    system_provider_definitions, ANTHROPIC_API_ID, CHATGPT_SUBSCRIPTION_ID, CLAUDE_SUBSCRIPTION_ID,
    OPENAI_API_ID, OPENROUTER_API_ID,
};
use crate::database::{to_json_string, Database};
use crate::error::AppError;
use rusqlite::{params, Connection, OptionalExtension};
use std::time::{SystemTime, UNIX_EPOCH};

const SYSTEM_KEY_INDEX: &str = "idx_usage_providers_system_preset_key";
const PROVIDER_FINGERPRINT_INDEX: &str = "idx_provider_api_credentials_fingerprint";
const PROVIDER_SLOT_INDEX: &str = "idx_provider_api_credentials_credential_slot";
const PROVIDER_PENDING_INDEX: &str = "idx_provider_credential_operations_pending_provider";

fn now_timestamp() -> Result<i64, AppError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|error| AppError::Database(format!("system clock before unix epoch: {error}")))
}

fn billing_kind_value(kind: BillingKind) -> &'static str {
    match kind {
        BillingKind::Subscription => "subscription",
        BillingKind::Metered => "metered",
    }
}

pub(crate) fn migrate_v16_to_v17(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch(
        "ALTER TABLE usage_providers ADD COLUMN system_preset_key TEXT;
         ALTER TABLE agent_provider_bindings ADD COLUMN route_protocol TEXT;

         UPDATE agent_provider_bindings
         SET route_protocol = (
             SELECT route_app_type FROM usage_providers
             WHERE usage_providers.id = agent_provider_bindings.provider_id
         );

         CREATE UNIQUE INDEX idx_usage_providers_system_preset_key
             ON usage_providers(system_preset_key)
             WHERE system_preset_key IS NOT NULL;

         CREATE TRIGGER usage_providers_system_identity_immutable
         BEFORE UPDATE OF id, system_preset_key ON usage_providers
         WHEN OLD.system_preset_key IS NOT NULL AND (
             NEW.id IS NOT OLD.id
             OR NEW.system_preset_key IS NOT OLD.system_preset_key
         )
         BEGIN
             SELECT RAISE(ABORT, 'system provider identity cannot be changed');
         END;

         CREATE TRIGGER usage_providers_system_delete
         BEFORE DELETE ON usage_providers
         WHEN OLD.system_preset_key IS NOT NULL
         BEGIN
             SELECT RAISE(ABORT, 'system provider cannot be deleted');
         END;

         CREATE TABLE provider_api_credentials (
             provider_id TEXT NOT NULL PRIMARY KEY
                 REFERENCES usage_providers(id) ON DELETE RESTRICT,
             api_key_fingerprint BLOB,
             credential_slot TEXT,
             credential_version INTEGER NOT NULL DEFAULT 0 CHECK (credential_version >= 0),
             last_test_at INTEGER,
             last_test_status TEXT CHECK (
                 last_test_status IS NULL OR last_test_status IN ('success','failed')
             ),
             last_test_error_code TEXT,
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
         CREATE UNIQUE INDEX idx_provider_api_credentials_fingerprint
             ON provider_api_credentials(api_key_fingerprint)
             WHERE api_key_fingerprint IS NOT NULL;
         CREATE UNIQUE INDEX idx_provider_api_credentials_credential_slot
             ON provider_api_credentials(credential_slot)
             WHERE credential_slot IS NOT NULL;

         CREATE TABLE provider_credential_operations (
             operation_id TEXT NOT NULL PRIMARY KEY CHECK (length(trim(operation_id)) > 0),
             provider_id TEXT NOT NULL,
             generation INTEGER NOT NULL CHECK (generation > 0),
             operation_kind TEXT NOT NULL
                 CHECK (operation_kind IN ('set','replace','clear')),
             status TEXT NOT NULL CHECK (status IN ('pending','committed','cleanup')),
             staging_slot TEXT,
             previous_slot TEXT,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             FOREIGN KEY (provider_id) REFERENCES provider_api_credentials(provider_id)
                 ON DELETE RESTRICT,
             UNIQUE (provider_id, generation),
             CHECK (staging_slot IS NULL OR length(trim(staging_slot)) > 0),
             CHECK (previous_slot IS NULL OR length(trim(previous_slot)) > 0),
             CHECK (operation_kind = 'clear' OR staging_slot IS NOT NULL)
         );
         CREATE UNIQUE INDEX idx_provider_credential_operations_pending_provider
             ON provider_credential_operations(provider_id)
             WHERE status = 'pending';",
    )
    .map_err(|error| AppError::Database(format!("create system Provider schema v17: {error}")))?;

    reconcile_system_provider_catalog(conn)?;
    seed_default_bindings_once(conn)?;
    validate_schema_v17_complete(conn)
}

pub(crate) fn reconcile_system_provider_catalog(conn: &Connection) -> Result<(), AppError> {
    let now = now_timestamp()?;
    for definition in system_provider_definitions() {
        let existing_preset: Option<Option<String>> = conn
            .query_row(
                "SELECT system_preset_key FROM usage_providers WHERE id = ?1",
                [definition.id],
                |row| row.get(0),
            )
            .optional()?;
        if existing_preset.is_some_and(|preset| preset.as_deref() != Some(definition.preset_key)) {
            return Err(AppError::Database(format!(
                "reserved system Provider id is already used: {}",
                definition.id
            )));
        }
        let existing_id: Option<String> = conn
            .query_row(
                "SELECT id FROM usage_providers WHERE system_preset_key = ?1",
                [definition.preset_key],
                |row| row.get(0),
            )
            .optional()?;
        if existing_id.as_deref().is_some_and(|id| id != definition.id) {
            return Err(AppError::Database(format!(
                "system Provider preset key is already used: {}",
                definition.preset_key
            )));
        }

        let token_sources = to_json_string(&definition.token_sources)?;
        let route_config = definition
            .route_config
            .as_ref()
            .map(to_json_string)
            .transpose()?;
        conn.execute(
            "INSERT INTO usage_providers (
                 id, name, billing_kind, product_group_id, token_sources,
                 quota_source, quota_interval_seconds, route_app_type, route_config,
                 quota_config, enabled, needs_review, legacy_app_type,
                 legacy_provider_id, created_at, updated_at, system_preset_key
             ) VALUES (
                 ?1, ?2, ?3, ?4, ?5,
                 NULL, NULL, ?6, ?7,
                 NULL, 1, 0, NULL,
                 NULL, ?8, ?8, ?9
             )
             ON CONFLICT(id) DO UPDATE SET
                 name = excluded.name,
                 billing_kind = excluded.billing_kind,
                 product_group_id = excluded.product_group_id,
                 token_sources = excluded.token_sources,
                 quota_source = NULL,
                 quota_interval_seconds = NULL,
                 route_app_type = excluded.route_app_type,
                 route_config = excluded.route_config,
                 quota_config = NULL,
                 needs_review = 0,
                 system_preset_key = excluded.system_preset_key",
            params![
                definition.id,
                definition.name,
                billing_kind_value(definition.billing_kind),
                definition.product_group_id,
                token_sources,
                definition.upstream_protocol,
                route_config,
                now,
                definition.preset_key,
            ],
        )?;

        if definition.auth_kind == SystemProviderAuthKind::ProviderApiKey {
            conn.execute(
                "INSERT OR IGNORE INTO provider_api_credentials (
                     provider_id, api_key_fingerprint, credential_slot,
                     credential_version, last_test_at, last_test_status,
                     last_test_error_code, created_at, updated_at
                 ) VALUES (?1, NULL, NULL, 0, NULL, NULL, NULL, ?2, ?2)",
                params![definition.id, now],
            )?;
        }
    }
    Ok(())
}

fn seed_default_bindings_once(conn: &Connection) -> Result<(), AppError> {
    let seeded: bool = conn.query_row(
        "SELECT EXISTS(
                 SELECT 1 FROM settings
                 WHERE key = 'system_provider_default_bindings_v1_seeded'
                   AND value = 'true'
             )",
        [],
        |row| row.get(0),
    )?;
    if seeded {
        return Ok(());
    }

    let now = now_timestamp()?;
    for (agent, provider, protocol) in [
        ("codex", CHATGPT_SUBSCRIPTION_ID, Some("codex")),
        ("claude-code", CLAUDE_SUBSCRIPTION_ID, None),
        ("opencode", OPENROUTER_API_ID, Some("opencode")),
        ("openclaw", OPENROUTER_API_ID, Some("openclaw")),
        ("hermes", OPENROUTER_API_ID, Some("hermes")),
    ] {
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO agent_provider_bindings (
                 id, agent_module_id, provider_id, enabled,
                 api_key_fingerprint, credential_slot, credential_version,
                 created_at, updated_at, route_protocol
             ) VALUES (?1, ?2, ?3, 1, NULL, NULL, 0, ?4, ?4, ?5)",
            params![id, agent, provider, now, protocol],
        )?;
        conn.execute(
            "UPDATE agent_modules SET ever_bound = 1 WHERE id = ?1",
            [agent],
        )?;
    }

    conn.execute(
        "INSERT OR IGNORE INTO usage_source_bindings (source, provider_id, updated_at)
         VALUES ('codex', ?1, ?3), ('claude', ?2, ?3)",
        params![CHATGPT_SUBSCRIPTION_ID, CLAUDE_SUBSCRIPTION_ID, now],
    )?;
    conn.execute(
        "INSERT INTO settings (key, value)
         VALUES ('system_provider_default_bindings_v1_seeded', 'true')
         ON CONFLICT(key) DO UPDATE SET value = 'true'",
        [],
    )?;
    Ok(())
}

pub(crate) fn validate_schema_v17_complete(conn: &Connection) -> Result<(), AppError> {
    require_column_shape(
        conn,
        "usage_providers",
        "system_preset_key",
        "TEXT",
        0,
        None,
        0,
    )?;
    require_column_shape(
        conn,
        "agent_provider_bindings",
        "route_protocol",
        "TEXT",
        0,
        None,
        0,
    )?;
    require_table_shape(
        conn,
        "provider_api_credentials",
        &[
            ("provider_id", "TEXT", 1, None, 1),
            ("api_key_fingerprint", "BLOB", 0, None, 0),
            ("credential_slot", "TEXT", 0, None, 0),
            ("credential_version", "INTEGER", 1, Some("0"), 0),
            ("last_test_at", "INTEGER", 0, None, 0),
            ("last_test_status", "TEXT", 0, None, 0),
            ("last_test_error_code", "TEXT", 0, None, 0),
            ("created_at", "INTEGER", 1, None, 0),
            ("updated_at", "INTEGER", 1, None, 0),
        ],
    )?;
    require_table_shape(
        conn,
        "provider_credential_operations",
        &[
            ("operation_id", "TEXT", 1, None, 1),
            ("provider_id", "TEXT", 1, None, 0),
            ("generation", "INTEGER", 1, None, 0),
            ("operation_kind", "TEXT", 1, None, 0),
            ("status", "TEXT", 1, None, 0),
            ("staging_slot", "TEXT", 0, None, 0),
            ("previous_slot", "TEXT", 0, None, 0),
            ("created_at", "INTEGER", 1, None, 0),
            ("updated_at", "INTEGER", 1, None, 0),
        ],
    )?;
    require_table_sql_fragments(
        conn,
        "provider_api_credentials",
        &[
            "check (credential_version >= 0)",
            "last_test_status is null or last_test_status in ('success','failed')",
            "(api_key_fingerprint is null and credential_slot is null)",
            "typeof(api_key_fingerprint) = 'blob'",
            "length(api_key_fingerprint) = 32",
            "length(trim(credential_slot)) > 0",
            "credential_version > 0",
        ],
    )?;
    require_table_sql_fragments(
        conn,
        "provider_credential_operations",
        &[
            "check (length(trim(operation_id)) > 0)",
            "check (generation > 0)",
            "check (operation_kind in ('set','replace','clear'))",
            "check (status in ('pending','committed','cleanup'))",
            "unique (provider_id, generation)",
            "check (operation_kind = 'clear' or staging_slot is not null)",
        ],
    )?;
    require_foreign_key(
        conn,
        "provider_api_credentials",
        "provider_id",
        "usage_providers",
        "id",
        "RESTRICT",
    )?;
    require_foreign_key(
        conn,
        "provider_credential_operations",
        "provider_id",
        "provider_api_credentials",
        "provider_id",
        "RESTRICT",
    )?;

    require_index_shape(
        conn,
        "usage_providers",
        SYSTEM_KEY_INDEX,
        &["system_preset_key"],
        true,
        true,
        Some("where system_preset_key is not null"),
    )?;
    require_index_shape(
        conn,
        "provider_api_credentials",
        PROVIDER_FINGERPRINT_INDEX,
        &["api_key_fingerprint"],
        true,
        true,
        Some("where api_key_fingerprint is not null"),
    )?;
    require_index_shape(
        conn,
        "provider_api_credentials",
        PROVIDER_SLOT_INDEX,
        &["credential_slot"],
        true,
        true,
        Some("where credential_slot is not null"),
    )?;
    require_index_shape(
        conn,
        "provider_credential_operations",
        PROVIDER_PENDING_INDEX,
        &["provider_id"],
        true,
        true,
        Some("where status = 'pending'"),
    )?;
    require_exact_trigger(
        conn,
        "usage_providers_system_identity_immutable",
        "usage_providers",
        "CREATE TRIGGER usage_providers_system_identity_immutable
         BEFORE UPDATE OF id, system_preset_key ON usage_providers
         WHEN OLD.system_preset_key IS NOT NULL AND (
             NEW.id IS NOT OLD.id
             OR NEW.system_preset_key IS NOT OLD.system_preset_key
         )
         BEGIN
             SELECT RAISE(ABORT, 'system provider identity cannot be changed');
         END",
    )?;
    require_exact_trigger(
        conn,
        "usage_providers_system_delete",
        "usage_providers",
        "CREATE TRIGGER usage_providers_system_delete
         BEFORE DELETE ON usage_providers
         WHEN OLD.system_preset_key IS NOT NULL
         BEGIN
             SELECT RAISE(ABORT, 'system provider cannot be deleted');
         END",
    )?;

    let actual_system_rows = conn.query_row(
        "SELECT COUNT(*) FROM usage_providers WHERE system_preset_key IS NOT NULL",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if actual_system_rows != 5 {
        return Err(incomplete(format!(
            "expected five system Provider rows, found {actual_system_rows}"
        )));
    }
    for definition in system_provider_definitions() {
        let found: bool = conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM usage_providers
                 WHERE id = ?1 AND system_preset_key = ?2
             )",
            params![definition.id, definition.preset_key],
            |row| row.get(0),
        )?;
        if !found {
            return Err(incomplete(format!(
                "invalid system Provider identity: {}",
                definition.id
            )));
        }
    }
    let api_credential_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM provider_api_credentials
         WHERE provider_id IN (?1, ?2, ?3)",
        params![OPENAI_API_ID, ANTHROPIC_API_ID, OPENROUTER_API_ID],
        |row| row.get(0),
    )?;
    if api_credential_rows != 3 {
        return Err(incomplete("fixed API Provider credential rows are missing"));
    }
    let invalid_provider_credential: Option<String> = conn
        .query_row(
            "SELECT provider_id FROM provider_api_credentials
             WHERE provider_id NOT IN (?1, ?2, ?3) LIMIT 1",
            params![OPENAI_API_ID, ANTHROPIC_API_ID, OPENROUTER_API_ID],
            |row| row.get(0),
        )
        .optional()?;
    if invalid_provider_credential.is_some() {
        return Err(incomplete("unexpected Provider credential owner"));
    }

    let foreign_key_violation: Option<String> = conn
        .query_row(
            "SELECT \"table\" FROM pragma_foreign_key_check LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(table) = foreign_key_violation {
        return Err(incomplete(format!("foreign key violation in {table}")));
    }
    Ok(())
}

fn incomplete(detail: impl Into<String>) -> AppError {
    AppError::Database(format!("incomplete schema v17: {}", detail.into()))
}

fn normalize_schema_sql(sql: &str) -> String {
    sql.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn require_table_shape(
    conn: &Connection,
    table: &str,
    expected: &[(&str, &str, i64, Option<&str>, i64)],
) -> Result<(), AppError> {
    if !Database::table_exists(conn, table)? {
        return Err(incomplete(format!("missing {table}")));
    }
    let mut statement = conn.prepare(
        "SELECT name, upper(type), \"notnull\", dflt_value, pk
         FROM pragma_table_info(?1) ORDER BY cid",
    )?;
    let actual = statement
        .query_map([table], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let expected = expected
        .iter()
        .map(|(name, data_type, not_null, default_value, primary_key)| {
            (
                (*name).to_string(),
                (*data_type).to_string(),
                *not_null,
                default_value.map(str::to_string),
                *primary_key,
            )
        })
        .collect::<Vec<_>>();
    if actual != expected {
        return Err(incomplete(format!("{table} column shape is invalid")));
    }
    Ok(())
}

fn require_column_shape(
    conn: &Connection,
    table: &str,
    column: &str,
    data_type: &str,
    not_null: i64,
    default_value: Option<&str>,
    primary_key: i64,
) -> Result<(), AppError> {
    let actual: Option<(String, i64, Option<String>, i64)> = conn
        .query_row(
            "SELECT upper(type), \"notnull\", dflt_value, pk
             FROM pragma_table_info(?1) WHERE name = ?2",
            params![table, column],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    if actual
        != Some((
            data_type.to_string(),
            not_null,
            default_value.map(str::to_string),
            primary_key,
        ))
    {
        return Err(incomplete(format!("{table}.{column} shape is invalid")));
    }
    Ok(())
}

fn require_table_sql_fragments(
    conn: &Connection,
    table: &str,
    fragments: &[&str],
) -> Result<(), AppError> {
    let sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get(0),
        )
        .optional()?;
    let Some(sql) = sql else {
        return Err(incomplete(format!("missing table {table}")));
    };
    let sql = normalize_schema_sql(&sql);
    if fragments
        .iter()
        .any(|fragment| !sql.contains(&fragment.to_ascii_lowercase()))
    {
        return Err(incomplete(format!("{table} constraints are invalid")));
    }
    Ok(())
}

fn require_foreign_key(
    conn: &Connection,
    table: &str,
    from: &str,
    target_table: &str,
    target_column: &str,
    on_delete: &str,
) -> Result<(), AppError> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM pragma_foreign_key_list(?1)
             WHERE \"from\" = ?2 AND \"table\" = ?3 AND \"to\" = ?4
               AND upper(on_delete) = ?5
         )",
        params![table, from, target_table, target_column, on_delete],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(incomplete(format!("{table}.{from} foreign key is invalid")));
    }
    Ok(())
}

struct IndexShape {
    unique: bool,
    partial: bool,
    columns: Vec<String>,
    sql: Option<String>,
}

fn named_index_shape(
    conn: &Connection,
    table: &str,
    index: &str,
) -> Result<Option<IndexShape>, AppError> {
    let flags: Option<(i64, i64)> = conn
        .query_row(
            "SELECT \"unique\", partial FROM pragma_index_list(?1) WHERE name = ?2",
            params![table, index],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((unique, partial)) = flags else {
        return Ok(None);
    };
    let mut statement = conn.prepare("SELECT name FROM pragma_index_info(?1) ORDER BY seqno")?;
    let columns = statement
        .query_map([index], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let sql = conn
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = 'index' AND name = ?1",
            [index],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    Ok(Some(IndexShape {
        unique: unique != 0,
        partial: partial != 0,
        columns,
        sql,
    }))
}

fn require_index_shape(
    conn: &Connection,
    table: &str,
    index: &str,
    expected_columns: &[&str],
    unique: bool,
    partial: bool,
    predicate: Option<&str>,
) -> Result<(), AppError> {
    let Some(shape) = named_index_shape(conn, table, index)? else {
        return Err(incomplete(format!("missing index {index}")));
    };
    let expected_columns = expected_columns
        .iter()
        .map(|column| (*column).to_string())
        .collect::<Vec<_>>();
    let predicate_matches = predicate.is_none_or(|predicate| {
        shape
            .sql
            .as_deref()
            .map(normalize_schema_sql)
            .and_then(|sql| {
                sql.find(" where ")
                    .map(|offset| sql[(offset + 1)..].to_string())
            })
            .is_some_and(|actual| actual == normalize_schema_sql(predicate))
    });
    if shape.unique != unique
        || shape.partial != partial
        || shape.columns != expected_columns
        || !predicate_matches
    {
        return Err(incomplete(format!("index {index} shape is invalid")));
    }
    Ok(())
}

fn require_exact_trigger(
    conn: &Connection,
    trigger: &str,
    table: &str,
    expected_sql: &str,
) -> Result<(), AppError> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT tbl_name, sql FROM sqlite_schema
             WHERE type = 'trigger' AND name = ?1",
            [trigger],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((actual_table, sql)) = row else {
        return Err(incomplete(format!("missing trigger {trigger}")));
    };
    if actual_table != table || normalize_schema_sql(&sql) != normalize_schema_sql(expected_sql) {
        return Err(incomplete(format!("trigger {trigger} shape is invalid")));
    }
    Ok(())
}
