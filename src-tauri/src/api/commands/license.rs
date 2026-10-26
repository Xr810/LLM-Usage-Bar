//! BE-1 License 外壳（契约 C2、C20）。
//!
//! 社区构建恒为 `tier=free`、`build=community`。激活和停用返回
//! `COMMUNITY_BUILD`，不读也不写设置、数据库或钥匙串。

use crate::error::AppError;
use crate::policy::{
    activate_community_license, community_license_status, deactivate_community_license,
    LicenseStatus,
};

#[tauri::command]
pub fn get_license_status() -> LicenseStatus {
    community_license_status()
}

#[tauri::command]
pub fn activate_license(token: String) -> Result<LicenseStatus, AppError> {
    activate_community_license(&token)
}

#[tauri::command]
pub fn deactivate_license() -> Result<(), AppError> {
    deactivate_community_license()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_shell_matches_the_community_provider() {
        let before = get_license_status();
        assert_eq!(before.tier, crate::policy::LicenseTier::Free);
        assert_eq!(before.build, crate::policy::LicenseBuild::Community);

        let token = "shell-token";
        let err = activate_license(token.to_string()).expect_err("community");
        assert_eq!(err.code(), Some(crate::error::CODE_COMMUNITY_BUILD));
        assert!(!serde_json::to_string(&err).expect("json").contains(token));
        assert!(deactivate_license().is_err());
        assert_eq!(get_license_status(), before);
    }
}
