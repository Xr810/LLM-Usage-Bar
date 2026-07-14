use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::usage::domain::{AgentModuleInput, AgentModuleView};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentModuleDeleteOutcome {
    HardDeleted,
    Archived { binding_ids: Vec<String> },
}

fn invalid_agent_module() -> AppError {
    AppError::Message("invalid_agent_module".to_string())
}

fn now_timestamp() -> Result<i64, AppError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|error| AppError::Database(format!("system clock before unix epoch: {error}")))
}

fn agent_module_from_row(row: &Row<'_>) -> rusqlite::Result<AgentModuleView> {
    let provider_count = row.get::<_, i64>(7)?;
    Ok(AgentModuleView {
        id: row.get(0)?,
        name: row.get(1)?,
        sort_order: row.get(2)?,
        visible: row.get(3)?,
        is_fixed: row.get(4)?,
        archived_at: row.get(5)?,
        provider_count: u64::try_from(provider_count).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?,
    })
}

const AGENT_MODULE_VIEW_QUERY: &str =
    "SELECT module.id, module.name, module.sort_order, module.visible,
            module.is_fixed, module.archived_at, module.updated_at,
            (SELECT COUNT(DISTINCT binding.provider_id)
             FROM agent_provider_bindings AS binding
             WHERE binding.agent_module_id = module.id) AS provider_count
     FROM agent_modules AS module";

fn list_agent_modules_on_conn(
    conn: &Connection,
    include_archived: bool,
) -> Result<Vec<AgentModuleView>, AppError> {
    let filter = if include_archived {
        ""
    } else {
        " WHERE module.archived_at IS NULL"
    };
    let mut statement = conn.prepare(&format!(
        "{AGENT_MODULE_VIEW_QUERY}{filter} ORDER BY module.sort_order, module.id"
    ))?;
    let result = statement
        .query_map([], agent_module_from_row)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(AppError::from)?;
    Ok(result)
}

fn get_agent_module_on_conn(
    conn: &Connection,
    id: &str,
) -> Result<Option<AgentModuleView>, AppError> {
    conn.query_row(
        &format!("{AGENT_MODULE_VIEW_QUERY} WHERE module.id = ?1"),
        [id],
        agent_module_from_row,
    )
    .optional()
    .map_err(AppError::from)
}

impl Database {
    pub fn list_agent_modules(&self) -> Result<Vec<AgentModuleView>, AppError> {
        let conn = lock_conn!(self.conn);
        list_agent_modules_on_conn(&conn, false)
    }

    #[allow(dead_code)] // Used by archived diagnostics in Task 6.
    pub(crate) fn get_agent_module_including_archived(
        &self,
        id: &str,
    ) -> Result<Option<AgentModuleView>, AppError> {
        let conn = lock_conn!(self.conn);
        get_agent_module_on_conn(&conn, id)
    }

    pub fn save_agent_module(&self, input: &AgentModuleInput) -> Result<AgentModuleView, AppError> {
        let name = input.name.trim();
        if name.is_empty() || input.id.as_deref() == Some("api") {
            return Err(invalid_agent_module());
        }
        let now = now_timestamp()?;
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction()?;
        let id = if let Some(id) = input.id.as_deref() {
            let existing = transaction
                .query_row(
                    "SELECT name, is_fixed, archived_at FROM agent_modules WHERE id = ?1",
                    [id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, bool>(1)?,
                            row.get::<_, Option<i64>>(2)?,
                        ))
                    },
                )
                .optional()?
                .ok_or_else(invalid_agent_module)?;
            if existing.2.is_some() || (existing.1 && existing.0 != name) {
                return Err(invalid_agent_module());
            }
            let updated = if existing.1 {
                transaction.execute(
                    "UPDATE agent_modules
                     SET sort_order = ?2, visible = ?3, updated_at = ?4
                     WHERE id = ?1 AND archived_at IS NULL",
                    params![id, input.sort_order, input.visible, now],
                )?
            } else {
                transaction.execute(
                    "UPDATE agent_modules
                     SET name = ?2, sort_order = ?3, visible = ?4, updated_at = ?5
                     WHERE id = ?1 AND archived_at IS NULL",
                    params![id, name, input.sort_order, input.visible, now],
                )?
            };
            if updated != 1 {
                return Err(invalid_agent_module());
            }
            id.to_string()
        } else {
            let id = uuid::Uuid::new_v4().to_string();
            transaction.execute(
                "INSERT INTO agent_modules (
                     id, name, sort_order, visible, is_fixed, archived_at,
                     created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, 0, NULL, ?5, ?5)",
                params![id, name, input.sort_order, input.visible, now],
            )?;
            id
        };
        let view = get_agent_module_on_conn(&transaction, &id)?.ok_or_else(invalid_agent_module)?;
        transaction.commit()?;
        Ok(view)
    }

    pub fn reorder_agent_modules(
        &self,
        agent_ids: &[String],
    ) -> Result<Vec<AgentModuleView>, AppError> {
        let now = now_timestamp()?;
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction()?;
        let existing = list_agent_modules_on_conn(&transaction, false)?;
        let expected = existing
            .iter()
            .map(|agent| agent.id.as_str())
            .collect::<HashSet<_>>();
        let requested = agent_ids.iter().map(String::as_str).collect::<HashSet<_>>();
        if agent_ids.len() != existing.len()
            || requested.len() != agent_ids.len()
            || requested != expected
        {
            return Err(invalid_agent_module());
        }
        for (sort_order, id) in agent_ids.iter().enumerate() {
            transaction.execute(
                "UPDATE agent_modules SET sort_order = ?2, updated_at = ?3
                 WHERE id = ?1 AND archived_at IS NULL",
                params![id, sort_order as i64, now],
            )?;
        }
        let result = list_agent_modules_on_conn(&transaction, false)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn set_agent_module_visibility(
        &self,
        agent_id: &str,
        visible: bool,
    ) -> Result<AgentModuleView, AppError> {
        if agent_id == "api" {
            return Err(invalid_agent_module());
        }
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction()?;
        if transaction.execute(
            "UPDATE agent_modules SET visible = ?2, updated_at = ?3
             WHERE id = ?1 AND archived_at IS NULL",
            params![agent_id, visible, now_timestamp()?],
        )? != 1
        {
            return Err(invalid_agent_module());
        }
        let view =
            get_agent_module_on_conn(&transaction, agent_id)?.ok_or_else(invalid_agent_module)?;
        transaction.commit()?;
        Ok(view)
    }

    pub fn delete_agent_module(
        &self,
        agent_id: &str,
    ) -> Result<AgentModuleDeleteOutcome, AppError> {
        if agent_id == "api" {
            return Err(invalid_agent_module());
        }
        let now = now_timestamp()?;
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction()?;
        let module = transaction
            .query_row(
                "SELECT is_fixed, archived_at, ever_bound
                 FROM agent_modules WHERE id = ?1",
                [agent_id],
                |row| {
                    Ok((
                        row.get::<_, bool>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(invalid_agent_module)?;
        if module.0 || module.1.is_some() {
            return Err(invalid_agent_module());
        }

        let has_history: bool = transaction.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM usage_events WHERE agent_module_id = ?1
             )",
            [agent_id],
            |row| row.get(0),
        )?;
        let binding_ids = {
            let mut statement = transaction.prepare(
                "SELECT id FROM agent_provider_bindings
                 WHERE agent_module_id = ?1 ORDER BY id",
            )?;
            let ids = statement
                .query_map([agent_id], |row| row.get(0))?
                .collect::<Result<Vec<String>, _>>()?;
            ids
        };

        let outcome = if has_history || module.2 || !binding_ids.is_empty() {
            // Any binding is also the lease/tombstone for a request that may
            // already have frozen this Agent identity. Archive first and make
            // every binding ineffective in the same transaction.
            transaction.execute(
                "UPDATE agent_modules
                 SET archived_at = ?2, visible = 0, updated_at = ?2 WHERE id = ?1",
                params![agent_id, now],
            )?;
            transaction.execute(
                "UPDATE agent_provider_bindings
                 SET enabled = 0, updated_at = ?2 WHERE agent_module_id = ?1",
                params![agent_id, now],
            )?;
            AgentModuleDeleteOutcome::Archived { binding_ids }
        } else {
            transaction.execute("DELETE FROM agent_modules WHERE id = ?1", [agent_id])?;
            AgentModuleDeleteOutcome::HardDeleted
        };
        transaction.commit()?;
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use crate::database::Database;
    use crate::usage::domain::{AgentModuleInput, AgentProviderBindingInput};

    fn create_custom(db: &Database, name: &str) -> crate::usage::domain::AgentModuleView {
        db.save_agent_module(&AgentModuleInput {
            id: None,
            name: name.to_string(),
            sort_order: 20,
            visible: true,
        })
        .unwrap()
    }

    fn insert_provider(db: &Database, id: &str) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO usage_providers (
                 id, name, billing_kind, product_group_id, token_sources,
                 route_app_type, enabled, needs_review, created_at, updated_at
             ) VALUES (?1, ?1, 'subscription', 'test', '[\"session_log\"]',
                       NULL, 1, 0, 10, 10)",
            [id],
        )
        .unwrap();
    }

    #[test]
    fn fresh_database_lists_exactly_five_fixed_agents_without_api() {
        let db = Database::memory().unwrap();
        let agents = db.list_agent_modules().unwrap();
        assert_eq!(
            agents
                .iter()
                .map(|agent| agent.id.as_str())
                .collect::<Vec<_>>(),
            vec!["codex", "claude-code", "opencode", "openclaw", "hermes"]
        );
        assert!(agents.iter().all(|agent| agent.is_fixed));
        assert!(agents.iter().all(|agent| agent.archived_at.is_none()));
        assert!(agents.iter().all(|agent| agent.provider_count == 0));
    }

    #[test]
    fn fixed_agents_reject_rename_and_delete_but_allow_visibility_and_order() {
        let db = Database::memory().unwrap();
        let error = db
            .save_agent_module(&AgentModuleInput {
                id: Some("codex".to_string()),
                name: "Renamed".to_string(),
                sort_order: 0,
                visible: true,
            })
            .unwrap_err();
        assert_eq!(error.to_string(), "invalid_agent_module");
        assert_eq!(
            db.delete_agent_module("codex").unwrap_err().to_string(),
            "invalid_agent_module"
        );

        let hidden = db.set_agent_module_visibility("codex", false).unwrap();
        assert!(!hidden.visible);
        let reordered = db
            .reorder_agent_modules(&[
                "hermes".into(),
                "openclaw".into(),
                "opencode".into(),
                "claude-code".into(),
                "codex".into(),
            ])
            .unwrap();
        assert_eq!(reordered[0].id, "hermes");
    }

    #[test]
    fn custom_agent_can_be_created_trimmed_renamed_hidden_and_hard_deleted_when_unused() {
        let db = Database::memory().unwrap();
        let mut custom = create_custom(&db, "  Team Agent  ");
        assert_eq!(custom.name, "Team Agent");
        assert!(!custom.is_fixed);
        uuid::Uuid::parse_str(&custom.id).expect("custom ID is an opaque UUID");

        custom = db
            .save_agent_module(&AgentModuleInput {
                id: Some(custom.id.clone()),
                name: "Renamed Agent".to_string(),
                sort_order: 8,
                visible: false,
            })
            .unwrap();
        assert_eq!(custom.name, "Renamed Agent");
        assert!(!custom.visible);

        let outcome = db.delete_agent_module(&custom.id).unwrap();
        assert_eq!(outcome, super::AgentModuleDeleteOutcome::HardDeleted);
        assert!(db
            .get_agent_module_including_archived(&custom.id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn custom_agent_with_history_is_archived_and_removed_from_navigation() {
        let db = Database::memory().unwrap();
        let custom = create_custom(&db, "History Agent");
        insert_provider(&db, "history-provider");
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO usage_events (
                     event_id, source, provider_id, product_group_id, agent_module_id,
                     occurred_at, model, input_tokens, output_tokens,
                     cache_read_tokens, cache_creation_tokens, cost_source, created_at
                 ) VALUES ('history-event', 'session_log', 'history-provider', 'test', ?1,
                           10, 'model', 1, 1, 0, 0, 'unavailable', 10)",
                [&custom.id],
            )
            .unwrap();
        }

        let outcome = db.delete_agent_module(&custom.id).unwrap();
        assert!(matches!(
            outcome,
            super::AgentModuleDeleteOutcome::Archived { .. }
        ));
        assert!(!db
            .list_agent_modules()
            .unwrap()
            .iter()
            .any(|agent| agent.id == custom.id));
        let archived = db
            .get_agent_module_including_archived(&custom.id)
            .unwrap()
            .unwrap();
        assert!(archived.archived_at.is_some());
        assert!(!archived.visible);
    }

    #[test]
    fn custom_agent_with_binding_is_archived_and_bindings_are_disabled_atomically() {
        let db = Database::memory().unwrap();
        let custom = create_custom(&db, "Bound Agent");
        insert_provider(&db, "bound-provider");
        let binding = db
            .save_agent_provider_binding(&AgentProviderBindingInput {
                id: None,
                agent_module_id: custom.id.clone(),
                provider_id: "bound-provider".to_string(),
                enabled: true,
            })
            .unwrap();

        let outcome = db.delete_agent_module(&custom.id).unwrap();
        let binding_ids = match outcome {
            super::AgentModuleDeleteOutcome::Archived { binding_ids } => binding_ids,
            super::AgentModuleDeleteOutcome::HardDeleted => {
                panic!("bound Agent must be archived")
            }
        };
        assert_eq!(binding_ids, vec![binding.id.clone()]);
        let listed = db.list_agent_provider_bindings(Some(&custom.id)).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].enabled);
    }

    #[test]
    fn formerly_bound_agent_keeps_a_tombstone_after_binding_metadata_is_deleted() {
        let db = Database::memory().unwrap();
        let custom = create_custom(&db, "Formerly Bound Agent");
        insert_provider(&db, "former-provider");
        let binding = db
            .save_agent_provider_binding(&AgentProviderBindingInput {
                id: None,
                agent_module_id: custom.id.clone(),
                provider_id: "former-provider".to_string(),
                enabled: false,
            })
            .unwrap();
        db.delete_agent_provider_binding_metadata(&binding.id, 0)
            .unwrap();

        let outcome = db.delete_agent_module(&custom.id).unwrap();
        assert_eq!(
            outcome,
            super::AgentModuleDeleteOutcome::Archived {
                binding_ids: Vec::new()
            }
        );
        let archived = db
            .get_agent_module_including_archived(&custom.id)
            .unwrap()
            .expect("ever-bound Agent must remain as an attribution tombstone");
        assert!(archived.archived_at.is_some());
    }

    #[test]
    fn reorder_requires_every_active_agent_exactly_once_and_rolls_back() {
        let db = Database::memory().unwrap();
        let before = db.list_agent_modules().unwrap();
        for invalid in [
            vec!["codex".to_string()],
            vec![
                "codex".into(),
                "codex".into(),
                "opencode".into(),
                "openclaw".into(),
                "hermes".into(),
            ],
            vec![
                "missing".into(),
                "claude-code".into(),
                "opencode".into(),
                "openclaw".into(),
                "hermes".into(),
            ],
        ] {
            assert_eq!(
                db.reorder_agent_modules(&invalid).unwrap_err().to_string(),
                "invalid_agent_module"
            );
            assert_eq!(db.list_agent_modules().unwrap(), before);
        }
    }

    #[test]
    fn provider_count_counts_distinct_bindings_including_disabled() {
        let db = Database::memory().unwrap();
        insert_provider(&db, "one");
        insert_provider(&db, "two");
        for provider_id in ["one", "two"] {
            db.save_agent_provider_binding(&AgentProviderBindingInput {
                id: None,
                agent_module_id: "codex".to_string(),
                provider_id: provider_id.to_string(),
                enabled: true,
            })
            .unwrap();
        }
        let codex = db
            .list_agent_modules()
            .unwrap()
            .into_iter()
            .find(|agent| agent.id == "codex")
            .unwrap();
        assert_eq!(codex.provider_count, 2);
    }
}
