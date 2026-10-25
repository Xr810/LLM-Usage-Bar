//! 社区构建的策略、License 与收据导出骨架（BE-1）。
//!
//! 契约 C1 的 trait 放在这里。硬顶、挂起、Kill、验签和 CSV 导出属于私有 crate，
//! 本模块的默认实现只放行，并且不读写 License 或导出状态。

mod gate;
mod license;
mod receipt;

pub use gate::{AllowAllGate, BillingIdentity, Decision, PolicyGate, ReqMeta, Usage};
pub use license::{
    activate_community_license, community_license_status, deactivate_community_license,
    CommunityLicenseProvider, LicenseBuild, LicenseChangedPayload, LicenseProvider, LicenseStatus,
    LicenseTier, LICENSE_CHANGED_EVENT, LICENSE_TOKEN_MAX_BYTES,
};
pub use receipt::{CommunityReceiptExporter, ExportFilter, ExportRange, ReceiptExporter};
