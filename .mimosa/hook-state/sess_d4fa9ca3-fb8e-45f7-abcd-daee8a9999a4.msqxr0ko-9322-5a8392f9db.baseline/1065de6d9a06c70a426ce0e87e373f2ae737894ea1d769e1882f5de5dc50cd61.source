use crate::database::Database;
use crate::error::AppError;
use rusqlite::Connection;

const DEFAULT_MODULES: [(&str, &str, &str, i64, i64); 4] = [
    ("codex", "Codex", "subscription", 0, 0),
    ("claude-code", "Claude Code", "subscription", 1, 0),
    ("kimi-coding-plan", "Kimi Coding Plan", "subscription", 2, 0),
    ("api", "API", "api", 3, 1),
];

pub(crate) fn migrate_v14_to_v15(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS dashboard_modules (
             id TEXT PRIMARY KEY,
             name TEXT NOT NULL CHECK (length(trim(name)) > 0),
             kind TEXT NOT NULL CHECK (kind IN ('subscription','api')),
             sort_order INTEGER NOT NULL,
             visible INTEGER NOT NULL CHECK (visible IN (0,1)),
             is_system INTEGER NOT NULL CHECK (is_system IN (0,1)),
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             CHECK (is_system = 0 OR (id = 'api' AND kind = 'api'))
         );
         CREATE UNIQUE INDEX IF NOT EXISTS idx_dashboard_modules_single_api
             ON dashboard_modules(kind) WHERE kind = 'api';
         CREATE TRIGGER IF NOT EXISTS dashboard_modules_protect_system_delete
         BEFORE DELETE ON dashboard_modules
         WHEN OLD.is_system = 1
         BEGIN
             SELECT RAISE(ABORT, 'system dashboard module cannot be deleted');
         END;
         CREATE TRIGGER IF NOT EXISTS dashboard_modules_protect_system_identity
         BEFORE UPDATE OF id, kind, is_system ON dashboard_modules
         WHEN OLD.is_system = 1 AND (
             NEW.id <> OLD.id OR NEW.kind <> OLD.kind OR NEW.is_system <> OLD.is_system
         )
         BEGIN
             SELECT RAISE(ABORT, 'system dashboard module identity cannot be changed');
         END;",
    )
    .map_err(|error| AppError::Database(format!("create dashboard module schema: {error}")))?;

    if !Database::has_column(conn, "usage_providers", "dashboard_module_id")? {
        conn.execute(
            "ALTER TABLE usage_providers
             ADD COLUMN dashboard_module_id TEXT REFERENCES dashboard_modules(id)",
            [],
        )
        .map_err(|error| {
            AppError::Database(format!("add usage provider dashboard module: {error}"))
        })?;
    }

    let now = unix_timestamp()?;
    for (id, name, kind, sort_order, is_system) in DEFAULT_MODULES {
        conn.execute(
            "INSERT INTO dashboard_modules (
                 id, name, kind, sort_order, visible, is_system, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?6)
             ON CONFLICT(id) DO NOTHING",
            rusqlite::params![id, name, kind, sort_order, is_system, now],
        )
        .map_err(|error| AppError::Database(format!("seed dashboard module {id}: {error}")))?;
    }

    let unknown_subscriptions: bool = conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM usage_providers
             WHERE billing_kind = 'subscription'
               AND lower(trim(product_group_id)) NOT IN (
                   'codex', 'openai', 'claude', 'claude-code', 'anthropic',
                   'kimi', 'kimi-coding-plan', 'moonshot'
               )
         )",
        [],
        |row| row.get(0),
    )?;
    if unknown_subscriptions {
        conn.execute(
            "INSERT INTO dashboard_modules (
                 id, name, kind, sort_order, visible, is_system, created_at, updated_at
             ) VALUES ('other-subscriptions', '其他订阅', 'subscription', 3, 1, 0, ?1, ?1)
             ON CONFLICT(id) DO NOTHING",
            [now],
        )?;
        conn.execute(
            "UPDATE dashboard_modules SET sort_order = 4, updated_at = ?1 WHERE id = 'api'",
            [now],
        )?;
    }

    conn.execute(
        "UPDATE usage_providers
         SET dashboard_module_id = CASE lower(trim(product_group_id))
             WHEN 'codex' THEN 'codex'
             WHEN 'openai' THEN 'codex'
             WHEN 'claude' THEN 'claude-code'
             WHEN 'claude-code' THEN 'claude-code'
             WHEN 'anthropic' THEN 'claude-code'
             WHEN 'kimi' THEN 'kimi-coding-plan'
             WHEN 'kimi-coding-plan' THEN 'kimi-coding-plan'
             WHEN 'moonshot' THEN 'kimi-coding-plan'
             ELSE 'other-subscriptions'
         END,
         needs_review = CASE
             WHEN lower(trim(product_group_id)) IN (
                 'codex', 'openai', 'claude', 'claude-code', 'anthropic',
                 'kimi', 'kimi-coding-plan', 'moonshot'
             ) THEN needs_review
             ELSE 1
         END,
         updated_at = ?1
         WHERE billing_kind = 'subscription'",
        [now],
    )
    .map_err(|error| AppError::Database(format!("backfill dashboard modules: {error}")))?;

    conn.execute(
        "UPDATE usage_providers
         SET dashboard_module_id = NULL
         WHERE billing_kind = 'metered' AND dashboard_module_id IS NOT NULL",
        [],
    )?;
    Ok(())
}

fn unix_timestamp() -> Result<i64, AppError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|error| AppError::Database(format!("system clock before unix epoch: {error}")))
}
