use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::proxy::usage::calculator::{CostBreakdown, CostCalculator, ModelPricing};
use crate::proxy::usage::cost_parser::UpstreamCost;
use crate::proxy::usage::parser::TokenUsage;
use crate::services::usage_stats::find_model_pricing_row;
use crate::usage::domain::{CostSource, TokenSource, UsageEvent};
use rusqlite::{params, OptionalExtension, Transaction};
use rust_decimal::Decimal;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct LegacyLogInput {
    /// Compatibility-table request key. This is intentionally independent
    /// from the v13 event ID and its stable identifiers.
    pub request_id: String,
    /// Transitional v12 provider identity used by `proxy_request_logs`.
    pub provider_id: String,
    pub app_type: String,
    pub request_model: String,
    pub pricing_model: String,
    pub latency_ms: u64,
    pub first_token_ms: Option<u64>,
    pub status_code: u16,
    pub error_message: Option<String>,
    pub session_id: Option<String>,
    pub provider_type: Option<String>,
    pub is_streaming: bool,
    pub cost_multiplier: Decimal,
}

#[derive(Debug, Clone)]
pub struct UsageIngestionInput {
    pub event_id: String,
    pub source: TokenSource,
    /// Global `usage_providers.id`. Never put a transitional v12 provider ID
    /// here; the compatibility identity belongs in `legacy.provider_id`.
    pub provider_id: String,
    pub occurred_at: i64,
    pub model: String,
    pub usage: TokenUsage,
    pub upstream_cost: Option<UpstreamCost>,
    pub request_id: Option<String>,
    pub session_id: Option<String>,
    pub upstream_correlation_id: Option<String>,
    pub legacy: Option<LegacyLogInput>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageIngestionOutcome {
    pub inserted: bool,
    pub link_created: bool,
}

#[derive(Debug, Clone)]
struct StoredProviderContext {
    product_group_id: String,
    route_app_type: Option<String>,
}

#[derive(Debug, Clone)]
struct TrustedCost {
    input: Option<String>,
    output: Option<String>,
    cache_read: Option<String>,
    cache_creation: Option<String>,
    total: Option<String>,
    source: CostSource,
}

impl TrustedCost {
    fn unavailable() -> Self {
        Self {
            input: None,
            output: None,
            cache_read: None,
            cache_creation: None,
            total: None,
            source: CostSource::Unavailable,
        }
    }

    fn from_upstream(cost: &UpstreamCost) -> Self {
        Self {
            input: cost.input_cost.map(|value| value.to_string()),
            output: cost.output_cost.map(|value| value.to_string()),
            cache_read: cost.cache_read_cost.map(|value| value.to_string()),
            cache_creation: cost.cache_creation_cost.map(|value| value.to_string()),
            total: cost.total_cost.map(|value| value.to_string()),
            source: CostSource::Upstream,
        }
    }

    fn from_estimate(cost: CostBreakdown) -> Self {
        Self {
            input: Some(cost.input_cost.to_string()),
            output: Some(cost.output_cost.to_string()),
            cache_read: Some(cost.cache_read_cost.to_string()),
            cache_creation: Some(cost.cache_creation_cost.to_string()),
            total: Some(cost.total_cost.to_string()),
            source: CostSource::Estimated,
        }
    }
}

pub struct UsageIngestionService<'a> {
    db: &'a Database,
}

impl<'a> UsageIngestionService<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn ingest(&self, input: &UsageIngestionInput) -> Result<UsageIngestionOutcome, AppError> {
        validate_input(input)?;
        let mut conn = lock_conn!(self.db.conn);
        let transaction = conn.transaction()?;

        let provider = load_and_validate_provider(&transaction, input)?;
        let trusted_cost = decide_cost(&transaction, input, &provider)?;
        let created_at = now_timestamp()?;
        let stable_match = find_stable_cross_source_match(&transaction, input)?;
        let event = build_event(input, &provider, &trusted_cost, created_at);
        let inserted = insert_event(&transaction, &event)?;

        if !inserted {
            transaction.commit()?;
            return Ok(UsageIngestionOutcome {
                inserted: false,
                link_created: false,
            });
        }

        let link_created = if let Some((canonical_event_id, link_kind, link_value)) = stable_match {
            transaction.execute(
                "INSERT INTO usage_event_links (
                    canonical_event_id, duplicate_event_id, link_kind, link_value, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT DO NOTHING",
                params![
                    canonical_event_id,
                    input.event_id,
                    link_kind,
                    link_value,
                    created_at,
                ],
            )? == 1
        } else {
            false
        };

        if let Some(legacy) = &input.legacy {
            insert_legacy_log(&transaction, input, legacy, &trusted_cost, created_at)?;
        }

        transaction.commit()?;
        crate::usage_events::notify_log_recorded();
        Ok(UsageIngestionOutcome {
            inserted: true,
            link_created,
        })
    }
}

fn validate_input(input: &UsageIngestionInput) -> Result<(), AppError> {
    if input.event_id.trim().is_empty() {
        return Err(AppError::Message(
            "usage event id must not be empty".to_string(),
        ));
    }
    if input.provider_id.trim().is_empty() {
        return Err(AppError::Message(
            "usage provider id must not be empty".to_string(),
        ));
    }
    if input.model.trim().is_empty() {
        return Err(AppError::Message(
            "usage model must not be empty".to_string(),
        ));
    }
    for (name, value) in [
        ("request_id", input.request_id.as_deref()),
        ("session_id", input.session_id.as_deref()),
        (
            "upstream_correlation_id",
            input.upstream_correlation_id.as_deref(),
        ),
    ] {
        if value.is_some_and(|value| value.is_empty()) {
            return Err(AppError::Message(format!(
                "stable identifier {name} must not be empty"
            )));
        }
    }
    Ok(())
}

fn load_and_validate_provider(
    transaction: &Transaction<'_>,
    input: &UsageIngestionInput,
) -> Result<StoredProviderContext, AppError> {
    let provider = transaction
        .query_row(
            "SELECT product_group_id, token_sources, route_app_type
             FROM usage_providers WHERE id = ?1",
            [&input.provider_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((product_group_id, token_sources, route_app_type)) = provider else {
        return Err(AppError::Message("usage provider not found".to_string()));
    };
    let token_sources: Vec<TokenSource> = serde_json::from_str(&token_sources)
        .map_err(|error| AppError::Database(format!("invalid provider token_sources: {error}")))?;
    if !token_sources.contains(&input.source) {
        return Err(AppError::Message(format!(
            "usage provider does not accept {}",
            token_source_value(input.source)
        )));
    }
    Ok(StoredProviderContext {
        product_group_id,
        route_app_type,
    })
}

fn decide_cost(
    transaction: &Transaction<'_>,
    input: &UsageIngestionInput,
    provider: &StoredProviderContext,
) -> Result<TrustedCost, AppError> {
    if let Some(upstream) = input
        .upstream_cost
        .as_ref()
        .filter(|cost| cost.has_any_value())
    {
        return Ok(TrustedCost::from_upstream(upstream));
    }

    let pricing_model = input
        .legacy
        .as_ref()
        .map(|legacy| legacy.pricing_model.as_str())
        .filter(|model| !model.trim().is_empty())
        .unwrap_or(&input.model);
    let pricing = find_model_pricing_row(transaction, pricing_model)?
        .map(|(input, output, cache_read, cache_creation)| {
            ModelPricing::from_strings(&input, &output, &cache_read, &cache_creation)
                .map_err(|error| AppError::Database(format!("invalid model pricing: {error}")))
        })
        .transpose()?;
    let Some(pricing) = pricing else {
        return Ok(TrustedCost::unavailable());
    };

    let app_type = input
        .legacy
        .as_ref()
        .map(|legacy| legacy.app_type.as_str())
        .or(provider.route_app_type.as_deref())
        .unwrap_or(&provider.product_group_id);
    let multiplier = input
        .legacy
        .as_ref()
        .map(|legacy| legacy.cost_multiplier)
        .unwrap_or(Decimal::ONE);
    Ok(TrustedCost::from_estimate(
        CostCalculator::calculate_for_app(app_type, &input.usage, &pricing, multiplier),
    ))
}

fn find_stable_cross_source_match(
    transaction: &Transaction<'_>,
    input: &UsageIngestionInput,
) -> Result<Option<(String, &'static str, String)>, AppError> {
    for (column, value) in [
        ("request_id", input.request_id.as_deref()),
        ("session_id", input.session_id.as_deref()),
        (
            "upstream_correlation_id",
            input.upstream_correlation_id.as_deref(),
        ),
    ] {
        let Some(value) = value.filter(|value| !value.is_empty()) else {
            continue;
        };
        let sql = format!(
            "SELECT event_id FROM usage_events
             WHERE provider_id = ?1 AND source <> ?2 AND {column} = ?3
             ORDER BY occurred_at ASC, event_id ASC LIMIT 1"
        );
        if let Some(event_id) = transaction
            .query_row(
                &sql,
                params![input.provider_id, token_source_value(input.source), value],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            return Ok(Some((event_id, column, value.to_string())));
        }
    }
    Ok(None)
}

fn build_event(
    input: &UsageIngestionInput,
    provider: &StoredProviderContext,
    cost: &TrustedCost,
    created_at: i64,
) -> UsageEvent {
    UsageEvent {
        event_id: input.event_id.clone(),
        source: input.source,
        provider_id: input.provider_id.clone(),
        product_group_id: provider.product_group_id.clone(),
        occurred_at: input.occurred_at,
        model: input.model.clone(),
        input_tokens: u64::from(input.usage.input_tokens),
        output_tokens: u64::from(input.usage.output_tokens),
        cache_read_tokens: u64::from(input.usage.cache_read_tokens),
        cache_creation_tokens: u64::from(input.usage.cache_creation_tokens),
        request_id: input.request_id.clone(),
        session_id: input.session_id.clone(),
        upstream_correlation_id: input.upstream_correlation_id.clone(),
        input_cost_usd: cost.input.clone(),
        output_cost_usd: cost.output.clone(),
        cache_read_cost_usd: cost.cache_read.clone(),
        cache_creation_cost_usd: cost.cache_creation.clone(),
        total_cost_usd: cost.total.clone(),
        cost_source: cost.source,
        legacy_request_id: input
            .legacy
            .as_ref()
            .map(|legacy| legacy.request_id.clone()),
        created_at,
    }
}

fn insert_event(transaction: &Transaction<'_>, event: &UsageEvent) -> Result<bool, AppError> {
    let inserted = transaction.execute(
        "INSERT INTO usage_events (
            event_id, source, provider_id, product_group_id, occurred_at, model,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
            request_id, session_id, upstream_correlation_id, input_cost_usd,
            output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd,
            total_cost_usd, cost_source, legacy_request_id, created_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
            ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21
         ) ON CONFLICT(event_id) DO NOTHING",
        params![
            event.event_id,
            token_source_value(event.source),
            event.provider_id,
            event.product_group_id,
            event.occurred_at,
            event.model,
            i64::try_from(event.input_tokens)
                .map_err(|_| AppError::Message("input token count is too large".to_string()))?,
            i64::try_from(event.output_tokens)
                .map_err(|_| AppError::Message("output token count is too large".to_string()))?,
            i64::try_from(event.cache_read_tokens).map_err(|_| AppError::Message(
                "cache read token count is too large".to_string()
            ))?,
            i64::try_from(event.cache_creation_tokens).map_err(|_| AppError::Message(
                "cache creation token count is too large".to_string()
            ))?,
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
        ],
    )?;
    Ok(inserted == 1)
}

fn insert_legacy_log(
    transaction: &Transaction<'_>,
    input: &UsageIngestionInput,
    legacy: &LegacyLogInput,
    cost: &TrustedCost,
    created_at: i64,
) -> Result<(), AppError> {
    transaction.execute(
        "INSERT INTO proxy_request_logs (
            request_id, provider_id, app_type, model, request_model, pricing_model,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
            input_cost_usd, output_cost_usd, cache_read_cost_usd,
            cache_creation_cost_usd, total_cost_usd, latency_ms, first_token_ms,
            status_code, error_message, session_id, provider_type, is_streaming,
            cost_multiplier, created_at, data_source
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
            ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25
         ) ON CONFLICT(request_id) DO NOTHING",
        params![
            legacy.request_id,
            legacy.provider_id,
            legacy.app_type,
            input.model,
            legacy.request_model,
            legacy.pricing_model,
            i64::from(input.usage.input_tokens),
            i64::from(input.usage.output_tokens),
            i64::from(input.usage.cache_read_tokens),
            i64::from(input.usage.cache_creation_tokens),
            cost.input.as_deref().unwrap_or("0"),
            cost.output.as_deref().unwrap_or("0"),
            cost.cache_read.as_deref().unwrap_or("0"),
            cost.cache_creation.as_deref().unwrap_or("0"),
            cost.total.as_deref().unwrap_or("0"),
            i64::try_from(legacy.latency_ms)
                .map_err(|_| AppError::Message("latency is too large".to_string()))?,
            legacy
                .first_token_ms
                .map(i64::try_from)
                .transpose()
                .map_err(|_| AppError::Message("first token latency is too large".to_string()))?,
            i64::from(legacy.status_code),
            legacy.error_message,
            legacy.session_id,
            legacy.provider_type,
            i64::from(legacy.is_streaming),
            legacy.cost_multiplier.to_string(),
            created_at,
            match input.source {
                TokenSource::Proxy => "proxy",
                TokenSource::SessionLog => "session",
            },
        ],
    )?;
    Ok(())
}

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

fn now_timestamp() -> Result<i64, AppError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|error| AppError::Database(format!("system clock before unix epoch: {error}")))
}

#[cfg(test)]
mod tests {
    use super::{
        LegacyLogInput, UsageIngestionInput, UsageIngestionOutcome, UsageIngestionService,
    };
    use crate::database::Database;
    use crate::proxy::usage::cost_parser::UpstreamCost;
    use crate::proxy::usage::parser::TokenUsage;
    use crate::usage::domain::{BillingKind, CostSource, TokenSource, UsageProviderInput};
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn save_provider(db: &Database, id: &str, legacy_id: Option<&str>) {
        db.save_usage_provider(&UsageProviderInput {
            id: id.to_string(),
            name: id.to_string(),
            billing_kind: BillingKind::Metered,
            product_group_id: "claude-product".to_string(),
            token_sources: vec![TokenSource::Proxy, TokenSource::SessionLog],
            session_source_bindings: None,
            quota_source: None,
            quota_interval_seconds: None,
            route_app_type: Some("claude".to_string()),
            route_config: None,
            quota_config: None,
            enabled: true,
        })
        .unwrap();

        if let Some(legacy_id) = legacy_id {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE usage_providers
                 SET legacy_app_type='claude', legacy_provider_id=?2
                 WHERE id=?1",
                rusqlite::params![id, legacy_id],
            )
            .unwrap();
        }
    }

    fn seed_pricing(db: &Database, model: &str) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO model_pricing (
                model_id, display_name, input_cost_per_million,
                output_cost_per_million, cache_read_cost_per_million,
                cache_creation_cost_per_million
             ) VALUES (?1, ?1, '1', '2', '0.5', '3')",
            [model],
        )
        .unwrap();
    }

    fn input(event_id: &str, source: TokenSource) -> UsageIngestionInput {
        UsageIngestionInput {
            event_id: event_id.to_string(),
            source,
            provider_id: "global-provider".to_string(),
            occurred_at: 100,
            model: "priced-model".to_string(),
            usage: TokenUsage {
                input_tokens: 1_000_000,
                output_tokens: 1_000_000,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                model: None,
                message_id: None,
            },
            upstream_cost: None,
            request_id: None,
            session_id: None,
            upstream_correlation_id: None,
            legacy: None,
        }
    }

    fn legacy(request_id: &str) -> LegacyLogInput {
        LegacyLogInput {
            request_id: request_id.to_string(),
            provider_id: "legacy-provider".to_string(),
            app_type: "claude".to_string(),
            request_model: "requested-model".to_string(),
            pricing_model: "priced-model".to_string(),
            latency_ms: 12,
            first_token_ms: Some(3),
            status_code: 200,
            error_message: None,
            session_id: Some("legacy-session".to_string()),
            provider_type: None,
            is_streaming: false,
            cost_multiplier: Decimal::ONE,
        }
    }

    #[test]
    fn upstream_cost_wins_over_known_pricing_and_preserves_global_legacy_identities() {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", Some("legacy-provider"));
        seed_pricing(&db, "priced-model");
        let service = UsageIngestionService::new(&db);
        let mut value = input("upstream", TokenSource::Proxy);
        value.upstream_cost = Some(UpstreamCost {
            input_cost: Some(Decimal::from_str("0.10").unwrap()),
            output_cost: None,
            cache_read_cost: None,
            cache_creation_cost: None,
            total_cost: Some(Decimal::from_str("0.25").unwrap()),
        });
        value.legacy = Some(legacy("legacy-upstream"));

        assert_eq!(
            service.ingest(&value).unwrap(),
            UsageIngestionOutcome {
                inserted: true,
                link_created: false,
            }
        );

        let page = db
            .list_usage_events("global-provider", 0, 200, 1, 10)
            .unwrap();
        let event = &page.items[0];
        assert_eq!(event.provider_id, "global-provider");
        assert_eq!(event.cost_source, CostSource::Upstream);
        assert_eq!(event.input_cost_usd.as_deref(), Some("0.10"));
        assert_eq!(event.output_cost_usd, None);
        assert_eq!(event.total_cost_usd.as_deref(), Some("0.25"));

        let conn = db.conn.lock().unwrap();
        let legacy_provider: String = conn
            .query_row(
                "SELECT provider_id FROM proxy_request_logs WHERE request_id='legacy-upstream'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(legacy_provider, "legacy-provider");
    }

    #[test]
    fn known_pricing_is_estimated_and_missing_pricing_is_unavailable() {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", None);
        seed_pricing(&db, "priced-model");
        let service = UsageIngestionService::new(&db);

        service
            .ingest(&input("estimated", TokenSource::Proxy))
            .unwrap();
        let mut unavailable = input("unavailable", TokenSource::Proxy);
        unavailable.model = "unknown-model".to_string();
        service.ingest(&unavailable).unwrap();

        let page = db
            .list_usage_events("global-provider", 0, 200, 1, 10)
            .unwrap();
        let estimated = page
            .items
            .iter()
            .find(|event| event.event_id == "estimated")
            .unwrap();
        assert_eq!(estimated.cost_source, CostSource::Estimated);
        assert_eq!(estimated.input_cost_usd.as_deref(), Some("1"));
        assert_eq!(estimated.output_cost_usd.as_deref(), Some("2"));
        assert_eq!(estimated.total_cost_usd.as_deref(), Some("3"));

        let unavailable = page
            .items
            .iter()
            .find(|event| event.event_id == "unavailable")
            .unwrap();
        assert_eq!(unavailable.cost_source, CostSource::Unavailable);
        assert_eq!(unavailable.input_cost_usd, None);
        assert_eq!(unavailable.output_cost_usd, None);
        assert_eq!(unavailable.cache_read_cost_usd, None);
        assert_eq!(unavailable.cache_creation_cost_usd, None);
        assert_eq!(unavailable.total_cost_usd, None);
    }

    #[test]
    fn duplicate_event_id_is_immutable() {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", None);
        let service = UsageIngestionService::new(&db);
        let original = input("same-event", TokenSource::Proxy);
        assert!(service.ingest(&original).unwrap().inserted);

        let mut changed = original;
        changed.model = "changed".to_string();
        changed.usage.input_tokens = 9;
        assert!(!service.ingest(&changed).unwrap().inserted);

        let stored = db
            .list_usage_events("global-provider", 0, 200, 1, 10)
            .unwrap()
            .items
            .pop()
            .unwrap();
        assert_eq!(stored.model, "priced-model");
        assert_eq!(stored.input_tokens, 1_000_000);
    }

    #[test]
    fn non_event_id_conflicts_are_not_silently_ignored() {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", Some("legacy-provider"));
        let service = UsageIngestionService::new(&db);

        let mut first = input("first-event", TokenSource::Proxy);
        first.legacy = Some(legacy("same-legacy-request"));
        assert!(service.ingest(&first).unwrap().inserted);

        let mut conflicting = input("different-event", TokenSource::Proxy);
        conflicting.legacy = Some(legacy("same-legacy-request"));
        assert!(service.ingest(&conflicting).is_err());

        let conn = db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM usage_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn forced_legacy_failure_rolls_back_new_event_and_exact_link() {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", Some("legacy-provider"));
        let service = UsageIngestionService::new(&db);
        let mut session = input("session", TokenSource::SessionLog);
        session.request_id = Some("stable-id".to_string());
        service.ingest(&session).unwrap();

        {
            let conn = db.conn.lock().unwrap();
            conn.execute_batch(
                "CREATE TRIGGER fail_legacy_insert
                 BEFORE INSERT ON proxy_request_logs
                 BEGIN
                   SELECT RAISE(ABORT, 'forced legacy failure');
                 END;",
            )
            .unwrap();
        }

        let mut proxy = input("proxy", TokenSource::Proxy);
        proxy.request_id = Some("stable-id".to_string());
        proxy.legacy = Some(legacy("legacy-fails"));
        assert!(service.ingest(&proxy).is_err());

        let conn = db.conn.lock().unwrap();
        let event_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM usage_events", [], |row| row.get(0))
            .unwrap();
        let link_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM usage_event_links", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(event_count, 1);
        assert_eq!(link_count, 0);
    }

    #[test]
    fn exact_cross_source_identifier_links_but_similarity_without_id_does_not() {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", None);
        let service = UsageIngestionService::new(&db);

        let mut proxy = input("proxy", TokenSource::Proxy);
        proxy.upstream_correlation_id = Some("exact-correlation".to_string());
        service.ingest(&proxy).unwrap();
        let mut session = input("session", TokenSource::SessionLog);
        session.occurred_at = 101;
        session.upstream_correlation_id = Some("exact-correlation".to_string());
        assert!(service.ingest(&session).unwrap().link_created);

        let similar_proxy = input("similar-proxy", TokenSource::Proxy);
        service.ingest(&similar_proxy).unwrap();
        let mut similar_session = input("similar-session", TokenSource::SessionLog);
        similar_session.occurred_at = 101;
        assert!(!service.ingest(&similar_session).unwrap().link_created);

        let conn = db.conn.lock().unwrap();
        let links: Vec<(String, String)> = {
            let mut statement = conn
                .prepare("SELECT link_kind, link_value FROM usage_event_links")
                .unwrap();
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(
            links,
            vec![(
                "upstream_correlation_id".to_string(),
                "exact-correlation".to_string()
            )]
        );
    }
}
