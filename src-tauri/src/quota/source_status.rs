//! BE-5b / C4 quota source status.
//!
//! Read-only projection of the latest stored quota snapshot. A failed provider
//! read is a row with `state=stale` or `state=error`; this module does not
//! fetch, throw, or change any gate decision.

use crate::model::{BillingKind, QuotaSnapshot, UsageProviderView};
use crate::providers::TIER_FIVE_HOUR;
use crate::store::Database;
use crate::usage::claude_oauth::CLAUDE_OAUTH_QUOTA_SOURCE_KIND;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::claude_quota::QUOTA_READ_FRESH_SECS;

pub const QUOTA_UPDATED_EVENT: &str = "quota-updated";

/// Volcengine Agent Plan (`GetAFPUsage`) is the only quota source ADR Q5 marks
/// official. The collector stores that plan on `credentialMessage`.
const OFFICIAL_PLAN_PREFIX: &str = "Agent Plan";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaOrigin {
    Official,
    BestEffort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaReadState {
    Ok,
    Stale,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuotaWindowKind {
    #[serde(rename = "5h")]
    FiveHour,
    #[serde(rename = "weekly")]
    Weekly,
    #[serde(rename = "daily")]
    Daily,
    #[serde(rename = "monthly")]
    Monthly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindowStatus {
    pub kind: QuotaWindowKind,
    pub used_pct: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderQuotaStatus {
    pub provider_id: String,
    pub windows: Vec<QuotaWindowStatus>,
    /// Tightest window: the maximum `usedPct`. `0` when there is no window.
    pub used_pct: f64,
    pub source: QuotaOrigin,
    pub state: QuotaReadState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<i64>,
}

/// `ok` while the newest success is at most 15 minutes old.
/// Strictly older than that is `stale`, including a later failed read that
/// left the previous snapshot in place.
/// `error` is only a read that failed and never produced a success.
pub fn classify_quota_read_state(
    now: i64,
    last_success_at: Option<i64>,
    last_error: Option<&str>,
) -> QuotaReadState {
    match last_success_at {
        Some(at) if now.saturating_sub(at) > QUOTA_READ_FRESH_SECS => QuotaReadState::Stale,
        Some(_) => QuotaReadState::Ok,
        None if last_error.is_some() => QuotaReadState::Error,
        None => QuotaReadState::Stale,
    }
}

fn origin_from_label(label: &str) -> QuotaOrigin {
    match label {
        "official" => QuotaOrigin::Official,
        _ => QuotaOrigin::BestEffort,
    }
}

pub fn quota_origin(quota_source: Option<&str>, credential_message: Option<&str>) -> QuotaOrigin {
    if credential_message.is_some_and(|message| message.starts_with(OFFICIAL_PLAN_PREFIX)) {
        return QuotaOrigin::Official;
    }
    // Claude's OAuth usage endpoint is best-effort (ADR Q5). Codex, Kimi, GLM,
    // MiniMax, and Volcengine Coding Plan stay best-effort as well.
    if quota_source == Some("claude_oauth") {
        return origin_from_label(CLAUDE_OAUTH_QUOTA_SOURCE_KIND);
    }
    QuotaOrigin::BestEffort
}

pub fn windows_from_snapshot(snapshot: &QuotaSnapshot) -> Vec<QuotaWindowStatus> {
    let from_tiers = windows_from_tiers(snapshot.raw_payload.get("tiers"));
    if !from_tiers.is_empty() {
        return from_tiers;
    }
    let mut windows = Vec::new();
    if let Some(window) = window_from_percent(
        QuotaWindowKind::FiveHour,
        snapshot.five_hour_utilization_percent.as_deref(),
        snapshot.five_hour_resets_at.clone(),
    ) {
        windows.push(window);
    }
    if let Some(window) = window_from_percent(
        QuotaWindowKind::Weekly,
        snapshot.seven_day_utilization_percent.as_deref(),
        snapshot.seven_day_resets_at.clone(),
    ) {
        windows.push(window);
    }
    windows
}

fn windows_from_tiers(tiers: Option<&Value>) -> Vec<QuotaWindowStatus> {
    let Some(tiers) = tiers.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut windows = Vec::new();
    for tier in tiers {
        let Some(kind) = tier
            .get("name")
            .and_then(Value::as_str)
            .and_then(window_kind)
        else {
            continue;
        };
        let Some(used_pct) = tier.get("utilization").and_then(finite_f64) else {
            continue;
        };
        let resets_at = tier
            .get("resetsAt")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        windows.push(QuotaWindowStatus {
            kind,
            used_pct,
            resets_at,
        });
    }
    windows
}

fn window_kind(name: &str) -> Option<QuotaWindowKind> {
    match name {
        TIER_FIVE_HOUR | "5h" => Some(QuotaWindowKind::FiveHour),
        "seven_day" | "seven_day_opus" | "seven_day_sonnet" | "weekly_limit" => {
            Some(QuotaWindowKind::Weekly)
        }
        "daily" | "day" => Some(QuotaWindowKind::Daily),
        "monthly" | "30_day" => Some(QuotaWindowKind::Monthly),
        _ => None,
    }
}

fn window_from_percent(
    kind: QuotaWindowKind,
    percent: Option<&str>,
    resets_at: Option<String>,
) -> Option<QuotaWindowStatus> {
    let used_pct = percent?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())?;
    let resets_at = resets_at
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    Some(QuotaWindowStatus {
        kind,
        used_pct,
        resets_at,
    })
}

fn finite_f64(value: &Value) -> Option<f64> {
    let number = value.as_f64().or_else(|| {
        value
            .as_str()
            .and_then(|text| text.trim().parse::<f64>().ok())
    })?;
    number.is_finite().then_some(number)
}

pub fn project_provider_quota_status(
    provider_id: &str,
    quota_source: Option<&str>,
    snapshot: Option<&QuotaSnapshot>,
    last_success_at: Option<i64>,
    last_error: Option<&str>,
    now: i64,
) -> ProviderQuotaStatus {
    let state = classify_quota_read_state(now, last_success_at, last_error);
    let credential_message = snapshot
        .and_then(|snapshot| snapshot.raw_payload.get("credentialMessage"))
        .and_then(Value::as_str);
    let windows = if state == QuotaReadState::Error {
        Vec::new()
    } else {
        snapshot.map(windows_from_snapshot).unwrap_or_default()
    };
    let used_pct = windows
        .iter()
        .map(|window| window.used_pct)
        .fold(0.0_f64, f64::max);
    ProviderQuotaStatus {
        provider_id: provider_id.to_string(),
        windows,
        used_pct,
        source: quota_origin(quota_source, credential_message),
        state,
        last_success_at,
    }
}

fn provider_has_quota(provider: &UsageProviderView) -> bool {
    provider.quota_source.is_some() || provider.billing_kind == BillingKind::Subscription
}

pub fn quota_status_at(
    db: &Database,
    now: i64,
) -> Result<Vec<ProviderQuotaStatus>, crate::error::AppError> {
    let mut providers = db.list_usage_providers()?;
    providers.retain(|provider| provider.enabled && provider_has_quota(provider));
    providers.sort_by(|left, right| left.id.cmp(&right.id));
    let mut rows = Vec::with_capacity(providers.len());
    for provider in providers {
        let (snapshot, fetch_state) = db.latest_quota_status(&provider.id)?;
        let last_success_at = fetch_state
            .as_ref()
            .and_then(|state| state.last_success_at)
            .or_else(|| snapshot.as_ref().map(|snapshot| snapshot.fetched_at));
        let last_error = fetch_state
            .as_ref()
            .and_then(|state| state.last_error.as_deref());
        rows.push(project_provider_quota_status(
            &provider.id,
            provider.quota_source.as_deref(),
            snapshot.as_ref(),
            last_success_at,
            last_error,
            now,
        ));
    }
    Ok(rows)
}

pub fn quota_updated_payload(rows: &[ProviderQuotaStatus]) -> Value {
    serde_json::to_value(rows).unwrap_or_else(|_| Value::Array(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{QuotaFetchState, QuotaSnapshot, TokenSource, UsageProviderInput};
    use crate::providers::{QuotaTier, SubscriptionQuota, TIER_FIVE_HOUR, TIER_SEVEN_DAY};
    use serde_json::json;

    fn assert_no_money(value: &Value) {
        let text = value.to_string().to_ascii_lowercase();
        for needle in ["usd", "dollar", "cost", "usedvalue", "maxvalue"] {
            assert!(!text.contains(needle), "{text} contains {needle}");
        }
    }

    #[test]
    fn state_is_stale_only_after_the_fifteen_minute_window() {
        let fresh_at = 1_000;
        assert_eq!(
            classify_quota_read_state(fresh_at + QUOTA_READ_FRESH_SECS, Some(fresh_at), None),
            QuotaReadState::Ok
        );
        assert_eq!(
            classify_quota_read_state(
                fresh_at + QUOTA_READ_FRESH_SECS + 1,
                Some(fresh_at),
                Some("timeout")
            ),
            QuotaReadState::Stale
        );
        assert_eq!(
            classify_quota_read_state(fresh_at, None, None),
            QuotaReadState::Stale
        );
        assert_eq!(
            classify_quota_read_state(fresh_at, None, Some("timeout")),
            QuotaReadState::Error
        );
    }

    #[test]
    fn claude_oauth_and_other_non_agent_plans_are_best_effort() {
        assert_eq!(CLAUDE_OAUTH_QUOTA_SOURCE_KIND, "best_effort");
        assert_eq!(
            quota_origin(Some("claude_oauth"), None),
            QuotaOrigin::BestEffort
        );
        assert_eq!(
            quota_origin(
                Some(crate::usage::system_providers::CLAUDE_LOCAL_QUOTA_SOURCE),
                None
            ),
            QuotaOrigin::BestEffort
        );
        assert_eq!(quota_origin(Some("codex"), None), QuotaOrigin::BestEffort);
        assert_eq!(
            quota_origin(Some("coding_plan"), Some("Coding Plan")),
            QuotaOrigin::BestEffort
        );
        assert_eq!(
            quota_origin(Some("coding_plan"), Some("Agent Plan Pro")),
            QuotaOrigin::Official
        );
    }

    #[test]
    fn tightest_window_is_the_max_and_dollars_are_dropped() {
        let snapshot = QuotaSnapshot {
            snapshot_id: "snap".to_string(),
            provider_id: "claude".to_string(),
            fetched_at: 10,
            five_hour_utilization_percent: Some("12.5".to_string()),
            five_hour_resets_at: Some("2026-07-11T05:00:00Z".to_string()),
            seven_day_utilization_percent: Some("80".to_string()),
            seven_day_resets_at: None,
            manual_resets_remaining: None,
            raw_payload: json!({
                "credentialMessage": "pro",
                "extraUsage": {"usedCredits": 3.5, "currency": "USD"},
                "tiers": [
                    {
                        "name": TIER_FIVE_HOUR,
                        "utilization": 12.5,
                        "resetsAt": "2026-07-11T05:00:00Z",
                        "usedValueUsd": 1.25,
                        "maxValueUsd": 20.0
                    },
                    {
                        "name": TIER_SEVEN_DAY,
                        "utilization": 80.0,
                        "resetsAt": "2026-07-18T00:00:00Z"
                    },
                    {"name": "monthly", "utilization": 10.0}
                ]
            }),
            created_at: 10,
        };
        let row = project_provider_quota_status(
            "claude",
            Some("claude"),
            Some(&snapshot),
            Some(10),
            None,
            10 + QUOTA_READ_FRESH_SECS + 5,
        );
        assert_eq!(row.state, QuotaReadState::Stale);
        assert_eq!(row.used_pct, 80.0);
        assert_eq!(row.windows.len(), 3);
        assert_eq!(row.source, QuotaOrigin::BestEffort);
        let json = serde_json::to_value(&row).unwrap();
        assert_no_money(&json);
        assert!(json.get("valueUsd").is_none());
        assert_eq!(json["windows"][0]["kind"], "5h");
        assert_eq!(json["windows"][1]["kind"], "weekly");
        assert_eq!(json["lastSuccessAt"], 10);
    }

    #[test]
    fn failed_read_with_no_snapshot_is_error_and_does_not_invent_windows() {
        let row =
            project_provider_quota_status("codex", Some("codex"), None, None, Some("boom"), 50);
        assert_eq!(row.state, QuotaReadState::Error);
        assert!(row.windows.is_empty());
        assert_eq!(row.used_pct, 0.0);
        assert!(row.last_success_at.is_none());
        assert_no_money(&serde_json::to_value(&row).unwrap());
    }

    #[test]
    fn stored_failure_is_returned_instead_of_an_error() {
        let db = Database::memory().unwrap();
        db.save_usage_provider(&UsageProviderInput {
            id: "sub".to_string(),
            name: "Claude".to_string(),
            billing_kind: BillingKind::Subscription,
            product_group_id: "claude".to_string(),
            token_sources: vec![TokenSource::SessionLog],
            session_source_bindings: None,
            quota_source: Some("claude".to_string()),
            quota_interval_seconds: Some(900),
            route_app_type: None,
            route_config: None,
            quota_config: None,
            enabled: true,
        })
        .unwrap();
        let mut quota = SubscriptionQuota::skeleton("claude");
        quota.success = true;
        quota.credential_message = Some("pro".to_string());
        quota.tiers = vec![QuotaTier {
            name: TIER_FIVE_HOUR.to_string(),
            utilization: 40.0,
            resets_at: None,
            used_value_usd: Some(9.99),
            max_value_usd: Some(20.0),
        }];
        let raw_payload = serde_json::to_value(&quota).unwrap();
        db.append_quota_success(&QuotaSnapshot {
            snapshot_id: "snap-1".to_string(),
            provider_id: "sub".to_string(),
            fetched_at: 100,
            five_hour_utilization_percent: Some("40".to_string()),
            five_hour_resets_at: None,
            seven_day_utilization_percent: None,
            seven_day_resets_at: None,
            manual_resets_remaining: None,
            raw_payload,
            created_at: 100,
        })
        .unwrap();
        db.record_quota_failure("sub", 100 + QUOTA_READ_FRESH_SECS + 30, "timeout")
            .unwrap();

        let rows = quota_status_at(&db, 100 + QUOTA_READ_FRESH_SECS + 30).unwrap();
        let row = rows.iter().find(|row| row.provider_id == "sub").unwrap();
        assert_eq!(row.state, QuotaReadState::Stale);
        assert_eq!(row.used_pct, 40.0);
        let payload = quota_updated_payload(&rows);
        assert_no_money(&payload);
        let serialized = payload
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["providerId"] == "sub")
            .unwrap();
        assert!(serialized.get("diff").is_none());

        let state = db.get_quota_fetch_state("sub").unwrap().unwrap();
        assert_eq!(
            state,
            QuotaFetchState {
                provider_id: "sub".to_string(),
                last_attempt_at: Some(100 + QUOTA_READ_FRESH_SECS + 30),
                last_success_at: Some(100),
                last_error: Some("timeout".to_string()),
                consecutive_failures: 1,
                stale: false,
            }
        );
    }
}
