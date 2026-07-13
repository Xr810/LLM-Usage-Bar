use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::usage::domain::{DashboardModuleInput, DashboardModuleKind, DashboardModuleView};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_timestamp() -> Result<i64, AppError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|error| AppError::Database(format!("system clock before unix epoch: {error}")))
}

fn module_kind_value(kind: DashboardModuleKind) -> &'static str {
    match kind {
        DashboardModuleKind::Subscription => "subscription",
        DashboardModuleKind::Api => "api",
    }
}

fn module_from_row(row: &Row<'_>) -> rusqlite::Result<DashboardModuleView> {
    let raw_kind = row.get::<_, String>(2)?;
    let kind = match raw_kind.as_str() {
        "subscription" => DashboardModuleKind::Subscription,
        "api" => DashboardModuleKind::Api,
        _ => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Text,
                format!("unknown dashboard module kind: {raw_kind}").into(),
            ));
        }
    };
    let provider_count = row.get::<_, i64>(6)?;
    Ok(DashboardModuleView {
        id: row.get(0)?,
        name: row.get(1)?,
        kind,
        sort_order: row.get(3)?,
        visible: row.get(4)?,
        is_system: row.get(5)?,
        provider_count: u64::try_from(provider_count).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                6,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?,
    })
}

const MODULE_VIEW_QUERY: &str = "SELECT module.id, module.name, module.kind, module.sort_order,
            module.visible, module.is_system,
            CASE module.kind
                WHEN 'api' THEN (
                    SELECT COUNT(*) FROM usage_providers WHERE billing_kind = 'metered'
                )
                ELSE (
                    SELECT COUNT(*) FROM usage_providers
                    WHERE dashboard_module_id = module.id
                )
            END AS provider_count
     FROM dashboard_modules AS module";

fn list_modules_on_conn(conn: &Connection) -> Result<Vec<DashboardModuleView>, AppError> {
    let mut statement = conn.prepare(&format!(
        "{MODULE_VIEW_QUERY} ORDER BY module.sort_order, module.id"
    ))?;
    let modules = statement
        .query_map([], module_from_row)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(AppError::from)?;
    Ok(modules)
}

fn module_on_conn(
    conn: &Connection,
    module_id: &str,
) -> Result<Option<DashboardModuleView>, AppError> {
    conn.query_row(
        &format!("{MODULE_VIEW_QUERY} WHERE module.id = ?1"),
        [module_id],
        module_from_row,
    )
    .optional()
    .map_err(AppError::from)
}

fn validate_input(input: &DashboardModuleInput) -> Result<(), AppError> {
    if input.name.trim().is_empty() {
        return Err(AppError::Message(
            "dashboard module name must not be empty".to_string(),
        ));
    }
    if input.sort_order < 0 {
        return Err(AppError::Message(
            "dashboard module sort order must not be negative".to_string(),
        ));
    }
    Ok(())
}

impl Database {
    pub fn list_dashboard_modules(&self) -> Result<Vec<DashboardModuleView>, AppError> {
        let conn = lock_conn!(self.conn);
        list_modules_on_conn(&conn)
    }

    pub fn save_dashboard_module(
        &self,
        input: &DashboardModuleInput,
    ) -> Result<DashboardModuleView, AppError> {
        validate_input(input)?;
        let now = now_timestamp()?;
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction()?;
        let module_id = if let Some(module_id) = input.id.as_deref() {
            let stored = transaction
                .query_row(
                    "SELECT kind, is_system FROM dashboard_modules WHERE id = ?1",
                    [module_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
                )
                .optional()?
                .ok_or_else(|| AppError::Message("dashboard module not found".to_string()))?;
            if stored.0 != module_kind_value(input.kind) {
                return Err(AppError::Message(
                    "dashboard module kind cannot be changed".to_string(),
                ));
            }
            if stored.1 && module_id != "api" {
                return Err(AppError::Message(
                    "unknown system dashboard module".to_string(),
                ));
            }
            transaction.execute(
                "UPDATE dashboard_modules
                 SET name = ?2, sort_order = ?3, visible = ?4, updated_at = ?5
                 WHERE id = ?1",
                params![
                    module_id,
                    input.name.trim(),
                    input.sort_order,
                    input.visible,
                    now
                ],
            )?;
            module_id.to_string()
        } else {
            if input.kind == DashboardModuleKind::Api {
                return Err(AppError::Message(
                    "the system API dashboard module already exists".to_string(),
                ));
            }
            let module_id = uuid::Uuid::new_v4().to_string();
            transaction.execute(
                "INSERT INTO dashboard_modules (
                     id, name, kind, sort_order, visible, is_system, created_at, updated_at
                 ) VALUES (?1, ?2, 'subscription', ?3, ?4, 0, ?5, ?5)",
                params![
                    module_id,
                    input.name.trim(),
                    input.sort_order,
                    input.visible,
                    now
                ],
            )?;
            module_id
        };
        let saved = module_on_conn(&transaction, &module_id)?
            .ok_or_else(|| AppError::Database("saved dashboard module disappeared".to_string()))?;
        transaction.commit()?;
        Ok(saved)
    }

    pub fn reorder_dashboard_modules(
        &self,
        module_ids: &[String],
    ) -> Result<Vec<DashboardModuleView>, AppError> {
        let now = now_timestamp()?;
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction()?;
        let current = list_modules_on_conn(&transaction)?;
        let current_ids = current
            .iter()
            .map(|module| module.id.as_str())
            .collect::<HashSet<_>>();
        let requested_ids = module_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        if module_ids.len() != current.len()
            || requested_ids.len() != module_ids.len()
            || requested_ids != current_ids
        {
            return Err(AppError::Message(
                "dashboard module order must include every module exactly once".to_string(),
            ));
        }
        for (sort_order, module_id) in module_ids.iter().enumerate() {
            transaction.execute(
                "UPDATE dashboard_modules SET sort_order = ?2, updated_at = ?3 WHERE id = ?1",
                params![module_id, sort_order as i64, now],
            )?;
        }
        let reordered = list_modules_on_conn(&transaction)?;
        transaction.commit()?;
        Ok(reordered)
    }

    pub fn set_dashboard_module_visibility(
        &self,
        module_id: &str,
        visible: bool,
    ) -> Result<DashboardModuleView, AppError> {
        let conn = lock_conn!(self.conn);
        let updated = conn.execute(
            "UPDATE dashboard_modules SET visible = ?2, updated_at = ?3 WHERE id = ?1",
            params![module_id, visible, now_timestamp()?],
        )?;
        if updated == 0 {
            return Err(AppError::Message("dashboard module not found".to_string()));
        }
        module_on_conn(&conn, module_id)?
            .ok_or_else(|| AppError::Database("updated dashboard module disappeared".to_string()))
    }

    pub fn delete_dashboard_module(&self, module_id: &str) -> Result<(), AppError> {
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction()?;
        let is_system = transaction
            .query_row(
                "SELECT is_system FROM dashboard_modules WHERE id = ?1",
                [module_id],
                |row| row.get::<_, bool>(0),
            )
            .optional()?
            .ok_or_else(|| AppError::Message("dashboard module not found".to_string()))?;
        if is_system {
            return Err(AppError::Message(
                "system dashboard module cannot be deleted".to_string(),
            ));
        }
        let provider_count: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM usage_providers WHERE dashboard_module_id = ?1",
            [module_id],
            |row| row.get(0),
        )?;
        if provider_count > 0 {
            return Err(AppError::Message(
                "dashboard module with providers cannot be deleted".to_string(),
            ));
        }
        transaction.execute("DELETE FROM dashboard_modules WHERE id = ?1", [module_id])?;
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::database::Database;
    use crate::usage::domain::{DashboardModuleInput, DashboardModuleKind};

    fn custom_module(name: &str) -> DashboardModuleInput {
        DashboardModuleInput {
            id: None,
            name: name.to_string(),
            kind: DashboardModuleKind::Subscription,
            sort_order: 4,
            visible: true,
        }
    }

    #[test]
    fn defaults_are_listed_in_persisted_order_with_provider_counts() {
        let db = Database::memory().unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO usage_providers (
                 id, name, billing_kind, product_group_id, token_sources,
                 enabled, needs_review, created_at, updated_at, dashboard_module_id
             ) VALUES ('codex-plan', 'Codex plan', 'subscription', 'codex', '[]',
                       1, 0, 1, 1, 'codex')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO usage_providers (
                 id, name, billing_kind, product_group_id, token_sources,
                 enabled, needs_review, created_at, updated_at, dashboard_module_id
             ) VALUES ('openrouter', 'OpenRouter', 'metered', 'openrouter', '[]',
                       1, 0, 1, 1, NULL)",
            [],
        )
        .unwrap();
        drop(conn);

        let modules = db.list_dashboard_modules().unwrap();
        assert_eq!(
            modules
                .iter()
                .map(|module| module.id.as_str())
                .collect::<Vec<_>>(),
            vec!["codex", "claude-code", "kimi-coding-plan", "api"]
        );
        assert_eq!(modules[0].provider_count, 1);
        assert_eq!(modules[3].provider_count, 1);
        assert!(modules[3].is_system);
    }

    #[test]
    fn custom_subscription_module_can_be_created_and_updated() {
        let db = Database::memory().unwrap();
        let created = db.save_dashboard_module(&custom_module("Gemini")).unwrap();
        assert!(!created.id.is_empty());
        assert_eq!(created.name, "Gemini");
        assert_eq!(created.kind, DashboardModuleKind::Subscription);
        assert!(!created.is_system);

        let updated = db
            .save_dashboard_module(&DashboardModuleInput {
                id: Some(created.id.clone()),
                name: "Gemini Advanced".to_string(),
                kind: DashboardModuleKind::Subscription,
                sort_order: 1,
                visible: false,
            })
            .unwrap();
        assert_eq!(updated.id, created.id);
        assert_eq!(updated.name, "Gemini Advanced");
        assert_eq!(updated.sort_order, 1);
        assert!(!updated.visible);
    }

    #[test]
    fn api_module_identity_is_protected_and_second_api_is_rejected() {
        let db = Database::memory().unwrap();
        let error = db
            .save_dashboard_module(&DashboardModuleInput {
                id: Some("api".to_string()),
                name: "Renamed API".to_string(),
                kind: DashboardModuleKind::Subscription,
                sort_order: 3,
                visible: true,
            })
            .unwrap_err();
        assert!(error.to_string().contains("kind"));

        let mut second_api = custom_module("Second API");
        second_api.kind = DashboardModuleKind::Api;
        let error = db.save_dashboard_module(&second_api).unwrap_err();
        assert!(error.to_string().contains("API"));
    }

    #[test]
    fn reorder_requires_each_module_exactly_once_and_updates_atomically() {
        let db = Database::memory().unwrap();
        let ids = vec![
            "api".to_string(),
            "codex".to_string(),
            "claude-code".to_string(),
            "kimi-coding-plan".to_string(),
        ];
        let reordered = db.reorder_dashboard_modules(&ids).unwrap();
        assert_eq!(
            reordered
                .iter()
                .map(|module| &module.id)
                .collect::<Vec<_>>(),
            ids.iter().collect::<Vec<_>>()
        );

        for invalid in [
            vec!["api".to_string()],
            vec![
                "api".to_string(),
                "api".to_string(),
                "claude-code".to_string(),
                "kimi-coding-plan".to_string(),
            ],
            vec![
                "api".to_string(),
                "codex".to_string(),
                "claude-code".to_string(),
                "missing".to_string(),
            ],
        ] {
            assert!(db.reorder_dashboard_modules(&invalid).is_err());
        }
        let unchanged = db.list_dashboard_modules().unwrap();
        assert_eq!(
            unchanged
                .iter()
                .map(|module| &module.id)
                .collect::<Vec<_>>(),
            ids.iter().collect::<Vec<_>>()
        );
    }

    #[test]
    fn visibility_and_delete_enforce_module_guards() {
        let db = Database::memory().unwrap();
        let hidden = db.set_dashboard_module_visibility("codex", false).unwrap();
        assert!(!hidden.visible);

        let custom = db.save_dashboard_module(&custom_module("Gemini")).unwrap();
        db.delete_dashboard_module(&custom.id).unwrap();
        assert!(db
            .list_dashboard_modules()
            .unwrap()
            .iter()
            .all(|module| module.id != custom.id));

        let error = db.delete_dashboard_module("api").unwrap_err();
        assert!(error.to_string().contains("system"));

        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO usage_providers (
                 id, name, billing_kind, product_group_id, token_sources,
                 enabled, needs_review, created_at, updated_at, dashboard_module_id
             ) VALUES ('codex-plan', 'Codex plan', 'subscription', 'codex', '[]',
                       1, 0, 1, 1, 'codex')",
            [],
        )
        .unwrap();
        drop(conn);
        let error = db.delete_dashboard_module("codex").unwrap_err();
        assert!(error.to_string().contains("providers"));
    }
}
