# Provider-Only Monitoring Implementation Plan

**Goal:** Make Provider accounts the only usage/remaining-balance dimension while
leaving CC Switch as the sole Provider-switching application.

**Canonical design:**
[`2026-07-17-provider-only-monitoring-design.md`](../specs/2026-07-17-provider-only-monitoring-design.md)

**Status (2026-07-17):** Implemented on `codex/provider-only-monitoring`.
Tasks 1–5 are complete. Task 6 is complete at the runtime boundary: Agent
storage and legacy API compatibility remain dormant by design, while no active
renderer, collection, dashboard, tray, settings, or navigation path depends on
Agent selection or Agent–Provider bindings.

## Constraints

- Preserve existing Provider usage events, quota snapshots, budgets, and fixed
  Provider records.
- Do not introduce schema v19 Agent switching tables.
- Do not drop schema v18 Agent tables in the first pass; make them dormant before
  any later destructive migration is considered.
- Never write CC Switch data or external Agent configuration.
- Use `pnpm rust -- <cargo arguments>` for Rust checks and focused Vitest as
  `pnpm test:unit <path>`.

## Task 1: Freeze the superseded direction

- Mark Agent-centric and Agent Provider switching plans as superseded.
- Remove the uncommitted physical-v19 switching migration draft.
- Record this plan as the current product direction.

## Task 2: Define a Provider-only backend projection

Primary files:

- `src-tauri/src/usage/domain.rs`
- `src-tauri/src/usage/dashboard.rs`
- `src-tauri/src/usage/aggregation.rs`
- `src-tauri/src/commands/usage_dashboard.rs`
- `src-tauri/src/database/dao/usage_providers.rs`
- `src-tauri/src/database/dao/usage_events.rs`

Actions:

- Return one monitoring row per `usage_provider_id`/account.
- Aggregate immutable events by Provider and time range while ignoring
  `agent_module_id` in grouping and filtering.
- Keep subscription quota, metered usage, cost source, budget, freshness, and
  diagnostics explicit in the Provider DTO.
- Prove that two accounts of the same Provider type never merge.

## Task 3: Decouple collection from Agent bindings

Primary files:

- `src-tauri/src/usage/quota.rs`
- `src-tauri/src/usage/session.rs`
- `src-tauri/src/services/tray_usage.rs`
- `src-tauri/src/services/tray_usage_scheduler.rs`

Actions:

- Schedule and manually refresh quota by Provider ID only.
- Remove Agent binding as a monitoring prerequisite.
- Attribute imported evidence directly to the configured Provider account using
  an explicit Provider-scoped source binding or credential identity.
- Preserve the rule that ambiguous evidence is rejected or shown as unassigned;
  never guess from model name or current Agent configuration.
- Return explicit unsupported/unavailable states for Providers lacking a usable
  usage or balance source.

## Task 4: Replace Agent-first main-window navigation

Primary files:

- `src/components/usage-dashboard/UsageDashboardPage.tsx`
- `src/components/usage-dashboard/usageDashboardProjection.ts`
- `src/components/usage-dashboard/SubscriptionProviderCard.tsx`
- `src/components/usage-dashboard/MeteredProviderCard.tsx`
- `src/App.usage-dashboard.test.tsx`

Actions:

- Render a Provider account grid/list directly.
- Remove `AgentUsagePage`, Agent module selection, and Agent-scoped empty states
  from the render path.
- Keep subscription and metered presentation differences inside each Provider
  card rather than as Agent pages.
- Add source/freshness/unavailable presentation and account disambiguation.

## Task 5: Simplify settings and menu-bar usage

Primary files:

- `src/components/settings/UsageProvidersSettings.tsx`
- `src/components/settings/SettingsPage.tsx`
- `src/components/tray-usage/TrayUsagePopover.tsx`
- `src/components/tray-usage/trayUsagePresentation.ts`
- `src-tauri/src/usage/tray_snapshot.rs`

Actions:

- Remove Agent binding, Agent proxy setup, and switching controls from active UI.
- Keep Provider account authentication/source settings, enabled/visible state,
  refresh interval, and daily budget.
- Replace Agent tray tabs with Provider rows or Provider filtering.
- Verify every monitoring interaction causes zero external Agent config writes
  and zero CC Switch writes.

## Task 6: Retire dormant Agent runtime safely

- Remove unused Agent commands, query hooks, and renderer components only after
  Tasks 2–5 no longer reference them.
- Retain schema v18 tables/columns for compatibility in this release.
- Keep historical Agent metadata readable internally until a separately approved
  destructive migration exists.

## Verification

Run focused checks after each task, then the repository gates:

```sh
pnpm test:unit src/components/usage-dashboard/usageDashboardProjection.test.ts
pnpm test:unit src/components/usage-dashboard/UsageDashboardPage.test.tsx
pnpm test:unit src/components/tray-usage/TrayUsagePopover.test.tsx
pnpm typecheck
pnpm build:renderer
pnpm rust -- fmt --check
pnpm rust -- test
pnpm test:unit --exclude '.worktrees/**' --exclude '.pnpm-store/**'
```

Final acceptance must prove Provider-only grouping, multiple-account isolation,
honest unavailable states, refresh behavior, preserved history, and no write to
CC Switch or Agent configuration.
