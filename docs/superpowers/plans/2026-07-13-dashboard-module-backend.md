# Dashboard Module Backend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist dynamic dashboard modules, expose the five frontend Tauri commands, and enforce subscription Provider membership without changing historical usage attribution.

**Architecture:** Add a v15 SQLite migration that creates `dashboard_modules`, adds the nullable `usage_providers.dashboard_module_id` foreign key, seeds four stable default modules, and backfills subscription Providers. A focused dashboard-module DAO owns module CRUD/order/visibility invariants, while the existing Provider DAO validates and persists membership in the same transaction. Thin Tauri commands expose these database operations to the already-completed frontend.

**Tech Stack:** Rust, rusqlite/SQLite, serde, Tauri 2, Vitest/React frontend, pnpm Cargo cache wrapper.

## Global Constraints

- Run Rust commands only through `pnpm rust -- <cargo arguments>`.
- Run Tauri only through `pnpm tauri -- <arguments>` or `pnpm dev`/`pnpm build`.
- Keep `usage_events.provider_id` and `usage_events.product_group_id` unchanged.
- Seed stable module IDs `codex`, `claude-code`, `kimi-coding-plan`, and `api`.
- `api` is the only system/API module; it can be renamed, reordered, or hidden but cannot be deleted or change kind.
- Enabled subscription Providers must reference a subscription module; metered Providers persist no editable membership and project to `api` in views.
- Do not run the desktop application against the user's real home or data directory during acceptance.

---

### Task 1: Add the v15 dashboard-module migration

**Files:**
- Create: `src-tauri/src/usage/module_migration.rs`
- Modify: `src-tauri/src/usage/mod.rs`
- Modify: `src-tauri/src/database/mod.rs`
- Modify: `src-tauri/src/database/schema.rs`
- Modify: `src-tauri/src/database/tests.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Produces: `usage::module_migration::migrate_v14_to_v15(conn: &Connection) -> Result<(), AppError>`.
- Produces: SQLite table `dashboard_modules` and column `usage_providers.dashboard_module_id`.

- [x] **Step 1: Write failing migration tests**

Add tests that build a v14 fixture and assert migration to v15 creates exactly these ordered rows:

```rust
[
    ("codex", "Codex", "subscription", 0, 1, 0),
    ("claude-code", "Claude Code", "subscription", 1, 1, 0),
    ("kimi-coding-plan", "Kimi Coding Plan", "subscription", 2, 1, 0),
    ("api", "API", "api", 3, 1, 1),
]
```

Also assert known subscription product groups backfill to their matching IDs, unknown subscription product groups backfill to an on-demand `other-subscriptions` row with `needs_review = 1`, metered Providers retain `NULL`, and a deliberately failing trigger rolls back table, column, rows, and `user_version`.

- [x] **Step 2: Verify the migration tests fail for the missing v15 migration**

Run: `pnpm rust -- test --manifest-path src-tauri/Cargo.toml database::tests::migration_v14_to_v15 --lib -- --nocapture`

Expected: FAIL because schema version remains 14 and `dashboard_modules` does not exist.

- [x] **Step 3: Implement the minimal transactional migration**

Create the module table with `CHECK` constraints, a partial unique index for the single API module, and a protected system API row. Add the Provider column only when absent, seed defaults with `ON CONFLICT(id) DO NOTHING`, and backfill by normalized `product_group_id`:

```rust
match product_group_id.trim().to_ascii_lowercase().as_str() {
    "codex" | "openai" => "codex",
    "claude" | "claude-code" | "anthropic" => "claude-code",
    "kimi" | "kimi-coding-plan" | "moonshot" => "kimi-coding-plan",
    _ => "other-subscriptions",
}
```

Wire schema version `14 -> 15`, run v15 completeness validation, and update the fixed v13 fixture guard to expect current schema 15 while preserving identity source schema 13.

- [x] **Step 4: Verify migration tests and existing schema tests pass**

Run: `pnpm rust -- test --manifest-path src-tauri/Cargo.toml database::tests::migration_v14_to_v15 --lib -- --nocapture`

Run: `pnpm rust -- test --manifest-path src-tauri/Cargo.toml database::tests::schema_migration --lib -- --nocapture`

Expected: PASS, with rollback assertions proving no partial migration survives.

- [x] **Step 5: Commit the migration**

```bash
git add src-tauri/src/usage/module_migration.rs src-tauri/src/usage/mod.rs src-tauri/src/database/mod.rs src-tauri/src/database/schema.rs src-tauri/src/database/tests.rs src-tauri/src/lib.rs
git commit -m "feat(usage): migrate dashboard modules"
```

### Task 2: Implement module DAO and Provider membership

**Files:**
- Create: `src-tauri/src/database/dao/dashboard_modules.rs`
- Modify: `src-tauri/src/database/dao/mod.rs`
- Modify: `src-tauri/src/database/dao/usage_providers.rs`
- Modify: `src-tauri/src/usage/domain.rs`
- Modify: all Rust test fixtures constructing `UsageProviderInput`

**Interfaces:**
- Produces: `DashboardModuleKind::{Subscription, Api}`.
- Produces: `DashboardModuleInput { id: Option<String>, name, kind, sort_order, visible }`.
- Produces: `DashboardModuleView { id, name, kind, sort_order, visible, is_system, provider_count }`.
- Produces: `Database::{list_dashboard_modules, save_dashboard_module, reorder_dashboard_modules, set_dashboard_module_visibility, delete_dashboard_module}`.
- Extends: `UsageProviderInput.dashboard_module_id: Option<String>` and `UsageProviderView.dashboard_module_id: Option<String>`.

- [x] **Step 1: Write failing DAO contract tests**

Cover one behavior per test:

```rust
assert_eq!(db.list_dashboard_modules()?.len(), 4);
assert_eq!(db.save_dashboard_module(&new_subscription)?.kind, DashboardModuleKind::Subscription);
assert_eq!(db.reorder_dashboard_modules(&ids)?[0].id, ids[0]);
assert!(!db.set_dashboard_module_visibility("codex", false)?.visible);
assert!(db.delete_dashboard_module("api").unwrap_err().to_string().contains("system"));
assert!(db.delete_dashboard_module("codex").unwrap_err().to_string().contains("providers"));
```

Add Provider tests proving enabled subscription saves reject missing/API membership, disabled subscription saves may remain unassigned, metered saves clear supplied membership and return `api`, reassignment changes only the Provider row, and enabling an unassigned subscription is rejected.

- [x] **Step 2: Verify the DAO tests fail for missing types and methods**

Run: `pnpm rust -- test --manifest-path src-tauri/Cargo.toml database::dao::dashboard_modules::tests --lib -- --nocapture`

Run: `pnpm rust -- test --manifest-path src-tauri/Cargo.toml database::dao::usage_providers::tests --lib -- --nocapture`

Expected: compile/test failure naming the missing dashboard module API.

- [x] **Step 3: Implement the minimal domain and DAO behavior**

Use `uuid::Uuid::new_v4().to_string()` for new custom IDs. In module updates, load the stored row first and preserve `kind`/`is_system` for system modules. Reordering must reject duplicates, unknown IDs, or an incomplete ID set and update all rows in one transaction. Delete only empty non-system subscription modules. Count subscription membership by foreign key and count all metered Providers for `api`.

In `save_usage_provider`, validate membership inside the existing Provider/session-binding transaction before upsert. Persist `NULL` for metered Providers and project their returned/listed `dashboardModuleId` to `api`; keep `NULL` for disabled unassigned subscriptions.

- [x] **Step 4: Run focused DAO tests until green, then run all Rust library tests**

Run: `pnpm rust -- test --manifest-path src-tauri/Cargo.toml database::dao::dashboard_modules::tests --lib -- --nocapture`

Run: `pnpm rust -- test --manifest-path src-tauri/Cargo.toml database::dao::usage_providers::tests --lib -- --nocapture`

Run: `pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib`

Expected: PASS with zero failed tests.

- [x] **Step 5: Commit the DAO and domain contract**

```bash
git add src-tauri/src/database/dao src-tauri/src/usage/domain.rs src-tauri/src/usage src-tauri/src/services src-tauri/src/proxy
git commit -m "feat(usage): persist dashboard module membership"
```

### Task 3: Expose commands and complete isolated acceptance

**Files:**
- Modify: `src-tauri/src/commands/usage_dashboard.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src/lib/query/usageDashboard.test.tsx`

**Interfaces:**
- Produces Tauri commands: `list_dashboard_modules`, `save_dashboard_module`, `reorder_dashboard_modules`, `set_dashboard_module_visibility`, `delete_dashboard_module`.

- [x] **Step 1: Add failing command-hook tests before handlers**

Inside `commands/usage_dashboard.rs`, construct `AppState::new(Arc::new(Database::memory()?))` and call all five wished-for test hooks. Assert the list hook returns four defaults, save returns the new module, reorder returns the requested first ID, visibility returns `visible = false`, and delete removes an empty custom module. Keep `tauri::generate_handler!` compilation as the registration check.

Run: `pnpm rust -- test --manifest-path src-tauri/Cargo.toml commands::usage_dashboard::tests::dashboard_module_command_hooks --lib -- --nocapture`

Expected: compile failure because the five test hooks do not exist.

- [x] **Step 2: Verify registration is absent**

Run: `rg -n "commands::list_dashboard_modules" src-tauri/src/lib.rs`

Expected: no match before implementation.

- [x] **Step 3: Add thin command handlers and register all five names**

Each handler accepts camelCase-compatible Tauri arguments and delegates directly to the same-named database method. No command duplicates validation or starts a second transaction.

- [x] **Step 4: Run full automated gates**

Run: `pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib`

Run: `pnpm test:unit`

Run: `pnpm typecheck`

Run: `pnpm format:check`

Run: `pnpm build`

Run: `pnpm tauri build --debug`

Expected: every command exits 0; known Vite chunk-size and pre-existing Rust dead-code warnings may remain, but no errors or test failures.

- [ ] **Step 5: Launch with isolated data and perform UI acceptance**

Acceptance status (2026-07-13): items 1-3 and 5-8 passed against a fresh temporary home. Item 4 passed for create, rename, reorder, hide, and show; the final empty-module delete confirmation remains pending because macOS locked before Computer Use could open the confirmation dialog.

Create a fresh temporary home, set both `HOME` and `LLM_USAGE_BAR_TEST_HOME` to it, and launch the debug `.app` binary. Verify:

1. Main navigation renders Codex, Claude Code, Kimi Coding Plan, and API without a command error.
2. Each module tab switches content and does not change route bindings.
3. Settings > 用量模块 lists four modules with correct counts and system protections.
4. Add Gemini, rename it, reorder it, hide/show it, and delete it while empty.
5. Provider form requires subscription membership and hides membership for metered Providers.
6. Invalid system deletion and non-empty module deletion show one recoverable error.
7. Relaunching against the same temporary home preserves module changes.
8. No real user data path appears in logs or UI.

- [ ] **Step 6: Review diff and commit command integration**

```bash
git diff --check
git status --short
git add src-tauri/src/commands/usage_dashboard.rs src-tauri/src/lib.rs
git commit -m "feat(usage): expose dashboard module commands"
```
