//! Versioned Unix-domain-socket bridge used by the native Swift preview.
//!
//! The bridge deliberately reuses the live `AppState`. Rust remains the sole
//! database/settings/Keychain owner and the sole scheduler while a preview is attached.

use crate::error::AppError;
use crate::store::AppState;
use crate::usage::domain::{
    BillingKind, CostSourceCounts, ProviderMonitoringDashboardView, ProviderUsageView,
    QuotaFetchState, QuotaStatusView, TokenSource, UsageTrendBucketView, UsageTrendGranularity,
};
use crate::usage::tray_snapshot::TrayUsageSnapshot;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener as StdUnixListener;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::watch;
use tokio::time::{timeout, Duration};

pub const PROTOCOL_VERSION: u32 = 1;
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;
pub const MUTATION_SCHEMA_VERSION: u32 = 1;
const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const SOCKET_FILE_NAME: &str = "native-bridge-v1.sock";
const SNAPSHOT_FILE_NAME: &str = "tray-snapshot-v1.json";

static STOP_SENDER: OnceLock<Mutex<Option<watch::Sender<bool>>>> = OnceLock::new();
static CLIENT_COUNT: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BridgeRequest {
    protocol_version: u32,
    id: String,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum HandoffDestination {
    Usage,
    Settings,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ShutdownParams {
    #[serde(default)]
    destination: Option<HandoffDestination>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DashboardRangeParams {
    start_at: i64,
    end_at: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UsageEventsParams {
    provider_id: String,
    start_at: i64,
    end_at: i64,
    page: u64,
    page_size: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetNativeSettingsParams {
    expected_revision: String,
    patch: Map<String, Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BridgeResponse {
    protocol_version: u32,
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<BridgeError>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BridgeError {
    code: &'static str,
    message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PersistedSnapshot<'a> {
    protocol_version: u32,
    schema_version: u32,
    written_at: i64,
    snapshot: &'a TrayUsageSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SchemaEnvelopeV1<T> {
    schema_version: u32,
    data: T,
}

impl<T> SchemaEnvelopeV1<T> {
    fn new(data: T) -> Self {
        Self {
            schema_version: 1,
            data,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum NativeApiBudgetModeV1 {
    Shared,
    PerProvider,
}

impl From<crate::settings::ApiBudgetMode> for NativeApiBudgetModeV1 {
    fn from(value: crate::settings::ApiBudgetMode) -> Self {
        match value {
            crate::settings::ApiBudgetMode::Shared => Self::Shared,
            crate::settings::ApiBudgetMode::PerProvider => Self::PerProvider,
        }
    }
}

impl From<NativeApiBudgetModeV1> for crate::settings::ApiBudgetMode {
    fn from(value: NativeApiBudgetModeV1) -> Self {
        match value {
            NativeApiBudgetModeV1::Shared => Self::Shared,
            NativeApiBudgetModeV1::PerProvider => Self::PerProvider,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeSettingsV1 {
    launch_on_startup: bool,
    silent_startup: bool,
    show_in_tray: bool,
    minimize_to_tray_on_close: bool,
    language: Option<String>,
    usage_warning_remaining_percent: u8,
    usage_critical_remaining_percent: u8,
    api_budget_mode: NativeApiBudgetModeV1,
    shared_api_daily_budget_usd: Option<String>,
    usage_dashboard_refresh_interval_ms: Option<u32>,
}

impl From<&crate::settings::AppSettings> for NativeSettingsV1 {
    fn from(value: &crate::settings::AppSettings) -> Self {
        Self {
            launch_on_startup: value.launch_on_startup,
            silent_startup: value.silent_startup,
            show_in_tray: value.show_in_tray,
            minimize_to_tray_on_close: value.minimize_to_tray_on_close,
            language: value.language.clone(),
            usage_warning_remaining_percent: value.usage_warning_remaining_percent,
            usage_critical_remaining_percent: value.usage_critical_remaining_percent,
            api_budget_mode: value.api_budget_mode.into(),
            shared_api_daily_budget_usd: value.shared_api_daily_budget_usd.clone(),
            usage_dashboard_refresh_interval_ms: value.usage_dashboard_refresh_interval_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeSettingsResponseV1 {
    #[serde(flatten)]
    envelope: SchemaEnvelopeV1<NativeSettingsV1>,
    revision: String,
}

impl NativeSettingsResponseV1 {
    fn from_settings(settings: &crate::settings::AppSettings) -> Self {
        let data = NativeSettingsV1::from(settings);
        let revision = native_settings_revision(&data);
        Self {
            envelope: SchemaEnvelopeV1::new(data),
            revision,
        }
    }
}

#[derive(Debug)]
struct NativeSettingsMutationError {
    code: &'static str,
    message: &'static str,
    data: Option<Value>,
}

impl NativeSettingsMutationError {
    fn invalid(key: &'static str) -> Self {
        Self {
            code: "invalid_setting",
            message: "Invalid native setting",
            data: Some(json!({"key": key})),
        }
    }

    fn unknown() -> Self {
        Self {
            code: "unknown_setting",
            message: "Unknown native setting",
            data: None,
        }
    }

    fn conflict(current: NativeSettingsResponseV1) -> Self {
        Self {
            code: "settings_conflict",
            message: "Native settings changed concurrently",
            data: serde_json::to_value(current).ok(),
        }
    }
}

impl From<AppError> for NativeSettingsMutationError {
    fn from(error: AppError) -> Self {
        log::warn!("native bridge settings mutation failed: {error}");
        Self {
            code: "settings_write_failed",
            message: "Could not save native settings",
            data: None,
        }
    }
}

fn native_settings_revision(settings: &NativeSettingsV1) -> String {
    let canonical = serde_json::to_vec(settings).expect("NativeSettingsV1 is always serializable");
    let digest = Sha256::digest(canonical);
    hex::encode(digest)[..32].to_string()
}

fn native_settings_response() -> NativeSettingsResponseV1 {
    NativeSettingsResponseV1::from_settings(&crate::settings::get_settings())
}

/// Writable settings that the tray projection is computed from, so a native
/// mutation knows when the published snapshot has to be rebuilt. Persisting
/// alone is not enough: the menu bar would keep serving the previous status
/// until some unrelated refresh happened to recompute it.
///
/// The budget pair drives the metered daily-budget card; the threshold pair
/// feeds `SubscriptionThresholds` in `usage::status`, which decides the
/// green/yellow/red classification for subscription windows.
fn tray_projection_inputs_changed(before: &NativeSettingsV1, after: &NativeSettingsV1) -> bool {
    before.api_budget_mode != after.api_budget_mode
        || before.shared_api_daily_budget_usd != after.shared_api_daily_budget_usd
        || before.usage_warning_remaining_percent != after.usage_warning_remaining_percent
        || before.usage_critical_remaining_percent != after.usage_critical_remaining_percent
}

fn set_native_settings(
    params: SetNativeSettingsParams,
) -> Result<NativeSettingsResponseV1, NativeSettingsMutationError> {
    set_native_settings_with_auto_launch(params, set_auto_launch_enabled)
}

fn set_native_settings_with_auto_launch<AutoLaunch>(
    params: SetNativeSettingsParams,
    auto_launch: AutoLaunch,
) -> Result<NativeSettingsResponseV1, NativeSettingsMutationError>
where
    AutoLaunch: Fn(bool) -> Result<(), AppError> + Copy,
{
    crate::settings::update_settings_checked_with_hooks(
        |existing| prepare_native_settings_update(existing, &params),
        |existing, next| {
            if existing.launch_on_startup == next.launch_on_startup {
                return Ok(());
            }
            if let Err(error) = auto_launch(next.launch_on_startup) {
                if let Err(rollback_error) = auto_launch(existing.launch_on_startup) {
                    log::error!(
                        "native bridge could not restore auto-launch after failure: {rollback_error}"
                    );
                }
                return Err(error.into());
            }
            Ok(())
        },
        |existing, next| {
            if existing.launch_on_startup != next.launch_on_startup {
                if let Err(error) = auto_launch(existing.launch_on_startup) {
                    log::error!("native bridge could not roll back auto-launch state: {error}");
                }
            }
        },
    )
}

#[cfg(test)]
fn set_native_settings_in_store_with_auto_launch<AutoLaunch>(
    store: &std::sync::RwLock<crate::settings::AppSettings>,
    path: &Path,
    params: SetNativeSettingsParams,
    auto_launch: AutoLaunch,
) -> Result<NativeSettingsResponseV1, NativeSettingsMutationError>
where
    AutoLaunch: Fn(bool) -> Result<(), AppError> + Copy,
{
    crate::settings::update_settings_in_store_with_hooks(
        store,
        path,
        |existing| prepare_native_settings_update(existing, &params),
        |existing, next| {
            if existing.launch_on_startup == next.launch_on_startup {
                return Ok(());
            }
            if let Err(error) = auto_launch(next.launch_on_startup) {
                let _ = auto_launch(existing.launch_on_startup);
                return Err(error.into());
            }
            Ok(())
        },
        |existing, next| {
            if existing.launch_on_startup != next.launch_on_startup {
                let _ = auto_launch(existing.launch_on_startup);
            }
        },
    )
}

fn set_auto_launch_enabled(enabled: bool) -> Result<(), AppError> {
    if enabled {
        crate::auto_launch::enable_auto_launch()
    } else {
        crate::auto_launch::disable_auto_launch()
    }
}

fn prepare_native_settings_update(
    existing: &crate::settings::AppSettings,
    params: &SetNativeSettingsParams,
) -> Result<(crate::settings::AppSettings, NativeSettingsResponseV1), NativeSettingsMutationError> {
    let current = NativeSettingsResponseV1::from_settings(existing);
    if params.expected_revision != current.revision {
        return Err(NativeSettingsMutationError::conflict(current));
    }

    let mut next = existing.clone();
    apply_native_settings_patch(&mut next, &params.patch)?;
    let response = NativeSettingsResponseV1::from_settings(&next);
    Ok((next, response))
}

fn apply_native_settings_patch(
    settings: &mut crate::settings::AppSettings,
    patch: &Map<String, Value>,
) -> Result<(), NativeSettingsMutationError> {
    const KNOWN_KEYS: &[&str] = &[
        "launchOnStartup",
        "silentStartup",
        "showInTray",
        "minimizeToTrayOnClose",
        "language",
        "usageWarningRemainingPercent",
        "usageCriticalRemainingPercent",
        "apiBudgetMode",
        "sharedApiDailyBudgetUsd",
        "usageDashboardRefreshIntervalMs",
    ];
    if patch.keys().any(|key| !KNOWN_KEYS.contains(&key.as_str())) {
        return Err(NativeSettingsMutationError::unknown());
    }

    if let Some(value) = patch.get("launchOnStartup") {
        settings.launch_on_startup = setting_bool(value, "launchOnStartup")?;
    }
    if let Some(value) = patch.get("silentStartup") {
        settings.silent_startup = setting_bool(value, "silentStartup")?;
    }
    if let Some(value) = patch.get("showInTray") {
        settings.show_in_tray = setting_bool(value, "showInTray")?;
    }
    if let Some(value) = patch.get("minimizeToTrayOnClose") {
        settings.minimize_to_tray_on_close = setting_bool(value, "minimizeToTrayOnClose")?;
    }
    if let Some(value) = patch.get("language") {
        settings.language = match value {
            Value::Null => None,
            Value::String(value) if matches!(value.as_str(), "en" | "zh" | "zh-TW" | "ja") => {
                Some(value.clone())
            }
            _ => return Err(NativeSettingsMutationError::invalid("language")),
        };
    }
    if let Some(value) = patch.get("usageWarningRemainingPercent") {
        settings.usage_warning_remaining_percent =
            setting_percent(value, "usageWarningRemainingPercent")?;
    }
    if let Some(value) = patch.get("usageCriticalRemainingPercent") {
        settings.usage_critical_remaining_percent =
            setting_percent(value, "usageCriticalRemainingPercent")?;
    }
    if settings.usage_warning_remaining_percent < settings.usage_critical_remaining_percent {
        let key = if patch.contains_key("usageWarningRemainingPercent") {
            "usageWarningRemainingPercent"
        } else {
            "usageCriticalRemainingPercent"
        };
        return Err(NativeSettingsMutationError::invalid(key));
    }
    if let Some(value) = patch.get("apiBudgetMode") {
        settings.api_budget_mode = match value.as_str() {
            Some("shared") => crate::settings::ApiBudgetMode::Shared,
            Some("perProvider") => crate::settings::ApiBudgetMode::PerProvider,
            _ => return Err(NativeSettingsMutationError::invalid("apiBudgetMode")),
        };
    }
    if let Some(value) = patch.get("sharedApiDailyBudgetUsd") {
        settings.shared_api_daily_budget_usd = match value {
            Value::Null => None,
            Value::String(value) => Some(validate_native_budget(value)?),
            _ => {
                return Err(NativeSettingsMutationError::invalid(
                    "sharedApiDailyBudgetUsd",
                ))
            }
        };
    }
    if let Some(value) = patch.get("usageDashboardRefreshIntervalMs") {
        settings.usage_dashboard_refresh_interval_ms = match value {
            Value::Null => None,
            Value::Number(value) => {
                let value = value.as_u64().and_then(|value| u32::try_from(value).ok());
                match value {
                    Some(0) => Some(0),
                    Some(value @ 5_000..=60_000) => Some(value),
                    _ => {
                        return Err(NativeSettingsMutationError::invalid(
                            "usageDashboardRefreshIntervalMs",
                        ));
                    }
                }
            }
            _ => {
                return Err(NativeSettingsMutationError::invalid(
                    "usageDashboardRefreshIntervalMs",
                ));
            }
        };
    }
    Ok(())
}

fn setting_bool(value: &Value, key: &'static str) -> Result<bool, NativeSettingsMutationError> {
    value
        .as_bool()
        .ok_or_else(|| NativeSettingsMutationError::invalid(key))
}

fn setting_percent(value: &Value, key: &'static str) -> Result<u8, NativeSettingsMutationError> {
    value
        .as_u64()
        .and_then(|value| u8::try_from(value).ok())
        .filter(|value| *value <= 100)
        .ok_or_else(|| NativeSettingsMutationError::invalid(key))
}

fn validate_native_budget(value: &str) -> Result<String, NativeSettingsMutationError> {
    let decimal = Decimal::from_str(value)
        .map_err(|_| NativeSettingsMutationError::invalid("sharedApiDailyBudgetUsd"))?;
    // A shared budget must be positive everywhere else in the app
    // (`canonicalize_daily_budget`, and the web form rejects non-positive input),
    // so the native path uses the same rule rather than inventing a second one.
    // "No budget" is already expressed by null, which makes zero redundant.
    if decimal <= Decimal::ZERO || decimal.scale() > 2 {
        return Err(NativeSettingsMutationError::invalid(
            "sharedApiDailyBudgetUsd",
        ));
    }
    Ok(decimal.normalize().to_string())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeProviderDashboardV1 {
    start_at: i64,
    end_at: i64,
    providers: Vec<NativeProviderUsageV1>,
    trend_granularity: UsageTrendGranularity,
    trend_buckets: Vec<UsageTrendBucketView>,
    warnings: Vec<String>,
}

impl From<ProviderMonitoringDashboardView> for NativeProviderDashboardV1 {
    fn from(value: ProviderMonitoringDashboardView) -> Self {
        Self {
            start_at: value.start_at,
            end_at: value.end_at,
            providers: value.providers.into_iter().map(Into::into).collect(),
            trend_granularity: value.trend_granularity,
            trend_buckets: value.trend_buckets,
            warnings: value.warnings,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeProviderUsageV1 {
    provider: NativeProviderSummaryV1,
    shared_account: bool,
    event_count: u64,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    total_cost_usd: Option<String>,
    cost_source_counts: CostSourceCounts,
    quota: Option<QuotaStatusView>,
    quota_fetch_state: Option<NativeQuotaFetchStateV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    token_sources: Vec<TokenSource>,
}

impl From<ProviderUsageView> for NativeProviderUsageV1 {
    fn from(value: ProviderUsageView) -> Self {
        Self {
            provider: NativeProviderSummaryV1 {
                id: value.provider.id,
                name: value.provider.name,
                billing_kind: value.provider.billing_kind,
                product_group_id: value.provider.product_group_id,
                enabled: value.provider.enabled,
                system_preset_key: value.provider.system_preset_key,
            },
            shared_account: value.shared_account,
            event_count: value.event_count,
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
            cache_read_tokens: value.cache_read_tokens,
            cache_creation_tokens: value.cache_creation_tokens,
            total_cost_usd: value.total_cost_usd,
            cost_source_counts: value.cost_source_counts,
            quota: value.quota,
            quota_fetch_state: value.quota_fetch_state.map(Into::into),
            token_sources: value.provider.token_sources,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeProviderSummaryV1 {
    id: String,
    name: String,
    billing_kind: BillingKind,
    product_group_id: String,
    enabled: bool,
    system_preset_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeQuotaFetchStateV1 {
    provider_id: String,
    last_attempt_at: Option<i64>,
    last_success_at: Option<i64>,
    last_error: Option<String>,
    consecutive_failures: u32,
    stale: bool,
}

impl From<QuotaFetchState> for NativeQuotaFetchStateV1 {
    fn from(value: QuotaFetchState) -> Self {
        Self {
            provider_id: value.provider_id,
            last_attempt_at: value.last_attempt_at,
            last_success_at: value.last_success_at,
            last_error: value
                .last_error
                .as_ref()
                .map(|_| "quota_refresh_failed".to_string()),
            consecutive_failures: value.consecutive_failures,
            stale: value.stale,
        }
    }
}

struct PreviewLease {
    app: AppHandle,
}

impl PreviewLease {
    fn acquire(app: AppHandle) -> Self {
        if CLIENT_COUNT.fetch_add(1, Ordering::AcqRel) == 0 {
            set_legacy_tray_visible(&app, false);
        }
        Self { app }
    }
}

impl Drop for PreviewLease {
    fn drop(&mut self) {
        if CLIENT_COUNT.fetch_sub(1, Ordering::AcqRel) != 1 {
            return;
        }
        if bridge_only() {
            let app = self.app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(5)).await;
                if bridge_only() && CLIENT_COUNT.load(Ordering::Acquire) == 0 {
                    log::info!("native bridge preview lease ended; exiting headless owner");
                    stop();
                    app.exit(0);
                }
            });
        } else {
            set_legacy_tray_visible(&self.app, true);
        }
    }
}

pub(crate) fn bridge_only() -> bool {
    std::env::var_os("LLM_USAGE_BAR_NATIVE_BRIDGE_ONLY").is_some()
}

pub(crate) fn isolated_test_mode() -> bool {
    bridge_only()
        && std::env::var_os("LLM_USAGE_BAR_NATIVE_BRIDGE_ISOLATED_TEST").is_some()
        && std::env::var("LLM_USAGE_BAR_TEST_HOME")
            .ok()
            .is_some_and(|home| !home.trim().is_empty())
}

fn set_legacy_tray_visible(app: &AppHandle, visible: bool) {
    if let Some(tray) = app.tray_by_id(crate::tray::TRAY_ID) {
        if tray.set_visible(visible).is_err() {
            log::warn!("native bridge could not update legacy tray visibility");
        }
    }
}

fn runtime_dir() -> PathBuf {
    // The bridge rendezvous is device-local even when the SQLite data directory
    // is overridden to iCloud, a NAS, or another shared location.
    crate::config::get_home_dir()
        .join(".llm-usage-bar")
        .join("runtime")
}

pub fn socket_path() -> PathBuf {
    runtime_dir().join(SOCKET_FILE_NAME)
}

pub fn snapshot_path() -> PathBuf {
    runtime_dir().join(SNAPSHOT_FILE_NAME)
}

fn ensure_private_runtime_dir(path: &Path) -> Result<(), AppError> {
    fs::create_dir_all(path).map_err(|error| AppError::io(path, error))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| AppError::io(path, error))
}

fn prepare_socket_path(path: &Path) -> Result<(), AppError> {
    if !path.exists() {
        return Ok(());
    }
    if std::os::unix::net::UnixStream::connect(path).is_ok() {
        return Err(AppError::Message(
            "native bridge socket is already active".to_string(),
        ));
    }
    fs::remove_file(path).map_err(|error| AppError::io(path, error))
}

pub fn start(app: AppHandle) -> Result<(), AppError> {
    let stop_slot = STOP_SENDER.get_or_init(|| Mutex::new(None));
    let mut stop_slot = stop_slot
        .lock()
        .map_err(|_| AppError::Lock("native bridge stop channel".to_string()))?;
    if stop_slot.is_some() {
        return Ok(());
    }

    let runtime_dir = runtime_dir();
    ensure_private_runtime_dir(&runtime_dir)?;
    let socket_path = socket_path();
    prepare_socket_path(&socket_path)?;
    let listener =
        StdUnixListener::bind(&socket_path).map_err(|error| AppError::io(&socket_path, error))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| AppError::io(&socket_path, error))?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))
        .map_err(|error| AppError::io(&socket_path, error))?;

    if bridge_only() {
        set_legacy_tray_visible(&app, false);
        crate::tray::apply_tray_policy(&app, false);
    }

    let (stop_sender, stop_receiver) = watch::channel(false);
    *stop_slot = Some(stop_sender);
    drop(stop_slot);

    tauri::async_runtime::spawn(async move {
        match UnixListener::from_std(listener) {
            Ok(listener) => run_server(app, listener, socket_path, stop_receiver).await,
            Err(error) => {
                log::error!("native bridge could not enter the async runtime: {error}");
                let _ = fs::remove_file(&socket_path);
            }
        }
    });
    Ok(())
}

pub fn stop() {
    let Some(slot) = STOP_SENDER.get() else {
        return;
    };
    let Ok(mut sender) = slot.lock() else {
        return;
    };
    if let Some(sender) = sender.take() {
        let _ = sender.send(true);
    }
}

async fn run_server(
    app: AppHandle,
    listener: UnixListener,
    socket_path: PathBuf,
    mut stop: watch::Receiver<bool>,
) {
    log::info!("native bridge listening at {}", socket_path.display());
    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break;
                }
            }
            incoming = listener.accept() => {
                match incoming {
                    Ok((stream, _)) if peer_is_current_user(&stream) => {
                        let client_app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) = serve_client(client_app, stream).await {
                                log::warn!("native bridge client disconnected: {error}");
                            }
                        });
                    }
                    Ok(_) => log::warn!("native bridge rejected a peer with a different uid"),
                    Err(error) => log::warn!("native bridge accept failed: {error}"),
                }
            }
        }
    }
    drop(listener);
    let _ = fs::remove_file(&socket_path);
}

fn peer_is_current_user(stream: &UnixStream) -> bool {
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    let result = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    result == 0 && uid == unsafe { libc::geteuid() }
}

async fn serve_client(app: AppHandle, stream: UnixStream) -> Result<(), std::io::Error> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let mut authenticated = false;
    let mut preview_lease: Option<PreviewLease> = None;

    loop {
        line.clear();
        let bytes = match timeout(
            REQUEST_TIMEOUT,
            (&mut reader)
                .take((MAX_REQUEST_BYTES + 1) as u64)
                .read_line(&mut line),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                write_error(
                    &mut writer,
                    String::new(),
                    "request_timeout",
                    "Request timed out",
                )
                .await?;
                break;
            }
        };
        if bytes == 0 {
            break;
        }
        if bytes > MAX_REQUEST_BYTES {
            write_error(
                &mut writer,
                String::new(),
                "request_too_large",
                "Request is too large",
            )
            .await?;
            break;
        }
        if !line.ends_with('\n') {
            write_error(
                &mut writer,
                String::new(),
                "incomplete_request",
                "Request was not newline terminated",
            )
            .await?;
            break;
        }

        let request = match serde_json::from_str::<BridgeRequest>(line.trim_end()) {
            Ok(request) => request,
            Err(_) => {
                write_error(
                    &mut writer,
                    String::new(),
                    "invalid_request",
                    "Invalid request",
                )
                .await?;
                continue;
            }
        };

        if request.protocol_version != PROTOCOL_VERSION {
            write_error(
                &mut writer,
                request.id,
                "protocol_mismatch",
                "Unsupported protocol version",
            )
            .await?;
            continue;
        }
        if method_takes_no_params(&request.method) && !params_are_empty(request.params.as_ref()) {
            write_error(
                &mut writer,
                request.id,
                "invalid_params",
                "This method takes no parameters",
            )
            .await?;
            continue;
        }
        if !authenticated && request.method != "hello" {
            write_error(
                &mut writer,
                request.id,
                "hello_required",
                "Call hello first",
            )
            .await?;
            continue;
        }

        let id = request.id;
        match request.method.as_str() {
            "hello" => {
                if !authenticated {
                    authenticated = true;
                    preview_lease = Some(PreviewLease::acquire(app.clone()));
                }
                write_result(&mut writer, id, hello_result()).await?;
            }
            "capabilities" => write_result(&mut writer, id, capabilities_result()).await?,
            "getTrayUsageSnapshot" => {
                let service = app.state::<AppState>().tray_usage_service.clone();
                let snapshot = service.cached_snapshot().await;
                write_serialized_result(&mut writer, id, &snapshot).await?;
            }
            "refreshTrayUsage" => {
                let service = app.state::<AppState>().tray_usage_service.clone();
                let publish_app = app.clone();
                let snapshot = service
                    .refresh(move |snapshot| {
                        crate::tray_status::publish_tray_usage(&publish_app, snapshot);
                        crate::usage_events::emit_dashboard_invalidated_only();
                    })
                    .await;
                write_serialized_result(&mut writer, id, &snapshot).await?;
            }
            "getRuntimeStatus" => {
                write_result(
                    &mut writer,
                    id,
                    json!({
                        "bridgeOnly": bridge_only(),
                        "clientCount": CLIENT_COUNT.load(Ordering::Acquire),
                        "databaseOwner": "rust",
                        "schedulerOwner": "rust"
                    }),
                )
                .await?;
            }
            "getNativeSettings" => {
                write_serialized_result(&mut writer, id, &native_settings_response()).await?;
            }
            "setNativeSettings" => {
                let params = match decode_params::<SetNativeSettingsParams>(request.params.as_ref())
                {
                    Ok(params) => params,
                    Err(()) => {
                        write_error(
                            &mut writer,
                            id,
                            "invalid_params",
                            "Invalid native settings mutation",
                        )
                        .await?;
                        continue;
                    }
                };
                let before = native_settings_response();
                match set_native_settings(params) {
                    Ok(response) => {
                        if tray_projection_inputs_changed(
                            &before.envelope.data,
                            &response.envelope.data,
                        ) {
                            let service = app.state::<AppState>().tray_usage_service.clone();
                            let publish_app = app.clone();
                            service
                                .rebuild_from_persisted(move |snapshot| {
                                    crate::tray_status::publish_tray_usage(&publish_app, snapshot);
                                })
                                .await;
                            crate::usage_events::emit_dashboard_invalidated_only();
                        }
                        write_serialized_result(&mut writer, id, &response).await?;
                    }
                    Err(error) => write_mutation_error(&mut writer, id, error).await?,
                }
            }
            "getProviderDashboard" => {
                let params = match decode_params::<DashboardRangeParams>(request.params.as_ref()) {
                    Ok(params) => params,
                    Err(()) => {
                        write_error(&mut writer, id, "invalid_params", "Invalid dashboard range")
                            .await?;
                        continue;
                    }
                };
                let state = app.state::<AppState>();
                let result = crate::commands::get_provider_usage_dashboard_test_hook(
                    &state,
                    params.start_at,
                    params.end_at,
                )
                .map(NativeProviderDashboardV1::from)
                .map(SchemaEnvelopeV1::new);
                write_app_result(&mut writer, id, result).await?;
            }
            "getProviderUsageActivity" => {
                let params = match decode_params::<DashboardRangeParams>(request.params.as_ref()) {
                    Ok(params) => params,
                    Err(()) => {
                        write_error(&mut writer, id, "invalid_params", "Invalid activity range")
                            .await?;
                        continue;
                    }
                };
                let state = app.state::<AppState>();
                let result = crate::commands::get_provider_usage_activity_test_hook(
                    &state,
                    params.start_at,
                    params.end_at,
                )
                .map(SchemaEnvelopeV1::new);
                write_app_result(&mut writer, id, result).await?;
            }
            "getModelDashboard" => {
                let params = match decode_params::<DashboardRangeParams>(request.params.as_ref()) {
                    Ok(params) => params,
                    Err(()) => {
                        write_error(&mut writer, id, "invalid_params", "Invalid dashboard range")
                            .await?;
                        continue;
                    }
                };
                let state = app.state::<AppState>();
                let result = crate::commands::get_model_usage_dashboard_test_hook(
                    &state,
                    params.start_at,
                    params.end_at,
                )
                .map(SchemaEnvelopeV1::new);
                write_app_result(&mut writer, id, result).await?;
            }
            "getAgentBreakdown" => {
                let params = match decode_params::<DashboardRangeParams>(request.params.as_ref()) {
                    Ok(params) => params,
                    Err(()) => {
                        write_error(&mut writer, id, "invalid_params", "Invalid dashboard range")
                            .await?;
                        continue;
                    }
                };
                let state = app.state::<AppState>();
                let result = crate::commands::get_agent_usage_breakdown_test_hook(
                    &state,
                    params.start_at,
                    params.end_at,
                )
                .map(SchemaEnvelopeV1::new);
                write_app_result(&mut writer, id, result).await?;
            }
            "getUsageEvents" => {
                let params = match decode_params::<UsageEventsParams>(request.params.as_ref()) {
                    Ok(params) => params,
                    Err(()) => {
                        write_error(&mut writer, id, "invalid_params", "Invalid event query")
                            .await?;
                        continue;
                    }
                };
                let state = app.state::<AppState>();
                let result = crate::commands::get_provider_usage_events_test_hook(
                    &state,
                    &params.provider_id,
                    params.start_at,
                    params.end_at,
                    params.page,
                    params.page_size,
                )
                .map(SchemaEnvelopeV1::new);
                write_app_result(&mut writer, id, result).await?;
            }
            "shutdown" => {
                let destination = match shutdown_destination(request.params.as_ref()) {
                    Ok(destination) => destination,
                    Err(()) => {
                        write_error(
                            &mut writer,
                            id,
                            "invalid_params",
                            "Invalid shutdown destination",
                        )
                        .await?;
                        continue;
                    }
                };
                write_result(&mut writer, id, json!({"accepted": true})).await?;
                let was_bridge_only = bridge_only();
                stop();
                if let Some(destination) = destination {
                    if was_bridge_only {
                        std::env::remove_var("LLM_USAGE_BAR_NATIVE_BRIDGE_ONLY");
                    }
                    if let Err(error) = crate::tray_popover::open_main_window(&app, destination) {
                        log::warn!("native bridge handoff failed to reveal legacy UI: {error}");
                    }
                } else if was_bridge_only {
                    app.exit(0);
                }
                break;
            }
            _ => write_error(&mut writer, id, "method_not_found", "Unknown method").await?,
        }
    }

    drop(preview_lease);
    Ok(())
}

fn method_takes_no_params(method: &str) -> bool {
    matches!(
        method,
        "hello"
            | "capabilities"
            | "getTrayUsageSnapshot"
            | "refreshTrayUsage"
            | "getRuntimeStatus"
            | "getNativeSettings"
    )
}

fn params_are_empty(params: Option<&Value>) -> bool {
    match params {
        None | Some(Value::Null) => true,
        Some(Value::Object(values)) => values.is_empty(),
        _ => false,
    }
}

fn decode_params<T: for<'de> Deserialize<'de>>(params: Option<&Value>) -> Result<T, ()> {
    serde_json::from_value(params.cloned().ok_or(())?).map_err(|_| ())
}

fn shutdown_destination(
    params: Option<&Value>,
) -> Result<Option<crate::tray_popover::MainWindowDestination>, ()> {
    if params_are_empty(params) {
        return Ok(None);
    }
    let params: ShutdownParams =
        serde_json::from_value(params.cloned().ok_or(())?).map_err(|_| ())?;
    Ok(params.destination.map(|destination| match destination {
        HandoffDestination::Usage => crate::tray_popover::MainWindowDestination::Usage {
            agent_module_id: None,
        },
        HandoffDestination::Settings => crate::tray_popover::MainWindowDestination::GeneralSettings,
    }))
}

fn hello_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "server": "llm-usage-bar-rust",
        "readOnly": false,
        "snapshotSchemaVersion": SNAPSHOT_SCHEMA_VERSION,
        "mutationSchemaVersion": MUTATION_SCHEMA_VERSION
    })
}

fn capabilities_result() -> Value {
    json!({
        "methods": [
            "hello",
            "capabilities",
            "getTrayUsageSnapshot",
            "refreshTrayUsage",
            "getRuntimeStatus",
            "getNativeSettings",
            "setNativeSettings",
            "getProviderDashboard",
            "getProviderUsageActivity",
            "getModelDashboard",
            "getAgentBreakdown",
            "getUsageEvents",
            "shutdown"
        ],
        "mutations": ["setNativeSettings"],
        "shutdownDestinations": ["usage", "settings"]
    })
}

async fn write_app_result<W, T>(
    writer: &mut W,
    id: String,
    result: Result<T, AppError>,
) -> Result<(), std::io::Error>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    match result {
        Ok(value) => write_serialized_result(writer, id, &value).await,
        Err(error) => {
            log::warn!("native bridge read query failed: {error}");
            write_error(writer, id, "query_failed", "Could not load usage data").await
        }
    }
}

async fn write_serialized_result<W, T>(
    writer: &mut W,
    id: String,
    value: &T,
) -> Result<(), std::io::Error>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    match serde_json::to_value(value) {
        Ok(value) => write_result(writer, id, value).await,
        Err(_) => write_error(writer, id, "internal_error", "Could not encode response").await,
    }
}

async fn write_result<W>(writer: &mut W, id: String, result: Value) -> Result<(), std::io::Error>
where
    W: AsyncWrite + Unpin,
{
    write_response(
        writer,
        BridgeResponse {
            protocol_version: PROTOCOL_VERSION,
            id,
            result: Some(result),
            error: None,
        },
    )
    .await
}

async fn write_error<W>(
    writer: &mut W,
    id: String,
    code: &'static str,
    message: &'static str,
) -> Result<(), std::io::Error>
where
    W: AsyncWrite + Unpin,
{
    write_response(
        writer,
        BridgeResponse {
            protocol_version: PROTOCOL_VERSION,
            id,
            result: None,
            error: Some(BridgeError {
                code,
                message,
                data: None,
            }),
        },
    )
    .await
}

async fn write_mutation_error<W>(
    writer: &mut W,
    id: String,
    error: NativeSettingsMutationError,
) -> Result<(), std::io::Error>
where
    W: AsyncWrite + Unpin,
{
    write_response(
        writer,
        BridgeResponse {
            protocol_version: PROTOCOL_VERSION,
            id,
            result: None,
            error: Some(BridgeError {
                code: error.code,
                message: error.message,
                data: error.data,
            }),
        },
    )
    .await
}

async fn write_response<W>(writer: &mut W, response: BridgeResponse) -> Result<(), std::io::Error>
where
    W: AsyncWrite + Unpin,
{
    let mut encoded = serde_json::to_vec(&response).map_err(std::io::Error::other)?;
    encoded.push(b'\n');
    writer.write_all(&encoded).await?;
    writer.flush().await
}

pub fn persist_tray_snapshot(snapshot: &TrayUsageSnapshot) -> Result<(), AppError> {
    let runtime_dir = runtime_dir();
    ensure_private_runtime_dir(&runtime_dir)?;
    let envelope = PersistedSnapshot {
        protocol_version: PROTOCOL_VERSION,
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        written_at: chrono::Utc::now().timestamp(),
        snapshot,
    };
    let data =
        serde_json::to_vec(&envelope).map_err(|source| AppError::JsonSerialize { source })?;
    let path = snapshot_path();
    crate::config::atomic_write(&path, &data)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .map_err(|error| AppError::io(&path, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{AppSettings, LocalMigrations, S3SyncSettings, WebDavSyncSettings};
    use std::sync::{Arc, Barrier, RwLock};

    fn settings_params(settings: &AppSettings, patch: Value) -> SetNativeSettingsParams {
        SetNativeSettingsParams {
            expected_revision: NativeSettingsResponseV1::from_settings(settings).revision,
            patch: patch.as_object().unwrap().clone(),
        }
    }

    fn recognizable_secret_settings() -> AppSettings {
        AppSettings {
            claude_config_dir: Some("/tmp/recognizable-auth-material".to_string()),
            current_provider_codex: Some("recognizable-provider-auth-id".to_string()),
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.secret.invalid".to_string(),
                username: "recognizable-webdav-user".to_string(),
                password: "RECOGNIZABLE_WEBDAV_PASSWORD".to_string(),
                ..WebDavSyncSettings::default()
            }),
            s3_sync: Some(S3SyncSettings {
                bucket: "recognizable-secret-bucket".to_string(),
                access_key_id: "RECOGNIZABLE_ACCESS_KEY_ID".to_string(),
                secret_access_key: "RECOGNIZABLE_S3_SECRET".to_string(),
                ..S3SyncSettings::default()
            }),
            webdav_backup: Some(json!({
                "authorization": "RECOGNIZABLE_LEGACY_BACKUP_SECRET"
            })),
            local_migrations: Some(LocalMigrations::default()),
            ..AppSettings::default()
        }
    }

    #[test]
    fn request_rejects_unknown_fields() {
        let request = r#"{"protocolVersion":1,"id":"1","method":"hello","extra":true}"#;
        assert!(serde_json::from_str::<BridgeRequest>(request).is_err());
    }

    #[test]
    fn capabilities_advertise_versioned_settings_mutation() {
        let capabilities = capabilities_result();
        assert_eq!(capabilities["mutations"], json!(["setNativeSettings"]));
        assert_eq!(hello_result()["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(hello_result()["readOnly"], false);
        assert_eq!(
            hello_result()["mutationSchemaVersion"],
            MUTATION_SCHEMA_VERSION
        );
    }

    #[test]
    fn native_settings_fixture_decodes_as_the_safe_rust_projection() {
        let fixture =
            include_str!("../../native/Tests/UsageCoreTests/Fixtures/native-settings-v1.json");
        let response: NativeSettingsResponseV1 = serde_json::from_str(fixture).unwrap();

        assert_eq!(response.envelope.schema_version, 1);
        assert_eq!(response.envelope.data.language.as_deref(), Some("zh-TW"));
        assert_eq!(
            response.envelope.data.api_budget_mode,
            NativeApiBudgetModeV1::PerProvider
        );
        assert_eq!(
            response
                .envelope
                .data
                .shared_api_daily_budget_usd
                .as_deref(),
            Some("12.50")
        );
        assert_eq!(
            response.revision,
            native_settings_revision(&response.envelope.data)
        );
    }

    #[test]
    fn native_settings_projection_and_bridge_response_never_leak_secrets() {
        let settings = recognizable_secret_settings();
        let projection = NativeSettingsV1::from(&settings);
        let projection_json = serde_json::to_string(&projection).unwrap();
        let response_json = serde_json::to_string(&BridgeResponse {
            protocol_version: PROTOCOL_VERSION,
            id: "secret-leak-test".to_string(),
            result: Some(
                serde_json::to_value(NativeSettingsResponseV1::from_settings(&settings)).unwrap(),
            ),
            error: None,
        })
        .unwrap();

        for forbidden in [
            "RECOGNIZABLE_WEBDAV_PASSWORD",
            "RECOGNIZABLE_ACCESS_KEY_ID",
            "RECOGNIZABLE_S3_SECRET",
            "RECOGNIZABLE_LEGACY_BACKUP_SECRET",
            "recognizable-auth-material",
            "recognizable-provider-auth-id",
            "webdavSync",
            "baseUrl",
            "username",
            "password",
            "s3Sync",
            "bucket",
            "accessKeyId",
            "secretAccessKey",
            "webdavBackup",
            "authorization",
            "claudeConfigDir",
            "currentProviderCodex",
        ] {
            assert!(
                !projection_json.contains(forbidden),
                "projection leaked {forbidden}"
            );
            assert!(
                !response_json.contains(forbidden),
                "response leaked {forbidden}"
            );
        }
    }

    #[test]
    fn native_settings_revision_is_stable_and_only_tracks_the_projection() {
        let settings = recognizable_secret_settings();
        let projection = NativeSettingsV1::from(&settings);
        let revision = native_settings_revision(&projection);
        assert_eq!(revision, native_settings_revision(&projection.clone()));
        assert_eq!(revision.len(), 32);
        assert!(revision
            .chars()
            .all(|character| character.is_ascii_hexdigit()));

        let mut secret_only_change = settings.clone();
        secret_only_change.webdav_sync.as_mut().unwrap().password =
            "A_DIFFERENT_SECRET".to_string();
        secret_only_change.local_migrations = None;
        assert_eq!(
            revision,
            native_settings_revision(&NativeSettingsV1::from(&secret_only_change))
        );

        let mut projected_change = settings;
        projected_change.silent_startup = !projected_change.silent_startup;
        assert_ne!(
            revision,
            native_settings_revision(&NativeSettingsV1::from(&projected_change))
        );
    }

    #[test]
    fn tray_projection_rebuild_covers_every_setting_the_snapshot_is_derived_from() {
        let baseline = NativeSettingsV1::from(&recognizable_secret_settings());

        // Both threshold fields feed `SubscriptionThresholds`, so a native write
        // that changes either one must republish the tray snapshot exactly like a
        // budget change does. Missing these persisted the value but left the menu
        // bar on its previous colour.
        let mut warning = baseline.clone();
        warning.usage_warning_remaining_percent = baseline.usage_warning_remaining_percent / 2;
        assert!(tray_projection_inputs_changed(&baseline, &warning));

        let mut critical = baseline.clone();
        critical.usage_critical_remaining_percent = baseline.usage_critical_remaining_percent / 2;
        assert!(tray_projection_inputs_changed(&baseline, &critical));

        let mut budget = baseline.clone();
        budget.shared_api_daily_budget_usd = Some("12.5".to_string());
        assert!(tray_projection_inputs_changed(&baseline, &budget));

        let mut mode = baseline.clone();
        mode.api_budget_mode = match baseline.api_budget_mode {
            NativeApiBudgetModeV1::Shared => NativeApiBudgetModeV1::PerProvider,
            NativeApiBudgetModeV1::PerProvider => NativeApiBudgetModeV1::Shared,
        };
        assert!(tray_projection_inputs_changed(&baseline, &mode));

        // Settings the tray projection does not read must not force a rebuild.
        let mut unrelated = baseline.clone();
        unrelated.minimize_to_tray_on_close = !unrelated.minimize_to_tray_on_close;
        unrelated.language = Some("ja".to_string());
        assert!(!tray_projection_inputs_changed(&baseline, &unrelated));
    }

    #[test]
    fn sparse_patch_preserves_every_unspecified_field_including_secrets() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let original = recognizable_secret_settings();
        fs::write(&path, serde_json::to_vec_pretty(&original).unwrap()).unwrap();
        let store = RwLock::new(original.clone());
        let params = settings_params(
            &original,
            json!({
                "silentStartup": true,
                "language": "ja",
                "sharedApiDailyBudgetUsd": "0.50"
            }),
        );
        set_native_settings_in_store_with_auto_launch(&store, &path, params, |_| Ok(())).unwrap();
        let updated = store.read().unwrap().clone();
        let persisted: AppSettings = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();

        let projected_keys = [
            "launchOnStartup",
            "silentStartup",
            "showInTray",
            "minimizeToTrayOnClose",
            "language",
            "usageWarningRemainingPercent",
            "usageCriticalRemainingPercent",
            "apiBudgetMode",
            "sharedApiDailyBudgetUsd",
            "usageDashboardRefreshIntervalMs",
        ];
        let original_value = serde_json::to_value(&original).unwrap();
        let updated_value = serde_json::to_value(&updated).unwrap();
        for (key, value) in original_value.as_object().unwrap() {
            if projected_keys.contains(&key.as_str()) {
                continue;
            }
            assert_eq!(
                serde_json::to_vec(value).unwrap(),
                serde_json::to_vec(&updated_value[key]).unwrap(),
                "sparse patch changed unspecified field {key}"
            );
            assert_eq!(
                serde_json::to_vec(value).unwrap(),
                serde_json::to_vec(&serde_json::to_value(&persisted).unwrap()[key]).unwrap(),
                "persisted sparse patch changed unspecified field {key}"
            );
        }
        assert_eq!(
            updated.webdav_sync.as_ref().unwrap().password,
            "RECOGNIZABLE_WEBDAV_PASSWORD"
        );
        assert_eq!(
            updated.s3_sync.as_ref().unwrap().secret_access_key,
            "RECOGNIZABLE_S3_SECRET"
        );
    }

    #[test]
    fn native_settings_patch_rejects_unknown_and_invalid_values() {
        let mut unknown_settings = AppSettings::default();
        let unknown_error = apply_native_settings_patch(
            &mut unknown_settings,
            json!({"futureSetting": true}).as_object().unwrap(),
        )
        .unwrap_err();
        assert_eq!(unknown_error.code, "unknown_setting");
        assert_eq!(unknown_error.data, None);

        let invalid_cases = [
            (
                json!({"launchOnStartup": "yes"}),
                "invalid_setting",
                "launchOnStartup",
            ),
            (
                json!({"silentStartup": "yes"}),
                "invalid_setting",
                "silentStartup",
            ),
            (
                json!({"showInTray": "yes"}),
                "invalid_setting",
                "showInTray",
            ),
            (
                json!({"minimizeToTrayOnClose": "yes"}),
                "invalid_setting",
                "minimizeToTrayOnClose",
            ),
            (json!({"language": "fr"}), "invalid_setting", "language"),
            (
                json!({"usageWarningRemainingPercent": 101}),
                "invalid_setting",
                "usageWarningRemainingPercent",
            ),
            (
                json!({"usageCriticalRemainingPercent": -1}),
                "invalid_setting",
                "usageCriticalRemainingPercent",
            ),
            (
                json!({"usageWarningRemainingPercent": 10}),
                "invalid_setting",
                "usageWarningRemainingPercent",
            ),
            (
                json!({"usageCriticalRemainingPercent": 80}),
                "invalid_setting",
                "usageCriticalRemainingPercent",
            ),
            (
                json!({"apiBudgetMode": "per_provider"}),
                "invalid_setting",
                "apiBudgetMode",
            ),
            (
                json!({"sharedApiDailyBudgetUsd": "-0.01"}),
                "invalid_setting",
                "sharedApiDailyBudgetUsd",
            ),
            (
                json!({"sharedApiDailyBudgetUsd": "1.001"}),
                "invalid_setting",
                "sharedApiDailyBudgetUsd",
            ),
            (
                json!({"sharedApiDailyBudgetUsd": 1.25}),
                "invalid_setting",
                "sharedApiDailyBudgetUsd",
            ),
            (
                json!({"usageDashboardRefreshIntervalMs": 4_999}),
                "invalid_setting",
                "usageDashboardRefreshIntervalMs",
            ),
            (
                json!({"usageDashboardRefreshIntervalMs": 60_001}),
                "invalid_setting",
                "usageDashboardRefreshIntervalMs",
            ),
        ];

        for (patch, expected_code, expected_key) in invalid_cases {
            let mut settings = AppSettings::default();
            let error =
                apply_native_settings_patch(&mut settings, patch.as_object().unwrap()).unwrap_err();
            assert_eq!(error.code, expected_code);
            assert_eq!(error.data.unwrap()["key"], expected_key);
        }
    }

    #[test]
    fn native_settings_patch_accepts_nullable_values_and_legacy_refresh_choices() {
        let mut settings = AppSettings {
            language: Some("en".to_string()),
            shared_api_daily_budget_usd: Some("1".to_string()),
            usage_dashboard_refresh_interval_ms: Some(30_000),
            ..AppSettings::default()
        };
        apply_native_settings_patch(
            &mut settings,
            json!({
                "language": null,
                "usageDashboardRefreshIntervalMs": 0
            })
            .as_object()
            .unwrap(),
        )
        .unwrap();

        assert_eq!(settings.language, None);
        assert_eq!(settings.usage_dashboard_refresh_interval_ms, Some(0));

        // Zero is not "no budget" — null is. Accepting it here would let the native
        // app store a value the web form and `set_api_budget_config` both reject.
        let zero_budget = apply_native_settings_patch(
            &mut settings.clone(),
            json!({"sharedApiDailyBudgetUsd": "0"}).as_object().unwrap(),
        )
        .unwrap_err();
        assert_eq!(zero_budget.code, "invalid_setting");
        assert_eq!(
            zero_budget.data,
            Some(json!({"key": "sharedApiDailyBudgetUsd"}))
        );

        apply_native_settings_patch(
            &mut settings,
            json!({
                "sharedApiDailyBudgetUsd": null,
                "usageDashboardRefreshIntervalMs": null
            })
            .as_object()
            .unwrap(),
        )
        .unwrap();
        assert_eq!(settings.shared_api_daily_budget_usd, None);
        assert_eq!(settings.usage_dashboard_refresh_interval_ms, None);
    }

    #[test]
    fn concurrent_settings_mutations_conflict_inside_the_settings_lock() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let initial = AppSettings::default();
        let expected_revision = NativeSettingsResponseV1::from_settings(&initial).revision;
        let store = Arc::new(RwLock::new(initial));
        let barrier = Arc::new(Barrier::new(3));

        let handles = [json!({"silentStartup": true}), json!({"showInTray": false})]
            .into_iter()
            .map(|patch| {
                let store = store.clone();
                let barrier = barrier.clone();
                let path = path.clone();
                let expected_revision = expected_revision.clone();
                std::thread::spawn(move || {
                    let params = SetNativeSettingsParams {
                        expected_revision,
                        patch: patch.as_object().unwrap().clone(),
                    };
                    barrier.wait();
                    set_native_settings_in_store_with_auto_launch(&store, &path, params, |_| Ok(()))
                })
            })
            .collect::<Vec<_>>();

        barrier.wait();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let conflict = results
            .into_iter()
            .find_map(Result::err)
            .expect("one concurrent mutation must conflict");
        assert_eq!(conflict.code, "settings_conflict");
        let current: NativeSettingsResponseV1 =
            serde_json::from_value(conflict.data.unwrap()).unwrap();
        assert_eq!(
            current.revision,
            NativeSettingsResponseV1::from_settings(&store.read().unwrap()).revision
        );
    }

    #[test]
    fn auto_launch_failure_persists_nothing() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let initial = recognizable_secret_settings();
        fs::write(&path, serde_json::to_vec_pretty(&initial).unwrap()).unwrap();
        let original_file = fs::read(&path).unwrap();
        let store = RwLock::new(initial.clone());
        let params = settings_params(&initial, json!({"launchOnStartup": true}));
        let system_state = AtomicBool::new(false);

        let error =
            set_native_settings_in_store_with_auto_launch(&store, &path, params, |enabled| {
                system_state.store(enabled, Ordering::SeqCst);
                if enabled {
                    Err(AppError::Message("auto-launch-test-failure".to_string()))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();

        assert_eq!(error.code, "settings_write_failed");
        assert!(!system_state.load(Ordering::SeqCst));
        assert!(!store.read().unwrap().launch_on_startup);
        assert_eq!(fs::read(&path).unwrap(), original_file);
    }

    #[test]
    fn shutdown_destination_is_typed_and_rejects_unknown_fields() {
        let settings = json!({"destination": "settings"});
        assert_eq!(
            shutdown_destination(Some(&settings)).unwrap(),
            Some(crate::tray_popover::MainWindowDestination::GeneralSettings)
        );
        assert!(
            shutdown_destination(Some(&json!({"destination": "settings", "token": "no"}))).is_err()
        );
    }

    #[test]
    fn dashboard_and_event_params_are_strict() {
        let range = json!({"startAt": 100, "endAt": 200});
        let decoded = decode_params::<DashboardRangeParams>(Some(&range)).unwrap();
        assert_eq!((decoded.start_at, decoded.end_at), (100, 200));
        assert!(decode_params::<DashboardRangeParams>(Some(
            &json!({"startAt": 100, "endAt": 200, "token": "no"})
        ))
        .is_err());

        let events = json!({
            "providerId": "provider-a",
            "startAt": 100,
            "endAt": 200,
            "page": 1,
            "pageSize": 50
        });
        let decoded = decode_params::<UsageEventsParams>(Some(&events)).unwrap();
        assert_eq!(decoded.provider_id, "provider-a");
        assert_eq!((decoded.page, decoded.page_size), (1, 50));
    }

    #[test]
    fn capabilities_include_versioned_dashboard_queries() {
        let capabilities = capabilities_result();
        let methods = capabilities["methods"].as_array().unwrap();
        for method in [
            "getProviderDashboard",
            "getProviderUsageActivity",
            "getModelDashboard",
            "getAgentBreakdown",
            "getUsageEvents",
        ] {
            assert!(methods.contains(&json!(method)));
        }
    }

    #[test]
    fn runtime_directory_permissions_are_private() {
        let directory = tempfile::tempdir().unwrap().path().join("runtime");
        ensure_private_runtime_dir(&directory).unwrap();
        let mode = fs::metadata(directory).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn persisted_snapshot_has_explicit_versions() {
        let snapshot = TrayUsageSnapshot::unknown(123);
        let envelope = PersistedSnapshot {
            protocol_version: PROTOCOL_VERSION,
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            written_at: 456,
            snapshot: &snapshot,
        };
        let value = serde_json::to_value(envelope).unwrap();
        assert_eq!(value["protocolVersion"], 1);
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["snapshot"]["generatedAt"], 123);
    }

    #[test]
    fn shared_contract_fixture_decodes_as_the_real_rust_dto() {
        let fixture =
            include_str!("../../native/Tests/UsageCoreTests/Fixtures/tray-usage-snapshot-v1.json");
        let snapshot: TrayUsageSnapshot = serde_json::from_str(fixture).unwrap();
        assert_eq!(snapshot.status, crate::usage::status::UsageStatus::Yellow);
        assert_eq!(snapshot.agents[0].providers.len(), 2);
        assert_eq!(
            snapshot.agents[0].providers[0]
                .subscription
                .as_ref()
                .unwrap()
                .windows[1]
                .remaining_percent,
            None
        );
    }

    #[test]
    fn dashboard_contract_fixture_decodes_as_real_rust_dtos() {
        let fixture =
            include_str!("../../native/Tests/UsageCoreTests/Fixtures/dashboard-contract-v1.json");
        let root: Value = serde_json::from_str(fixture).unwrap();
        let providers: SchemaEnvelopeV1<NativeProviderDashboardV1> =
            serde_json::from_value(root["providerDashboard"].clone()).unwrap();
        let models: SchemaEnvelopeV1<crate::usage::domain::ModelUsageDashboardView> =
            serde_json::from_value(root["modelDashboard"].clone()).unwrap();
        let agents: SchemaEnvelopeV1<crate::usage::domain::AgentUsageBreakdownView> =
            serde_json::from_value(root["agentBreakdown"].clone()).unwrap();
        let events: SchemaEnvelopeV1<crate::usage::domain::UsageEventPage> =
            serde_json::from_value(root["usageEvents"].clone()).unwrap();

        assert_eq!(providers.schema_version, 1);
        assert_eq!(providers.data.providers[0].provider.id, "codex-work");
        let serialized = serde_json::to_value(&providers).unwrap();
        let provider = &serialized["data"]["providers"][0]["provider"];
        assert!(provider.get("routeBaseUrl").is_none());
        assert!(provider.get("apiKeys").is_none());
        assert!(provider.get("bindings").is_none());
        assert!(provider.get("hasRouteCredentials").is_none());
        assert_eq!(
            providers.data.providers[0]
                .quota
                .as_ref()
                .unwrap()
                .seven_day_utilization_percent,
            None
        );
        assert_eq!(models.data.total_tokens, 2_600);
        assert_eq!(
            agents.data.agents[0].agent_module_id.as_deref(),
            Some("codex")
        );
        assert_eq!(events.data.items[0].total_cost_usd, None);
    }
}
