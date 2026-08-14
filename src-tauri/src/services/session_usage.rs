//! 兼容转发层。
//!
//! T1 之后 Claude 会话日志代码已迁至 [`crate::services::ingest`]
//! (解析器在 `ingest/claude.rs`,共享流水线在 `ingest/mod.rs`)。
//! `crate::services::session_usage` 这个旧路径仍被 `usage/session.rs`、
//! `commands/usage.rs` 引用;保留为转发层,避免改动任务书之外的文件。
//! (T14b 把 gemini/opencode 也迁进了 `ingest/`,它们不再引用这里。)

pub use crate::services::ingest::claude::{
    sync_claude_session_logs, sync_claude_session_logs_bound,
};
pub use crate::services::ingest::{
    get_data_source_breakdown, DataSourceSummary, SessionSyncResult,
};
