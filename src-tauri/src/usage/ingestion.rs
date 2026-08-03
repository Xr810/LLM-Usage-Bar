use crate::credentials::CredentialExposureGuardSet as CredentialExposureGuard;
use crate::database::{lock_conn, Database, UsageSyncCursor};
use crate::error::AppError;
use crate::proxy::usage::calculator::{CostBreakdown, CostCalculator, ModelPricing};
use crate::proxy::usage::cost_parser::UpstreamCost;
use crate::proxy::usage::parser::TokenUsage;
use crate::services::usage_stats::{find_model_pricing_row, find_provider_model_pricing_row};
use crate::usage::domain::{BillingKind, CostSource, PricingOrigin, TokenSource, UsageEvent};
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
    /// Frozen Agent ownership supplied by the trusted source boundary. Ambiguous
    /// historical rows may remain null in `UsageEvent`, but new inputs may not.
    pub agent_module_id: String,
    /// Provider grouping and route protocol frozen at the same trusted proxy
    /// binding lookup as `provider_id`. Non-proxy importers leave this absent
    /// and resolve their provider context transactionally during ingestion.
    pub frozen_provider_context: Option<FrozenUsageProviderContext>,
    pub occurred_at: i64,
    pub model: String,
    pub usage: TokenUsage,
    pub upstream_cost: Option<UpstreamCost>,
    pub request_id: Option<String>,
    pub session_id: Option<String>,
    pub upstream_correlation_id: Option<String>,
    pub legacy: Option<LegacyLogInput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenUsageProviderContext {
    pub product_group_id: String,
    pub route_app_type: String,
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
    /// Subscription accounts are always valued at the official reference price;
    /// only metered accounts consult the user's own `provider_model_pricing`.
    billing_kind: BillingKind,
}

#[derive(Debug, Clone)]
struct StableCrossSourceMatch {
    event_id: String,
    source: TokenSource,
    link_kind: &'static str,
    link_value: String,
}

#[derive(Debug, Clone)]
struct TrustedCost {
    input: Option<String>,
    output: Option<String>,
    cache_read: Option<String>,
    cache_creation: Option<String>,
    total: Option<String>,
    source: CostSource,
    /// Set only for estimates, so a stored row records which catalogue priced it.
    pricing_origin: Option<PricingOrigin>,
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
            pricing_origin: None,
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
            pricing_origin: None,
        }
    }

    fn from_estimate(cost: CostBreakdown, pricing_origin: PricingOrigin) -> Self {
        Self {
            input: Some(cost.input_cost.to_string()),
            output: Some(cost.output_cost.to_string()),
            cache_read: Some(cost.cache_read_cost.to_string()),
            cache_creation: Some(cost.cache_creation_cost.to_string()),
            total: Some(cost.total_cost.to_string()),
            source: CostSource::Estimated,
            pricing_origin: Some(pricing_origin),
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
        self.ingest_internal(input, None)
    }

    pub(crate) fn ingest_with_credential_guard(
        &self,
        input: &UsageIngestionInput,
        credential_guard: &CredentialExposureGuard,
    ) -> Result<UsageIngestionOutcome, AppError> {
        self.ingest_internal(input, Some(credential_guard))
    }

    fn ingest_internal(
        &self,
        input: &UsageIngestionInput,
        credential_guard: Option<&CredentialExposureGuard>,
    ) -> Result<UsageIngestionOutcome, AppError> {
        validate_input(input)?;
        let mut conn = lock_conn!(self.db.conn);
        let transaction = conn.transaction()?;
        let created_at = now_timestamp()?;
        let outcome =
            ingest_on_transaction_with_guard(&transaction, input, created_at, credential_guard)?;

        transaction.commit()?;
        if outcome.inserted {
            crate::usage_events::notify_log_recorded();
        }
        Ok(outcome)
    }

    /// Persist a parsed source batch and its durable cursors in one SQLite
    /// transaction. Callers may pass an empty input slice when a successfully
    /// parsed resource contains no billable events but its cursor still needs
    /// to advance.
    pub fn ingest_batch_and_advance_cursors(
        &self,
        inputs: &[UsageIngestionInput],
        cursors: &[UsageSyncCursor],
    ) -> Result<Vec<UsageIngestionOutcome>, AppError> {
        for input in inputs {
            validate_input(input)?;
        }

        let mut conn = lock_conn!(self.db.conn);
        let transaction = conn.transaction()?;
        let created_at = now_timestamp()?;
        let mut outcomes = Vec::with_capacity(inputs.len());
        for input in inputs {
            outcomes.push(ingest_on_transaction(&transaction, input, created_at)?);
        }
        for cursor in cursors {
            Database::put_usage_sync_cursor_on_conn(&transaction, cursor)?;
        }

        transaction.commit()?;
        if outcomes.iter().any(|outcome| outcome.inserted) {
            crate::usage_events::notify_log_recorded();
        }
        Ok(outcomes)
    }
}

fn ingest_on_transaction(
    transaction: &Transaction<'_>,
    input: &UsageIngestionInput,
    created_at: i64,
) -> Result<UsageIngestionOutcome, AppError> {
    ingest_on_transaction_with_guard(transaction, input, created_at, None)
}

fn ingest_on_transaction_with_guard(
    transaction: &Transaction<'_>,
    input: &UsageIngestionInput,
    created_at: i64,
    credential_guard: Option<&CredentialExposureGuard>,
) -> Result<UsageIngestionOutcome, AppError> {
    let provider = load_and_validate_provider(transaction, input)?;
    let trusted_cost = decide_cost(transaction, input, &provider)?;
    let stable_match = find_stable_cross_source_match(transaction, input)?;
    let event = build_event(input, &provider, &trusted_cost, created_at);
    if credential_guard.is_some_and(|credential_guard| {
        persisted_values_contain_credential(
            credential_guard,
            input,
            &event,
            &trusted_cost,
            stable_match.as_ref(),
        )
    }) {
        log::warn!(
            "Usage event omitted because persistence would repeat protected credential material"
        );
        return Ok(UsageIngestionOutcome {
            inserted: false,
            link_created: false,
        });
    }
    let inserted = insert_event(transaction, &event)?;

    if !inserted {
        return Ok(UsageIngestionOutcome {
            inserted: false,
            link_created: false,
        });
    }

    let link_created = if let Some(stable_match) = stable_match {
        let (canonical_event_id, duplicate_event_id) = match (input.source, stable_match.source) {
            (TokenSource::Proxy, TokenSource::SessionLog) => {
                (input.event_id.as_str(), stable_match.event_id.as_str())
            }
            (TokenSource::SessionLog, TokenSource::Proxy) => {
                (stable_match.event_id.as_str(), input.event_id.as_str())
            }
            _ => {
                return Err(AppError::Database(
                    "invalid cross-source usage match".to_string(),
                ))
            }
        };
        transaction.execute(
            "INSERT INTO usage_event_links (
                    canonical_event_id, duplicate_event_id, link_kind, link_value, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT DO NOTHING",
            params![
                canonical_event_id,
                duplicate_event_id,
                stable_match.link_kind,
                stable_match.link_value,
                created_at,
            ],
        )? == 1
    } else {
        false
    };

    if let Some(legacy) = &input.legacy {
        insert_legacy_log(transaction, input, legacy, &trusted_cost, created_at)?;
    }

    Ok(UsageIngestionOutcome {
        inserted: true,
        link_created,
    })
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
    if input.agent_module_id.trim().is_empty() {
        return Err(AppError::Message(
            "usage agent module id must not be empty".to_string(),
        ));
    }
    if (input.source == TokenSource::Proxy) != input.frozen_provider_context.is_some() {
        return Err(AppError::Message(
            "invalid frozen usage provider context".to_string(),
        ));
    }
    if let Some(context) = &input.frozen_provider_context {
        if context.product_group_id.trim().is_empty() || context.route_app_type.trim().is_empty() {
            return Err(AppError::Message(
                "invalid frozen usage provider context".to_string(),
            ));
        }
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
            "SELECT product_group_id, token_sources, route_app_type, billing_kind
             FROM usage_providers WHERE id = ?1",
            [&input.provider_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((product_group_id, token_sources, route_app_type, billing_kind)) = provider else {
        return Err(AppError::Message("usage provider not found".to_string()));
    };
    let token_sources: Vec<TokenSource> = serde_json::from_str(&token_sources)
        .map_err(|error| AppError::Database(format!("invalid provider token_sources: {error}")))?;
    if input.source != TokenSource::Proxy && !token_sources.contains(&input.source) {
        return Err(AppError::Message(format!(
            "usage provider does not accept {}",
            token_source_value(input.source)
        )));
    }
    let billing_kind = match billing_kind.as_str() {
        "subscription" => BillingKind::Subscription,
        "metered" => BillingKind::Metered,
        other => {
            return Err(AppError::Database(format!(
                "invalid provider billing_kind: {other}"
            )))
        }
    };
    if input.source == TokenSource::Proxy {
        if let Some(context) = &input.frozen_provider_context {
            return Ok(StoredProviderContext {
                product_group_id: context.product_group_id.clone(),
                route_app_type: Some(context.route_app_type.clone()),
                billing_kind,
            });
        }
    }
    Ok(StoredProviderContext {
        product_group_id,
        route_app_type,
        billing_kind,
    })
}

/// Decide what one event cost, in this order:
///
/// 1. A metered account's own price for the model, when the user has set one.
///    The user's real purchase price outranks whatever the upstream reported,
///    because a relay's self-reported cost is its list price, not the rate the
///    user actually pays.
/// 2. An explicit upstream cost, which is real billing evidence.
/// 3. The official reference catalogue, as an estimate.
///
/// Subscription accounts never reach steps 1 and 2: their spend is a synthetic
/// "equivalent API cost" and is always valued at the official list price.
///
/// Prices are resolved once, at ingest. Editing a price later does not rewrite
/// stored events — `usage_events` is append-only evidence.
fn decide_cost(
    transaction: &Transaction<'_>,
    input: &UsageIngestionInput,
    provider: &StoredProviderContext,
) -> Result<TrustedCost, AppError> {
    let pricing_model = input
        .legacy
        .as_ref()
        .map(|legacy| legacy.pricing_model.as_str())
        .filter(|model| !model.trim().is_empty())
        .unwrap_or(&input.model);
    let is_metered = provider.billing_kind == BillingKind::Metered;

    let user_pricing = if is_metered {
        parse_pricing_row(find_provider_model_pricing_row(
            transaction,
            &input.provider_id,
            pricing_model,
        )?)?
    } else {
        None
    };

    let (pricing, pricing_origin) = match user_pricing {
        Some(pricing) => (pricing, PricingOrigin::User),
        None => {
            if is_metered {
                if let Some(upstream) = input
                    .upstream_cost
                    .as_ref()
                    .filter(|cost| cost.has_any_value())
                {
                    return Ok(TrustedCost::from_upstream(upstream));
                }
            }
            match parse_pricing_row(find_model_pricing_row(transaction, pricing_model)?)? {
                Some(pricing) => (pricing, PricingOrigin::Official),
                None => return Ok(TrustedCost::unavailable()),
            }
        }
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
        pricing_origin,
    ))
}

fn parse_pricing_row(
    row: Option<(String, String, String, String)>,
) -> Result<Option<ModelPricing>, AppError> {
    row.map(|(input, output, cache_read, cache_creation)| {
        ModelPricing::from_strings(&input, &output, &cache_read, &cache_creation)
            .map_err(|error| AppError::Database(format!("invalid model pricing: {error}")))
    })
    .transpose()
}

fn find_stable_cross_source_match(
    transaction: &Transaction<'_>,
    input: &UsageIngestionInput,
) -> Result<Option<StableCrossSourceMatch>, AppError> {
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
            "SELECT event_id, source FROM usage_events
             WHERE provider_id = ?1 AND agent_module_id = ?2
               AND source <> ?3 AND {column} = ?4
             ORDER BY occurred_at ASC, event_id ASC LIMIT 1"
        );
        if let Some((event_id, source)) = transaction
            .query_row(
                &sql,
                params![
                    input.provider_id,
                    input.agent_module_id,
                    token_source_value(input.source),
                    value
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
        {
            let source = match source.as_str() {
                "proxy" => TokenSource::Proxy,
                "session_log" => TokenSource::SessionLog,
                _ => return Err(AppError::Database("invalid usage event source".to_string())),
            };
            return Ok(Some(StableCrossSourceMatch {
                event_id,
                source,
                link_kind: column,
                link_value: value.to_string(),
            }));
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
        agent_module_id: Some(input.agent_module_id.clone()),
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
        pricing_origin: cost.pricing_origin,
        legacy_request_id: input
            .legacy
            .as_ref()
            .map(|legacy| legacy.request_id.clone()),
        created_at,
    }
}

fn persisted_values_contain_credential(
    credential_guard: &CredentialExposureGuard,
    input: &UsageIngestionInput,
    event: &UsageEvent,
    cost: &TrustedCost,
    stable_match: Option<&StableCrossSourceMatch>,
) -> bool {
    let text_values = [
        event.event_id.as_str(),
        token_source_value(event.source),
        event.provider_id.as_str(),
        event.product_group_id.as_str(),
        event.model.as_str(),
        cost_source_value(event.cost_source),
    ];
    if text_values
        .into_iter()
        .any(|value| credential_guard.contains(value))
    {
        return true;
    }

    let optional_text_values = [
        event.agent_module_id.as_deref(),
        event.request_id.as_deref(),
        event.session_id.as_deref(),
        event.upstream_correlation_id.as_deref(),
        event.input_cost_usd.as_deref(),
        event.output_cost_usd.as_deref(),
        event.cache_read_cost_usd.as_deref(),
        event.cache_creation_cost_usd.as_deref(),
        event.total_cost_usd.as_deref(),
        event.legacy_request_id.as_deref(),
    ];
    if optional_text_values
        .into_iter()
        .flatten()
        .any(|value| credential_guard.contains(value))
    {
        return true;
    }

    let numeric_values = [
        event.occurred_at.to_string(),
        event.input_tokens.to_string(),
        event.output_tokens.to_string(),
        event.cache_read_tokens.to_string(),
        event.cache_creation_tokens.to_string(),
        event.created_at.to_string(),
    ];
    if numeric_values
        .into_iter()
        .any(|value| credential_guard.contains(&value))
    {
        return true;
    }

    if stable_match.is_some_and(|stable_match| {
        [
            stable_match.event_id.as_str(),
            token_source_value(stable_match.source),
            stable_match.link_kind,
            stable_match.link_value.as_str(),
        ]
        .into_iter()
        .any(|value| credential_guard.contains(value))
    }) {
        return true;
    }

    input.legacy.as_ref().is_some_and(|legacy| {
        let text_values = [
            legacy.request_id.as_str(),
            legacy.provider_id.as_str(),
            legacy.app_type.as_str(),
            input.model.as_str(),
            legacy.request_model.as_str(),
            legacy.pricing_model.as_str(),
            cost.input.as_deref().unwrap_or("0"),
            cost.output.as_deref().unwrap_or("0"),
            cost.cache_read.as_deref().unwrap_or("0"),
            cost.cache_creation.as_deref().unwrap_or("0"),
            cost.total.as_deref().unwrap_or("0"),
            token_source_value(input.source),
        ];
        text_values
            .into_iter()
            .any(|value| credential_guard.contains(value))
            || [
                legacy.error_message.as_deref(),
                legacy.session_id.as_deref(),
                legacy.provider_type.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(|value| credential_guard.contains(value))
            || [
                legacy.latency_ms.to_string(),
                legacy
                    .first_token_ms
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                legacy.status_code.to_string(),
                i64::from(legacy.is_streaming).to_string(),
                legacy.cost_multiplier.to_string(),
                event.created_at.to_string(),
            ]
            .into_iter()
            .filter(|value| !value.is_empty())
            .any(|value| credential_guard.contains(&value))
    })
}

fn insert_event(transaction: &Transaction<'_>, event: &UsageEvent) -> Result<bool, AppError> {
    let inserted = transaction.execute(
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
            event.agent_module_id,
            event.pricing_origin.map(PricingOrigin::as_str),
        ],
    )?;
    if inserted == 0 {
        let existing_ownership = transaction
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
                "usage_event_ownership_conflict".to_string(),
            ));
        }
    }
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
        FrozenUsageProviderContext, LegacyLogInput, UsageIngestionInput, UsageIngestionOutcome,
        UsageIngestionService,
    };
    use crate::database::{Database, UsageSyncCursor};
    use crate::proxy::usage::cost_parser::UpstreamCost;
    use crate::proxy::usage::parser::TokenUsage;
    use crate::usage::domain::{
        AgentModuleInput, AgentProviderBindingInput, BillingKind, CostSource, ModelPriceInput,
        PricingOrigin, TokenSource, UsageProviderInput,
    };

    fn price(input: &str, output: &str, cache_read: &str, cache_creation: &str) -> ModelPriceInput {
        ModelPriceInput {
            input_cost_per_million: input.to_string(),
            output_cost_per_million: output.to_string(),
            cache_read_cost_per_million: cache_read.to_string(),
            cache_creation_cost_per_million: cache_creation.to_string(),
        }
    }
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
            agent_module_id: "claude-code".to_string(),
            frozen_provider_context: (source == TokenSource::Proxy).then(frozen_provider_context),
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

    fn frozen_provider_context() -> FrozenUsageProviderContext {
        FrozenUsageProviderContext {
            product_group_id: "claude-product".to_string(),
            route_app_type: "claude".to_string(),
        }
    }

    fn seed_session_binding(
        db: &Database,
        agent_module_id: &str,
        provider_id: &str,
        enabled: bool,
    ) -> String {
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
        binding.id
    }

    fn attributed_input(
        event_id: &str,
        source: TokenSource,
        agent_module_id: &str,
    ) -> UsageIngestionInput {
        let mut value = input(event_id, source);
        value.agent_module_id = agent_module_id.to_string();
        value
    }

    #[test]
    fn resolved_proxy_event_survives_later_agent_delete_and_source_mutation() {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", None);
        let agent = db
            .save_agent_module(&AgentModuleInput {
                id: None,
                name: "In-flight Agent".to_string(),
                sort_order: 50,
                visible: true,
            })
            .unwrap();
        db.save_agent_provider_binding(&AgentProviderBindingInput {
            id: None,
            agent_module_id: agent.id.clone(),
            provider_id: "global-provider".to_string(),
            enabled: false,
        })
        .unwrap();

        let _delete_outcome = db.delete_agent_module(&agent.id).unwrap();
        let archived = db
            .get_agent_module_including_archived(&agent.id)
            .unwrap()
            .expect("resolved Agent identity must remain as a tombstone");
        assert!(archived.archived_at.is_some());
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE usage_providers SET token_sources = '[\"session_log\"]'
                 WHERE id = 'global-provider'",
                [],
            )
            .unwrap();
        }

        let mut event = input("resolved-before-mutation", TokenSource::Proxy);
        event.agent_module_id = agent.id.clone();
        event.frozen_provider_context = Some(frozen_provider_context());
        let outcome = UsageIngestionService::new(&db).ingest(&event).unwrap();
        assert!(outcome.inserted);
        let stored = db
            .list_usage_events("global-provider", 0, 200, 1, 10)
            .unwrap()
            .items
            .pop()
            .unwrap();
        assert_eq!(stored.agent_module_id.as_deref(), Some(agent.id.as_str()));
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

    fn cursor(byte_offset: i64) -> UsageSyncCursor {
        UsageSyncCursor {
            source: "claude".to_string(),
            cursor_key: "session.jsonl".to_string(),
            resource_path: Some("/tmp/session.jsonl".to_string()),
            resource_identity: Some("dev:1:ino:2".to_string()),
            modified_at_ns: 123,
            size_bytes: byte_offset,
            byte_offset,
            line_offset: 2,
            parser_state_json: Some("{\"version\":1}".to_string()),
            last_success_at: 456,
        }
    }

    #[test]
    fn batch_commits_all_events_and_cursor_together() {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", None);
        seed_session_binding(&db, "claude-code", "global-provider", true);
        let service = UsageIngestionService::new(&db);
        let inputs = [
            input("batch-first", TokenSource::SessionLog),
            input("batch-second", TokenSource::SessionLog),
        ];
        let next_cursor = cursor(200);

        let outcomes = service
            .ingest_batch_and_advance_cursors(&inputs, std::slice::from_ref(&next_cursor))
            .unwrap();

        assert_eq!(
            outcomes,
            vec![
                UsageIngestionOutcome {
                    inserted: true,
                    link_created: false,
                },
                UsageIngestionOutcome {
                    inserted: true,
                    link_created: false,
                },
            ]
        );
        assert_eq!(
            db.get_usage_sync_cursor("claude", "session.jsonl").unwrap(),
            Some(next_cursor)
        );
        let conn = db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM usage_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn batch_failure_rolls_back_prior_events_and_cursor() {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", None);
        seed_session_binding(&db, "claude-code", "global-provider", true);
        let service = UsageIngestionService::new(&db);
        let first = input("batch-rollback-first", TokenSource::SessionLog);
        let mut second = input("batch-rollback-second", TokenSource::SessionLog);
        second.provider_id = "missing-provider".to_string();

        assert!(service
            .ingest_batch_and_advance_cursors(&[first, second], &[cursor(300)])
            .is_err());

        assert!(db
            .get_usage_sync_cursor("claude", "session.jsonl")
            .unwrap()
            .is_none());
        let conn = db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM usage_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn batch_duplicate_still_advances_cursor() {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", None);
        seed_session_binding(&db, "claude-code", "global-provider", true);
        let service = UsageIngestionService::new(&db);
        let duplicate = input("batch-duplicate", TokenSource::SessionLog);
        assert!(service.ingest(&duplicate).unwrap().inserted);
        let next_cursor = cursor(400);

        let outcomes = service
            .ingest_batch_and_advance_cursors(&[duplicate], std::slice::from_ref(&next_cursor))
            .unwrap();

        assert_eq!(
            outcomes,
            vec![UsageIngestionOutcome {
                inserted: false,
                link_created: false,
            }]
        );
        assert_eq!(
            db.get_usage_sync_cursor("claude", "session.jsonl").unwrap(),
            Some(next_cursor)
        );
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

    fn save_subscription_provider(db: &Database, id: &str) {
        db.save_usage_provider(&UsageProviderInput {
            id: id.to_string(),
            name: id.to_string(),
            billing_kind: BillingKind::Subscription,
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
    }

    /// The user's own purchase price outranks the upstream's self-reported
    /// cost: a relay reports its list price, not the rate this account pays.
    #[test]
    fn provider_custom_pricing_outranks_upstream_cost_and_official_pricing() {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", None);
        seed_pricing(&db, "priced-model");
        db.upsert_provider_model_pricing(
            "global-provider",
            "priced-model",
            "Priced Model",
            &price("0.5", "1", "0", "0"),
        )
        .unwrap();
        let service = UsageIngestionService::new(&db);

        let mut value = input("custom-priced", TokenSource::Proxy);
        value.upstream_cost = Some(UpstreamCost {
            input_cost: Some(Decimal::from_str("9").unwrap()),
            output_cost: None,
            cache_read_cost: None,
            cache_creation_cost: None,
            total_cost: Some(Decimal::from_str("99").unwrap()),
        });
        service.ingest(&value).unwrap();

        let page = db
            .list_usage_events("global-provider", 0, 200, 1, 10)
            .unwrap();
        let event = &page.items[0];
        assert_eq!(event.cost_source, CostSource::Estimated);
        assert_eq!(event.pricing_origin, Some(PricingOrigin::User));
        // 1M input @ 0.5 + 1M output @ 1, not the official 1/2 and not the
        // upstream-reported 99.
        assert_eq!(event.input_cost_usd.as_deref(), Some("0.5"));
        assert_eq!(event.output_cost_usd.as_deref(), Some("1"));
        assert_eq!(event.total_cost_usd.as_deref(), Some("1.5"));
    }

    #[test]
    fn custom_pricing_for_another_account_does_not_leak_across_providers() {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", None);
        save_provider(&db, "other-provider", None);
        seed_pricing(&db, "priced-model");
        db.upsert_provider_model_pricing(
            "other-provider",
            "priced-model",
            "Priced Model",
            &price("0.5", "1", "0", "0"),
        )
        .unwrap();

        UsageIngestionService::new(&db)
            .ingest(&input("official-priced", TokenSource::Proxy))
            .unwrap();

        let page = db
            .list_usage_events("global-provider", 0, 200, 1, 10)
            .unwrap();
        let event = &page.items[0];
        assert_eq!(event.pricing_origin, Some(PricingOrigin::Official));
        assert_eq!(event.total_cost_usd.as_deref(), Some("3"));
    }

    /// Subscription spend is a synthetic equivalent-API figure, so it is always
    /// valued at the official list price.
    #[test]
    fn subscription_events_always_use_official_pricing() {
        let db = Database::memory().unwrap();
        save_subscription_provider(&db, "claude-max");
        seed_pricing(&db, "priced-model");
        // A stray custom price must not reach a subscription account even if a
        // row exists for it.
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO provider_model_pricing (
                    provider_id, model_id, display_name,
                    input_cost_per_million, output_cost_per_million,
                    cache_read_cost_per_million, cache_creation_cost_per_million,
                    created_at, updated_at
                 ) VALUES ('claude-max', 'priced-model', '', '0.01', '0.02', '0', '0', 1, 1)",
                [],
            )
            .unwrap();
        }

        let mut value = input("subscription-event", TokenSource::SessionLog);
        value.provider_id = "claude-max".to_string();
        value.upstream_cost = Some(UpstreamCost {
            input_cost: None,
            output_cost: None,
            cache_read_cost: None,
            cache_creation_cost: None,
            total_cost: Some(Decimal::from_str("77").unwrap()),
        });
        UsageIngestionService::new(&db).ingest(&value).unwrap();

        let page = db.list_usage_events("claude-max", 0, 200, 1, 10).unwrap();
        let event = &page.items[0];
        assert_eq!(event.cost_source, CostSource::Estimated);
        assert_eq!(event.pricing_origin, Some(PricingOrigin::Official));
        assert_eq!(event.total_cost_usd.as_deref(), Some("3"));
    }

    /// Editing a price is not retroactive: stored events keep the price that
    /// applied when they were recorded.
    #[test]
    fn changing_a_price_leaves_already_stored_events_untouched() {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", None);
        seed_pricing(&db, "priced-model");
        let service = UsageIngestionService::new(&db);

        service
            .ingest(&input("before", TokenSource::Proxy))
            .unwrap();
        db.upsert_provider_model_pricing(
            "global-provider",
            "priced-model",
            "Priced Model",
            &price("0.5", "1", "0", "0"),
        )
        .unwrap();
        service.ingest(&input("after", TokenSource::Proxy)).unwrap();

        let page = db
            .list_usage_events("global-provider", 0, 200, 1, 10)
            .unwrap();
        let before = page
            .items
            .iter()
            .find(|event| event.event_id == "before")
            .unwrap();
        let after = page
            .items
            .iter()
            .find(|event| event.event_id == "after")
            .unwrap();
        assert_eq!(before.total_cost_usd.as_deref(), Some("3"));
        assert_eq!(before.pricing_origin, Some(PricingOrigin::Official));
        assert_eq!(after.total_cost_usd.as_deref(), Some("1.5"));
        assert_eq!(after.pricing_origin, Some(PricingOrigin::User));
    }

    /// A custom price entered for the undated family id must cover the dated
    /// model names that actually appear in session logs, exactly like the
    /// official catalogue does.
    #[test]
    fn custom_pricing_matches_dated_model_variants() {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", None);
        db.upsert_provider_model_pricing(
            "global-provider",
            "claude-sonnet-5",
            "Claude Sonnet 5",
            &price("1.5", "7.5", "0", "0"),
        )
        .unwrap();

        let mut value = input("dated", TokenSource::Proxy);
        value.model = "claude-sonnet-5-20260514".to_string();
        UsageIngestionService::new(&db).ingest(&value).unwrap();

        let page = db
            .list_usage_events("global-provider", 0, 200, 1, 10)
            .unwrap();
        let event = &page.items[0];
        assert_eq!(event.pricing_origin, Some(PricingOrigin::User));
        assert_eq!(event.total_cost_usd.as_deref(), Some("9.0"));
    }

    /// Removing the custom price falls back to upstream evidence first.
    #[test]
    fn deleting_custom_pricing_restores_the_upstream_cost_priority() {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", None);
        seed_pricing(&db, "priced-model");
        db.upsert_provider_model_pricing(
            "global-provider",
            "priced-model",
            "Priced Model",
            &price("0.5", "1", "0", "0"),
        )
        .unwrap();
        db.delete_provider_model_pricing("global-provider", "priced-model")
            .unwrap();

        let mut value = input("fallback", TokenSource::Proxy);
        value.upstream_cost = Some(UpstreamCost {
            input_cost: None,
            output_cost: None,
            cache_read_cost: None,
            cache_creation_cost: None,
            total_cost: Some(Decimal::from_str("0.25").unwrap()),
        });
        UsageIngestionService::new(&db).ingest(&value).unwrap();

        let page = db
            .list_usage_events("global-provider", 0, 200, 1, 10)
            .unwrap();
        let event = &page.items[0];
        assert_eq!(event.cost_source, CostSource::Upstream);
        assert_eq!(event.pricing_origin, None);
        assert_eq!(event.total_cost_usd.as_deref(), Some("0.25"));
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
    fn duplicate_event_id_rejects_conflicting_provider_ownership() {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", None);
        save_provider(&db, "other-provider", None);
        let service = UsageIngestionService::new(&db);
        let original = input("same-event-owner", TokenSource::Proxy);
        assert!(service.ingest(&original).unwrap().inserted);

        let mut conflicting = original;
        conflicting.provider_id = "other-provider".to_string();
        let error = service.ingest(&conflicting).unwrap_err();

        assert_eq!(error.to_string(), "usage_event_ownership_conflict");
    }

    #[test]
    fn proxy_ingestion_persists_frozen_agent_ownership() {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", None);
        let service = UsageIngestionService::new(&db);
        let mut owned = input("agent-owned-proxy", TokenSource::Proxy);
        owned.agent_module_id = "codex".to_string();

        assert!(service.ingest(&owned).unwrap().inserted);
        let stored = db
            .list_usage_events("global-provider", 0, 200, 1, 10)
            .unwrap()
            .items
            .pop()
            .unwrap();

        assert_eq!(stored.agent_module_id.as_deref(), Some("codex"));
    }

    #[test]
    fn proxy_agent_ownership_requires_a_frozen_provider_context() {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", None);
        let mut owned = input("missing-frozen-context", TokenSource::Proxy);
        owned.agent_module_id = "codex".to_string();
        owned.frozen_provider_context = None;

        let error = UsageIngestionService::new(&db).ingest(&owned).unwrap_err();
        assert_eq!(error.to_string(), "invalid frozen usage provider context");
    }

    #[test]
    fn duplicate_event_id_rejects_conflicting_agent_ownership() {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", None);
        let service = UsageIngestionService::new(&db);
        let mut original = input("same-agent-event-owner", TokenSource::Proxy);
        original.agent_module_id = "codex".to_string();
        assert!(service.ingest(&original).unwrap().inserted);

        let mut conflicting = original;
        conflicting.agent_module_id = "claude-code".to_string();
        let error = service.ingest(&conflicting).unwrap_err();

        assert_eq!(error.to_string(), "usage_event_ownership_conflict");
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
        seed_session_binding(&db, "claude-code", "global-provider", true);
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
        seed_session_binding(&db, "claude-code", "global-provider", true);
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

    #[test]
    fn cross_source_matching_rejects_a_different_agent_on_the_same_provider() {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", None);
        seed_session_binding(&db, "codex", "global-provider", true);
        seed_session_binding(&db, "claude-code", "global-provider", true);
        let service = UsageIngestionService::new(&db);

        let mut proxy = attributed_input("proxy-codex", TokenSource::Proxy, "codex");
        proxy.request_id = Some("shared-request".to_string());
        assert!(service.ingest(&proxy).unwrap().inserted);

        let mut session =
            attributed_input("session-claude", TokenSource::SessionLog, "claude-code");
        session.request_id = Some("shared-request".to_string());
        let outcome = service.ingest(&session).unwrap();

        assert!(outcome.inserted);
        assert!(!outcome.link_created);
        let conn = db.conn.lock().unwrap();
        let link_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM usage_event_links", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(link_count, 0);
    }

    fn assert_proxy_is_canonical_for_arrival_order(session_first: bool) {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", None);
        seed_session_binding(&db, "claude-code", "global-provider", true);
        let service = UsageIngestionService::new(&db);

        let mut proxy = attributed_input("proxy-owned", TokenSource::Proxy, "claude-code");
        proxy.request_id = Some("same-request".to_string());
        proxy.upstream_cost = Some(UpstreamCost {
            input_cost: Some(Decimal::from_str("0.10").unwrap()),
            output_cost: Some(Decimal::from_str("0.20").unwrap()),
            cache_read_cost: None,
            cache_creation_cost: None,
            total_cost: Some(Decimal::from_str("0.30").unwrap()),
        });
        let mut session = attributed_input("session-owned", TokenSource::SessionLog, "claude-code");
        session.request_id = Some("same-request".to_string());

        if session_first {
            assert!(service.ingest(&session).unwrap().inserted);
            assert!(service.ingest(&proxy).unwrap().link_created);
        } else {
            assert!(service.ingest(&proxy).unwrap().inserted);
            assert!(service.ingest(&session).unwrap().link_created);
        }

        let conn = db.conn.lock().unwrap();
        let link: (String, String) = conn
            .query_row(
                "SELECT canonical_event_id, duplicate_event_id FROM usage_event_links",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            link,
            ("proxy-owned".to_string(), "session-owned".to_string())
        );
        let costs: Vec<(String, String, Option<String>)> = {
            let mut statement = conn
                .prepare(
                    "SELECT event_id, cost_source, total_cost_usd
                     FROM usage_events ORDER BY event_id",
                )
                .unwrap();
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(costs.iter().any(|(event_id, cost_source, total)| {
            event_id == "proxy-owned"
                && cost_source == "upstream"
                && total.as_deref() == Some("0.30")
        }));
        assert!(costs
            .iter()
            .any(|(event_id, _, _)| event_id == "session-owned"));
    }

    #[test]
    fn proxy_with_upstream_cost_is_canonical_when_proxy_arrives_first() {
        assert_proxy_is_canonical_for_arrival_order(false);
    }

    #[test]
    fn proxy_with_upstream_cost_is_canonical_when_session_arrives_first() {
        assert_proxy_is_canonical_for_arrival_order(true);
    }

    #[test]
    fn session_ingestion_does_not_require_an_agent_provider_binding() {
        let db = Database::memory().unwrap();
        save_provider(&db, "global-provider", None);
        let service = UsageIngestionService::new(&db);

        let missing = attributed_input("missing-binding", TokenSource::SessionLog, "claude-code");
        assert!(service.ingest(&missing).unwrap().inserted);

        seed_session_binding(&db, "claude-code", "global-provider", false);
        let disabled = attributed_input("disabled-binding", TokenSource::SessionLog, "claude-code");
        assert!(service.ingest(&disabled).unwrap().inserted);
    }
}
