//! Claude Desktop 的磁盘位置与共享常量。
//!
//! 供应商切换/代理接管被移除后，这里只保留使用量统计仍然需要的东西：
//! 3P 配置库目录（设置页要打开它）和 1M 上下文标记（模型定价要区分它）。

use crate::error::AppError;

/// Claude Desktop 模型名里的 1M 上下文标记。带此标记的模型走不同的计价档位。
pub const ONE_M_CONTEXT_MARKER: &str = "[1m]";

#[cfg(any(target_os = "macos", windows))]
const CONFIG_LIBRARY_DIR: &str = "configLibrary";

/// 3P 配置库目录（`.../Claude-3p/configLibrary`）。
pub fn get_config_library_path() -> Result<std::path::PathBuf, AppError> {
    #[cfg(target_os = "macos")]
    {
        Ok(crate::config::get_home_dir()
            .join("Library")
            .join("Application Support")
            .join("Claude-3p")
            .join(CONFIG_LIBRARY_DIR))
    }

    #[cfg(windows)]
    {
        let local_app_data = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| crate::config::get_home_dir().join("AppData").join("Local"));
        Ok(local_app_data.join("Claude-3p").join(CONFIG_LIBRARY_DIR))
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Err(AppError::localized(
            "claude_desktop.unsupported_platform",
            "当前平台暂不支持 Claude Desktop 3P 配置。第一阶段仅支持 macOS 和 Windows。",
            "Claude Desktop 3P configuration is not supported on this platform yet. Phase 1 only supports macOS and Windows.",
        ))
    }
}
