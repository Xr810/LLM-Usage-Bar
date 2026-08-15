//! 官方订阅额度查询服务
//!
//! Claude Pro/Max 读取 Claude Desktop 本地额度历史，并以 Claude Code
//! status-line 作为可验证的补充来源；其他工具沿用各自现有实现。
//!
//! 本模块只保留共享类型、tier 常量与分发入口；Codex / Gemini 的凭据读取和
//! 接口查询分别在 `codex` / `gemini` 子模块里。

pub mod balance;
pub mod claude;
pub(crate) mod codex;
pub mod coding_plan;
mod gemini;
pub mod shared;

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

// `src/usage/quota.rs` 经 `crate::providers::` 这条路径引用它，
// 转出以保持拆分前的路径不变。
//
// 同为 `pub(crate)` 的 `query_codex_quota` 不在这里转出：它模块外没有调用方，
// 转出只会换来一条 unused_imports，而 `-D warnings` 下那就是编译失败。
// 它在 `codex/subscription.rs` 里的可见性照原样保留，将来有人要用再转不迟。
pub(crate) use codex::subscription::query_managed_codex_oauth_quota;

pub use claude::cli_auth::{ClaudeCliAuthService, ClaudeCliAuthStatus};
pub use shared::connection::{SystemProviderConnectionService, SystemProviderConnectionTestResult};

// ── 数据类型 ──────────────────────────────────────────────

/// 凭据状态
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialStatus {
    Valid,
    Expired,
    NotFound,
    ParseError,
    /// 需要用户授权（Claude 钥匙串 OAuth 同意门控未开启）。
    ConsentRequired,
}

/// 单个限速窗口（如 5小时会话、7天周期）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaTier {
    /// 窗口标识：five_hour, seven_day, seven_day_opus, seven_day_sonnet 等
    pub name: String,
    /// 使用百分比 0–100
    pub utilization: f64,
    /// ISO 8601 重置时间
    pub resets_at: Option<String>,
    /// ZenMux: 已用额度（USD）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_value_usd: Option<f64>,
    /// ZenMux: 窗口上限（USD）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_value_usd: Option<f64>,
}

/// 超额使用信息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraUsage {
    pub is_enabled: bool,
    pub monthly_limit: Option<f64>,
    pub used_credits: Option<f64>,
    pub utilization: Option<f64>,
    pub currency: Option<String>,
}

/// 一张可手动消耗的订阅额度重置券。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualResetCredit {
    pub id: String,
    pub reset_type: Option<String>,
    pub status: Option<String>,
    pub granted_at: Option<String>,
    pub expires_at: String,
    pub title: Option<String>,
    pub description: Option<String>,
}

/// Codex 账号当前可用的手动额度重置券。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualResetCredits {
    pub available_count: i64,
    pub credits: Vec<ManualResetCredit>,
}

/// 订阅额度查询结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionQuota {
    pub tool: String,
    pub credential_status: CredentialStatus,
    pub credential_message: Option<String>,
    pub success: bool,
    pub tiers: Vec<QuotaTier>,
    /// ChatGPT plan as reported by the OAuth token: "pro", "plus", "team", ...
    /// None for tools that do not report one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    /// Unix seconds at which the current subscription period ends.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_renews_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual_reset_credits: Option<ManualResetCredits>,
    pub extra_usage: Option<ExtraUsage>,
    pub error: Option<String>,
    pub queried_at: Option<i64>,
}

impl SubscriptionQuota {
    /// 一次采集结果的骨架：只有 `tool` 必填，其余字段取中性值（没数据、没错误）。
    /// 构造时用结构体更新语法只写自己关心的部分：
    ///
    /// ```ignore
    /// SubscriptionQuota {
    ///     success: true,
    ///     tiers,
    ///     ..SubscriptionQuota::skeleton("codex")
    /// }
    /// ```
    ///
    /// 这样给某个 provider 加一个它独有的字段时，只动这里和真正用得上的那一处，
    /// 不必回头去补另外三十几个与它无关的构造点——那正是 Codex 的点数余额一直
    /// 没落地的原因（「加了要动 30+ 处构造字面量」）。
    ///
    /// 刻意不实现 `Default`：`tool` 是语义上必填的，给它一个空字符串默认值等于
    /// 埋一个可以被忘记的坑。
    pub(crate) fn skeleton(tool: &str) -> Self {
        Self {
            tool: tool.to_string(),
            credential_status: CredentialStatus::Valid,
            credential_message: None,
            success: false,
            tiers: Vec::new(),
            plan_type: None,
            plan_renews_at: None,
            manual_reset_credits: None,
            extra_usage: None,
            error: None,
            queried_at: None,
        }
    }

    pub(crate) fn not_found(tool: &str) -> Self {
        Self {
            credential_status: CredentialStatus::NotFound,
            ..Self::skeleton(tool)
        }
    }

    pub(crate) fn error(tool: &str, status: CredentialStatus, message: String) -> Self {
        Self {
            credential_status: status,
            credential_message: Some(message.clone()),
            error: Some(message),
            queried_at: Some(now_millis()),
            ..Self::skeleton(tool)
        }
    }
}

/// 已知的 Claude 用量窗口名称。`QuotaTier::name` 会是其中之一。
pub const TIER_FIVE_HOUR: &str = "five_hour";
pub const TIER_SEVEN_DAY: &str = "seven_day";
pub const TIER_SEVEN_DAY_OPUS: &str = "seven_day_opus";
pub const TIER_SEVEN_DAY_SONNET: &str = "seven_day_sonnet";

/// Coding Plan（Kimi / MiniMax）的周窗口 tier 名。与 `coding_plan::query_*`
/// 写入、tray 渲染、commands::provider 扁平化三处共用同一标识。
pub const TIER_WEEKLY_LIMIT: &str = "weekly_limit";

/// 月窗口 tier 名。火山方舟 Agent Plan / Coding Plan 有 5h / 周 / 月 三个展示
/// 窗口（Kimi / MiniMax 只有 5h + 周），月窗口共用此标识；前端 `TIER_I18N_KEYS`
/// 映射到 `subscription.monthly`。
pub const TIER_MONTHLY: &str = "monthly";

/// Codex 免费方案的 30 天（月）滚动窗口 tier 名。付费方案的次要窗口是 7 天
/// (`seven_day`)，免费方案则是 30 天。由 `window_seconds_to_tier_name` 产出、
/// tray 的月分组渲染、前端 `TIER_I18N_KEYS` 映射到 `subscription.thirtyDay`
/// 三处共用同一标识。见 #3651。
pub const TIER_THIRTY_DAY: &str = "30_day";

/// Gemini 用量分组名称（按模型而非时间窗口）。`classify_gemini_model` 输出。
pub const TIER_GEMINI_PRO: &str = "gemini_pro";
pub const TIER_GEMINI_FLASH: &str = "gemini_flash";
pub const TIER_GEMINI_FLASH_LITE: &str = "gemini_flash_lite";

/// 根据窗口秒数映射到 tier 名称（与 Claude 的命名兼容以复用前端 i18n）
fn window_seconds_to_tier_name(secs: i64) -> String {
    match secs {
        18000 => TIER_FIVE_HOUR.to_string(),
        604800 => TIER_SEVEN_DAY.to_string(),
        // Codex 免费方案的 30 天窗口。显式映射到常量，与 tray 月分组、前端
        // TIER_I18N_KEYS 保持同一标识（否则动态回退虽也得到 "30_day"，但字符串
        // 分散在多处、易和托盘/前端白名单脱节）。见 #3651。
        2_592_000 => TIER_THIRTY_DAY.to_string(),
        s => {
            let hours = s / 3600;
            if hours >= 24 {
                format!("{}_day", hours / 24)
            } else {
                format!("{}_hour", hours)
            }
        }
    }
}

/// Unix 时间戳（秒）转 ISO 8601 字符串
fn unix_ts_to_iso(ts: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(ts, 0).map(|dt| dt.to_rfc3339())
}

// ── 入口函数 ──────────────────────────────────────────────

/// 查询指定 CLI 工具的官方订阅额度。
///
/// 这里只做分发：每个工具的凭据读取与接口查询都在自己的采集函数里
/// （Claude 更进一步，整个在 `claude_quota` 模块）。加一个工具＝写一个
/// `collect_*_quota` 再在下面加一行，不必往这个函数里塞逻辑——它一度装着
/// Codex 与 Gemini 各四十行的内联实现。
///
/// `tool` 来自前端的字符串，认不出来的一律当作"没有这个工具的额度"。
///
/// 瞬时传输失败以 `Err` 传播（前端 reject → retry + 保留上次成功值）。Expired
/// 分支的"过期也试一把"重试同样用 `?` 传播瞬时错误——不能折叠成"已过期"，
/// 否则一次网络抖动会被误报成确定性的凭据过期。
pub async fn get_subscription_quota(tool: &str) -> Result<SubscriptionQuota, String> {
    match tool {
        // Claude Pro/Max only uses local data emitted by official Claude apps.
        // Do not read Claude OAuth credentials or call a private usage endpoint.
        "claude" => crate::quota::claude_quota::collect_local_quota(),
        "codex" => codex::subscription::collect_codex_quota().await,
        "gemini" => gemini::subscription::collect_gemini_quota().await,
        _ => Ok(SubscriptionQuota::not_found(tool)),
    }
}

// ── 辅助函数 ──────────────────────────────────────────────

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `tool` 是前端传进来的任意字符串，认不出来必须是确定性的「没有这个工具的
    /// 额度」，而不是报错——否则前端会把它当成瞬时失败去重试。走 `_` 分支不碰
    /// 任何凭据、不发网络请求，可以安全地在单测里调。
    #[tokio::test]
    async fn unknown_tool_reports_not_found_without_touching_credentials() {
        let quota = get_subscription_quota("definitely-not-a-tool")
            .await
            .expect("未知工具是确定性结果，不是传输失败");
        assert_eq!(quota.tool, "definitely-not-a-tool");
        assert!(!quota.success);
        assert!(matches!(
            quota.credential_status,
            CredentialStatus::NotFound
        ));
    }

    /// 24 处构造点现在靠 `skeleton` 兜住那些它们不关心的字段。改动这里的任何一个
    /// 默认值，等于一次性改掉那 24 处的行为——所以把它钉死，让改动必须先过这一关。
    #[test]
    fn skeleton_defaults_are_pinned() {
        let quota = SubscriptionQuota::skeleton("probe");
        assert_eq!(quota.tool, "probe");
        assert!(matches!(quota.credential_status, CredentialStatus::Valid));
        assert!(quota.credential_message.is_none());
        assert!(!quota.success, "骨架默认不是成功结果");
        assert!(quota.tiers.is_empty());
        assert!(quota.plan_type.is_none());
        assert!(quota.plan_renews_at.is_none());
        assert!(quota.manual_reset_credits.is_none());
        assert!(quota.extra_usage.is_none());
        assert!(quota.error.is_none());
        assert!(quota.queried_at.is_none(), "没查过就没有查询时刻");
    }

    /// 两个既有构造器改成基于骨架后，对外行为必须与重构前逐字段一致。
    #[test]
    fn not_found_and_error_keep_their_previous_shape() {
        let not_found = SubscriptionQuota::not_found("codex");
        assert_eq!(not_found.tool, "codex");
        assert!(matches!(
            not_found.credential_status,
            CredentialStatus::NotFound
        ));
        assert!(!not_found.success);
        assert!(not_found.credential_message.is_none());
        assert!(not_found.error.is_none());
        assert!(not_found.queried_at.is_none());

        let failed = SubscriptionQuota::error(
            "claude",
            CredentialStatus::Expired,
            "some_error_code".to_string(),
        );
        assert_eq!(failed.tool, "claude");
        assert!(matches!(
            failed.credential_status,
            CredentialStatus::Expired
        ));
        assert!(!failed.success);
        // 错误文案同时进 credential_message 和 error 两个字段（前端两处都在读）。
        assert_eq!(
            failed.credential_message.as_deref(),
            Some("some_error_code")
        );
        assert_eq!(failed.error.as_deref(), Some("some_error_code"));
        assert!(failed.queried_at.is_some(), "错误结果带查询时刻");
    }

    #[test]
    fn window_seconds_map_to_expected_tier_names() {
        // 官方特例窗口
        assert_eq!(window_seconds_to_tier_name(18000), TIER_FIVE_HOUR);
        assert_eq!(window_seconds_to_tier_name(604800), TIER_SEVEN_DAY);
        // Codex 免费方案的次要窗口是 30 天（30 * 24 * 3600 = 2_592_000 秒）。
        // 前端 TIER_I18N_KEYS 与 tray 月分组都需要认得 "30_day"，见 #3651。
        assert_eq!(window_seconds_to_tier_name(2_592_000), TIER_THIRTY_DAY);
        // 其他窗口按小时/天回退命名
        assert_eq!(window_seconds_to_tier_name(3600), "1_hour");
        assert_eq!(window_seconds_to_tier_name(86400), "1_day");
    }
}
