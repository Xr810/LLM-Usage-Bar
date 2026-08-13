use crate::database::Database;
use crate::error::AppError;
use crate::usage::domain::{session_agent_module_id, TokenSource};
use crate::usage::ingestion::{LegacyLogInput, UsageIngestionInput, UsageIngestionService};
use crate::usage::metering::parser::TokenUsage;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ParsedSessionRecord {
    pub event_id: String,
    pub occurred_at: i64,
    pub model: String,
    pub usage: TokenUsage,
    pub request_id: Option<String>,
    pub session_id: Option<String>,
    pub upstream_correlation_id: Option<String>,
    pub legacy: Option<LegacyLogInput>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSessionSyncResult {
    pub imported: u32,
    pub skipped: u32,
    pub files_scanned: u32,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

pub(crate) fn validate_bound_session_agent(
    _db: &Database,
    source: &str,
    _provider_id: &str,
) -> Result<&'static str, AppError> {
    session_agent_module_id(source)
        .ok_or_else(|| AppError::Message(format!("unsupported usage source: {source}")))
}

#[derive(Clone)]
pub struct SessionUsageService {
    db: Arc<Database>,
}

impl SessionUsageService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub fn sync_provider(&self, provider_id: &str) -> Result<ProviderSessionSyncResult, AppError> {
        self.db
            .get_usage_provider(provider_id)?
            .ok_or_else(|| AppError::Message("usage provider not found".to_string()))?;
        let sources = self.bound_sources_for_provider(provider_id)?;
        if sources.is_empty() {
            return Ok(ProviderSessionSyncResult {
                warnings: vec![format!(
                    "no usage source binding for provider {provider_id}"
                )],
                ..ProviderSessionSyncResult::default()
            });
        }

        self.sync_bound_sources(provider_id, sources)
    }

    fn sync_bound_sources(
        &self,
        provider_id: &str,
        sources: Vec<&str>,
    ) -> Result<ProviderSessionSyncResult, AppError> {
        let mut combined = ProviderSessionSyncResult::default();
        for source in sources {
            let result = self.sync_bound_source(source, provider_id)?;
            combined.imported = combined.imported.saturating_add(result.imported);
            combined.skipped = combined.skipped.saturating_add(result.skipped);
            combined.files_scanned = combined.files_scanned.saturating_add(result.files_scanned);
            combined.errors.extend(result.errors);
            combined.warnings.extend(result.warnings);
        }
        Ok(combined)
    }

    pub fn sync_source(&self, source: &str) -> Result<ProviderSessionSyncResult, AppError> {
        let Some(binding) = self.binding_or_warning(source)? else {
            return Ok(ProviderSessionSyncResult {
                warnings: vec![format!("no usage source binding for {source}")],
                ..ProviderSessionSyncResult::default()
            });
        };

        self.sync_bound_source(source, &binding.provider_id)
    }

    fn sync_bound_source(
        &self,
        source: &str,
        provider_id: &str,
    ) -> Result<ProviderSessionSyncResult, AppError> {
        match source {
            "claude" => crate::services::session_usage::sync_claude_session_logs_bound(
                &self.db,
                provider_id,
            ),
            "codex" => {
                crate::services::session_usage_codex::sync_codex_usage_bound(&self.db, provider_id)
            }
            _ => Err(AppError::Message(format!(
                "unsupported usage source: {source}"
            ))),
        }
    }

    pub fn ingest_records(
        &self,
        source: &str,
        records: Vec<ParsedSessionRecord>,
    ) -> Result<ProviderSessionSyncResult, AppError> {
        let Some(binding) = self.binding_or_warning(source)? else {
            return Ok(ProviderSessionSyncResult {
                warnings: vec![format!("no usage source binding for {source}")],
                ..ProviderSessionSyncResult::default()
            });
        };
        let agent_module_id = validate_bound_session_agent(&self.db, source, &binding.provider_id)?;

        let ingestion = UsageIngestionService::new(&self.db);
        let mut result = ProviderSessionSyncResult::default();
        for record in records {
            let outcome = ingestion.ingest(&UsageIngestionInput {
                event_id: record.event_id,
                source: TokenSource::SessionLog,
                provider_id: binding.provider_id.clone(),
                agent_module_id: agent_module_id.to_string(),
                frozen_provider_context: None,
                occurred_at: record.occurred_at,
                model: record.model,
                usage: record.usage,
                upstream_cost: None,
                request_id: record.request_id,
                session_id: record.session_id,
                upstream_correlation_id: record.upstream_correlation_id,
                legacy: record.legacy,
            })?;
            if outcome.inserted {
                result.imported += 1;
            } else {
                result.skipped += 1;
            }
        }
        Ok(result)
    }

    fn binding_or_warning(
        &self,
        source: &str,
    ) -> Result<Option<crate::usage::domain::UsageSourceBinding>, AppError> {
        if !matches!(source, "claude" | "codex") {
            return Err(AppError::Message(format!(
                "unsupported usage source: {source}"
            )));
        }
        self.db.get_usage_source_binding(source)
    }

    fn bound_sources_for_provider(&self, provider_id: &str) -> Result<Vec<&'static str>, AppError> {
        let mut sources = Vec::new();
        for source in ["claude", "codex"] {
            if self
                .db
                .get_usage_source_binding(source)?
                .is_some_and(|binding| binding.provider_id == provider_id)
            {
                sources.push(source);
            }
        }
        Ok(sources)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::usage::domain::{
        AgentProviderBindingInput, BillingKind, TokenSource, UsageProviderInput,
    };
    use crate::usage::ingestion::{
        FrozenUsageProviderContext, UsageIngestionInput, UsageIngestionService,
    };
    use crate::usage::metering::parser::TokenUsage;
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::Duration;

    fn database_without_system_source_bindings() -> Database {
        let db = Database::memory().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "DELETE FROM usage_source_bindings
                 WHERE provider_id IN (
                     SELECT id FROM usage_providers WHERE system_preset_key IS NOT NULL
                 )",
                [],
            )
            .unwrap();
        }
        db
    }

    fn provider(id: &str) -> UsageProviderInput {
        UsageProviderInput {
            id: id.to_string(),
            name: id.to_string(),
            billing_kind: BillingKind::Subscription,
            product_group_id: "claude".to_string(),
            token_sources: vec![TokenSource::Proxy, TokenSource::SessionLog],
            session_source_bindings: None,
            quota_source: None,
            quota_interval_seconds: Some(300),
            route_app_type: None,
            route_config: None,
            quota_config: None,
            enabled: true,
        }
    }

    fn record(event_id: &str, request_id: Option<&str>) -> ParsedSessionRecord {
        ParsedSessionRecord {
            event_id: event_id.to_string(),
            occurred_at: 1_000,
            model: "claude-sonnet-4-5".to_string(),
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 2,
                cache_read_tokens: 1,
                cache_creation_tokens: 0,
                model: Some("claude-sonnet-4-5".to_string()),
                message_id: None,
            },
            request_id: request_id.map(str::to_string),
            session_id: None,
            upstream_correlation_id: None,
            legacy: None,
        }
    }

    fn seed_agent_binding(db: &Database, agent_module_id: &str, provider_id: &str, enabled: bool) {
        let binding = db
            .save_agent_provider_binding(&AgentProviderBindingInput {
                id: None,
                agent_module_id: agent_module_id.to_string(),
                provider_id: provider_id.to_string(),
                enabled: false,
            })
            .unwrap();
        if enabled {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE agent_provider_bindings SET enabled = 1 WHERE id = ?1",
                [&binding.id],
            )
            .unwrap();
        }
    }

    #[test]
    fn unbound_source_is_skipped_with_visible_warning() {
        let db = Arc::new(database_without_system_source_bindings());
        db.save_usage_provider(&provider("sub")).unwrap();
        let service = SessionUsageService::new(db.clone());

        let result = service
            .ingest_records("claude", vec![record("session-1", None)])
            .unwrap();

        assert_eq!(result.imported, 0);
        assert_eq!(result.warnings, vec!["no usage source binding for claude"]);
        let conn = db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM usage_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn bound_records_use_provider_and_only_exact_ids_link() {
        let db = Arc::new(database_without_system_source_bindings());
        db.save_usage_provider(&provider("sub")).unwrap();
        db.set_usage_source_binding("claude", "sub").unwrap();
        seed_agent_binding(&db, "claude-code", "sub", true);
        let service = SessionUsageService::new(db.clone());

        UsageIngestionService::new(&db)
            .ingest(&UsageIngestionInput {
                event_id: "proxy-like".to_string(),
                source: TokenSource::Proxy,
                provider_id: "sub".to_string(),
                agent_module_id: "claude-code".to_string(),
                frozen_provider_context: Some(FrozenUsageProviderContext {
                    product_group_id: "claude".to_string(),
                    route_app_type: "claude".to_string(),
                }),
                occurred_at: 1_000,
                model: "claude-sonnet-4-5".to_string(),
                usage: record("unused", None).usage,
                upstream_cost: None,
                request_id: Some("stable-1".to_string()),
                session_id: None,
                upstream_correlation_id: None,
                legacy: None,
            })
            .unwrap();

        let result = service
            .ingest_records(
                "claude",
                vec![
                    record("exact-match", Some("stable-1")),
                    record("similar-no-id", None),
                ],
            )
            .unwrap();
        assert_eq!(result.imported, 2);

        let conn = db.conn.lock().unwrap();
        let providers: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM usage_events WHERE provider_id='sub'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let links: i64 = conn
            .query_row("SELECT COUNT(*) FROM usage_event_links", [], |row| {
                row.get(0)
            })
            .unwrap();
        let quotas: i64 = conn
            .query_row("SELECT COUNT(*) FROM quota_snapshots", [], |row| row.get(0))
            .unwrap();
        assert_eq!(providers, 3);
        assert_eq!(links, 1, "only the exact cross-source ID must link");
        assert_eq!(quotas, 0, "session import must never create quota rows");
    }

    #[test]
    fn source_parsers_freeze_their_fixed_agents_even_with_multiple_provider_bindings() {
        let db = Arc::new(database_without_system_source_bindings());
        db.save_usage_provider(&provider("shared")).unwrap();
        db.set_usage_source_binding("claude", "shared").unwrap();
        db.set_usage_source_binding("codex", "shared").unwrap();
        seed_agent_binding(&db, "claude-code", "shared", true);
        seed_agent_binding(&db, "codex", "shared", true);
        let service = SessionUsageService::new(db.clone());

        assert_eq!(
            service
                .ingest_records("claude", vec![record("claude-event", None)])
                .unwrap()
                .imported,
            1
        );
        assert_eq!(
            service
                .ingest_records("codex", vec![record("codex-event", None)])
                .unwrap()
                .imported,
            1
        );

        let conn = db.conn.lock().unwrap();
        let owners: Vec<(String, Option<String>)> = {
            let mut statement = conn
                .prepare("SELECT event_id, agent_module_id FROM usage_events ORDER BY event_id")
                .unwrap();
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(
            owners,
            vec![
                ("claude-event".to_string(), Some("claude-code".to_string())),
                ("codex-event".to_string(), Some("codex".to_string())),
            ]
        );
    }

    #[test]
    fn source_binding_imports_without_an_agent_provider_binding() {
        let db = Arc::new(database_without_system_source_bindings());
        db.save_usage_provider(&provider("provider-account"))
            .unwrap();
        db.set_usage_source_binding("claude", "provider-account")
            .unwrap();
        let service = SessionUsageService::new(db.clone());

        let result = service
            .ingest_records("claude", vec![record("provider-event", None)])
            .unwrap();

        assert_eq!(result.imported, 1);
        let conn = db.conn.lock().unwrap();
        let ownership: (String, Option<String>) = conn
            .query_row(
                "SELECT provider_id, agent_module_id FROM usage_events",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            ownership,
            (
                "provider-account".to_string(),
                Some("claude-code".to_string())
            )
        );
    }

    #[test]
    fn rebinding_and_rescan_cannot_move_an_existing_session_event() {
        let db = Arc::new(database_without_system_source_bindings());
        db.save_usage_provider(&provider("first-owner")).unwrap();
        db.save_usage_provider(&provider("second-owner")).unwrap();
        seed_agent_binding(&db, "claude-code", "first-owner", true);
        seed_agent_binding(&db, "claude-code", "second-owner", true);
        db.set_usage_source_binding("claude", "first-owner")
            .unwrap();
        let service = SessionUsageService::new(db.clone());
        let replayed = record("stable-session-event", None);

        assert_eq!(
            service
                .ingest_records("claude", vec![replayed.clone()])
                .unwrap()
                .imported,
            1
        );
        db.set_usage_source_binding("claude", "second-owner")
            .unwrap();
        assert_eq!(
            service
                .ingest_records("claude", vec![replayed])
                .unwrap_err()
                .to_string(),
            "usage_event_ownership_conflict"
        );

        let conn = db.conn.lock().unwrap();
        let owner: (String, Option<String>) = conn
            .query_row(
                "SELECT provider_id, agent_module_id FROM usage_events
                 WHERE event_id = 'stable-session-event'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            owner,
            ("first-owner".to_string(), Some("claude-code".to_string()))
        );
    }

    #[test]
    fn sync_provider_never_uses_another_providers_source_binding() {
        let db = Arc::new(database_without_system_source_bindings());
        db.save_usage_provider(&provider("requested-a")).unwrap();
        db.save_usage_provider(&provider("bound-b")).unwrap();
        db.set_usage_source_binding("claude", "bound-b").unwrap();
        let service = SessionUsageService::new(db.clone());

        let result = service.sync_provider("requested-a").unwrap();

        assert_eq!(result.imported, 0);
        assert_eq!(result.files_scanned, 0);
        assert_eq!(
            result.warnings,
            vec!["no usage source binding for provider requested-a"]
        );
        let conn = db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM usage_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn explicit_source_bindings_override_product_group_and_include_every_source() {
        let db = Arc::new(database_without_system_source_bindings());
        db.save_usage_provider(&provider("sub")).unwrap();
        let service = SessionUsageService::new(db.clone());

        db.set_usage_source_binding("codex", "sub").unwrap();
        assert_eq!(
            service.bound_sources_for_provider("sub").unwrap(),
            vec!["codex"]
        );

        db.set_usage_source_binding("claude", "sub").unwrap();
        assert_eq!(
            service.bound_sources_for_provider("sub").unwrap(),
            vec!["claude", "codex"]
        );
    }

    #[test]
    fn provider_sync_rechecks_requested_owner_after_binding_changes() {
        let db = Arc::new(database_without_system_source_bindings());
        db.save_usage_provider(&provider("requested-a")).unwrap();
        db.save_usage_provider(&provider("new-owner-b")).unwrap();
        db.set_usage_source_binding("claude", "requested-a")
            .unwrap();
        let service = SessionUsageService::new(db.clone());

        let sources = service.bound_sources_for_provider("requested-a").unwrap();
        assert_eq!(sources, vec!["claude"]);
        db.set_usage_source_binding("claude", "new-owner-b")
            .unwrap();

        let result = service.sync_bound_sources("requested-a", sources).unwrap();
        assert_eq!(result.imported, 0);
        assert_eq!(result.files_scanned, 0);
        assert_eq!(result.warnings, vec!["no usage source binding for claude"]);
    }

    #[test]
    fn claude_and_codex_bound_entrypoints_wait_for_the_binding_operation_guard() {
        let db = Arc::new(database_without_system_source_bindings());
        let guard = db.usage_source_binding_operation.lock().unwrap();
        let (done_tx, done_rx) = mpsc::channel();

        let claude_db = db.clone();
        let claude_done = done_tx.clone();
        let claude = std::thread::spawn(move || {
            let result = crate::services::session_usage::sync_claude_session_logs_bound(
                &claude_db,
                "unbound-claude",
            )
            .unwrap();
            claude_done.send(("claude", result)).unwrap();
        });
        let codex_db = db.clone();
        let codex = std::thread::spawn(move || {
            let result = crate::services::session_usage_codex::sync_codex_usage_bound(
                &codex_db,
                "unbound-codex",
            )
            .unwrap();
            done_tx.send(("codex", result)).unwrap();
        });

        assert!(done_rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(guard);

        let mut completed = [
            done_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            done_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        ];
        completed.sort_by_key(|(source, _)| *source);
        assert_eq!(completed[0].0, "claude");
        assert_eq!(
            completed[0].1.warnings,
            vec!["no usage source binding for claude"]
        );
        assert_eq!(completed[1].0, "codex");
        assert_eq!(
            completed[1].1.warnings,
            vec!["no usage source binding for codex"]
        );
        claude.join().unwrap();
        codex.join().unwrap();
    }
}
