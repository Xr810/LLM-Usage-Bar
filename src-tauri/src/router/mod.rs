//! 本地 router:请求转发、路由决策与 Codex 指针管理。
//!
//! T4/T5/T7 并行往这里加子模块,合并时保留所有 `pub mod xxx;` 行即可。

pub mod failure;
pub mod pointer;
