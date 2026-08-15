mod agent_paths;
pub mod api;
mod app_config;
mod app_store;
mod auto_launch;
mod claude_desktop_config;
mod claude_plugin;
mod claude_quota;
mod commands;
mod config;
mod error;
pub mod http_client;
mod init_status;
mod lightweight;
#[cfg(target_os = "linux")]
mod linux_fix;
#[cfg(target_os = "macos")]
mod macos_fix;
#[cfg(any(target_os = "macos", test))]
mod macos_material;
mod panic_hook;
pub mod product_identity;
mod prompt;
mod provider;
mod provider_defaults;
pub mod secrets;
mod store;
// router 用 pub 而不是 mod:T4/T5/T7 往里面放的 pub 入口在启动层接线
// 之前没有调用点,私有模块会触发 dead_code(clippy -D warnings 直接挂)。
mod app_state;
pub mod route;
mod services;
mod settings;

mod tray;
mod tray_popover;
pub mod tray_status;
pub mod usage;
mod usage_events;

pub use app_config::{AppType, MultiAppConfig};
pub use app_state::AppState;
pub use claude_quota::run_claude_statusline_bridge;
pub use commands::*;
pub use config::{get_claude_account_path, get_claude_settings_path, read_json_file};
pub use error::AppError;
pub use prompt::Prompt;
pub use provider::{Provider, ProviderMeta};
pub use settings::{update_settings, AppSettings};
pub use store::Database;
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
pub use usage::domain::{
    AgentModuleInput, AgentModuleView, AgentProviderBindingInput, AgentProviderBindingView,
    AgentProxyRouteSetup, AgentProxySetupInfo, ArchivedAgentUsageSummary, BillingKind,
    BindingCredentialStatus, CostSource, CostSourceCounts, InvalidUsageLinkSummary,
    ProductUsageView, ProviderUsageView, QuotaFetchState, QuotaSnapshot, QuotaStatusView,
    RouteBinding, SystemProviderKeyUsageView, TokenSource, UnassignedUsageDiagnostics,
    UnassignedUsageGroup, UsageDashboardView, UsageEvent, UsageEventLink, UsageEventPage,
    UsageProviderInput, UsageProviderView, UsageSourceBinding,
};

use std::path::Path;
#[cfg(debug_assertions)]
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(target_os = "macos")]
use tauri::image::Image;
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::Manager;
use tauri::RunEvent;
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

#[derive(Debug)]
struct PreparedDatabaseRuntime {
    outcome: store::DatabaseIdentityOutcome,
}

#[derive(Debug)]
enum DatabaseRuntimePreflight<'a> {
    Ready(ReadyDatabaseRuntime<'a>),
    TooNew { version: i32 },
}

#[derive(Debug, Clone, Copy)]
struct ReadyDatabaseRuntime<'a> {
    prepared: &'a PreparedDatabaseRuntime,
}

impl PreparedDatabaseRuntime {
    fn prepare(app_config_dir: &Path) -> Result<Self, AppError> {
        Self::prepare_with(app_config_dir, store::prepare_database_identity)
    }

    fn prepare_with<F>(app_config_dir: &Path, prepare: F) -> Result<Self, AppError>
    where
        F: FnOnce(&Path) -> Result<store::DatabaseIdentityOutcome, AppError>,
    {
        Ok(Self {
            outcome: prepare(app_config_dir)?,
        })
    }

    fn outcome(&self) -> &store::DatabaseIdentityOutcome {
        &self.outcome
    }

    fn database_path(&self) -> &Path {
        &self.outcome.database_path
    }

    fn database_exists(&self) -> bool {
        self.database_path().exists()
    }

    fn needs_json_migration(&self, json_path: &Path) -> bool {
        !self.database_exists() && json_path.exists()
    }

    fn preflight(&self) -> Result<DatabaseRuntimePreflight<'_>, AppError> {
        self.preflight_with(store::Database::stored_user_version_exceeds_supported)
    }

    fn preflight_with<F>(&self, preflight: F) -> Result<DatabaseRuntimePreflight<'_>, AppError>
    where
        F: FnOnce(&Path) -> Result<Option<i32>, AppError>,
    {
        match preflight(self.database_path())? {
            Some(version) => Ok(DatabaseRuntimePreflight::TooNew { version }),
            None => Ok(DatabaseRuntimePreflight::Ready(self.ready())),
        }
    }

    fn ready(&self) -> ReadyDatabaseRuntime<'_> {
        ReadyDatabaseRuntime { prepared: self }
    }
}

impl ReadyDatabaseRuntime<'_> {
    fn database_path(&self) -> &Path {
        self.prepared.database_path()
    }

    fn open(&self) -> Result<store::Database, AppError> {
        self.open_with(store::Database::init_at)
    }

    fn open_with<F>(&self, open: F) -> Result<store::Database, AppError>
    where
        F: FnOnce(&Path) -> Result<store::Database, AppError>,
    {
        open(self.database_path())
    }
}

/// Narrow integration-test view of the production database identity decision.
#[cfg(debug_assertions)]
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseIdentityTestReport {
    pub database_path: PathBuf,
    pub archived_prior_path: Option<PathBuf>,
    pub retained_prior_path: Option<PathBuf>,
    pub migrated: bool,
    pub durability_warning: Option<String>,
}

#[cfg(debug_assertions)]
impl From<&store::DatabaseIdentityOutcome> for DatabaseIdentityTestReport {
    fn from(outcome: &store::DatabaseIdentityOutcome) -> Self {
        Self {
            database_path: outcome.database_path.clone(),
            archived_prior_path: outcome.archived_prior_path.clone(),
            retained_prior_path: outcome.retained_prior_path.clone(),
            migrated: outcome.migrated,
            durability_warning: outcome.durability_warning.clone(),
        }
    }
}

#[cfg(debug_assertions)]
#[doc(hidden)]
pub struct PreparedDatabaseRuntimeTestResult {
    pub identity: DatabaseIdentityTestReport,
    pub database: Database,
}

#[cfg(debug_assertions)]
#[doc(hidden)]
pub fn prepare_database_runtime_test_hook(
    app_config_dir: &Path,
) -> Result<PreparedDatabaseRuntimeTestResult, AppError> {
    let runtime = PreparedDatabaseRuntime::prepare(app_config_dir)?;
    let identity = DatabaseIdentityTestReport::from(runtime.outcome());
    let ready = match runtime.preflight() {
        Ok(DatabaseRuntimePreflight::Ready(ready)) => ready,
        Ok(DatabaseRuntimePreflight::TooNew { version }) => {
            return Err(AppError::Database(format!(
                "数据库版本过新（v{version}），无法由测试启动 helper 打开"
            )))
        }
        Err(error) => {
            log::warn!("测试启动 helper 预检数据库版本失败，继续正常初始化流程: {error}");
            runtime.ready()
        }
    };
    let database = ready.open()?;
    Ok(PreparedDatabaseRuntimeTestResult { identity, database })
}

#[cfg(debug_assertions)]
#[doc(hidden)]
pub fn create_schema_v13_fixture_test_hook(path: &Path) -> Result<(), AppError> {
    // Reviewed for schema v23: the v21 -> v22 step only rewrites cost columns on
    // existing usage_events rows and drops/recreates the immutability trigger;
    // v22 -> v23 only rebuilds provider_model_pricing with nullable rates. Neither
    // changes the v13 baseline below. Reviewed again for schema v24: v23 -> v24
    // only adds the write-only usage_light_predictions calibration table. Reviewed
    // for schema v25: v24 -> v25 only adds provider_key_usage_snapshots. Reviewed
    // for schema v26: v25 -> v26 adds provider_api_keys and re-keys the snapshot
    // and credential-journal tables onto it — all tables introduced well after
    // v13, so the baseline below is untouched.
    // Reviewed for schema v27: v26 -> v27 只新增三张 router_* 表，与 v13 基线无关。
    // Reviewed for schema v28: v27 -> v28 只给 router_providers 加两个可空/带默认的列，与 v13 基线无关。
    if store::SCHEMA_VERSION != 28
        || product_identity::DATABASE_IDENTITY_SOURCE_SCHEMA_VERSION != 13
    {
        return Err(AppError::Database(
            "fixed schema-v13 fixture builder must be reviewed when schema versions change"
                .to_string(),
        ));
    }
    let conn = rusqlite::Connection::open(path)
        .map_err(|error| AppError::Database(format!("open fixed v13 fixture: {error}")))?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|error| AppError::Database(format!("configure fixed v13 fixture: {error}")))?;
    Database::create_tables_on_conn(&conn)?;
    conn.execute_batch(
        "CREATE TABLE mcp_servers (id TEXT);
         CREATE TABLE prompts (id TEXT);
         CREATE TABLE profiles (id TEXT);
         CREATE TABLE provider_health (id TEXT);
         CREATE TABLE skills (id TEXT);
         CREATE TABLE skill_repos (id TEXT);
         CREATE TABLE proxy_config (id TEXT);
         CREATE TABLE proxy_live_backup (id TEXT);
         CREATE TABLE stream_check_logs (id TEXT);",
    )
    .map_err(|error| AppError::Database(format!("create fixed v13 retired tables: {error}")))?;
    usage::migration::migrate_v12_to_v13(&conn)?;
    Database::set_user_version(
        &conn,
        product_identity::DATABASE_IDENTITY_SOURCE_SCHEMA_VERSION,
    )?;
    if Database::table_exists(&conn, "usage_sync_cursors")?
        || Database::table_exists(&conn, "session_log_sync_v13_archive")?
        || !Database::table_exists(&conn, "session_log_sync")?
    {
        return Err(AppError::Database(
            "fixed schema-v13 fixture unexpectedly contains v14 cursor structures".to_string(),
        ));
    }
    Ok(())
}

#[cfg(debug_assertions)]
#[doc(hidden)]
pub fn runtime_log_paths_test_hook(app_config_dir: &Path) -> (PathBuf, PathBuf) {
    (
        panic_hook::file_log_path_for(app_config_dir),
        panic_hook::crash_log_path_for(app_config_dir),
    )
}

fn log_database_identity_outcome(outcome: &store::DatabaseIdentityOutcome) {
    let archived = outcome
        .archived_prior_path
        .as_deref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "none".to_string());
    let retained = outcome
        .retained_prior_path
        .as_deref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "none".to_string());
    let warning = outcome.durability_warning.as_deref().unwrap_or("none");
    log::info!(
        "Database identity ready: path={}, migrated={}, archived={}, retained={}, durability_warning={}",
        outcome.database_path.display(),
        outcome.migrated,
        archived,
        retained,
        warning
    );
    if let Some(warning) = outcome.durability_warning.as_deref() {
        log::warn!("Database identity migration committed with durability warning: {warning}");
    }
}

#[cfg(target_os = "windows")]
fn set_windows_app_user_model_id(app: &tauri::AppHandle) {
    let app_id = app.config().identifier.clone();
    let wide_app_id: Vec<u16> = app_id.encode_utf16().chain(std::iter::once(0)).collect();

    let result = unsafe {
        windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(wide_app_id.as_ptr())
    };

    if result < 0 {
        log::warn!("设置 Windows AppUserModelID 失败: 0x{result:08X}");
    } else {
        log::debug!("Windows AppUserModelID 已设置为 {app_id}");
    }
}

fn redact_url_for_log(url_str: &str) -> String {
    match url::Url::parse(url_str) {
        Ok(url) => {
            let mut output = format!("{}://", url.scheme());
            if let Some(host) = url.host_str() {
                output.push_str(host);
            }
            output.push_str(url.path());

            let mut keys: Vec<String> = url.query_pairs().map(|(k, _)| k.to_string()).collect();
            keys.sort();
            keys.dedup();

            if !keys.is_empty() {
                output.push_str("?[keys:");
                output.push_str(&keys.join(","));
                output.push(']');
            }

            output
        }
        Err(_) => {
            let base = url_str.split('#').next().unwrap_or(url_str);
            match base.split_once('?') {
                Some((prefix, _)) => format!("{prefix}?[redacted]"),
                None => base.to_string(),
            }
        }
    }
}

/// 更新托盘菜单的Tauri命令
#[tauri::command]
async fn update_tray_menu(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    match tray::create_tray_menu(&app, state.inner()) {
        Ok(new_menu) => {
            if let Some(tray) = app.tray_by_id(tray::TRAY_ID) {
                tray.set_menu(Some(new_menu))
                    .map_err(|e| format!("更新托盘菜单失败: {e}"))?;
                return Ok(true);
            }
            Ok(false)
        }
        Err(err) => {
            log::error!("创建托盘菜单失败: {err}");
            Ok(false)
        }
    }
}

#[cfg(target_os = "macos")]
fn macos_tray_icon() -> Option<Image<'static>> {
    const ICON_BYTES: &[u8] = include_bytes!("../icons/tray/macos/statusbar_template_3x.png");

    match Image::from_bytes(ICON_BYTES) {
        Ok(icon) => Some(icon),
        Err(err) => {
            log::warn!("Failed to load macOS tray icon: {err}");
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowEventKind {
    FocusLost,
    CloseRequested,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowEventRoute {
    HidePopover,
    HandleMainClose,
    Ignore,
}

fn classify_window_event_route(label: &str, event: WindowEventKind) -> WindowEventRoute {
    if label == tray_popover::TRAY_POPOVER_LABEL {
        return match event {
            WindowEventKind::FocusLost | WindowEventKind::CloseRequested => {
                WindowEventRoute::HidePopover
            }
            WindowEventKind::Other => WindowEventRoute::Ignore,
        };
    }

    if label == "main" && event == WindowEventKind::CloseRequested {
        WindowEventRoute::HandleMainClose
    } else {
        WindowEventRoute::Ignore
    }
}

#[cfg(any(target_os = "macos", test))]
fn should_hide_minimized_main(dock_visible: bool, is_minimized: bool) -> bool {
    dock_visible && is_minimized
}

/// 事件路由：哪些窗口事件应触发主窗口最小化检查。
/// 只认 `main` 窗口的 `Focused(false)`：tao 0.35.3 / tauri 2.11.5 没有专门的
/// Minimized 事件，最小化一个处于 key 状态的窗口必然让它 resign key，因此失焦是
/// 伴随最小化的可靠信号；失焦的其它原因由 `handle_minimized_main_window` 里的
/// `is_minimized()` 判定过滤。抽成不带 cfg 的纯函数，测试里才能编译到。
#[cfg(any(target_os = "macos", test))]
fn classify_minimize_check_event(label: &str, event: &tauri::WindowEvent) -> bool {
    label == "main" && matches!(event, tauri::WindowEvent::Focused(false))
}

/// 主窗口最小化后的隐藏处理：读取 Dock 可见性并按判定结果隐藏窗口、调整托盘策略。
/// 事件回调与兜底定时器共用这段逻辑，判定语义与原来 300ms 轮询的循环体完全一致。
#[cfg(all(target_os = "macos", not(test)))]
fn handle_minimized_main_window(app: &tauri::AppHandle) {
    let dock_visible = crate::tray::is_macos_dock_visible();
    if !dock_visible {
        return;
    }
    let Some(main) = app.get_webview_window("main") else {
        return;
    };
    if should_hide_minimized_main(dock_visible, main.is_minimized().unwrap_or(false)) {
        let _ = main.hide();
        crate::tray::apply_tray_policy(app, false);
    }
}

/// 主窗口最小化监控（原 300ms 常驻轮询已改为事件驱动）。
///
/// 事件驱动部分挂在 builder 级 `on_window_event`（见 `run()` 中的订阅）：tao 0.35.3
/// 和 tauri 2.11.5 都没有专门的 Minimized/Occluded 窗口事件（`tauri::WindowEvent`
/// 枚举见 tauri-2.11.5 src/app.rs:111；tao 的 macOS 窗口代理只订阅了
/// windowShouldClose / windowDidResize / windowDidMove / windowDidBecomeKey /
/// windowDidResignKey 等通知，见 tao-0.35.3 src/platform_impl/macos/window_delegate.rs）。
/// 最小化一个处于 key 状态的窗口必然让它 resign key，因此 `Focused(false)` 是伴随
/// 最小化的可靠信号；订阅挂在 builder 级而非 setup 里的窗口实例上，是因为轻量模式
/// 会销毁并重建主窗口，builder 级监听对每次新建的窗口都生效。
///
/// 这里只保留一条 5 秒一次的慢速兜底检查，兜住事件覆盖不全的场景：窗口并非 key 时
/// 被最小化（例如通过 Dock 图标右键菜单最小化）不会产生 Focused 变化。兜底频率
/// 远低于原来的 300ms，常驻开销可忽略。
#[cfg(all(target_os = "macos", not(test)))]
fn start_main_window_visibility_monitor(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        // 睡醒/挂起恢复后不追补错过的 tick，而是推迟到下一个整周期：
        // 避免唤醒瞬间连发多次检查，也让唤醒节奏对 timer coalescing 更友好
        // （真正的 dispatch-timer leeway 见 HANDOFF §11.3 的说明）。
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await;
        loop {
            interval.tick().await;
            handle_minimized_main_window(&app);
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 设置 panic hook，在应用崩溃时记录日志到 <app_config_dir>/crash.log（默认 ~/.llm-usage-bar/crash.log）
    panic_hook::setup_panic_hook();

    let async_runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .on_thread_start(|| {
            // macOS:这个 runtime 上跑的全是后台工作（用量同步、事件驱动循环、
            // 5 秒最小化兜底），把 worker/blocking 线程降到 Utility QoS，让系统
            // 调度排到 E-core 并配合 App Nap。主线程（UI）不受影响。
            #[cfg(target_os = "macos")]
            unsafe {
                libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
            }
        })
        .build()
        .expect("failed to initialize async runtime");
    tauri::async_runtime::set(async_runtime.handle().clone());

    let mut builder = tauri::Builder::default();

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            log::info!("=== Single Instance Callback Triggered ===");
            log::debug!("Args count: {}", args.len());
            for (i, arg) in args.iter().enumerate() {
                log::debug!("  arg[{i}]: {}", redact_url_for_log(arg));
            }

            if crate::lightweight::is_lightweight_mode() {
                if let Err(e) = crate::lightweight::exit_lightweight_mode(app) {
                    log::error!("退出轻量模式重建窗口失败: {e}");
                }
            }

            // Show and focus window regardless.
            if let Err(error) = crate::tray_popover::reveal_main_window(app) {
                log::error!("Failed to reveal main window: {error}");
            }
        }));
    }

    let builder = builder
        // 注册 deep-link 插件（处理 macOS AppleEvent 和其他平台的深链接）
        .plugin(tauri_plugin_deep_link::init())
        // 拦截窗口关闭：根据设置决定是否最小化到托盘
        .on_window_event(|window, event| {
            // macOS：主窗口最小化后隐藏并切回托盘策略（事件驱动，替代原来的 300ms 轮询）。
            // 最小化会伴随 Focused(false)（tao 在 windowDidResignKey 时发出），回调里
            // 用 is_minimized() 确认真实状态，其它原因导致的失焦会被判定逻辑过滤掉。
            #[cfg(all(target_os = "macos", not(test)))]
            {
                if classify_minimize_check_event(window.label(), event) {
                    handle_minimized_main_window(window.app_handle());
                }
            }
            let event_kind = match event {
                tauri::WindowEvent::Focused(false) => WindowEventKind::FocusLost,
                tauri::WindowEvent::CloseRequested { .. } => WindowEventKind::CloseRequested,
                _ => WindowEventKind::Other,
            };
            match classify_window_event_route(window.label(), event_kind) {
                WindowEventRoute::HidePopover => {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                    }
                    if let Err(error) = window.hide() {
                        log::warn!("failed to hide tray popover window: {error}");
                    }
                    return;
                }
                WindowEventRoute::HandleMainClose => {}
                WindowEventRoute::Ignore => return,
            }

            let tauri::WindowEvent::CloseRequested { api, .. } = event else {
                return;
            };

            // 数据库版本过新的恢复模式下没有托盘可唤回，关闭即退出，避免应用隐身后台
            let in_db_recovery = crate::init_status::get_init_error()
                .map(|p| p.kind.as_deref() == Some("db_version_too_new"))
                .unwrap_or(false);
            if in_db_recovery {
                api.prevent_close();
                window.app_handle().exit(0);
                return;
            }

            let settings = crate::settings::get_settings();

            if settings.minimize_to_tray_on_close {
                api.prevent_close();
                let _ = window.hide();
                #[cfg(target_os = "windows")]
                {
                    let _ = window.set_skip_taskbar(true);
                }
                #[cfg(target_os = "macos")]
                {
                    tray::apply_tray_policy(window.app_handle(), false);
                }
            } else {
                api.prevent_close();
                window.app_handle().exit(0);
            }
        })
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(window_state_flags())
                .with_denylist(&[tray_popover::TRAY_POPOVER_LABEL])
                .build(),
        )
        .setup(|app| {
            let _ = rustls::crypto::ring::default_provider().install_default();

            // 预先刷新 Store 覆盖配置，确保后续路径读取正确（日志/数据库等）
            app_store::refresh_app_config_dir_override(app.handle());
            panic_hook::init_app_config_dir(crate::config::get_app_config_dir());
            #[cfg(target_os = "windows")]
            set_windows_app_user_model_id(app.handle());

            // 初始化日志（单文件输出到 <app_config_dir>/logs/llm-usage-bar.log）
            {
                use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

                let log_dir = panic_hook::get_log_dir();

                // 确保日志目录存在
                if let Err(e) = std::fs::create_dir_all(&log_dir) {
                    eprintln!("创建日志目录失败: {e}");
                }

                // 启动时删除旧日志文件，实现单文件覆盖效果
                let log_file_path = panic_hook::file_log_path_for(
                    log_dir.parent().unwrap_or_else(|| std::path::Path::new(".")),
                );
                let _ = std::fs::remove_file(&log_file_path);

                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        // 初始化为 Trace，允许后续通过 log::set_max_level() 动态调整级别
                        .level(log::LevelFilter::Trace)
                        .targets([
                            Target::new(TargetKind::Stdout),
                            Target::new(TargetKind::Folder {
                                path: log_dir,
                                file_name: Some(crate::product_identity::LOG_BASENAME.into()),
                            }),
                        ])
                        // 单文件模式：启动时删除旧文件，达到大小时轮转
                        // 注意：KeepSome(n) 内部会做 n-2 运算，n=1 会导致 usize 下溢
                        // KeepSome(2) 是最小安全值，表示不保留轮转文件
                        .rotation_strategy(RotationStrategy::KeepSome(2))
                        // 单文件大小限制 1GB
                        .max_file_size(1024 * 1024 * 1024)
                        .timezone_strategy(TimezoneStrategy::UseLocal)
                        .build(),
                )?;
            }

            // Keep the OS login item aligned with the persisted setting on every
            // manual launch. This also repairs the invalid `.app` LaunchAgent
            // written by auto-launch 0.6 before the executable-path fix.
            #[cfg(target_os = "macos")]
            {
                let launch_on_startup = crate::settings::get_settings().launch_on_startup;
                let result = if launch_on_startup {
                    crate::auto_launch::enable_auto_launch()
                } else {
                    crate::auto_launch::disable_auto_launch()
                };
                if let Err(error) = result {
                    log::warn!("Failed to reconcile launch-at-login state: {error}");
                }
            }

            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                macos_material::apply_native_material(
                    &window,
                    macos_material::NativeMaterialSurface::MainWindow,
                );
            }

            // 注入 AppHandle 给 usage_events，让无 AppHandle 持有的写日志路径
            // 也能向前端推送 `usage-log-recorded`。
            // 放在日志系统初始化之后，确保 init 的日志能正常输出。
            usage_events::init(app.handle().clone());

            // 初始化数据库
            let app_config_dir = crate::config::get_app_config_dir();
            // 文件名迁移必须先于版本预检或任何 Database open/write，并且本次启动只执行一次。
            let prepared_database = match PreparedDatabaseRuntime::prepare(&app_config_dir) {
                Ok(prepared) => prepared,
                Err(error) => {
                    let prospective_path =
                        crate::product_identity::current_database_path(&app_config_dir);
                    let error_message = error.to_string();
                    log::error!(
                        "Failed to prepare authoritative database identity at {}: {}",
                        prospective_path.display(),
                        error_message
                    );
                    show_database_identity_error_dialog(
                        app.handle(),
                        &prospective_path,
                        &error_message,
                    );
                    return Err(Box::new(error));
                }
            };
            log_database_identity_outcome(prepared_database.outcome());
            let json_path = app_config_dir.join("config.json");

            // 检查是否需要从 config.json 迁移到 SQLite
            // 如果需要迁移，先验证 config.json 是否可以加载（在创建数据库之前）
            // 这样如果加载失败用户选择退出，数据库文件还没被创建，下次可以正常重试
            let migration_config = if prepared_database.needs_json_migration(&json_path) {
                log::info!("检测到旧版配置文件，验证配置文件...");

                // 循环：支持用户重试加载配置文件
                loop {
                    match crate::app_config::MultiAppConfig::load() {
                        Ok(config) => {
                            log::info!("✓ 配置文件加载成功");
                            break Some(config);
                        }
                        Err(e) => {
                            log::error!("加载旧配置文件失败: {e}");
                            // 弹出系统对话框让用户选择
                            if !show_migration_error_dialog(app.handle(), &e.to_string()) {
                                // 用户选择退出（此时数据库还没创建，下次启动可以重试）
                                log::info!("用户选择退出程序");
                                std::process::exit(1);
                            }
                            // 用户选择重试，继续循环
                            log::info!("用户选择重试加载配置文件");
                        }
                    }
                }
            } else {
                None
            };

            // 现在创建数据库（包含 Schema 迁移）
            //
            // 说明：从 v3.8.* 升级的用户通常会走到这里的 SQLite schema 迁移，
            // 若迁移失败（数据库损坏/权限不足/user_version 过新等），需要给用户明确提示，
            // 否则表现可能只是“应用打不开/闪退”。
            //
            // 预检：数据库版本过新时，必须先于任何 schema 写操作（create_tables 内含
            // DROP/ALTER 等 DDL）进入恢复界面，避免旧应用对读不懂的更新版 DB 落写。
            let ready_database = match prepared_database.preflight() {
                Ok(DatabaseRuntimePreflight::TooNew { version }) => {
                    log::warn!("数据库版本过新（v{version}），引导用户在应用内升级应用");
                    crate::init_status::set_init_error(crate::init_status::InitErrorPayload {
                        path: prepared_database.database_path().display().to_string(),
                        error: format!(
                            "数据库版本过新（{version}），当前应用仅支持 {}，请升级应用后再尝试。",
                            crate::store::SCHEMA_VERSION
                        ),
                        kind: Some("db_version_too_new".to_string()),
                        db_version: Some(version),
                        supported_version: Some(crate::store::SCHEMA_VERSION),
                    });
                    // 主窗口默认 visible:false，恢复界面必须强制显示
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                    return Ok(());
                }
                Ok(DatabaseRuntimePreflight::Ready(ready)) => ready,
                Err(e) => {
                    log::warn!("预检数据库版本失败，继续正常初始化流程: {e}");
                    prepared_database.ready()
                }
            };

            let db = loop {
                match ready_database.open() {
                    Ok(db) => break Arc::new(db),
                    Err(e) => {
                        log::error!("Failed to init database: {e}");

                        if !show_database_init_error_dialog(
                            app.handle(),
                            ready_database.database_path(),
                            &e.to_string(),
                        ) {
                            log::info!("用户选择退出程序");
                            std::process::exit(1);
                        }

                        log::info!("用户选择重试初始化数据库");
                    }
                }
            };

            if let Err(error) = db.reconcile_system_providers() {
                log::error!("Failed to reconcile fixed system Providers: {error}");
                show_database_init_error_dialog(
                    app.handle(),
                    ready_database.database_path(),
                    &error.to_string(),
                );
                return Err(Box::new(error));
            }

            // 如果有预加载的配置，执行迁移
            if let Some(config) = migration_config {
                log::info!("开始执行数据迁移...");

                match db.migrate_from_json(&config) {
                    Ok(_) => {
                        log::info!("✓ 配置迁移成功");
                        // 标记迁移成功，供前端显示 Toast
                        crate::init_status::set_migration_success();
                        // 归档旧配置文件（重命名而非删除，便于用户恢复）
                        let archive_path = json_path.with_extension("json.migrated");
                        if let Err(e) = std::fs::rename(&json_path, &archive_path) {
                            log::warn!("归档旧配置文件失败: {e}");
                        } else {
                            log::info!("✓ 旧配置已归档为 config.json.migrated");
                        }
                    }
                    Err(e) => {
                        // 配置加载成功但迁移失败的情况极少（磁盘满等），仅记录日志
                        log::error!("配置迁移失败: {e}，将从现有配置导入");
                    }
                }
            }

            use crate::secrets::codex_oauth_auth::CodexOAuthManager;
            use tokio::sync::RwLock;

            let app_config_dir = crate::config::get_app_config_dir();
            let codex_oauth_manager =
                Arc::new(RwLock::new(CodexOAuthManager::new(app_config_dir)));
            let quota_service = Arc::new(usage::quota::QuotaService::production(
                db.clone(),
                codex_oauth_manager.clone(),
            ));
            let credential_store = crate::secrets::production_credential_store();
            let app_state = AppState::new_with_credential_store_and_quota_service(
                db,
                credential_store,
                quota_service,
            );

            // —— T10:本地 router 启动接线 ——
            // 决定 5②:先把监听端口开起来,再做其余初始化。绑定失败只记 log::error、
            // 不中止 setup——端口被占是常见情况(上次没退干净、别的软件占了),
            // 不能因此打不开界面(任务书 §1.4)。
            let router_port = crate::api::router::read_router_port(&app_state.db);
            let router_auth: Arc<dyn crate::route::server::UpstreamAuth> =
                Arc::new(crate::route::auth::RouterUpstreamAuth::new(
                    app_state.db.clone(),
                    app_state.binding_credential_service.clone(),
                ));
            let router_db = app_state.db.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) =
                    crate::route::server::start(router_db, router_port, router_auth).await
                {
                    log::error!("[ROUTER] 启动失败: {error}");
                }
            });

            // 启动时只读指针,绝不写(决定 34/36):发现指针不是自己写的只记缺口
            // 标记、不静默覆盖;真正写指针只有 enable_router_pointer 一条路,
            // 由用户显式点「启用」触发。
            let router_pointer_state = crate::route::pointer::inspect_pointer();
            if let Err(error) =
                crate::api::router::apply_pointer_gap_marker(&app_state.db, &router_pointer_state)
            {
                log::warn!("记录 router 指针缺口标记失败: {error}");
            }

            // 1.5. 自动导入 live 配置 + seed 官方预设供应商（Claude / Codex / Gemini）
            //
            // 先 import 后 seed 是有意为之：先把用户手动配置的 settings.json / auth.json / .env
            // 落成 "default" provider 设为 current，再追加官方预设（is_current=false）。
            // 这样用户切到官方预设时，回填机制会保护原 live 配置不丢失。
            //
            // 捕获首次运行快照：所有全新装用户都会看到欢迎弹窗介绍 LLM Usage Bar 的工作方式。
            // 读失败时默认不弹，宁可漏弹也不要因为故障打扰用户。
            let first_run_already_confirmed = crate::settings::get_settings()
                .first_run_notice_confirmed
                .unwrap_or(false);
            let fresh_install_at_startup =
                app_state.db.is_providers_empty().unwrap_or(false);

            match app_state.db.init_default_official_providers() {
                Ok(count) if count > 0 => {
                    log::info!("✓ Seeded {count} official provider(s)");
                }
                Ok(_) => {}
                Err(e) => log::warn!("✗ Failed to seed official providers: {e}"),
            }

            // 老用户 / 已确认的路径由 `fresh_install_at_startup` 自行拦截，这里不做写入。
            // 字段只由前端在用户点击"我知道了"时 save_settings 回写，语义是"用户显式确认过"。
            if !first_run_already_confirmed && fresh_install_at_startup {
                log::info!("✓ First-run welcome notice pending");
            }

            crate::services::budget_alert::ensure_permission(app.handle());

            // 迁移旧的 app_config_dir 配置到 Store
            if let Err(e) = app_store::migrate_app_config_dir_from_settings(app.handle()) {
                log::warn!("迁移 app_config_dir 失败: {e}");
            }

            // 启动阶段不再无条件保存,避免意外覆盖用户配置。

            // 注册 deep-link URL 处理器（使用正确的 DeepLinkExt API）
            log::info!("=== Registering deep-link URL handler ===");

            // Linux 和 Windows 调试模式需要显式注册
            #[cfg(any(target_os = "linux", all(debug_assertions, windows)))]
            {
                #[cfg(target_os = "linux")]
                {
                    // Use Tauri's path API to get correct path (includes app identifier)
                    // tauri-plugin-deep-link writes to: ~/.local/share/com.ccswitch.desktop/applications/cc-switch-handler.desktop
                    // Only register if .desktop file doesn't exist to avoid overwriting user customizations
                    let should_register = app
                        .path()
                        .data_dir()
                        .map(|d| !d.join("applications/cc-switch-handler.desktop").exists())
                        .unwrap_or(true);

                    if should_register {
                        if let Err(e) = app.deep_link().register_all() {
                            log::error!("✗ Failed to register deep link schemes: {}", e);
                        } else {
                            log::info!("✓ Deep link schemes registered (Linux)");
                        }
                    } else {
                        log::info!("⊘ Deep link handler already exists, skipping registration");
                    }
                }

                #[cfg(all(debug_assertions, windows))]
                {
                    if let Err(e) = app.deep_link().register_all() {
                        log::error!("✗ Failed to register deep link schemes: {}", e);
                    } else {
                        log::info!("✓ Deep link schemes registered (Windows debug)");
                    }
                }
            }

            // 注册 URL 处理回调（所有平台通用）
            app.deep_link().on_open_url({
                let app_handle = app.handle().clone();
                move |_event| {

                    if crate::lightweight::is_lightweight_mode() {
                        if let Err(e) = crate::lightweight::exit_lightweight_mode(&app_handle) {
                            log::error!("退出轻量模式重建窗口失败: {e}");
                        }
                    }

                }
            });
            log::info!("✓ Deep-link URL handler registered");

            // 创建动态托盘菜单
            let menu = tray::create_tray_menu(app.handle(), &app_state)?;

            // 构建托盘
            let mut tray_builder = TrayIconBuilder::with_id(tray::TRAY_ID)
                .tooltip(tray_status::tray_status_tooltip(
                    usage::status::UsageStatus::Unknown,
                ))
                .on_tray_icon_event(|tray, event| {
                    match event {
                        TrayIconEvent::Click {
                            rect,
                            button,
                            button_state,
                            ..
                        } => {
                            #[cfg(target_os = "macos")]
                            match crate::tray_popover::classify_tray_click(button, button_state) {
                                crate::tray_popover::TrayClickAction::TogglePopover => {
                                    if let Err(error) = crate::tray_popover::toggle(
                                        tray.app_handle(),
                                        rect,
                                    ) {
                                        log::warn!("failed to toggle tray popover: {error}");
                                    }
                                }
                                crate::tray_popover::TrayClickAction::HidePopover => {
                                    let _ = crate::tray_popover::hide(tray.app_handle());
                                }
                                crate::tray_popover::TrayClickAction::Ignore => {}
                            }

                            // `tray` is only read by the macOS popover branch
                            // above, so it joins the rest here rather than
                            // becoming an unused parameter off that platform.
                            #[cfg(not(target_os = "macos"))]
                            let _ = (tray, rect, button, button_state);
                        }
                        _ => log::debug!("unhandled event {event:?}"),
                    }
                })
                .menu(&menu)
                .on_menu_event(|app, event| {
                    tray::handle_tray_menu_event(app, &event.id.0);
                });

            #[cfg(target_os = "macos")]
            {
                tray_builder = tray_builder.show_menu_on_left_click(false);
            }

            #[cfg(not(target_os = "macos"))]
            {
                tray_builder = tray_builder.show_menu_on_left_click(true);
            }

            // macOS starts with the safe Unknown status color. The legacy
            // template image remains a decode fallback only.
            #[cfg(target_os = "macos")]
            {
                let unknown_icon = tray_status::decode_status_icon(
                    usage::status::UsageStatus::Unknown,
                );
                if unknown_icon.is_err() {
                    log::warn!("Failed to load bundled Unknown tray status icon");
                }
                if let Some((icon, is_template)) =
                    tray_status::select_initial_status_icon(unknown_icon, macos_tray_icon)
                {
                    tray_builder = tray_builder.icon(icon).icon_as_template(is_template);
                } else if let Some(icon) = app.default_window_icon() {
                    log::warn!("Falling back to default window icon for tray");
                    tray_builder = tray_builder.icon(icon.clone());
                } else {
                    log::warn!("Failed to load macOS tray icon for tray");
                }
            }

            #[cfg(not(target_os = "macos"))]
            {
                if let Some(icon) = app.default_window_icon() {
                    tray_builder = tray_builder.icon(icon.clone());
                } else {
                    log::warn!("Failed to get default window icon for tray");
                }
            }

            let _tray = tray_builder.build(app)?;
            crate::services::webdav_auto_sync::start_worker(
                app_state.db.clone(),
                app.handle().clone(),
            );
            crate::services::s3_auto_sync::start_worker(
                app_state.db.clone(),
                app.handle().clone(),
            );
            // 将同一个实例注入到全局状态，避免重复创建导致的不一致
            app.manage(app_state);
            app.manage(commands::CodexOAuthState(codex_oauth_manager));
            log::info!("✓ CodexOAuthManager initialized");

            // 初始化全局出站代理 HTTP 客户端
            {
                let db = &app.state::<AppState>().db;
                let proxy_url = db.get_global_proxy_url().ok().flatten();

                if let Err(e) = crate::http_client::init(proxy_url.as_deref()) {
                    log::error!(
                        "[GlobalProxy] [GP-005] Failed to initialize with saved config: {e}"
                    );

                    // 清除无效的代理配置
                    if proxy_url.is_some() {
                        log::warn!(
                            "[GlobalProxy] [GP-006] Clearing invalid proxy config from database"
                        );
                        if let Err(clear_err) = db.set_global_proxy_url(None) {
                            log::error!(
                                "[GlobalProxy] [GP-007] Failed to clear invalid config: {clear_err}"
                            );
                        }
                    }

                    // 使用直连模式重新初始化
                    if let Err(fallback_err) = crate::http_client::init(None) {
                        log::error!(
                            "[GlobalProxy] [GP-008] Failed to initialize direct connection: {fallback_err}"
                        );
                    }
                }
            }

            let quota_callback_app = app.handle().clone();
            let quota_after_cycle: usage::quota::QuotaCycleCallback = Arc::new(move |outcome| {
                let app_handle = quota_callback_app.clone();
                Box::pin(async move {
                    let service = {
                        let Some(state) = app_handle.try_state::<AppState>() else {
                            log::warn!("tray usage state is unavailable after quota cycle");
                            return;
                        };
                        state.tray_usage_service.clone()
                    };

                    let publish_app = app_handle.clone();
                    let publish = move |snapshot: &usage::tray_snapshot::TrayUsageSnapshot| {
                        tray_status::publish_tray_usage(&publish_app, snapshot);
                    };
                    match outcome {
                        usage::quota::QuotaSchedulerOutcome::Completed {
                            had_errors: false,
                            ..
                        } => {
                            service.rebuild_from_persisted(publish).await;
                        }
                        usage::quota::QuotaSchedulerOutcome::Completed {
                            had_errors: true, ..
                        }
                        | usage::quota::QuotaSchedulerOutcome::Failed => {
                            service
                                .mark_refresh_failed_at(chrono::Local::now().timestamp(), publish)
                                .await;
                        }
                    }
                    usage_events::emit_dashboard_invalidated_only();
                })
            });
            if !app
                .state::<AppState>()
                .start_quota_scheduler(quota_after_cycle)
            {
                log::warn!("quota scheduler was already started");
            }

            if !app
                .state::<AppState>()
                .start_official_pricing_scheduler()
            {
                log::warn!("official pricing scheduler was already started");
            }

            if !app
                .state::<AppState>()
                .start_provider_key_usage_scheduler()
            {
                log::warn!("provider key usage scheduler was already started");
            }

            let tray_publisher_app = app.handle().clone();
            let tray_publisher: services::tray_usage_scheduler::TraySnapshotPublisher =
                Arc::new(move |snapshot| {
                    tray_status::publish_tray_usage(&tray_publisher_app, snapshot);
                    usage_events::emit_dashboard_invalidated_only();
                });
            if !app
                .state::<AppState>()
                .start_midnight_scheduler(tray_publisher.clone())
            {
                log::warn!("tray usage midnight scheduler was already started");
            }

            let startup_tray_service = app.state::<AppState>().tray_usage_service.clone();
            tauri::async_runtime::spawn(async move {
                startup_tray_service
                    .rebuild_from_persisted(move |snapshot| tray_publisher(snapshot))
                    .await;
            });

            // 异常退出恢复 + 代理状态自动恢复
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = app_handle.state::<AppState>();

                // Reconcile protected binding generations before any proxy route
                // can be restored. Failures remain local and all lookups continue
                // to fail closed.
                if state
                    .binding_credential_service
                    .reconcile_startup_journals()
                    .await
                    .is_err()
                {
                    log::error!("protected credential startup reconciliation failed");
                }
                if state
                    .binding_credential_service
                    .initialize_startup_binding_keys()
                    .await
                    .is_err()
                {
                    log::error!("fixed API binding local credential initialization failed");
                }

                // Periodic backup check (on startup)
                if let Err(e) = state.db.periodic_backup_if_needed() {
                    log::warn!("Periodic backup failed on startup: {e}");
                }

                // Periodic maintenance timer: run once per day while the app is running
                let db_for_timer = state.db.clone();
                tauri::async_runtime::spawn(async move {
                    const PERIODIC_MAINTENANCE_INTERVAL_SECS: u64 = 24 * 60 * 60;
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                        PERIODIC_MAINTENANCE_INTERVAL_SECS,
                    ));
                    interval.tick().await; // skip immediate first tick (already checked above)
                    loop {
                        interval.tick().await;
                        if let Err(e) = db_for_timer.periodic_backup_if_needed() {
                            log::warn!("Periodic maintenance timer failed: {e}");
                        }
                    }
                });

                // Session log usage sync: 启动时全量同步一次，之后由文件监听事件驱动；
                // 15 分钟慢速兜底保证监听彻底失效时用量仍会更新（安全底线，见 usage/watcher）。
                let db_for_session_sync = state.db.clone();
                let provider_session_sync = state.session_usage_service.clone();
                let sync_schedule = Arc::new(std::sync::Mutex::new(
                    usage::watcher_state::WatcherSchedule::new(
                        usage::watcher::SYNC_MIN_INTERVAL_SECS,
                    ),
                ));
                let sync_wake = Arc::new(tokio::sync::Notify::new());
                usage::watcher::start_usage_watcher(Arc::clone(&sync_schedule), Arc::clone(&sync_wake));
                tauri::async_runtime::spawn(async move {
                    fn run_step<T>(name: &str, result: Result<T, crate::error::AppError>) {
                        if let Err(e) = result {
                            log::warn!("{name} failed: {e}");
                        }
                    }

                    let db = &db_for_session_sync;

                    // 首次同步
                    run_step(
                        "Usage cost startup backfill",
                        db.backfill_missing_usage_costs(),
                    );
                    run_step(
                        "Session usage initial sync",
                        provider_session_sync.sync_source("claude"),
                    );
                    run_step(
                        "Codex usage initial sync",
                        provider_session_sync.sync_source("codex"),
                    );
                    run_step(
                        "Gemini usage initial sync",
                        crate::services::session_usage_gemini::sync_gemini_usage(db),
                    );
                    run_step(
                        "OpenCode usage initial sync",
                        crate::services::session_usage_opencode::sync_opencode_usage(db),
                    );

                    // 单个源的一次同步，把结果映射成调度退避信号：
                    // Err(AppError) 与「Ok 但 errors 非空」（个别文件解析失败）都算失败——
                    // 游标按文件推进，坏文件下次同步自然会重试；退避只是避免坏文件把
                    // 事件驱动的同步重新拖成每 60 秒的全量扫描。
                    let mut sync_source = |source: usage::watcher_state::SourceId| {
                        let clean = match source {
                            usage::watcher_state::SourceId::Claude => provider_session_sync
                                .sync_source("claude")
                                .map(|r| r.errors.is_empty()),
                            usage::watcher_state::SourceId::Codex => provider_session_sync
                                .sync_source("codex")
                                .map(|r| r.errors.is_empty()),
                            usage::watcher_state::SourceId::Gemini => {
                                crate::services::session_usage_gemini::sync_gemini_usage(
                                    &db_for_session_sync,
                                )
                                .map(|r| r.errors.is_empty())
                            }
                            usage::watcher_state::SourceId::OpenCode => {
                                crate::services::session_usage_opencode::sync_opencode_usage(
                                    &db_for_session_sync,
                                )
                                .map(|r| r.errors.is_empty())
                            }
                        };
                        match clean {
                            Ok(true) => Ok(()),
                            Ok(false) => {
                                log::warn!("{source:?} usage sync: 部分文件解析失败，触发退避");
                                Err(())
                            }
                            Err(e) => {
                                log::warn!("{source:?} usage sync failed: {e}");
                                Err(())
                            }
                        }
                    };

                    // 启动时把四个源全部标脏，保证首轮该跑的源都跑。
                    // SYNC_MIN_INTERVAL_SECS=60 是「同一个源两次同步之间的最小间隔」，
                    // 不是轮询周期。
                    sync_schedule.lock().unwrap().mark_all_dirty();

                    let mut last_fallback_second = usage::watcher::unix_seconds_now();
                    loop {
                        // 有文件事件时被立刻唤醒；超时仅用于慢速兜底检查。
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_secs(usage::watcher::DRIVER_WAKE_TIMEOUT_SECS),
                            sync_wake.notified(),
                        )
                        .await;

                        if sync_schedule.lock().unwrap().is_shutdown() {
                            break;
                        }

                        let now = usage::watcher::unix_seconds_now();
                        usage::watcher::apply_fallback_if_due(
                            &sync_schedule,
                            now,
                            &mut last_fallback_second,
                        );
                        usage::watcher::drive_due_syncs(
                            &sync_schedule,
                            usage::watcher::unix_seconds_now,
                            &mut sync_source,
                        );
                    }
                });
            });

            // Linux: 禁用 WebKitGTK 硬件加速，防止 EGL 初始化失败导致白屏
            #[cfg(target_os = "linux")]
            {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.with_webview(|webview| {
                        use webkit2gtk::{WebViewExt, SettingsExt, HardwareAccelerationPolicy};
                        let wk_webview = webview.inner();
                        if let Some(settings) = WebViewExt::settings(&wk_webview) {
                            SettingsExt::set_hardware_acceleration_policy(&settings, HardwareAccelerationPolicy::Never);
                            log::info!("已禁用 WebKitGTK 硬件加速");
                        }
                    });
                }
            }

            // macOS 始终以菜单栏优先模式启动；Windows/Linux 保留原静默启动设置。
            if let Some(window) = app.get_webview_window("main") {
                #[cfg(target_os = "macos")]
                {
                    let _ = window.hide();
                    tray::apply_tray_policy(app.handle(), false);
                    log::info!("macOS 菜单栏模式：主窗口已隐藏");
                }

                #[cfg(not(target_os = "macos"))]
                {
                    let settings = crate::settings::get_settings();

                    // 在窗口首次显示前同步装饰状态，避免前端加载后再切换导致标题栏闪烁
                    // 仅 Linux 生效：解决 Wayland 下系统窗口按钮不可用的问题
                    #[cfg(target_os = "linux")]
                    let _ = window.set_decorations(!settings.use_app_window_controls);

                    if settings.silent_startup {
                        // 静默启动模式：保持窗口隐藏
                        let _ = window.hide();
                        #[cfg(target_os = "windows")]
                        let _ = window.set_skip_taskbar(true);
                        log::info!("静默启动模式：主窗口已隐藏");
                    } else {
                        // 正常启动模式：显示窗口
                        let _ = window.show();
                        log::info!("正常启动模式：主窗口已显示");

                        // Linux: 解决首次启动 UI 无响应问题（Tauri #10746 + wry #637）。
                        // 启动时 webview 未获取焦点 + surface 尺寸协商失败，导致点击无效。
                        // 这里做 set_focus + 伪 resize，等价于无视觉版本的"最大化-还原"。
                        #[cfg(target_os = "linux")]
                        {
                            linux_fix::nudge_main_window(window.clone());
                        }
                    }
                }
            }

            #[cfg(all(target_os = "macos", not(test)))]
            {
                start_main_window_visibility_monitor(app.handle().clone());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_dashboard_modules,
            commands::save_dashboard_module,
            commands::reorder_dashboard_modules,
            commands::set_dashboard_module_visibility,
            commands::delete_dashboard_module,
            commands::list_agent_provider_bindings,
            commands::save_agent_provider_binding,
            commands::delete_agent_provider_binding,
            commands::set_agent_provider_binding_api_key,
            commands::replace_agent_provider_binding_api_key,
            commands::clear_agent_provider_binding_api_key,
            commands::list_provider_api_keys,
            commands::create_provider_api_key,
            commands::rename_provider_api_key,
            commands::delete_provider_api_key,
            commands::set_system_provider_api_key,
            commands::replace_system_provider_api_key,
            commands::clear_system_provider_api_key,
            commands::test_system_provider_connection,
            commands::refresh_system_provider_key_usage,
            commands::list_system_provider_models,
            commands::reveal_agent_provider_local_key,
            commands::rotate_agent_provider_local_key,
            commands::get_unassigned_usage_diagnostics,
            commands::list_usage_providers,
            commands::get_claude_cli_auth_status,
            commands::start_claude_cli_login,
            commands::logout_claude_cli,
            commands::save_usage_provider,
            commands::delete_usage_provider,
            commands::set_usage_provider_enabled,
            commands::get_usage_dashboard,
            commands::get_usage_events,
            commands::get_provider_usage_dashboard,
            commands::get_model_usage_dashboard,
            commands::get_agent_usage_breakdown,
            commands::get_provider_usage_activity,
            commands::get_provider_usage_events,
            commands::refresh_provider_quota,
            commands::sync_provider_session_usage,
            commands::get_tray_usage_snapshot,
            commands::refresh_tray_usage,
            commands::get_api_budget_config,
            commands::set_api_budget_config,
            commands::set_provider_daily_budget,
            commands::hide_tray_popover,
            commands::open_main_from_tray,
            commands::take_pending_main_window_destination,
            commands::acknowledge_main_window_ready,
            commands::quit_from_tray,
            commands::get_claude_code_config_path,
            commands::get_config_dir,
            commands::open_config_folder,
            commands::pick_directory,
            commands::open_external,
            commands::get_init_error,
            commands::get_app_config_path,
            commands::open_app_config_folder,
            commands::get_settings,
            commands::save_settings,
            commands::restart_app,
            commands::install_update_and_restart,
            commands::check_for_updates,
            commands::is_portable_mode,
            commands::copy_text_to_clipboard,
            commands::apply_claude_plugin_config,
            // Claude MCP management
            // usage query
            // subscription quota
            commands::get_subscription_quota,
            commands::get_codex_oauth_quota,
            commands::get_coding_plan_quota,
            commands::get_balance,
            // New MCP via config.json (SSOT)
            // Unified MCP management
            // Prompt management
            // Profile management (项目配置方案)
            // model list fetch (OpenAI-compatible /v1/models)
            // ours: endpoint speed test + custom endpoint management
            // app_config_dir override via Store
            commands::get_app_config_dir_override,
            commands::set_app_config_dir_override,
            // provider sort order management
            // theirs: config import/export and dialogs
            commands::export_config_to_file,
            commands::import_config_from_file,
            commands::webdav_test_connection,
            commands::webdav_sync_upload,
            commands::webdav_sync_download,
            commands::webdav_sync_save_settings,
            commands::webdav_sync_fetch_remote_info,
            commands::s3_test_connection,
            commands::s3_sync_upload,
            commands::s3_sync_download,
            commands::s3_sync_save_settings,
            commands::s3_sync_fetch_remote_info,
            commands::save_file_dialog,
            commands::open_file_dialog,
            commands::create_db_backup,
            commands::list_db_backups,
            commands::restore_db_backup,
            commands::rename_db_backup,
            commands::delete_db_backup,
            // Deep link import
            update_tray_menu,
            // Environment variable management
            // Skill management (v3.10.0+ unified)
            // Skill management (legacy API compatibility)
            // Auto launch
            commands::set_auto_launch,
            commands::get_auto_launch_status,
            // Proxy server management
            // Global & Per-App Config
            // Proxy failover commands
            // Failover queue management
            // Usage statistics
            commands::get_usage_summary,
            commands::get_usage_summary_by_app,
            commands::get_usage_trends,
            commands::get_provider_stats,
            commands::get_model_stats,
            commands::get_request_logs,
            commands::get_request_detail,
            commands::get_model_pricing,
            commands::refresh_official_pricing,
            commands::get_official_pricing_last_refresh_at,
            commands::get_official_pricing_last_imported_count,
            commands::update_model_pricing,
            commands::delete_model_pricing,
            commands::get_provider_model_pricing,
            commands::update_provider_model_pricing,
            commands::delete_provider_model_pricing,
            commands::check_provider_limits,
            // Session usage sync
            commands::sync_session_usage,
            commands::get_usage_data_sources,
            // Stream health check
            // Session manager
            commands::get_tool_versions,
            commands::run_tool_lifecycle_action,
            commands::probe_tool_installations,
            // Provider terminal
            // Universal Provider management
            // OpenCode specific
            // OpenClaw specific
            // Hermes specific
            // Global upstream proxy
            // Window theme control
            commands::set_window_theme,
            // Generic managed auth commands
            commands::auth_start_login,
            commands::auth_poll_for_account,
            commands::auth_list_accounts,
            commands::auth_get_status,
            commands::auth_remove_account,
            commands::auth_set_default_account,
            commands::auth_logout,
            // Local router commands (T10)
            commands::list_router_providers,
            commands::upsert_router_provider,
            commands::delete_router_provider,
            commands::set_model_routes,
            commands::get_router_mode,
            commands::set_router_mode,
            commands::inspect_router_pointer,
            commands::enable_router_pointer,
            commands::recent_router_attempts,
            // Copilot OAuth commands (multi-account support)
            // OMO commands
            // Workspace files (OpenClaw)
            // Daily memory files (OpenClaw workspace)
            // lightweight mode (for testing or low-resource environments)
        ]);

    let app = builder
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    app.run(|app_handle, event| {
        // 处理退出请求（所有平台）
        if let RunEvent::ExitRequested { api, code, .. } = &event {
            match classify_exit_request(*code) {
                // code 为 None 表示运行时自动触发（如隐藏窗口的 WebView 被回收导致无存活窗口），
                // 此时应仅阻止退出、保持托盘后台运行。
                ExitRequestAction::StayInTray => {
                    log::info!("运行时触发退出请求（无存活窗口），阻止退出以保持托盘后台运行");
                    api.prevent_exit();
                    return;
                }
                // code 为 RESTART_EXIT_CODE：app.restart() / 自更新 relaunch 发起的重启。
                // 这条路径上 prevent_exit() 会被 Tauri 忽略，事件循环必定退出，随后由
                // Tauri 在 RunEvent::Exit 后用新二进制 re-exec（macOS 会按更新后的
                // Info.plist 解析可执行名）。
                //
                // 绝不能复用下面的异步清理任务：该任务在 tokio 线程调 save_window_state，
                // 持有 window-state 插件锁的同时向主线程查询窗口几何；而主线程此刻正在
                // 退出事件循环，并在插件自带的 RunEvent::Exit 钩子里等待同一把锁——双方
                // 互等造成进程永久卡死（更新已安装但应用冻结、不再重启，见 #3998）。
                //
                // 重启路径交还 Tauri 默认流程即可：
                //   - 窗口状态：插件 Exit 钩子在主线程保存（同线程读取窗口几何，无死锁）
                //   - 托盘图标：Tauri 内部 cleanup_before_exit 清理，正常走 Drop
                //   - 代理/Live 配置：无需恢复，重启后新实例立即接管并恢复代理状态
                //   - 100ms 落盘等待：重启前的 DB 写入均为命令驱动、此刻已完成，
                //     与所有 Tauri 应用默认重启路径的行为一致，无需额外等待
                ExitRequestAction::DeferToTauriRestart => {
                    log::info!("收到重启请求 (code={code:?})，交由 Tauri 默认重启流程 re-exec");
                    return;
                }
                // 其它 Some(_)：用户主动调用 app.exit() 退出（如托盘菜单"退出"），
                // 此时执行清理后退出。
                ExitRequestAction::CleanupAndExit => {}
            }

            // 停止文件监听与同步调度（驱动循环看到 is_shutdown 后退出，
            // 不留下永远跑不完的 spawn）；重启路径走 re-exec，无需显式清理。
            usage::watcher::stop_usage_watcher();

            log::info!("收到用户主动退出请求 (code={code:?})，开始清理...");
            api.prevent_exit();

            let app_handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                save_window_state_before_exit(&app_handle);
                cleanup_before_exit(&app_handle).await;
                // 先于 std::process::exit 显式移除托盘图标。
                // 进程直接退出时 Tauri 运行时不走正常 Drop 流程，
                // 不会向 Windows Shell 发送 NIM_DELETE，导致已退出的进程
                // 注册的图标仍残留在系统托盘（鼠标悬停 Shell 才会重绘发现进程已死）。
                remove_tray_icon_before_exit(&app_handle);
                log::info!("清理完成，退出应用");

                // 短暂等待确保所有 I/O 操作（如数据库写入）刷新到磁盘
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                // 使用 std::process::exit 避免再次触发 ExitRequested
                std::process::exit(0);
            });
            return;
        }

        #[cfg(target_os = "macos")]
        {
            // macOS 在 Dock 图标被点击并重新激活应用时会触发 Reopen 事件，这里手动恢复主窗口
            if let RunEvent::Reopen { .. } = event {
                if let Err(error) = tray_popover::open_main_window(
                    app_handle,
                    tray_popover::MainWindowDestination::Usage {
                        agent_module_id: None,
                    },
                ) {
                    log::error!("macOS reopen failed to reveal main window: {error}");
                }
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (app_handle, event);
        }
    });
}

// ============================================================
// 应用退出清理
// ============================================================

/// 应用退出前的清理工作
///
/// 在应用退出前检查代理服务器状态，如果正在运行则停止代理并恢复 Live 配置。
/// 确保 Claude Code/Codex/Gemini 的配置不会处于损坏状态。
/// 使用 stop_with_restore_keep_state 保留 settings 表中的代理状态，下次启动时自动恢复。
pub async fn cleanup_before_exit(app_handle: &tauri::AppHandle) {
    let cleanup_resources = app_handle.try_state::<app_state::AppState>().map(|state| {
        (
            state.take_quota_scheduler(),
            state.take_midnight_scheduler(),
            state.take_official_pricing_scheduler(),
            state.take_provider_key_usage_scheduler(),
        )
    });
    if let Some((
        quota_scheduler,
        midnight_scheduler,
        official_pricing_scheduler,
        provider_key_usage_scheduler,
    )) = cleanup_resources
    {
        if let Some(scheduler) = quota_scheduler {
            scheduler.stop().await;
        }
        if let Some(scheduler) = midnight_scheduler {
            scheduler.stop().await;
        }
        if let Some(scheduler) = official_pricing_scheduler {
            scheduler.stop().await;
        }
        if let Some(scheduler) = provider_key_usage_scheduler {
            scheduler.stop().await;
        }
    }
}

/// 主动从系统托盘移除托盘图标。
///
/// `std::process::exit` 会绕过 Tauri 运行时，触发不了 `TrayIcon::drop()`，
/// 也就不会向 Windows Shell 发 `NIM_DELETE`。结果是进程退出后托盘里
/// 仍保留一个死图标的缓存占位（Shell 不会主动重绘，需要鼠标悬停才刷新）。
///
/// 通过 `set_visible(false)` 走 `WM_USER_HIDE_TRAYICON` 消息路径，
/// 触发 tray-icon 内部的 `remove_tray_icon` → `Shell_NotifyIconW(NIM_DELETE)`，
/// 在进程结束前干净地把图标摘掉。其它平台 `set_visible(false)` 也是
/// 正常的隐藏/移除语义，作为跨平台兜底也安全。
pub(crate) fn remove_tray_icon_before_exit(app_handle: &tauri::AppHandle) {
    if let Some(tray) = app_handle.tray_by_id(tray::TRAY_ID) {
        if let Err(e) = tray.set_visible(false) {
            log::warn!("退出时移除托盘图标失败: {e}");
        } else {
            log::info!("已显式从系统托盘移除图标");
        }
    }
}

// ============================================================
// 启动时恢复代理状态
// ============================================================

// ============================================================
// 迁移错误对话框辅助函数
// ============================================================

/// 检测是否为中文环境
fn is_chinese_locale() -> bool {
    std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .map(|lang| lang.starts_with("zh"))
        .unwrap_or(false)
}

/// 显示迁移错误对话框
/// 返回 true 表示用户选择重试，false 表示用户选择退出
fn show_migration_error_dialog(app: &tauri::AppHandle, error: &str) -> bool {
    let title = if is_chinese_locale() {
        "配置迁移失败"
    } else {
        "Migration Failed"
    };

    let message = if is_chinese_locale() {
        format!(
            "从旧版本迁移配置时发生错误：\n\n{error}\n\n\
            您的数据尚未丢失，旧配置文件仍然保留。\n\
            建议回退到旧版本 LLM Usage Bar 以保护数据。\n\n\
            点击「重试」重新尝试迁移\n\
            点击「退出」关闭程序（可回退版本后重新打开）"
        )
    } else {
        format!(
            "An error occurred while migrating configuration:\n\n{error}\n\n\
            Your data is NOT lost - the old config file is still preserved.\n\
            Consider rolling back to an older LLM Usage Bar version.\n\n\
            Click 'Retry' to attempt migration again\n\
            Click 'Exit' to close the program"
        )
    };

    let retry_text = if is_chinese_locale() {
        "重试"
    } else {
        "Retry"
    };
    let exit_text = if is_chinese_locale() {
        "退出"
    } else {
        "Exit"
    };

    // 使用 blocking_show 同步等待用户响应
    // OkCancelCustom: 第一个按钮（重试）返回 true，第二个按钮（退出）返回 false
    app.dialog()
        .message(&message)
        .title(title)
        .kind(MessageDialogKind::Error)
        .buttons(MessageDialogButtons::OkCancelCustom(
            retry_text.to_string(),
            exit_text.to_string(),
        ))
        .blocking_show()
}

/// 数据库文件名迁移失败属于启动前的单次故障；显示明确错误后终止本次启动，
/// 不进入会重复执行数据库初始化的重试循环。
fn show_database_identity_error_dialog(
    app: &tauri::AppHandle,
    database_path: &std::path::Path,
    error: &str,
) {
    let title = if is_chinese_locale() {
        "数据库迁移失败"
    } else {
        "Database Migration Failed"
    };
    let message = if is_chinese_locale() {
        format!(
            "准备 LLM Usage Bar 数据库时发生错误：\n\n{error}\n\n\
             目标数据库路径：\n{path}\n\n\
             应用尚未进入常规数据库初始化。旧数据库及迁移证据会保留，\
             请检查磁盘空间、文件权限或迁移锁后重新启动。",
            path = database_path.display()
        )
    } else {
        format!(
            "LLM Usage Bar could not prepare its database:\n\n{error}\n\n\
             Target database path:\n{path}\n\n\
             The app has not entered normal database initialization. The prior database and \
             migration evidence are retained. Check disk space, file permissions, or the \
             migration lock, then restart the app.",
            path = database_path.display()
        )
    };

    let _ = app
        .dialog()
        .message(&message)
        .title(title)
        .kind(MessageDialogKind::Error)
        .buttons(MessageDialogButtons::Ok)
        .blocking_show();
}

/// 显示数据库初始化/Schema 迁移失败对话框
/// 返回 true 表示用户选择重试，false 表示用户选择退出
fn show_database_init_error_dialog(
    app: &tauri::AppHandle,
    db_path: &std::path::Path,
    error: &str,
) -> bool {
    let title = if is_chinese_locale() {
        "数据库初始化失败"
    } else {
        "Database Initialization Failed"
    };

    let message = if is_chinese_locale() {
        format!(
            "初始化数据库或迁移数据库结构时发生错误：\n\n{error}\n\n\
            数据库文件路径：\n{db}\n\n\
            您的数据尚未丢失，应用不会自动删除数据库文件。\n\
            常见原因包括：数据库版本过新、文件损坏、权限不足、磁盘空间不足等。\n\n\
            建议：\n\
            1) 先备份整个配置目录（包含 {database_file}）\n\
            2) 如果提示“数据库版本过新”，请升级到更新版本\n\
            3) 如果刚升级出现异常，可回退旧版本导出/备份后再升级\n\n\
            点击「重试」重新尝试初始化\n\
            点击「退出」关闭程序",
            db = db_path.display(),
            database_file = crate::product_identity::DATABASE_FILE,
        )
    } else {
        format!(
            "An error occurred while initializing or migrating the database:\n\n{error}\n\n\
            Database file path:\n{db}\n\n\
            Your data is NOT lost - the app will not delete the database automatically.\n\
            Common causes include: newer database version, corrupted file, permission issues, or low disk space.\n\n\
            Suggestions:\n\
            1) Back up the entire config directory (including {database_file})\n\
            2) If you see “database version is newer”, please upgrade {display_name}\n\
            3) If this happened right after upgrading, consider rolling back to export/backup then upgrade again\n\n\
            Click 'Retry' to attempt initialization again\n\
            Click 'Exit' to close the program",
            db = db_path.display(),
            database_file = crate::product_identity::DATABASE_FILE,
            display_name = crate::product_identity::DISPLAY_NAME,
        )
    };

    let retry_text = if is_chinese_locale() {
        "重试"
    } else {
        "Retry"
    };
    let exit_text = if is_chinese_locale() {
        "退出"
    } else {
        "Exit"
    };

    app.dialog()
        .message(&message)
        .title(title)
        .kind(MessageDialogKind::Error)
        .buttons(MessageDialogButtons::OkCancelCustom(
            retry_text.to_string(),
            exit_text.to_string(),
        ))
        .blocking_show()
}

// ============================================================
// 退出请求分类
// ============================================================

/// `RunEvent::ExitRequested` 的三类来源，处理方式必须区分。
///
/// 关键约束：重启请求（`code == RESTART_EXIT_CODE`）上 `prevent_exit()` 会被
/// Tauri 静默忽略（见 `ExitRequestApi::prevent_exit` 文档），事件循环必定继续
/// 退出并触发各插件的 `RunEvent::Exit` 钩子；任何与之并发的自定义清理任务都
/// 可能与插件退出钩子争用同一状态而死锁。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitRequestAction {
    /// `code` 为 `None`：运行时自动触发（如隐藏窗口的 WebView 被回收导致无存活
    /// 窗口），阻止退出、保持托盘后台运行。
    StayInTray,
    /// `code` 为 `RESTART_EXIT_CODE`：`app.restart()` / 自更新 relaunch 发起的
    /// 重启，不拦截、不做自定义清理，交还 Tauri 默认 re-exec 流程。
    DeferToTauriRestart,
    /// 其它 `Some(_)`：用户主动退出（托盘「退出」等），执行完整异步清理后结束进程。
    CleanupAndExit,
}

fn classify_exit_request(code: Option<i32>) -> ExitRequestAction {
    match code {
        None => ExitRequestAction::StayInTray,
        Some(tauri::RESTART_EXIT_CODE) => ExitRequestAction::DeferToTauriRestart,
        Some(_) => ExitRequestAction::CleanupAndExit,
    }
}

// ============================================================
// 在应用主动退出前显式持久化窗口状态
// ============================================================

fn window_state_flags() -> StateFlags {
    StateFlags::POSITION | StateFlags::SIZE | StateFlags::MAXIMIZED
}

/// 当前应用的退出路径会拦截 `ExitRequested` 并最终直接 `std::process::exit(0)`，
/// 这里需要在真正结束进程前手动落盘，避免 window-state 插件的默认退出钩子被绕过。
pub fn save_window_state_before_exit(app_handle: &tauri::AppHandle) {
    if let Err(err) = app_handle.save_window_state(window_state_flags()) {
        log::error!("退出前保存窗口状态失败: {err}");
    } else {
        log::info!("已在退出前保存窗口状态");
    }
}

/// 主动释放 single-instance 锁。
///
/// macOS single-instance 使用 `/tmp/{identifier}.sock`。我们有若干路径会直接
/// `std::process::exit(0)`，不会触发插件挂在 `RunEvent::Exit` 上的清理钩子。
/// 重启前主动 destroy 可以避免新进程误连旧 listener 后自行退出。
pub fn destroy_single_instance_lock(app_handle: &tauri::AppHandle) {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    tauri_plugin_single_instance::destroy(app_handle);
}

/// 清理托盘图标、释放 single-instance 锁后重启当前应用。
///
/// 直接走 `tauri::process::restart`（spawn 新进程 + `exit(0)`），不经过事件
/// 循环退出，因此 Tauri 内部的 `cleanup_before_exit` 和各插件的
/// `RunEvent::Exit` 钩子都不会执行。需要的清理由调用方与本函数显式补偿：
/// 窗口状态、代理/Live 恢复（调用方）；托盘图标、single-instance 锁（本函数）。
///
/// 有意不调 `AppHandle::cleanup_before_exit()`：它会在调用线程上 Drop 托盘
/// 图标，而 macOS 的 NSStatusItem 操作要求主线程；`set_visible(false)` 走
/// `run_item_main_thread` 代理，跨线程安全（见 `remove_tray_icon_before_exit`）。
pub fn restart_process(app_handle: &tauri::AppHandle) -> ! {
    remove_tray_icon_before_exit(app_handle);
    destroy_single_instance_lock(app_handle);
    tauri::process::restart(&app_handle.env());
}

#[cfg(test)]
mod tests {
    use super::{
        classify_exit_request, classify_minimize_check_event, classify_window_event_route,
        should_hide_minimized_main, DatabaseRuntimePreflight, ExitRequestAction,
        PreparedDatabaseRuntime, WindowEventKind, WindowEventRoute,
    };
    use crate::error::AppError;
    use crate::product_identity::DATABASE_FILE;
    use crate::store::DatabaseIdentityOutcome;
    use std::cell::{Cell, RefCell};
    use std::path::{Path, PathBuf};

    fn synthetic_identity(database_path: PathBuf) -> DatabaseIdentityOutcome {
        DatabaseIdentityOutcome {
            database_path,
            archived_prior_path: None,
            retained_prior_path: None,
            migrated: false,
            durability_warning: None,
        }
    }

    #[test]
    fn prepared_database_runtime_orders_prepare_preflight_open_once_on_one_path() {
        let temp = tempfile::tempdir().expect("create prepared runtime test dir");
        let app_dir = temp.path().join("app");
        std::fs::create_dir_all(&app_dir).expect("create prepared runtime app dir");
        let authoritative = app_dir.join(DATABASE_FILE);
        let prepare_calls = Cell::new(0usize);
        let events = RefCell::new(Vec::<(&'static str, PathBuf)>::new());

        let runtime = PreparedDatabaseRuntime::prepare_with(&app_dir, |observed_app_dir| {
            prepare_calls.set(prepare_calls.get() + 1);
            events
                .borrow_mut()
                .push(("prepare", observed_app_dir.to_path_buf()));
            Ok(synthetic_identity(authoritative.clone()))
        })
        .expect("prepare runtime capability");

        let ready = match runtime
            .preflight_with(|observed_path| {
                events
                    .borrow_mut()
                    .push(("preflight", observed_path.to_path_buf()));
                Ok(None)
            })
            .expect("preflight runtime capability")
        {
            DatabaseRuntimePreflight::Ready(ready) => ready,
            DatabaseRuntimePreflight::TooNew { version } => {
                panic!("unexpected future database v{version}")
            }
        };

        let database = ready
            .open_with(|observed_path| {
                events
                    .borrow_mut()
                    .push(("open", observed_path.to_path_buf()));
                crate::store::Database::init_at(observed_path)
            })
            .expect("open prepared database");

        assert_eq!(prepare_calls.get(), 1);
        assert_eq!(database.database_path(), Some(authoritative.as_path()));
        assert_eq!(
            events.into_inner(),
            vec![
                ("prepare", app_dir),
                ("preflight", authoritative.clone()),
                ("open", authoritative),
            ]
        );
    }

    #[test]
    fn prepared_database_runtime_prepare_error_stops_before_preflight_or_open() {
        let events = RefCell::new(Vec::<&'static str>::new());

        let error = PreparedDatabaseRuntime::prepare_with(Path::new("unused"), |_| {
            events.borrow_mut().push("prepare");
            Err(AppError::Config("injected prepare failure".to_string()))
        })
        .expect_err("prepare failure must stop startup capability creation");

        assert!(error.to_string().contains("injected prepare failure"));
        assert_eq!(events.into_inner(), vec!["prepare"]);
    }

    #[test]
    fn prepared_database_runtime_future_version_stops_before_open() {
        let temp = tempfile::tempdir().expect("create future preflight test dir");
        let authoritative = temp.path().join(DATABASE_FILE);
        let events = RefCell::new(Vec::<&'static str>::new());
        let runtime = PreparedDatabaseRuntime::prepare_with(temp.path(), |_| {
            Ok(synthetic_identity(authoritative.clone()))
        })
        .expect("prepare runtime capability");

        let decision = runtime
            .preflight_with(|observed_path| {
                events.borrow_mut().push("preflight");
                assert_eq!(observed_path, authoritative);
                Ok(Some(crate::store::SCHEMA_VERSION + 1))
            })
            .expect("future-version preflight returns a decision");

        match decision {
            DatabaseRuntimePreflight::TooNew { version } => {
                assert_eq!(version, crate::store::SCHEMA_VERSION + 1)
            }
            DatabaseRuntimePreflight::Ready(_) => panic!("future database must not be openable"),
        }
        assert_eq!(events.into_inner(), vec!["preflight"]);
    }

    #[test]
    fn no_code_keeps_app_alive_in_tray() {
        assert_eq!(classify_exit_request(None), ExitRequestAction::StayInTray);
    }

    #[test]
    fn popover_focus_loss_and_close_hide_without_entering_main_close_logic() {
        assert_eq!(
            classify_window_event_route("tray-popover", WindowEventKind::FocusLost),
            WindowEventRoute::HidePopover,
        );
        assert_eq!(
            classify_window_event_route("tray-popover", WindowEventKind::CloseRequested),
            WindowEventRoute::HidePopover,
        );
        assert_eq!(
            classify_window_event_route("tray-popover", WindowEventKind::Other),
            WindowEventRoute::Ignore,
        );
        assert_eq!(
            classify_window_event_route("main", WindowEventKind::FocusLost),
            WindowEventRoute::Ignore,
        );
        assert_eq!(
            classify_window_event_route("main", WindowEventKind::CloseRequested),
            WindowEventRoute::HandleMainClose,
        );
        assert_eq!(
            classify_window_event_route("secondary", WindowEventKind::CloseRequested),
            WindowEventRoute::Ignore,
        );
    }

    #[test]
    fn main_visibility_monitor_only_hides_a_minimized_window_while_regular() {
        assert!(should_hide_minimized_main(true, true));
        assert!(!should_hide_minimized_main(true, false));
        assert!(!should_hide_minimized_main(false, true));
        assert!(!should_hide_minimized_main(false, false));
    }

    #[test]
    fn minimize_check_event_fires_only_for_main_window_focus_loss() {
        use tauri::WindowEvent;

        assert!(classify_minimize_check_event(
            "main",
            &WindowEvent::Focused(false)
        ));
        assert!(!classify_minimize_check_event(
            "main",
            &WindowEvent::Focused(true)
        ));
        assert!(!classify_minimize_check_event(
            "main",
            &WindowEvent::Resized(tauri::PhysicalSize::new(800, 600)),
        ));
        assert!(!classify_minimize_check_event(
            crate::tray_popover::TRAY_POPOVER_LABEL,
            &WindowEvent::Focused(false),
        ));
    }

    #[test]
    fn restart_exit_code_defers_to_tauri_default_restart() {
        assert_eq!(
            classify_exit_request(Some(tauri::RESTART_EXIT_CODE)),
            ExitRequestAction::DeferToTauriRestart
        );
    }

    #[test]
    fn user_exit_codes_run_cleanup_then_exit() {
        assert_eq!(
            classify_exit_request(Some(0)),
            ExitRequestAction::CleanupAndExit
        );
        assert_eq!(
            classify_exit_request(Some(1)),
            ExitRequestAction::CleanupAndExit
        );
    }
}
