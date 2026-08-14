//! 兼容转发层。
//!
//! T1 之后 Claude 会话日志代码已迁至 [`crate::services::ingest`]
//! (解析器在 `ingest/claude.rs`,共享流水线在 `ingest/mod.rs`)。
//! `crate::services::session_usage` 这个旧路径仍被 gemini/opencode 会话日志
//! 与 `usage/session.rs`、`commands/usage.rs` 引用;保留为转发层,避免改动
//! 任务书之外的文件。

pub use crate::services::ingest::claude::{
    sync_claude_session_logs, sync_claude_session_logs_bound,
};
pub use crate::services::ingest::{
    get_data_source_breakdown, DataSourceSummary, SessionSyncResult,
};
pub(crate) use crate::services::ingest::{
    load_sync_cursors, metadata_modified_nanos, update_sync_state, SyncCursorMap,
};
