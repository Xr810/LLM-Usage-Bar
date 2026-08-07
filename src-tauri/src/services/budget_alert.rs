//! 每日 API 预算提醒。
//!
//! 需求是「按量计费的 provider，每天花费达到我设定的额度时提醒我」。托盘快照
//! 已经算好了 `budget_consumed_percent`，这里只负责两件事：判断哪些额度今天刚
//! 越线，以及保证同一个额度一天只提醒一次。
//!
//! 去重账本按**本地日期**记账并持久化到 settings 表，所以重启应用不会把今天
//! 已经发过的提醒再发一遍；跨过午夜后账本里的旧日期会被丢掉，新的一天重新开始。

use std::collections::BTreeSet;

use rust_decimal::Decimal;

use crate::database::Database;
use crate::error::AppError;
use crate::settings::ApiBudgetMode;
use crate::usage::tray_snapshot::TrayUsageSnapshot;

/// 触发阈值：花费达到预算的 100%。
const ALERT_AT_PERCENT: i64 = 100;

const LEDGER_SETTING_KEY: &str = "api_budget_alert_ledger";

/// 一条待发提醒。文案由调用方按语言渲染，这里只给事实。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetAlert {
    /// 去重键：共享额度是 `shared`，单 provider 是 `provider:<id>`。
    pub scope_key: String,
    /// 展示名。共享额度没有名字，用 `None`。
    pub scope_label: Option<String>,
    pub spent_usd: String,
    pub budget_usd: String,
    pub consumed_percent: String,
}

/// 今天已经提醒过的额度。`date` 是本地日期（`YYYY-MM-DD`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AlertLedger {
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub alerted: BTreeSet<String>,
}

impl AlertLedger {
    /// 换一天就清空——账本只对「今天」有意义。
    fn rolled_to(&self, today: &str) -> Self {
        if self.date == today {
            self.clone()
        } else {
            Self {
                date: today.to_string(),
                alerted: BTreeSet::new(),
            }
        }
    }
}

fn crossed_threshold(consumed_percent: Option<&str>) -> bool {
    consumed_percent
        .and_then(|raw| Decimal::from_str_exact(raw.trim()).ok())
        .is_some_and(|percent| percent >= Decimal::from(ALERT_AT_PERCENT))
}

/// 挑出这份快照里今天刚越线、且账本上还没记过的额度。
///
/// 返回的账本已经把新提醒记进去了，调用方发完通知后直接持久化即可；先记账再
/// 发通知会在通知失败时漏掉提醒，先发再记账则可能重复——所以调用方拿到提醒后
/// 应当**发送成功与否都记账**，宁可漏一条也不要每次刷新都轰炸用户。
pub fn decide_alerts(
    snapshot: &TrayUsageSnapshot,
    today: &str,
    ledger: &AlertLedger,
) -> (Vec<BudgetAlert>, AlertLedger) {
    let mut ledger = ledger.rolled_to(today);
    let mut alerts = Vec::new();

    match snapshot.api_budget.mode {
        ApiBudgetMode::Shared => {
            let budget = &snapshot.api_budget;
            if crossed_threshold(budget.budget_consumed_percent.as_deref()) {
                let key = "shared".to_string();
                if ledger.alerted.insert(key.clone()) {
                    alerts.push(BudgetAlert {
                        scope_key: key,
                        scope_label: None,
                        spent_usd: budget.today_cost_usd.clone().unwrap_or_default(),
                        budget_usd: budget.daily_budget_usd.clone().unwrap_or_default(),
                        consumed_percent: budget
                            .budget_consumed_percent
                            .clone()
                            .unwrap_or_default(),
                    });
                }
            }
        }
        ApiBudgetMode::PerProvider => {
            for agent in &snapshot.agents {
                for provider in &agent.providers {
                    let Some(metered) = provider.metered.as_ref() else {
                        continue;
                    };
                    if !crossed_threshold(metered.budget_consumed_percent.as_deref()) {
                        continue;
                    }
                    let key = format!("provider:{}", provider.provider_id);
                    if !ledger.alerted.insert(key.clone()) {
                        continue;
                    }
                    alerts.push(BudgetAlert {
                        scope_key: key,
                        scope_label: Some(provider.provider_name.clone()),
                        spent_usd: metered.today_cost_usd.clone().unwrap_or_default(),
                        budget_usd: metered.daily_budget_usd.clone().unwrap_or_default(),
                        consumed_percent: metered
                            .budget_consumed_percent
                            .clone()
                            .unwrap_or_default(),
                    });
                }
            }
        }
    }

    (alerts, ledger)
}

pub fn load_ledger(db: &Database) -> AlertLedger {
    db.get_setting(LEDGER_SETTING_KEY)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save_ledger(db: &Database, ledger: &AlertLedger) -> Result<(), AppError> {
    let encoded = crate::database::to_json_string(ledger)?;
    db.set_setting(LEDGER_SETTING_KEY, &encoded)
}

/// 提醒文案。跟托盘菜单一样按 `settings.language` 选语言，未知语言回落中文。
pub fn render_alert(alert: &BudgetAlert, language: &str) -> (String, String) {
    let scope = alert.scope_label.as_deref();
    match language {
        "en" => (
            "Daily API budget reached".to_string(),
            match scope {
                Some(name) => format!(
                    "{name} has spent ${} of its ${} daily budget ({}%).",
                    alert.spent_usd, alert.budget_usd, alert.consumed_percent
                ),
                None => format!(
                    "Today's API spend is ${} of your ${} daily budget ({}%).",
                    alert.spent_usd, alert.budget_usd, alert.consumed_percent
                ),
            },
        ),
        "ja" => (
            "1日のAPI予算に到達しました".to_string(),
            match scope {
                Some(name) => format!(
                    "{name} は1日の予算 ${} のうち ${} を使用しました（{}%）。",
                    alert.budget_usd, alert.spent_usd, alert.consumed_percent
                ),
                None => format!(
                    "本日のAPI利用額は1日の予算 ${} のうち ${} です（{}%）。",
                    alert.budget_usd, alert.spent_usd, alert.consumed_percent
                ),
            },
        ),
        "zh-TW" => (
            "已達每日 API 預算".to_string(),
            match scope {
                Some(name) => format!(
                    "{name} 今日已花費 ${}，佔每日預算 ${} 的 {}%。",
                    alert.spent_usd, alert.budget_usd, alert.consumed_percent
                ),
                None => format!(
                    "今日 API 花費 ${}，佔每日預算 ${} 的 {}%。",
                    alert.spent_usd, alert.budget_usd, alert.consumed_percent
                ),
            },
        ),
        _ => (
            "已达每日 API 预算".to_string(),
            match scope {
                Some(name) => format!(
                    "{name} 今日已花费 ${}，占每日预算 ${} 的 {}%。",
                    alert.spent_usd, alert.budget_usd, alert.consumed_percent
                ),
                None => format!(
                    "今日 API 花费 ${}，占每日预算 ${} 的 {}%。",
                    alert.spent_usd, alert.budget_usd, alert.consumed_percent
                ),
            },
        ),
    }
}

/// 快照发布时的副作用入口：判断、发通知、记账。
///
/// 通知失败也照样记账——重试的代价是每次刷新都弹一次，比漏一条提醒更糟。
pub fn notify_for_snapshot(app: &tauri::AppHandle, snapshot: &TrayUsageSnapshot) {
    use tauri::Manager;

    let Some(state) = app.try_state::<crate::store::AppState>() else {
        return;
    };
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let ledger = load_ledger(&state.db);
    let (alerts, next_ledger) = decide_alerts(snapshot, &today, &ledger);
    if alerts.is_empty() {
        if next_ledger != ledger {
            let _ = save_ledger(&state.db, &next_ledger);
        }
        return;
    }

    let language = crate::settings::get_settings()
        .language
        .unwrap_or_else(|| "zh".to_string());
    for alert in &alerts {
        let (title, body) = render_alert(alert, &language);
        deliver(app, &title, &body);
    }
    if let Err(error) = save_ledger(&state.db, &next_ledger) {
        log::warn!("budget alert ledger persist failed: {error}");
    }
}

fn deliver(app: &tauri::AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;

    if let Err(error) = app.notification().builder().title(title).body(body).show() {
        log::warn!("budget alert notification failed: {error}");
    }
}

/// 启动时申请一次系统通知权限。macOS 上没授权的话 `show()` 会静默失败，
/// 等到真的超预算才申请就已经晚了——那条提醒会丢。
pub fn ensure_permission(app: &tauri::AppHandle) {
    use tauri_plugin_notification::{NotificationExt, PermissionState};

    match app.notification().permission_state() {
        Ok(PermissionState::Granted) => {}
        Ok(_) => {
            if let Err(error) = app.notification().request_permission() {
                log::warn!("notification permission request failed: {error}");
            }
        }
        Err(error) => log::warn!("notification permission state unavailable: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::domain::BillingKind;
    use crate::usage::status::{CostQuality, UsageStatus};
    use crate::usage::tray_snapshot::{
        TrayAgentUsageView, TrayApiBudgetView, TrayMeteredUsageView, TrayProviderRecentUsageView,
        TrayProviderUsageView, TrayUsageSnapshot,
    };

    fn shared_snapshot(consumed_percent: Option<&str>) -> TrayUsageSnapshot {
        let mut snapshot = TrayUsageSnapshot::unknown(0);
        snapshot.api_budget = TrayApiBudgetView {
            mode: ApiBudgetMode::Shared,
            provider_count: 1,
            today_cost_usd: Some("12.5".to_string()),
            daily_budget_usd: Some("10".to_string()),
            budget_consumed_percent: consumed_percent.map(str::to_string),
            cost_quality: CostQuality::Complete,
            status: UsageStatus::Red,
            warning_reason: None,
        };
        snapshot
    }

    fn metered_provider(id: &str, consumed_percent: Option<&str>) -> TrayProviderUsageView {
        TrayProviderUsageView {
            provider_id: id.to_string(),
            provider_name: format!("{id} name"),
            system_preset_key: None,
            billing_kind: BillingKind::Metered,
            status: UsageStatus::Red,
            warning_reason: None,
            recent_usage: TrayProviderRecentUsageView {
                start_at: 0,
                end_at: 0,
                today_tokens: 0,
                total_tokens: 0,
                today_cost_usd: None,
                total_cost_usd: None,
                cost_quality: CostQuality::Complete,
                most_used_model: None,
                trend_buckets: Vec::new(),
            },
            subscription: None,
            metered: Some(TrayMeteredUsageView {
                today_cost_usd: Some("7".to_string()),
                rolling_30_day_cost_usd: None,
                daily_budget_usd: Some("5".to_string()),
                budget_consumed_percent: consumed_percent.map(str::to_string),
                total_tokens: 0,
                cost_quality: CostQuality::Complete,
            }),
        }
    }

    fn per_provider_snapshot(providers: Vec<TrayProviderUsageView>) -> TrayUsageSnapshot {
        let mut snapshot = TrayUsageSnapshot::unknown(0);
        snapshot.api_budget.mode = ApiBudgetMode::PerProvider;
        snapshot.agents = vec![TrayAgentUsageView {
            agent_module_id: "agent".to_string(),
            name: "Agent".to_string(),
            sort_order: 0,
            status: UsageStatus::Red,
            providers,
        }];
        snapshot
    }

    #[test]
    fn alerts_once_the_shared_budget_is_fully_spent() {
        let (alerts, ledger) = decide_alerts(
            &shared_snapshot(Some("100")),
            "2026-08-04",
            &AlertLedger::default(),
        );
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].scope_key, "shared");
        assert_eq!(alerts[0].scope_label, None);
        assert_eq!(alerts[0].spent_usd, "12.5");
        assert_eq!(alerts[0].budget_usd, "10");
        assert!(ledger.alerted.contains("shared"));
        assert_eq!(ledger.date, "2026-08-04");
    }

    #[test]
    fn stays_quiet_below_the_threshold() {
        for percent in [
            None,
            Some("0"),
            Some("50"),
            Some("99.9"),
            Some("not a number"),
        ] {
            let (alerts, _) = decide_alerts(
                &shared_snapshot(percent),
                "2026-08-04",
                &AlertLedger::default(),
            );
            assert!(alerts.is_empty(), "{percent:?} must not alert");
        }
    }

    #[test]
    fn does_not_repeat_the_same_alert_within_one_day() {
        let snapshot = shared_snapshot(Some("140"));
        let (first, ledger) = decide_alerts(&snapshot, "2026-08-04", &AlertLedger::default());
        assert_eq!(first.len(), 1);
        let (second, ledger) = decide_alerts(&snapshot, "2026-08-04", &ledger);
        assert!(second.is_empty(), "a second refresh must stay silent");
        assert_eq!(ledger.alerted.len(), 1);
    }

    #[test]
    fn alerts_again_after_the_local_date_rolls_over() {
        let snapshot = shared_snapshot(Some("140"));
        let (_, ledger) = decide_alerts(&snapshot, "2026-08-04", &AlertLedger::default());
        let (next_day, ledger) = decide_alerts(&snapshot, "2026-08-05", &ledger);
        assert_eq!(next_day.len(), 1, "a new day earns a new alert");
        assert_eq!(ledger.date, "2026-08-05");
        assert_eq!(
            ledger.alerted.len(),
            1,
            "yesterday's entries must not accumulate"
        );
    }

    #[test]
    fn per_provider_mode_alerts_each_over_budget_provider_separately() {
        let snapshot = per_provider_snapshot(vec![
            metered_provider("over-one", Some("101")),
            metered_provider("under", Some("40")),
            metered_provider("over-two", Some("300")),
        ]);
        let (alerts, ledger) = decide_alerts(&snapshot, "2026-08-04", &AlertLedger::default());
        let keys: Vec<&str> = alerts.iter().map(|a| a.scope_key.as_str()).collect();
        assert_eq!(keys, ["provider:over-one", "provider:over-two"]);
        assert_eq!(alerts[0].scope_label.as_deref(), Some("over-one name"));

        let (repeat, _) = decide_alerts(&snapshot, "2026-08-04", &ledger);
        assert!(repeat.is_empty());
    }

    #[test]
    fn shared_mode_ignores_per_provider_overspend_and_vice_versa() {
        // 共享额度模式下单个 provider 越线不该单独报，否则用户会收到两份提醒。
        let mut snapshot = per_provider_snapshot(vec![metered_provider("over", Some("400"))]);
        snapshot.api_budget = shared_snapshot(Some("10")).api_budget;
        let (alerts, _) = decide_alerts(&snapshot, "2026-08-04", &AlertLedger::default());
        assert!(alerts.is_empty());
    }

    #[test]
    fn subscription_providers_never_trigger_a_spend_alert() {
        let mut provider = metered_provider("subscription", Some("999"));
        provider.billing_kind = BillingKind::Subscription;
        provider.metered = None;
        let snapshot = per_provider_snapshot(vec![provider]);
        let (alerts, _) = decide_alerts(&snapshot, "2026-08-04", &AlertLedger::default());
        assert!(alerts.is_empty());
    }

    #[test]
    fn rendered_copy_names_the_provider_in_per_provider_mode() {
        let alert = BudgetAlert {
            scope_key: "provider:x".to_string(),
            scope_label: Some("OpenRouter".to_string()),
            spent_usd: "7".to_string(),
            budget_usd: "5".to_string(),
            consumed_percent: "140".to_string(),
        };
        for language in ["zh", "en", "ja", "zh-TW", "de"] {
            let (title, body) = render_alert(&alert, language);
            assert!(!title.is_empty(), "{language}");
            assert!(body.contains("OpenRouter"), "{language}: {body}");
            assert!(
                body.contains('7') && body.contains('5'),
                "{language}: {body}"
            );
        }
    }

    #[test]
    fn ledger_survives_a_json_round_trip() {
        let (_, ledger) = decide_alerts(
            &shared_snapshot(Some("120")),
            "2026-08-04",
            &AlertLedger::default(),
        );
        let encoded = serde_json::to_string(&ledger).unwrap();
        let decoded: AlertLedger = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, ledger);
    }
}
