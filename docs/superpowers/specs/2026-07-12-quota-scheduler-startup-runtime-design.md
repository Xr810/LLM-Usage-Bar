# Quota Scheduler Startup Runtime Fix

## Problem

The desktop application constructs `AppState` inside Tauri's synchronous setup callback. `AppState::start_quota_scheduler` currently reaches `tokio::spawn` directly, but that callback is not entered through a Tokio runtime. A fresh desktop launch therefore aborts with `there is no reactor running` before the main window can be shown.

The existing async unit and integration tests do not reproduce this lifecycle boundary because they already run inside Tokio.

## Design

Keep the scheduler lifecycle and cancellation handle unchanged, but spawn the scheduler future through Tauri's application-wide async runtime. Tauri owns that runtime throughout the desktop process, so it is available from the synchronous setup callback and remains compatible with the existing async shutdown path.

The fix is intentionally limited to the scheduler spawn boundary. Quota collection, scheduling intervals, cancellation semantics, persistence, and the `AppState` public interface remain unchanged.

## Regression coverage

Add a synchronous Rust regression test that constructs an isolated in-memory `QuotaService` and starts its scheduler without first entering a Tokio runtime. The test must fail against the current direct `tokio::spawn` implementation and pass when the Tauri runtime owns the task. Dropping the returned handle must abort the background task so the test leaves no scheduler running.

After the focused test passes, rerun:

- Rust formatting, Clippy with warnings denied, and the full Rust test suite.
- Frontend type checking, formatting, unit tests, and renderer build.
- A real `tauri dev` launch with an isolated `HOME`, followed by UI smoke checks of the main Usage Dashboard and basic navigation.

## Safety and scope

The desktop launch must continue to use a temporary isolated `HOME`; it must not read or modify the user's real CC Switch or LLM Usage Bar data. No PR branch, remote `main`, or the user's original local `main` worktree is changed by this fix. The resulting code commit remains on the integration branch until the user chooses how to publish it.
