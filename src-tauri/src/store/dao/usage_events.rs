use crate::error::AppError;
use crate::model::{
    ArchivedAgentUsageSummary, CostSource, InvalidUsageLinkSummary, TokenSource,
    UnassignedUsageDiagnostics, UnassignedUsageGroup, UsageEvent, UsageEventLink, UsageEventPage,
};
use crate::store::{lock_conn, Database};
use rusqlite::{params, types::Type, OptionalExtension, Row};

fn token_source_value(source: TokenSource) -> &'static str {
    match source {
        TokenSource::Proxy => "proxy",
        TokenSource::SessionLog => "session_log",
    }
}

fn cost_source_value(source: CostSource) -> &'static str {
    match source {
        CostSource::Upstream => "upstream",
        CostSource::Estimated => "estimated",
        CostSource::Unavailable => "unavailable",
    }
}

fn enum_from_text<T: serde::de::DeserializeOwned>(
    value: String,
    column: usize,
) -> rusqlite::Result<T> {
    serde_json::from_str(&format!("\"{value}\"")).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(error))
    })
}

fn nonnegative_u64(row: &Row<'_>, column: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(column)?;
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(column, Type::Integer, Box::new(error))
    })
}

fn usage_event_from_row(row: &Row<'_>) -> rusqlite::Result<UsageEvent> {
    Ok(UsageEvent {
        event_id: row.get(0)?,
        source: enum_from_text(row.get(1)?, 1)?,
        provider_id: row.get(2)?,
        product_group_id: row.get(3)?,
        occurred_at: row.get(4)?,
        model: row.get(5)?,
        input_tokens: nonnegative_u64(row, 6)?,
        output_tokens: nonnegative_u64(row, 7)?,
        cache_read_tokens: nonnegative_u64(row, 8)?,
        cache_creation_tokens: nonnegative_u64(row, 9)?,
        request_id: row.get(10)?,
        session_id: row.get(11)?,
        upstream_correlation_id: row.get(12)?,
        input_cost_usd: row.get(13)?,
        output_cost_usd: row.get(14)?,
        cache_read_cost_usd: row.get(15)?,
        cache_creation_cost_usd: row.get(16)?,
        total_cost_usd: row.get(17)?,
        cost_source: enum_from_text(row.get(18)?, 18)?,
        legacy_request_id: row.get(19)?,
        created_at: row.get(20)?,
        agent_module_id: row.get(21)?,
        pricing_origin: row
            .get::<_, Option<String>>(22)?
            .map(|value| enum_from_text(value, 22))
            .transpose()?,
    })
}

const EVENT_COLUMNS: &str = "event_id, source, provider_id, product_group_id, occurred_at,
    model, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
    request_id, session_id, upstream_correlation_id, input_cost_usd, output_cost_usd,
    cache_read_cost_usd, cache_creation_cost_usd, total_cost_usd, cost_source,
    legacy_request_id, created_at, agent_module_id, pricing_origin";

const USAGE_EVENT_OWNERSHIP_CONFLICT: &str = "usage_event_ownership_conflict";

fn token_count(value: u64) -> Result<i64, AppError> {
    i64::try_from(value).map_err(|_| AppError::Message("token count is too large".to_string()))
}

impl Database {
    pub fn insert_usage_event(&self, event: &UsageEvent) -> Result<bool, AppError> {
        let conn = lock_conn!(self.conn);
        let inserted = conn.execute(
            "INSERT INTO usage_events (
                event_id, source, provider_id, product_group_id, occurred_at, model,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                request_id, session_id, upstream_correlation_id, input_cost_usd,
                output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd,
                total_cost_usd, cost_source, legacy_request_id, created_at,
                agent_module_id, pricing_origin
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23
             ) ON CONFLICT(event_id) DO NOTHING",
            params![
                event.event_id,
                token_source_value(event.source),
                event.provider_id,
                event.product_group_id,
                event.occurred_at,
                event.model,
                token_count(event.input_tokens)?,
                token_count(event.output_tokens)?,
                token_count(event.cache_read_tokens)?,
                token_count(event.cache_creation_tokens)?,
                event.request_id,
                event.session_id,
                event.upstream_correlation_id,
                event.input_cost_usd,
                event.output_cost_usd,
                event.cache_read_cost_usd,
                event.cache_creation_cost_usd,
                event.total_cost_usd,
                cost_source_value(event.cost_source),
                event.legacy_request_id,
                event.created_at,
                event.agent_module_id,
                event
                    .pricing_origin
                    .map(crate::model::PricingOrigin::as_str),
            ],
        )?;
        if inserted == 0 {
            let existing_ownership = conn
                .query_row(
                    "SELECT provider_id, agent_module_id
                     FROM usage_events WHERE event_id = ?1",
                    [&event.event_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                )
                .optional()?;
            if !existing_ownership.is_some_and(|(provider_id, agent_module_id)| {
                provider_id == event.provider_id && agent_module_id == event.agent_module_id
            }) {
                return Err(AppError::Message(
                    USAGE_EVENT_OWNERSHIP_CONFLICT.to_string(),
                ));
            }
        }
        Ok(inserted == 1)
    }

    pub fn find_matching_usage_event(
        &self,
        provider_id: &str,
        event: &UsageEvent,
    ) -> Result<Option<UsageEvent>, AppError> {
        if provider_id != event.provider_id {
            return Err(AppError::Message(
                "provider_id must match event.provider_id".to_string(),
            ));
        }
        let Some(agent_module_id) = event.agent_module_id.as_deref() else {
            return Ok(None);
        };
        let conn = lock_conn!(self.conn);
        for (column, value) in [
            ("request_id", event.request_id.as_deref()),
            ("session_id", event.session_id.as_deref()),
            (
                "upstream_correlation_id",
                event.upstream_correlation_id.as_deref(),
            ),
        ] {
            let Some(value) = value.filter(|value| !value.is_empty()) else {
                continue;
            };
            let sql = format!(
                "SELECT {EVENT_COLUMNS} FROM usage_events
                 WHERE provider_id = ?1
                   AND agent_module_id = ?2
                   AND source <> ?3
                   AND {column} = ?4
                 ORDER BY occurred_at DESC, event_id DESC LIMIT 1"
            );
            if let Some(matched) = conn
                .query_row(
                    &sql,
                    params![
                        provider_id,
                        agent_module_id,
                        token_source_value(event.source),
                        value
                    ],
                    usage_event_from_row,
                )
                .optional()?
            {
                return Ok(Some(matched));
            }
        }
        Ok(None)
    }

    pub fn insert_usage_event_link(&self, link: &UsageEventLink) -> Result<bool, AppError> {
        let conn = lock_conn!(self.conn);
        let canonical = conn
            .query_row(
                "SELECT source, provider_id, agent_module_id
                 FROM usage_events WHERE event_id = ?1",
                [&link.canonical_event_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;
        let duplicate = conn
            .query_row(
                "SELECT source, provider_id, agent_module_id
                 FROM usage_events WHERE event_id = ?1",
                [&link.duplicate_event_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;
        let (
            Some((canonical_source, canonical_provider, canonical_agent)),
            Some((duplicate_source, duplicate_provider, duplicate_agent)),
        ) = (canonical, duplicate)
        else {
            return Err(AppError::Message("usage event not found".to_string()));
        };
        if canonical_provider != duplicate_provider {
            return Err(AppError::Message(
                "usage event links require same provider".to_string(),
            ));
        }
        if canonical_agent.is_none() || canonical_agent != duplicate_agent {
            return Err(AppError::Message(
                "usage event links require identical non-null agent ownership".to_string(),
            ));
        }
        if canonical_source == duplicate_source {
            return Err(AppError::Message(
                "usage event links require different sources".to_string(),
            ));
        }
        if canonical_source != token_source_value(TokenSource::Proxy)
            || duplicate_source != token_source_value(TokenSource::SessionLog)
        {
            return Err(AppError::Message(
                "usage event links require proxy canonical and session_log duplicate".to_string(),
            ));
        }

        let inserted = conn.execute(
            "INSERT INTO usage_event_links (
                canonical_event_id, duplicate_event_id, link_kind, link_value, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(canonical_event_id, duplicate_event_id, link_kind, link_value)
             DO NOTHING",
            params![
                link.canonical_event_id,
                link.duplicate_event_id,
                link.link_kind,
                link.link_value,
                link.created_at,
            ],
        )?;
        Ok(inserted == 1)
    }

    pub fn list_usage_events(
        &self,
        provider_id: &str,
        start_at: i64,
        end_at: i64,
        page: u32,
        page_size: u32,
    ) -> Result<UsageEventPage, AppError> {
        if !(1..=200).contains(&page_size) {
            return Err(AppError::Message(
                "page size must be between 1 and 200".to_string(),
            ));
        }
        if page == 0 {
            return Err(AppError::Message("page must be at least 1".to_string()));
        }
        let offset = u64::from(page - 1)
            .checked_mul(u64::from(page_size))
            .and_then(|value| i64::try_from(value).ok())
            .ok_or_else(|| AppError::Message("pagination offset is too large".to_string()))?;

        let conn = lock_conn!(self.conn);
        let total = conn.query_row(
            "SELECT COUNT(*) FROM usage_events
             WHERE provider_id = ?1 AND occurred_at >= ?2 AND occurred_at < ?3",
            params![provider_id, start_at, end_at],
            |row| row.get::<_, u64>(0),
        )?;
        let mut statement = conn.prepare(&format!(
            "SELECT {EVENT_COLUMNS} FROM usage_events
             WHERE provider_id = ?1 AND occurred_at >= ?2 AND occurred_at < ?3
             ORDER BY occurred_at DESC, event_id DESC LIMIT ?4 OFFSET ?5"
        ))?;
        let items = statement
            .query_map(
                params![provider_id, start_at, end_at, page_size, offset],
                usage_event_from_row,
            )?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(UsageEventPage {
            items,
            total,
            page: u64::from(page),
            page_size: u64::from(page_size),
        })
    }

    pub fn list_agent_usage_events(
        &self,
        agent_module_id: &str,
        provider_id: Option<&str>,
        start_at: i64,
        end_at: i64,
        page: u64,
        page_size: u64,
    ) -> Result<UsageEventPage, AppError> {
        if !(1..=200).contains(&page_size) {
            return Err(AppError::Message(
                "page size must be between 1 and 200".to_string(),
            ));
        }
        if page == 0 {
            return Err(AppError::Message("page must be at least 1".to_string()));
        }
        let offset = page
            .checked_sub(1)
            .and_then(|value| value.checked_mul(page_size))
            .and_then(|value| i64::try_from(value).ok())
            .ok_or_else(|| AppError::Message("pagination offset is too large".to_string()))?;
        let limit = i64::try_from(page_size)
            .map_err(|_| AppError::Message("page size is too large".to_string()))?;

        let conn = lock_conn!(self.conn);
        let total = conn.query_row(
            "SELECT COUNT(*) FROM usage_events
             WHERE agent_module_id = ?1
               AND (?2 IS NULL OR provider_id = ?2)
               AND occurred_at >= ?3 AND occurred_at < ?4",
            params![agent_module_id, provider_id, start_at, end_at],
            |row| row.get::<_, u64>(0),
        )?;
        let mut statement = conn.prepare(&format!(
            "SELECT {EVENT_COLUMNS} FROM usage_events
             WHERE agent_module_id = ?1
               AND (?2 IS NULL OR provider_id = ?2)
               AND occurred_at >= ?3 AND occurred_at < ?4
             ORDER BY occurred_at DESC, event_id DESC LIMIT ?5 OFFSET ?6"
        ))?;
        let items = statement
            .query_map(
                params![
                    agent_module_id,
                    provider_id,
                    start_at,
                    end_at,
                    limit,
                    offset
                ],
                usage_event_from_row,
            )?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(UsageEventPage {
            items,
            total,
            page,
            page_size,
        })
    }

    pub fn get_unassigned_usage_diagnostics(&self) -> Result<UnassignedUsageDiagnostics, AppError> {
        let conn = lock_conn!(self.conn);

        let unassigned_event_count = conn.query_row(
            "SELECT COUNT(*) FROM usage_events WHERE agent_module_id IS NULL",
            [],
            |row| row.get::<_, u64>(0),
        )?;
        let unassigned_groups = {
            let mut statement = conn.prepare(
                "SELECT provider_id, source, COUNT(*), MIN(occurred_at), MAX(occurred_at)
                 FROM usage_events
                 WHERE agent_module_id IS NULL
                 GROUP BY provider_id, source
                 ORDER BY provider_id, source",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok(UnassignedUsageGroup {
                        provider_id: row.get(0)?,
                        source: enum_from_text(row.get(1)?, 1)?,
                        event_count: nonnegative_u64(row, 2)?,
                        first_occurred_at: row.get(3)?,
                        last_occurred_at: row.get(4)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let archived_agent_history = {
            let mut statement = conn.prepare(
                "SELECT event.agent_module_id, COUNT(*),
                        MIN(event.occurred_at), MAX(event.occurred_at)
                 FROM usage_events AS event
                 JOIN agent_modules AS agent ON agent.id = event.agent_module_id
                 WHERE agent.archived_at IS NOT NULL
                 GROUP BY event.agent_module_id
                 ORDER BY event.agent_module_id",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok(ArchivedAgentUsageSummary {
                        agent_module_id: row.get(0)?,
                        event_count: nonnegative_u64(row, 1)?,
                        first_occurred_at: row.get(2)?,
                        last_occurred_at: row.get(3)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let invalid_link_summaries = {
            let mut statement = conn.prepare(
                "WITH classified AS (
                     SELECT link.created_at,
                            CASE
                              WHEN canonical.event_id IS NULL OR duplicate.event_id IS NULL
                                THEN 'missing_endpoint'
                              WHEN canonical.source != 'proxy'
                                OR duplicate.source != 'session_log'
                                THEN 'invalid_source_direction'
                              WHEN canonical.provider_id != duplicate.provider_id
                                THEN 'provider_mismatch'
                              WHEN canonical.agent_module_id IS NULL
                                OR duplicate.agent_module_id IS NULL
                                OR canonical.agent_module_id != duplicate.agent_module_id
                                THEN 'agent_missing_or_mismatch'
                              ELSE NULL
                            END AS reason
                     FROM usage_event_links AS link
                     LEFT JOIN usage_events AS canonical
                       ON canonical.event_id = link.canonical_event_id
                     LEFT JOIN usage_events AS duplicate
                       ON duplicate.event_id = link.duplicate_event_id
                 )
                 SELECT reason, COUNT(*), MIN(created_at), MAX(created_at)
                 FROM classified
                 WHERE reason IS NOT NULL
                 GROUP BY reason
                 ORDER BY reason",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok(InvalidUsageLinkSummary {
                        reason: row.get(0)?,
                        link_count: nonnegative_u64(row, 1)?,
                        first_created_at: row.get(2)?,
                        last_created_at: row.get(3)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };

        Ok(UnassignedUsageDiagnostics {
            unassigned_event_count,
            unassigned_groups,
            archived_agent_history,
            invalid_link_summaries,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{
        AgentModuleInput, BillingKind, CostSource, TokenSource, UsageEvent, UsageEventLink,
        UsageProviderInput,
    };
    use crate::store::Database;

    fn save_provider(db: &Database, id: &str) {
        db.save_usage_provider(&UsageProviderInput {
            id: id.to_string(),
            name: id.to_string(),
            billing_kind: BillingKind::Metered,
            product_group_id: "claude".to_string(),
            token_sources: vec![TokenSource::Proxy, TokenSource::SessionLog],
            session_source_bindings: None,
            quota_source: None,
            quota_interval_seconds: None,
            route_app_type: None,
            route_config: None,
            quota_config: None,
            enabled: true,
        })
        .unwrap();
    }

    fn event(id: &str, provider_id: &str, source: TokenSource, occurred_at: i64) -> UsageEvent {
        UsageEvent {
            event_id: id.to_string(),
            source,
            provider_id: provider_id.to_string(),
            agent_module_id: None,
            product_group_id: "claude".to_string(),
            occurred_at,
            model: "claude-sonnet".to_string(),
            input_tokens: 10,
            output_tokens: 20,
            cache_read_tokens: 3,
            cache_creation_tokens: 4,
            request_id: None,
            session_id: None,
            upstream_correlation_id: None,
            input_cost_usd: Some("0.01".to_string()),
            output_cost_usd: Some("0.02".to_string()),
            cache_read_cost_usd: None,
            cache_creation_cost_usd: None,
            total_cost_usd: Some("0.03".to_string()),
            cost_source: CostSource::Upstream,
            pricing_origin: None,
            legacy_request_id: None,
            created_at: occurred_at + 1,
        }
    }

    fn event_for_agent(
        id: &str,
        provider_id: &str,
        source: TokenSource,
        occurred_at: i64,
        agent_module_id: Option<&str>,
    ) -> UsageEvent {
        let mut event = event(id, provider_id, source, occurred_at);
        event.agent_module_id = agent_module_id.map(str::to_string);
        event
    }

    #[test]
    fn duplicate_event_id_does_not_overwrite_the_original() {
        let db = Database::memory().unwrap();
        save_provider(&db, "metered");
        let original = event("evt", "metered", TokenSource::Proxy, 100);
        assert!(db.insert_usage_event(&original).unwrap());

        let mut changed_duplicate = original.clone();
        changed_duplicate.model = "changed".to_string();
        changed_duplicate.input_tokens = 999;
        assert!(!db.insert_usage_event(&changed_duplicate).unwrap());

        let page = db.list_usage_events("metered", 0, 200, 1, 10).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items, vec![original]);
    }

    #[test]
    fn duplicate_event_id_rejects_conflicting_provider_ownership() {
        let db = Database::memory().unwrap();
        save_provider(&db, "one");
        save_provider(&db, "two");
        assert!(db
            .insert_usage_event(&event("evt", "one", TokenSource::Proxy, 100))
            .unwrap());

        let error = db
            .insert_usage_event(&event("evt", "two", TokenSource::Proxy, 101))
            .unwrap_err();

        assert_eq!(error.to_string(), "usage_event_ownership_conflict");
    }

    #[test]
    fn event_round_trips_frozen_agent_ownership_and_rejects_agent_conflicts() {
        let db = Database::memory().unwrap();
        save_provider(&db, "metered");
        let mut original = event("agent-owned", "metered", TokenSource::Proxy, 100);
        original.agent_module_id = Some("codex".to_string());
        assert!(db.insert_usage_event(&original).unwrap());

        let mut same_ownership = original.clone();
        same_ownership.model = "changed-but-idempotent".to_string();
        assert!(!db.insert_usage_event(&same_ownership).unwrap());

        let mut conflicting = original.clone();
        conflicting.agent_module_id = Some("claude-code".to_string());
        let error = db.insert_usage_event(&conflicting).unwrap_err();
        assert_eq!(error.to_string(), "usage_event_ownership_conflict");

        let stored = db
            .list_usage_events("metered", 0, 200, 1, 10)
            .unwrap()
            .items
            .pop()
            .unwrap();
        assert_eq!(stored, original);
    }

    #[test]
    fn stable_match_requires_same_provider_and_an_exact_non_empty_identifier() {
        let db = Database::memory().unwrap();
        save_provider(&db, "one");
        save_provider(&db, "two");
        let mut stored = event_for_agent(
            "stored",
            "one",
            TokenSource::Proxy,
            100,
            Some("claude-code"),
        );
        stored.request_id = Some("request-exact".to_string());
        stored.session_id = Some("session-exact".to_string());
        db.insert_usage_event(&stored).unwrap();

        let mut exact = event_for_agent(
            "probe",
            "one",
            TokenSource::SessionLog,
            101,
            Some("claude-code"),
        );
        exact.request_id = Some("request-exact".to_string());
        assert_eq!(
            db.find_matching_usage_event("one", &exact)
                .unwrap()
                .unwrap()
                .event_id,
            "stored"
        );

        exact.request_id = Some("request".to_string());
        assert!(db
            .find_matching_usage_event("one", &exact)
            .unwrap()
            .is_none());
        let mut other_provider = exact.clone();
        other_provider.provider_id = "two".to_string();
        other_provider.request_id = Some("request-exact".to_string());
        assert!(db
            .find_matching_usage_event("two", &other_provider)
            .unwrap()
            .is_none());

        exact.request_id = Some(String::new());
        exact.session_id = Some(String::new());
        exact.upstream_correlation_id = None;
        assert!(db
            .find_matching_usage_event("one", &exact)
            .unwrap()
            .is_none());
    }

    #[test]
    fn stable_match_requires_identical_non_null_agent_ownership() {
        let db = Database::memory().unwrap();
        save_provider(&db, "metered");
        let mut stored = event_for_agent(
            "stored",
            "metered",
            TokenSource::Proxy,
            100,
            Some("claude-code"),
        );
        stored.request_id = Some("shared-assigned".to_string());
        db.insert_usage_event(&stored).unwrap();

        let mut same_agent = event_for_agent(
            "same-agent",
            "metered",
            TokenSource::SessionLog,
            101,
            Some("claude-code"),
        );
        same_agent.request_id = Some("shared-assigned".to_string());
        assert_eq!(
            db.find_matching_usage_event("metered", &same_agent)
                .unwrap()
                .unwrap()
                .event_id,
            "stored"
        );

        let mut different_agent = event_for_agent(
            "different-agent",
            "metered",
            TokenSource::SessionLog,
            102,
            Some("codex"),
        );
        different_agent.request_id = Some("shared-assigned".to_string());
        assert!(db
            .find_matching_usage_event("metered", &different_agent)
            .unwrap()
            .is_none());

        let mut missing_probe_agent = event_for_agent(
            "missing-probe-agent",
            "metered",
            TokenSource::SessionLog,
            103,
            None,
        );
        missing_probe_agent.request_id = Some("shared-assigned".to_string());
        assert!(db
            .find_matching_usage_event("metered", &missing_probe_agent)
            .unwrap()
            .is_none());

        let mut unassigned =
            event_for_agent("unassigned", "metered", TokenSource::Proxy, 104, None);
        unassigned.request_id = Some("shared-unassigned".to_string());
        db.insert_usage_event(&unassigned).unwrap();

        let mut assigned_probe = same_agent.clone();
        assigned_probe.request_id = Some("shared-unassigned".to_string());
        assert!(db
            .find_matching_usage_event("metered", &assigned_probe)
            .unwrap()
            .is_none());

        let mut unassigned_probe = missing_probe_agent;
        unassigned_probe.request_id = Some("shared-unassigned".to_string());
        assert!(db
            .find_matching_usage_event("metered", &unassigned_probe)
            .unwrap()
            .is_none());
    }

    #[test]
    fn matching_rejects_provider_parameter_mismatch() {
        let db = Database::memory().unwrap();
        save_provider(&db, "one");
        save_provider(&db, "two");
        let mut stored = event("stored", "one", TokenSource::Proxy, 100);
        stored.request_id = Some("request-exact".to_string());
        db.insert_usage_event(&stored).unwrap();

        let mut probe = event("probe", "two", TokenSource::SessionLog, 101);
        probe.request_id = Some("request-exact".to_string());
        assert_eq!(
            db.find_matching_usage_event("one", &probe)
                .unwrap_err()
                .to_string(),
            "provider_id must match event.provider_id"
        );
    }

    #[test]
    fn event_links_only_connect_events_from_different_sources_and_are_idempotent() {
        let db = Database::memory().unwrap();
        save_provider(&db, "metered");
        let proxy = event_for_agent(
            "proxy",
            "metered",
            TokenSource::Proxy,
            100,
            Some("claude-code"),
        );
        let session = event_for_agent(
            "session",
            "metered",
            TokenSource::SessionLog,
            101,
            Some("claude-code"),
        );
        let second_proxy = event_for_agent(
            "proxy-2",
            "metered",
            TokenSource::Proxy,
            102,
            Some("claude-code"),
        );
        db.insert_usage_event(&proxy).unwrap();
        db.insert_usage_event(&session).unwrap();
        db.insert_usage_event(&second_proxy).unwrap();

        let link = UsageEventLink {
            canonical_event_id: "proxy".to_string(),
            duplicate_event_id: "session".to_string(),
            link_kind: "request_id".to_string(),
            link_value: "shared".to_string(),
            created_at: 103,
        };
        assert!(db.insert_usage_event_link(&link).unwrap());
        assert!(!db.insert_usage_event_link(&link).unwrap());

        let same_source = UsageEventLink {
            duplicate_event_id: "proxy-2".to_string(),
            ..link
        };
        assert_eq!(
            db.insert_usage_event_link(&same_source)
                .unwrap_err()
                .to_string(),
            "usage event links require different sources"
        );
    }

    #[test]
    fn event_links_require_identical_non_null_agent_ownership() {
        let db = Database::memory().unwrap();
        save_provider(&db, "metered");
        for event in [
            event_for_agent(
                "proxy-claude",
                "metered",
                TokenSource::Proxy,
                100,
                Some("claude-code"),
            ),
            event_for_agent(
                "session-codex",
                "metered",
                TokenSource::SessionLog,
                101,
                Some("codex"),
            ),
            event_for_agent(
                "session-unassigned",
                "metered",
                TokenSource::SessionLog,
                102,
                None,
            ),
            event_for_agent("proxy-unassigned", "metered", TokenSource::Proxy, 103, None),
        ] {
            db.insert_usage_event(&event).unwrap();
        }

        let link = |canonical_event_id: &str, duplicate_event_id: &str| UsageEventLink {
            canonical_event_id: canonical_event_id.to_string(),
            duplicate_event_id: duplicate_event_id.to_string(),
            link_kind: "request_id".to_string(),
            link_value: "shared".to_string(),
            created_at: 104,
        };

        assert!(db
            .insert_usage_event_link(&link("proxy-claude", "session-codex"))
            .is_err());
        assert!(db
            .insert_usage_event_link(&link("proxy-claude", "session-unassigned"))
            .is_err());
        assert!(db
            .insert_usage_event_link(&link("proxy-unassigned", "session-unassigned"))
            .is_err());
    }

    #[test]
    fn event_links_require_proxy_canonical_and_session_log_duplicate() {
        let db = Database::memory().unwrap();
        save_provider(&db, "metered");
        let proxy = event_for_agent(
            "proxy",
            "metered",
            TokenSource::Proxy,
            100,
            Some("claude-code"),
        );
        let session = event_for_agent(
            "session",
            "metered",
            TokenSource::SessionLog,
            101,
            Some("claude-code"),
        );
        db.insert_usage_event(&proxy).unwrap();
        db.insert_usage_event(&session).unwrap();

        let reversed = UsageEventLink {
            canonical_event_id: "session".to_string(),
            duplicate_event_id: "proxy".to_string(),
            link_kind: "request_id".to_string(),
            link_value: "shared".to_string(),
            created_at: 102,
        };
        assert!(db.insert_usage_event_link(&reversed).is_err());
    }

    #[test]
    fn event_links_reject_events_from_different_providers() {
        let db = Database::memory().unwrap();
        save_provider(&db, "one");
        save_provider(&db, "two");
        db.insert_usage_event(&event_for_agent(
            "proxy",
            "one",
            TokenSource::Proxy,
            100,
            Some("claude-code"),
        ))
        .unwrap();
        db.insert_usage_event(&event_for_agent(
            "session",
            "two",
            TokenSource::SessionLog,
            101,
            Some("claude-code"),
        ))
        .unwrap();

        let link = UsageEventLink {
            canonical_event_id: "proxy".to_string(),
            duplicate_event_id: "session".to_string(),
            link_kind: "request_id".to_string(),
            link_value: "shared".to_string(),
            created_at: 102,
        };
        assert_eq!(
            db.insert_usage_event_link(&link).unwrap_err().to_string(),
            "usage event links require same provider"
        );
    }

    #[test]
    fn pagination_is_deterministic_half_open_and_enforces_limits() {
        let db = Database::memory().unwrap();
        save_provider(&db, "metered");
        for (id, occurred_at) in [("before", 9), ("a", 10), ("b", 10), ("end", 20)] {
            db.insert_usage_event(&event(id, "metered", TokenSource::Proxy, occurred_at))
                .unwrap();
        }

        let first = db.list_usage_events("metered", 10, 20, 1, 1).unwrap();
        assert_eq!(first.total, 2);
        assert_eq!(first.items[0].event_id, "b");
        assert_eq!(first.page, 1);
        assert_eq!(first.page_size, 1);
        let second = db.list_usage_events("metered", 10, 20, 2, 1).unwrap();
        assert_eq!(second.items[0].event_id, "a");

        for invalid_size in [0, 201] {
            assert_eq!(
                db.list_usage_events("metered", 10, 20, 1, invalid_size)
                    .unwrap_err()
                    .to_string(),
                "page size must be between 1 and 200"
            );
        }
    }

    #[test]
    fn agent_event_page_filters_immutable_owner_and_optional_provider() {
        let db = Database::memory().unwrap();
        save_provider(&db, "one");
        save_provider(&db, "two");
        for usage_event in [
            event_for_agent("codex-one", "one", TokenSource::Proxy, 10, Some("codex")),
            event_for_agent(
                "claude-one",
                "one",
                TokenSource::Proxy,
                11,
                Some("claude-code"),
            ),
            event_for_agent("unassigned", "one", TokenSource::Proxy, 12, None),
            event_for_agent("codex-two", "two", TokenSource::Proxy, 13, Some("codex")),
            event_for_agent("codex-end", "one", TokenSource::Proxy, 20, Some("codex")),
        ] {
            db.insert_usage_event(&usage_event).unwrap();
        }

        let all = db
            .list_agent_usage_events("codex", None, 10, 20, 1, 10)
            .unwrap();
        assert_eq!(all.total, 2);
        assert_eq!(
            all.items
                .iter()
                .map(|event| event.event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["codex-two", "codex-one"]
        );

        let one = db
            .list_agent_usage_events("codex", Some("one"), 10, 20, 1, 10)
            .unwrap();
        assert_eq!(one.total, 1);
        assert_eq!(one.items[0].event_id, "codex-one");

        let first = db
            .list_agent_usage_events("codex", None, 10, 20, 1, 1)
            .unwrap();
        let second = db
            .list_agent_usage_events("codex", None, 10, 20, 2, 1)
            .unwrap();
        assert_eq!(first.items[0].event_id, "codex-two");
        assert_eq!(second.items[0].event_id, "codex-one");
        assert!(db
            .list_agent_usage_events("codex", None, 10, 20, u64::MAX, 200)
            .unwrap_err()
            .to_string()
            .contains("pagination offset is too large"));
    }

    #[test]
    fn diagnostics_group_only_unassigned_and_summarize_archived_and_invalid_links() {
        let db = Database::memory().unwrap();
        save_provider(&db, "one");
        save_provider(&db, "two");
        for (id, provider_id, source, occurred_at) in [
            ("one-proxy-a", "one", TokenSource::Proxy, 10),
            ("one-session", "one", TokenSource::SessionLog, 20),
            ("one-proxy-b", "one", TokenSource::Proxy, 30),
            ("two-proxy", "two", TokenSource::Proxy, 40),
        ] {
            let mut unassigned = event_for_agent(id, provider_id, source, occurred_at, None);
            unassigned.request_id = Some("request-secret".to_string());
            unassigned.session_id = Some("session-secret".to_string());
            unassigned.upstream_correlation_id = Some("correlation-secret".to_string());
            db.insert_usage_event(&unassigned).unwrap();
        }

        let archived = db
            .save_agent_module(&AgentModuleInput {
                id: None,
                name: "Archived".to_string(),
                sort_order: 20,
                visible: true,
            })
            .unwrap();
        db.insert_usage_event(&event_for_agent(
            "archived-event",
            "one",
            TokenSource::Proxy,
            50,
            Some(&archived.id),
        ))
        .unwrap();
        db.delete_agent_module(&archived.id).unwrap();

        let canonical = event_for_agent(
            "invalid-canonical",
            "one",
            TokenSource::Proxy,
            60,
            Some("claude-code"),
        );
        let duplicate = event_for_agent(
            "invalid-duplicate",
            "one",
            TokenSource::SessionLog,
            61,
            Some("codex"),
        );
        db.insert_usage_event(&canonical).unwrap();
        db.insert_usage_event(&duplicate).unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO usage_event_links (
                     canonical_event_id, duplicate_event_id, link_kind, link_value, created_at
                 ) VALUES (?1, ?2, 'request_id', 'legacy-secret', 62)",
                [canonical.event_id.as_str(), duplicate.event_id.as_str()],
            )
            .unwrap();

        let diagnostics = db.get_unassigned_usage_diagnostics().unwrap();
        assert_eq!(diagnostics.unassigned_event_count, 4);
        assert_eq!(diagnostics.unassigned_groups.len(), 3);
        let one_proxy = diagnostics
            .unassigned_groups
            .iter()
            .find(|group| group.provider_id == "one" && group.source == TokenSource::Proxy)
            .unwrap();
        assert_eq!(one_proxy.event_count, 2);
        assert_eq!(one_proxy.first_occurred_at, 10);
        assert_eq!(one_proxy.last_occurred_at, 30);
        assert_eq!(
            diagnostics.archived_agent_history[0].agent_module_id,
            archived.id
        );
        assert_eq!(diagnostics.archived_agent_history[0].event_count, 1);
        assert_eq!(
            diagnostics.invalid_link_summaries[0].reason,
            "agent_missing_or_mismatch"
        );
        assert_eq!(diagnostics.invalid_link_summaries[0].link_count, 1);

        let public = serde_json::to_string(&diagnostics).unwrap();
        for forbidden in [
            "request-secret",
            "session-secret",
            "correlation-secret",
            "legacy-secret",
        ] {
            assert!(!public.contains(forbidden));
        }
    }
}
