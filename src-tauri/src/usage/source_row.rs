//! C13 source rows for the tray snapshot (BE-21).
//!
//! Classification reads the stored provider and the BE-5b quota projection.
//! A stale subscription stays `subscription`; it is not rewritten as
//! `untracked`. This module does not fetch quota and does not enforce a cap.

use std::collections::HashMap;

use crate::model::{BillingKind, UsageProviderView};
use crate::quota::source_status::{
    quota_status_at, ProviderQuotaStatus, QuotaReadState, QuotaWindowStatus,
};
use crate::store::Database;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceBillingKind {
    Metered,
    Subscription,
    Untracked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BetaTruncateUnavailable {
    LicenseRequired,
}

/// Community build has no Pro license, so truncation is present only as a
/// disabled flag. `thresholdPct` lives on C17 and is intentionally absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaTruncateView {
    pub enabled: bool,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<BetaTruncateUnavailable>,
}

impl BetaTruncateView {
    pub fn community_unavailable() -> Self {
        Self {
            enabled: false,
            available: false,
            unavailable_reason: Some(BetaTruncateUnavailable::LicenseRequired),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRowView {
    pub provider_id: String,
    pub display_name: String,
    pub billing_kind: SourceBillingKind,
    pub enforceable: bool,
    /// Metered rows only. Subscription rows leave this absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_usd: Option<String>,
    /// Tightest window. Absent when the row has no quota windows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows: Option<Vec<QuotaWindowStatus>>,
    /// `usedPct / 100`, clamped to `0..=1`. `0` when there is no percent.
    pub progress: f64,
    /// Community build never stops traffic.
    pub stopped: bool,
    pub source_state: QuotaReadState,
    /// C17 reason codes. No classifications are inferred from provider names.
    /// Emission awaits the C17 mapping and provider-scoped transport evidence.
    #[serde(default)]
    pub note_codes: Vec<String>,
    /// Earliest known window reset; never a synthetic local-day boundary.
    pub next_reset_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beta_truncate: Option<BetaTruncateView>,
}

pub fn source_rows_at(
    db: &Database,
    now: i64,
    metered_value_usd: &HashMap<String, String>,
) -> Result<Vec<SourceRowView>, crate::error::AppError> {
    let quota = quota_status_at(db, now)?;
    let quota_by_id = quota
        .into_iter()
        .map(|row| (row.provider_id.clone(), row))
        .collect::<HashMap<_, _>>();
    let mut providers = db.list_usage_providers()?;
    providers.retain(|provider| provider.enabled);
    providers.sort_by(|left, right| {
        (left.name.to_lowercase(), left.id.as_str())
            .cmp(&(right.name.to_lowercase(), right.id.as_str()))
    });
    Ok(providers
        .iter()
        .map(|provider| {
            source_row_for_provider(
                provider,
                quota_by_id.get(&provider.id),
                metered_value_usd.get(&provider.id).map(String::as_str),
            )
        })
        .collect())
}

pub fn source_row_for_provider(
    provider: &UsageProviderView,
    quota: Option<&ProviderQuotaStatus>,
    metered_value_usd: Option<&str>,
) -> SourceRowView {
    if is_cursor_provider(provider) {
        return SourceRowView {
            provider_id: provider.id.clone(),
            display_name: provider.name.clone(),
            billing_kind: SourceBillingKind::Untracked,
            enforceable: false,
            value_usd: None,
            used_pct: None,
            windows: None,
            progress: 0.0,
            stopped: false,
            source_state: QuotaReadState::Ok,
            note_codes: Vec::new(),
            next_reset_at: None,
            beta_truncate: None,
        };
    }

    if provider.billing_kind == BillingKind::Subscription {
        let (used_pct, windows, source_state) = match quota {
            Some(row) => (
                row.used_pct,
                nonempty_windows(publishable_windows(row.windows.clone())),
                row.state,
            ),
            // No stored quota: omit usedPct. Do not publish 0%.
            None => (None, None, QuotaReadState::Stale),
        };
        return SourceRowView {
            provider_id: provider.id.clone(),
            display_name: provider.name.clone(),
            billing_kind: SourceBillingKind::Subscription,
            enforceable: false,
            value_usd: None,
            used_pct,
            next_reset_at: windows
                .iter()
                .flatten()
                .filter_map(|window| window.resets_at.as_deref())
                .filter_map(|value| chrono::DateTime::parse_from_rfc3339(value.trim()).ok())
                .min()
                .map(|value| {
                    value
                        .with_timezone(&chrono::Utc)
                        .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
                }),
            windows,
            progress: progress_from_used_pct(used_pct),
            stopped: false,
            source_state,
            note_codes: Vec::new(),
            beta_truncate: Some(BetaTruncateView::community_unavailable()),
        };
    }

    SourceRowView {
        provider_id: provider.id.clone(),
        display_name: provider.name.clone(),
        billing_kind: SourceBillingKind::Metered,
        // Billing identity is not stored yet (BE-2). Metered rows are API-key
        // shaped, and an unknown identity would also be enforceable.
        enforceable: true,
        value_usd: metered_value_usd
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        used_pct: None,
        windows: None,
        progress: 0.0,
        stopped: false,
        source_state: QuotaReadState::Ok,
        note_codes: Vec::new(),
        next_reset_at: None,
        beta_truncate: None,
    }
}

fn nonempty_windows(windows: Vec<QuotaWindowStatus>) -> Option<Vec<QuotaWindowStatus>> {
    (!windows.is_empty()).then_some(windows)
}

/// The tray snapshot must not echo an unparsed reset string. A quota payload
/// can store a sentinel there; only an ISO-8601 timestamp is published.
fn publishable_windows(windows: Vec<QuotaWindowStatus>) -> Vec<QuotaWindowStatus> {
    windows
        .into_iter()
        .map(|mut window| {
            if window
                .resets_at
                .as_deref()
                .is_some_and(|value| !is_iso_timestamp(value))
            {
                window.resets_at = None;
            }
            window
        })
        .collect()
}

fn is_iso_timestamp(value: &str) -> bool {
    let value = value.trim();
    chrono::DateTime::parse_from_rfc3339(value).is_ok()
        || chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S").is_ok()
        || chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%SZ").is_ok()
}

fn progress_from_used_pct(used_pct: Option<f64>) -> f64 {
    let Some(used_pct) = used_pct else {
        return 0.0;
    };
    (used_pct / 100.0).clamp(0.0, 1.0)
}

fn is_cursor_provider(provider: &UsageProviderView) -> bool {
    let preset = provider.system_preset_key.as_deref();
    [
        Some(provider.id.as_str()),
        Some(provider.name.as_str()),
        preset,
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_ascii_lowercase().contains("cursor"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BillingKind, QuotaSnapshot, TokenSource, UsageProviderInput};
    use crate::providers::{QuotaTier, SubscriptionQuota, TIER_FIVE_HOUR};
    use crate::quota::claude_quota::QUOTA_READ_FRESH_SECS;

    fn provider(id: &str, name: &str, billing: BillingKind) -> UsageProviderInput {
        UsageProviderInput {
            id: id.to_string(),
            name: name.to_string(),
            billing_kind: billing,
            product_group_id: "fixture".to_string(),
            token_sources: vec![TokenSource::SessionLog],
            session_source_bindings: None,
            quota_source: if billing == BillingKind::Subscription {
                Some("claude".to_string())
            } else {
                None
            },
            quota_interval_seconds: None,
            route_app_type: None,
            route_config: None,
            quota_config: None,
            enabled: true,
        }
    }

    fn assert_no_value_usd(value: &serde_json::Value) {
        assert!(value.get("valueUsd").is_none(), "{value}");
        let text = value.to_string().to_ascii_lowercase();
        assert!(!text.contains("valueusd"), "{text}");
    }

    #[test]
    fn source_reset_contract_uses_earliest_instant_or_explicit_null() {
        use crate::quota::source_status::{QuotaOrigin, QuotaWindowKind};
        use crate::usage::tray_snapshot::{TrayUsageProjector, TrayUsageSnapshot};
        use chrono::TimeZone;
        use std::sync::Arc;

        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/tray-source-contract.json"
        ))
        .unwrap();
        let db = Arc::new(Database::memory().unwrap());
        db.save_usage_provider(&provider("sub", "Claude", BillingKind::Subscription))
            .unwrap();
        let provider = db
            .list_usage_providers()
            .unwrap()
            .into_iter()
            .find(|provider| provider.id == "sub")
            .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let quota = ProviderQuotaStatus {
                provider_id: provider.id.clone(),
                windows: case["resetsAt"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|reset| QuotaWindowStatus {
                        kind: QuotaWindowKind::FiveHour,
                        used_pct: 40.0,
                        resets_at: reset.as_str().map(str::to_owned),
                    })
                    .collect(),
                used_pct: Some(40.0),
                source: QuotaOrigin::BestEffort,
                state: QuotaReadState::Ok,
                last_success_at: Some(100),
            };
            let row = source_row_for_provider(&provider, Some(&quota), None);
            let json = serde_json::to_value(row).unwrap();
            assert!(json.get("nextResetAt").is_some(), "{}", case["id"]);
            assert_eq!(json["nextResetAt"], case["nextResetAt"], "{}", case["id"]);
            assert!(json["noteCodes"].is_array());
        }
        let now = chrono::Local.timestamp_opt(100, 0).single().unwrap();
        for snapshot in [
            TrayUsageSnapshot::unknown(100),
            TrayUsageProjector::new(db).project_at(now).unwrap(),
        ] {
            assert_eq!(
                serde_json::to_value(snapshot).unwrap()["buildType"],
                fixture["buildType"]
            );
        }
    }

    #[test]
    fn cursor_is_untracked_and_a_stale_subscription_stays_subscription() {
        let db = Database::memory().unwrap();
        db.save_usage_provider(&provider("cursor", "Cursor", BillingKind::Metered))
            .unwrap();
        db.save_usage_provider(&provider("sub", "Claude", BillingKind::Subscription))
            .unwrap();
        let mut quota = SubscriptionQuota::skeleton("claude");
        quota.success = true;
        quota.tiers = vec![QuotaTier {
            name: TIER_FIVE_HOUR.to_string(),
            utilization: 40.0,
            resets_at: None,
            used_value_usd: Some(9.99),
            max_value_usd: Some(20.0),
        }];
        db.append_quota_success(&QuotaSnapshot {
            snapshot_id: "snap-1".to_string(),
            provider_id: "sub".to_string(),
            fetched_at: 100,
            five_hour_utilization_percent: Some("40".to_string()),
            five_hour_resets_at: None,
            seven_day_utilization_percent: None,
            seven_day_resets_at: None,
            manual_resets_remaining: None,
            raw_payload: serde_json::to_value(&quota).unwrap(),
            created_at: 100,
        })
        .unwrap();

        let now = 100 + QUOTA_READ_FRESH_SECS + 1;
        let mut costs = HashMap::new();
        costs.insert("cursor".to_string(), "12.5".to_string());
        let rows = source_rows_at(&db, now, &costs).unwrap();
        let cursor = rows.iter().find(|row| row.provider_id == "cursor").unwrap();
        assert_eq!(cursor.billing_kind, SourceBillingKind::Untracked);
        assert!(!cursor.enforceable);
        assert!(!cursor.stopped);
        assert_eq!(cursor.source_state, QuotaReadState::Ok);
        assert!(cursor.beta_truncate.is_none());
        assert_no_value_usd(&serde_json::to_value(cursor).unwrap());

        let subscription = rows.iter().find(|row| row.provider_id == "sub").unwrap();
        assert_eq!(subscription.billing_kind, SourceBillingKind::Subscription);
        assert_eq!(subscription.source_state, QuotaReadState::Stale);
        assert_eq!(subscription.used_pct, Some(40.0));
        assert!((subscription.progress - 0.4).abs() < f64::EPSILON);
        assert!(!subscription.enforceable);
        let json = serde_json::to_value(subscription).unwrap();
        assert_no_value_usd(&json);
        assert!(json["betaTruncate"].get("thresholdPct").is_none());
        assert_eq!(json["betaTruncate"]["enabled"], false);
        assert_eq!(json["betaTruncate"]["available"], false);
        assert_eq!(
            json["betaTruncate"]["unavailableReason"],
            "license_required"
        );
        assert_eq!(json["windows"][0]["kind"], "5h");
    }

    #[test]
    fn subscription_without_quota_data_omits_used_pct() {
        let db = Database::memory().unwrap();
        db.save_usage_provider(&provider("sub", "Claude", BillingKind::Subscription))
            .unwrap();
        let rows = source_rows_at(&db, 10, &HashMap::new()).unwrap();
        let row = rows.iter().find(|row| row.provider_id == "sub").unwrap();
        assert_eq!(row.billing_kind, SourceBillingKind::Subscription);
        assert_eq!(row.used_pct, None);
        assert!(row.windows.is_none());
        assert_eq!(row.progress, 0.0);
        assert!(serde_json::to_value(row).unwrap().get("usedPct").is_none());
        assert_eq!(
            serde_json::to_value(row).unwrap().get("nextResetAt"),
            Some(&serde_json::Value::Null)
        );
    }

    #[test]
    fn progress_clamps_and_metered_keeps_its_amount() {
        let row = source_row_for_provider(
            &UsageProviderView {
                id: "api".to_string(),
                name: "API".to_string(),
                billing_kind: BillingKind::Metered,
                product_group_id: "api".to_string(),
                token_sources: Vec::new(),
                session_source_bindings: Vec::new(),
                bindings: Vec::new(),
                quota_source: None,
                quota_interval_seconds: None,
                route_app_type: None,
                enabled: true,
                needs_review: false,
                created_at: 0,
                updated_at: 0,
                route_base_url: None,
                has_route_credentials: false,
                system_preset_key: None,
                system_auth_kind: None,
                canonical_endpoint: None,
                compatible_agent_module_ids: Vec::new(),
                api_keys: Vec::new(),
                daily_budget_usd: None,
                supports_key_usage: false,
                key_usage_total: None,
            },
            None,
            Some("1.50"),
        );
        assert_eq!(row.billing_kind, SourceBillingKind::Metered);
        assert!(row.enforceable);
        assert_eq!(row.value_usd.as_deref(), Some("1.50"));
        assert_eq!(row.progress, 0.0);
        assert!(row.beta_truncate.is_none());

        let over = progress_from_used_pct(Some(150.0));
        let under = progress_from_used_pct(Some(-3.0));
        assert_eq!(over, 1.0);
        assert_eq!(under, 0.0);
    }
}
