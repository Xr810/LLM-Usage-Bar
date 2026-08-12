# Quota Scheduler Startup Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent the desktop application from aborting when the quota scheduler starts from Tauri's synchronous setup callback.

**Architecture:** Keep the existing quota scheduler future, cancellation channel, handle, and `AppState` lifecycle intact. Replace the ambient Tokio spawn requirement with Tauri's process-wide async runtime and prove the lifecycle boundary with a synchronous regression test.

**Tech Stack:** Rust, Tokio, Tauri 2 async runtime, Cargo test/Clippy, pnpm, Vitest, Vite, macOS Tauri desktop runtime.

## Global Constraints

- Use a temporary isolated `HOME` for every desktop launch.
- Do not read or modify the user's real `~/.cc-switch` or `~/.llm-usage-bar` data.
- Do not change quota collection, timing, persistence, cancellation behavior, or the `AppState` public interface.
- Keep all code and commits on `codex/integration-pr4-pr5-pr6`; do not modify or push remote PR branches or `main`.

---

### Task 1: Make scheduler startup independent of an ambient Tokio reactor

**Files:**
- Modify: `src-tauri/src/usage/quota.rs:14-16,275-305,327-345`
- Test: `src-tauri/src/usage/quota.rs` inline test module

**Interfaces:**
- Consumes: `QuotaService::start_scheduler(self: Arc<Self>) -> QuotaSchedulerHandle` and Tauri's singleton async runtime.
- Produces: the same `QuotaSchedulerHandle` API, backed by `tauri::async_runtime::JoinHandle<()>`.

- [ ] **Step 1: Write the failing regression test**

Add this synchronous test to the existing `quota.rs` test module:

```rust
#[test]
fn scheduler_can_start_without_an_ambient_tokio_runtime() {
    let service = Arc::new(QuotaService::new(Arc::new(Database::memory().unwrap())));

    let scheduler = service.start_scheduler();

    drop(scheduler);
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml scheduler_can_start_without_an_ambient_tokio_runtime -- --exact --nocapture
```

Expected: FAIL with `there is no reactor running, must be called from the context of a Tokio 1.x runtime` at the direct `tokio::spawn` call.

- [ ] **Step 3: Make the minimal runtime change**

Replace the task handle import and spawn call:

```rust
use tauri::async_runtime::JoinHandle;
use tokio::sync::watch;
```

```rust
let task = tauri::async_runtime::spawn(async move {
```

Leave the scheduler future body and `QuotaSchedulerHandle` cancellation and drop implementations unchanged.

- [ ] **Step 4: Run the focused test and verify GREEN**

Run the same focused Cargo command. Expected: one matching test passes, with no panic.

- [ ] **Step 5: Commit the tested fix**

```bash
git add src-tauri/src/usage/quota.rs
git commit -m "fix(usage): start quota scheduler on Tauri runtime"
```

### Task 2: Re-run all automated gates

**Files:**
- Verify only: repository-wide frontend and Rust gates

**Interfaces:**
- Consumes: the Task 1 scheduler fix.
- Produces: clean full-suite evidence suitable for desktop smoke testing.

- [ ] **Step 1: Run the Rust gate**

```bash
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: all commands exit 0; the prior baseline of 1857 passed and 2 ignored library tests remains green, with the new synchronous regression adding one passing test.

- [ ] **Step 2: Run the frontend gate**

```bash
npx --yes pnpm@10.12.3 install --frozen-lockfile
npx --yes pnpm@10.12.3 typecheck
npx --yes pnpm@10.12.3 format:check
npx --yes pnpm@10.12.3 test:unit
npx --yes pnpm@10.12.3 build:renderer
```

Expected: install, type checking, formatting, 433 unit tests, and renderer build all exit 0.

- [ ] **Step 3: Verify repository hygiene**

```bash
git diff --check
git status --short --branch
```

Expected: no unstaged or untracked files; the integration branch is ahead of `origin/main` only by its intentional commits.

### Task 3: Complete the isolated desktop smoke test

**Files:**
- Runtime fixture only: `/tmp/llm-usage-bar-smoke.<random>`

**Interfaces:**
- Consumes: a green automated gate and the Tauri development application.
- Produces: observed macOS UI evidence without touching real user data.

- [ ] **Step 1: Create an isolated desktop home and launch Tauri**

```bash
SMOKE_HOME="$(mktemp -d /tmp/llm-usage-bar-smoke.XXXXXX)"
HOME="$SMOKE_HOME" npx --yes pnpm@10.12.3 tauri dev
```

Expected: the process reaches `Running target/debug/cc-switch`, performs schema migration inside `$SMOKE_HOME/.llm-usage-bar`, and stays running without a crash report.

- [ ] **Step 2: Inspect the real desktop window**

Use macOS UI automation to verify:

- The `LLM Usage Bar` main window becomes visible and responsive.
- The Usage Dashboard is the main route and renders its summary/filter/provider surfaces without an error overlay.
- Basic navigation to another visible section and back succeeds.
- No startup panic or fatal error appears in the Tauri terminal.

- [ ] **Step 3: Stop the development process cleanly**

Send Ctrl-C to the running Tauri terminal and confirm the process exits. Preserve the isolated home path for diagnostic evidence; do not delete user or application data.

- [ ] **Step 4: Record final branch state**

```bash
git status --short --branch
git log -3 --oneline
```

Expected: the worktree is clean and contains the design, plan, and tested implementation commits.
