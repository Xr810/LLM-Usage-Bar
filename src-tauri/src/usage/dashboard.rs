use crate::database::Database;
use crate::error::AppError;
use crate::usage::domain::{
    BillingKind, CostSourceCounts, ProductUsageView, ProviderUsageView, QuotaStatusView,
    TokenSource, UsageDashboardView, UsageProviderView,
};
use rusqlite::params;
use rust_decimal::Decimal;
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

pub struct UsageDashboardService<'a> {
    db: &'a Database,
}

#[derive(Default)]
struct ProductAccumulator {
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    total_cost: Decimal,
    has_cost: bool,
    cost_source_counts: CostSourceCounts,
    token_sources: BTreeSet<&'static str>,
    subscription_providers: Vec<ProviderUsageView>,
    metered_providers: Vec<ProviderUsageView>,
}

impl<'a> UsageDashboardService<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn get_dashboard(
        &self,
        start_at: i64,
        end_at: i64,
        product_group_id: Option<&str>,
    ) -> Result<UsageDashboardView, AppError> {
        if start_at >= end_at {
            return Err(AppError::Message(
                "start_at must be before end_at".to_string(),
            ));
        }

        let mut products: BTreeMap<String, ProductAccumulator> = BTreeMap::new();
        for provider in self.db.list_usage_providers()? {
            let mut groups = self.event_product_groups(&provider.id, start_at, end_at)?;
            groups.insert(provider.product_group_id.clone());
            for group_id in groups {
                if product_group_id.is_some_and(|filter| filter != group_id) {
                    continue;
                }
                let is_current_group = group_id == provider.product_group_id;
                let usage = self.aggregate_provider(
                    &provider,
                    &group_id,
                    start_at,
                    end_at,
                    is_current_group,
                )?;
                if usage.event_count == 0 && !is_current_group {
                    continue;
                }

                let product = products.entry(group_id).or_default();
                product.input_tokens = checked_sum(product.input_tokens, usage.input_tokens)?;
                product.output_tokens = checked_sum(product.output_tokens, usage.output_tokens)?;
                product.cache_read_tokens =
                    checked_sum(product.cache_read_tokens, usage.cache_read_tokens)?;
                product.cache_creation_tokens =
                    checked_sum(product.cache_creation_tokens, usage.cache_creation_tokens)?;
                if let Some(cost) = usage.total_cost_usd.as_deref() {
                    product.total_cost =
                        checked_cost_sum(product.total_cost, parse_decimal(cost)?)?;
                    product.has_cost = true;
                }
                add_cost_counts(&mut product.cost_source_counts, &usage.cost_source_counts)?;
                for source in &provider.token_sources {
                    product.token_sources.insert(match source {
                        TokenSource::Proxy => "proxy",
                        TokenSource::SessionLog => "session_log",
                    });
                }
                match provider.billing_kind {
                    BillingKind::Subscription => product.subscription_providers.push(usage),
                    BillingKind::Metered => product.metered_providers.push(usage),
                }
            }
        }

        let product_groups = products
            .into_iter()
            .map(|(product_group_id, mut product)| {
                product
                    .subscription_providers
                    .sort_by(|left, right| left.provider.id.cmp(&right.provider.id));
                product
                    .metered_providers
                    .sort_by(|left, right| left.provider.id.cmp(&right.provider.id));
                ProductUsageView {
                    product_group_id,
                    input_tokens: product.input_tokens,
                    output_tokens: product.output_tokens,
                    cache_read_tokens: product.cache_read_tokens,
                    cache_creation_tokens: product.cache_creation_tokens,
                    total_cost_usd: product
                        .has_cost
                        .then(|| product.total_cost.normalize().to_string()),
                    cost_source_counts: product.cost_source_counts,
                    token_sources: product
                        .token_sources
                        .into_iter()
                        .map(|source| match source {
                            "proxy" => TokenSource::Proxy,
                            _ => TokenSource::SessionLog,
                        })
                        .collect(),
                    subscription_providers: product.subscription_providers,
                    metered_providers: product.metered_providers,
                }
            })
            .collect();

        Ok(UsageDashboardView {
            start_at,
            end_at,
            product_groups,
            warnings: vec![],
        })
    }

    fn aggregate_provider(
        &self,
        provider: &UsageProviderView,
        product_group_id: &str,
        start_at: i64,
        end_at: i64,
        include_quota: bool,
    ) -> Result<ProviderUsageView, AppError> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|error| AppError::Database(format!("Mutex lock failed: {error}")))?;
        let mut statement = conn.prepare(
            "SELECT input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    total_cost_usd, cost_source
             FROM usage_events AS event
             WHERE event.provider_id = ?1
               AND event.product_group_id = ?2
               AND event.occurred_at >= ?3 AND event.occurred_at < ?4
               AND NOT EXISTS (
                   SELECT 1 FROM usage_event_links AS link
                   WHERE link.duplicate_event_id = event.event_id
               )
             ORDER BY event.occurred_at, event.event_id",
        )?;
        let rows = statement.query_map(
            params![provider.id, product_group_id, start_at, end_at],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )?;

        let mut event_count = 0_u64;
        let mut input_tokens = 0_u64;
        let mut output_tokens = 0_u64;
        let mut cache_read_tokens = 0_u64;
        let mut cache_creation_tokens = 0_u64;
        let mut total_cost = Decimal::ZERO;
        let mut has_cost = false;
        let mut cost_source_counts = CostSourceCounts::default();
        for row in rows {
            let (input, output, cache_read, cache_creation, cost, source) = row?;
            event_count = checked_sum(event_count, 1)?;
            input_tokens = checked_sum(input_tokens, non_negative(input, "input_tokens")?)?;
            output_tokens = checked_sum(output_tokens, non_negative(output, "output_tokens")?)?;
            cache_read_tokens = checked_sum(
                cache_read_tokens,
                non_negative(cache_read, "cache_read_tokens")?,
            )?;
            cache_creation_tokens = checked_sum(
                cache_creation_tokens,
                non_negative(cache_creation, "cache_creation_tokens")?,
            )?;
            if let Some(cost) = cost {
                total_cost = checked_cost_sum(total_cost, parse_decimal(&cost)?)?;
                has_cost = true;
            }
            match source.as_str() {
                "upstream" => cost_source_counts.upstream += 1,
                "estimated" => cost_source_counts.estimated += 1,
                "unavailable" => cost_source_counts.unavailable += 1,
                _ => {
                    return Err(AppError::Database(format!(
                        "invalid usage cost source: {source}"
                    )))
                }
            }
        }
        drop(statement);
        drop(conn);

        let (quota, quota_fetch_state) =
            if provider.billing_kind == BillingKind::Subscription && include_quota {
                let (snapshot, fetch_state) = self.db.latest_quota_status(&provider.id)?;
                let quota = snapshot.map(|snapshot| QuotaStatusView {
                    snapshot_id: snapshot.snapshot_id,
                    fetched_at: snapshot.fetched_at,
                    five_hour_utilization_percent: snapshot.five_hour_utilization_percent,
                    five_hour_resets_at: snapshot.five_hour_resets_at,
                    seven_day_utilization_percent: snapshot.seven_day_utilization_percent,
                    seven_day_resets_at: snapshot.seven_day_resets_at,
                    manual_resets_remaining: snapshot.manual_resets_remaining,
                });
                (quota, fetch_state)
            } else {
                (None, None)
            };

        Ok(ProviderUsageView {
            provider: provider.clone(),
            event_count,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
            total_cost_usd: has_cost.then(|| total_cost.normalize().to_string()),
            cost_source_counts,
            quota,
            quota_fetch_state,
        })
    }

    fn event_product_groups(
        &self,
        provider_id: &str,
        start_at: i64,
        end_at: i64,
    ) -> Result<BTreeSet<String>, AppError> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|error| AppError::Database(format!("Mutex lock failed: {error}")))?;
        let mut statement = conn.prepare(
            "SELECT DISTINCT event.product_group_id
             FROM usage_events AS event
             WHERE event.provider_id = ?1
               AND event.occurred_at >= ?2 AND event.occurred_at < ?3
               AND NOT EXISTS (
                   SELECT 1 FROM usage_event_links AS link
                   WHERE link.duplicate_event_id = event.event_id
               )
             ORDER BY event.product_group_id",
        )?;
        let groups = statement
            .query_map(params![provider_id, start_at, end_at], |row| row.get(0))?
            .collect::<Result<BTreeSet<_>, _>>()?;
        Ok(groups)
    }
}

fn non_negative(value: i64, field: &str) -> Result<u64, AppError> {
    u64::try_from(value).map_err(|_| AppError::Database(format!("negative usage value in {field}")))
}

fn checked_sum(left: u64, right: u64) -> Result<u64, AppError> {
    left.checked_add(right)
        .ok_or_else(|| AppError::Database("usage aggregate overflow".to_string()))
}

fn parse_decimal(value: &str) -> Result<Decimal, AppError> {
    Decimal::from_str(value)
        .map_err(|error| AppError::Database(format!("invalid usage cost {value}: {error}")))
}

fn checked_cost_sum(left: Decimal, right: Decimal) -> Result<Decimal, AppError> {
    left.checked_add(right)
        .ok_or_else(|| AppError::Database("usage cost aggregate overflow".to_string()))
}

fn add_cost_counts(
    target: &mut CostSourceCounts,
    source: &CostSourceCounts,
) -> Result<(), AppError> {
    target.upstream = checked_sum(target.upstream, source.upstream)?;
    target.estimated = checked_sum(target.estimated, source.estimated)?;
    target.unavailable = checked_sum(target.unavailable, source.unavailable)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::database::Database;
    use crate::usage::dashboard::UsageDashboardService;
    use crate::usage::domain::{
        BillingKind, CostSource, QuotaSnapshot, TokenSource, UsageEvent, UsageEventLink,
        UsageProviderInput,
    };
    use rust_decimal::Decimal;
    use serde_json::json;

    fn provider(id: &str, billing_kind: BillingKind, product_group_id: &str) -> UsageProviderInput {
        UsageProviderInput {
            id: id.to_string(),
            name: id.to_string(),
            billing_kind,
            product_group_id: product_group_id.to_string(),
            token_sources: vec![TokenSource::Proxy, TokenSource::SessionLog],
            session_source_bindings: None,
            quota_source: (billing_kind == BillingKind::Subscription).then(|| "claude".to_string()),
            quota_interval_seconds: Some(300),
            route_app_type: None,
            route_config: None,
            quota_config: None,
            enabled: true,
        }
    }

    fn event(
        event_id: &str,
        provider_id: &str,
        product_group_id: &str,
        source: TokenSource,
        cost_source: CostSource,
        occurred_at: i64,
        total_cost_usd: Option<&str>,
    ) -> UsageEvent {
        UsageEvent {
            event_id: event_id.to_string(),
            source,
            provider_id: provider_id.to_string(),
            agent_module_id: None,
            product_group_id: product_group_id.to_string(),
            occurred_at,
            model: "model".to_string(),
            input_tokens: 10,
            output_tokens: 2,
            cache_read_tokens: 3,
            cache_creation_tokens: 4,
            request_id: None,
            session_id: None,
            upstream_correlation_id: None,
            input_cost_usd: None,
            output_cost_usd: None,
            cache_read_cost_usd: None,
            cache_creation_cost_usd: None,
            total_cost_usd: total_cost_usd.map(str::to_string),
            cost_source,
            legacy_request_id: None,
            created_at: occurred_at,
        }
    }

    #[test]
    fn aggregation_separates_billing_excludes_only_linked_duplicates_and_attaches_subscription_quota(
    ) {
        let db = Database::memory().unwrap();
        db.save_usage_provider(&provider("sub", BillingKind::Subscription, "product"))
            .unwrap();
        db.save_usage_provider(&provider("metered", BillingKind::Metered, "product"))
            .unwrap();

        let mut proxy = event(
            "proxy",
            "sub",
            "product",
            TokenSource::Proxy,
            CostSource::Upstream,
            100,
            Some("0.10"),
        );
        proxy.request_id = Some("stable".to_string());
        let mut linked = event(
            "linked-session",
            "sub",
            "product",
            TokenSource::SessionLog,
            CostSource::Estimated,
            101,
            Some("0.20"),
        );
        linked.request_id = Some("stable".to_string());
        let similar_a = event(
            "similar-a",
            "metered",
            "product",
            TokenSource::Proxy,
            CostSource::Estimated,
            102,
            Some("0.30"),
        );
        let similar_b = event(
            "similar-b",
            "metered",
            "product",
            TokenSource::SessionLog,
            CostSource::Unavailable,
            102,
            None,
        );
        for value in [&proxy, &linked, &similar_a, &similar_b] {
            db.insert_usage_event(value).unwrap();
        }
        db.insert_usage_event_link(&UsageEventLink {
            canonical_event_id: "proxy".to_string(),
            duplicate_event_id: "linked-session".to_string(),
            link_kind: "request_id".to_string(),
            link_value: "stable".to_string(),
            created_at: 103,
        })
        .unwrap();
        db.append_quota_success(&QuotaSnapshot {
            snapshot_id: "quota".to_string(),
            provider_id: "sub".to_string(),
            fetched_at: 120,
            five_hour_utilization_percent: Some("25".to_string()),
            five_hour_resets_at: None,
            seven_day_utilization_percent: Some("50".to_string()),
            seven_day_resets_at: None,
            manual_resets_remaining: Some(2),
            raw_payload: json!({"secret": "must-not-leak"}),
            created_at: 120,
        })
        .unwrap();

        let dashboard = UsageDashboardService::new(&db)
            .get_dashboard(100, 200, None)
            .unwrap();
        assert_eq!(dashboard.product_groups.len(), 1);
        let product = &dashboard.product_groups[0];
        assert_eq!(product.subscription_providers.len(), 1);
        assert_eq!(product.metered_providers.len(), 1);
        assert_eq!(
            product.input_tokens, 30,
            "linked duplicate only is excluded"
        );
        assert_eq!(product.output_tokens, 6);
        assert_eq!(product.total_cost_usd.as_deref(), Some("0.4"));

        let subscription = &product.subscription_providers[0];
        assert_eq!(subscription.event_count, 1);
        assert_eq!(subscription.cost_source_counts.upstream, 1);
        assert_eq!(subscription.cost_source_counts.estimated, 0);
        assert!(subscription.quota.is_some());
        assert!(subscription.quota_fetch_state.is_some());
        assert!(!serde_json::to_string(subscription)
            .unwrap()
            .contains("must-not-leak"));

        let metered = &product.metered_providers[0];
        assert_eq!(
            metered.event_count, 2,
            "similar events without IDs both count"
        );
        assert_eq!(metered.cost_source_counts.estimated, 1);
        assert_eq!(metered.cost_source_counts.unavailable, 1);
        assert!(metered.quota.is_none());
        assert!(metered.quota_fetch_state.is_none());

        let detail = db.list_usage_events("sub", 100, 200, 1, 10).unwrap();
        assert_eq!(detail.total, 2, "duplicate remains queryable in details");
    }

    #[test]
    fn dashboard_range_and_product_filter_are_exact() {
        let db = Database::memory().unwrap();
        db.save_usage_provider(&provider("a", BillingKind::Metered, "one"))
            .unwrap();
        db.save_usage_provider(&provider("b", BillingKind::Metered, "two"))
            .unwrap();
        for (id, provider_id, product, at) in [
            ("before", "a", "one", 9),
            ("start", "a", "one", 10),
            ("end", "a", "one", 20),
            ("other", "b", "two", 15),
        ] {
            db.insert_usage_event(&event(
                id,
                provider_id,
                product,
                TokenSource::Proxy,
                CostSource::Unavailable,
                at,
                None,
            ))
            .unwrap();
        }

        let dashboard = UsageDashboardService::new(&db)
            .get_dashboard(10, 20, Some("one"))
            .unwrap();
        assert_eq!(dashboard.product_groups.len(), 1);
        assert_eq!(dashboard.product_groups[0].product_group_id, "one");
        assert_eq!(dashboard.product_groups[0].input_tokens, 10);
        assert!(UsageDashboardService::new(&db)
            .get_dashboard(20, 20, None)
            .unwrap_err()
            .to_string()
            .contains("start_at must be before end_at"));
    }

    #[test]
    fn subscription_fetch_failure_is_visible_without_a_successful_snapshot() {
        let db = Database::memory().unwrap();
        db.save_usage_provider(&provider("sub", BillingKind::Subscription, "product"))
            .unwrap();
        db.record_quota_failure("sub", 50, "credentials unavailable")
            .unwrap();

        let dashboard = UsageDashboardService::new(&db)
            .get_dashboard(0, 100, None)
            .unwrap();
        let subscription = &dashboard.product_groups[0].subscription_providers[0];
        assert!(subscription.quota.is_none());
        let fetch_state = subscription.quota_fetch_state.as_ref().unwrap();
        assert_eq!(fetch_state.last_attempt_at, Some(50));
        assert_eq!(
            fetch_state.last_error.as_deref(),
            Some("credentials unavailable")
        );
    }

    #[test]
    fn immutable_event_product_group_survives_provider_reclassification() {
        let db = Database::memory().unwrap();
        db.save_usage_provider(&provider("metered", BillingKind::Metered, "old-product"))
            .unwrap();
        db.insert_usage_event(&event(
            "historical",
            "metered",
            "old-product",
            TokenSource::Proxy,
            CostSource::Upstream,
            50,
            Some("0.5"),
        ))
        .unwrap();
        db.save_usage_provider(&provider("metered", BillingKind::Metered, "new-product"))
            .unwrap();

        let old = UsageDashboardService::new(&db)
            .get_dashboard(0, 100, Some("old-product"))
            .unwrap();
        assert_eq!(old.product_groups.len(), 1);
        assert_eq!(old.product_groups[0].input_tokens, 10);
        assert_eq!(old.product_groups[0].metered_providers[0].event_count, 1);

        let new = UsageDashboardService::new(&db)
            .get_dashboard(0, 100, Some("new-product"))
            .unwrap();
        assert_eq!(new.product_groups.len(), 1);
        assert_eq!(new.product_groups[0].input_tokens, 0);
        assert_eq!(new.product_groups[0].metered_providers[0].event_count, 0);
    }

    #[test]
    fn provider_cost_overflow_returns_an_error_instead_of_panicking() {
        let db = Database::memory().unwrap();
        db.save_usage_provider(&provider("metered", BillingKind::Metered, "product"))
            .unwrap();
        let maximum = Decimal::MAX.to_string();
        for (id, cost) in [("maximum", maximum.as_str()), ("one", "1")] {
            db.insert_usage_event(&event(
                id,
                "metered",
                "product",
                TokenSource::Proxy,
                CostSource::Upstream,
                50,
                Some(cost),
            ))
            .unwrap();
        }

        let error = UsageDashboardService::new(&db)
            .get_dashboard(0, 100, None)
            .unwrap_err();
        assert!(error.to_string().contains("usage cost aggregate overflow"));
    }

    #[test]
    fn product_cost_overflow_returns_an_error_instead_of_panicking() {
        let db = Database::memory().unwrap();
        for id in ["first", "second"] {
            db.save_usage_provider(&provider(id, BillingKind::Metered, "product"))
                .unwrap();
        }
        let maximum = Decimal::MAX.to_string();
        db.insert_usage_event(&event(
            "maximum",
            "first",
            "product",
            TokenSource::Proxy,
            CostSource::Upstream,
            50,
            Some(&maximum),
        ))
        .unwrap();
        db.insert_usage_event(&event(
            "one",
            "second",
            "product",
            TokenSource::Proxy,
            CostSource::Upstream,
            50,
            Some("1"),
        ))
        .unwrap();

        let error = UsageDashboardService::new(&db)
            .get_dashboard(0, 100, None)
            .unwrap_err();
        assert!(error.to_string().contains("usage cost aggregate overflow"));
    }
}
