use crate::credentials::codex_oauth_auth::CodexOAuthManager;
use crate::database::Database;
use crate::error::AppError;
use crate::services::coding_plan::get_coding_plan_quota;
use crate::services::subscription::{
    get_subscription_quota, query_managed_codex_oauth_quota, SubscriptionQuota, TIER_FIVE_HOUR,
    TIER_SEVEN_DAY, TIER_WEEKLY_LIMIT,
};
use crate::usage::domain::{
    BillingKind, QuotaFetchState, QuotaSnapshot, QuotaStatusView, UsageProviderStored,
};
use crate::usage::system_providers::{CLAUDE_LOCAL_QUOTA_SOURCE, MANAGED_CODEX_QUOTA_SOURCE};
use futures::future::BoxFuture;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::async_runtime::JoinHandle;
use tokio::sync::{watch, Mutex as AsyncMutex, RwLock};
use uuid::Uuid;

/// 空闲时的默认刷新间隔(秒)。有用量活动时,调度器会在更短的去抖窗口内
/// 补刷一次(见 ACTIVE_* 常量),所以空闲间隔可以放宽到 15 分钟。
pub const DEFAULT_QUOTA_INTERVAL_SECONDS: u64 = 900;
const SCHEDULER_TICK_SECONDS: u64 = 30;
const FAILURE_RETRY_DELAYS_SECONDS: [u64; 5] = [30, 60, 300, 600, 1_800];
/// 手动刷新的冷却:60 秒内重复点击复用上一次结果,不再向外部取数。
const MANUAL_REFRESH_MIN_INTERVAL_SECS: i64 = 60;
/// 活动触发刷新的去抖窗口:活动标记写入后等这么久再刷,合并密集写入。
const ACTIVITY_DEBOUNCE_SECS: i64 = 30;
/// 活动期间两次刷新的最小间隔:再活跃也至少隔这么久(失败阶梯优先)。
const ACTIVE_MIN_SPACING_SECS: i64 = 120;

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedQuota {
    pub five_hour_utilization_percent: Option<String>,
    pub five_hour_resets_at: Option<String>,
    pub seven_day_utilization_percent: Option<String>,
    pub seven_day_resets_at: Option<String>,
    pub manual_resets_remaining: Option<i64>,
    pub raw_payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaRefreshResult {
    pub snapshot: QuotaStatusView,
    pub fetch_state: QuotaFetchState,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuotaSchedulerCycle {
    pub attempted: u32,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaSchedulerOutcome {
    Completed { attempted: u32, had_errors: bool },
    Failed,
}

pub type QuotaCycleCallback =
    Arc<dyn Fn(QuotaSchedulerOutcome) -> BoxFuture<'static, ()> + Send + Sync + 'static>;

/// Object-safe boundary around existing quota HTTP/parsing implementations.
/// The adapter returns their common `SubscriptionQuota` representation so this
/// module only owns normalization and persistence semantics.
///
/// `interactive` marks user-initiated refreshes (manual refresh button), which
/// layers behind consent gates may treat differently from background cycles
/// (e.g. prompting the macOS Keychain dialog only on user action).
pub trait QuotaCollector: Send + Sync {
    fn source(&self) -> &'static str;

    fn collect<'a>(
        &'a self,
        provider: &'a UsageProviderStored,
        interactive: bool,
    ) -> BoxFuture<'a, Result<SubscriptionQuota, String>>;
}

struct SubscriptionQuotaCollector {
    source: &'static str,
}

impl QuotaCollector for SubscriptionQuotaCollector {
    fn source(&self) -> &'static str {
        self.source
    }

    fn collect<'a>(
        &'a self,
        _provider: &'a UsageProviderStored,
        _interactive: bool,
    ) -> BoxFuture<'a, Result<SubscriptionQuota, String>> {
        Box::pin(async move { get_subscription_quota(self.source).await })
    }
}

/// Claude 订阅额度的三层回退链:官方 OAuth 接口 → CLI 探测 → 本地文件。
///
/// 前两层失败以稳定错误码降级,第三层(现有本地采集)是零成本兜底;
/// 三层全败时错误串只保留各层错误码的摘要,绝不透传凭据。
type ClaudeStage =
    Arc<dyn Fn(bool) -> BoxFuture<'static, Result<SubscriptionQuota, String>> + Send + Sync>;

struct ClaudeChainCollector {
    oauth_stage: ClaudeStage,
    probe_stage: ClaudeStage,
    local_stage: ClaudeStage,
}

impl ClaudeChainCollector {
    fn production() -> Self {
        Self {
            oauth_stage: Arc::new(|interactive| {
                Box::pin(async move {
                    crate::usage::claude_oauth::collect_claude_oauth_quota(interactive).await
                })
            }),
            probe_stage: Arc::new(|interactive| {
                Box::pin(async move { crate::usage::cli_probe::probe_claude_usage(interactive).await })
            }),
            local_stage: Arc::new(|_| {
                Box::pin(async move { crate::claude_quota::collect_local_quota() })
            }),
        }
    }

    #[cfg(test)]
    fn with_stages(oauth_stage: ClaudeStage, probe_stage: ClaudeStage, local_stage: ClaudeStage) -> Self {
        Self {
            oauth_stage,
            probe_stage,
            local_stage,
        }
    }
}

impl QuotaCollector for ClaudeChainCollector {
    fn source(&self) -> &'static str {
        CLAUDE_LOCAL_QUOTA_SOURCE
    }

    fn collect<'a>(
        &'a self,
        _provider: &'a UsageProviderStored,
        interactive: bool,
    ) -> BoxFuture<'a, Result<SubscriptionQuota, String>> {
        let oauth_stage = self.oauth_stage.clone();
        let probe_stage = self.probe_stage.clone();
        let local_stage = self.local_stage.clone();
        Box::pin(async move {
            let mut chain_errors = Vec::new();
            for (label, stage) in [
                ("oauth", &oauth_stage),
                ("probe", &probe_stage),
                ("local", &local_stage),
            ] {
                match stage(interactive).await {
                    Ok(quota) if quota.success => return Ok(quota),
                    Ok(quota) => {
                        let error = quota
                            .error
                            .or(quota.credential_message)
                            .unwrap_or_else(|| "quota collection failed".to_string());
                        chain_errors.push(stage_error(label, &error));
                    }
                    Err(error) => chain_errors.push(stage_error(label, &error)),
                }
            }
            Err(format!("claude quota chain failed: {}", chain_errors.join(" → ")))
        })
    }
}

/// 层摘要只保留稳定错误码的前缀段,截断长文案,避免链错误串夹带路径等细节。
fn stage_error(label: &str, error: &str) -> String {
    let cap = error.char_indices().nth(64).map(|(index, _)| index).unwrap_or(error.len());
    format!("{label}({})", &error[..cap])
}

// ── 用量活动信号(自适应刷新) ─────────────────────────────

/// 用量活动标记:provider_id → 最近一次用量写入的 Unix 秒。
/// 写入路径(session 同步、事件摄入)没有 QuotaService 句柄,
/// 用进程级静态表传递信号;调度器 tick 时消费(去抖后补刷一次并清除)。
static SUBSCRIPTION_ACTIVITY: OnceLock<std::sync::RwLock<HashMap<String, i64>>> = OnceLock::new();

fn subscription_activity() -> &'static std::sync::RwLock<HashMap<String, i64>> {
    SUBSCRIPTION_ACTIVITY.get_or_init(|| std::sync::RwLock::new(HashMap::new()))
}

/// 用量写入路径调用:给对应 Provider 打活动标记(触发一次补刷)。
pub(crate) fn mark_subscription_activity(provider_id: &str) {
    let now = now_timestamp().unwrap_or(0);
    if let Ok(mut map) = subscription_activity().write() {
        map.insert(provider_id.to_string(), now);
    }
}

#[cfg(test)]
fn mark_subscription_activity_at(provider_id: &str, now: i64) {
    if let Ok(mut map) = subscription_activity().write() {
        map.insert(provider_id.to_string(), now);
    }
}

fn read_subscription_activity(provider_id: &str) -> Option<i64> {
    subscription_activity()
        .read()
        .ok()?
        .get(provider_id)
        .copied()
}

fn clear_subscription_activity(provider_id: &str) {
    if let Ok(mut map) = subscription_activity().write() {
        map.remove(provider_id);
    }
}

/// 空闲基线到期:距上次尝试超过间隔(或失败阶梯)。
fn idle_due(state: Option<&QuotaFetchState>, interval: u64, now: i64) -> bool {
    let Some(state) = state else { return true };
    let Some(last_attempt) = state.last_attempt_at else {
        return true;
    };
    let delay = quota_refresh_delay_seconds(state, interval);
    now.saturating_sub(last_attempt) >= delay as i64
}

/// 活动触发到期:标记写入超过去抖窗口、距上次尝试超过活跃最小间隔、
/// 且失败阶梯已让路,才允许补刷。每次标记最多触发一次(触发即清)。
fn activity_due(provider_id: &str, state: Option<&QuotaFetchState>, now: i64) -> bool {
    let Some(activity_at) = read_subscription_activity(provider_id) else {
        return false;
    };
    let Some(state) = state else {
        return false;
    };
    let Some(last_attempt) = state.last_attempt_at else {
        return false;
    };
    // 活动必须发生在最近一次尝试之后,且过完去抖窗口。
    if last_attempt >= activity_at || now.saturating_sub(activity_at) < ACTIVITY_DEBOUNCE_SECS {
        return false;
    }
    if state.consecutive_failures > 0 {
        // 失败阶梯优先:退避期内不因活动补刷。
        let backoff = quota_refresh_delay_seconds(state, 0) as i64;
        return now.saturating_sub(last_attempt) >= backoff;
    }
    // 再活跃也至少隔 ACTIVITY_MIN_SPACING 秒。
    now.saturating_sub(last_attempt) >= ACTIVE_MIN_SPACING_SECS
}

struct CodingPlanQuotaCollector;

impl QuotaCollector for CodingPlanQuotaCollector {
    fn source(&self) -> &'static str {
        "coding_plan"
    }

    fn collect<'a>(
        &'a self,
        provider: &'a UsageProviderStored,
        _interactive: bool,
    ) -> BoxFuture<'a, Result<SubscriptionQuota, String>> {
        Box::pin(async move {
            let empty_config = Value::Null;
            let config = provider.quota_config.as_ref().unwrap_or(&empty_config);
            let string = |keys: &[&str]| {
                keys.iter()
                    .find_map(|key| config.get(key).and_then(Value::as_str))
                    .unwrap_or("")
            };
            let optional = |keys: &[&str]| {
                keys.iter()
                    .find_map(|key| config.get(key).and_then(Value::as_str))
                    .filter(|value| !value.trim().is_empty())
            };
            get_coding_plan_quota(
                string(&["base_url", "baseUrl"]),
                string(&["api_key", "apiKey"]),
                optional(&["coding_plan_provider", "codingPlanProvider"]),
                optional(&["access_key_id", "accessKeyId"]),
                optional(&["secret_access_key", "secretAccessKey"]),
                optional(&["team_organization_id", "teamOrganizationId"]),
                optional(&["team_project_id", "teamProjectId"]),
            )
            .await
        })
    }
}

struct ManagedCodexOAuthQuotaCollector {
    manager: Arc<RwLock<CodexOAuthManager>>,
}

impl QuotaCollector for ManagedCodexOAuthQuotaCollector {
    fn source(&self) -> &'static str {
        MANAGED_CODEX_QUOTA_SOURCE
    }

    fn collect<'a>(
        &'a self,
        _provider: &'a UsageProviderStored,
        _interactive: bool,
    ) -> BoxFuture<'a, Result<SubscriptionQuota, String>> {
        Box::pin(async move { query_managed_codex_oauth_quota(&self.manager, None).await })
    }
}

/// ChatGPT/Codex 订阅额度的三层回退链:自管 OAuth → CLI 凭据 → CLI 探测。
///
/// 链在 `codex_oauth` 这个 key 下替换原来的单层 collector;第三层
/// (portable-pty `codex /status`)只在自管与 CLI 凭据都拿不到时才触发。
/// 注意:降级到 CLI 凭据时,显示的账号可能与应用自管账号不同——这是
/// 「有额度可看」对「精确账号」的取舍,且 CLI 凭据正是用户正在使用的账号。
type CodexStage =
    Arc<dyn Fn(bool) -> BoxFuture<'static, Result<SubscriptionQuota, String>> + Send + Sync>;

struct CodexChainCollector {
    managed_stage: CodexStage,
    cli_stage: CodexStage,
    probe_stage: CodexStage,
}

impl CodexChainCollector {
    fn production(codex_oauth_manager: Arc<RwLock<CodexOAuthManager>>) -> Self {
        let manager_for_stage = codex_oauth_manager.clone();
        Self {
            managed_stage: Arc::new(move |_interactive| {
                let manager = manager_for_stage.clone();
                Box::pin(async move { query_managed_codex_oauth_quota(&manager, None).await })
            }),
            cli_stage: Arc::new(|_interactive| {
                Box::pin(async move { get_subscription_quota("codex").await })
            }),
            probe_stage: Arc::new(|interactive| {
                Box::pin(
                    async move { crate::usage::cli_probe::probe_codex_status(interactive).await },
                )
            }),
        }
    }

    #[cfg(test)]
    fn with_stages(managed_stage: CodexStage, cli_stage: CodexStage, probe_stage: CodexStage) -> Self {
        Self {
            managed_stage,
            cli_stage,
            probe_stage,
        }
    }
}

impl QuotaCollector for CodexChainCollector {
    fn source(&self) -> &'static str {
        MANAGED_CODEX_QUOTA_SOURCE
    }

    fn collect<'a>(
        &'a self,
        _provider: &'a UsageProviderStored,
        interactive: bool,
    ) -> BoxFuture<'a, Result<SubscriptionQuota, String>> {
        let managed_stage = self.managed_stage.clone();
        let cli_stage = self.cli_stage.clone();
        let probe_stage = self.probe_stage.clone();
        Box::pin(async move {
            let mut chain_errors = Vec::new();
            for (label, stage) in [
                ("managed", &managed_stage),
                ("cli", &cli_stage),
                ("probe", &probe_stage),
            ] {
                match stage(interactive).await {
                    Ok(quota) if quota.success => return Ok(quota),
                    Ok(quota) => {
                        let error = quota
                            .error
                            .or(quota.credential_message)
                            .unwrap_or_else(|| "quota collection failed".to_string());
                        chain_errors.push(stage_error(label, &error));
                    }
                    Err(error) => chain_errors.push(stage_error(label, &error)),
                }
            }
            Err(format!("codex quota chain failed: {}", chain_errors.join(" → ")))
        })
    }
}

#[derive(Clone)]
pub struct QuotaService {
    db: Arc<Database>,
    collectors: Arc<HashMap<String, Arc<dyn QuotaCollector>>>,
    in_flight: Arc<AsyncMutex<HashMap<String, Arc<QuotaFlight>>>>,
    /// provider_id → (手动刷新时间, 上次结果)。冷却期内的重复手动刷新
    /// 复用上次结果(成功或失败都缓存),避免狂点刷新打外部接口。
    last_manual: Arc<RwLock<HashMap<String, (i64, Result<QuotaRefreshResult, String>)>>>,
    /// 手动刷新冷却(秒),测试可调 0 禁用。
    manual_refresh_cooldown_secs: i64,
}

type SharedQuotaRefreshResult = Result<QuotaRefreshResult, String>;

struct QuotaFlight {
    result: watch::Sender<Option<SharedQuotaRefreshResult>>,
}

impl QuotaService {
    pub fn new(db: Arc<Database>) -> Self {
        Self::with_collectors(db, Self::legacy_collectors())
    }

    pub fn production(
        db: Arc<Database>,
        codex_oauth_manager: Arc<RwLock<CodexOAuthManager>>,
    ) -> Self {
        let mut collectors = Self::legacy_collectors();
        collectors.push(Arc::new(CodexChainCollector::production(
            codex_oauth_manager,
        )));
        Self::with_collectors(db, collectors)
    }

    fn legacy_collectors() -> Vec<Arc<dyn QuotaCollector>> {
        vec![
            Arc::new(SubscriptionQuotaCollector { source: "claude" }),
            Arc::new(SubscriptionQuotaCollector { source: "codex" }),
            Arc::new(ClaudeChainCollector::production()),
            Arc::new(CodingPlanQuotaCollector),
        ]
    }

    pub fn with_collectors(db: Arc<Database>, collectors: Vec<Arc<dyn QuotaCollector>>) -> Self {
        let collectors = collectors
            .into_iter()
            .map(|collector| (collector.source().to_string(), collector))
            .collect();
        Self {
            db,
            collectors: Arc::new(collectors),
            in_flight: Arc::new(AsyncMutex::new(HashMap::new())),
            last_manual: Arc::new(RwLock::new(HashMap::new())),
            manual_refresh_cooldown_secs: MANUAL_REFRESH_MIN_INTERVAL_SECS,
        }
    }

    /// 测试用:调整手动刷新冷却(0 表示禁用)。
    #[cfg(test)]
    pub fn with_manual_refresh_cooldown(mut self, secs: i64) -> Self {
        self.manual_refresh_cooldown_secs = secs;
        self
    }

    pub async fn refresh_provider(
        &self,
        provider_id: &str,
    ) -> Result<QuotaRefreshResult, AppError> {
        // 手动刷新是用户主动操作,允许触发同意门控的系统授权对话框;
        // 60 秒内的重复点击复用上一次结果,不再向外部取数。
        self.refresh_provider_manual_at(provider_id, now_timestamp()?)
            .await
    }

    pub(crate) async fn refresh_provider_manual_at(
        &self,
        provider_id: &str,
        now: i64,
    ) -> Result<QuotaRefreshResult, AppError> {
        if let Some((attempted_at, cached)) = self.last_manual.read().await.get(provider_id) {
            if now.saturating_sub(*attempted_at) < self.manual_refresh_cooldown_secs {
                return cached.clone().map_err(AppError::Message);
            }
        }
        let result = self.refresh_provider_at(provider_id, now, true).await;
        let cached = match &result {
            Ok(ok) => Ok(ok.clone()),
            Err(error) => Err(error.to_string()),
        };
        self.last_manual
            .write()
            .await
            .insert(provider_id.to_string(), (now, cached));
        result
    }

    async fn refresh_provider_at(
        &self,
        provider_id: &str,
        attempted_at: i64,
        interactive: bool,
    ) -> Result<QuotaRefreshResult, AppError> {
        let (flight, is_leader) = {
            let mut in_flight = self.in_flight.lock().await;
            if let Some(flight) = in_flight.get(provider_id) {
                (flight.clone(), false)
            } else {
                let (result, _) = watch::channel(None);
                let flight = Arc::new(QuotaFlight { result });
                in_flight.insert(provider_id.to_string(), flight.clone());
                (flight, true)
            }
        };

        if is_leader {
            let service = self.clone();
            let provider_id = provider_id.to_string();
            let leader_flight = flight.clone();
            tokio::spawn(async move {
                let result = service
                    .refresh_provider_once_at(&provider_id, attempted_at, interactive)
                    .await
                    .map_err(|error| error.to_string());
                leader_flight.result.send_replace(Some(result));

                let mut in_flight = service.in_flight.lock().await;
                if in_flight
                    .get(&provider_id)
                    .is_some_and(|current| Arc::ptr_eq(current, &leader_flight))
                {
                    in_flight.remove(&provider_id);
                }
            });
        }

        let mut result = flight.result.subscribe();
        loop {
            if let Some(result) = result.borrow().clone() {
                return result.map_err(AppError::Message);
            }
            result.changed().await.map_err(|_| {
                AppError::Message("quota refresh singleflight unavailable".to_string())
            })?;
        }
    }

    async fn refresh_provider_once_at(
        &self,
        provider_id: &str,
        attempted_at: i64,
        interactive: bool,
    ) -> Result<QuotaRefreshResult, AppError> {
        let provider = self
            .db
            .get_usage_provider(provider_id)?
            .ok_or_else(|| AppError::Message("usage provider not found".to_string()))?;
        validate_refresh_provider(&provider)?;

        let normalized = match self.collect_normalized_quota(&provider, interactive).await {
            Ok(normalized) => normalized,
            Err(error) => {
                self.db
                    .record_quota_failure(provider_id, attempted_at, &error.to_string())?;
                return Err(error);
            }
        };

        let snapshot = QuotaSnapshot {
            snapshot_id: Uuid::new_v4().to_string(),
            provider_id: provider.id,
            fetched_at: attempted_at,
            five_hour_utilization_percent: normalized.five_hour_utilization_percent,
            five_hour_resets_at: normalized.five_hour_resets_at,
            seven_day_utilization_percent: normalized.seven_day_utilization_percent,
            seven_day_resets_at: normalized.seven_day_resets_at,
            manual_resets_remaining: normalized.manual_resets_remaining,
            raw_payload: normalized.raw_payload,
            created_at: attempted_at,
        };
        let fetch_state = self.db.append_quota_success(&snapshot)?;
        Ok(QuotaRefreshResult {
            snapshot: quota_status_view(&snapshot),
            fetch_state,
        })
    }

    async fn collect_normalized_quota(
        &self,
        provider: &UsageProviderStored,
        interactive: bool,
    ) -> Result<NormalizedQuota, AppError> {
        let quota_source = provider
            .quota_source
            .as_deref()
            .ok_or_else(|| AppError::Message("quota source is not configured".to_string()))?;
        self.validate_local_quota_source_ownership(provider, quota_source)?;
        let collector = self.collectors.get(quota_source).ok_or_else(|| {
            AppError::Message(format!("unsupported quota source: {quota_source}"))
        })?;

        let quota = match collector.collect(provider, interactive).await {
            Ok(quota) if quota.success => quota,
            Ok(quota) => {
                let error = quota
                    .error
                    .or(quota.credential_message)
                    .unwrap_or_else(|| "quota collection failed".to_string());
                return Err(AppError::Message(error));
            }
            Err(error) => return Err(AppError::Message(error)),
        };

        let manual_resets_remaining = provider
            .quota_config
            .as_ref()
            .and_then(manual_resets_from_config);
        normalize_subscription_quota(&quota, manual_resets_remaining)
    }

    fn validate_local_quota_source_ownership(
        &self,
        provider: &UsageProviderStored,
        quota_source: &str,
    ) -> Result<(), AppError> {
        if !matches!(quota_source, "claude" | "codex" | CLAUDE_LOCAL_QUOTA_SOURCE)
            && quota_source != MANAGED_CODEX_QUOTA_SOURCE
        {
            return Ok(());
        }
        let conflict = self.db.list_usage_providers()?.into_iter().any(|other| {
            other.id != provider.id
                && other.enabled
                && other.billing_kind == BillingKind::Subscription
                && other.quota_source.as_deref() == Some(quota_source)
        });
        if conflict {
            return Err(AppError::Message(format!(
                "local quota source {quota_source} must belong to exactly one enabled provider"
            )));
        }
        Ok(())
    }

    /// Runs one deterministic scheduling pass. Normal refreshes use the
    /// configured idle interval; usage activity marks trigger one extra
    /// refresh after a short debounce (spaced at least 2 minutes apart).
    /// Failures retry after 30s, 1m, 5m, 10m, then 30m and always outrank
    /// activity; the fetch state only becomes stale when the 10-minute
    /// retry fails.
    pub async fn refresh_due_at(&self, now: i64) -> Result<QuotaSchedulerCycle, AppError> {
        let mut cycle = QuotaSchedulerCycle::default();
        for view in self.db.list_usage_providers()? {
            if !view.enabled
                || view.billing_kind != BillingKind::Subscription
                || view.quota_source.is_none()
            {
                continue;
            }
            let Some(interval) = quota_interval_seconds(view.quota_interval_seconds)? else {
                continue;
            };
            let state = self.db.get_quota_fetch_state(&view.id)?;
            let due = idle_due(state.as_ref(), interval, now)
                || activity_due(&view.id, state.as_ref(), now);
            if !due {
                continue;
            }
            cycle.attempted += 1;
            // 活动标记最多消费一次:无论因空闲还是活动到期,都清掉。
            clear_subscription_activity(&view.id);
            // 调度器路径是后台刷新,同意门控不应因此弹系统对话框。
            if let Err(error) = self.refresh_provider_at(&view.id, now, false).await {
                cycle.errors.push(format!("{}: {error}", view.id));
            }
        }
        Ok(cycle)
    }

    async fn run_scheduler_cycle_at(&self, now: i64, after_cycle: &QuotaCycleCallback) {
        let outcome = match self.refresh_due_at(now).await {
            Ok(cycle) => {
                let had_errors = !cycle.errors.is_empty();
                if had_errors {
                    log::warn!("quota scheduler refresh failed");
                }
                QuotaSchedulerOutcome::Completed {
                    attempted: cycle.attempted,
                    had_errors,
                }
            }
            Err(_) => {
                log::warn!("quota scheduler cycle failed");
                QuotaSchedulerOutcome::Failed
            }
        };
        after_cycle(outcome).await;
    }

    pub fn start_scheduler(
        self: Arc<Self>,
        after_cycle: QuotaCycleCallback,
    ) -> QuotaSchedulerHandle {
        let (cancel_tx, mut cancel_rx) = watch::channel(false);
        let task = tauri::async_runtime::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(SCHEDULER_TICK_SECONDS));
            interval.tick().await;
            loop {
                tokio::select! {
                    changed = cancel_rx.changed() => {
                        if changed.is_err() || *cancel_rx.borrow() {
                            break;
                        }
                    }
                    _ = interval.tick() => {
                        let Some(now) = scheduler_time_or_failure(
                            now_timestamp(),
                            &after_cycle,
                        ).await else {
                            continue;
                        };
                        self.run_scheduler_cycle_at(now, &after_cycle).await;
                    }
                }
            }
        });
        QuotaSchedulerHandle {
            cancel_tx,
            task: Some(task),
        }
    }
}

async fn scheduler_time_or_failure(
    now: Result<i64, AppError>,
    after_cycle: &QuotaCycleCallback,
) -> Option<i64> {
    match now {
        Ok(now) => Some(now),
        Err(_) => {
            log::warn!("quota scheduler clock failed");
            after_cycle(QuotaSchedulerOutcome::Failed).await;
            None
        }
    }
}

fn quota_status_view(snapshot: &QuotaSnapshot) -> QuotaStatusView {
    QuotaStatusView::from_snapshot(snapshot)
}

pub struct QuotaSchedulerHandle {
    cancel_tx: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
}

impl QuotaSchedulerHandle {
    pub async fn stop(mut self) {
        let _ = self.cancel_tx.send(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for QuotaSchedulerHandle {
    fn drop(&mut self) {
        let _ = self.cancel_tx.send(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub fn quota_interval_seconds(configured: Option<u64>) -> Result<Option<u64>, AppError> {
    match configured.unwrap_or(DEFAULT_QUOTA_INTERVAL_SECONDS) {
        0 => Ok(None),
        1..=59 => Err(AppError::Message(
            "quota interval must be 0 or at least 60 seconds".to_string(),
        )),
        interval => Ok(Some(interval)),
    }
}

fn quota_refresh_delay_seconds(state: &QuotaFetchState, interval: u64) -> u64 {
    if state.consecutive_failures == 0 {
        return interval;
    }
    let index = state
        .consecutive_failures
        .saturating_sub(1)
        .min((FAILURE_RETRY_DELAYS_SECONDS.len() - 1) as u32) as usize;
    FAILURE_RETRY_DELAYS_SECONDS[index]
}

pub fn normalize_subscription_quota(
    quota: &SubscriptionQuota,
    manual_resets_remaining: Option<i64>,
) -> Result<NormalizedQuota, AppError> {
    if !quota.success {
        return Err(AppError::Message(
            quota
                .error
                .clone()
                .or_else(|| quota.credential_message.clone())
                .unwrap_or_else(|| "quota collection failed".to_string()),
        ));
    }
    let tier = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| quota.tiers.iter().find(|tier| tier.name == *name))
    };
    let five_hour = tier(&[TIER_FIVE_HOUR]);
    let seven_day = tier(&[TIER_SEVEN_DAY, TIER_WEEKLY_LIMIT]);
    Ok(NormalizedQuota {
        five_hour_utilization_percent: five_hour
            .map(|tier| decimal_percent(tier.utilization))
            .transpose()?,
        five_hour_resets_at: five_hour.and_then(|tier| tier.resets_at.clone()),
        seven_day_utilization_percent: seven_day
            .map(|tier| decimal_percent(tier.utilization))
            .transpose()?,
        seven_day_resets_at: seven_day.and_then(|tier| tier.resets_at.clone()),
        manual_resets_remaining: quota
            .manual_reset_credits
            .as_ref()
            .map(|credits| credits.available_count)
            .or(manual_resets_remaining),
        raw_payload: serde_json::to_value(quota)
            .map_err(|error| AppError::Config(format!("serialize quota payload: {error}")))?,
    })
}

fn decimal_percent(value: f64) -> Result<String, AppError> {
    if !value.is_finite() || value < 0.0 {
        return Err(AppError::Message(
            "quota utilization must be a non-negative finite number".to_string(),
        ));
    }
    Decimal::from_str(&value.to_string())
        .map(|value| value.normalize().to_string())
        .map_err(|error| AppError::Message(format!("invalid quota utilization: {error}")))
}

fn manual_resets_from_config(config: &Value) -> Option<i64> {
    config
        .get("manual_resets_remaining")
        .or_else(|| config.get("manualResetsRemaining"))
        .and_then(Value::as_i64)
}

fn validate_refresh_provider(provider: &UsageProviderStored) -> Result<(), AppError> {
    if !provider.enabled {
        return Err(AppError::Message("quota provider is disabled".to_string()));
    }
    if provider.billing_kind != BillingKind::Subscription {
        return Err(AppError::Message(
            "quota provider must be subscription".to_string(),
        ));
    }
    Ok(())
}

fn now_timestamp() -> Result<i64, AppError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|error| AppError::Message(format!("system clock before unix epoch: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::codex_oauth_auth::CodexOAuthManager;
    use crate::database::Database;
    use crate::services::subscription::{
        CredentialStatus, ManualResetCredit, ManualResetCredits, QuotaTier, SubscriptionQuota,
        TIER_FIVE_HOUR, TIER_SEVEN_DAY,
    };
    use crate::usage::domain::{BillingKind, TokenSource, UsageProviderInput, UsageProviderStored};
    use crate::usage::system_providers::{CHATGPT_SUBSCRIPTION_ID, CLAUDE_SUBSCRIPTION_ID};
    use serde_json::json;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use tokio::sync::RwLock;

    fn successful_quota(tool: &str) -> SubscriptionQuota {
        SubscriptionQuota {
            tool: tool.to_string(),
            credential_status: CredentialStatus::Valid,
            credential_message: None,
            success: true,
            tiers: vec![
                QuotaTier {
                    name: TIER_FIVE_HOUR.to_string(),
                    utilization: 12.5,
                    resets_at: Some("2026-07-11T05:00:00Z".to_string()),
                    used_value_usd: None,
                    max_value_usd: None,
                },
                QuotaTier {
                    name: TIER_SEVEN_DAY.to_string(),
                    utilization: 42.0,
                    resets_at: Some("2026-07-18T00:00:00Z".to_string()),
                    used_value_usd: None,
                    max_value_usd: None,
                },
            ],
            plan_type: None,
            plan_renews_at: None,
            manual_reset_credits: None,
            extra_usage: None,
            error: None,
            queried_at: Some(100_000),
        }
    }

    fn provider(id: &str, billing_kind: BillingKind, enabled: bool) -> UsageProviderInput {
        UsageProviderInput {
            id: id.to_string(),
            name: id.to_string(),
            billing_kind,
            product_group_id: "claude".to_string(),
            token_sources: vec![TokenSource::SessionLog],
            session_source_bindings: None,
            quota_source: Some("fake".to_string()),
            quota_interval_seconds: Some(300),
            route_app_type: None,
            route_config: None,
            quota_config: Some(json!({"manualResetsRemaining": 2})),
            enabled,
        }
    }

    fn isolated_quota_test_db() -> Arc<Database> {
        let db = Arc::new(Database::memory().unwrap());
        db.set_usage_provider_enabled(CHATGPT_SUBSCRIPTION_ID, false)
            .unwrap();
        db.set_usage_provider_enabled(CLAUDE_SUBSCRIPTION_ID, false)
            .unwrap();
        db
    }

    struct FakeCollector {
        calls: AtomicUsize,
        responses: Mutex<VecDeque<Result<SubscriptionQuota, String>>>,
    }

    impl FakeCollector {
        fn new(responses: Vec<Result<SubscriptionQuota, String>>) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                responses: Mutex::new(responses.into()),
            }
        }
    }

    impl QuotaCollector for FakeCollector {
        fn source(&self) -> &'static str {
            "fake"
        }

        fn collect<'a>(
            &'a self,
            _provider: &'a UsageProviderStored,
            _interactive: bool,
        ) -> BoxFuture<'a, Result<SubscriptionQuota, String>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let result = self
                .responses
                .lock()
                .expect("fake responses")
                .pop_front()
                .expect("fake response");
            Box::pin(async move { result })
        }
    }

    #[derive(Default)]
    struct YieldingCollector {
        calls: AtomicUsize,
    }

    impl QuotaCollector for YieldingCollector {
        fn source(&self) -> &'static str {
            "fake"
        }

        fn collect<'a>(
            &'a self,
            _provider: &'a UsageProviderStored,
            _interactive: bool,
        ) -> BoxFuture<'a, Result<SubscriptionQuota, String>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                tokio::task::yield_now().await;
                Ok(successful_quota("claude"))
            })
        }
    }

    #[test]
    fn normalizes_documented_windows_and_optional_manual_resets() {
        let mut quota = successful_quota("codex");
        quota.plan_type = Some("pro".to_string());
        quota.plan_renews_at = Some(1_789_876_543);

        let normalized = normalize_subscription_quota(&quota, Some(3)).unwrap();
        assert_eq!(
            normalized.five_hour_utilization_percent.as_deref(),
            Some("12.5")
        );
        assert_eq!(
            normalized.seven_day_utilization_percent.as_deref(),
            Some("42")
        );
        assert_eq!(normalized.manual_resets_remaining, Some(3));
        assert_eq!(normalized.raw_payload["planType"], json!("pro"));
        assert_eq!(
            normalized.raw_payload["planRenewsAt"],
            json!(1_789_876_543_i64)
        );
    }

    #[test]
    fn codex_reported_manual_reset_count_overrides_legacy_config_fallback() {
        let mut quota = successful_quota("codex");
        quota.manual_reset_credits = Some(ManualResetCredits {
            available_count: 3,
            credits: vec![ManualResetCredit {
                id: "reset-1".to_string(),
                reset_type: Some("codexRateLimits".to_string()),
                status: Some("available".to_string()),
                granted_at: None,
                expires_at: "2026-07-27T00:00:00+00:00".to_string(),
                title: Some("Full reset".to_string()),
                description: None,
            }],
        });

        let normalized = normalize_subscription_quota(&quota, Some(1)).unwrap();

        assert_eq!(normalized.manual_resets_remaining, Some(3));
        assert_eq!(
            normalized.raw_payload["manualResetCredits"]["credits"][0]["expiresAt"],
            json!("2026-07-27T00:00:00+00:00")
        );
    }

    #[test]
    fn interval_contract_defaults_to_idle_fifteen_minutes_and_zero_disables() {
        assert_eq!(quota_interval_seconds(None).unwrap(), Some(900));
        assert_eq!(quota_interval_seconds(Some(0)).unwrap(), None);
        assert!(quota_interval_seconds(Some(59)).is_err());
        assert_eq!(quota_interval_seconds(Some(60)).unwrap(), Some(60));
    }

    #[tokio::test]
    async fn production_registers_local_subscription_collectors() {
        let data_dir = std::env::temp_dir().join(format!(
            "llm-usage-bar-empty-codex-oauth-{}",
            Uuid::new_v4()
        ));
        let manager = Arc::new(RwLock::new(CodexOAuthManager::new(data_dir)));
        assert_eq!(Arc::strong_count(&manager), 1);

        let db = Arc::new(Database::memory().unwrap());
        db.reconcile_system_providers().unwrap();
        let service = QuotaService::production(db.clone(), manager.clone());
        let mut sources = service
            .collectors
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        sources.sort_unstable();

        assert_eq!(
            sources,
            [
                "claude",
                "claude_local",
                "codex",
                "codex_oauth",
                "coding_plan"
            ]
        );
        assert!(!service.collectors.contains_key("claude_oauth"));
        assert_eq!(Arc::strong_count(&manager), 2);

        // 生产链的 collect 会读本机真实 ~/.codex/auth.json 并可能发网络请求,
        // 不在单测里调用;这里直接验证链的第一层(自管 OAuth)在空账号库下
        // 返回 NotFound。链结构本身由 CodexChainCollector 单测覆盖。
        let provider = db
            .get_usage_provider(crate::usage::system_providers::CHATGPT_SUBSCRIPTION_ID)
            .unwrap()
            .unwrap();
        let managed = ManagedCodexOAuthQuotaCollector { manager };
        let quota = managed.collect(&provider, true).await.unwrap();
        assert!(!quota.success);
        assert!(matches!(
            quota.credential_status,
            CredentialStatus::NotFound
        ));
    }

    // ── Claude 回退链 ────────────────────────────────────

    fn stored_provider() -> UsageProviderStored {
        UsageProviderStored {
            id: "sub".to_string(),
            name: "sub".to_string(),
            billing_kind: BillingKind::Subscription,
            product_group_id: "claude".to_string(),
            token_sources: vec![TokenSource::SessionLog],
            quota_source: Some(CLAUDE_LOCAL_QUOTA_SOURCE.to_string()),
            quota_interval_seconds: Some(300),
            route_app_type: None,
            route_config: None,
            quota_config: None,
            enabled: true,
            needs_review: false,
            legacy_app_type: None,
            legacy_provider_id: None,
            created_at: 0,
            updated_at: 0,
            system_preset_key: None,
            daily_budget_usd: None,
        }
    }

    fn chain_stage(
        response: Result<SubscriptionQuota, String>,
        calls: Arc<AtomicUsize>,
    ) -> ClaudeStage {
        Arc::new(move |_interactive| {
            let response = response.clone();
            let calls = calls.clone();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                response
            })
        })
    }

    #[tokio::test]
    async fn claude_chain_returns_first_successful_stage() {
        for winner in 0..3 {
            let calls = [
                Arc::new(AtomicUsize::new(0)),
                Arc::new(AtomicUsize::new(0)),
                Arc::new(AtomicUsize::new(0)),
            ];
            let stage = |index: usize| -> Result<SubscriptionQuota, String> {
                if index == winner {
                    Ok(successful_quota("claude"))
                } else {
                    Err(format!("stage-{index}-failed"))
                }
            };
            let collector = ClaudeChainCollector::with_stages(
                chain_stage(stage(0), calls[0].clone()),
                chain_stage(stage(1), calls[1].clone()),
                chain_stage(stage(2), calls[2].clone()),
            );
            let quota = collector
                .collect(&stored_provider(), false)
                .await
                .expect("chain succeeds");
            assert!(quota.success, "winner stage {winner}");
            for (index, calls) in calls.iter().enumerate() {
                assert_eq!(calls.load(Ordering::SeqCst), usize::from(index <= winner));
            }
        }
    }

    #[tokio::test]
    async fn claude_chain_summary_keeps_stage_codes_in_order() {
        let collector = ClaudeChainCollector::with_stages(
            chain_stage(
                Ok(SubscriptionQuota::error(
                    "claude_oauth",
                    CredentialStatus::ConsentRequired,
                    crate::usage::claude_oauth::CONSENT_ERROR_CODE.to_string(),
                )),
                Arc::new(AtomicUsize::new(0)),
            ),
            chain_stage(Err("probe failed".to_string()), Arc::new(AtomicUsize::new(0))),
            chain_stage(Err("local stage failed".to_string()), Arc::new(AtomicUsize::new(0))),
        );
        let error = collector
            .collect(&stored_provider(), false)
            .await
            .expect_err("all stages fail");
        assert_eq!(
            error,
            "claude quota chain failed: \
             oauth(claude_oauth_consent_required) → probe(probe failed) → local(local stage failed)"
        );
    }

    // ── Codex 回退链 ────────────────────────────────────

    #[tokio::test]
    async fn codex_chain_returns_first_successful_stage() {
        for winner in 0..3 {
            let calls = [
                Arc::new(AtomicUsize::new(0)),
                Arc::new(AtomicUsize::new(0)),
                Arc::new(AtomicUsize::new(0)),
            ];
            let stage = |index: usize| -> Result<SubscriptionQuota, String> {
                if index == winner {
                    Ok(successful_quota("codex"))
                } else {
                    Err(format!("stage-{index}-failed"))
                }
            };
            let collector = CodexChainCollector::with_stages(
                chain_stage(stage(0), calls[0].clone()),
                chain_stage(stage(1), calls[1].clone()),
                chain_stage(stage(2), calls[2].clone()),
            );
            let quota = collector
                .collect(&stored_provider(), false)
                .await
                .expect("chain succeeds");
            assert!(quota.success, "winner stage {winner}");
            for (index, calls) in calls.iter().enumerate() {
                assert_eq!(calls.load(Ordering::SeqCst), usize::from(index <= winner));
            }
        }
    }

    #[tokio::test]
    async fn codex_chain_summary_keeps_stage_codes_in_order() {
        let collector = CodexChainCollector::with_stages(
            chain_stage(
                Ok(SubscriptionQuota::not_found("codex_oauth")),
                Arc::new(AtomicUsize::new(0)),
            ),
            chain_stage(Err("cli failed".to_string()), Arc::new(AtomicUsize::new(0))),
            chain_stage(Err("probe failed".to_string()), Arc::new(AtomicUsize::new(0))),
        );
        let error = collector
            .collect(&stored_provider(), false)
            .await
            .expect_err("all stages fail");
        assert_eq!(
            error,
            "codex quota chain failed: managed(quota collection failed) → cli(cli failed) → probe(probe failed)"
        );
    }

    // ── 手动刷新冷却 ────────────────────────────────────

    #[tokio::test]
    async fn manual_refresh_cooldown_reuses_result_within_sixty_seconds() {
        let db = isolated_quota_test_db();
        db.save_usage_provider(&provider("sub", BillingKind::Subscription, true))
            .unwrap();
        let collector = Arc::new(FakeCollector::new(vec![
            Ok(successful_quota("claude")),
            Ok(successful_quota("claude")),
        ]));
        let service = QuotaService::with_collectors(db, vec![collector.clone()]);

        let first = service.refresh_provider_manual_at("sub", 100).await.unwrap();
        assert_eq!(collector.calls.load(Ordering::SeqCst), 1);

        // 60 秒内:复用上次结果,不采集。
        let reused = service.refresh_provider_manual_at("sub", 150).await.unwrap();
        assert_eq!(reused, first);
        assert_eq!(collector.calls.load(Ordering::SeqCst), 1);

        // 60 秒后:重新采集。
        let second = service.refresh_provider_manual_at("sub", 160).await.unwrap();
        assert!(second.snapshot.snapshot_id != first.snapshot.snapshot_id);
        assert_eq!(collector.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn manual_refresh_cooldown_caches_failures_too() {
        let db = isolated_quota_test_db();
        db.save_usage_provider(&provider("sub", BillingKind::Subscription, true))
            .unwrap();
        let collector = Arc::new(FakeCollector::new(vec![Err("boom".to_string())]));
        let service = QuotaService::with_collectors(db, vec![collector.clone()]);

        let first = service.refresh_provider_manual_at("sub", 100).await;
        assert!(first.is_err());
        // 冷却期内的重复点击复用失败的缓存,不再打采集器。
        let second = service.refresh_provider_manual_at("sub", 130).await;
        assert!(second.is_err());
        assert_eq!(collector.calls.load(Ordering::SeqCst), 1);
    }

    // ── 活动自适应刷新 ──────────────────────────────────

    #[tokio::test]
    async fn activity_mark_triggers_one_refresh_after_debounce_and_spacing() {
        let db = isolated_quota_test_db();
        db.save_usage_provider(&provider("activity-sub", BillingKind::Subscription, true))
            .unwrap();
        let collector = Arc::new(FakeCollector::new(vec![
            Ok(successful_quota("claude")),
            Ok(successful_quota("claude")),
            Ok(successful_quota("claude")),
        ]));
        let service = QuotaService::with_collectors(db, vec![collector.clone()]);
        clear_subscription_activity("activity-sub");

        // 空闲首次采集(t=100)。
        assert_eq!(service.refresh_due_at(100).await.unwrap().attempted, 1);
        assert_eq!(collector.calls.load(Ordering::SeqCst), 1);

        // 活动标记发生在 t=250;去抖窗口(30s)内不到期。
        mark_subscription_activity_at("activity-sub", 250);
        assert_eq!(service.refresh_due_at(260).await.unwrap().attempted, 0);

        // t=285:过完去抖且距上次尝试超过 120s → 补刷一次,标记被消费。
        let cycle = service.refresh_due_at(285).await.unwrap();
        assert_eq!(cycle.attempted, 1);
        assert_eq!(collector.calls.load(Ordering::SeqCst), 2);

        // 无新活动:空闲基线(夹具 300s,上次 285)未到,不刷。
        assert_eq!(service.refresh_due_at(400).await.unwrap().attempted, 0);

        // 新活动在 120s 活跃间隔内(距上次尝试 45s) → 不刷。
        mark_subscription_activity_at("activity-sub", 310);
        assert_eq!(service.refresh_due_at(330).await.unwrap().attempted, 0);

        // 距上次尝试超过 120s 且过完去抖 → 补刷。
        mark_subscription_activity_at("activity-sub", 420);
        assert_eq!(service.refresh_due_at(450).await.unwrap().attempted, 1);
        assert_eq!(collector.calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn activity_does_not_bypass_failure_backoff() {
        // 与上一个活动测试用不同的 provider id:静态活动表是进程级共享的,
        // 测试并行时同 id 的标记会互相干扰。
        let db = isolated_quota_test_db();
        db.save_usage_provider(&provider("activity-sub-failure", BillingKind::Subscription, true))
            .unwrap();
        let collector = Arc::new(FakeCollector::new(vec![
            Err("boom".to_string()),
            Ok(successful_quota("claude")),
        ]));
        let service = QuotaService::with_collectors(db, vec![collector.clone()]);
        clear_subscription_activity("activity-sub-failure");

        // 首次失败(t=100) → 阶梯 30s。
        let cycle = service.refresh_due_at(100).await.unwrap();
        assert_eq!(cycle.attempted, 1);
        assert_eq!(cycle.errors.len(), 1);

        // 活动发生在 t=105,但失败阶梯 30s 未让路(t=120 < 130) → 不刷。
        mark_subscription_activity_at("activity-sub-failure", 105);
        assert_eq!(service.refresh_due_at(120).await.unwrap().attempted, 0);

        // t=140:阶梯让路 + 去抖完成 → 补刷成功(消费第二个响应)。
        let cycle = service.refresh_due_at(140).await.unwrap();
        assert_eq!(cycle.attempted, 1);
        assert_eq!(cycle.errors.len(), 0);
        assert_eq!(collector.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn claude_codex_and_coding_plan_share_window_normalization() {
        for tool in ["claude", "codex", "coding_plan"] {
            let mut quota = successful_quota(tool);
            if tool == "coding_plan" {
                quota.tiers[1].name = TIER_WEEKLY_LIMIT.to_string();
            }
            let normalized = normalize_subscription_quota(&quota, None).unwrap();
            assert_eq!(
                normalized.five_hour_utilization_percent.as_deref(),
                Some("12.5")
            );
            assert_eq!(
                normalized.seven_day_utilization_percent.as_deref(),
                Some("42")
            );
            assert_eq!(normalized.manual_resets_remaining, None);
        }
    }

    #[test]
    fn scheduler_can_start_without_an_ambient_tokio_runtime() {
        let service = Arc::new(QuotaService::new(isolated_quota_test_db()));

        let scheduler = service.start_scheduler(Arc::new(|_| Box::pin(async {})));

        drop(scheduler);
    }

    #[tokio::test]
    async fn scheduler_cycle_callback_reports_zero_attempt_success_once() {
        let service = QuotaService::new(isolated_quota_test_db());
        let outcomes = Arc::new(Mutex::new(Vec::new()));
        let captured = outcomes.clone();
        let after_cycle: QuotaCycleCallback = Arc::new(move |outcome| {
            let captured = captured.clone();
            Box::pin(async move {
                captured.lock().expect("captured outcomes").push(outcome);
            })
        });

        service.run_scheduler_cycle_at(100, &after_cycle).await;

        assert_eq!(
            *outcomes.lock().expect("captured outcomes"),
            vec![QuotaSchedulerOutcome::Completed {
                attempted: 0,
                had_errors: false,
            }]
        );
    }

    #[tokio::test]
    async fn scheduler_cycle_waits_for_async_callback_completion() {
        let service = Arc::new(QuotaService::new(isolated_quota_test_db()));
        let callback_started = Arc::new(tokio::sync::Notify::new());
        let release_callback = Arc::new(tokio::sync::Notify::new());
        let callback_finished = Arc::new(AtomicUsize::new(0));
        let after_cycle: QuotaCycleCallback = {
            let callback_started = callback_started.clone();
            let release_callback = release_callback.clone();
            let callback_finished = callback_finished.clone();
            Arc::new(move |_| {
                let callback_started = callback_started.clone();
                let release_callback = release_callback.clone();
                let callback_finished = callback_finished.clone();
                Box::pin(async move {
                    callback_started.notify_one();
                    release_callback.notified().await;
                    callback_finished.fetch_add(1, Ordering::SeqCst);
                })
            })
        };

        let cycle = tokio::spawn({
            let service = service.clone();
            async move {
                service.run_scheduler_cycle_at(100, &after_cycle).await;
            }
        });
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            callback_started.notified(),
        )
        .await
        .expect("callback should start");
        assert_eq!(callback_finished.load(Ordering::SeqCst), 0);

        release_callback.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(1), cycle)
            .await
            .expect("scheduler cycle should finish after its callback")
            .expect("scheduler cycle task should not panic");
        assert_eq!(callback_finished.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn scheduler_cycle_callback_reports_provider_errors_without_details() {
        const SECRET_SENTINEL: &str = "sk-secret-scheduler-provider-error";
        let db = isolated_quota_test_db();
        db.save_usage_provider(&provider("sub", BillingKind::Subscription, true))
            .unwrap();
        let collector = Arc::new(FakeCollector::new(vec![Err(SECRET_SENTINEL.to_string())]));
        let service = QuotaService::with_collectors(db, vec![collector]);
        let outcomes = Arc::new(Mutex::new(Vec::new()));
        let captured = outcomes.clone();
        let after_cycle: QuotaCycleCallback = Arc::new(move |outcome| {
            let captured = captured.clone();
            Box::pin(async move {
                captured.lock().expect("captured outcomes").push(outcome);
            })
        });

        service.run_scheduler_cycle_at(100, &after_cycle).await;

        let outcomes = outcomes.lock().expect("captured outcomes");
        assert_eq!(
            outcomes.as_slice(),
            [QuotaSchedulerOutcome::Completed {
                attempted: 1,
                had_errors: true,
            }]
        );
        assert!(!format!("{outcomes:?}").contains(SECRET_SENTINEL));
    }

    #[tokio::test]
    async fn scheduler_cycle_callback_reports_top_level_failure_once() {
        let db = isolated_quota_test_db();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "ALTER TABLE usage_providers RENAME TO unavailable_usage_providers",
                [],
            )
            .unwrap();
        let service = QuotaService::with_collectors(db, vec![]);
        let outcomes = Arc::new(Mutex::new(Vec::new()));
        let captured = outcomes.clone();
        let after_cycle: QuotaCycleCallback = Arc::new(move |outcome| {
            let captured = captured.clone();
            Box::pin(async move {
                captured.lock().expect("captured outcomes").push(outcome);
            })
        });

        service.run_scheduler_cycle_at(100, &after_cycle).await;

        let outcomes = outcomes.lock().expect("captured outcomes");
        assert_eq!(outcomes.as_slice(), [QuotaSchedulerOutcome::Failed]);
        assert!(!format!("{outcomes:?}").contains("usage_providers"));
    }

    #[tokio::test]
    async fn scheduler_clock_failure_reports_sanitized_failure_once() {
        let outcomes = Arc::new(Mutex::new(Vec::new()));
        let captured = outcomes.clone();
        let after_cycle: QuotaCycleCallback = Arc::new(move |outcome| {
            let captured = captured.clone();
            Box::pin(async move {
                captured.lock().expect("captured outcomes").push(outcome);
            })
        });

        let now = scheduler_time_or_failure(
            Err(AppError::Message(
                "system clock before unix epoch: secret-clock-details".to_string(),
            )),
            &after_cycle,
        )
        .await;

        assert_eq!(now, None);
        let outcomes = outcomes.lock().expect("captured outcomes");
        assert_eq!(outcomes.as_slice(), [QuotaSchedulerOutcome::Failed]);
        assert!(!format!("{outcomes:?}").contains("secret-clock-details"));
    }

    #[tokio::test]
    async fn failures_follow_backoff_and_only_expire_after_ten_minute_retry() {
        let db = isolated_quota_test_db();
        db.save_usage_provider(&provider("sub", BillingKind::Subscription, true))
            .unwrap();
        let collector = Arc::new(FakeCollector::new(vec![
            Ok(successful_quota("claude")),
            Err("timeout".to_string()),
            Err("timeout".to_string()),
            Err("timeout".to_string()),
            Err("timeout".to_string()),
            Err("timeout".to_string()),
            Err("timeout".to_string()),
        ]));
        let service = QuotaService::with_collectors(db.clone(), vec![collector.clone()]);

        let first = service.refresh_due_at(100).await.unwrap();
        assert_eq!(first.attempted, 1);
        assert!(first.errors.is_empty());
        let first_snapshot = db.latest_quota_snapshot("sub").unwrap().unwrap();
        assert_eq!(first_snapshot.fetched_at, 100);
        assert_eq!(first_snapshot.manual_resets_remaining, Some(2));

        assert_eq!(service.refresh_due_at(399).await.unwrap().attempted, 0);
        let failed = service.refresh_due_at(400).await.unwrap();
        assert_eq!(failed.attempted, 1);
        assert_eq!(failed.errors.len(), 1);
        assert_eq!(collector.calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            db.latest_quota_snapshot("sub").unwrap().unwrap(),
            first_snapshot
        );
        let state = db.get_quota_fetch_state("sub").unwrap().unwrap();
        assert_eq!(state.last_success_at, Some(100));
        assert_eq!(state.last_attempt_at, Some(400));
        assert_eq!(state.consecutive_failures, 1);
        assert!(!state.stale);

        for (not_due_at, due_at, expected_failures, expected_stale) in [
            (429, 430, 2, false),
            (489, 490, 3, false),
            (789, 790, 4, false),
            (1_389, 1_390, 5, true),
            (3_189, 3_190, 6, true),
        ] {
            assert_eq!(
                service.refresh_due_at(not_due_at).await.unwrap().attempted,
                0
            );
            assert_eq!(service.refresh_due_at(due_at).await.unwrap().attempted, 1);
            let state = db.get_quota_fetch_state("sub").unwrap().unwrap();
            assert_eq!(state.consecutive_failures, expected_failures);
            assert_eq!(state.stale, expected_stale);
        }
        assert_eq!(collector.calls.load(Ordering::SeqCst), 7);
    }

    #[tokio::test]
    async fn custom_interval_only_changes_healthy_refresh_cadence() {
        let db = isolated_quota_test_db();
        let mut input = provider("sub", BillingKind::Subscription, true);
        input.quota_interval_seconds = Some(480);
        db.save_usage_provider(&input).unwrap();
        let collector = Arc::new(FakeCollector::new(vec![
            Ok(successful_quota("claude")),
            Err("timeout".to_string()),
            Err("timeout".to_string()),
        ]));
        let service = QuotaService::with_collectors(db, vec![collector]);

        assert_eq!(service.refresh_due_at(100).await.unwrap().attempted, 1);
        assert_eq!(service.refresh_due_at(579).await.unwrap().attempted, 0);
        assert_eq!(service.refresh_due_at(580).await.unwrap().attempted, 1);
        assert_eq!(service.refresh_due_at(609).await.unwrap().attempted, 0);
        assert_eq!(service.refresh_due_at(610).await.unwrap().attempted, 1);
    }

    #[tokio::test]
    async fn manual_refresh_collects_once_and_rejects_metered_or_disabled() {
        let db = isolated_quota_test_db();
        for input in [
            provider("sub", BillingKind::Subscription, true),
            provider("metered", BillingKind::Metered, true),
            provider("disabled", BillingKind::Subscription, false),
        ] {
            db.save_usage_provider(&input).unwrap();
        }
        let collector = Arc::new(FakeCollector::new(vec![Ok(successful_quota("claude"))]));
        let service = QuotaService::with_collectors(db, vec![collector.clone()]);

        let refreshed = service.refresh_provider("sub").await.unwrap();
        let public_json = serde_json::to_value(&refreshed).unwrap();
        assert!(public_json["snapshot"].get("rawPayload").is_none());
        assert_eq!(collector.calls.load(Ordering::SeqCst), 1);
        assert!(service.refresh_provider("metered").await.is_err());
        assert!(service.refresh_provider("disabled").await.is_err());
        assert_eq!(collector.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn zero_interval_disables_scheduled_collection() {
        let db = isolated_quota_test_db();
        let mut input = provider("sub", BillingKind::Subscription, true);
        input.quota_interval_seconds = Some(0);
        db.save_usage_provider(&input).unwrap();
        let collector = Arc::new(FakeCollector::new(vec![]));
        let service = QuotaService::with_collectors(db, vec![collector.clone()]);

        assert_eq!(service.refresh_due_at(1_000).await.unwrap().attempted, 0);
        assert_eq!(collector.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn configuration_failures_enter_the_retry_schedule() {
        let db = isolated_quota_test_db();
        let mut input = provider("sub", BillingKind::Subscription, true);
        input.quota_source = Some("unsupported".to_string());
        db.save_usage_provider(&input).unwrap();
        let service = QuotaService::with_collectors(db.clone(), vec![]);

        let first = service.refresh_due_at(100).await.unwrap();
        assert_eq!(first.attempted, 1);
        assert_eq!(first.errors.len(), 1);
        assert_eq!(
            db.get_quota_fetch_state("sub")
                .unwrap()
                .unwrap()
                .last_attempt_at,
            Some(100)
        );
        assert_eq!(service.refresh_due_at(101).await.unwrap().attempted, 0);
        assert_eq!(service.refresh_due_at(129).await.unwrap().attempted, 0);
        assert_eq!(service.refresh_due_at(130).await.unwrap().attempted, 1);
    }

    #[tokio::test]
    async fn normalization_failures_enter_the_retry_schedule() {
        let db = isolated_quota_test_db();
        db.save_usage_provider(&provider("sub", BillingKind::Subscription, true))
            .unwrap();
        let mut invalid = successful_quota("claude");
        invalid.tiers[0].utilization = f64::NAN;
        let collector = Arc::new(FakeCollector::new(vec![Ok(invalid.clone()), Ok(invalid)]));
        let service = QuotaService::with_collectors(db.clone(), vec![collector.clone()]);

        let first = service.refresh_due_at(100).await.unwrap();
        assert_eq!(first.attempted, 1);
        assert_eq!(first.errors.len(), 1);
        let state = db.get_quota_fetch_state("sub").unwrap().unwrap();
        assert_eq!(state.last_attempt_at, Some(100));
        assert!(state
            .last_error
            .as_deref()
            .unwrap_or_default()
            .contains("non-negative finite"));
        assert_eq!(service.refresh_due_at(129).await.unwrap().attempted, 0);
        assert_eq!(service.refresh_due_at(130).await.unwrap().attempted, 1);
        assert_eq!(collector.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn machine_local_quota_source_cannot_be_attributed_to_two_providers() {
        for source in ["claude", "claude_local", "codex", "codex_oauth"] {
            let db = isolated_quota_test_db();
            for id in ["first", "second"] {
                let mut input = provider(id, BillingKind::Subscription, true);
                input.quota_source = Some(source.to_string());
                db.save_usage_provider(&input).unwrap();
            }
            let service = QuotaService::with_collectors(db.clone(), vec![]);

            let cycle = service.refresh_due_at(500).await.unwrap();
            assert_eq!(cycle.attempted, 2, "{source}");
            assert_eq!(cycle.errors.len(), 2, "{source}");
            for id in ["first", "second"] {
                let state = db.get_quota_fetch_state(id).unwrap().unwrap();
                assert_eq!(state.last_attempt_at, Some(500), "{source}/{id}");
                assert!(
                    state
                        .last_error
                        .as_deref()
                        .unwrap_or_default()
                        .contains("exactly one enabled provider"),
                    "{source}/{id}: {:?}",
                    state.last_error
                );
            }
        }
    }

    #[tokio::test]
    async fn concurrent_manual_refreshes_share_one_provider_snapshot() {
        let db = isolated_quota_test_db();
        db.save_usage_provider(&provider("sub", BillingKind::Subscription, true))
            .unwrap();
        let collector = Arc::new(YieldingCollector::default());
        let service = QuotaService::with_collectors(db.clone(), vec![collector.clone()]);

        let (first, second) = tokio::join!(
            service.refresh_provider("sub"),
            service.refresh_provider("sub")
        );
        let first = first.unwrap();
        let second = second.unwrap();

        assert_eq!(collector.calls.load(Ordering::SeqCst), 1);
        assert_eq!(first, second);
        assert_eq!(
            db.conn
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM quota_snapshots WHERE provider_id = 'sub'",
                    [],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn manual_and_scheduler_refresh_share_the_same_provider_flight() {
        let db = isolated_quota_test_db();
        db.save_usage_provider(&provider("sub", BillingKind::Subscription, true))
            .unwrap();
        let collector = Arc::new(YieldingCollector::default());
        let service = QuotaService::with_collectors(db.clone(), vec![collector.clone()]);

        let (manual, scheduler) =
            tokio::join!(service.refresh_provider("sub"), service.refresh_due_at(100));

        manual.unwrap();
        assert!(scheduler.unwrap().errors.is_empty());
        assert_eq!(collector.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            db.conn
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM quota_snapshots WHERE provider_id = 'sub'",
                    [],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            1
        );
    }
}
