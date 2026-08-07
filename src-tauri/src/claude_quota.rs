//! Local Claude Pro/Max quota collection.
//!
//! The primary macOS source is Claude Desktop's local plan-usage history. Claude
//! Code's documented status-line JSON is a second, independently timestamped
//! source whose own observations can include reset times. Neither path reads
//! Claude credentials nor performs a network request.

use crate::config;
use crate::error::AppError;
use crate::services::subscription::{
    CredentialStatus, QuotaTier, SubscriptionQuota, TIER_FIVE_HOUR, TIER_SEVEN_DAY,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CACHE_SCHEMA_VERSION: u8 = 5;
const CACHE_FILE_NAME: &str = "claude-statusline-quota.json";
const CACHE_LOCK_FILE_NAME: &str = "claude-statusline-quota.lock";
const RESET_LATCH_SCHEMA_VERSION: u8 = 1;
const RESET_LATCH_FILE_NAME: &str = "claude-quota-reset-latch.json";
const RESET_LATCH_LOCK_FILE_NAME: &str = "claude-quota-reset-latch.lock";
#[cfg(any(target_os = "macos", test))]
const DESKTOP_HISTORY_FILE_NAME: &str = "plan-usage-history.json";
const MAX_STATUSLINE_INPUT_BYTES: u64 = 1_048_576;
const MAX_STATUSLINE_CACHE_BYTES: u64 = 256 * 1_024;
const MAX_RESET_LATCH_BYTES: u64 = 256 * 1_024;
const MAX_DESKTOP_HISTORY_BYTES: u64 = 4 * 1_048_576;
const MAX_DESKTOP_HISTORY_SAMPLES: usize = 20_000;
const MAX_CACHED_STATUSLINE_SESSIONS: usize = 32;
const MAX_CLI_ACCOUNT_BYTES: u64 = 4 * 1_048_576;
const MAX_CLAUDE_VERSION_CHARS: usize = 64;
const MAX_CACHE_AGE_SECONDS: i64 = 15 * 60;
const RESET_LATCH_PERCENTAGE_TOLERANCE: f64 = 1.0;
const RESET_LATCH_IDENTITY_TOLERANCE: f64 = 2.0;
const RESET_LATCH_OBSERVATION_TOLERANCE_MS: u64 = 15 * 60 * 1_000;
const CACHE_LOCK_TIMEOUT: Duration = Duration::from_millis(250);
const CACHE_LOCK_RETRY_DELAY: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BridgeOutcome {
    Updated,
    NoRateLimits,
    NonSubscriptionResponse,
    IgnoredStale,
}

#[derive(Debug, Deserialize)]
struct StatuslinePayload {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    rate_limits: Option<StatuslineRateLimits>,
    #[serde(default)]
    context_window: Option<StatuslineContextWindow>,
}

#[derive(Debug, Default, Deserialize)]
struct StatuslineContextWindow {
    #[serde(default)]
    current_usage: Option<serde_json::Value>,
}

#[derive(Debug, Default, Deserialize)]
struct StatuslineRateLimits {
    #[serde(default)]
    five_hour: Option<StatuslineWindow>,
    #[serde(default)]
    seven_day: Option<StatuslineWindow>,
}

#[derive(Debug, Clone, Deserialize)]
struct StatuslineWindow {
    used_percentage: f64,
    resets_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeStatuslineCache {
    schema_version: u8,
    updated_at: i64,
    sessions: Vec<CachedStatuslineSession>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedStatuslineSession {
    updated_at: i64,
    event_at_ms: i64,
    session_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    claude_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rate_limits: Option<CachedRateLimits>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedRateLimits {
    #[serde(skip_serializing_if = "Option::is_none")]
    five_hour: Option<CachedWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seven_day: Option<CachedWindow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedWindow {
    used_percentage: f64,
    resets_at: i64,
    observed_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeQuotaResetLatch {
    schema_version: u8,
    windows: LatchedResetWindows,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LatchedResetWindows {
    #[serde(rename = "five_hour", skip_serializing_if = "Option::is_none")]
    five_hour: Option<LatchedResetWindow>,
    #[serde(rename = "seven_day", skip_serializing_if = "Option::is_none")]
    seven_day: Option<LatchedResetWindow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LatchedResetWindow {
    resets_at: i64,
    latched_at: i64,
    utilization_at_latch: f64,
}

#[derive(Debug, Clone, Copy)]
enum QuotaWindowKind {
    FiveHour,
    SevenDay,
}

impl QuotaWindowKind {
    fn name(self) -> &'static str {
        match self {
            Self::FiveHour => TIER_FIVE_HOUR,
            Self::SevenDay => TIER_SEVEN_DAY,
        }
    }

    fn latch(self, windows: &LatchedResetWindows) -> Option<&LatchedResetWindow> {
        match self {
            Self::FiveHour => windows.five_hour.as_ref(),
            Self::SevenDay => windows.seven_day.as_ref(),
        }
    }

    fn latch_mut(self, windows: &mut LatchedResetWindows) -> &mut Option<LatchedResetWindow> {
        match self {
            Self::FiveHour => &mut windows.five_hour,
            Self::SevenDay => &mut windows.seven_day,
        }
    }
}

#[derive(Debug, Deserialize)]
struct DesktopPlanUsageHistory {
    version: u64,
    samples: Vec<DesktopUsageSample>,
}

#[derive(Debug, Deserialize)]
struct DesktopUsageSample {
    t: i64,
    #[serde(default)]
    org: Option<String>,
    #[serde(default)]
    u: Option<DesktopUsage>,
    // Claude Desktop v1 stored the two main windows directly on the sample.
    #[serde(default)]
    fh: Option<f64>,
    #[serde(default)]
    sd: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct DesktopUsage {
    #[serde(default)]
    fh: Option<f64>,
    #[serde(default)]
    sd: Option<f64>,
}

pub fn run_claude_statusline_bridge() -> Result<(), AppError> {
    let cache_path = statusline_cache_path();
    let (observed_at, event_at_ms) = now_times()?;
    ingest_statusline_with_event_time(io::stdin().lock(), &cache_path, observed_at, event_at_ms)?;
    Ok(())
}

pub(crate) fn collect_local_quota() -> Result<SubscriptionQuota, String> {
    let now = now_seconds().map_err(|error| error.to_string())?;
    collect_local_quota_from_paths_at(
        desktop_history_path().as_deref(),
        &statusline_cache_path(),
        &reset_latch_path(),
        now,
    )
}

/// The subscription tier, read from the Claude CLI's own account profile.
///
/// `~/.claude.json` carries `oauthAccount.organizationType`, which is where the
/// CLI keeps the plan it reports as `subscriptionType`. Reading the file costs
/// nothing; asking the CLI would mean spawning a process on every quota
/// refresh. Neither the file's credentials nor any other field is touched.
fn read_cli_plan_type() -> Option<String> {
    read_cli_plan_type_at(&config::get_claude_account_path())
}

fn read_cli_plan_type_at(path: &Path) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() > MAX_CLI_ACCOUNT_BYTES {
        return None;
    }
    let contents = fs::read_to_string(path).ok()?;
    let document: serde_json::Value = serde_json::from_str(&contents).ok()?;
    let organization_type = document
        .get("oauthAccount")?
        .get("organizationType")?
        .as_str()?;
    normalize_plan_type(organization_type)
}

/// `claude_pro` is Pro, `claude_max_5x` is Max 5x. The prefix is Anthropic's
/// namespace rather than part of the plan's name, and anything past it is left
/// as written: a tier this app has not seen should still read as itself rather
/// than be dropped for not matching a known list.
fn normalize_plan_type(organization_type: &str) -> Option<String> {
    let plan = organization_type
        .trim()
        .to_lowercase()
        .strip_prefix("claude_")
        .unwrap_or(organization_type.trim())
        .replace('_', " ");
    let plan = plan.trim().to_string();
    (!plan.is_empty()).then_some(plan)
}

fn statusline_cache_path() -> PathBuf {
    config::get_home_dir()
        .join(".llm-usage-bar")
        .join("runtime")
        .join(CACHE_FILE_NAME)
}

fn reset_latch_path() -> PathBuf {
    config::get_home_dir()
        .join(".llm-usage-bar")
        .join("runtime")
        .join(RESET_LATCH_FILE_NAME)
}

#[cfg(target_os = "macos")]
fn desktop_history_path() -> Option<PathBuf> {
    Some(
        config::get_home_dir()
            .join("Library")
            .join("Application Support")
            .join("Claude")
            .join(DESKTOP_HISTORY_FILE_NAME),
    )
}

#[cfg(not(target_os = "macos"))]
fn desktop_history_path() -> Option<PathBuf> {
    None
}

#[cfg(test)]
fn ingest_statusline(
    reader: impl Read,
    cache_path: &Path,
    observed_at: i64,
) -> Result<BridgeOutcome, AppError> {
    ingest_statusline_with_event_time(
        reader,
        cache_path,
        observed_at,
        observed_at.saturating_mul(1_000),
    )
}

fn ingest_statusline_with_event_time(
    reader: impl Read,
    cache_path: &Path,
    observed_at: i64,
    event_at_ms: i64,
) -> Result<BridgeOutcome, AppError> {
    if observed_at <= 0 || event_at_ms <= 0 {
        return Err(AppError::InvalidInput(
            "Claude status-line observation time is invalid".to_string(),
        ));
    }
    let mut bytes = Vec::new();
    reader
        .take(MAX_STATUSLINE_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| AppError::IoContext {
            context: "read Claude status-line input".to_string(),
            source,
        })?;
    if bytes.len() as u64 > MAX_STATUSLINE_INPUT_BYTES {
        return Err(AppError::InvalidInput(
            "Claude status-line input exceeds 1 MiB".to_string(),
        ));
    }

    let payload: StatuslinePayload = serde_json::from_slice(&bytes)
        .map_err(|error| AppError::Config(format!("invalid Claude status-line JSON: {error}")))?;
    let session_key = statusline_session_key(payload.session_id.as_deref())?;
    let _cache_lock = acquire_cache_lock(cache_path, CACHE_LOCK_FILE_NAME)?;
    let mut cache = read_cache(cache_path)
        .ok()
        .filter(|cache| cache.schema_version == CACHE_SCHEMA_VERSION)
        .unwrap_or_else(empty_statusline_cache);
    // A callback can be delayed before it acquires the cross-process lock.
    // Never use that older event's clock to evict newer sessions.
    let cache_clock = cache.updated_at.max(observed_at);
    prune_statusline_sessions(&mut cache, cache_clock);
    let previous = cache
        .sessions
        .iter()
        .find(|session| session.session_key == session_key)
        .cloned();
    if previous
        .as_ref()
        .is_some_and(|session| session.event_at_ms > event_at_ms)
    {
        return Ok(BridgeOutcome::IgnoredStale);
    }
    let claude_version = payload
        .version
        .as_deref()
        .map(str::trim)
        .filter(|version| !version.is_empty())
        .map(|version| version.chars().take(MAX_CLAUDE_VERSION_CHARS).collect())
        .or_else(|| {
            previous
                .as_ref()
                .and_then(|session| session.claude_version.clone())
        });

    let Some(rate_limits) = payload.rate_limits else {
        // Before the first API response, current_usage is null and rate_limits
        // is legitimately absent. Once an API response exists, an absent
        // rate_limits object means this Claude Code session is not using a
        // Claude.ai Pro/Max subscription. Only invalidate a cache proven to
        // belong to this exact session; another concurrent Pro/Max session may
        // still be the newest valid subscription observation.
        if payload
            .context_window
            .as_ref()
            .and_then(|window| window.current_usage.as_ref())
            .is_some()
        {
            cache
                .sessions
                .retain(|session| session.session_key != session_key);
            cache.sessions.push(CachedStatuslineSession {
                updated_at: observed_at,
                event_at_ms,
                session_key,
                claude_version,
                rate_limits: None,
            });
            persist_statusline_cache(cache_path, &mut cache, cache_clock)?;
            return Ok(BridgeOutcome::NonSubscriptionResponse);
        }
        return Ok(BridgeOutcome::NoRateLimits);
    };

    let previous_rate_limits = previous
        .as_ref()
        .and_then(|session| session.rate_limits.as_ref());

    let five_hour = rate_limits
        .five_hour
        .map(|window| validate_statusline_window(window, observed_at))
        .transpose()?
        .flatten();
    let seven_day = rate_limits
        .seven_day
        .map(|window| validate_statusline_window(window, observed_at))
        .transpose()?
        .flatten();
    if five_hour.is_none() && seven_day.is_none() {
        return Ok(BridgeOutcome::NoRateLimits);
    }

    let merged = CachedRateLimits {
        five_hour: five_hour.or_else(|| {
            previous_rate_limits
                .and_then(|limits| limits.five_hour.clone())
                .filter(|window| window.resets_at > observed_at)
        }),
        seven_day: seven_day.or_else(|| {
            previous_rate_limits
                .and_then(|limits| limits.seven_day.clone())
                .filter(|window| window.resets_at > observed_at)
        }),
    };
    let session_updated_at = [
        merged.five_hour.as_ref().map(|window| window.observed_at),
        merged.seven_day.as_ref().map(|window| window.observed_at),
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(observed_at);

    cache
        .sessions
        .retain(|session| session.session_key != session_key);
    cache.sessions.push(CachedStatuslineSession {
        updated_at: session_updated_at,
        event_at_ms,
        session_key,
        claude_version,
        rate_limits: Some(merged),
    });
    persist_statusline_cache(cache_path, &mut cache, cache_clock)?;
    Ok(BridgeOutcome::Updated)
}

fn empty_statusline_cache() -> ClaudeStatuslineCache {
    ClaudeStatuslineCache {
        schema_version: CACHE_SCHEMA_VERSION,
        updated_at: 0,
        sessions: Vec::new(),
    }
}

fn statusline_session_key(session_id: Option<&str>) -> Result<String, AppError> {
    let session_id = session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::InvalidInput("Claude status-line session ID is missing".to_string())
        })?;
    Ok(hex::encode(Sha256::digest(session_id.as_bytes())))
}

struct QuotaCacheLock(File);

impl Drop for QuotaCacheLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

fn acquire_cache_lock(cache_path: &Path, lock_file_name: &str) -> Result<QuotaCacheLock, AppError> {
    let parent = cache_path.parent().ok_or_else(|| {
        AppError::InvalidInput("Claude quota cache has no parent directory".to_string())
    })?;
    fs::create_dir_all(parent).map_err(|source| AppError::IoContext {
        context: "create Claude quota cache directory".to_string(),
        source,
    })?;
    let lock_path = cache_path.with_file_name(lock_file_name);
    // The lock file is only an advisory flock target; its contents are never
    // read or written, so it must not be truncated out from under a holder.
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|source| AppError::IoContext {
            context: "open Claude quota cache lock".to_string(),
            source,
        })?;
    let deadline = Instant::now() + CACHE_LOCK_TIMEOUT;
    loop {
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => break,
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(CACHE_LOCK_RETRY_DELAY);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                return Err(AppError::Message("Claude quota cache is busy".to_string()));
            }
            Err(source) => {
                return Err(AppError::IoContext {
                    context: "lock Claude quota cache".to_string(),
                    source,
                });
            }
        }
    }
    restrict_cache_permissions(&lock_path)?;
    Ok(QuotaCacheLock(file))
}

fn prune_statusline_sessions(cache: &mut ClaudeStatuslineCache, now: i64) {
    cache.sessions.retain(|session| {
        session.updated_at > 0
            && session.updated_at <= now.saturating_add(300)
            && now.saturating_sub(session.updated_at) <= MAX_CACHE_AGE_SECONDS
    });
    cache.sessions.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.event_at_ms.cmp(&left.event_at_ms))
            .then_with(|| left.session_key.cmp(&right.session_key))
    });
    cache.sessions.truncate(MAX_CACHED_STATUSLINE_SESSIONS);
    cache.updated_at = cache
        .sessions
        .iter()
        .map(|session| session.updated_at)
        .max()
        .unwrap_or(0);
}

fn persist_statusline_cache(
    cache_path: &Path,
    cache: &mut ClaudeStatuslineCache,
    now: i64,
) -> Result<(), AppError> {
    prune_statusline_sessions(cache, now);
    if cache.sessions.is_empty() {
        return remove_statusline_cache(cache_path);
    }
    config::write_json_file(cache_path, cache)
        .map_err(|_| AppError::Message("write Claude quota cache failed".to_string()))?;
    restrict_cache_permissions(cache_path)?;
    Ok(())
}

fn empty_reset_latch() -> ClaudeQuotaResetLatch {
    ClaudeQuotaResetLatch {
        schema_version: RESET_LATCH_SCHEMA_VERSION,
        windows: LatchedResetWindows::default(),
    }
}

fn prune_reset_latch(latch: &mut ClaudeQuotaResetLatch, now: i64) {
    for kind in [QuotaWindowKind::FiveHour, QuotaWindowKind::SevenDay] {
        let window = kind.latch_mut(&mut latch.windows);
        if window
            .as_ref()
            .is_some_and(|window| window.resets_at <= now)
        {
            *window = None;
        }
    }
}

fn persist_reset_latch(
    latch_path: &Path,
    latch: &mut ClaudeQuotaResetLatch,
    now: i64,
) -> Result<(), AppError> {
    prune_reset_latch(latch, now);
    if latch.windows.five_hour.is_none() && latch.windows.seven_day.is_none() {
        return remove_reset_latch(latch_path);
    }
    config::write_json_file(latch_path, latch)
        .map_err(|_| AppError::Message("write Claude quota reset latch failed".to_string()))?;
    restrict_cache_permissions(latch_path)?;
    Ok(())
}

fn remove_reset_latch(path: &Path) -> Result<(), AppError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(AppError::IoContext {
            context: "remove Claude quota reset latch".to_string(),
            source,
        }),
    }
}

fn remove_statusline_cache(path: &Path) -> Result<(), AppError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(AppError::IoContext {
            context: "remove Claude quota cache".to_string(),
            source,
        }),
    }
}

fn validate_statusline_window(
    window: StatuslineWindow,
    observed_at: i64,
) -> Result<Option<CachedWindow>, AppError> {
    if !window.used_percentage.is_finite() || !(0.0..=100.0).contains(&window.used_percentage) {
        return Err(AppError::InvalidInput(
            "Claude rate-limit percentage must be between 0 and 100".to_string(),
        ));
    }
    if window.resets_at <= 0 {
        return Err(AppError::InvalidInput(
            "Claude rate-limit reset time must be a positive Unix timestamp".to_string(),
        ));
    }
    if window.resets_at <= observed_at {
        return Ok(None);
    }
    Ok(Some(CachedWindow {
        used_percentage: window.used_percentage,
        resets_at: window.resets_at,
        observed_at,
    }))
}

#[derive(Debug, Clone)]
struct ObservedQuotaTier {
    tier: QuotaTier,
    observed_at_ms: i64,
    resets_at_unix: Option<i64>,
}

fn collect_local_quota_from_paths_at(
    desktop_path: Option<&Path>,
    statusline_path: &Path,
    latch_path: &Path,
    now: i64,
) -> Result<SubscriptionQuota, String> {
    let desktop = desktop_path
        .map(|path| read_desktop_observations(path, now))
        .unwrap_or_else(|| Err("Claude Desktop local history is unavailable".to_string()));
    let statusline = read_statusline_observations(statusline_path, now);
    // Reset metadata is supplemental. If its separate lock or file cannot be
    // used, quota collection must continue with the two percentage sources.
    let latch_lock = acquire_cache_lock(latch_path, RESET_LATCH_LOCK_FILE_NAME).ok();
    let mut reset_latch = if latch_lock.is_some() {
        read_reset_latch(latch_path).unwrap_or_else(|_| empty_reset_latch())
    } else {
        empty_reset_latch()
    };
    prune_reset_latch(&mut reset_latch, now);

    let mut tiers = Vec::new();
    // The card-level timestamp is conservative: every displayed window is at
    // least this fresh, even when their newest samples were observed at
    // different times.
    let mut observed_at_ms = None;
    let kinds = [QuotaWindowKind::FiveHour, QuotaWindowKind::SevenDay];
    let observed: Vec<(
        QuotaWindowKind,
        Option<ObservedQuotaTier>,
        Option<ObservedQuotaTier>,
    )> = kinds
        .iter()
        .map(|kind| {
            let name = kind.name();
            let pick = |source: &Result<Vec<ObservedQuotaTier>, String>| {
                source
                    .as_ref()
                    .ok()
                    .and_then(|values| values.iter().find(|value| value.tier.name == name))
                    .cloned()
            };
            (*kind, pick(&desktop), pick(&statusline))
        })
        .collect();
    // One verdict for the whole pass, decided before any window is latched:
    // identity is a property of the two sources, not of one window.
    let evidence = same_account_evidence(&observed);

    for (kind, desktop_tier, statusline_tier) in observed {
        update_reset_latch(
            kind,
            statusline_tier.as_ref(),
            evidence,
            &mut reset_latch,
            now,
        );

        let selected = match (desktop_tier, statusline_tier) {
            (Some(desktop), Some(statusline))
                if desktop.observed_at_ms > statusline.observed_at_ms =>
            {
                // Claude Desktop and Claude Code do not expose a common
                // account identifier. A reset timestamp may cross sources only
                // through a latch formed from same-pass percentage and time
                // agreement above.
                Some((desktop, true))
            }
            (Some(_), Some(statusline)) => Some((statusline, false)),
            (Some(desktop), None) => Some((desktop, true)),
            (None, Some(statusline)) => Some((statusline, false)),
            (None, None) => None,
        };

        if let Some((mut selected, selected_desktop)) = selected {
            if selected_desktop && selected.tier.resets_at.is_none() {
                attach_latched_reset(kind, &mut selected, &mut reset_latch, now);
            }
            observed_at_ms = Some(
                observed_at_ms
                    .unwrap_or(i64::MAX)
                    .min(selected.observed_at_ms),
            );
            tiers.push(selected.tier);
        }
    }

    if latch_lock.is_some() {
        // A latch failure must never suppress otherwise valid percentages.
        let _ = persist_reset_latch(latch_path, &mut reset_latch, now);
    }

    if tiers.is_empty() {
        let desktop_error = desktop
            .err()
            .unwrap_or_else(|| "no current Desktop quota windows".to_string());
        let statusline_error = statusline
            .err()
            .unwrap_or_else(|| "no current Claude Code quota windows".to_string());
        return Err(format!(
            "Claude quota is unavailable. Desktop: {desktop_error}. Claude Code: {statusline_error}"
        ));
    }

    Ok(SubscriptionQuota {
        tool: "claude".to_string(),
        credential_status: CredentialStatus::Valid,
        credential_message: None,
        success: true,
        tiers,
        plan_type: read_cli_plan_type(),
        plan_renews_at: None,
        manual_reset_credits: None,
        extra_usage: None,
        error: None,
        queried_at: observed_at_ms,
    })
}

/// Whether this pass showed the two local sources describe the same account.
///
/// Claude Desktop and Claude Code expose no common account identifier, so a
/// reset instant may only cross from one to the other when their percentages
/// agree. Agreement on a single window is weak evidence — two unrelated
/// accounts both sit at 0% early in a five-hour window — so the verdict is
/// taken across every window both sources reported, and one disagreement
/// rejects the whole pass rather than just that window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SameAccountEvidence {
    /// At least one window agreed and none disagreed.
    Confirmed,
    /// Some window disagreed: treat the sources as different accounts.
    Contradicted,
    /// Nothing comparable this pass. Neither confirms nor denies, so latches
    /// from earlier passes stand.
    Absent,
}

/// The two sources are never sampled together — Claude Desktop writes every 15
/// minutes while the status line is live — so the windows carry very different
/// amounts of identity information. Measured on a real account over one
/// afternoon, the five-hour figure climbed 5% to 35% while the weekly figure
/// moved 63% to 66%. Within one 15-minute sampling gap that is around six
/// points of honest drift on the fast window against under half a point on the
/// slow one.
///
/// So the weekly window decides identity, where near-equality is both fair to
/// ask and hard for two unrelated accounts to satisfy. The five-hour window
/// only ever vetoes: it is too noisy to confirm anything, but an *older*
/// reading above a newer one is impossible for a single account, since
/// utilization climbs until the window resets.
fn same_account_evidence(
    pairs: &[(
        QuotaWindowKind,
        Option<ObservedQuotaTier>,
        Option<ObservedQuotaTier>,
    )],
) -> SameAccountEvidence {
    let mut confirmed = false;
    for (kind, desktop, statusline) in pairs {
        let (Some(desktop), Some(statusline)) = (desktop, statusline) else {
            continue;
        };
        if desktop.observed_at_ms.abs_diff(statusline.observed_at_ms)
            > RESET_LATCH_OBSERVATION_TOLERANCE_MS
        {
            continue;
        }
        let difference = (desktop.tier.utilization - statusline.tier.utilization).abs();
        match kind {
            QuotaWindowKind::SevenDay => {
                if difference > RESET_LATCH_IDENTITY_TOLERANCE {
                    return SameAccountEvidence::Contradicted;
                }
                confirmed = true;
            }
            QuotaWindowKind::FiveHour => {
                let (older, newer) = if desktop.observed_at_ms <= statusline.observed_at_ms {
                    (desktop.tier.utilization, statusline.tier.utilization)
                } else {
                    (statusline.tier.utilization, desktop.tier.utilization)
                };
                if older > newer + RESET_LATCH_PERCENTAGE_TOLERANCE {
                    return SameAccountEvidence::Contradicted;
                }
            }
        }
    }
    if confirmed {
        SameAccountEvidence::Confirmed
    } else {
        SameAccountEvidence::Absent
    }
}

fn update_reset_latch(
    kind: QuotaWindowKind,
    statusline: Option<&ObservedQuotaTier>,
    evidence: SameAccountEvidence,
    latch: &mut ClaudeQuotaResetLatch,
    now: i64,
) {
    let window = kind.latch_mut(&mut latch.windows);
    match evidence {
        SameAccountEvidence::Contradicted => *window = None,
        SameAccountEvidence::Absent => {}
        SameAccountEvidence::Confirmed => {
            // The reset instant is the status line's own; the evidence only
            // decides whether it may later be read alongside a Desktop
            // percentage. So a window the Desktop source never reported can
            // still latch, on identity established by the other window.
            let Some(statusline) = statusline else { return };
            let Some(resets_at) = statusline.resets_at_unix else {
                return;
            };
            if resets_at > now {
                *window = Some(LatchedResetWindow {
                    resets_at,
                    latched_at: now,
                    utilization_at_latch: statusline.tier.utilization,
                });
            }
        }
    }
}

fn attach_latched_reset(
    kind: QuotaWindowKind,
    desktop: &mut ObservedQuotaTier,
    latch: &mut ClaudeQuotaResetLatch,
    now: i64,
) {
    let window = kind.latch_mut(&mut latch.windows);
    let Some(latched) = window.as_ref() else {
        return;
    };
    if latched.resets_at <= now
        || desktop.tier.utilization
            < latched.utilization_at_latch - RESET_LATCH_PERCENTAGE_TOLERANCE
    {
        *window = None;
        return;
    }
    let Some(resets_at) = chrono::DateTime::from_timestamp(latched.resets_at, 0)
        .map(|timestamp| timestamp.to_rfc3339())
    else {
        *window = None;
        return;
    };
    desktop.tier.resets_at = Some(resets_at);
}

fn read_desktop_observations(
    history_path: &Path,
    now: i64,
) -> Result<Vec<ObservedQuotaTier>, String> {
    let metadata = fs::metadata(history_path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            "local history has not been created yet".to_string()
        } else {
            format!("failed to inspect local history: {error}")
        }
    })?;
    if metadata.len() > MAX_DESKTOP_HISTORY_BYTES {
        return Err("local history exceeds 4 MiB".to_string());
    }
    let bytes =
        fs::read(history_path).map_err(|error| format!("failed to read local history: {error}"))?;
    if bytes.len() as u64 > MAX_DESKTOP_HISTORY_BYTES {
        return Err("local history exceeds 4 MiB".to_string());
    }
    let history: DesktopPlanUsageHistory = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid local history JSON: {error}"))?;
    if !matches!(history.version, 1 | 2) {
        return Err(format!(
            "unsupported local history schema: {}",
            history.version
        ));
    }
    if history.samples.len() > MAX_DESKTOP_HISTORY_SAMPLES {
        return Err("local history contains too many samples".to_string());
    }
    let latest_sample = history
        .samples
        .iter()
        .max_by_key(|sample| sample.t)
        .ok_or_else(|| "local history contains no samples".to_string())?;

    let now_ms = now.saturating_mul(1_000);
    if latest_sample.t <= 0 || latest_sample.t > now_ms.saturating_add(300_000) {
        return Err("invalid local history sample timestamp".to_string());
    }
    if now_ms.saturating_sub(latest_sample.t) > MAX_CACHE_AGE_SECONDS.saturating_mul(1_000) {
        return Err("local history is stale; keep Claude Desktop open to refresh it".to_string());
    }

    let active_org = normalized_org(latest_sample.org.as_deref());
    let mut tiers = Vec::new();
    for (name, select_utilization) in [
        (
            TIER_FIVE_HOUR,
            desktop_five_hour as fn(&DesktopUsageSample) -> Option<f64>,
        ),
        (
            TIER_SEVEN_DAY,
            desktop_seven_day as fn(&DesktopUsageSample) -> Option<f64>,
        ),
    ] {
        let Some((sample, utilization)) = history
            .samples
            .iter()
            .filter(|sample| normalized_org(sample.org.as_deref()) == active_org)
            .filter_map(|sample| select_utilization(sample).map(|value| (sample, value)))
            .max_by_key(|(sample, _)| sample.t)
        else {
            continue;
        };
        if sample.t <= 0 || sample.t > now_ms.saturating_add(300_000) {
            return Err("invalid local history sample timestamp".to_string());
        }
        if now_ms.saturating_sub(sample.t) > MAX_CACHE_AGE_SECONDS.saturating_mul(1_000) {
            continue;
        }
        validate_percentage(name, utilization)?;
        tiers.push(ObservedQuotaTier {
            tier: QuotaTier {
                name: name.to_string(),
                utilization,
                resets_at: None,
                used_value_usd: None,
                max_value_usd: None,
            },
            observed_at_ms: sample.t,
            resets_at_unix: None,
        });
    }
    if tiers.is_empty() {
        return Err("local history has no current supported quota windows".to_string());
    }
    Ok(tiers)
}

fn normalized_org(org: Option<&str>) -> Option<&str> {
    org.map(str::trim).filter(|value| !value.is_empty())
}

fn desktop_five_hour(sample: &DesktopUsageSample) -> Option<f64> {
    sample.u.as_ref().and_then(|usage| usage.fh).or(sample.fh)
}

fn desktop_seven_day(sample: &DesktopUsageSample) -> Option<f64> {
    sample.u.as_ref().and_then(|usage| usage.sd).or(sample.sd)
}

fn validate_percentage(name: &str, percentage: f64) -> Result<(), String> {
    if percentage.is_finite() && (0.0..=100.0).contains(&percentage) {
        Ok(())
    } else {
        Err(format!(
            "invalid Claude {name} utilization percentage in local history"
        ))
    }
}

#[cfg(test)]
fn collect_statusline_quota_from_path_at(
    cache_path: &Path,
    now: i64,
) -> Result<SubscriptionQuota, String> {
    let observations = read_statusline_observations(cache_path, now)?;
    let queried_at = observations.iter().map(|value| value.observed_at_ms).min();
    Ok(SubscriptionQuota {
        tool: "claude".to_string(),
        credential_status: CredentialStatus::Valid,
        credential_message: None,
        success: true,
        tiers: observations.into_iter().map(|value| value.tier).collect(),
        // Both Claude collectors describe the same account, so both report its
        // plan — whichever one a given machine ends up serving.
        plan_type: read_cli_plan_type(),
        plan_renews_at: None,
        manual_reset_credits: None,
        extra_usage: None,
        error: None,
        queried_at,
    })
}

fn read_statusline_observations(
    cache_path: &Path,
    now: i64,
) -> Result<Vec<ObservedQuotaTier>, String> {
    let cache = read_cache(cache_path).map_err(|error| match error {
        CacheReadError::Missing => "no Pro/Max rate-limit sample has been received".to_string(),
        CacheReadError::Invalid(message) => message,
    })?;
    if cache.schema_version != CACHE_SCHEMA_VERSION {
        return Err(format!(
            "unsupported Claude status-line cache schema: {}",
            cache.schema_version
        ));
    }
    if cache.updated_at <= 0 || cache.updated_at > now.saturating_add(300) {
        return Err("invalid Claude status-line cache timestamp".to_string());
    }
    if cache.sessions.is_empty() || cache.sessions.len() > MAX_CACHED_STATUSLINE_SESSIONS {
        return Err("invalid Claude status-line session cache".to_string());
    }
    let newest_session_at = cache
        .sessions
        .iter()
        .map(|session| session.updated_at)
        .max()
        .unwrap_or(0);
    if newest_session_at != cache.updated_at {
        return Err("invalid Claude status-line cache timestamp".to_string());
    }
    if now.saturating_sub(cache.updated_at) > MAX_CACHE_AGE_SECONDS {
        return Err("Pro/Max rate-limit sample is stale".to_string());
    }

    let mut five_hour = None;
    let mut seven_day = None;
    for session in &cache.sessions {
        if session.updated_at <= 0 || session.updated_at > now.saturating_add(300) {
            return Err("invalid Claude status-line session timestamp".to_string());
        }
        if session.event_at_ms <= 0
            || session.event_at_ms > now.saturating_add(300).saturating_mul(1_000)
        {
            return Err("invalid Claude status-line session event time".to_string());
        }
        if now.saturating_sub(session.updated_at) > MAX_CACHE_AGE_SECONDS {
            continue;
        }
        if session.session_key.len() != 64
            || !session
                .session_key
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("invalid Claude status-line session key".to_string());
        }
        let Some(rate_limits) = session.rate_limits.as_ref() else {
            continue;
        };
        if let Some(candidate) =
            cached_window_to_observation(TIER_FIVE_HOUR, rate_limits.five_hour.as_ref(), now)?
        {
            if five_hour
                .as_ref()
                .is_none_or(|current: &ObservedQuotaTier| {
                    candidate.observed_at_ms > current.observed_at_ms
                })
            {
                five_hour = Some(candidate);
            }
        }
        if let Some(candidate) =
            cached_window_to_observation(TIER_SEVEN_DAY, rate_limits.seven_day.as_ref(), now)?
        {
            if seven_day
                .as_ref()
                .is_none_or(|current: &ObservedQuotaTier| {
                    candidate.observed_at_ms > current.observed_at_ms
                })
            {
                seven_day = Some(candidate);
            }
        }
    }
    let observations = [five_hour, seven_day]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if observations.is_empty() {
        return Err("Pro/Max rate-limit sample has expired".to_string());
    }
    Ok(observations)
}

fn cached_window_to_observation(
    name: &str,
    window: Option<&CachedWindow>,
    now: i64,
) -> Result<Option<ObservedQuotaTier>, String> {
    let Some(window) = window else {
        return Ok(None);
    };
    if !window.used_percentage.is_finite() || !(0.0..=100.0).contains(&window.used_percentage) {
        return Err(format!("invalid Claude {name} rate-limit percentage"));
    }
    if window.observed_at <= 0 || window.resets_at <= 0 {
        return Err(format!("invalid Claude {name} rate-limit timestamp"));
    }
    if window.observed_at > now.saturating_add(300) {
        return Err(format!("invalid Claude {name} observation timestamp"));
    }
    if now.saturating_sub(window.observed_at) > MAX_CACHE_AGE_SECONDS {
        return Ok(None);
    }
    if window.resets_at <= now {
        return Ok(None);
    }
    let resets_at = chrono::DateTime::from_timestamp(window.resets_at, 0)
        .map(|timestamp| timestamp.to_rfc3339())
        .ok_or_else(|| format!("invalid Claude {name} reset timestamp"))?;
    Ok(Some(ObservedQuotaTier {
        tier: QuotaTier {
            name: name.to_string(),
            utilization: window.used_percentage,
            resets_at: Some(resets_at),
            used_value_usd: None,
            max_value_usd: None,
        },
        observed_at_ms: window.observed_at.saturating_mul(1_000),
        resets_at_unix: Some(window.resets_at),
    }))
}

#[derive(Debug)]
enum CacheReadError {
    Missing,
    Invalid(String),
}

fn read_cache(path: &Path) -> Result<ClaudeStatuslineCache, CacheReadError> {
    let file = File::open(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            CacheReadError::Missing
        } else {
            CacheReadError::Invalid(format!("failed to read Claude quota cache: {error}"))
        }
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_STATUSLINE_CACHE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            CacheReadError::Invalid(format!("failed to read Claude quota cache: {error}"))
        })?;
    if bytes.len() as u64 > MAX_STATUSLINE_CACHE_BYTES {
        return Err(CacheReadError::Invalid(
            "Claude quota cache exceeds 256 KiB".to_string(),
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| CacheReadError::Invalid(format!("invalid Claude quota cache: {error}")))
}

fn read_reset_latch(path: &Path) -> Result<ClaudeQuotaResetLatch, CacheReadError> {
    let file = File::open(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            CacheReadError::Missing
        } else {
            CacheReadError::Invalid("failed to read Claude quota reset latch".to_string())
        }
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_RESET_LATCH_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            CacheReadError::Invalid("failed to read Claude quota reset latch".to_string())
        })?;
    if bytes.len() as u64 > MAX_RESET_LATCH_BYTES {
        return Err(CacheReadError::Invalid(
            "Claude quota reset latch exceeds 256 KiB".to_string(),
        ));
    }
    let latch: ClaudeQuotaResetLatch = serde_json::from_slice(&bytes)
        .map_err(|_| CacheReadError::Invalid("invalid Claude quota reset latch".to_string()))?;
    validate_reset_latch(&latch)?;
    Ok(latch)
}

fn validate_reset_latch(latch: &ClaudeQuotaResetLatch) -> Result<(), CacheReadError> {
    if latch.schema_version != RESET_LATCH_SCHEMA_VERSION {
        return Err(CacheReadError::Invalid(
            "unsupported Claude quota reset latch schema".to_string(),
        ));
    }
    for kind in [QuotaWindowKind::FiveHour, QuotaWindowKind::SevenDay] {
        let Some(window) = kind.latch(&latch.windows) else {
            continue;
        };
        if window.resets_at <= 0
            || window.latched_at <= 0
            || chrono::DateTime::from_timestamp(window.resets_at, 0).is_none()
        {
            return Err(CacheReadError::Invalid(format!(
                "invalid Claude {} reset latch timestamp",
                kind.name()
            )));
        }
        if !window.utilization_at_latch.is_finite()
            || !(0.0..=100.0).contains(&window.utilization_at_latch)
        {
            return Err(CacheReadError::Invalid(format!(
                "invalid Claude {} reset latch percentage",
                kind.name()
            )));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn restrict_cache_permissions(path: &Path) -> Result<(), AppError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|source| {
        AppError::IoContext {
            context: "set Claude quota cache permissions".to_string(),
            source,
        }
    })
}

#[cfg(not(unix))]
fn restrict_cache_permissions(_path: &Path) -> Result<(), AppError> {
    Ok(())
}

fn now_seconds() -> Result<i64, AppError> {
    now_times().map(|(seconds, _)| seconds)
}

fn now_times() -> Result<(i64, i64), AppError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| {
            (
                duration.as_secs() as i64,
                duration.as_millis().min(i64::MAX as u128) as i64,
            )
        })
        .map_err(|error| AppError::Message(format!("system clock before Unix epoch: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;
    use uuid::Uuid;

    fn temp_cache_path() -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "llm-usage-bar-claude-statusline-{}",
                Uuid::new_v4()
            ))
            .join(CACHE_FILE_NAME)
    }

    fn temp_history_path() -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "llm-usage-bar-claude-desktop-history-{}",
                Uuid::new_v4()
            ))
            .join(DESKTOP_HISTORY_FILE_NAME)
    }

    fn temp_latch_path() -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "llm-usage-bar-claude-reset-latch-{}",
                Uuid::new_v4()
            ))
            .join(RESET_LATCH_FILE_NAME)
    }

    fn temp_quota_paths() -> (PathBuf, PathBuf, PathBuf) {
        let directory = std::env::temp_dir().join(format!(
            "llm-usage-bar-claude-local-quota-{}",
            Uuid::new_v4()
        ));
        (
            directory.join(DESKTOP_HISTORY_FILE_NAME),
            directory.join(CACHE_FILE_NAME),
            directory.join(RESET_LATCH_FILE_NAME),
        )
    }

    fn write_desktop_history(
        path: &Path,
        observed_at_ms: i64,
        five_hour: Option<f64>,
        seven_day: Option<f64>,
    ) {
        config::write_json_file(
            path,
            &json!({
                "version": 2,
                "samples": [{
                    "t": observed_at_ms,
                    "u": {
                        "fh": five_hour,
                        "sd": seven_day
                    }
                }]
            }),
        )
        .unwrap();
    }

    fn write_statusline_cache(
        path: &Path,
        observed_at: i64,
        five_hour: Option<(f64, i64)>,
        seven_day: Option<(f64, i64)>,
    ) {
        let cached_window = |(used_percentage, resets_at)| CachedWindow {
            used_percentage,
            resets_at,
            observed_at,
        };
        let cache = ClaudeStatuslineCache {
            schema_version: CACHE_SCHEMA_VERSION,
            updated_at: observed_at,
            sessions: vec![CachedStatuslineSession {
                updated_at: observed_at,
                event_at_ms: observed_at.saturating_mul(1_000),
                session_key: statusline_session_key(Some("latch-test-session")).unwrap(),
                claude_version: None,
                rate_limits: Some(CachedRateLimits {
                    five_hour: five_hour.map(cached_window),
                    seven_day: seven_day.map(cached_window),
                }),
            }],
        };
        config::write_json_file(path, &cache).unwrap();
    }

    fn write_reset_latch(
        path: &Path,
        five_hour: Option<LatchedResetWindow>,
        seven_day: Option<LatchedResetWindow>,
    ) {
        config::write_json_file(
            path,
            &ClaudeQuotaResetLatch {
                schema_version: RESET_LATCH_SCHEMA_VERSION,
                windows: LatchedResetWindows {
                    five_hour,
                    seven_day,
                },
            },
        )
        .unwrap();
    }

    fn reset_window(
        resets_at: i64,
        latched_at: i64,
        utilization_at_latch: f64,
    ) -> LatchedResetWindow {
        LatchedResetWindow {
            resets_at,
            latched_at,
            utilization_at_latch,
        }
    }

    fn find_tier<'a>(quota: &'a SubscriptionQuota, name: &str) -> &'a QuotaTier {
        quota.tiers.iter().find(|tier| tier.name == name).unwrap()
    }

    #[test]
    fn bridge_persists_only_sanitized_rate_limit_fields() {
        let path = temp_cache_path();
        let input = serde_json::to_vec(&json!({
            "cwd": "/private/project",
            "session_id": "secret-session",
            "transcript_path": "/private/transcript.jsonl",
            "version": "2.1.220",
            "rate_limits": {
                "five_hour": {
                    "used_percentage": 23.5,
                    "resets_at": 20_000
                },
                "seven_day": {
                    "used_percentage": 41.2,
                    "resets_at": 30_000
                }
            }
        }))
        .unwrap();

        assert_eq!(
            ingest_statusline(Cursor::new(input), &path, 10_000).unwrap(),
            BridgeOutcome::Updated
        );
        let stored: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(stored["schemaVersion"], json!(5));
        assert_eq!(stored["updatedAt"], json!(10_000));
        assert_eq!(stored["sessions"][0]["claudeVersion"], json!("2.1.220"));
        assert_eq!(
            stored["sessions"][0]["sessionKey"],
            json!(statusline_session_key(Some("secret-session")).unwrap())
        );
        assert_ne!(stored["sessions"][0]["sessionKey"], json!("secret-session"));
        assert_eq!(
            stored["sessions"][0]["rateLimits"]["fiveHour"]["usedPercentage"],
            json!(23.5)
        );
        assert!(stored.get("cwd").is_none());
        assert!(stored.get("sessionId").is_none());
        assert!(stored.get("transcriptPath").is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            let lock_path = path.with_file_name(CACHE_LOCK_FILE_NAME);
            assert_eq!(
                fs::metadata(lock_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn missing_rate_limits_do_not_overwrite_the_last_valid_cache() {
        let path = temp_cache_path();
        let first = serde_json::to_vec(&json!({
            "session_id": "before-first-response",
            "rate_limits": {
                "five_hour": {
                    "used_percentage": 10.0,
                    "resets_at": 20_000
                }
            }
        }))
        .unwrap();
        ingest_statusline(Cursor::new(first), &path, 10_000).unwrap();
        let before = fs::read(&path).unwrap();

        let without_limits = serde_json::to_vec(&json!({
            "version": "2.1.220",
            "session_id": "before-first-response"
        }))
        .unwrap();
        assert_eq!(
            ingest_statusline(Cursor::new(without_limits), &path, 11_000).unwrap(),
            BridgeOutcome::NoRateLimits
        );
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn bridge_rejects_a_missing_session_id_without_writing_cache() {
        let path = temp_cache_path();
        let input = serde_json::to_vec(&json!({
            "rate_limits": {
                "five_hour": {
                    "used_percentage": 10.0,
                    "resets_at": 20_000
                }
            }
        }))
        .unwrap();

        let error = ingest_statusline(Cursor::new(input), &path, 10_000).unwrap_err();
        assert!(error.to_string().contains("session ID is missing"));
        assert!(!path.exists());
    }

    #[test]
    fn non_subscription_response_invalidates_the_statusline_cache() {
        let path = temp_cache_path();
        let subscription = serde_json::to_vec(&json!({
            "session_id": "same-session",
            "rate_limits": {
                "five_hour": {
                    "used_percentage": 10.0,
                    "resets_at": 20_000
                }
            }
        }))
        .unwrap();
        ingest_statusline(Cursor::new(subscription), &path, 10_000).unwrap();
        assert!(path.exists());

        let non_subscription = serde_json::to_vec(&json!({
            "session_id": "same-session",
            "context_window": {
                "current_usage": {
                    "input_tokens": 123,
                    "output_tokens": 45
                }
            }
        }))
        .unwrap();
        assert_eq!(
            ingest_statusline(Cursor::new(non_subscription), &path, 10_100).unwrap(),
            BridgeOutcome::NonSubscriptionResponse
        );
        let cache = read_cache(&path).unwrap();
        assert_eq!(cache.sessions.len(), 1);
        assert!(cache.sessions[0].rate_limits.is_none());
        assert!(collect_statusline_quota_from_path_at(&path, 10_100).is_err());
    }

    #[test]
    fn non_subscription_response_does_not_delete_another_sessions_cache() {
        let path = temp_cache_path();
        let subscription = serde_json::to_vec(&json!({
            "session_id": "pro-session",
            "rate_limits": {
                "five_hour": {
                    "used_percentage": 10.0,
                    "resets_at": 20_000
                }
            }
        }))
        .unwrap();
        ingest_statusline(Cursor::new(subscription), &path, 10_000).unwrap();
        let non_subscription = serde_json::to_vec(&json!({
            "session_id": "api-session",
            "context_window": {
                "current_usage": {
                    "input_tokens": 123,
                    "output_tokens": 45
                }
            }
        }))
        .unwrap();
        assert_eq!(
            ingest_statusline(Cursor::new(non_subscription), &path, 10_100).unwrap(),
            BridgeOutcome::NonSubscriptionResponse
        );
        let cache = read_cache(&path).unwrap();
        assert_eq!(cache.sessions.len(), 2);
        assert!(cache.sessions.iter().any(|session| {
            session.session_key == statusline_session_key(Some("pro-session")).unwrap()
                && session.rate_limits.is_some()
        }));
        assert!(cache.sessions.iter().any(|session| {
            session.session_key == statusline_session_key(Some("api-session")).unwrap()
                && session.rate_limits.is_none()
        }));
        let quota = collect_statusline_quota_from_path_at(&path, 10_100).unwrap();
        assert_eq!(quota.tiers[0].utilization, 10.0);
    }

    #[test]
    fn independently_missing_window_keeps_an_unexpired_previous_value() {
        let path = temp_cache_path();
        let first = serde_json::to_vec(&json!({
            "session_id": "same-session",
            "rate_limits": {
                "five_hour": {
                    "used_percentage": 10.0,
                    "resets_at": 20_000
                },
                "seven_day": {
                    "used_percentage": 20.0,
                    "resets_at": 30_000
                }
            }
        }))
        .unwrap();
        ingest_statusline(Cursor::new(first), &path, 10_000).unwrap();

        let second = serde_json::to_vec(&json!({
            "session_id": "same-session",
            "rate_limits": {
                "five_hour": {
                    "used_percentage": 15.0,
                    "resets_at": 21_000
                }
            }
        }))
        .unwrap();
        ingest_statusline(Cursor::new(second), &path, 10_100).unwrap();

        let cache = read_cache(&path).unwrap();
        assert_eq!(
            cache.sessions[0]
                .rate_limits
                .as_ref()
                .unwrap()
                .five_hour
                .as_ref()
                .unwrap()
                .used_percentage,
            15.0
        );
        assert_eq!(
            cache.sessions[0]
                .rate_limits
                .as_ref()
                .unwrap()
                .seven_day
                .as_ref()
                .unwrap()
                .used_percentage,
            20.0
        );
    }

    #[test]
    fn a_new_statusline_session_does_not_merge_another_sessions_windows() {
        let path = temp_cache_path();
        let first = serde_json::to_vec(&json!({
            "session_id": "first-session",
            "rate_limits": {
                "five_hour": {
                    "used_percentage": 10.0,
                    "resets_at": 20_000
                },
                "seven_day": {
                    "used_percentage": 20.0,
                    "resets_at": 30_000
                }
            }
        }))
        .unwrap();
        ingest_statusline(Cursor::new(first), &path, 10_000).unwrap();

        let second = serde_json::to_vec(&json!({
            "session_id": "second-session",
            "rate_limits": {
                "five_hour": {
                    "used_percentage": 15.0,
                    "resets_at": 21_000
                }
            }
        }))
        .unwrap();
        ingest_statusline(Cursor::new(second), &path, 10_100).unwrap();

        let cache = read_cache(&path).unwrap();
        assert_eq!(cache.sessions.len(), 2);
        let second = cache
            .sessions
            .iter()
            .find(|session| {
                session.session_key == statusline_session_key(Some("second-session")).unwrap()
            })
            .unwrap();
        assert_eq!(
            second
                .rate_limits
                .as_ref()
                .unwrap()
                .five_hour
                .as_ref()
                .unwrap()
                .used_percentage,
            15.0
        );
        assert!(second.rate_limits.as_ref().unwrap().seven_day.is_none());
    }

    #[test]
    fn switching_one_pro_session_to_api_keeps_another_pro_session() {
        let path = temp_cache_path();
        for (session_id, used_percentage, observed_at) in [
            ("first-session", 10.0, 10_000),
            ("second-session", 20.0, 10_100),
        ] {
            let input = serde_json::to_vec(&json!({
                "session_id": session_id,
                "rate_limits": {
                    "five_hour": {
                        "used_percentage": used_percentage,
                        "resets_at": 20_000
                    }
                }
            }))
            .unwrap();
            ingest_statusline(Cursor::new(input), &path, observed_at).unwrap();
        }

        let non_subscription = serde_json::to_vec(&json!({
            "session_id": "second-session",
            "context_window": {
                "current_usage": {
                    "input_tokens": 123,
                    "output_tokens": 45
                }
            }
        }))
        .unwrap();
        ingest_statusline(Cursor::new(non_subscription), &path, 10_200).unwrap();

        let cache = read_cache(&path).unwrap();
        assert_eq!(cache.sessions.len(), 2);
        assert!(cache.sessions.iter().any(|session| {
            session.session_key == statusline_session_key(Some("first-session")).unwrap()
                && session.rate_limits.is_some()
        }));
        assert!(cache.sessions.iter().any(|session| {
            session.session_key == statusline_session_key(Some("second-session")).unwrap()
                && session.rate_limits.is_none()
        }));
        let quota = collect_statusline_quota_from_path_at(&path, 10_200).unwrap();
        assert_eq!(quota.tiers[0].utilization, 10.0);
    }

    #[test]
    fn delayed_same_session_events_cannot_roll_back_or_revive_quota() {
        let pro_payload = |used_percentage| {
            serde_json::to_vec(&json!({
                "session_id": "same-session",
                "rate_limits": {
                    "five_hour": {
                        "used_percentage": used_percentage,
                        "resets_at": 20_000
                    }
                }
            }))
            .unwrap()
        };
        let non_subscription_payload = || {
            serde_json::to_vec(&json!({
                "session_id": "same-session",
                "context_window": {
                    "current_usage": {
                        "input_tokens": 123,
                        "output_tokens": 45
                    }
                }
            }))
            .unwrap()
        };

        let pro_then_old_pro = temp_cache_path();
        ingest_statusline_with_event_time(
            Cursor::new(pro_payload(20.0)),
            &pro_then_old_pro,
            10_000,
            10_000_200,
        )
        .unwrap();
        assert_eq!(
            ingest_statusline_with_event_time(
                Cursor::new(pro_payload(10.0)),
                &pro_then_old_pro,
                10_000,
                10_000_100,
            )
            .unwrap(),
            BridgeOutcome::IgnoredStale
        );
        assert_eq!(
            collect_statusline_quota_from_path_at(&pro_then_old_pro, 10_000)
                .unwrap()
                .tiers[0]
                .utilization,
            20.0
        );

        let tombstone_then_old_pro = temp_cache_path();
        ingest_statusline_with_event_time(
            Cursor::new(non_subscription_payload()),
            &tombstone_then_old_pro,
            10_000,
            10_000_200,
        )
        .unwrap();
        assert_eq!(
            ingest_statusline_with_event_time(
                Cursor::new(pro_payload(10.0)),
                &tombstone_then_old_pro,
                9_600,
                9_600_100,
            )
            .unwrap(),
            BridgeOutcome::IgnoredStale
        );
        assert!(collect_statusline_quota_from_path_at(&tombstone_then_old_pro, 10_000).is_err());

        let pro_then_old_tombstone = temp_cache_path();
        ingest_statusline_with_event_time(
            Cursor::new(pro_payload(20.0)),
            &pro_then_old_tombstone,
            10_000,
            10_000_200,
        )
        .unwrap();
        assert_eq!(
            ingest_statusline_with_event_time(
                Cursor::new(non_subscription_payload()),
                &pro_then_old_tombstone,
                10_000,
                10_000_100,
            )
            .unwrap(),
            BridgeOutcome::IgnoredStale
        );
        assert_eq!(
            collect_statusline_quota_from_path_at(&pro_then_old_tombstone, 10_000)
                .unwrap()
                .tiers[0]
                .utilization,
            20.0
        );
    }

    #[test]
    fn repeated_statusline_payload_is_a_fresh_observation() {
        let path = temp_cache_path();
        let input = serde_json::to_vec(&json!({
            "session_id": "same-session",
            "rate_limits": {
                "five_hour": {
                    "used_percentage": 10.0,
                    "resets_at": 20_000
                }
            }
        }))
        .unwrap();
        ingest_statusline(Cursor::new(input.clone()), &path, 10_000).unwrap();
        ingest_statusline(Cursor::new(input), &path, 11_000).unwrap();

        let cache = read_cache(&path).unwrap();
        assert_eq!(cache.updated_at, 11_000);
        assert_eq!(
            cache.sessions[0]
                .rate_limits
                .as_ref()
                .unwrap()
                .five_hour
                .as_ref()
                .unwrap()
                .observed_at,
            11_000
        );
    }

    #[test]
    fn collector_ignores_expired_windows_and_converts_reset_time() {
        let path = temp_cache_path();
        let cache = ClaudeStatuslineCache {
            schema_version: CACHE_SCHEMA_VERSION,
            updated_at: 10_000,
            sessions: vec![CachedStatuslineSession {
                updated_at: 10_000,
                event_at_ms: 10_000_000,
                session_key: statusline_session_key(Some("session")).unwrap(),
                claude_version: Some("2.1.220".to_string()),
                rate_limits: Some(CachedRateLimits {
                    five_hour: Some(CachedWindow {
                        used_percentage: 80.0,
                        resets_at: 10_500,
                        observed_at: 10_000,
                    }),
                    seven_day: Some(CachedWindow {
                        used_percentage: 40.0,
                        resets_at: 20_000,
                        observed_at: 10_000,
                    }),
                }),
            }],
        };
        config::write_json_file(&path, &cache).unwrap();

        let quota = collect_statusline_quota_from_path_at(&path, 10_800).unwrap();
        assert_eq!(quota.tiers.len(), 1);
        assert_eq!(quota.tiers[0].name, TIER_SEVEN_DAY);
        assert_eq!(quota.tiers[0].utilization, 40.0);
        assert_eq!(
            quota.tiers[0].resets_at.as_deref(),
            Some("1970-01-01T05:33:20+00:00")
        );
        assert_eq!(quota.queried_at, Some(10_000_000));
    }

    #[test]
    fn statusline_collector_rejects_expired_windows() {
        let path = temp_cache_path();
        let input = serde_json::to_vec(&json!({
            "session_id": "same-session",
            "rate_limits": {
                "five_hour": {
                    "used_percentage": 80.0,
                    "resets_at": 10_500
                }
            }
        }))
        .unwrap();
        ingest_statusline(Cursor::new(input), &path, 10_000).unwrap();

        let error = collect_statusline_quota_from_path_at(&path, 10_600).unwrap_err();
        assert!(error.contains("expired"));
    }

    #[test]
    fn bridge_rejects_invalid_percentages_without_replacing_cache() {
        let path = temp_cache_path();
        let valid = serde_json::to_vec(&json!({
            "session_id": "same-session",
            "rate_limits": {
                "five_hour": {
                    "used_percentage": 50.0,
                    "resets_at": 20_000
                }
            }
        }))
        .unwrap();
        ingest_statusline(Cursor::new(valid), &path, 10_000).unwrap();
        let before = fs::read(&path).unwrap();

        let invalid = serde_json::to_vec(&json!({
            "session_id": "same-session",
            "rate_limits": {
                "five_hour": {
                    "used_percentage": 101.0,
                    "resets_at": 21_000
                }
            }
        }))
        .unwrap();
        assert!(ingest_statusline(Cursor::new(invalid), &path, 11_000).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn desktop_v2_history_uses_only_the_latest_sanitized_sample() {
        let history_path = temp_history_path();
        config::write_json_file(
            &history_path,
            &json!({
                "version": 2,
                "samples": [
                    {
                        "t": 9_600_000,
                        "org": "private-organization-id",
                        "u": {"fh": 12.0, "sd": 34.0}
                    },
                    {
                        "t": 9_900_000,
                        "org": "private-organization-id",
                        "u": {"fh": 23.5, "sd": 45.5, "future_key": 99}
                    }
                ]
            }),
        )
        .unwrap();
        let missing_statusline = temp_cache_path();

        let quota = collect_local_quota_from_paths_at(
            Some(&history_path),
            &missing_statusline,
            &temp_latch_path(),
            10_000,
        )
        .unwrap();
        assert_eq!(quota.queried_at, Some(9_900_000));
        assert_eq!(quota.tiers.len(), 2);
        assert_eq!(quota.tiers[0].name, TIER_FIVE_HOUR);
        assert_eq!(quota.tiers[0].utilization, 23.5);
        assert_eq!(quota.tiers[0].resets_at, None);
        assert_eq!(quota.tiers[1].name, TIER_SEVEN_DAY);
        assert_eq!(quota.tiers[1].utilization, 45.5);
        let serialized = serde_json::to_string(&quota).unwrap();
        assert!(!serialized.contains("private-organization-id"));
        assert!(!serialized.contains("future_key"));
    }

    #[test]
    fn desktop_history_selects_each_windows_latest_sample_for_the_current_org() {
        let history_path = temp_history_path();
        config::write_json_file(
            &history_path,
            &json!({
                "version": 2,
                "samples": [
                    {
                        "t": 9_950_000,
                        "org": "older-org",
                        "u": {"fh": 99.0, "sd": 98.0}
                    },
                    {
                        "t": 9_900_000,
                        "org": "current-org",
                        "u": {"sd": 44.0}
                    },
                    {
                        "t": 9_980_000,
                        "org": "current-org",
                        "u": {"fh": 22.0}
                    }
                ]
            }),
        )
        .unwrap();

        let quota = collect_local_quota_from_paths_at(
            Some(&history_path),
            &temp_cache_path(),
            &temp_latch_path(),
            10_000,
        )
        .unwrap();
        assert_eq!(quota.queried_at, Some(9_900_000));
        assert_eq!(quota.tiers.len(), 2);
        assert_eq!(quota.tiers[0].name, TIER_FIVE_HOUR);
        assert_eq!(quota.tiers[0].utilization, 22.0);
        assert_eq!(quota.tiers[1].name, TIER_SEVEN_DAY);
        assert_eq!(quota.tiers[1].utilization, 44.0);
    }

    #[test]
    fn desktop_v1_history_supports_flat_window_fields() {
        let history_path = temp_history_path();
        config::write_json_file(
            &history_path,
            &json!({
                "version": 1,
                "samples": [{
                    "t": 9_900_000,
                    "org": "discard-me",
                    "fh": 7.0,
                    "sd": 8.0
                }]
            }),
        )
        .unwrap();

        let quota = collect_local_quota_from_paths_at(
            Some(&history_path),
            &temp_cache_path(),
            &temp_latch_path(),
            10_000,
        )
        .unwrap();
        assert_eq!(quota.tiers[0].utilization, 7.0);
        assert_eq!(quota.tiers[1].utilization, 8.0);
    }

    #[test]
    fn hybrid_collector_selects_the_latest_source_per_window() {
        let history_path = temp_history_path();
        config::write_json_file(
            &history_path,
            &json!({
                "version": 2,
                "samples": [{
                    "t": 11_000_000,
                    "org": "discard-me",
                    "u": {"fh": 25.0, "sd": 35.0}
                }]
            }),
        )
        .unwrap();
        let statusline_path = temp_cache_path();
        let cache = ClaudeStatuslineCache {
            schema_version: CACHE_SCHEMA_VERSION,
            updated_at: 11_050,
            sessions: vec![CachedStatuslineSession {
                updated_at: 11_050,
                event_at_ms: 11_050_000,
                session_key: statusline_session_key(Some("session")).unwrap(),
                claude_version: Some("2.1.220".to_string()),
                rate_limits: Some(CachedRateLimits {
                    five_hour: Some(CachedWindow {
                        used_percentage: 30.0,
                        resets_at: 12_000,
                        observed_at: 11_050,
                    }),
                    seven_day: Some(CachedWindow {
                        used_percentage: 20.0,
                        resets_at: 20_000,
                        observed_at: 10_900,
                    }),
                }),
            }],
        };
        config::write_json_file(&statusline_path, &cache).unwrap();

        let quota = collect_local_quota_from_paths_at(
            Some(&history_path),
            &statusline_path,
            &temp_latch_path(),
            11_100,
        )
        .unwrap();
        let five_hour = quota
            .tiers
            .iter()
            .find(|tier| tier.name == TIER_FIVE_HOUR)
            .unwrap();
        assert_eq!(five_hour.utilization, 30.0);
        assert_eq!(
            five_hour.resets_at.as_deref(),
            Some("1970-01-01T03:20:00+00:00")
        );
        let seven_day = quota
            .tiers
            .iter()
            .find(|tier| tier.name == TIER_SEVEN_DAY)
            .unwrap();
        assert_eq!(seven_day.utilization, 35.0);
        assert_eq!(seven_day.resets_at, None);
        assert_eq!(quota.queried_at, Some(11_000_000));
    }

    #[test]
    fn reset_latch_is_written_when_sources_agree_within_tolerance() {
        let (history_path, statusline_path, latch_path) = temp_quota_paths();
        write_desktop_history(&history_path, 10_000_000, Some(16.0), Some(64.0));
        write_statusline_cache(
            &statusline_path,
            10_020,
            Some((16.8, 20_000)),
            Some((64.0, 30_000)),
        );

        collect_local_quota_from_paths_at(
            Some(&history_path),
            &statusline_path,
            &latch_path,
            10_030,
        )
        .unwrap();

        let latch = read_reset_latch(&latch_path).unwrap();
        assert_eq!(latch.schema_version, RESET_LATCH_SCHEMA_VERSION);
        assert_eq!(
            latch.windows.five_hour,
            Some(reset_window(20_000, 10_030, 16.8))
        );
        // The weekly window supplied the identity evidence, so it latches too.
        assert_eq!(
            latch.windows.seven_day,
            Some(reset_window(30_000, 10_030, 64.0))
        );
        let stored: serde_json::Value =
            serde_json::from_slice(&fs::read(&latch_path).unwrap()).unwrap();
        assert_eq!(stored["schemaVersion"], json!(1));
        assert_eq!(stored["windows"]["five_hour"]["resetsAt"], json!(20_000));
        assert_eq!(stored["windows"]["seven_day"]["resetsAt"], json!(30_000));
    }

    #[test]
    fn a_lagging_desktop_sample_is_drift_not_a_different_account() {
        // Regression for a rule that looked right against synthetic data and
        // failed on the real thing: Desktop samples every 15 minutes, so its
        // five-hour figure trails a live status line by several points during
        // active use. Demanding near-equality there read honest lag as proof of
        // a second account, which not only refused to latch but wiped the
        // existing latch on every pass.
        let (history_path, statusline_path, latch_path) = temp_quota_paths();
        write_desktop_history(&history_path, 10_000_000, Some(35.0), Some(66.0));
        write_statusline_cache(
            &statusline_path,
            10_360,
            Some((41.0, 20_000)),
            Some((66.0, 30_000)),
        );

        collect_local_quota_from_paths_at(
            Some(&history_path),
            &statusline_path,
            &latch_path,
            10_400,
        )
        .unwrap();

        let latch = read_reset_latch(&latch_path).expect("six points of lag must still latch");
        assert_eq!(latch.windows.five_hour.map(|w| w.resets_at), Some(20_000));
    }

    #[test]
    fn a_five_hour_figure_that_went_backwards_still_vetoes() {
        // The fast window cannot confirm identity, but it can still rule one
        // out: an older reading above a newer one is impossible while a window
        // is climbing, so the weekly window's agreement must not override it.
        let (history_path, statusline_path, latch_path) = temp_quota_paths();
        write_desktop_history(&history_path, 10_000_000, Some(80.0), Some(66.0));
        write_statusline_cache(
            &statusline_path,
            10_360,
            Some((20.0, 20_000)),
            Some((66.0, 30_000)),
        );

        collect_local_quota_from_paths_at(
            Some(&history_path),
            &statusline_path,
            &latch_path,
            10_400,
        )
        .unwrap();

        assert!(
            !latch_path.exists(),
            "a backwards five-hour figure must veto the pass"
        );
    }

    #[test]
    fn a_disagreeing_window_clears_every_latch_not_just_its_own() {
        // Disagreement says the two sources are different accounts. That is a
        // fact about the pair, so every window's latch becomes untrustworthy —
        // including one this pass had no other reason to doubt.
        let (history_path, statusline_path, latch_path) = temp_quota_paths();
        write_desktop_history(&history_path, 10_050_000, Some(20.0), Some(64.0));
        write_statusline_cache(
            &statusline_path,
            10_000,
            Some((18.9, 20_000)),
            Some((30.0, 30_000)),
        );
        write_reset_latch(
            &latch_path,
            Some(reset_window(20_000, 9_000, 18.9)),
            Some(reset_window(30_000, 9_000, 40.0)),
        );

        let quota = collect_local_quota_from_paths_at(
            Some(&history_path),
            &statusline_path,
            &latch_path,
            10_100,
        )
        .unwrap();

        assert_eq!(find_tier(&quota, TIER_FIVE_HOUR).resets_at, None);
        assert!(
            !latch_path.exists(),
            "clearing both windows must delete the latch file"
        );
    }

    #[test]
    fn one_agreeing_window_cannot_launder_a_disagreement_on_the_other() {
        // Two unrelated accounts both sit at 0% early in a five-hour window, so
        // that window alone proves nothing. The weekly window disagreeing is
        // what settles it, and it must veto the whole pass.
        let (history_path, statusline_path, latch_path) = temp_quota_paths();
        write_desktop_history(&history_path, 10_050_000, Some(0.0), Some(64.0));
        write_statusline_cache(
            &statusline_path,
            10_000,
            Some((0.0, 20_000)),
            Some((30.0, 30_000)),
        );

        collect_local_quota_from_paths_at(
            Some(&history_path),
            &statusline_path,
            &latch_path,
            10_100,
        )
        .unwrap();

        assert!(
            !latch_path.exists(),
            "a contradicted pass must not latch the window that happened to match"
        );
    }

    #[test]
    fn reset_latch_is_untouched_without_a_desktop_window_observation() {
        let (history_path, statusline_path, latch_path) = temp_quota_paths();
        write_desktop_history(&history_path, 10_050_000, None, Some(40.0));
        write_statusline_cache(&statusline_path, 10_000, Some((10.0, 20_000)), None);
        let existing = reset_window(20_000, 9_000, 10.0);
        write_reset_latch(&latch_path, Some(existing.clone()), None);

        collect_local_quota_from_paths_at(
            Some(&history_path),
            &statusline_path,
            &latch_path,
            10_100,
        )
        .unwrap();

        let latch = read_reset_latch(&latch_path).unwrap();
        assert_eq!(latch.windows.five_hour, Some(existing));
    }

    #[test]
    fn future_reset_latch_is_attached_when_desktop_wins() {
        let (history_path, statusline_path, latch_path) = temp_quota_paths();
        write_desktop_history(&history_path, 10_050_000, Some(17.0), None);
        write_reset_latch(&latch_path, Some(reset_window(20_000, 9_000, 16.0)), None);

        let quota = collect_local_quota_from_paths_at(
            Some(&history_path),
            &statusline_path,
            &latch_path,
            10_100,
        )
        .unwrap();

        assert_eq!(
            find_tier(&quota, TIER_FIVE_HOUR).resets_at.as_deref(),
            Some("1970-01-01T05:33:20+00:00")
        );
    }

    #[test]
    fn reset_latch_is_dropped_when_desktop_utilization_rolls_over() {
        let (history_path, statusline_path, latch_path) = temp_quota_paths();
        write_desktop_history(&history_path, 10_050_000, Some(14.9), None);
        write_reset_latch(&latch_path, Some(reset_window(20_000, 9_000, 16.0)), None);

        let quota = collect_local_quota_from_paths_at(
            Some(&history_path),
            &statusline_path,
            &latch_path,
            10_100,
        )
        .unwrap();

        assert_eq!(find_tier(&quota, TIER_FIVE_HOUR).resets_at, None);
        assert!(!latch_path.exists());
    }

    #[test]
    fn expired_reset_latch_window_is_removed() {
        let (history_path, statusline_path, latch_path) = temp_quota_paths();
        write_desktop_history(&history_path, 9_950_000, Some(20.0), Some(40.0));
        let active_seven_day = reset_window(20_000, 9_000, 40.0);
        write_reset_latch(
            &latch_path,
            Some(reset_window(10_000, 9_000, 20.0)),
            Some(active_seven_day.clone()),
        );

        let quota = collect_local_quota_from_paths_at(
            Some(&history_path),
            &statusline_path,
            &latch_path,
            10_000,
        )
        .unwrap();

        assert_eq!(find_tier(&quota, TIER_FIVE_HOUR).resets_at, None);
        assert!(find_tier(&quota, TIER_SEVEN_DAY).resets_at.is_some());
        let latch = read_reset_latch(&latch_path).unwrap();
        assert!(latch.windows.five_hour.is_none());
        assert_eq!(latch.windows.seven_day, Some(active_seven_day));
    }

    #[test]
    fn reset_latch_file_is_deleted_when_its_last_window_expires() {
        let (history_path, statusline_path, latch_path) = temp_quota_paths();
        write_desktop_history(&history_path, 9_950_000, Some(20.0), None);
        write_reset_latch(&latch_path, Some(reset_window(10_000, 9_000, 20.0)), None);

        let quota = collect_local_quota_from_paths_at(
            Some(&history_path),
            &statusline_path,
            &latch_path,
            10_000,
        )
        .unwrap();

        assert_eq!(find_tier(&quota, TIER_FIVE_HOUR).resets_at, None);
        assert!(!latch_path.exists());
    }

    #[test]
    fn unusable_reset_latch_files_behave_as_no_latch() {
        let cases = [
            ("corrupt", b"not json".to_vec()),
            ("oversized", vec![b' '; MAX_RESET_LATCH_BYTES as usize + 1]),
            (
                "wrong-schema",
                serde_json::to_vec(&json!({
                    "schemaVersion": RESET_LATCH_SCHEMA_VERSION + 1,
                    "windows": {}
                }))
                .unwrap(),
            ),
            (
                "invalid-reset-time",
                serde_json::to_vec(&json!({
                    "schemaVersion": RESET_LATCH_SCHEMA_VERSION,
                    "windows": {
                        "five_hour": {
                            "resetsAt": 0,
                            "latchedAt": 9_000,
                            "utilizationAtLatch": 20.0
                        }
                    }
                }))
                .unwrap(),
            ),
            (
                "invalid-latched-time",
                serde_json::to_vec(&json!({
                    "schemaVersion": RESET_LATCH_SCHEMA_VERSION,
                    "windows": {
                        "five_hour": {
                            "resetsAt": 20_000,
                            "latchedAt": -1,
                            "utilizationAtLatch": 20.0
                        }
                    }
                }))
                .unwrap(),
            ),
            (
                "invalid-percentage",
                serde_json::to_vec(&json!({
                    "schemaVersion": RESET_LATCH_SCHEMA_VERSION,
                    "windows": {
                        "five_hour": {
                            "resetsAt": 20_000,
                            "latchedAt": 9_000,
                            "utilizationAtLatch": 101.0
                        }
                    }
                }))
                .unwrap(),
            ),
        ];

        for (case, contents) in cases {
            let (history_path, statusline_path, latch_path) = temp_quota_paths();
            write_desktop_history(&history_path, 9_950_000, Some(20.0), None);
            fs::create_dir_all(latch_path.parent().unwrap()).unwrap();
            fs::write(&latch_path, contents).unwrap();

            let quota = collect_local_quota_from_paths_at(
                Some(&history_path),
                &statusline_path,
                &latch_path,
                10_000,
            )
            .unwrap_or_else(|error| panic!("{case} latch blocked quota collection: {error}"));

            assert_eq!(find_tier(&quota, TIER_FIVE_HOUR).resets_at, None, "{case}");
        }
    }

    #[test]
    fn reset_latch_file_and_lock_are_created_with_private_permissions() {
        let (history_path, statusline_path, latch_path) = temp_quota_paths();
        write_desktop_history(&history_path, 10_000_000, Some(16.0), Some(64.0));
        write_statusline_cache(
            &statusline_path,
            10_020,
            Some((16.0, 20_000)),
            Some((64.0, 30_000)),
        );

        collect_local_quota_from_paths_at(
            Some(&history_path),
            &statusline_path,
            &latch_path,
            10_030,
        )
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                fs::metadata(&latch_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            let lock_path = latch_path.with_file_name(RESET_LATCH_LOCK_FILE_NAME);
            assert_eq!(
                fs::metadata(lock_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert!(!latch_path.with_file_name(CACHE_LOCK_FILE_NAME).exists());
        }
    }

    #[test]
    fn reset_latch_survives_statusline_then_desktop_source_switch() {
        let (history_path, statusline_path, latch_path) = temp_quota_paths();
        write_desktop_history(&history_path, 10_000_000, Some(16.0), Some(64.0));
        write_statusline_cache(
            &statusline_path,
            10_020,
            Some((16.4, 20_000)),
            Some((64.0, 30_000)),
        );

        let statusline_quota = collect_local_quota_from_paths_at(
            Some(&history_path),
            &statusline_path,
            &latch_path,
            10_030,
        )
        .unwrap();
        let reset = find_tier(&statusline_quota, TIER_FIVE_HOUR)
            .resets_at
            .clone();
        assert_eq!(reset.as_deref(), Some("1970-01-01T05:33:20+00:00"));
        assert!(read_reset_latch(&latch_path)
            .unwrap()
            .windows
            .five_hour
            .is_some());

        // The status-line sample is now stale, while Desktop has advanced and
        // supplies the selected percentage. The fixed reset remains attached.
        write_desktop_history(&history_path, 11_000_000, Some(18.0), Some(65.0));
        let desktop_quota = collect_local_quota_from_paths_at(
            Some(&history_path),
            &statusline_path,
            &latch_path,
            11_010,
        )
        .unwrap();

        let five_hour = find_tier(&desktop_quota, TIER_FIVE_HOUR);
        assert_eq!(five_hour.utilization, 18.0);
        assert_eq!(five_hour.resets_at, reset);
    }

    #[test]
    fn cache_read_errors_do_not_expose_the_local_path() {
        let path = temp_cache_path();
        fs::create_dir_all(&path).unwrap();

        let error = read_statusline_observations(&path, 10_000).unwrap_err();
        assert!(error.contains("failed to read Claude quota cache"));
        assert!(!error.contains(path.to_string_lossy().as_ref()));
    }

    #[test]
    fn cache_reads_are_bounded_before_json_parsing() {
        let path = temp_cache_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![b' '; MAX_STATUSLINE_CACHE_BYTES as usize + 1]).unwrap();

        let error = read_cache(&path).unwrap_err();
        assert!(matches!(
            error,
            CacheReadError::Invalid(message) if message.contains("exceeds 256 KiB")
        ));
    }

    #[test]
    fn invalid_desktop_percentage_is_never_used_as_quota() {
        let history_path = temp_history_path();
        config::write_json_file(
            &history_path,
            &json!({
                "version": 2,
                "samples": [{
                    "t": 9_900_000,
                    "u": {"fh": 101.0}
                }]
            }),
        )
        .unwrap();

        let error = collect_local_quota_from_paths_at(
            Some(&history_path),
            &temp_cache_path(),
            &temp_latch_path(),
            10_000,
        )
        .unwrap_err();
        assert!(error.contains("invalid Claude five_hour"));
    }
    #[test]
    fn plan_type_strips_the_namespace_and_keeps_what_follows() {
        assert_eq!(normalize_plan_type("claude_pro").as_deref(), Some("pro"));
        assert_eq!(normalize_plan_type("claude_max").as_deref(), Some("max"));
        // A tier this app has not seen must read as itself rather than be
        // dropped for failing to match a known list.
        assert_eq!(
            normalize_plan_type("claude_max_20x").as_deref(),
            Some("max 20x")
        );
        assert_eq!(
            normalize_plan_type("  CLAUDE_TEAM ").as_deref(),
            Some("team")
        );
        // Something outside the namespace is still a plan, just not ours to rename.
        assert_eq!(
            normalize_plan_type("enterprise").as_deref(),
            Some("enterprise")
        );
        assert_eq!(normalize_plan_type("claude_"), None);
        assert_eq!(normalize_plan_type("   "), None);
    }

    #[test]
    fn plan_type_reads_the_cli_account_profile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude.json");

        // Absent, unparseable, and shapes without the field all mean "unknown",
        // never a guess.
        assert_eq!(read_cli_plan_type_at(&path), None);
        fs::write(&path, b"not json").unwrap();
        assert_eq!(read_cli_plan_type_at(&path), None);
        fs::write(&path, br#"{"oauthAccount":{}}"#).unwrap();
        assert_eq!(read_cli_plan_type_at(&path), None);

        fs::write(
            &path,
            br#"{"oauthAccount":{"organizationType":"claude_pro","emailAddress":"a@b.c"}}"#,
        )
        .unwrap();
        assert_eq!(read_cli_plan_type_at(&path).as_deref(), Some("pro"));
    }

    #[test]
    fn plan_type_ignores_an_implausibly_large_account_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        let padding = " ".repeat(MAX_CLI_ACCOUNT_BYTES as usize + 1);
        fs::write(
            &path,
            format!(r#"{{"oauthAccount":{{"organizationType":"claude_pro"}}}}{padding}"#),
        )
        .unwrap();
        assert_eq!(read_cli_plan_type_at(&path), None);
    }
}
