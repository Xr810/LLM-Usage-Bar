<!-- BEGIN KIMI CODING PLAN DELEGATION (managed) -->
# Kimi Coding Plan 委派规则

- 每次考虑委派前，先调用 `kimi_delegation_status`；总闸关闭或状态不可用时，由 Codex 自己完成。
- 只委派低风险、边界清楚的 Git 任务：测试、lint、简单 bug、机械重构、文档、类型标注和样板代码。
- 禁止委派架构、安全、认证与权限、数据库迁移、依赖升级。
- Codex 必须按顺序读完整个候选补丁，取得 review token 后才能 apply。
- apply 后必须在真实工作区复测；成功则 accept，失败则 rollback。
- 任何 Kimi 失败都立即回到 Codex；同一任务不自动重试 Kimi。
<!-- END KIMI CODING PLAN DELEGATION (managed) -->

## Local Cargo build cache

- Run local Rust commands through `pnpm rust -- <cargo arguments>`.
- Run Tauri through `pnpm tauri -- <arguments>` or `pnpm dev`/`pnpm build`.
- Do not invoke local `cargo build`, `cargo test`, `cargo clippy`, or `tauri` directly.
- Run `pnpm cargo:cache -- status` before removing a worktree.
- `pnpm cargo:cache -- prune` is dry-run; deletion requires `--apply`.
- With pnpm 11, run focused Vitest as `pnpm test:unit <path>`; do not insert `--` before the path, because Vitest may fall back to a broader concurrent run.
