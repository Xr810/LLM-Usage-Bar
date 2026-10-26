//! 每日 API 预算提醒。
//!
//! 需求是「按量计费的 provider，每天花费达到我设定的额度时提醒我」。是否越线
//! 只看 C10 托盘预算视图上的 `level`，和 `classify_metered` 是同一处判断：
//! 共享额度看 `TrayApiBudgetView.level`，按 provider 看
//! `TrayMeteredUsageView.level`。只有 `reached`（今日已知花费达到日预算）才
//! 提醒。废弃的 `warn` 不提醒，这里也不再单独比较百分比。主线没有 C12
//! `get_guard_state`，本模块不读它。
//!
//! 去重账本按**本地日期**记账并持久化到 settings 表，所以重启应用不会把今天
//! 已经发过的提醒再发一遍；跨过午夜后账本里的旧日期会被丢掉，新的一天重新开始。

use std::collections::BTreeSet;

use crate::config::settings::ApiBudgetMode;
use crate::error::AppError;
use crate::store::Database;
use crate::usage::tray_snapshot::TrayUsageSnapshot;

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

/// 挑出这份快照里今天刚越线、且账本上还没记过的额度。
///
/// 越线只认 `BudgetLevel::Reached`。`ok` 和废弃的 `warn` 都不提醒。
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
            if budget.level.is_reached() {
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
                    if !metered.level.is_reached() {
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
    let encoded = crate::store::to_json_string(ledger)?;
    db.set_setting(LEDGER_SETTING_KEY, &encoded)
}

/// 提醒文案。跟托盘菜单一样使用嵌入式语言目录，未知语言回落英文。
pub fn render_alert(alert: &BudgetAlert, language: &str) -> (String, String) {
    let texts = crate::tray::NotificationTexts::from_language(language);
    let body = texts.budget_reached_body(
        alert.scope_label.as_deref(),
        &alert.spent_usd,
        &alert.budget_usd,
        &alert.consumed_percent,
    );
    (texts.budget_reached_title, body)
}

/// 快照发布时的副作用入口：判断、发通知、记账。
///
/// 通知失败也照样记账——重试的代价是每次刷新都弹一次，比漏一条提醒更糟。
pub fn notify_for_snapshot(app: &tauri::AppHandle, snapshot: &TrayUsageSnapshot) {
    use tauri::Manager;

    let Some(state) = app.try_state::<crate::app_state::AppState>() else {
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

    let settings = crate::config::settings::get_settings();
    let language = crate::i18n::effective_language(settings.language.as_deref());
    for alert in &alerts {
        let (title, body) = render_alert(alert, language);
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
    use crate::model::{BillingKind, BudgetLevel};
    use crate::usage::status::{CostQuality, PaceBasis, UsageStatus};
    use crate::usage::tray_snapshot::{
        TrayAgentUsageView, TrayApiBudgetView, TrayMeteredUsageView, TrayProviderRecentUsageView,
        TrayProviderUsageView, TrayUsageSnapshot,
    };

    fn shared_snapshot(consumed_percent: Option<&str>, level: BudgetLevel) -> TrayUsageSnapshot {
        let mut snapshot = TrayUsageSnapshot::unknown(0);
        snapshot.api_budget = TrayApiBudgetView {
            mode: ApiBudgetMode::Shared,
            provider_count: 1,
            today_cost_usd: Some("12.5".to_string()),
            daily_budget_usd: Some("10".to_string()),
            budget_consumed_percent: consumed_percent.map(str::to_string),
            cost_quality: CostQuality::Complete,
            status: UsageStatus::Red,
            level,
            warning_reason: None,
            burn_rate_usd_per_hour: None,
            projected_exhaust_at: None,
            headroom_ratio: None,
            pace_basis: PaceBasis::Static,
            rhythm_adjustment: None,
            flat_status: None,
            ..TrayApiBudgetView::default()
        };
        snapshot
    }

    fn metered_provider(
        id: &str,
        consumed_percent: Option<&str>,
        level: BudgetLevel,
    ) -> TrayProviderUsageView {
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
                level,
                total_tokens: 0,
                cost_quality: CostQuality::Complete,
                rhythm_adjustment: None,
                flat_status: None,
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
            &shared_snapshot(Some("100"), BudgetLevel::Reached),
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
    fn stays_quiet_when_the_level_is_not_reached() {
        for percent in [
            None,
            Some("0"),
            Some("50"),
            Some("99.9"),
            Some("100"),
            Some("140"),
            Some("not a number"),
        ] {
            let (alerts, _) = decide_alerts(
                &shared_snapshot(percent, BudgetLevel::Ok),
                "2026-08-04",
                &AlertLedger::default(),
            );
            assert!(alerts.is_empty(), "{percent:?} must not alert");
        }
    }

    #[test]
    fn alerts_follow_the_level_even_when_the_percent_disagrees() {
        let (quiet, _) = decide_alerts(
            &shared_snapshot(Some("100"), BudgetLevel::Ok),
            "2026-08-04",
            &AlertLedger::default(),
        );
        assert!(quiet.is_empty());

        let (fired, _) = decide_alerts(
            &shared_snapshot(Some("40"), BudgetLevel::Reached),
            "2026-08-04",
            &AlertLedger::default(),
        );
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].consumed_percent, "40");
    }

    #[test]
    #[allow(deprecated)]
    fn deprecated_warn_level_does_not_alert() {
        let (alerts, _) = decide_alerts(
            &shared_snapshot(Some("200"), BudgetLevel::Warn),
            "2026-08-04",
            &AlertLedger::default(),
        );
        assert!(alerts.is_empty());

        let decoded = serde_json::from_str::<TrayApiBudgetView>(
            r#"{
                "mode": "shared",
                "providerCount": 1,
                "todayCostUsd": "12.5",
                "dailyBudgetUsd": "10",
                "budgetConsumedPercent": "200",
                "costQuality": "complete",
                "status": "red",
                "warningReason": null,
                "level": "warn"
            }"#,
        )
        .unwrap();
        assert!(!decoded.level.is_reached());
    }

    #[test]
    fn does_not_repeat_the_same_alert_within_one_day() {
        let snapshot = shared_snapshot(Some("140"), BudgetLevel::Reached);
        let (first, ledger) = decide_alerts(&snapshot, "2026-08-04", &AlertLedger::default());
        assert_eq!(first.len(), 1);
        let (second, ledger) = decide_alerts(&snapshot, "2026-08-04", &ledger);
        assert!(second.is_empty(), "a second refresh must stay silent");
        assert_eq!(ledger.alerted.len(), 1);
    }

    #[test]
    fn alerts_again_after_the_local_date_rolls_over() {
        let snapshot = shared_snapshot(Some("140"), BudgetLevel::Reached);
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
            metered_provider("over-one", Some("101"), BudgetLevel::Reached),
            metered_provider("under", Some("40"), BudgetLevel::Ok),
            metered_provider("over-two", Some("300"), BudgetLevel::Reached),
        ]);
        let (alerts, ledger) = decide_alerts(&snapshot, "2026-08-04", &AlertLedger::default());
        let keys: Vec<&str> = alerts.iter().map(|a| a.scope_key.as_str()).collect();
        assert_eq!(keys, ["provider:over-one", "provider:over-two"]);
        assert_eq!(alerts[0].scope_label.as_deref(), Some("over-one name"));

        let (repeat, _) = decide_alerts(&snapshot, "2026-08-04", &ledger);
        assert!(repeat.is_empty());
    }

    #[test]
    fn per_provider_alerts_follow_each_metered_level() {
        let snapshot = per_provider_snapshot(vec![
            metered_provider("reached-low-percent", Some("40"), BudgetLevel::Reached),
            metered_provider("ok-over-percent", Some("180"), BudgetLevel::Ok),
        ]);
        let (alerts, _) = decide_alerts(&snapshot, "2026-08-04", &AlertLedger::default());
        assert_eq!(
            alerts
                .iter()
                .map(|alert| alert.scope_key.as_str())
                .collect::<Vec<_>>(),
            ["provider:reached-low-percent"]
        );
    }

    #[test]
    fn shared_mode_ignores_per_provider_overspend_and_vice_versa() {
        // 共享额度模式下单个 provider 越线不该单独报，否则用户会收到两份提醒。
        let mut snapshot = per_provider_snapshot(vec![metered_provider(
            "over",
            Some("400"),
            BudgetLevel::Reached,
        )]);
        snapshot.api_budget = shared_snapshot(Some("10"), BudgetLevel::Ok).api_budget;
        let (alerts, _) = decide_alerts(&snapshot, "2026-08-04", &AlertLedger::default());
        assert!(alerts.is_empty());
    }

    #[test]
    fn subscription_providers_never_trigger_a_spend_alert() {
        let mut provider = metered_provider("subscription", Some("999"), BudgetLevel::Reached);
        provider.billing_kind = BillingKind::Subscription;
        provider.metered = None;
        let snapshot = per_provider_snapshot(vec![provider]);
        let (alerts, _) = decide_alerts(&snapshot, "2026-08-04", &AlertLedger::default());
        assert!(alerts.is_empty());
    }

    #[test]
    fn rendered_copy_preserves_amounts_scope_and_catalog_fallbacks() {
        let mut alert = BudgetAlert {
            scope_key: "provider:x".to_string(),
            scope_label: Some("OpenRouter".to_string()),
            spent_usd: "7.1250".to_string(),
            budget_usd: "5.00".to_string(),
            consumed_percent: "142.50".to_string(),
        };
        for (language, title, scoped, shared) in [
            (
                "en",
                "Daily API budget reached",
                "OpenRouter has spent $7.1250 of its $5.00 daily budget (142.50%).",
                "Today's API spend is $7.1250 of your $5.00 daily budget (142.50%).",
            ),
            (
                "zh",
                "已达每日 API 预算",
                "OpenRouter 今日已花费 $7.1250，占每日预算 $5.00 的 142.50%。",
                "今日 API 花费 $7.1250，占每日预算 $5.00 的 142.50%。",
            ),
            (
                "zh-TW",
                "已達每日 API 預算",
                "OpenRouter 今日已花費 $7.1250，佔每日預算 $5.00 的 142.50%。",
                "今日 API 花費 $7.1250，佔每日預算 $5.00 的 142.50%。",
            ),
            (
                "ja",
                "1日のAPI予算に到達しました",
                "OpenRouter は1日の予算 $5.00 のうち $7.1250 を使用しました（142.50%）。",
                "本日のAPI利用額は1日の予算 $5.00 のうち $7.1250 です（142.50%）。",
            ),
        ] {
            alert.scope_label = Some("OpenRouter".to_string());
            assert_eq!(
                render_alert(&alert, language),
                (title.to_string(), scoped.to_string())
            );
            alert.scope_label = None;
            assert_eq!(
                render_alert(&alert, language),
                (title.to_string(), shared.to_string())
            );
        }
        for scope in [None, Some("Provider {{spent}} <&>".to_string())] {
            alert.scope_label = scope;
            assert_eq!(render_alert(&alert, "unknown"), render_alert(&alert, "en"));
            if let Some(scope) = &alert.scope_label {
                assert!(render_alert(&alert, "en").1.starts_with(scope));
            }
        }
    }

    #[test]
    fn ledger_survives_a_json_round_trip() {
        let (_, ledger) = decide_alerts(
            &shared_snapshot(Some("120"), BudgetLevel::Reached),
            "2026-08-04",
            &AlertLedger::default(),
        );
        let encoded = serde_json::to_string(&ledger).unwrap();
        let decoded: AlertLedger = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, ledger);
    }
}
