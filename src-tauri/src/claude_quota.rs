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
const DESKTOP_HISTORY_FILE_NAME: &str = "plan-usage-history.json";
const MAX_STATUSLINE_INPUT_BYTES: u64 = 1_048_576;
const MAX_STATUSLINE_CACHE_BYTES: u64 = 256 * 1_024;
const MAX_DESKTOP_HISTORY_BYTES: u64 = 4 * 1_048_576;
const MAX_DESKTOP_HISTORY_SAMPLES: usize = 20_000;
const MAX_CACHED_STATUSLINE_SESSIONS: usize = 32;
const MAX_CLAUDE_VERSION_CHARS: usize = 64;
const MAX_CACHE_AGE_SECONDS: i64 = 15 * 60;
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
        now,
    )
}

fn statusline_cache_path() -> PathBuf {
    config::get_home_dir()
        .join(".llm-usage-bar")
        .join("runtime")
        .join(CACHE_FILE_NAME)
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
    let _cache_lock = acquire_statusline_cache_lock(cache_path)?;
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

struct StatuslineCacheLock(File);

impl Drop for StatuslineCacheLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

fn acquire_statusline_cache_lock(cache_path: &Path) -> Result<StatuslineCacheLock, AppError> {
    let parent = cache_path.parent().ok_or_else(|| {
        AppError::InvalidInput("Claude quota cache has no parent directory".to_string())
    })?;
    fs::create_dir_all(parent).map_err(|source| AppError::IoContext {
        context: "create Claude quota cache directory".to_string(),
        source,
    })?;
    let lock_path = cache_path.with_file_name(CACHE_LOCK_FILE_NAME);
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
    Ok(StatuslineCacheLock(file))
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
}

fn collect_local_quota_from_paths_at(
    desktop_path: Option<&Path>,
    statusline_path: &Path,
    now: i64,
) -> Result<SubscriptionQuota, String> {
    let desktop = desktop_path
        .map(|path| read_desktop_observations(path, now))
        .unwrap_or_else(|| Err("Claude Desktop local history is unavailable".to_string()));
    let statusline = read_statusline_observations(statusline_path, now);

    let mut tiers = Vec::new();
    // The card-level timestamp is conservative: every displayed window is at
    // least this fresh, even when their newest samples were observed at
    // different times.
    let mut observed_at_ms = None;
    for name in [TIER_FIVE_HOUR, TIER_SEVEN_DAY] {
        let desktop_tier = desktop
            .as_ref()
            .ok()
            .and_then(|values| values.iter().find(|value| value.tier.name == name))
            .cloned();
        let statusline_tier = statusline
            .as_ref()
            .ok()
            .and_then(|values| values.iter().find(|value| value.tier.name == name))
            .cloned();

        let selected = match (desktop_tier, statusline_tier) {
            (Some(desktop), Some(statusline))
                if desktop.observed_at_ms > statusline.observed_at_ms =>
            {
                // Claude Desktop and Claude Code do not expose a common
                // account identifier. Never splice a reset timestamp from one
                // source onto a percentage from the other.
                Some(desktop)
            }
            (Some(_), Some(statusline)) => Some(statusline),
            (Some(desktop), None) => Some(desktop),
            (None, Some(statusline)) => Some(statusline),
            (None, None) => None,
        };

        if let Some(selected) = selected {
            observed_at_ms = Some(
                observed_at_ms
                    .unwrap_or(i64::MAX)
                    .min(selected.observed_at_ms),
            );
            tiers.push(selected.tier);
        }
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
        plan_type: None,
        plan_renews_at: None,
        manual_reset_credits: None,
        extra_usage: None,
        error: None,
        queried_at: observed_at_ms,
    })
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
        plan_type: None,
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

        let quota =
            collect_local_quota_from_paths_at(Some(&history_path), &missing_statusline, 10_000)
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

        let quota =
            collect_local_quota_from_paths_at(Some(&history_path), &temp_cache_path(), 10_000)
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

        let quota =
            collect_local_quota_from_paths_at(Some(&history_path), &temp_cache_path(), 10_000)
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

        let quota =
            collect_local_quota_from_paths_at(Some(&history_path), &statusline_path, 11_100)
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

        let error =
            collect_local_quota_from_paths_at(Some(&history_path), &temp_cache_path(), 10_000)
                .unwrap_err();
        assert!(error.contains("invalid Claude five_hour"));
    }
}
