//! 兼容转发层。
//!
//! T14b 之后 OpenCode 会话日志代码已迁至 [`crate::services::ingest`]
//! (解析器在 `ingest/opencode.rs`,共享流水线在 `ingest/mod.rs`)。
//! 这个旧路径仍被 `lib.rs` 与 `commands/usage.rs` 引用;保留为转发层,
//! 避免改动任务书之外的文件。

pub use crate::services::ingest::opencode::sync_opencode_usage;
