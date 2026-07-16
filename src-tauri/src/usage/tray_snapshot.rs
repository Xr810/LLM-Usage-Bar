use super::domain::BillingKind;
use super::status::{CostQuality, UsageStatus};
use chrono::{DateTime, Local, LocalResult, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};

const ROLLING_30_DAYS_SECONDS: i64 = 30 * 86_400;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayUsageSnapshot {
    pub status: UsageStatus,
    pub generated_at: i64,
    pub last_success_at: Option<i64>,
    pub stale: bool,
    pub refresh_error: Option<String>,
    pub refresh_in_progress: bool,
    pub agents: Vec<TrayAgentUsageView>,
}

impl TrayUsageSnapshot {
    pub fn unknown(generated_at: i64) -> Self {
        Self {
            status: UsageStatus::Unknown,
            generated_at,
            last_success_at: None,
            stale: false,
            refresh_error: None,
            refresh_in_progress: false,
            agents: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayAgentUsageView {
    pub agent_module_id: String,
    pub name: String,
    pub sort_order: i64,
    pub status: UsageStatus,
    pub providers: Vec<TrayProviderUsageView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayProviderUsageView {
    pub provider_id: String,
    pub provider_name: String,
    pub system_preset_key: Option<String>,
    pub billing_kind: BillingKind,
    pub status: UsageStatus,
    pub warning_reason: Option<String>,
    pub subscription: Option<TraySubscriptionUsageView>,
    pub metered: Option<TrayMeteredUsageView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayQuotaWindowView {
    pub kind: String,
    pub used_percent: Option<String>,
    pub remaining_percent: Option<String>,
    pub resets_at: Option<String>,
    pub status: UsageStatus,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraySubscriptionUsageView {
    pub plan_label: Option<String>,
    pub windows: Vec<TrayQuotaWindowView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayMeteredUsageView {
    pub today_cost_usd: Option<String>,
    pub rolling_30_day_cost_usd: Option<String>,
    pub daily_budget_usd: Option<String>,
    pub budget_consumed_percent: Option<String>,
    pub total_tokens: u64,
    pub cost_quality: CostQuality,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrayUsageWindows {
    pub today_start_at: i64,
    pub rolling_30_start_at: i64,
    pub end_at: i64,
    pub next_local_midnight_at: i64,
}

impl TrayUsageWindows {
    pub fn from_local_now(now: DateTime<Local>) -> Option<Self> {
        Self::from_zoned_now(now)
    }

    pub fn from_zoned_now<Tz>(now: DateTime<Tz>) -> Option<Self>
    where
        Tz: TimeZone,
    {
        let now_timestamp = now.timestamp();
        let local_date = now.date_naive();
        let timezone = now.timezone();

        Self::from_calendar_now(now_timestamp, local_date, |date| {
            resolve_local_midnight(&timezone, date)
        })
    }

    fn from_calendar_now(
        now_timestamp: i64,
        local_date: NaiveDate,
        mut resolve_midnight: impl FnMut(NaiveDate) -> Option<i64>,
    ) -> Option<Self> {
        let end_at = now_timestamp.checked_add(1)?;
        let rolling_30_start_at = end_at.checked_sub(ROLLING_30_DAYS_SECONDS)?;
        let next_local_date = local_date.succ_opt()?;
        let today_start_at = resolve_midnight(local_date)?;
        let next_local_midnight_at = resolve_midnight(next_local_date)?;

        if today_start_at > end_at || end_at > next_local_midnight_at {
            return None;
        }

        Some(Self {
            today_start_at,
            rolling_30_start_at,
            end_at,
            next_local_midnight_at,
        })
    }
}

fn resolve_local_midnight<Tz>(timezone: &Tz, date: NaiveDate) -> Option<i64>
where
    Tz: TimeZone,
{
    let midnight = date.and_hms_opt(0, 0, 0)?;
    timestamp_from_local_result(timezone.from_local_datetime(&midnight))
}

fn timestamp_from_local_result<Tz>(result: LocalResult<DateTime<Tz>>) -> Option<i64>
where
    Tz: TimeZone,
{
    match result {
        LocalResult::Single(value) => Some(value.timestamp()),
        LocalResult::Ambiguous(first, second) => Some(first.timestamp().min(second.timestamp())),
        LocalResult::None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::domain::BillingKind;
    use crate::usage::status::{CostQuality, UsageStatus};
    use chrono::{Datelike, FixedOffset, LocalResult, NaiveDate, TimeZone};
    use serde_json::json;

    const DAY_SECONDS: i64 = 86_400;

    fn midnight_at(offset: FixedOffset, date: NaiveDate) -> i64 {
        offset
            .with_ymd_and_hms(date.year(), date.month(), date.day(), 0, 0, 0)
            .single()
            .unwrap()
            .timestamp()
    }

    #[test]
    fn ordinary_day_windows_are_half_open_and_include_the_current_second() {
        let offset = FixedOffset::east_opt(8 * 3_600).unwrap();
        let now = offset
            .with_ymd_and_hms(2026, 7, 16, 12, 30, 45)
            .single()
            .unwrap();

        let windows = TrayUsageWindows::from_zoned_now(now).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 7, 16).unwrap();
        let tomorrow = NaiveDate::from_ymd_opt(2026, 7, 17).unwrap();

        assert_eq!(windows.today_start_at, midnight_at(offset, today));
        assert_eq!(windows.end_at, now.timestamp() + 1);
        assert_eq!(
            windows.next_local_midnight_at,
            midnight_at(offset, tomorrow)
        );
        assert_eq!(
            windows.end_at - windows.rolling_30_start_at,
            30 * DAY_SECONDS,
        );
        assert!(windows.today_start_at <= now.timestamp());
        assert!(now.timestamp() < windows.end_at);
        assert!(windows.end_at <= windows.next_local_midnight_at);
    }

    #[test]
    fn last_second_of_the_day_uses_next_midnight_as_the_exclusive_end() {
        let offset = FixedOffset::east_opt(8 * 3_600).unwrap();
        let now = offset
            .with_ymd_and_hms(2026, 7, 16, 23, 59, 59)
            .single()
            .unwrap();

        let windows = TrayUsageWindows::from_zoned_now(now).unwrap();

        assert_eq!(windows.end_at, now.timestamp() + 1);
        assert_eq!(windows.end_at, windows.next_local_midnight_at);
        assert!(windows.end_at <= windows.next_local_midnight_at);
    }

    #[test]
    fn spring_forward_uses_the_next_local_calendar_midnight() {
        let standard = FixedOffset::west_opt(5 * 3_600).unwrap();
        let daylight = FixedOffset::west_opt(4 * 3_600).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();
        let tomorrow = NaiveDate::from_ymd_opt(2026, 3, 9).unwrap();
        let now = daylight
            .with_ymd_and_hms(2026, 3, 8, 12, 30, 0)
            .single()
            .unwrap();

        let windows =
            TrayUsageWindows::from_calendar_now(now.timestamp(), today, |date| match date {
                value if value == today => Some(midnight_at(standard, value)),
                value if value == tomorrow => Some(midnight_at(daylight, value)),
                _ => None,
            })
            .unwrap();

        assert_eq!(windows.today_start_at, 1_772_946_000);
        assert_eq!(windows.next_local_midnight_at, 1_773_028_800);
        assert_eq!(
            windows.next_local_midnight_at - windows.today_start_at,
            23 * 3_600,
        );
        assert_ne!(
            windows.next_local_midnight_at,
            windows.today_start_at + DAY_SECONDS,
        );
        assert_eq!(windows.end_at, now.timestamp() + 1);
    }

    #[test]
    fn fall_back_uses_the_next_local_calendar_midnight() {
        let daylight = FixedOffset::west_opt(4 * 3_600).unwrap();
        let standard = FixedOffset::west_opt(5 * 3_600).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        let tomorrow = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        let now = standard
            .with_ymd_and_hms(2026, 11, 1, 12, 30, 0)
            .single()
            .unwrap();

        let windows =
            TrayUsageWindows::from_calendar_now(now.timestamp(), today, |date| match date {
                value if value == today => Some(midnight_at(daylight, value)),
                value if value == tomorrow => Some(midnight_at(standard, value)),
                _ => None,
            })
            .unwrap();

        assert_eq!(windows.today_start_at, 1_793_505_600);
        assert_eq!(windows.next_local_midnight_at, 1_793_595_600);
        assert_eq!(
            windows.next_local_midnight_at - windows.today_start_at,
            25 * 3_600,
        );
        assert_ne!(
            windows.next_local_midnight_at,
            windows.today_start_at + DAY_SECONDS,
        );
        assert_eq!(
            windows.end_at - windows.rolling_30_start_at,
            30 * DAY_SECONDS,
        );
        assert!(windows.next_local_midnight_at > windows.end_at);
    }

    #[test]
    fn calendar_window_fails_when_either_local_midnight_cannot_be_resolved() {
        let today = NaiveDate::from_ymd_opt(2026, 7, 16).unwrap();

        assert!(TrayUsageWindows::from_calendar_now(1_000, today, |_| None).is_none());
    }

    #[test]
    fn calendar_window_rejects_timestamp_and_date_overflow() {
        let today = NaiveDate::from_ymd_opt(2026, 7, 16).unwrap();
        let midnight = 1_000;

        assert!(
            TrayUsageWindows::from_calendar_now(i64::MAX, today, |_| { Some(midnight) }).is_none()
        );
        assert!(
            TrayUsageWindows::from_calendar_now(i64::MIN, today, |_| { Some(midnight) }).is_none()
        );
        assert!(
            TrayUsageWindows::from_calendar_now(0, NaiveDate::MAX, |_| { Some(midnight) })
                .is_none()
        );
    }

    #[test]
    fn ambiguous_midnight_uses_the_earliest_instant_and_gap_returns_none() {
        let offset = FixedOffset::east_opt(0).unwrap();
        let earlier = offset.timestamp_opt(1_000, 0).single().unwrap();
        let later = offset.timestamp_opt(2_000, 0).single().unwrap();

        assert_eq!(
            timestamp_from_local_result(LocalResult::Ambiguous(later, earlier)),
            Some(1_000),
        );

        let gap: LocalResult<chrono::DateTime<FixedOffset>> = LocalResult::None;
        assert_eq!(timestamp_from_local_result(gap), None);
    }

    #[test]
    fn unknown_snapshot_has_safe_empty_defaults() {
        let snapshot = TrayUsageSnapshot::unknown(123);

        assert_eq!(snapshot.status, UsageStatus::Unknown);
        assert_eq!(snapshot.generated_at, 123);
        assert_eq!(snapshot.last_success_at, None);
        assert!(!snapshot.stale);
        assert_eq!(snapshot.refresh_error, None);
        assert!(!snapshot.refresh_in_progress);
        assert!(snapshot.agents.is_empty());
    }

    #[test]
    fn stale_snapshot_can_retain_a_known_alert() {
        let snapshot = TrayUsageSnapshot {
            status: UsageStatus::Red,
            generated_at: 200,
            last_success_at: Some(100),
            stale: true,
            refresh_error: Some("refresh_failed".to_string()),
            refresh_in_progress: false,
            agents: Vec::new(),
        };

        assert_eq!(snapshot.status, UsageStatus::Red);
        assert!(snapshot.stale);
        assert_eq!(snapshot.last_success_at, Some(100));
    }

    #[test]
    fn tray_snapshot_serializes_only_presentation_safe_fields() {
        let snapshot = TrayUsageSnapshot {
            status: UsageStatus::Yellow,
            generated_at: 200,
            last_success_at: Some(190),
            stale: false,
            refresh_error: None,
            refresh_in_progress: true,
            agents: vec![TrayAgentUsageView {
                agent_module_id: "codex".to_string(),
                name: "Codex".to_string(),
                sort_order: 1,
                status: UsageStatus::Yellow,
                providers: vec![TrayProviderUsageView {
                    provider_id: "openai".to_string(),
                    provider_name: "OpenAI".to_string(),
                    system_preset_key: Some("openai".to_string()),
                    billing_kind: BillingKind::Metered,
                    status: UsageStatus::Yellow,
                    warning_reason: Some("partial_cost".to_string()),
                    subscription: Some(TraySubscriptionUsageView {
                        plan_label: Some("Plus".to_string()),
                        windows: vec![TrayQuotaWindowView {
                            kind: "five_hour".to_string(),
                            used_percent: Some("50".to_string()),
                            remaining_percent: Some("50".to_string()),
                            resets_at: Some("2026-07-16T13:00:00Z".to_string()),
                            status: UsageStatus::Yellow,
                            unavailable_reason: None,
                        }],
                    }),
                    metered: Some(TrayMeteredUsageView {
                        today_cost_usd: Some("5".to_string()),
                        rolling_30_day_cost_usd: Some("80".to_string()),
                        daily_budget_usd: Some("10".to_string()),
                        budget_consumed_percent: Some("50".to_string()),
                        total_tokens: 42,
                        cost_quality: CostQuality::Partial,
                    }),
                }],
            }],
        };

        let value = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(
            value,
            json!({
                "status": "yellow",
                "generatedAt": 200,
                "lastSuccessAt": 190,
                "stale": false,
                "refreshError": null,
                "refreshInProgress": true,
                "agents": [{
                    "agentModuleId": "codex",
                    "name": "Codex",
                    "sortOrder": 1,
                    "status": "yellow",
                    "providers": [{
                        "providerId": "openai",
                        "providerName": "OpenAI",
                        "systemPresetKey": "openai",
                        "billingKind": "metered",
                        "status": "yellow",
                        "warningReason": "partial_cost",
                        "subscription": {
                            "planLabel": "Plus",
                            "windows": [{
                                "kind": "five_hour",
                                "usedPercent": "50",
                                "remainingPercent": "50",
                                "resetsAt": "2026-07-16T13:00:00Z",
                                "status": "yellow",
                                "unavailableReason": null
                            }]
                        },
                        "metered": {
                            "todayCostUsd": "5",
                            "rolling30DayCostUsd": "80",
                            "dailyBudgetUsd": "10",
                            "budgetConsumedPercent": "50",
                            "totalTokens": 42,
                            "costQuality": "partial"
                        }
                    }]
                }]
            }),
        );

        let serialized = serde_json::to_string(&snapshot).unwrap();
        let round_trip: TrayUsageSnapshot = serde_json::from_str(&serialized).unwrap();

        assert_eq!(round_trip, snapshot);
        assert!(serialized.contains("\"generatedAt\""));
        assert!(serialized.contains("\"agentModuleId\""));
        assert!(serialized.contains("\"rolling30DayCostUsd\""));
        for forbidden in [
            "apiKey",
            "oauthToken",
            "quotaConfig",
            "routeConfig",
            "credentialFingerprint",
            "rawPayload",
            "rawError",
            "lastError",
        ] {
            assert!(!serialized.contains(forbidden), "{forbidden}: {serialized}");
        }
    }
}
