//! GitHub Copilot Tauri Commands
//!
//! 提供 Copilot OAuth 认证相关的 Tauri 命令，支持多账号管理。

use crate::proxy::providers::copilot_auth::CopilotAuthManager;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Copilot 认证状态
pub struct CopilotAuthState(pub Arc<RwLock<CopilotAuthManager>>);

// ==================== 设备码流程 ====================

// ==================== 多账号管理 ====================

// ==================== 状态查询 ====================

// ==================== Token 获取 ====================

// ==================== 模型和使用量 ====================
