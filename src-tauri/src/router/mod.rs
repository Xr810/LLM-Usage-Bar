//! 本地路由:为模型请求决定「按什么顺序试哪几家 provider」。
//!
//! v1 目前只有路由决策纯函数(decision);失败分类(T5)与指针写入/逃生命令(T7)
//! 会作为同级子模块挂进这里。

// decision 的纯函数在 T6(HTTP 转发层)接入前没有生产调用方,lib 构建里会被
// dead_code 报「从未使用」(仅 cfg(test) 的单元测试不算 lib 的使用),而 clippy
// 是 -D warnings。与 usage/mod.rs 里 watcher_state 的豁免同款:非测试构建下
// 整体豁免,T6 接入真实调用方后可移除。
#![cfg_attr(not(test), allow(dead_code))]

pub mod decision;
