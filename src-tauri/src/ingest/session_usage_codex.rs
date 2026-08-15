//! 兼容转发层。
//!
//! T1 之后 Codex 会话日志代码已迁至 [`crate::services::ingest::codex`]。
//! `crate::services::session_usage_codex` 这个旧路径仍被
//! `usage/session.rs`、`commands/usage.rs` 引用;保留为转发层,避免改动
//! 任务书之外的文件。

pub use crate::ingest::codex::{sync_codex_usage, sync_codex_usage_bound};
