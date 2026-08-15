//! 与传输无关的业务 api 层(T16 起)。
//!
//! 模块化设计 §9.3 第三条:契约(有哪些查询、哪些命令、返回什么、错误怎么脱敏)
//! 若直接长在 tauri 命令上,socket 面板就得再写一遍。这一层收契约,
//! `commands/` 退化成薄壳,启动接线与将来的 socket 端都直接调这里。
//!
//! 硬约定:本目录任何文件不得 import 任何 tauri 类型——「与传输无关」可机械验证。

pub mod commands;
pub mod openrouter_balance;
pub mod router;
