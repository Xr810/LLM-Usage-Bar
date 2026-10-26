//! 契约 C1 的 `LicenseProvider`，以及 C2 / C20 的社区外壳。
//!
//! 公开构建不验签。激活和停用返回 `COMMUNITY_BUILD`，不写设置、数据库或钥匙串。

use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// `license-changed` 事件名（契约 C2，kebab-case）。
///
/// 社区外壳的状态不会变化，所以命令不发送这个事件。载荷类型是 [`LicenseStatus`]。
pub const LICENSE_CHANGED_EVENT: &str = "license-changed";

/// `activate_license` 的 token 上限（契约 C20：≤ 8KB）。按 UTF-8 字节计。
pub const LICENSE_TOKEN_MAX_BYTES: usize = 8 * 1024;

/// C2 的 `license-changed` 载荷。字段与 [`LicenseStatus`] 相同。
pub type LicenseChangedPayload = LicenseStatus;

/// `free | pro | pro_p3`。枚举值用 snake_case。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseTier {
    Free,
    Pro,
    ProP3,
}

/// `community | official`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseBuild {
    Community,
    Official,
}

/// Display hints only; never use these flags to authorize a Pro operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LicenseUiFlags {
    pub upsell_entry: bool,
}

impl LicenseUiFlags {
    /// Official providers use build-time configuration, not process/runtime settings.
    /// Community remains false even when the official build variable is enabled.
    pub fn for_build(build: LicenseBuild) -> Self {
        Self {
            upsell_entry: build == LicenseBuild::Official
                && matches!(option_env!("LLM_USAGE_BAR_UPSELL_ENTRY"), Some("true")),
        }
    }
}

/// `get_license_status` / `activate_license` 的成功载荷（契约 C2、C20）。
///
/// 残留在契约正文里的 snake_case 字段按 R-2 收成 camelCase：
/// `expiresAt`、`graceUntil`、`emailMasked`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LicenseStatus {
    pub tier: LicenseTier,
    pub state: LicenseState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grace_until: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grace_days: Option<u32>,
    pub build: LicenseBuild,
    pub features: Vec<String>,
    pub ui_flags: LicenseUiFlags,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_masked: Option<String>,
}

impl LicenseStatus {
    /// 社区构建的固定状态：`tier=free`，`build=community`，没有到期或宽限。
    pub fn community_free() -> Self {
        Self {
            tier: LicenseTier::Free,
            state: LicenseState::Free,
            expires_at: None,
            grace_until: None,
            grace_days: None,
            build: LicenseBuild::Community,
            features: Vec::new(),
            ui_flags: LicenseUiFlags::for_build(LicenseBuild::Community),
            email_masked: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseState {
    Active,
    Grace,
    Free,
}

/// License 状态来源（契约 C1）。
///
/// C1 只要求 `status`。`activate` / `deactivate` 放在同一个 trait 上，
/// 这样 C20 的命令壳可以换实现，而不改 command 签名。社区实现不验签。
pub trait LicenseProvider: Send + Sync {
    fn status(&self) -> LicenseStatus;

    /// 激活 token。实现不得把 token 写进日志。
    /// 社区实现返回 [`crate::error::CODE_COMMUNITY_BUILD`]，并且不保存 token。
    ///
    /// Official activation must persist any higher retention via
    /// [`super::RetentionStore::set_pending_target`] BEFORE the new tier
    /// takes effect. This uses BE-30's raise-only operation and durably records
    /// a failed target for the core's tier-independent startup retry. On error,
    /// keep the current tier and propagate LICENSE_ACTIVATION_RETRYABLE unchanged.
    /// Never interrupt proxy traffic or existing features. Token verification
    /// and tier publication remain the injected provider's responsibility.
    fn activate(&self, token: &str) -> Result<LicenseStatus, AppError>;

    /// 停用当前 License。社区实现返回 [`crate::error::CODE_COMMUNITY_BUILD`]，不改状态。
    fn deactivate(&self) -> Result<(), AppError>;
}

/// 公开仓库里唯一的 [`LicenseProvider`]。没有字段，不能保存激活结果。
#[derive(Debug, Default, Clone, Copy)]
pub struct CommunityLicenseProvider;

impl LicenseProvider for CommunityLicenseProvider {
    fn status(&self) -> LicenseStatus {
        LicenseStatus::community_free()
    }

    fn activate(&self, _token: &str) -> Result<LicenseStatus, AppError> {
        Err(AppError::community_build(
            "License activation is unavailable in a community build.",
        ))
    }

    fn deactivate(&self) -> Result<(), AppError> {
        Err(AppError::community_build(
            "License deactivation is unavailable in a community build.",
        ))
    }
}

/// 校验 C20 的 token 形状。超限或空字符串返回 `INVALID_ARGUMENT`，不写状态。
pub fn validate_license_token(token: &str) -> Result<(), AppError> {
    if token.is_empty() {
        return Err(AppError::invalid_argument(
            "token must not be empty.",
            "token",
        ));
    }
    if token.len() > LICENSE_TOKEN_MAX_BYTES {
        return Err(AppError::invalid_argument(
            "token must be at most 8192 bytes.",
            "token",
        ));
    }
    Ok(())
}

pub fn community_license_status() -> LicenseStatus {
    CommunityLicenseProvider.status()
}

/// 社区构建的激活。校验通过后仍返回 `COMMUNITY_BUILD`，不记录 token。
pub fn activate_community_license(token: &str) -> Result<LicenseStatus, AppError> {
    validate_license_token(token)?;
    CommunityLicenseProvider.activate(token)
}

pub fn deactivate_community_license() -> Result<(), AppError> {
    CommunityLicenseProvider.deactivate()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn community_status_is_free_and_omits_optional_fields() {
        let status = community_license_status();
        assert_eq!(status, LicenseStatus::community_free());
        assert_eq!(
            serde_json::to_string(&status).expect("json"),
            r#"{"tier":"free","state":"free","build":"community","features":[],"uiFlags":{"upsellEntry":false}}"#
        );
        let payload: LicenseChangedPayload = status;
        assert_eq!(payload.tier, LicenseTier::Free);
        assert_eq!(LICENSE_CHANGED_EVENT, "license-changed");
    }

    #[test]
    fn official_status_fields_use_camel_case() {
        let status = LicenseStatus {
            tier: LicenseTier::ProP3,
            state: LicenseState::Active,
            expires_at: Some("2026-12-01T00:00:00Z".to_string()),
            grace_until: Some("2026-12-15T00:00:00Z".to_string()),
            grace_days: Some(14),
            build: LicenseBuild::Official,
            features: vec!["heatmap_12m".to_string()],
            ui_flags: LicenseUiFlags::for_build(LicenseBuild::Official),
            email_masked: Some("a***@example.com".to_string()),
        };
        let upsell_entry = option_env!("LLM_USAGE_BAR_UPSELL_ENTRY") == Some("true");
        assert_eq!(
            serde_json::to_value(&status).expect("json"),
            serde_json::json!({
                "tier": "pro_p3",
                "state": "active",
                "expiresAt": "2026-12-01T00:00:00Z",
                "graceUntil": "2026-12-15T00:00:00Z",
                "graceDays": 14,
                "build": "official",
                "features": ["heatmap_12m"],
                "uiFlags": { "upsellEntry": upsell_entry },
                "emailMasked": "a***@example.com"
            })
        );
        assert_eq!(
            serde_json::from_value::<LicenseStatus>(serde_json::to_value(&status).unwrap())
                .expect("round trip"),
            status
        );
        let mut legacy = serde_json::to_value(&status).unwrap();
        legacy.as_object_mut().unwrap().remove("graceDays");
        let decoded = serde_json::from_value::<LicenseStatus>(legacy.clone()).unwrap();
        assert_eq!(decoded.grace_days, None);
        assert_eq!(serde_json::to_value(decoded).unwrap(), legacy);
    }

    #[test]
    fn activate_returns_community_build_and_does_not_change_status() {
        let before = community_license_status();
        let token = "not-a-real-license";
        let first = activate_community_license(token).expect_err("community");
        let second = activate_community_license(token).expect_err("idempotent");
        assert_eq!(first.code(), Some(crate::error::CODE_COMMUNITY_BUILD));
        assert_eq!(
            serde_json::to_string(&first).expect("json"),
            serde_json::to_string(&second).expect("json")
        );
        let rendered = serde_json::to_string(&first).expect("json");
        assert!(!rendered.contains(token));
        assert!(!first.to_string().contains(token));
        assert_eq!(community_license_status(), before);

        let deactivate = deactivate_community_license().expect_err("community");
        assert_eq!(deactivate.code(), Some(crate::error::CODE_COMMUNITY_BUILD));
        assert_eq!(community_license_status(), before);
    }

    #[test]
    fn empty_or_oversized_token_is_invalid_argument_and_writes_nothing() {
        let before = community_license_status();

        let empty = activate_community_license("").expect_err("empty");
        assert_eq!(
            serde_json::to_value(&empty).expect("json"),
            serde_json::json!({
                "code": "INVALID_ARGUMENT",
                "message": "token must not be empty.",
                "details": { "field": "token" }
            })
        );

        let token = "x".repeat(LICENSE_TOKEN_MAX_BYTES + 1);
        let oversized = activate_community_license(&token).expect_err("too long");
        let rendered = serde_json::to_string(&oversized).expect("json");
        assert_eq!(
            serde_json::to_value(&oversized).expect("json"),
            serde_json::json!({
                "code": "INVALID_ARGUMENT",
                "message": "token must be at most 8192 bytes.",
                "details": { "field": "token" }
            })
        );
        assert!(!rendered.contains(&token));
        assert_eq!(community_license_status(), before);

        let boundary = "y".repeat(LICENSE_TOKEN_MAX_BYTES);
        let accepted_shape = activate_community_license(&boundary).expect_err("still community");
        assert_eq!(
            accepted_shape.code(),
            Some(crate::error::CODE_COMMUNITY_BUILD)
        );
        assert!(!serde_json::to_string(&accepted_shape)
            .expect("json")
            .contains(&boundary));
        assert_eq!(community_license_status(), before);
    }
}
