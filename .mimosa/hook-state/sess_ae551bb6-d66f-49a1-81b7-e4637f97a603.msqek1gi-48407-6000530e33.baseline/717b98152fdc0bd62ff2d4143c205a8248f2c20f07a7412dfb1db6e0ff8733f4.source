use crate::error::AppError;
#[cfg(target_os = "macos")]
use auto_launch::MacOSLaunchMode;
use auto_launch::{AutoLaunch, AutoLaunchBuilder};

fn build_auto_launch(exe_path: &std::path::Path) -> Result<AutoLaunch, AppError> {
    let app_name = "LLM Usage Bar";
    let mut builder = AutoLaunchBuilder::new();
    builder
        .set_app_name(app_name)
        .set_app_path(&exe_path.to_string_lossy());

    // auto-launch 0.6 changed its macOS default from AppleScript to LaunchAgent.
    // LaunchAgent must execute the binary inside the bundle; passing the `.app`
    // directory produces an invalid ProgramArguments entry and launchd exits
    // with EX_CONFIG.
    #[cfg(target_os = "macos")]
    builder.set_macos_launch_mode(MacOSLaunchMode::LaunchAgent);

    builder
        .build()
        .map_err(|e| AppError::Message(format!("创建 AutoLaunch 失败: {e}")))
}

/// 初始化 AutoLaunch 实例
fn get_auto_launch() -> Result<AutoLaunch, AppError> {
    let exe_path =
        std::env::current_exe().map_err(|e| AppError::Message(format!("无法获取应用路径: {e}")))?;
    build_auto_launch(&exe_path)
}

/// 启用开机自启
pub fn enable_auto_launch() -> Result<(), AppError> {
    let auto_launch = get_auto_launch()?;
    auto_launch
        .enable()
        .map_err(|e| AppError::Message(format!("启用开机自启失败: {e}")))?;
    log::info!("已启用开机自启");
    Ok(())
}

/// 禁用开机自启
pub fn disable_auto_launch() -> Result<(), AppError> {
    let auto_launch = get_auto_launch()?;
    auto_launch
        .disable()
        .map_err(|e| AppError::Message(format!("禁用开机自启失败: {e}")))?;
    log::info!("已禁用开机自启");
    Ok(())
}

/// 检查是否已启用开机自启
pub fn is_auto_launch_enabled() -> Result<bool, AppError> {
    let auto_launch = get_auto_launch()?;
    auto_launch
        .is_enabled()
        .map_err(|e| AppError::Message(format!("检查开机自启状态失败: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_launch_agent_targets_bundle_executable() {
        let exe_path =
            std::path::Path::new("/Applications/LLM Usage Bar.app/Contents/MacOS/llm-usage-bar");
        let auto_launch = build_auto_launch(exe_path).unwrap();

        assert_eq!(
            auto_launch.get_app_path(),
            "/Applications/LLM Usage Bar.app/Contents/MacOS/llm-usage-bar"
        );
        assert_ne!(
            auto_launch.get_app_path(),
            "/Applications/LLM Usage Bar.app"
        );
    }
}
