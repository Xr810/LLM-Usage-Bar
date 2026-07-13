use crate::database::Database;
use crate::error::AppError;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

const FIXED_AGENTS: [(&str, &str, i64); 5] = [
    ("codex", "Codex", 0),
    ("claude-code", "Claude Code", 1),
    ("opencode", "OpenCode", 2),
    ("openclaw", "OpenClaw", 3),
    ("hermes", "Hermes", 4),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailurePoint {
    AfterTriggerDrop,
    DuringHistoryBackfill,
    BeforeTriggerRecreation,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MigrationFailurePoint {
    AfterTriggerDrop,
    DuringHistoryBackfill,
    BeforeTriggerRecreation,
}

#[cfg(test)]
pub(crate) fn migrate_v15_to_v16_with_failure(
    conn: &Connection,
    point: MigrationFailurePoint,
) -> Result<(), AppError> {
    let point = match point {
        MigrationFailurePoint::AfterTriggerDrop => FailurePoint::AfterTriggerDrop,
        MigrationFailurePoint::DuringHistoryBackfill => FailurePoint::DuringHistoryBackfill,
        MigrationFailurePoint::BeforeTriggerRecreation => FailurePoint::BeforeTriggerRecreation,
    };
    migrate_v15_to_v16_inner(conn, Some(point))
}

pub(crate) fn migrate_v15_to_v16(conn: &Connection) -> Result<(), AppError> {
    migrate_v15_to_v16_inner(conn, None)
}

fn migrate_v15_to_v16_inner(
    conn: &Connection,
    failure: Option<FailurePoint>,
) -> Result<(), AppError> {
    create_agent_schema(conn)?;
    seed_agent_modules(conn)?;
    backfill_agent_bindings(conn)?;

    let immutable_update_trigger: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_schema
         WHERE type = 'trigger' AND name = 'usage_events_immutable_update'",
        [],
        |row| row.get(0),
    )?;
    if immutable_update_trigger != 1 {
        return Err(AppError::Database(
            "v15 usage_events immutable UPDATE trigger is missing".to_string(),
        ));
    }
    conn.execute_batch("DROP TRIGGER usage_events_immutable_update;")?;
    fail_if(failure, FailurePoint::AfterTriggerDrop)?;

    conn.execute_batch(
        "ALTER TABLE usage_events
             ADD COLUMN agent_module_id TEXT REFERENCES agent_modules(id) ON DELETE RESTRICT;",
    )
    .map_err(|error| AppError::Database(format!("add event Agent ownership: {error}")))?;

    if failure == Some(FailurePoint::DuringHistoryBackfill) {
        conn.execute(
            "UPDATE usage_events
             SET agent_module_id = 'claude-code'
             WHERE source = 'session_log'
               AND substr(event_id, 1, 15) = 'claude-session:'
               AND length(event_id) > 15",
            [],
        )?;
        return Err(injected_failure(FailurePoint::DuringHistoryBackfill));
    }

    backfill_event_agents(conn)?;
    conn.execute_batch(
        "CREATE INDEX idx_usage_events_agent_occurred_at
             ON usage_events(agent_module_id, occurred_at DESC)
             WHERE agent_module_id IS NOT NULL;
         CREATE INDEX idx_usage_event_links_duplicate
             ON usage_event_links(duplicate_event_id);",
    )?;

    fail_if(failure, FailurePoint::BeforeTriggerRecreation)?;
    conn.execute_batch(
        "CREATE TRIGGER usage_events_immutable_update
         BEFORE UPDATE ON usage_events
         BEGIN
             SELECT RAISE(ABORT, 'usage_events are immutable');
         END;",
    )?;
    validate_schema_v16_complete(conn)?;
    verify_usage_events_update_trigger(conn)?;
    Ok(())
}

fn create_agent_schema(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch(
        "CREATE TABLE agent_modules (
             id TEXT NOT NULL PRIMARY KEY
                CHECK (length(trim(id)) > 0 AND id <> 'api'),
             name TEXT NOT NULL CHECK (length(trim(name)) > 0),
             sort_order INTEGER NOT NULL,
             visible INTEGER NOT NULL CHECK (visible IN (0, 1)),
             is_fixed INTEGER NOT NULL CHECK (is_fixed IN (0, 1)),
             archived_at INTEGER,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             CHECK (
                 (id IN ('codex','claude-code','opencode','openclaw','hermes') AND is_fixed = 1)
                 OR
                 (id NOT IN ('codex','claude-code','opencode','openclaw','hermes') AND is_fixed = 0)
             ),
             CHECK (is_fixed = 0 OR archived_at IS NULL)
         );

         CREATE TRIGGER agent_modules_identity_immutable
         BEFORE UPDATE OF id ON agent_modules
         WHEN NEW.id IS NOT OLD.id
         BEGIN
             SELECT RAISE(ABORT, 'agent module identity cannot be changed');
         END;

         CREATE TRIGGER agent_modules_fixed_immutable
         BEFORE UPDATE OF name, is_fixed, archived_at, created_at ON agent_modules
         WHEN OLD.is_fixed = 1 AND (
             NEW.name IS NOT OLD.name
             OR NEW.is_fixed IS NOT OLD.is_fixed
             OR NEW.archived_at IS NOT OLD.archived_at
             OR NEW.created_at IS NOT OLD.created_at
         )
         BEGIN
             SELECT RAISE(ABORT, 'fixed agent module cannot be changed');
         END;

         CREATE TRIGGER agent_modules_fixed_delete
         BEFORE DELETE ON agent_modules
         WHEN OLD.is_fixed = 1
         BEGIN
             SELECT RAISE(ABORT, 'fixed agent module cannot be deleted');
         END;

         CREATE TABLE agent_provider_bindings (
             id TEXT NOT NULL PRIMARY KEY CHECK (length(trim(id)) > 0),
             agent_module_id TEXT NOT NULL,
             provider_id TEXT NOT NULL,
             enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
             api_key_fingerprint BLOB,
             credential_slot TEXT,
             credential_version INTEGER NOT NULL DEFAULT 0 CHECK (credential_version >= 0),
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             FOREIGN KEY (agent_module_id) REFERENCES agent_modules(id) ON DELETE RESTRICT,
             FOREIGN KEY (provider_id) REFERENCES usage_providers(id) ON DELETE RESTRICT,
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
         CREATE UNIQUE INDEX idx_agent_provider_bindings_agent_provider
             ON agent_provider_bindings(agent_module_id, provider_id);
         CREATE UNIQUE INDEX idx_agent_provider_bindings_fingerprint
             ON agent_provider_bindings(api_key_fingerprint)
             WHERE api_key_fingerprint IS NOT NULL;
         CREATE INDEX idx_agent_provider_bindings_provider
             ON agent_provider_bindings(provider_id);

         CREATE TABLE agent_credential_operations (
             operation_id TEXT NOT NULL PRIMARY KEY CHECK (length(trim(operation_id)) > 0),
             binding_id TEXT NOT NULL,
             generation INTEGER NOT NULL CHECK (generation > 0),
             operation_kind TEXT NOT NULL
                CHECK (operation_kind IN ('set','replace','clear','delete')),
             status TEXT NOT NULL CHECK (status IN ('pending','committed','cleanup')),
             staging_slot TEXT,
             previous_slot TEXT,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             FOREIGN KEY (binding_id) REFERENCES agent_provider_bindings(id) ON DELETE RESTRICT,
             UNIQUE (binding_id, generation),
             CHECK (staging_slot IS NULL OR length(trim(staging_slot)) > 0),
             CHECK (previous_slot IS NULL OR length(trim(previous_slot)) > 0),
             CHECK (
                 operation_kind IN ('clear','delete')
                 OR staging_slot IS NOT NULL
             )
         );
         CREATE UNIQUE INDEX idx_agent_credential_operations_pending_binding
             ON agent_credential_operations(binding_id)
             WHERE status = 'pending';",
    )
    .map_err(|error| AppError::Database(format!("create Agent schema v16: {error}")))?;
    Ok(())
}

#[derive(Clone)]
struct LegacyModuleState {
    name: String,
    sort_order: i64,
    visible: i64,
    created_at: i64,
    updated_at: i64,
}

fn legacy_module(conn: &Connection, id: &str) -> Result<Option<LegacyModuleState>, AppError> {
    conn.query_row(
        "SELECT name, sort_order, visible, created_at, updated_at
         FROM dashboard_modules WHERE id = ?1",
        [id],
        |row| {
            Ok(LegacyModuleState {
                name: row.get(0)?,
                sort_order: row.get(1)?,
                visible: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(AppError::from)
}

fn seed_agent_modules(conn: &Connection) -> Result<(), AppError> {
    let reserved_collision: i64 = conn.query_row(
        "SELECT COUNT(*) FROM dashboard_modules
         WHERE id IN ('opencode','openclaw','hermes')",
        [],
        |row| row.get(0),
    )?;
    if reserved_collision != 0 {
        return Err(AppError::Database(
            "legacy custom module collides with a fixed Agent id".to_string(),
        ));
    }

    let now = unix_timestamp()?;
    for (id, canonical_name, canonical_sort) in FIXED_AGENTS {
        let legacy = legacy_module(conn, id)?;
        let (sort_order, visible, created_at, updated_at) = legacy
            .map(|row| (row.sort_order, row.visible, row.created_at, row.updated_at))
            .unwrap_or((canonical_sort, 1, now, now));
        conn.execute(
            "INSERT INTO agent_modules (
                 id, name, sort_order, visible, is_fixed, archived_at, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 1, NULL, ?5, ?6)",
            params![
                id,
                canonical_name,
                sort_order,
                visible,
                created_at,
                updated_at
            ],
        )?;
    }

    let custom_modules = {
        let mut statement = conn.prepare(
            "SELECT id, name, sort_order, visible, created_at, updated_at
             FROM dashboard_modules
             WHERE kind = 'subscription'
               AND id NOT IN (
                   'codex','claude-code','kimi-coding-plan','api','other-subscriptions',
                   'opencode','openclaw','hermes'
               )
             ORDER BY sort_order, id",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    for (id, name, sort_order, visible, created_at, updated_at) in custom_modules {
        conn.execute(
            "INSERT INTO agent_modules (
                 id, name, sort_order, visible, is_fixed, archived_at, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 0, NULL, ?5, ?6)",
            params![id, name, sort_order, visible, created_at, updated_at],
        )?;
    }

    if let Some(kimi) = legacy_module(conn, "kimi-coding-plan")? {
        let provider_uses_kimi: bool = conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM usage_providers
                 WHERE dashboard_module_id = 'kimi-coding-plan'
             )",
            [],
            |row| row.get(0),
        )?;
        let customized =
            kimi.name != "Kimi Coding Plan" || kimi.sort_order != 2 || kimi.visible != 1;
        if provider_uses_kimi || customized {
            conn.execute(
                "INSERT INTO agent_modules (
                     id, name, sort_order, visible, is_fixed, archived_at,
                     created_at, updated_at
                 ) VALUES ('kimi-coding-plan', ?1, ?2, ?3, 0, NULL, ?4, ?5)",
                params![
                    kimi.name,
                    kimi.sort_order,
                    kimi.visible,
                    kimi.created_at,
                    kimi.updated_at
                ],
            )?;
        }
    }
    Ok(())
}

fn backfill_agent_bindings(conn: &Connection) -> Result<(), AppError> {
    let mut pairs = BTreeSet::<(String, String)>::new();
    {
        let mut statement = conn.prepare(
            "SELECT source, provider_id FROM usage_source_bindings
             WHERE source IN ('claude','codex')",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (source, provider_id) = row?;
            let agent_id = if source == "claude" {
                "claude-code"
            } else {
                "codex"
            };
            pairs.insert((agent_id.to_string(), provider_id));
        }
    }
    {
        let mut statement = conn.prepare(
            "SELECT agent.id, provider.id
             FROM usage_providers AS provider
             JOIN agent_modules AS agent
               ON agent.id = provider.dashboard_module_id
             WHERE agent.is_fixed = 0
               AND agent.id <> 'kimi-coding-plan'",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            pairs.insert(row?);
        }
    }

    let now = unix_timestamp()?;
    for (agent_module_id, provider_id) in pairs {
        conn.execute(
            "INSERT INTO agent_provider_bindings (
                 id, agent_module_id, provider_id, enabled,
                 api_key_fingerprint, credential_slot, credential_version,
                 created_at, updated_at
             ) VALUES (?1, ?2, ?3, 0, NULL, NULL, 0, ?4, ?4)",
            params![
                uuid::Uuid::new_v4().to_string(),
                agent_module_id,
                provider_id,
                now
            ],
        )?;
    }
    Ok(())
}

fn backfill_event_agents(conn: &Connection) -> Result<(), AppError> {
    let mut candidates = HashMap::<String, HashSet<String>>::new();
    let mut statement = conn.prepare(
        "SELECT event_id, agent_module_id FROM (
             SELECT event.event_id AS event_id, 'claude-code' AS agent_module_id
             FROM usage_events AS event
             JOIN proxy_request_logs AS log
               ON log.request_id = event.legacy_request_id
             WHERE event.source = 'session_log'
               AND log.app_type = 'claude'
               AND log.provider_type = 'session_log'
               AND log.data_source IN ('session','session_log')
             UNION ALL
             SELECT event.event_id, 'codex'
             FROM usage_events AS event
             JOIN proxy_request_logs AS log
               ON log.request_id = event.legacy_request_id
             WHERE event.source = 'session_log'
               AND log.app_type = 'codex'
               AND log.provider_type = 'codex_session'
               AND log.data_source IN ('session','codex_session')
             UNION ALL
             SELECT event_id, 'claude-code'
             FROM usage_events
             WHERE source = 'session_log'
               AND substr(event_id, 1, 15) = 'claude-session:'
               AND length(event_id) > 15
             UNION ALL
             SELECT event_id, 'codex'
             FROM usage_events
             WHERE source = 'session_log'
               AND substr(event_id, 1, 14) = 'codex-session:'
               AND length(event_id) > 14
             UNION ALL
             SELECT event.event_id,
                    CASE lower(trim(log.app_type))
                        WHEN 'codex' THEN 'codex'
                        WHEN 'claude' THEN 'claude-code'
                        WHEN 'opencode' THEN 'opencode'
                        WHEN 'openclaw' THEN 'openclaw'
                        WHEN 'hermes' THEN 'hermes'
                    END
             FROM usage_events AS event
             JOIN proxy_request_logs AS log
               ON log.request_id = event.legacy_request_id
             WHERE event.source = 'proxy'
               AND log.data_source = 'proxy'
               AND lower(trim(log.app_type)) IN (
                   'codex','claude','opencode','openclaw','hermes'
               )
         ) ORDER BY event_id, agent_module_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (event_id, agent_module_id) = row?;
        candidates
            .entry(event_id)
            .or_default()
            .insert(agent_module_id);
    }
    drop(statement);

    let mut adjacency = HashMap::<String, Vec<String>>::new();
    {
        let mut statement =
            conn.prepare("SELECT canonical_event_id, duplicate_event_id FROM usage_event_links")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (left, right) = row?;
            adjacency
                .entry(left.clone())
                .or_default()
                .push(right.clone());
            adjacency.entry(right).or_default().push(left);
        }
    }

    let mut visited = HashSet::<String>::new();
    let mut assignments = Vec::<(String, String)>::new();
    for event_id in candidates.keys() {
        if visited.contains(event_id) {
            continue;
        }
        let mut queue = VecDeque::from([event_id.clone()]);
        let mut component = Vec::new();
        let mut component_agents = HashSet::<String>::new();
        while let Some(current) = queue.pop_front() {
            if !visited.insert(current.clone()) {
                continue;
            }
            if let Some(event_agents) = candidates.get(&current) {
                component_agents.extend(event_agents.iter().cloned());
            }
            if let Some(neighbors) = adjacency.get(&current) {
                queue.extend(neighbors.iter().cloned());
            }
            component.push(current);
        }
        if component_agents.len() != 1 {
            continue;
        }
        let component_agent = component_agents.into_iter().next().expect("one Agent");
        for event in component {
            let Some(event_agents) = candidates.get(&event) else {
                continue;
            };
            if event_agents.len() == 1 && event_agents.contains(&component_agent) {
                assignments.push((event, component_agent.clone()));
            }
        }
    }

    let mut update =
        conn.prepare("UPDATE usage_events SET agent_module_id = ?2 WHERE event_id = ?1")?;
    for (event_id, agent_module_id) in assignments {
        update.execute(params![event_id, agent_module_id])?;
    }
    Ok(())
}

fn fail_if(failure: Option<FailurePoint>, expected: FailurePoint) -> Result<(), AppError> {
    if failure == Some(expected) {
        return Err(injected_failure(expected));
    }
    Ok(())
}

fn injected_failure(point: FailurePoint) -> AppError {
    AppError::Database(format!("injected v16 migration failure: {point:?}"))
}

fn unix_timestamp() -> Result<i64, AppError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|error| AppError::Database(format!("system clock before unix epoch: {error}")))
}

pub(crate) fn validate_schema_v16_complete(conn: &Connection) -> Result<(), AppError> {
    require_table_shape(
        conn,
        "agent_modules",
        &[
            ("id", "TEXT", 1, None, 1),
            ("name", "TEXT", 1, None, 0),
            ("sort_order", "INTEGER", 1, None, 0),
            ("visible", "INTEGER", 1, None, 0),
            ("is_fixed", "INTEGER", 1, None, 0),
            ("archived_at", "INTEGER", 0, None, 0),
            ("created_at", "INTEGER", 1, None, 0),
            ("updated_at", "INTEGER", 1, None, 0),
        ],
    )?;
    require_table_shape(
        conn,
        "agent_provider_bindings",
        &[
            ("id", "TEXT", 1, None, 1),
            ("agent_module_id", "TEXT", 1, None, 0),
            ("provider_id", "TEXT", 1, None, 0),
            ("enabled", "INTEGER", 1, Some("0"), 0),
            ("api_key_fingerprint", "BLOB", 0, None, 0),
            ("credential_slot", "TEXT", 0, None, 0),
            ("credential_version", "INTEGER", 1, Some("0"), 0),
            ("created_at", "INTEGER", 1, None, 0),
            ("updated_at", "INTEGER", 1, None, 0),
        ],
    )?;
    require_table_shape(
        conn,
        "agent_credential_operations",
        &[
            ("operation_id", "TEXT", 1, None, 1),
            ("binding_id", "TEXT", 1, None, 0),
            ("generation", "INTEGER", 1, None, 0),
            ("operation_kind", "TEXT", 1, None, 0),
            ("status", "TEXT", 1, None, 0),
            ("staging_slot", "TEXT", 0, None, 0),
            ("previous_slot", "TEXT", 0, None, 0),
            ("created_at", "INTEGER", 1, None, 0),
            ("updated_at", "INTEGER", 1, None, 0),
        ],
    )?;
    require_column_shape(conn, "usage_events", "agent_module_id", "TEXT", 0, None, 0)?;

    require_table_sql_fragments(
        conn,
        "agent_modules",
        &[
            "check (length(trim(id)) > 0 and id <> 'api')",
            "check (length(trim(name)) > 0)",
            "check (visible in (0, 1))",
            "check (is_fixed in (0, 1))",
            "check ( (id in ('codex','claude-code','opencode','openclaw','hermes') and is_fixed = 1) or (id not in ('codex','claude-code','opencode','openclaw','hermes') and is_fixed = 0) )",
            "check (is_fixed = 0 or archived_at is null)",
        ],
    )?;
    require_table_sql_fragments(
        conn,
        "agent_provider_bindings",
        &[
            "check (length(trim(id)) > 0)",
            "check (enabled in (0, 1))",
            "check (credential_version >= 0)",
            "check ( (api_key_fingerprint is null and credential_slot is null) or (api_key_fingerprint is not null and typeof(api_key_fingerprint) = 'blob' and length(api_key_fingerprint) = 32 and credential_slot is not null and length(trim(credential_slot)) > 0 and credential_version > 0) )",
        ],
    )?;
    require_table_sql_fragments(
        conn,
        "agent_credential_operations",
        &[
            "check (length(trim(operation_id)) > 0)",
            "check (generation > 0)",
            "check (operation_kind in ('set','replace','clear','delete'))",
            "check (status in ('pending','committed','cleanup'))",
            "unique (binding_id, generation)",
            "check (staging_slot is null or length(trim(staging_slot)) > 0)",
            "check (previous_slot is null or length(trim(previous_slot)) > 0)",
            "check ( operation_kind in ('clear','delete') or staging_slot is not null )",
        ],
    )?;

    let fixed_rows = {
        let mut statement =
            conn.prepare("SELECT id, name FROM agent_modules WHERE is_fixed = 1 ORDER BY id")?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let expected = vec![
        ("claude-code".to_string(), "Claude Code".to_string()),
        ("codex".to_string(), "Codex".to_string()),
        ("hermes".to_string(), "Hermes".to_string()),
        ("openclaw".to_string(), "OpenClaw".to_string()),
        ("opencode".to_string(), "OpenCode".to_string()),
    ];
    if fixed_rows != expected {
        return Err(AppError::Database(format!(
            "incomplete schema v16: fixed Agent set is invalid (found {})",
            fixed_rows.len()
        )));
    }
    let invalid_fixed_or_api: i64 = conn.query_row(
        "SELECT COUNT(*) FROM agent_modules
         WHERE id = 'api' OR (is_fixed = 1 AND archived_at IS NOT NULL)",
        [],
        |row| row.get(0),
    )?;
    if invalid_fixed_or_api != 0 {
        return Err(AppError::Database(
            "incomplete schema v16: invalid API or archived fixed Agent row".to_string(),
        ));
    }

    require_foreign_key(
        conn,
        "agent_provider_bindings",
        "agent_module_id",
        "agent_modules",
        "id",
        "RESTRICT",
    )?;
    require_foreign_key(
        conn,
        "agent_provider_bindings",
        "provider_id",
        "usage_providers",
        "id",
        "RESTRICT",
    )?;
    require_foreign_key(
        conn,
        "agent_credential_operations",
        "binding_id",
        "agent_provider_bindings",
        "id",
        "RESTRICT",
    )?;
    require_foreign_key(
        conn,
        "usage_events",
        "agent_module_id",
        "agent_modules",
        "id",
        "RESTRICT",
    )?;

    require_index_shape(
        conn,
        "agent_provider_bindings",
        "idx_agent_provider_bindings_agent_provider",
        &["agent_module_id", "provider_id"],
        true,
        false,
        None,
    )?;
    require_index_shape(
        conn,
        "agent_provider_bindings",
        "idx_agent_provider_bindings_fingerprint",
        &["api_key_fingerprint"],
        true,
        true,
        Some("where api_key_fingerprint is not null"),
    )?;
    require_index_shape(
        conn,
        "agent_provider_bindings",
        "idx_agent_provider_bindings_provider",
        &["provider_id"],
        false,
        false,
        None,
    )?;
    require_index_shape(
        conn,
        "agent_credential_operations",
        "idx_agent_credential_operations_pending_binding",
        &["binding_id"],
        true,
        true,
        Some("where status = 'pending'"),
    )?;
    require_unique_index(
        conn,
        "agent_credential_operations",
        &["binding_id", "generation"],
    )?;
    require_index_shape(
        conn,
        "usage_events",
        "idx_usage_events_agent_occurred_at",
        &["agent_module_id", "occurred_at"],
        false,
        true,
        Some("where agent_module_id is not null"),
    )?;
    require_index_shape(
        conn,
        "usage_event_links",
        "idx_usage_event_links_duplicate",
        &["duplicate_event_id"],
        false,
        false,
        None,
    )?;

    require_exact_trigger(
        conn,
        "agent_modules_identity_immutable",
        "agent_modules",
        "CREATE TRIGGER agent_modules_identity_immutable
         BEFORE UPDATE OF id ON agent_modules
         WHEN NEW.id IS NOT OLD.id
         BEGIN
             SELECT RAISE(ABORT, 'agent module identity cannot be changed');
         END",
    )?;
    require_exact_trigger(
        conn,
        "agent_modules_fixed_immutable",
        "agent_modules",
        "CREATE TRIGGER agent_modules_fixed_immutable
         BEFORE UPDATE OF name, is_fixed, archived_at, created_at ON agent_modules
         WHEN OLD.is_fixed = 1 AND (
             NEW.name IS NOT OLD.name
             OR NEW.is_fixed IS NOT OLD.is_fixed
             OR NEW.archived_at IS NOT OLD.archived_at
             OR NEW.created_at IS NOT OLD.created_at
         )
         BEGIN
             SELECT RAISE(ABORT, 'fixed agent module cannot be changed');
         END",
    )?;
    require_exact_trigger(
        conn,
        "agent_modules_fixed_delete",
        "agent_modules",
        "CREATE TRIGGER agent_modules_fixed_delete
         BEFORE DELETE ON agent_modules
         WHEN OLD.is_fixed = 1
         BEGIN
             SELECT RAISE(ABORT, 'fixed agent module cannot be deleted');
         END",
    )?;
    require_exact_trigger(
        conn,
        "usage_events_immutable_update",
        "usage_events",
        "CREATE TRIGGER usage_events_immutable_update
         BEFORE UPDATE ON usage_events
         BEGIN
             SELECT RAISE(ABORT, 'usage_events are immutable');
         END",
    )?;
    require_exact_trigger(
        conn,
        "usage_events_immutable_delete",
        "usage_events",
        "CREATE TRIGGER usage_events_immutable_delete
         BEFORE DELETE ON usage_events
         BEGIN
             SELECT RAISE(ABORT, 'usage_events are immutable');
         END",
    )?;

    let foreign_key_violation: Option<String> = conn
        .query_row(
            "SELECT \"table\" FROM pragma_foreign_key_check LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(table) = foreign_key_violation {
        return Err(AppError::Database(format!(
            "incomplete schema v16: foreign key violation in {table}"
        )));
    }
    Ok(())
}

fn incomplete(detail: impl Into<String>) -> AppError {
    AppError::Database(format!("incomplete schema v16: {}", detail.into()))
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

fn require_unique_index(
    conn: &Connection,
    table: &str,
    expected_columns: &[&str],
) -> Result<(), AppError> {
    let mut statement =
        conn.prepare("SELECT name FROM pragma_index_list(?1) WHERE \"unique\" = 1 ORDER BY seq")?;
    let names = statement
        .query_map([table], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let expected_columns = expected_columns
        .iter()
        .map(|column| (*column).to_string())
        .collect::<Vec<_>>();
    for index in names {
        if let Some(shape) = named_index_shape(conn, table, &index)? {
            if !shape.partial && shape.columns == expected_columns {
                return Ok(());
            }
        }
    }
    Err(incomplete(format!(
        "{table} required unique constraint is missing"
    )))
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
    let sql = normalize_schema_sql(&sql);
    if actual_table != table || sql != normalize_schema_sql(expected_sql) {
        return Err(incomplete(format!("trigger {trigger} shape is invalid")));
    }
    Ok(())
}

fn verify_usage_events_update_trigger(conn: &Connection) -> Result<(), AppError> {
    let event_id: Option<String> = conn
        .query_row(
            "SELECT event_id FROM usage_events ORDER BY event_id LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(event_id) = event_id {
        return expect_immutable_update_rejection(conn, &event_id);
    }

    conn.execute("SAVEPOINT v16_immutable_trigger_probe", [])?;
    let probe_id = uuid::Uuid::new_v4().to_string();
    let provider_id = format!("v16-trigger-probe-provider-{probe_id}");
    let event_id = format!("v16-trigger-probe-event-{probe_id}");
    let probe_result = (|| {
        conn.execute(
            "INSERT INTO usage_providers (
                 id, name, billing_kind, product_group_id, token_sources,
                 dashboard_module_id, enabled, needs_review, created_at, updated_at
             ) VALUES (?1, 'v16 trigger probe', 'metered', 'v16-trigger-probe',
                       '[\"proxy\"]', NULL, 0, 0, 0, 0)",
            [&provider_id],
        )?;
        conn.execute(
            "INSERT INTO usage_events (
                 event_id, source, provider_id, product_group_id, occurred_at,
                 model, cost_source, created_at
             ) VALUES (?1, 'proxy', ?2, 'v16-trigger-probe', 0,
                       'v16-trigger-probe', 'unavailable', 0)",
            params![event_id, provider_id],
        )?;
        expect_immutable_update_rejection(conn, &event_id)
    })();

    let cleanup = conn.execute_batch(
        "ROLLBACK TO v16_immutable_trigger_probe;
         RELEASE v16_immutable_trigger_probe;",
    );
    if let Err(error) = cleanup {
        return Err(AppError::Database(format!(
            "v16 immutable trigger probe cleanup failed; connection state is uncertain: {error}"
        )));
    }
    probe_result
}

fn expect_immutable_update_rejection(conn: &Connection, event_id: &str) -> Result<(), AppError> {
    match conn.execute(
        "UPDATE usage_events
         SET agent_module_id = agent_module_id
         WHERE event_id = ?1",
        [event_id],
    ) {
        Err(rusqlite::Error::SqliteFailure(_, Some(message)))
            if message == "usage_events are immutable" =>
        {
            Ok(())
        }
        Ok(_) => Err(incomplete(
            "usage_events immutable UPDATE trigger did not reject an update",
        )),
        Err(error) => Err(incomplete(format!(
            "usage_events immutable UPDATE trigger returned an unexpected error: {error}"
        ))),
    }
}
