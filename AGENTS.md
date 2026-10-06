<!-- BEGIN KIMI CODING PLAN DELEGATION (managed) -->
# Kimi Coding Plan 委派规则

- 每次考虑委派前，先调用 `kimi_delegation_status`；总闸关闭或状态不可用时，由 Codex 自己完成。
- 只委派低风险、边界清楚的 Git 任务：测试、lint、简单 bug、机械重构、文档、类型标注和样板代码。
- 禁止委派架构、安全、认证与权限、数据库迁移、依赖升级。
- Codex 必须按顺序读完整个候选补丁，取得 review token 后才能 apply。
- apply 后必须在真实工作区复测；成功则 accept，失败则 rollback。
- 任何 Kimi 失败都立即回到 Codex；同一任务不自动重试 Kimi。
<!-- END KIMI CODING PLAN DELEGATION (managed) -->

## 模块化维护与交接

- 接手模块化、CLI、后台任务或接口契约工作前，先读并遵循 [HANDOFF.md §20：模块化维护路线](HANDOFF.md#modularity-maintenance-roadmap)。
- 优先完成后台退出集成测试与跨平台验证，再推进 CLI 的安装发现、安装／升级计划、平台执行拆分；之后逐条消除依赖守卫例外、扩展核心接口契约覆盖。
- 按该节的验收标准分批执行并回填验证证据；环境受限的未执行项必须明确记录，不能当作通过。不要另建重复的交接清单。

## Local Cargo build cache

- Run local Rust commands through `pnpm rust -- <cargo arguments>`.
- Run Tauri through `pnpm tauri -- <arguments>` or `pnpm dev`/`pnpm build`.
- Do not invoke local `cargo build`, `cargo test`, `cargo clippy`, or `tauri` directly.
- Run `pnpm cargo:cache -- status` before removing a worktree.
- `pnpm cargo:cache -- prune` is dry-run; deletion requires `--apply`.
- `pnpm cargo:cache -- clean-current` is dry-run; use `--apply` to run a
  wrapper-managed `cargo clean` for the current lock bucket after local Rust
  and Tauri builds, and apps launched from that target, have stopped.
- With pnpm 11, run focused Vitest as `pnpm test:unit <path>`; do not insert `--` before the path, because Vitest may fall back to a broader concurrent run.
