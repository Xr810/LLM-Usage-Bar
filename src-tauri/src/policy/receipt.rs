//! 契约 C1：`ReceiptExporter`。
//!
//! CSV 导出是私有能力（C21 / BE-11）。社区实现返回 `PRO_REQUIRED`，不写文件。

use crate::error::AppError;

/// 导出时间范围。Unix 毫秒，`end_ms` 为开区间。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportRange {
    pub start_ms: i64,
    pub end_ms: i64,
}

/// 导出过滤条件。路径不属于 trait：写文件是 C21 command 的事。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExportFilter {
    pub client_tag_id: Option<String>,
}

/// 收据导出（契约 C1）。社区实现不产生 CSV，也不碰磁盘。
pub trait ReceiptExporter: Send + Sync {
    /// 返回 CSV 文本。社区构建返回 `PRO_REQUIRED`。
    fn export(&self, range: &ExportRange, filter: &ExportFilter) -> Result<String, AppError>;
}

/// 公开仓库的收据导出实现。没有字段，不能记住导出结果。
#[derive(Debug, Default, Clone, Copy)]
pub struct CommunityReceiptExporter;

impl ReceiptExporter for CommunityReceiptExporter {
    fn export(&self, _range: &ExportRange, _filter: &ExportFilter) -> Result<String, AppError> {
        Err(AppError::pro_required(
            "Receipt export is unavailable in a community build.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn community_export_returns_pro_required_without_echoing_the_filter() {
        let exporter = CommunityReceiptExporter;
        let range = ExportRange {
            start_ms: 0,
            end_ms: 1,
        };
        let filter = ExportFilter {
            client_tag_id: Some("client-secret-tag".to_string()),
        };
        let err = exporter.export(&range, &filter).expect_err("community");
        let rendered = serde_json::to_string(&err).expect("json");
        assert_eq!(
            rendered,
            r#"{"code":"PRO_REQUIRED","message":"Receipt export is unavailable in a community build."}"#
        );
        assert!(!rendered.contains("client-secret-tag"));
        assert_eq!(
            exporter
                .export(&range, &filter)
                .expect_err("idempotent")
                .code(),
            err.code()
        );
    }
}
