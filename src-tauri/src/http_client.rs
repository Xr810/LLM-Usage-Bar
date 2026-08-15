//! 全局 HTTP 客户端模块
//!
//! 提供支持全局代理配置的 HTTP 客户端。
//! 所有需要发送 HTTP 请求的模块都应使用此模块提供的客户端。

use once_cell::sync::OnceCell;
use reqwest::Client;
use std::env;
use std::net::IpAddr;
use std::sync::{Mutex, MutexGuard, RwLock};
use std::time::Duration;

/// 全局 HTTP 客户端实例（None 表示尚未构建）
static GLOBAL_CLIENT: RwLock<Option<Client>> = RwLock::new(None);
static GLOBAL_NO_REDIRECT_CLIENT: RwLock<Option<Client>> = RwLock::new(None);

/// 当前代理 URL（用于日志和状态查询）
static CURRENT_PROXY_URL: RwLock<Option<String>> = RwLock::new(None);

/// 已保存的代理配置（启动时从数据库读出后廉价记录，只存字符串、不构建客户端）
///
/// get() 在全局客户端尚未构建时按它惰性初始化，保证后台 init 完成前
/// 第一个请求也不会绕过用户配置的代理。
static CONFIGURED_PROXY_URL: RwLock<Option<String>> = RwLock::new(None);

/// 初始化互斥锁：把「构建客户端 + 写入全局」串行化
///
/// init / apply_proxy / update_proxy 与 get() 的惰性构建都在这把锁内
/// 完成，避免两个线程同时初始化时一方覆盖另一方的新配置，也避免
/// get() 在初始化进行到一半时缓存一个不带代理的客户端。
static INIT_LOCK: Mutex<()> = Mutex::new(());

/// LLM Usage Bar 代理服务器当前监听的端口
static LLM_USAGE_BAR_PROXY_PORT: OnceCell<RwLock<u16>> = OnceCell::new();

/// 设置 LLM Usage Bar 代理服务器的监听端口
///
/// 应在代理服务器启动时调用，以便系统代理检测能正确识别自己的端口
pub fn set_proxy_port(port: u16) {
    if let Some(lock) = LLM_USAGE_BAR_PROXY_PORT.get() {
        if let Ok(mut current_port) = lock.write() {
            *current_port = port;
            log::debug!("[GlobalProxy] Updated LLM Usage Bar proxy port to {port}");
        }
    } else {
        let _ = LLM_USAGE_BAR_PROXY_PORT.set(RwLock::new(port));
        log::debug!("[GlobalProxy] Initialized LLM Usage Bar proxy port to {port}");
    }
}

/// 获取 LLM Usage Bar 代理服务器的监听端口
fn get_proxy_port() -> u16 {
    LLM_USAGE_BAR_PROXY_PORT
        .get()
        .and_then(|lock| lock.read().ok())
        .map(|port| *port)
        .unwrap_or(15722) // LLM Usage Bar 默认端口作为回退
}

/// 初始化全局 HTTP 客户端
///
/// 应用启动时调用一次（T24 起挪到后台任务）。若已初始化过，
/// 等价于 apply_proxy 做一次热更新。
///
/// # Arguments
/// * `proxy_url` - 代理 URL，如 `http://127.0.0.1:7890` 或 `socks5://127.0.0.1:1080`
///   传入 None 或空字符串表示直连
pub fn init(proxy_url: Option<&str>) -> Result<(), String> {
    let effective_url = proxy_url.filter(|s| !s.trim().is_empty());
    let _guard = lock_init();
    let already_initialized = GLOBAL_CLIENT
        .read()
        .ok()
        .map(|client| client.is_some())
        .unwrap_or(false);

    install_clients(effective_url)?;

    let masked = effective_url
        .map(mask_url)
        .unwrap_or_else(|| "direct connection".to_string());
    if already_initialized {
        log::warn!("[GlobalProxy] [GP-003] Already initialized, updating instead: {masked}");
    } else {
        log::info!("[GlobalProxy] Initialized: {masked}");
    }
    Ok(())
}

/// 验证代理配置（不应用）
///
/// 只验证代理 URL 是否有效，不实际更新全局客户端。
/// 用于在持久化之前验证配置的有效性。
///
/// # Arguments
/// * `proxy_url` - 代理 URL，None 或空字符串表示直连
///
/// # Returns
/// 验证成功返回 Ok(())，失败返回错误信息
pub fn validate_proxy(proxy_url: Option<&str>) -> Result<(), String> {
    let effective_url = proxy_url.filter(|s| !s.trim().is_empty());
    // 只调用 build_client 来验证，但不应用
    build_client(effective_url)?;
    Ok(())
}

/// 应用代理配置（假设已验证）
///
/// 直接应用代理配置到全局客户端，不做额外验证。
/// 应在 validate_proxy 成功后调用。
///
/// # Arguments
/// * `proxy_url` - 代理 URL，None 或空字符串表示直连
pub fn apply_proxy(proxy_url: Option<&str>) -> Result<(), String> {
    let effective_url = proxy_url.filter(|s| !s.trim().is_empty());
    let _guard = lock_init();
    install_clients(effective_url)?;
    log::info!(
        "[GlobalProxy] Applied: {}",
        effective_url
            .map(mask_url)
            .unwrap_or_else(|| "direct connection".to_string())
    );
    Ok(())
}

/// 更新代理配置（热更新）
///
/// 可在运行时调用以更改代理设置，无需重启应用。
/// 注意：此函数同时验证和应用，如果需要先验证后持久化再应用，
/// 请使用 validate_proxy + apply_proxy 组合。
///
/// # Arguments
/// * `proxy_url` - 新的代理 URL，None 或空字符串表示直连
pub fn update_proxy(proxy_url: Option<&str>) -> Result<(), String> {
    let effective_url = proxy_url.filter(|s| !s.trim().is_empty());
    let _guard = lock_init();
    install_clients(effective_url)?;
    log::info!(
        "[GlobalProxy] Updated: {}",
        effective_url
            .map(mask_url)
            .unwrap_or_else(|| "direct connection".to_string())
    );
    Ok(())
}

/// 获取全局 HTTP 客户端
///
/// 若尚未初始化（例如后台 init 还在构建），按已保存的代理配置惰性
/// 初始化后返回。绝不会在用户配置了代理时返回并缓存一个不带代理的
/// 客户端：构建失败只会回落直连，与启动路径 init 失败的行为一致。
pub fn get() -> Client {
    // 快路径：已初始化直接返回，不碰初始化锁
    if let Some(client) = GLOBAL_CLIENT.read().ok().and_then(|c| c.clone()) {
        return client;
    }

    // 慢路径：与 init / update_proxy 串行，消除初始化竞态
    let _guard = lock_init();

    // 拿锁后重查：等待期间可能已被其他线程初始化
    if let Some(client) = GLOBAL_CLIENT.read().ok().and_then(|c| c.clone()) {
        return client;
    }

    let saved_url = CONFIGURED_PROXY_URL.read().ok().and_then(|u| u.clone());
    match install_clients(saved_url.as_deref()) {
        Ok(client) => {
            log::info!(
                "[GlobalProxy] Lazily initialized on first get(): {}",
                saved_url
                    .as_deref()
                    .map(mask_url)
                    .unwrap_or_else(|| "direct connection".to_string())
            );
            client
        }
        Err(e) => {
            // 已保存的代理配置非法：回落直连。数据库里那份无效配置
            // 仍由 lib.rs 后台 init 的清理逻辑负责清除。
            log::error!(
                "[GlobalProxy] [GP-009] Failed to build client from saved proxy config, falling back to direct: {e}"
            );
            match install_clients(None) {
                Ok(client) => client,
                Err(direct_err) => {
                    log::warn!(
                        "[GlobalProxy] [GP-004] Failed to build direct fallback client: {direct_err}"
                    );
                    Client::default()
                }
            }
        }
    }
}

/// 获取当前代理 URL
///
/// 返回当前配置的代理 URL，None 表示直连。
pub fn get_current_proxy_url() -> Option<String> {
    CURRENT_PROXY_URL.read().ok().and_then(|url| url.clone())
}

/// 检查是否正在使用代理
pub fn is_proxy_enabled() -> bool {
    get_current_proxy_url().is_some()
}

/// 记录已保存的代理配置（不构建客户端，开销极小）
///
/// 启动早期由 lib.rs 调用：把数据库里保存的代理 URL 先记录进内存，
/// 供 get() 在后台 init 完成前按正确配置惰性构建，避免拿到不带代理
/// 的兜底客户端。init / apply_proxy / update_proxy 生效后也会同步
/// 刷新这份记录。
pub(crate) fn set_saved_proxy_url(proxy_url: Option<&str>) {
    let effective = proxy_url
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string());
    let _guard = lock_init();
    if let Ok(mut saved) = CONFIGURED_PROXY_URL.write() {
        *saved = effective;
    }
}

/// 获取初始化互斥锁；锁被毒化时沿用内部值继续（全局状态本身无损坏）
fn lock_init() -> MutexGuard<'static, ()> {
    INIT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 在 INIT_LOCK 保护下构建并安装两个全局客户端，同时刷新代理记录
///
/// 先构建、后写入：构建失败时全局状态保持不变（与 update_proxy 现有
/// 的「验证失败不影响当前客户端」语义一致）。
fn install_clients(proxy_url: Option<&str>) -> Result<Client, String> {
    let client = build_client(proxy_url)?;
    let no_redirect_client = build_client_with_redirects(proxy_url, false)?;

    let mut global = GLOBAL_CLIENT.write().map_err(|e| {
        log::error!("[GlobalProxy] [GP-001] Failed to acquire write lock: {e}");
        "Failed to update proxy: lock poisoned".to_string()
    })?;
    *global = Some(client.clone());
    drop(global);

    let mut no_redirect = GLOBAL_NO_REDIRECT_CLIENT.write().map_err(|e| {
        log::error!("[GlobalProxy] Failed to acquire no-redirect write lock: {e}");
        "Failed to update no-redirect client: lock poisoned".to_string()
    })?;
    *no_redirect = Some(no_redirect_client);
    drop(no_redirect);

    let recorded = proxy_url.map(|s| s.to_string());

    // CONFIGURED_PROXY_URL 与 CURRENT_PROXY_URL 保存同一个值：
    // 前者供 get() 惰性构建使用，后者用于日志和状态查询。
    let mut saved = CONFIGURED_PROXY_URL.write().map_err(|e| {
        log::error!("[GlobalProxy] Failed to acquire saved-config write lock: {e}");
        "Failed to record proxy config: lock poisoned".to_string()
    })?;
    *saved = recorded.clone();
    drop(saved);

    let mut current = CURRENT_PROXY_URL.write().map_err(|e| {
        log::error!("[GlobalProxy] [GP-002] Failed to acquire URL write lock: {e}");
        "Failed to update proxy URL record: lock poisoned".to_string()
    })?;
    *current = recorded;

    Ok(client)
}

/// 构建 HTTP 客户端
fn build_client(proxy_url: Option<&str>) -> Result<Client, String> {
    build_client_with_redirects(proxy_url, true)
}

fn build_client_with_redirects(
    proxy_url: Option<&str>,
    follow_redirects: bool,
) -> Result<Client, String> {
    let mut builder = Client::builder()
        .timeout(Duration::from_secs(600))
        .connect_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(10)
        .tcp_keepalive(Duration::from_secs(60))
        // 禁用 reqwest 自动解压：防止 reqwest 覆盖客户端原始 accept-encoding header。
        // 响应解压由 response_processor 根据 content-encoding 手动处理。
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .redirect(if follow_redirects {
            reqwest::redirect::Policy::default()
        } else {
            reqwest::redirect::Policy::none()
        });

    // 有代理地址则使用代理，否则跟随系统代理
    if let Some(url) = proxy_url {
        // 先验证 URL 格式和 scheme
        let parsed = url::Url::parse(url)
            .map_err(|e| format!("Invalid proxy URL '{}': {}", mask_url(url), e))?;

        let scheme = parsed.scheme();
        if !["http", "https", "socks5", "socks5h"].contains(&scheme) {
            return Err(format!(
                "Invalid proxy scheme '{}' in URL '{}'. Supported: http, https, socks5, socks5h",
                scheme,
                mask_url(url)
            ));
        }

        let proxy = reqwest::Proxy::all(url)
            .map_err(|e| format!("Invalid proxy URL '{}': {}", mask_url(url), e))?;
        builder = builder.proxy(proxy);
        log::debug!("[GlobalProxy] Proxy configured: {}", mask_url(url));
    } else {
        // 未设置全局代理时，让 reqwest 自动检测系统代理（环境变量）
        // 若系统代理指向本机，禁用系统代理避免自环
        if system_proxy_points_to_loopback() {
            builder = builder.no_proxy();
            log::warn!(
                "[GlobalProxy] System proxy points to localhost, bypassing to avoid recursion"
            );
        } else {
            log::debug!("[GlobalProxy] Following system proxy (no explicit proxy configured)");
        }
    }

    builder
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))
}

fn system_proxy_points_to_loopback() -> bool {
    const KEYS: [&str; 6] = [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ];

    KEYS.iter()
        .filter_map(|key| env::var(key).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .any(|value| proxy_points_to_loopback(&value))
}

fn proxy_points_to_loopback(value: &str) -> bool {
    fn host_is_loopback(host: &str) -> bool {
        if host.eq_ignore_ascii_case("localhost") {
            return true;
        }
        host.parse::<IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
    }

    // 检查是否指向 LLM Usage Bar 自己的代理端口
    // 只有指向自己的代理才需要跳过，避免递归
    fn is_llm_usage_bar_proxy_port(port: Option<u16>) -> bool {
        let llm_usage_bar_port = get_proxy_port();
        port == Some(llm_usage_bar_port)
    }

    if let Ok(parsed) = url::Url::parse(value) {
        if let Some(host) = parsed.host_str() {
            // 只有当主机是 loopback 且端口是 LLM Usage Bar 的端口时才返回 true
            return host_is_loopback(host) && is_llm_usage_bar_proxy_port(parsed.port());
        }
        return false;
    }

    let with_scheme = format!("http://{value}");
    if let Ok(parsed) = url::Url::parse(&with_scheme) {
        if let Some(host) = parsed.host_str() {
            return host_is_loopback(host) && is_llm_usage_bar_proxy_port(parsed.port());
        }
    }

    false
}

/// 隐藏 URL 中的敏感信息（用于日志）
pub fn mask_url(url: &str) -> String {
    if let Ok(parsed) = url::Url::parse(url) {
        // 隐藏用户名和密码，保留 scheme、host 和端口
        let host = parsed.host_str().unwrap_or("?");
        match parsed.port() {
            Some(port) => format!("{}://{}:{}", parsed.scheme(), host, port),
            None => format!("{}://{}", parsed.scheme(), host),
        }
    } else {
        // URL 解析失败，返回部分内容
        if url.len() > 20 {
            format!("{}...", &url[..20])
        } else {
            url.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier, Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    /// 测试专用：把全局状态清回未初始化。
    ///
    /// 全仓库只有本文件的测试（经 env_lock 串行）与 s3.rs 两处
    /// #[ignore] 测试会碰这些全局，因此测试里直接写锁即可。
    fn reset_globals_for_test() {
        *GLOBAL_CLIENT.write().unwrap() = None;
        *GLOBAL_NO_REDIRECT_CLIENT.write().unwrap() = None;
        *CURRENT_PROXY_URL.write().unwrap() = None;
        *CONFIGURED_PROXY_URL.write().unwrap() = None;
    }

    #[test]
    fn test_mask_url() {
        assert_eq!(mask_url("http://127.0.0.1:7890"), "http://127.0.0.1:7890");
        assert_eq!(
            mask_url("http://user:pass@127.0.0.1:7890"),
            "http://127.0.0.1:7890"
        );
        assert_eq!(
            mask_url("socks5://admin:secret@proxy.example.com:1080"),
            "socks5://proxy.example.com:1080"
        );
        // 无端口的 URL 不应显示 ":?"
        assert_eq!(
            mask_url("http://proxy.example.com"),
            "http://proxy.example.com"
        );
        assert_eq!(
            mask_url("https://user:pass@proxy.example.com"),
            "https://proxy.example.com"
        );
    }

    #[test]
    fn test_build_client_direct() {
        let result = build_client(None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_client_with_http_proxy() {
        let result = build_client(Some("http://127.0.0.1:7890"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_client_with_socks5_proxy() {
        let result = build_client(Some("socks5://127.0.0.1:1080"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_client_invalid_url() {
        // reqwest::Proxy::all 对某些无效 URL 不会立即报错
        // 使用明确无效的 scheme 来触发错误
        let result = build_client(Some("invalid-scheme://127.0.0.1:7890"));
        assert!(result.is_err(), "Should reject invalid proxy scheme");
    }

    #[test]
    fn test_proxy_points_to_loopback() {
        // 设置 LLM Usage Bar 代理端口为 15721（默认值）
        set_proxy_port(15721);

        // 只有指向 LLM Usage Bar 自己端口的 loopback 地址才返回 true
        assert!(proxy_points_to_loopback("http://127.0.0.1:15721"));
        assert!(proxy_points_to_loopback("socks5://localhost:15721"));
        assert!(proxy_points_to_loopback("127.0.0.1:15721"));

        // 其他 loopback 端口不应该被跳过（允许使用其他本地代理工具）
        assert!(!proxy_points_to_loopback("http://127.0.0.1:7890"));
        assert!(!proxy_points_to_loopback("socks5://localhost:1080"));

        // 非 loopback 地址不应该被跳过
        assert!(!proxy_points_to_loopback("http://192.168.1.10:7890"));
        assert!(!proxy_points_to_loopback("http://192.168.1.10:15721"));
    }

    #[test]
    fn test_system_proxy_points_to_loopback() {
        let _guard = env_lock().lock().unwrap();

        // 设置 LLM Usage Bar 代理端口
        set_proxy_port(15721);

        let keys = [
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
        ];

        for key in &keys {
            std::env::remove_var(key);
        }

        // 指向 LLM Usage Bar 端口的代理应该被跳过
        std::env::set_var("HTTP_PROXY", "http://127.0.0.1:15721");
        assert!(system_proxy_points_to_loopback());

        // 指向其他端口的本地代理不应该被跳过
        std::env::set_var("HTTP_PROXY", "http://127.0.0.1:7890");
        assert!(!system_proxy_points_to_loopback());

        // 非 loopback 地址不应该被跳过
        std::env::set_var("HTTP_PROXY", "http://10.0.0.2:7890");
        assert!(!system_proxy_points_to_loopback());

        for key in &keys {
            std::env::remove_var(key);
        }
    }

    #[test]
    fn test_get_lazily_initializes_with_saved_proxy() {
        let _env = env_lock().lock().unwrap();
        reset_globals_for_test();

        let proxy = "http://127.0.0.1:7890";
        *CONFIGURED_PROXY_URL.write().unwrap() = Some(proxy.to_string());

        let _client = get();

        // get() 按已保存的代理配置惰性初始化，而不是拿直连 fallback
        assert_eq!(get_current_proxy_url().as_deref(), Some(proxy));

        reset_globals_for_test();
    }

    #[test]
    fn test_get_falls_back_to_direct_on_invalid_saved_proxy() {
        let _env = env_lock().lock().unwrap();
        reset_globals_for_test();

        // scheme 非法，与 build_client 的校验路径一致
        *CONFIGURED_PROXY_URL.write().unwrap() =
            Some("invalid-scheme://127.0.0.1:7890".to_string());

        let _client = get(); // 不应 panic

        // 配置非法时回落直连，与启动路径 init 失败 → init(None) 一致
        assert_eq!(get_current_proxy_url(), None);

        reset_globals_for_test();
    }

    #[test]
    fn test_get_concurrent_threads_share_one_proxy_client() {
        let _env = env_lock().lock().unwrap();
        reset_globals_for_test();

        let proxy = "http://127.0.0.1:7890";
        *CONFIGURED_PROXY_URL.write().unwrap() = Some(proxy.to_string());

        let barrier = Arc::new(Barrier::new(8));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                let _client = get();
                // 谁先初始化都行，但所有线程看到的必须是同一个
                // 带代理配置的全局客户端，而不是直连 fallback
                assert_eq!(get_current_proxy_url().as_deref(), Some(proxy));
            }));
        }
        for handle in handles {
            handle.join().expect("worker thread panicked");
        }

        reset_globals_for_test();
    }
}
