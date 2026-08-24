//! 本地 router:路由决策、失败分类、Codex 指针管理与 HTTP 转发层。
//!
//! T4/T5/T7 并行往这里加子模块,合并时保留所有 `pub mod xxx;` 行即可。

/// 目前只有 Codex 这一条转发链在跑。
///
/// 路由按 agent 分(`usage_providers.route_app_type`),而这个 server 是接在
/// `~/.codex/config.toml` 上的,所以它服务的就是 codex。将来 Claude 那条链
/// 要么另起一个监听、要么按入站路径分流,到时这个常量会变成参数。
pub const ROUTER_AGENT_CODEX: &str = "codex";

pub mod auth;
pub mod decision;
pub mod failure;
pub mod pointer;
pub mod server;
