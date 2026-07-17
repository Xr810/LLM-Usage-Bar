use crate::database::Database;
use crate::error::AppError;
use crate::usage::aggregation::{aggregate_provider_account_range, aggregate_provider_range};
use crate::usage::domain::{
    BillingKind, CostSourceCounts, ProductUsageView, ProviderMonitoringDashboardView,
    ProviderUsageView, QuotaStatusView, TokenSource, UsageDashboardView, UsageProviderView,
};
use rusqlite::params;
use rust_decimal::Decimal;
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

pub struct UsageDashboardService<'a> {
    db: &'a Database,
    shared_provider_ids: BTreeSet<String>,
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
        Self {
            db,
            shared_provider_ids: BTreeSet::new(),
        }
    }

    pub fn with_shared_provider_ids(
        mut self,
        provider_ids: impl IntoIterator<Item = String>,
    ) -> Self {
        self.shared_provider_ids = provider_ids.into_iter().collect();
        self
    }

    pub fn get_provider_dashboard(
        &self,
        start_at: i64,
        end_at: i64,
    ) -> Result<ProviderMonitoringDashboardView, AppError> {
        if start_at >= end_at {
            return Err(AppError::Message(
                "start_at must be before end_at".to_string(),
            ));
        }

        let mut provider_ids = self.event_provider_ids_all(start_at, end_at)?;
        let providers = self
            .db
            .list_usage_providers()?
            .into_iter()
            .map(|provider| (provider.id.clone(), provider))
            .collect::<BTreeMap<_, _>>();
        provider_ids.extend(
            providers
                .values()
                .filter(|provider| provider.enabled)
                .map(|provider| provider.id.clone()),
        );

        let mut rows = Vec::with_capacity(provider_ids.len());
        for provider_id in provider_ids {
            let Some(provider) = providers.get(&provider_id) else {
                continue;
            };
            let aggregate =
                aggregate_provider_account_range(self.db, &provider.id, start_at, end_at)?;
            let (snapshot, quota_fetch_state) = self.db.latest_quota_status(&provider.id)?;
            let quota = snapshot.map(|snapshot| QuotaStatusView {
                snapshot_id: snapshot.snapshot_id,
                fetched_at: snapshot.fetched_at,
                five_hour_utilization_percent: snapshot.five_hour_utilization_percent,
                five_hour_resets_at: snapshot.five_hour_resets_at,
                seven_day_utilization_percent: snapshot.seven_day_utilization_percent,
                seven_day_resets_at: snapshot.seven_day_resets_at,
                manual_resets_remaining: snapshot.manual_resets_remaining,
            });
            rows.push(ProviderUsageView {
                provider: provider.clone(),
                shared_account: false,
                event_count: aggregate.event_count,
                input_tokens: aggregate.input_tokens,
                output_tokens: aggregate.output_tokens,
                cache_read_tokens: aggregate.cache_read_tokens,
                cache_creation_tokens: aggregate.cache_creation_tokens,
                total_cost_usd: aggregate.total_cost_usd,
                cost_source_counts: aggregate.cost_source_counts,
                quota,
                quota_fetch_state,
            });
        }
        rows.sort_by(|left, right| {
            (left.provider.name.to_lowercase(), left.provider.id.as_str()).cmp(&(
                right.provider.name.to_lowercase(),
                right.provider.id.as_str(),
            ))
        });

        Ok(ProviderMonitoringDashboardView {
            start_at,
            end_at,
            providers: rows,
            warnings: vec![],
        })
    }

    fn event_provider_ids_all(
        &self,
        start_at: i64,
        end_at: i64,
    ) -> Result<BTreeSet<String>, AppError> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|error| AppError::Database(format!("Mutex lock failed: {error}")))?;
        let mut statement = conn.prepare(
            "SELECT DISTINCT provider_id
             FROM usage_events
             WHERE occurred_at >= ?1 AND occurred_at < ?2
             ORDER BY provider_id",
        )?;
        let provider_ids = statement
            .query_map(params![start_at, end_at], |row| row.get(0))?
            .collect::<Result<BTreeSet<_>, _>>()?;
        Ok(provider_ids)
    }

    pub fn get_dashboard(
        &self,
        start_at: i64,
        end_at: i64,
        agent_module_id: &str,
    ) -> Result<UsageDashboardView, AppError> {
        if start_at >= end_at {
            return Err(AppError::Message(
                "start_at must be before end_at".to_string(),
            ));
        }
        let agent = self
            .db
            .get_agent_module_including_archived(agent_module_id)?
            .filter(|agent| agent.archived_at.is_none())
            .ok_or_else(|| AppError::Message("invalid_agent_module".to_string()))?;
        debug_assert_eq!(agent.id, agent_module_id);

        let providers = self
            .db
            .list_usage_providers()?
            .into_iter()
            .map(|provider| (provider.id.clone(), provider))
            .collect::<BTreeMap<_, _>>();
        let mut active_provider_ids = BTreeSet::new();
        let mut provider_ids = self.event_provider_ids(agent_module_id, start_at, end_at)?;
        for binding in self
            .db
            .list_agent_provider_bindings(Some(agent_module_id))?
        {
            if binding.enabled
                && providers
                    .get(&binding.provider_id)
                    .is_some_and(|provider| provider.enabled)
            {
                active_provider_ids.insert(binding.provider_id.clone());
                provider_ids.insert(binding.provider_id);
            }
        }

        let mut products: BTreeMap<String, ProductAccumulator> = BTreeMap::new();
        for provider_id in std::mem::take(&mut provider_ids) {
            let Some(provider) = providers.get(&provider_id) else {
                continue;
            };
            let mut groups =
                self.event_product_groups(&provider.id, agent_module_id, start_at, end_at)?;
            if active_provider_ids.contains(&provider.id) {
                groups.insert(provider.product_group_id.clone());
            }
            let quota_group_id = if groups.contains(&provider.product_group_id) {
                Some(provider.product_group_id.clone())
            } else {
                groups.first().cloned()
            };
            for group_id in groups {
                let is_current_group = group_id == provider.product_group_id;
                let usage = self.aggregate_provider(
                    provider,
                    &group_id,
                    agent_module_id,
                    start_at,
                    end_at,
                    quota_group_id.as_deref() == Some(group_id.as_str()),
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
            agent_module_id: agent_module_id.to_string(),
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
        agent_module_id: &str,
        start_at: i64,
        end_at: i64,
        include_quota: bool,
    ) -> Result<ProviderUsageView, AppError> {
        let aggregate = aggregate_provider_range(
            self.db,
            agent_module_id,
            &provider.id,
            product_group_id,
            start_at,
            end_at,
        )?;

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
            shared_account: self.shared_provider_ids.contains(&provider.id),
            event_count: aggregate.event_count,
            input_tokens: aggregate.input_tokens,
            output_tokens: aggregate.output_tokens,
            cache_read_tokens: aggregate.cache_read_tokens,
            cache_creation_tokens: aggregate.cache_creation_tokens,
            total_cost_usd: aggregate.total_cost_usd,
            cost_source_counts: aggregate.cost_source_counts,
            quota,
            quota_fetch_state,
        })
    }

    fn event_product_groups(
        &self,
        provider_id: &str,
        agent_module_id: &str,
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
               AND event.agent_module_id = ?4
               AND NOT EXISTS (
                   SELECT 1
                   FROM usage_event_links AS link
                   JOIN usage_events AS canonical
                     ON canonical.event_id = link.canonical_event_id
                   JOIN usage_events AS duplicate
                     ON duplicate.event_id = link.duplicate_event_id
                   WHERE duplicate.event_id = event.event_id
                     AND canonical.source = 'proxy'
                     AND duplicate.source = 'session_log'
                     AND canonical.provider_id = duplicate.provider_id
                     AND canonical.agent_module_id IS NOT NULL
                     AND canonical.agent_module_id = duplicate.agent_module_id
               )
             ORDER BY event.product_group_id",
        )?;
        let groups = statement
            .query_map(
                params![provider_id, start_at, end_at, agent_module_id],
                |row| row.get(0),
            )?
            .collect::<Result<BTreeSet<_>, _>>()?;
        Ok(groups)
    }

    fn event_provider_ids(
        &self,
        agent_module_id: &str,
        start_at: i64,
        end_at: i64,
    ) -> Result<BTreeSet<String>, AppError> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|error| AppError::Database(format!("Mutex lock failed: {error}")))?;
        let mut statement = conn.prepare(
            "SELECT DISTINCT provider_id
             FROM usage_events
             WHERE agent_module_id = ?1
               AND occurred_at >= ?2 AND occurred_at < ?3
             ORDER BY provider_id",
        )?;
        let provider_ids = statement
            .query_map(params![agent_module_id, start_at, end_at], |row| row.get(0))?
            .collect::<Result<BTreeSet<_>, _>>()?;
        Ok(provider_ids)
    }
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
        AgentProviderBindingInput, BillingKind, CostSource, QuotaSnapshot, TokenSource, UsageEvent,
        UsageEventLink, UsageProviderInput,
    };
    use rusqlite::params;
    use rust_decimal::Decimal;
    use serde_json::json;

    fn database_without_system_bindings() -> Database {
        let db = Database::memory().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "DELETE FROM agent_provider_bindings
                 WHERE provider_id IN (
                     SELECT id FROM usage_providers WHERE system_preset_key IS NOT NULL
                 )",
                [],
            )
            .unwrap();
        }
        db
    }

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
        let db = database_without_system_bindings();
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
        proxy.agent_module_id = Some("claude-code".to_string());
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
        linked.agent_module_id = Some("claude-code".to_string());
        let mut similar_a = event(
            "similar-a",
            "metered",
            "product",
            TokenSource::Proxy,
            CostSource::Estimated,
            102,
            Some("0.30"),
        );
        similar_a.agent_module_id = Some("claude-code".to_string());
        let mut similar_b = event(
            "similar-b",
            "metered",
            "product",
            TokenSource::SessionLog,
            CostSource::Unavailable,
            102,
            None,
        );
        similar_b.agent_module_id = Some("claude-code".to_string());
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
            .get_dashboard(100, 200, "claude-code")
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
    fn provider_dashboard_aggregates_all_agent_and_unassigned_history_by_account() {
        let db = database_without_system_bindings();
        db.save_usage_provider(&provider(
            "account-personal",
            BillingKind::Metered,
            "openai",
        ))
        .unwrap();
        db.save_usage_provider(&provider("account-work", BillingKind::Metered, "openai"))
            .unwrap();

        let mut personal_codex = event(
            "personal-codex",
            "account-personal",
            "openai",
            TokenSource::Proxy,
            CostSource::Upstream,
            100,
            Some("0.10"),
        );
        personal_codex.agent_module_id = Some("codex".to_string());
        let mut personal_claude = event(
            "personal-claude",
            "account-personal",
            "legacy-openai",
            TokenSource::SessionLog,
            CostSource::Estimated,
            101,
            Some("0.20"),
        );
        personal_claude.agent_module_id = Some("claude-code".to_string());
        let work_unassigned = event(
            "work-unassigned",
            "account-work",
            "openai",
            TokenSource::Proxy,
            CostSource::Unavailable,
            102,
            None,
        );
        for usage_event in [&personal_codex, &personal_claude, &work_unassigned] {
            db.insert_usage_event(usage_event).unwrap();
        }

        let dashboard = UsageDashboardService::new(&db)
            .get_provider_dashboard(100, 200)
            .unwrap();
        let personal = dashboard
            .providers
            .iter()
            .find(|row| row.provider.id == "account-personal")
            .unwrap();
        let work = dashboard
            .providers
            .iter()
            .find(|row| row.provider.id == "account-work")
            .unwrap();

        assert_eq!(personal.event_count, 2);
        assert_eq!(personal.input_tokens, 20);
        assert_eq!(personal.total_cost_usd.as_deref(), Some("0.3"));
        assert_eq!(work.event_count, 1);
        assert_eq!(work.cost_source_counts.unavailable, 1);
        assert!(!personal.shared_account);
        assert!(!work.shared_account);
    }

    #[test]
    fn dashboard_range_is_half_open_for_the_requested_agent() {
        let db = database_without_system_bindings();
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
            let mut owned = event(
                id,
                provider_id,
                product,
                TokenSource::Proxy,
                CostSource::Unavailable,
                at,
                None,
            );
            owned.agent_module_id = Some("codex".to_string());
            db.insert_usage_event(&owned).unwrap();
        }

        let dashboard = UsageDashboardService::new(&db)
            .get_dashboard(10, 20, "codex")
            .unwrap();
        assert_eq!(dashboard.product_groups.len(), 2);
        let one = dashboard
            .product_groups
            .iter()
            .find(|product| product.product_group_id == "one")
            .unwrap();
        assert_eq!(one.input_tokens, 10);
        assert!(UsageDashboardService::new(&db)
            .get_dashboard(20, 20, "codex")
            .unwrap_err()
            .to_string()
            .contains("start_at must be before end_at"));
    }

    #[test]
    fn subscription_fetch_failure_is_visible_without_a_successful_snapshot() {
        let db = database_without_system_bindings();
        let mut subscription = provider("sub", BillingKind::Subscription, "product");
        subscription.token_sources = vec![TokenSource::SessionLog];
        db.save_usage_provider(&subscription).unwrap();
        db.record_quota_failure("sub", 50, "credentials unavailable")
            .unwrap();
        db.save_agent_provider_binding(&AgentProviderBindingInput {
            id: None,
            agent_module_id: "codex".to_string(),
            provider_id: "sub".to_string(),
            enabled: true,
        })
        .unwrap();

        let dashboard = UsageDashboardService::new(&db)
            .get_dashboard(0, 100, "codex")
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
        let db = database_without_system_bindings();
        db.save_usage_provider(&provider("metered", BillingKind::Metered, "old-product"))
            .unwrap();
        let mut historical = event(
            "historical",
            "metered",
            "old-product",
            TokenSource::Proxy,
            CostSource::Upstream,
            50,
            Some("0.5"),
        );
        historical.agent_module_id = Some("codex".to_string());
        db.insert_usage_event(&historical).unwrap();
        db.save_usage_provider(&provider("metered", BillingKind::Metered, "new-product"))
            .unwrap();

        let old = UsageDashboardService::new(&db)
            .get_dashboard(0, 100, "codex")
            .unwrap();
        assert_eq!(old.product_groups.len(), 1);
        assert_eq!(old.product_groups[0].product_group_id, "old-product");
        assert_eq!(old.product_groups[0].input_tokens, 10);
        assert_eq!(old.product_groups[0].metered_providers[0].event_count, 1);
    }

    #[test]
    fn historical_only_subscription_provider_still_attaches_quota_once() {
        let db = database_without_system_bindings();
        db.save_usage_provider(&provider(
            "historical-subscription",
            BillingKind::Subscription,
            "old-product",
        ))
        .unwrap();
        let mut historical = event(
            "historical-subscription-event",
            "historical-subscription",
            "old-product",
            TokenSource::SessionLog,
            CostSource::Unavailable,
            50,
            None,
        );
        historical.agent_module_id = Some("codex".to_string());
        db.insert_usage_event(&historical).unwrap();
        db.append_quota_success(&QuotaSnapshot {
            snapshot_id: "historical-subscription-quota".to_string(),
            provider_id: "historical-subscription".to_string(),
            fetched_at: 60,
            five_hour_utilization_percent: Some("20".to_string()),
            five_hour_resets_at: None,
            seven_day_utilization_percent: Some("40".to_string()),
            seven_day_resets_at: None,
            manual_resets_remaining: None,
            raw_payload: json!({"private": "not-public"}),
            created_at: 60,
        })
        .unwrap();
        db.save_usage_provider(&provider(
            "historical-subscription",
            BillingKind::Subscription,
            "new-product",
        ))
        .unwrap();

        let dashboard = UsageDashboardService::new(&db)
            .get_dashboard(0, 100, "codex")
            .unwrap();
        let cards = dashboard
            .product_groups
            .iter()
            .flat_map(|product| product.subscription_providers.iter())
            .collect::<Vec<_>>();

        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].provider.id, "historical-subscription");
        assert_eq!(cards.iter().filter(|card| card.quota.is_some()).count(), 1);
    }

    #[test]
    fn provider_cost_overflow_returns_an_error_instead_of_panicking() {
        let db = database_without_system_bindings();
        db.save_usage_provider(&provider("metered", BillingKind::Metered, "product"))
            .unwrap();
        let maximum = Decimal::MAX.to_string();
        for (id, cost) in [("maximum", maximum.as_str()), ("one", "1")] {
            let mut usage_event = event(
                id,
                "metered",
                "product",
                TokenSource::Proxy,
                CostSource::Upstream,
                50,
                Some(cost),
            );
            usage_event.agent_module_id = Some("codex".to_string());
            db.insert_usage_event(&usage_event).unwrap();
        }

        let error = UsageDashboardService::new(&db)
            .get_dashboard(0, 100, "codex")
            .unwrap_err();
        assert!(error.to_string().contains("usage cost aggregate overflow"));
    }

    #[test]
    fn product_cost_overflow_returns_an_error_instead_of_panicking() {
        let db = database_without_system_bindings();
        for id in ["first", "second"] {
            db.save_usage_provider(&provider(id, BillingKind::Metered, "product"))
                .unwrap();
        }
        let maximum = Decimal::MAX.to_string();
        let mut maximum_event = event(
            "maximum",
            "first",
            "product",
            TokenSource::Proxy,
            CostSource::Upstream,
            50,
            Some(&maximum),
        );
        maximum_event.agent_module_id = Some("codex".to_string());
        db.insert_usage_event(&maximum_event).unwrap();
        let mut one_event = event(
            "one",
            "second",
            "product",
            TokenSource::Proxy,
            CostSource::Upstream,
            50,
            Some("1"),
        );
        one_event.agent_module_id = Some("codex".to_string());
        db.insert_usage_event(&one_event).unwrap();

        let error = UsageDashboardService::new(&db)
            .get_dashboard(0, 100, "codex")
            .unwrap_err();
        assert!(error.to_string().contains("usage cost aggregate overflow"));
    }

    #[test]
    fn dashboard_totals_include_only_the_requested_agents_immutable_events() {
        let db = database_without_system_bindings();
        db.save_usage_provider(&provider("metered", BillingKind::Metered, "codex"))
            .unwrap();

        for (event_id, agent_module_id) in [
            ("codex-event", Some("codex")),
            ("other-agent-event", Some("claude-code")),
            ("unassigned-event", None),
        ] {
            let mut usage_event = event(
                event_id,
                "metered",
                "codex",
                TokenSource::Proxy,
                CostSource::Unavailable,
                50,
                None,
            );
            usage_event.agent_module_id = agent_module_id.map(str::to_string);
            db.insert_usage_event(&usage_event).unwrap();
        }

        let dashboard = UsageDashboardService::new(&db)
            .get_dashboard(0, 100, "codex")
            .unwrap();

        assert_eq!(dashboard.product_groups.len(), 1);
        assert_eq!(dashboard.product_groups[0].input_tokens, 10);
        assert_eq!(
            dashboard.product_groups[0].metered_providers[0].event_count,
            1
        );
    }

    #[test]
    fn dashboard_provider_membership_is_active_bindings_union_agent_history() {
        let db = database_without_system_bindings();
        for provider_id in ["active", "historical", "unrelated"] {
            let mut input = provider(provider_id, BillingKind::Subscription, "product");
            input.token_sources = vec![TokenSource::SessionLog];
            db.save_usage_provider(&input).unwrap();
        }
        db.save_agent_provider_binding(&AgentProviderBindingInput {
            id: None,
            agent_module_id: "codex".to_string(),
            provider_id: "active".to_string(),
            enabled: true,
        })
        .unwrap();

        let mut historical = event(
            "historical-event",
            "historical",
            "product",
            TokenSource::SessionLog,
            CostSource::Unavailable,
            50,
            None,
        );
        historical.agent_module_id = Some("codex".to_string());
        db.insert_usage_event(&historical).unwrap();

        let dashboard = UsageDashboardService::new(&db)
            .get_dashboard(0, 100, "codex")
            .unwrap();
        let providers = dashboard.product_groups[0]
            .subscription_providers
            .iter()
            .map(|usage| usage.provider.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(providers, vec!["active", "historical"]);
    }

    #[test]
    fn only_fully_valid_legacy_links_suppress_requested_agent_usage() {
        let db = database_without_system_bindings();
        for provider_id in ["metered", "other-provider"] {
            db.save_usage_provider(&provider(provider_id, BillingKind::Metered, "product"))
                .unwrap();
        }
        let owned_event = |event_id: &str,
                           provider_id: &str,
                           source: TokenSource,
                           agent_module_id: Option<&str>,
                           occurred_at: i64| {
            let mut usage_event = event(
                event_id,
                provider_id,
                "product",
                source,
                CostSource::Unavailable,
                occurred_at,
                None,
            );
            usage_event.agent_module_id = agent_module_id.map(str::to_string);
            usage_event
        };
        for usage_event in [
            owned_event(
                "valid-proxy",
                "metered",
                TokenSource::Proxy,
                Some("codex"),
                10,
            ),
            owned_event(
                "valid-session",
                "metered",
                TokenSource::SessionLog,
                Some("codex"),
                11,
            ),
            owned_event(
                "cross-agent-proxy",
                "metered",
                TokenSource::Proxy,
                Some("claude-code"),
                20,
            ),
            owned_event(
                "cross-agent-session",
                "metered",
                TokenSource::SessionLog,
                Some("codex"),
                21,
            ),
            owned_event(
                "cross-provider-proxy",
                "other-provider",
                TokenSource::Proxy,
                Some("codex"),
                30,
            ),
            owned_event(
                "cross-provider-session",
                "metered",
                TokenSource::SessionLog,
                Some("codex"),
                31,
            ),
            owned_event("null-agent-proxy", "metered", TokenSource::Proxy, None, 40),
            owned_event(
                "null-agent-session",
                "metered",
                TokenSource::SessionLog,
                Some("codex"),
                41,
            ),
            owned_event(
                "reverse-session",
                "metered",
                TokenSource::SessionLog,
                Some("codex"),
                50,
            ),
            owned_event(
                "reverse-proxy",
                "metered",
                TokenSource::Proxy,
                Some("codex"),
                51,
            ),
            owned_event(
                "missing-endpoint-session",
                "metered",
                TokenSource::SessionLog,
                Some("codex"),
                61,
            ),
        ] {
            db.insert_usage_event(&usage_event).unwrap();
        }
        db.insert_usage_event_link(&UsageEventLink {
            canonical_event_id: "valid-proxy".to_string(),
            duplicate_event_id: "valid-session".to_string(),
            link_kind: "request_id".to_string(),
            link_value: "valid".to_string(),
            created_at: 12,
        })
        .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
            for (canonical_event_id, duplicate_event_id, created_at) in [
                ("cross-agent-proxy", "cross-agent-session", 22),
                ("cross-provider-proxy", "cross-provider-session", 32),
                ("null-agent-proxy", "null-agent-session", 42),
                ("reverse-session", "reverse-proxy", 52),
                ("missing-canonical", "missing-endpoint-session", 62),
            ] {
                conn.execute(
                    "INSERT INTO usage_event_links (
                         canonical_event_id, duplicate_event_id,
                         link_kind, link_value, created_at
                     ) VALUES (?1, ?2, 'request_id', ?3, ?4)",
                    params![
                        canonical_event_id,
                        duplicate_event_id,
                        format!("legacy-{created_at}"),
                        created_at
                    ],
                )
                .unwrap();
            }
            conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        }

        let dashboard = UsageDashboardService::new(&db)
            .get_dashboard(0, 100, "codex")
            .unwrap();
        let usage = dashboard.product_groups[0]
            .metered_providers
            .iter()
            .find(|usage| usage.provider.id == "metered")
            .unwrap();
        assert_eq!(usage.event_count, 7);
        assert_eq!(usage.input_tokens, 70);

        let detail = db
            .list_agent_usage_events("codex", Some("metered"), 0, 100, 1, 20)
            .unwrap();
        assert_eq!(detail.total, 8, "linked duplicates remain in event detail");

        let diagnostics = db.get_unassigned_usage_diagnostics().unwrap();
        assert_eq!(diagnostics.invalid_link_summaries.len(), 4);
        for (reason, count) in [
            ("agent_missing_or_mismatch", 2),
            ("invalid_source_direction", 1),
            ("missing_endpoint", 1),
            ("provider_mismatch", 1),
        ] {
            assert_eq!(
                diagnostics
                    .invalid_link_summaries
                    .iter()
                    .find(|summary| summary.reason == reason)
                    .unwrap()
                    .link_count,
                count
            );
        }
    }

    #[test]
    fn public_agent_dashboard_identifies_owner_and_shared_account_state() {
        let db = database_without_system_bindings();
        db.save_usage_provider(&provider("metered", BillingKind::Metered, "product"))
            .unwrap();
        let mut owned = event(
            "owned",
            "metered",
            "product",
            TokenSource::Proxy,
            CostSource::Unavailable,
            50,
            None,
        );
        owned.agent_module_id = Some("codex".to_string());
        db.insert_usage_event(&owned).unwrap();

        let dashboard = UsageDashboardService::new(&db)
            .get_dashboard(0, 100, "codex")
            .unwrap();
        let public = serde_json::to_value(dashboard).unwrap();

        assert_eq!(public["agentModuleId"], json!("codex"));
        assert_eq!(
            public["productGroups"][0]["meteredProviders"][0]["sharedAccount"],
            json!(false)
        );
    }
}
