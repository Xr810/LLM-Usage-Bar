# Menu Bar Usage Status and Popover Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the macOS menu-bar glyph with one authoritative green/yellow/red/gray usage dot and add a 380×520 custom usage popover that opens on left click while preserving the existing right-click native menu.

**Architecture:** Schema v18 stores an optional positive daily USD budget in a dedicated Provider column. A pure Rust projector combines visible Agent bindings, persisted subscription quota, and exact Decimal cost aggregates into one `TrayUsageSnapshot`; a cache/refresh service publishes that same object to the icon and React popover. macOS lazily creates a borderless `tray-popover` webview, while the main window remains a separate surface and is shown only through explicit detail/settings navigation.

**Tech Stack:** Tauri 2.11.5, Rust, rusqlite, rust_decimal, chrono, tokio, React 18, TypeScript, TanStack Query, Tailwind, i18next, Vitest, Testing Library.

**Approved design:** [`docs/superpowers/specs/2026-07-15-menu-bar-usage-popover-design.md`](../specs/2026-07-15-menu-bar-usage-popover-design.md)

## Global Constraints

- Subscription: remaining `> 50%` is green, `20%...50%` inclusive is yellow, and `< 20%` is red.
- Metered API: consumed `< 50%` is green, `50%...<80%` is yellow, and `>= 80%` is red.
- Overall severity is `red > yellow > green`; `unknown` is used only before any usable monitored value exists.
- A missing daily budget is normal, remains visible in the popover, and is excluded from status classification.
- Persist the budget only in nullable `usage_providers.daily_budget_usd TEXT`. Never put it in `quota_config`.
- The renderer, command payloads, events, logs, diagnostics, and backups must never expose API keys, OAuth tokens, raw `quota_config`, raw upstream payloads, or credential fingerprints.
- Icon and popover always consume the same cached `TrayUsageSnapshot`. The tray layer must not independently reclassify values.
- Any failed refresh keeps the last successful snapshot and severity, marks it stale, and exposes only a fixed sanitized error code.
- The custom click/window behavior is macOS-only. Windows and Linux keep their current native tray behavior.
- Left `MouseButtonState::Down` toggles the popover; `Up` must be ignored. Right click hides the popover and retains the native menu.
- The popover never changes activation policy or shows the Dock. `Open details` and `Settings` apply Regular policy before showing/focusing `main`.
- Use four real 18×18 transparent PNG assets with an approximately 10-pixel circle; do not use emoji, CSS art, inline SVG, or template rendering for the colored dot.
- Reuse the existing design tokens, components, focus rings, Provider/Agent assets, and installed icon library. Do not add a new package.
- Before implementation, use `superpowers:using-git-worktrees` to create an isolated worktree from `main@3d847d65`. The root worktree has unrelated user edits; never reset, overwrite, format, or stage those edits or `.pnpm-store/`.
- Do not delegate the schema, architecture, credential boundary, or macOS lifecycle work to Kimi. If Kimi is considered for a later mechanical task, call `kimi_delegation_status` first and follow `AGENTS.md`.
- Run Rust only through `pnpm rust -- ...`. Run focused Vitest as `pnpm test:unit <path>` without an inserted `--`.

## File Responsibility Map

### Backend persistence and projection

- Create `src-tauri/src/usage/budget_migration.rs` — v17→v18 additive migration and exact schema validator.
- Create `src-tauri/src/usage/status.rs` — pure threshold classification and worst-state aggregation.
- Create `src-tauri/src/usage/aggregation.rs` — shared exact per-Provider range aggregation.
- Create `src-tauri/src/usage/tray_snapshot.rs` — safe DTOs, local-day windows, and side-effect-free projection.
- Create `src-tauri/src/services/tray_usage.rs` — cache, refresh gate, last-success preservation, source refresh, and midnight loop.
- Create `src-tauri/src/commands/tray_usage.rs` — snapshot, refresh, budget, popover, navigation, pending destination, quit commands.
- Modify `src-tauri/src/database/{mod.rs,schema.rs,tests.rs,backup.rs}` — schema v18 and backup/restore.
- Modify `src-tauri/src/database/dao/usage_providers.rs` — budget read/write and safe view projection.
- Modify `src-tauri/src/usage/{mod.rs,domain.rs,dashboard.rs,quota.rs}` — modules, DTO field, shared aggregation, scheduler callback.
- Modify `src-tauri/src/{store.rs,usage_events.rs,lib.rs}` — service ownership, debounced rebuild triggers, setup, commands.

### macOS tray and window

- Create `src-tauri/src/tray_status.rs` — asset loading, tooltip mapping, and atomic icon updates.
- Create `src-tauri/src/tray_popover.rs` — click classification, monitor clamping, lazy window, blur/close handling, main navigation.
- Create `src-tauri/capabilities/tray-popover.json` — minimum popover capability.
- Create `src-tauri/icons/tray/macos/status_{green,yellow,red,unknown}.png` — four non-template status assets.
- Modify `src-tauri/src/{tray.rs,lib.rs}` — retain native menu, route left/right events, and separate main/popover lifecycle.

### Renderer

- Create `src/types/trayUsage.ts` — TypeScript mirror of the safe Rust DTO.
- Create `src/lib/api/trayUsage.ts` — typed invoke wrappers.
- Create `src/lib/query/trayUsage.ts` — snapshot query, refresh mutation, and event bridge.
- Create `src/windowSurface.tsx` — pure `main`/`tray-popover` surface selection.
- Create `src/components/tray-usage/*.tsx` and `trayUsagePresentation.ts` — popover shell, tabs, sections, progress, footer, formatting.
- Create `src/components/settings/ProviderDailyBudgetField.tsx` — dedicated metered budget editor.
- Create `src/hooks/useMainWindowNavigation.ts` — event plus pending-destination navigation.
- Modify `src/{main.tsx,App.tsx,index.css}` — window bootstrap, main destination state, scoped popover surface.
- Modify Provider settings, four locale files, MSW handlers, and focused tests.

---

## Execution Preflight

- [ ] **Step 1: Create an isolated worktree**

Read `superpowers:using-git-worktrees`, confirm `git status --short` and `git worktree list --porcelain`, then create `codex/menu-bar-usage-popover` under the repository's established `.worktrees` directory. Do not stash or move the root worktree's edits.

```bash
git status --short
git worktree list --porcelain
git worktree add .worktrees/menu-bar-usage-popover -b codex/menu-bar-usage-popover 3d847d65
```

Expected: the new worktree is clean at `3d847d65` and the original root remains dirty and unchanged.

- [ ] **Step 2: Confirm the execution baseline**

```bash
git status --short
pnpm rust -- test --manifest-path src-tauri/Cargo.toml usage::status::tests --lib
pnpm test:unit src/i18n/dashboardCopy.test.ts
```

Expected: `git status` is empty; the future `usage::status` filter reports zero matching tests before Task 2; the existing i18n test passes.

---

### Task 1: Persist a safe per-Provider daily budget in schema v18

**Files:**

- Create: `src-tauri/src/usage/budget_migration.rs`
- Modify: `src-tauri/src/usage/mod.rs`
- Modify: `src-tauri/src/database/mod.rs`
- Modify: `src-tauri/src/database/schema.rs`
- Modify: `src-tauri/src/database/tests.rs`
- Modify: `src-tauri/src/database/backup.rs`
- Modify: `src-tauri/src/usage/domain.rs`
- Modify: `src-tauri/src/database/dao/usage_providers.rs`
- Test: `src-tauri/src/database/tests.rs`
- Test: `src-tauri/src/database/dao/usage_providers.rs`
- Test: `src-tauri/src/database/backup.rs`

**Interfaces:**

- Produces: `UsageProviderStored.daily_budget_usd: Option<String>`.
- Produces: safe renderer field `UsageProviderView.daily_budget_usd: Option<String>`.
- Produces: `Database::set_provider_daily_budget(&self, provider_id: &str, value: Option<&str>) -> Result<UsageProviderView, AppError>`.
- Does not change `UsageProviderInput`; ordinary Provider saves cannot alter a budget.

- [ ] **Step 1: Write failing v17→v18 migration and preservation tests**

Add tests that create an exact v17 fixture, insert one custom Provider and run the existing system catalog reconciliation, migrate, and verify:

```rust
assert_eq!(Database::get_user_version(&conn).unwrap(), 18);
assert!(Database::has_column(&conn, "usage_providers", "daily_budget_usd").unwrap());
assert_eq!(
    conn.query_row(
        "SELECT daily_budget_usd FROM usage_providers WHERE id = 'custom-metered'",
        [],
        |row| row.get::<_, Option<String>>(0),
    ).unwrap(),
    None,
);

conn.execute(
    "UPDATE usage_providers SET daily_budget_usd = '25.5'
     WHERE id = 'system-openai-api'",
    [],
).unwrap();
reconcile_system_providers(&mut conn).unwrap();
assert_eq!(
    conn.query_row(
        "SELECT daily_budget_usd FROM usage_providers
         WHERE id = 'system-openai-api'",
        [],
        |row| row.get::<_, Option<String>>(0),
    ).unwrap(),
    Some("25.5".to_string()),
);
```

Also assert a forced migration failure rolls back the column and `user_version`, and a database claiming v18 without the column fails completeness validation.

- [ ] **Step 2: Run the migration test and confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml database::tests::migration_v17_to_v18 --lib -- --nocapture
```

Expected: FAIL because schema v18 and `daily_budget_usd` do not exist.

- [ ] **Step 3: Implement the additive migration**

Create `budget_migration.rs` with these exact public functions:

```rust
use crate::database::Database;
use crate::error::AppError;
use rusqlite::Connection;

pub fn migrate_v17_to_v18(conn: &mut Connection) -> Result<(), AppError> {
    let transaction = conn.transaction()?;
    transaction.execute(
        "ALTER TABLE usage_providers ADD COLUMN daily_budget_usd TEXT",
        [],
    )?;
    Database::set_user_version(&transaction, 18)?;
    validate_schema_v18_complete(&transaction)?;
    transaction.commit()?;
    Ok(())
}

pub fn validate_schema_v18_complete(conn: &Connection) -> Result<(), AppError> {
    if !Database::has_column(conn, "usage_providers", "daily_budget_usd")? {
        return Err(AppError::Database(
            "incomplete schema v18: usage_providers.daily_budget_usd is missing".to_string(),
        ));
    }
    Ok(())
}
```

Export the module, change `SCHEMA_VERSION` to `18`, add the `17 => migrate_v17_to_v18` dispatch branch, and invoke `validate_schema_v18_complete` from the current-schema validator. Do not edit the system reconciliation update list; omitting the new column is what preserves it.

- [ ] **Step 4: Write failing DAO validation and preservation tests**

Cover canonical positive decimals, clearing, invalid inputs, subscription rejection, ordinary save preservation, and safe serialization:

```rust
for (raw, canonical) in [
    ("1", "1"),
    ("001.2500", "1.25"),
    ("0.000001", "0.000001"),
] {
    let view = db
        .set_provider_daily_budget("metered", Some(raw))
        .unwrap();
    assert_eq!(view.daily_budget_usd.as_deref(), Some(canonical));
}

for raw in ["", "0", "-1", "NaN", "inf", "1e999999"] {
    assert!(db
        .set_provider_daily_budget("metered", Some(raw))
        .is_err(), "{raw}");
}

assert!(db
    .set_provider_daily_budget("subscription", Some("20"))
    .is_err());
assert_eq!(
    db.set_provider_daily_budget("metered", None)
        .unwrap()
        .daily_budget_usd,
    None,
);

db.set_provider_daily_budget("metered", Some("9.5")).unwrap();
db.save_usage_provider(&changed_non_budget_input).unwrap();
assert_eq!(
    db.get_usage_provider("metered")
        .unwrap().unwrap().daily_budget_usd.as_deref(),
    Some("9.5"),
);
```

Serialize the returned view and assert it contains `dailyBudgetUsd` but does not contain a known sentinel inserted into `quota_config`.

- [ ] **Step 5: Implement canonicalization, storage, and safe projection**

Append `daily_budget_usd` to `PROVIDER_COLUMNS` so all existing indices stay stable, then read it at index 17. Add the field to stored/view types and populate only the safe field.

```rust
use rust_decimal::Decimal;
use std::str::FromStr;

fn canonicalize_daily_budget(raw: &str) -> Result<String, AppError> {
    let trimmed = raw.trim();
    let value = Decimal::from_str(trimmed)
        .map_err(|_| AppError::Message("invalid_daily_budget".to_string()))?;
    if value <= Decimal::ZERO {
        return Err(AppError::Message("invalid_daily_budget".to_string()));
    }
    Ok(value.normalize().to_string())
}

pub fn set_provider_daily_budget(
    &self,
    provider_id: &str,
    value: Option<&str>,
) -> Result<UsageProviderView, AppError> {
    let canonical = value.map(canonicalize_daily_budget).transpose()?;
    let mut conn = lock_conn!(self.conn);
    let transaction = conn.transaction()?;
    let billing_kind = transaction
        .query_row(
            "SELECT billing_kind FROM usage_providers WHERE id = ?1",
            [provider_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| AppError::Message("usage_provider_not_found".to_string()))?;
    if billing_kind != "metered" {
        return Err(AppError::Message(
            "daily_budget_requires_metered_provider".to_string(),
        ));
    }
    transaction.execute(
        "UPDATE usage_providers
         SET daily_budget_usd = ?2, updated_at = ?3
         WHERE id = ?1",
        params![provider_id, canonical, now_timestamp()?],
    )?;
    let stored = transaction.query_row(
        &format!("SELECT {PROVIDER_COLUMNS} FROM usage_providers WHERE id = ?1"),
        [provider_id],
        provider_from_row,
    )?;
    let view = provider_view(
        &transaction,
        &stored,
        source_bindings_for_provider(&transaction, provider_id)?,
        bindings_for_provider_on_conn(&transaction, provider_id)?,
    )?;
    transaction.commit()?;
    Ok(view)
}
```

`UsageProviderInput` remains unchanged. Its existing upsert must continue to omit `daily_budget_usd`.

- [ ] **Step 6: Preserve the field in backup/restore**

Add `daily_budget_usd: Option<String>` to the explicit usage-Provider backup row, select it by name, and include it in restore insert/update SQL. The round-trip test must prove both a value and null survive:

```rust
db.set_provider_daily_budget("backup-metered", Some("42.75"))
    .unwrap();
let archive = db.create_backup_for_test().unwrap();
let restored = Database::restore_backup_for_test(&archive).unwrap();
assert_eq!(
    restored
        .get_usage_provider("backup-metered")
        .unwrap().unwrap().daily_budget_usd.as_deref(),
    Some("42.75"),
);
```

Never add `quota_config` to any new frontend-facing structure.

- [ ] **Step 7: Run focused persistence tests**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml database::tests::migration_v17_to_v18 --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml database::dao::usage_providers::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml database::backup::tests --lib -- --nocapture
```

Expected: PASS, including reconciliation/save/backup preservation and the serialization redaction assertion.

- [ ] **Step 8: Commit the persistence slice**

```bash
git add src-tauri/src/usage/budget_migration.rs src-tauri/src/usage/mod.rs src-tauri/src/database/mod.rs src-tauri/src/database/schema.rs src-tauri/src/database/tests.rs src-tauri/src/database/backup.rs src-tauri/src/usage/domain.rs src-tauri/src/database/dao/usage_providers.rs
git commit -m "feat: persist provider daily usage budgets"
```

---

### Task 2: Lock the exact status model and local time windows

**Files:**

- Create: `src-tauri/src/usage/status.rs`
- Create: `src-tauri/src/usage/tray_snapshot.rs`
- Modify: `src-tauri/src/usage/mod.rs`
- Test: `src-tauri/src/usage/status.rs`
- Test: `src-tauri/src/usage/tray_snapshot.rs`

**Interfaces:**

- Produces: `UsageStatus::{Green, Yellow, Red, Unknown}` and `CostQuality::{Complete, Estimated, Partial, Unavailable}`.
- Produces: `classify_subscription`, `classify_metered`, and `worst_status`.
- Produces: `TrayUsageWindows::from_local_now(DateTime<Local>)`.
- Later tasks consume the serializable tray DTOs defined here.

- [ ] **Step 1: Write every boundary test before implementation**

Use exact Decimal strings so the thresholds cannot drift through floating-point rounding:

```rust
#[test]
fn subscription_remaining_boundaries_are_exact() {
    for (used, expected) in [
        ("49.999", UsageStatus::Green),
        ("50", UsageStatus::Yellow),
        ("79.999", UsageStatus::Yellow),
        ("80", UsageStatus::Yellow),
        ("80.001", UsageStatus::Red),
    ] {
        assert_eq!(
            classify_subscription(Some(used), None).status,
            expected,
            "{used}",
        );
    }
}

#[test]
fn metered_budget_boundaries_are_exact() {
    for (cost, expected) in [
        ("4.999", UsageStatus::Green),
        ("5", UsageStatus::Yellow),
        ("7.999", UsageStatus::Yellow),
        ("8", UsageStatus::Red),
    ] {
        assert_eq!(
            classify_metered(
                Some(cost),
                Some("10"),
                CostQuality::Complete,
            ).status,
            expected,
            "{cost}",
        );
    }
}

#[test]
fn partial_cost_only_proves_escalation() {
    assert_eq!(
        classify_metered(Some("4"), Some("10"), CostQuality::Partial).status,
        UsageStatus::Unknown,
    );
    assert_eq!(
        classify_metered(Some("5"), Some("10"), CostQuality::Partial).status,
        UsageStatus::Yellow,
    );
    assert_eq!(
        classify_metered(Some("8"), Some("10"), CostQuality::Partial).status,
        UsageStatus::Red,
    );
}

#[test]
fn unknown_never_erases_a_known_alert() {
    assert_eq!(
        worst_status([
            UsageStatus::Unknown,
            UsageStatus::Green,
            UsageStatus::Red,
            UsageStatus::Yellow,
        ]),
        UsageStatus::Red,
    );
}
```

Also cover clamp below 0/above 100, highest used window wins, malformed percentages, missing/invalid budgets, unavailable cost, exactly 20% remaining, and all-unknown aggregation.

- [ ] **Step 2: Run the status filter and confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml usage::status::tests --lib -- --nocapture
```

Expected: FAIL because `usage::status` does not exist.

- [ ] **Step 3: Implement the pure status module**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageStatus {
    Green,
    Yellow,
    Red,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CostQuality {
    Complete,
    Estimated,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceClassification {
    pub status: UsageStatus,
    pub used_percent: Option<String>,
    pub remaining_percent: Option<String>,
    pub consumed_percent: Option<String>,
    pub reason: Option<&'static str>,
}

pub fn classify_subscription(
    five_hour_used: Option<&str>,
    seven_day_used: Option<&str>,
) -> SourceClassification;

pub fn classify_metered(
    numeric_cost_usd: Option<&str>,
    daily_budget_usd: Option<&str>,
    quality: CostQuality,
) -> SourceClassification;

pub fn worst_status(values: impl IntoIterator<Item = UsageStatus>) -> UsageStatus;
```

Implement subscription classification with `Decimal::clamp(0, 100)`, take the maximum valid used value, compute `100 - used`, then compare remaining in the exact order `<20`, `<=50`, green. Metered classification computes `cost / budget * 100` and compares `>=80`, `>=50`, green; return unknown for absent budget, unavailable cost, invalid values, or partial below 50%.

- [ ] **Step 4: Write deterministic local-window tests**

Use `FixedOffset` test helpers and convert through a generic calendar function so DST can be tested without changing the process timezone:

```rust
let now = FixedOffset::west_opt(4 * 3600)
    .unwrap()
    .with_ymd_and_hms(2026, 11, 1, 12, 30, 0)
    .single()
    .unwrap();
let windows = TrayUsageWindows::from_zoned_now(now).unwrap();
assert_eq!(windows.today_start_at, 1_793_505_600);
assert_eq!(windows.end_at - windows.rolling_30_start_at, 30 * 86_400);
assert!(windows.next_local_midnight_at > windows.end_at);
```

Add spring-forward, fall-back, ordinary day, and half-open range assertions. The next midnight must be derived from the next local calendar date, never `today_start + 86_400`.

- [ ] **Step 5: Define safe tray DTOs and window calculation**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayUsageSnapshot {
    pub status: UsageStatus,
    pub generated_at: i64,
    pub last_success_at: Option<i64>,
    pub stale: bool,
    pub refresh_error: Option<String>,
    pub refresh_in_progress: bool,
    pub agents: Vec<TrayAgentUsageView>,
}

impl TrayUsageSnapshot {
    pub fn unknown(generated_at: i64) -> Self {
        Self {
            status: UsageStatus::Unknown,
            generated_at,
            last_success_at: None,
            stale: false,
            refresh_error: None,
            refresh_in_progress: false,
            agents: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayAgentUsageView {
    pub agent_module_id: String,
    pub name: String,
    pub sort_order: i64,
    pub status: UsageStatus,
    pub providers: Vec<TrayProviderUsageView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayProviderUsageView {
    pub provider_id: String,
    pub provider_name: String,
    pub system_preset_key: Option<String>,
    pub billing_kind: BillingKind,
    pub status: UsageStatus,
    pub warning_reason: Option<String>,
    pub subscription: Option<TraySubscriptionUsageView>,
    pub metered: Option<TrayMeteredUsageView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayQuotaWindowView {
    pub kind: String,
    pub used_percent: Option<String>,
    pub remaining_percent: Option<String>,
    pub resets_at: Option<String>,
    pub status: UsageStatus,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraySubscriptionUsageView {
    pub plan_label: Option<String>,
    pub windows: Vec<TrayQuotaWindowView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayMeteredUsageView {
    pub today_cost_usd: Option<String>,
    pub rolling_30_day_cost_usd: Option<String>,
    pub daily_budget_usd: Option<String>,
    pub budget_consumed_percent: Option<String>,
    pub total_tokens: u64,
    pub cost_quality: CostQuality,
}

pub struct TrayUsageWindows {
    pub today_start_at: i64,
    pub rolling_30_start_at: i64,
    pub end_at: i64,
    pub next_local_midnight_at: i64,
}
```

Keep reasons as fixed codes such as `invalid_quota_percent`, `cost_unavailable`, `partial_cost`, and `daily_budget_missing`. Do not copy `QuotaFetchState.last_error` into the DTO.

- [ ] **Step 6: Run and commit the pure model**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml usage::status::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml usage::tray_snapshot::tests --lib -- --nocapture
git add src-tauri/src/usage/status.rs src-tauri/src/usage/tray_snapshot.rs src-tauri/src/usage/mod.rs
git commit -m "feat: define tray usage status model"
```

Expected: all boundary/time tests PASS.

---

### Task 3: Build the side-effect-free Agent/Provider snapshot projector

**Files:**

- Create: `src-tauri/src/usage/aggregation.rs`
- Modify: `src-tauri/src/usage/dashboard.rs`
- Modify: `src-tauri/src/usage/tray_snapshot.rs`
- Test: `src-tauri/src/usage/aggregation.rs`
- Test: `src-tauri/src/usage/dashboard.rs`
- Test: `src-tauri/src/usage/tray_snapshot.rs`

**Interfaces:**

- Consumes: `UsageStatus`, `CostQuality`, `TrayUsageWindows`, `UsageProviderView.daily_budget_usd`.
- Produces: `aggregate_provider_range(&Database, agent_id, provider_id, product_group_id, start_at, end_at) -> Result<ProviderRangeAggregate, AppError>`.
- Produces: `TrayUsageProjector::project_at(DateTime<Local>) -> Result<TrayUsageSnapshot, AppError>`.

- [ ] **Step 1: Write failing shared-aggregation regression tests**

Create a proxy event and its session-log duplicate, plus estimated/unavailable rows. Assert exact Decimal totals, token totals, source counts, and half-open bounds:

```rust
let result = aggregate_provider_range(
    &db,
    "codex",
    "metered",
    "openai",
    1_000,
    2_000,
).unwrap();
assert_eq!(result.event_count, 3);
assert_eq!(result.input_tokens, 150);
assert_eq!(result.output_tokens, 75);
assert_eq!(result.total_cost_usd.as_deref(), Some("1.23456789"));
assert_eq!(result.cost_source_counts.upstream, 1);
assert_eq!(result.cost_source_counts.estimated, 1);
assert_eq!(result.cost_source_counts.unavailable, 1);
```

The duplicate session event and event at exactly `end_at` must be excluded.

- [ ] **Step 2: Run the aggregation filter and confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml usage::aggregation::tests --lib -- --nocapture
```

Expected: FAIL because the shared function does not exist.

- [ ] **Step 3: Extract the existing exact aggregation without semantic change**

Move the query, duplicate exclusion, checked u64 sums, Decimal parsing/summing, and `CostSourceCounts` mapping from `UsageDashboardService::aggregate_provider` into:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRangeAggregate {
    pub event_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub total_cost_usd: Option<String>,
    pub cost_source_counts: CostSourceCounts,
}

pub fn aggregate_provider_range(
    db: &Database,
    agent_module_id: &str,
    provider_id: &str,
    product_group_id: &str,
    start_at: i64,
    end_at: i64,
) -> Result<ProviderRangeAggregate, AppError>;
```

Make the dashboard call this function and then attach quota/fetch state exactly as before. Run its existing suite to prove no dashboard regression.

- [ ] **Step 4: Write projector tests for visibility, binding, cost quality, and worst status**

Seed visible/hidden/archived Agents; enabled/disabled bindings; enabled/disabled Providers; a session-only subscription binding; shared metered Providers; quota snapshots; and today/30-day events. Assert:

```rust
let snapshot = TrayUsageProjector::new(db.clone())
    .project_at(local_now)
    .unwrap();

assert_eq!(
    snapshot.agents.iter()
        .map(|agent| agent.agent_module_id.as_str())
        .collect::<Vec<_>>(),
    vec!["codex", "claude-code"],
);
assert_eq!(snapshot.status, UsageStatus::Red);
assert!(snapshot.agents.iter().all(|agent| {
    agent.providers.iter().all(|provider| {
        provider.provider_id != "disabled-provider"
    })
}));
assert!(find_provider(&snapshot, "claude-code", "subscription").is_some());
assert_eq!(
    find_provider(&snapshot, "codex", "metered")
        .unwrap().metered.as_ref().unwrap().rolling_30_day_cost_usd.as_deref(),
    Some("18.75"),
);
```

The session-only row must be included based on `binding.enabled`, not `effective_enabled`. Add explicit tests for:

- budget missing → visible, unknown for that source;
- zero events with no unavailable rows → numeric `0` and complete;
- all cost rows unavailable → unavailable and unclassified;
- mixed numeric/unavailable → partial, can escalate but cannot prove green;
- reset timestamp in the past → `reset_pending_refresh` reason;
- raw quota error/sentinel API key never appears in serialized snapshot.

- [ ] **Step 5: Implement projection using persisted state only**

```rust
pub struct TrayUsageProjector {
    db: Arc<Database>,
}

impl TrayUsageProjector {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub fn project_at(
        &self,
        now: DateTime<Local>,
    ) -> Result<TrayUsageSnapshot, AppError> {
        let generated_at = now.timestamp();
        let windows = TrayUsageWindows::from_local_now(now)
            .ok_or_else(|| AppError::Message(
                "invalid_tray_usage_windows".to_string(),
            ))?;
        let agents = self.db.list_agent_modules()?
            .into_iter()
            .filter(|agent| agent.visible && agent.archived_at.is_none())
            .collect::<Vec<_>>();
        let providers = self.db.list_usage_providers()?
            .into_iter()
            .filter(|provider| provider.enabled)
            .collect::<Vec<_>>();
        let bindings = self.db.list_agent_provider_bindings(None)?
            .into_iter()
            .filter(|binding| binding.enabled)
            .collect::<Vec<_>>();

        // Group binding/provider pairs by visible Agent, aggregate today's and
        // rolling-30-day ranges, attach persisted quota, then classify each row.
        // Sort Agents by (sort_order, id) and Providers by their existing DAO order.

        Ok(TrayUsageSnapshot {
            status: worst_status(projected_agent_statuses),
            generated_at,
            last_success_at: Some(generated_at),
            stale: false,
            refresh_error: None,
            refresh_in_progress: false,
            agents: projected_agents,
        })
    }
}
```

The two explanatory comments above are the only acceptable compression: implement them with focused private helpers `project_agent`, `project_subscription`, `project_metered`, and `cost_quality` in the same file. `cost_quality` is exact:

```rust
fn cost_quality(aggregate: &ProviderRangeAggregate) -> CostQuality {
    if aggregate.event_count == 0 {
        CostQuality::Complete
    } else if aggregate.cost_source_counts.unavailable == aggregate.event_count {
        CostQuality::Unavailable
    } else if aggregate.cost_source_counts.unavailable > 0 {
        CostQuality::Partial
    } else if aggregate.cost_source_counts.estimated > 0 {
        CostQuality::Estimated
    } else {
        CostQuality::Complete
    }
}
```

For a zero-event range, project cost `"0"`. Never use SQLite `REAL` or JavaScript arithmetic to classify USD.
`TrayMeteredUsageView.total_tokens` is the checked sum of input, output,
cache-read, and cache-creation tokens from the rolling-30-day aggregate, not only
today's range.

- [ ] **Step 6: Run focused projection and dashboard regression tests**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml usage::aggregation::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml usage::tray_snapshot::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml usage::dashboard::tests --lib -- --nocapture
```

Expected: PASS and the dashboard results remain byte-for-byte equivalent for existing fixtures.

- [ ] **Step 7: Commit the pure projector**

```bash
git add src-tauri/src/usage/aggregation.rs src-tauri/src/usage/dashboard.rs src-tauri/src/usage/tray_snapshot.rs
git commit -m "feat: project authoritative tray usage snapshot"
```

---

### Task 4: Add a cache-backed refresh service that preserves the last alert

> **Execution correction (2026-07-16):** the approved implementation uses an
> atomic CAS refresh lease plus a separate ordered commit/publish lock. Cache
> locks are released before callbacks; normal rebuilds preserve an active
> refresh; cancellation clears the atomic lease; and production samples its
> clock after source work and immediately before projection. Task 5, rather
> than Task 4, owns the concrete cancellable midnight scheduler. These rules
> supersede the simplified mutex/cache callback examples below.

**Files:**

- Create: `src-tauri/src/services/tray_usage.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Modify: `src-tauri/src/store.rs`
- Test: `src-tauri/src/services/tray_usage.rs`

**Interfaces:**

- Consumes: `TrayUsageProjector::project_at`, `QuotaService`, and `SessionUsageService`.
- Produces: `TrayUsageService::cached_snapshot`, `rebuild_from_persisted_at`, `refresh_at`, and `mark_refresh_failed_at`.
- Produces: a non-blocking refresh gate; repeated refreshes return the current `refresh_in_progress` snapshot.
- The service remains Tauri-independent. Callers provide a publish callback.

- [ ] **Step 1: Write failing cache/failure/concurrency tests**

Use the dependency-injected constructor so the service can deterministically
advance and inject failure without a Tauri runtime:

```rust
#[tokio::test]
async fn failed_refresh_keeps_the_entire_last_successful_snapshot() {
    let projections = Arc::new(std::sync::Mutex::new(VecDeque::from([
        Ok(snapshot(UsageStatus::Yellow, "old-provider-row")),
        Err(AppError::Message("sk-secret raw upstream failure".to_string())),
    ])));
    let project_snapshot: ProjectSnapshotFn = {
        let projections = projections.clone();
        Arc::new(move |_| projections.lock().unwrap().pop_front().unwrap())
    };
    let refresh_sources: RefreshSourcesFn =
        Arc::new(|| Box::pin(async { Ok(()) }));
    let service = TrayUsageService::new_with_dependencies(
        project_snapshot,
        refresh_sources,
    );

    let first = service
        .rebuild_from_persisted_at(local_time(1_000), |_| {})
        .await;
    assert_eq!(first.status, UsageStatus::Yellow);

    let failed = service
        .refresh_at(local_time(2_000), |_| {})
        .await;
    assert_eq!(failed.status, UsageStatus::Yellow);
    assert_eq!(failed.agents, first.agents);
    assert_eq!(failed.last_success_at, first.last_success_at);
    assert!(failed.stale);
    assert_eq!(
        failed.refresh_error.as_deref(),
        Some("tray_usage_refresh_failed"),
    );
    assert!(!serde_json::to_string(&failed).unwrap().contains("sk-secret"));
}

#[tokio::test]
async fn concurrent_refresh_does_not_start_a_second_collection() {
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let refresh_sources: RefreshSourcesFn = {
        let started = started.clone();
        let release = release.clone();
        let calls = calls.clone();
        Arc::new(move || {
            let started = started.clone();
            let release = release.clone();
            let calls = calls.clone();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                started.notify_one();
                release.notified().await;
                Ok(())
            })
        })
    };
    let service = Arc::new(TrayUsageService::new_with_dependencies(
        Arc::new(|_| Ok(snapshot(UsageStatus::Green, "row"))),
        refresh_sources,
    ));
    let first = tokio::spawn({
        let service = service.clone();
        async move { service.refresh_at(local_time(3_000), |_| {}).await }
    });
    started.notified().await;
    let second = service.refresh_at(local_time(3_001), |_| {}).await;
    assert!(second.refresh_in_progress);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    release.notify_one();
    first.await.unwrap();
}
```

Also test: first failure returns unknown/gray; successful rebuild clears stale/error; a persisted projection with no classifiable value keeps `last_success_at = None`; the publish callback receives `refresh_in_progress=true` before source work and a final false snapshot after it.

- [ ] **Step 2: Run the service test and confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml services::tray_usage::tests --lib -- --nocapture
```

Expected: FAIL because `TrayUsageService` does not exist.

- [ ] **Step 3: Implement the cache and atomic commit rules**

```rust
type ProjectSnapshotFn = Arc<
    dyn Fn(DateTime<Local>) -> Result<TrayUsageSnapshot, AppError>
        + Send
        + Sync,
>;
type RefreshSourcesFuture =
    Pin<Box<dyn Future<Output = Result<(), ()>> + Send>>;
type RefreshSourcesFn =
    Arc<dyn Fn() -> RefreshSourcesFuture + Send + Sync>;

pub struct TrayUsageService {
    project_snapshot: ProjectSnapshotFn,
    refresh_sources: RefreshSourcesFn,
    cache: tokio::sync::RwLock<TrayUsageSnapshot>,
    refresh_gate: tokio::sync::Mutex<()>,
}

impl TrayUsageService {
    pub fn new(
        db: Arc<Database>,
        quota_service: Arc<QuotaService>,
        session_usage_service: Arc<SessionUsageService>,
    ) -> Self {
        let projector = Arc::new(TrayUsageProjector::new(db.clone()));
        let project_snapshot: ProjectSnapshotFn = Arc::new(move |now| {
            projector.project_at(now)
        });
        let refresh_sources: RefreshSourcesFn = Arc::new(move || {
            let db = db.clone();
            let quota_service = quota_service.clone();
            let session_usage_service = session_usage_service.clone();
            Box::pin(async move {
                refresh_all_sources(
                    db,
                    quota_service,
                    session_usage_service,
                ).await
            })
        });
        Self::new_with_dependencies(project_snapshot, refresh_sources)
    }

    pub(crate) fn new_with_dependencies(
        project_snapshot: ProjectSnapshotFn,
        refresh_sources: RefreshSourcesFn,
    ) -> Self {
        Self {
            project_snapshot,
            refresh_sources,
            cache: tokio::sync::RwLock::new(TrayUsageSnapshot::unknown(0)),
            refresh_gate: tokio::sync::Mutex::new(()),
        }
    }

    pub async fn cached_snapshot(&self) -> TrayUsageSnapshot {
        self.cache.read().await.clone()
    }

    pub async fn rebuild_from_persisted_at<F>(
        &self,
        now: DateTime<Local>,
        publish: F,
    ) -> TrayUsageSnapshot
    where
        F: Fn(&TrayUsageSnapshot),
    {
        match (self.project_snapshot)(now) {
            Ok(candidate) => self.commit_success(candidate, &publish).await,
            Err(_) => self
                .mark_refresh_failed_at(now.timestamp(), &publish)
                .await,
        }
    }

    pub async fn mark_refresh_failed_at<F>(
        &self,
        generated_at: i64,
        publish: F,
    ) -> TrayUsageSnapshot
    where
        F: Fn(&TrayUsageSnapshot),
    {
        let mut snapshot = self.cache.write().await;
        snapshot.generated_at = generated_at;
        snapshot.stale = true;
        snapshot.refresh_in_progress = false;
        snapshot.refresh_error = Some("tray_usage_refresh_failed".to_string());
        publish(&snapshot);
        snapshot.clone()
    }

    async fn commit_success<F>(
        &self,
        mut candidate: TrayUsageSnapshot,
        publish: &F,
    ) -> TrayUsageSnapshot
    where
        F: Fn(&TrayUsageSnapshot),
    {
        let previous_last_success = self.cache.read().await.last_success_at;
        candidate.last_success_at =
            if candidate.status == UsageStatus::Unknown {
                previous_last_success
            } else {
                Some(candidate.generated_at)
            };
        candidate.stale = false;
        candidate.refresh_error = None;
        candidate.refresh_in_progress = false;
        let mut cache = self.cache.write().await;
        *cache = candidate;
        publish(&cache);
        cache.clone()
    }
}
```

`commit_success` sets `last_success_at = Some(generated_at)` only when the projection contains at least one classifiable source; it clears stale/error and atomically replaces the whole cache. It never merges DTO strings from an error.

- [ ] **Step 4: Implement source refresh as an all-or-last-success transaction**

The refresh cycle gathers every enabled subscription quota source and every enabled Provider with a session source. Quota refreshes remain async; filesystem session scans run in `spawn_blocking`. Any call error or non-empty session `errors` makes the cycle fail:

```rust
pub async fn refresh_at<F>(
    &self,
    now: DateTime<Local>,
    publish: F,
) -> TrayUsageSnapshot
where
    F: Fn(&TrayUsageSnapshot),
{
    let Ok(_guard) = self.refresh_gate.try_lock() else {
        return self.cached_snapshot().await;
    };

    {
        let mut snapshot = self.cache.write().await;
        snapshot.refresh_in_progress = true;
        snapshot.refresh_error = None;
        publish(&snapshot);
    }

    if (self.refresh_sources)().await.is_err() {
        self.mark_refresh_failed_at(now.timestamp(), publish).await
    } else {
        self.rebuild_from_persisted_at(now, publish).await
    }
}
```

Do not log the returned upstream error string. Log only `tray usage source refresh failed` at debug/warn level.
`refresh_all_sources` lists enabled Providers, awaits every eligible
subscription quota refresh, runs each session scan in `spawn_blocking`, and
returns `Err(())` when any call fails or any session result has non-empty
`errors`. Production `new` and tests both use
`new_with_dependencies(ProjectSnapshotFn, RefreshSourcesFn)`, so dependency
injection remains real production structure rather than a test-only API.

- [ ] **Step 5: Own the service in `AppState`**

Construct quota/session services once, then pass their same `Arc` values into the tray service:

```rust
pub struct AppState {
    // existing fields...
    pub tray_usage_service: Arc<TrayUsageService>,
    quota_scheduler: Mutex<Option<QuotaSchedulerHandle>>,
    midnight_scheduler: Mutex<Option<TrayUsageSchedulerHandle>>,
}

let quota_service = Arc::new(QuotaService::new(db.clone()));
let session_usage_service = Arc::new(SessionUsageService::new(db.clone()));
let tray_usage_service = Arc::new(TrayUsageService::new(
    db.clone(),
    quota_service.clone(),
    session_usage_service.clone(),
));
```

Keep all existing test constructors routed through `new_with_services` so no test needs to initialize the field separately.

- [ ] **Step 6: Run and commit the service**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml services::tray_usage::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml store::tests --lib -- --nocapture
git add src-tauri/src/services/tray_usage.rs src-tauri/src/services/mod.rs src-tauri/src/store.rs
git commit -m "feat: cache and refresh tray usage state"
```

Expected: PASS; no test output contains the injected secret sentinel.

---

### Task 5: Publish the shared snapshot to commands, events, and status-dot assets

**Files:**

- Create: `src-tauri/src/commands/tray_usage.rs`
- Create: `src-tauri/src/tray_status.rs`
- Create: `src-tauri/icons/tray/macos/status_green.png`
- Create: `src-tauri/icons/tray/macos/status_yellow.png`
- Create: `src-tauri/icons/tray/macos/status_red.png`
- Create: `src-tauri/icons/tray/macos/status_unknown.png`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/usage_events.rs`
- Modify: `src-tauri/src/usage/quota.rs`
- Modify: `src-tauri/src/store.rs`
- Modify: `src-tauri/src/commands/usage_dashboard.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/tray_status.rs`
- Test: `src-tauri/src/usage_events.rs`
- Test: `src-tauri/src/commands/tray_usage.rs`

**Interfaces:**

- Produces commands `get_tray_usage_snapshot`, `refresh_tray_usage`, and `set_provider_daily_budget`.
- Produces event `tray-usage-updated` whose payload is exactly `TrayUsageSnapshot`.
- Produces `publish_tray_usage(app, snapshot)`; it updates icon, tooltip, and event from the same `snapshot.status`.
- Produces quota-scheduler callback and a local-calendar midnight rebuild loop.

- [ ] **Step 1: Write failing command and publisher tests**

```rust
#[tokio::test]
async fn budget_command_updates_provider_then_publishes_rebuilt_snapshot() {
    let state = test_state();
    let published = Arc::new(Mutex::new(Vec::new()));
    let view = set_provider_daily_budget_test_hook(
        &state,
        "metered",
        Some("10.00"),
        |snapshot| published.lock().unwrap().push(snapshot.clone()),
    ).await.unwrap();
    assert_eq!(view.daily_budget_usd.as_deref(), Some("10"));
    assert_eq!(published.lock().unwrap().last().unwrap().status, UsageStatus::Green);
}

#[test]
fn tooltip_is_accessible_and_contains_no_usage_payload() {
    assert_eq!(
        tray_status_tooltip(UsageStatus::Green),
        "LLM Usage Bar — Usage healthy",
    );
    assert_eq!(
        tray_status_tooltip(UsageStatus::Yellow),
        "LLM Usage Bar — Usage warning",
    );
    assert_eq!(
        tray_status_tooltip(UsageStatus::Red),
        "LLM Usage Bar — Usage critical",
    );
    assert_eq!(
        tray_status_tooltip(UsageStatus::Unknown),
        "LLM Usage Bar — Data unavailable",
    );
}
```

Add tests that `notify_log_recorded` coalesces snapshot rebuilds, metadata mutations notify, a quota scheduler failure calls the stale path, and the publisher emits no `quota_config`/credential sentinel.

- [ ] **Step 2: Run focused tests and confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml commands::tray_usage::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml tray_status::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml usage_events::tests --lib -- --nocapture
```

Expected: FAIL because commands, publisher, and assets do not exist.

- [ ] **Step 3: Generate and inspect the four real PNG status assets**

Use the built-in Image Gen tool for each source raster with this exact art direction, one asset per call:

```text
A single perfectly circular macOS status indicator centered on a plain neutral
background, flat solid fill, no outline, no shadow, no glow, no text, no icon,
geometrically crisp. Circle color: #34C759.
```

Repeat with `#FFCC00`, `#FF3B30`, and `#8E8E93`. Then load the bundled workspace image runtime, remove the neutral background, crop/resize to an 18×18 RGBA canvas, and center an antialiased 10×10 circle. Save to the four exact paths above. This post-processing is required because Image Gen output is not transparent.

Inspect every result with `view_image` and verify metadata:

```bash
sips -g pixelWidth -g pixelHeight -g hasAlpha src-tauri/icons/tray/macos/status_green.png
sips -g pixelWidth -g pixelHeight -g hasAlpha src-tauri/icons/tray/macos/status_yellow.png
sips -g pixelWidth -g pixelHeight -g hasAlpha src-tauri/icons/tray/macos/status_red.png
sips -g pixelWidth -g pixelHeight -g hasAlpha src-tauri/icons/tray/macos/status_unknown.png
```

Expected for each: width 18, height 18, alpha present. The visible circle is approximately 10 pixels and remains crisp at 1×/2× menu-bar display.

- [ ] **Step 4: Implement atomic icon/tooltip mapping with fallback**

```rust
pub const EVENT_TRAY_USAGE_UPDATED: &str = "tray-usage-updated";

pub fn tray_status_tooltip(status: UsageStatus) -> &'static str {
    match status {
        UsageStatus::Green => "LLM Usage Bar — Usage healthy",
        UsageStatus::Yellow => "LLM Usage Bar — Usage warning",
        UsageStatus::Red => "LLM Usage Bar — Usage critical",
        UsageStatus::Unknown => "LLM Usage Bar — Data unavailable",
    }
}

#[cfg(target_os = "macos")]
fn status_icon_bytes(status: UsageStatus) -> &'static [u8] {
    match status {
        UsageStatus::Green => include_bytes!("../icons/tray/macos/status_green.png"),
        UsageStatus::Yellow => include_bytes!("../icons/tray/macos/status_yellow.png"),
        UsageStatus::Red => include_bytes!("../icons/tray/macos/status_red.png"),
        UsageStatus::Unknown => include_bytes!("../icons/tray/macos/status_unknown.png"),
    }
}

pub fn update_tray_status_icon(app: &AppHandle, status: UsageStatus) {
    let Some(tray) = app.tray_by_id(crate::tray::TRAY_ID) else {
        return;
    };
    #[cfg(target_os = "macos")]
    match Image::from_bytes(status_icon_bytes(status)) {
        Ok(icon) => {
            if let Err(error) = tray.set_icon_with_as_template(Some(icon), false) {
                log::warn!("failed to update tray status icon: {error}");
            }
        }
        Err(error) => {
            log::warn!("failed to decode tray status icon: {error}");
        }
    }
    if let Err(error) = tray.set_tooltip(Some(tray_status_tooltip(status))) {
        log::warn!("failed to update tray tooltip: {error}");
    }
}
```

On decode/set failure, do not call `set_icon(None)`; the previous icon remains.
In `lib.rs`, initialize the macOS builder with
`Image::from_bytes(status_icon_bytes(UsageStatus::Unknown))` and
`icon_as_template(false)` instead of the legacy template glyph. If decoding the
initial asset fails, retain `macos_tray_icon()` so the status item still exists;
all later successful updates remain non-template.

- [ ] **Step 5: Add focused commands and one publisher**

```rust
pub fn publish_tray_usage(app: &AppHandle, snapshot: &TrayUsageSnapshot) {
    crate::tray_status::update_tray_status_icon(app, snapshot.status);
    if let Err(error) = app.emit(EVENT_TRAY_USAGE_UPDATED, snapshot.clone()) {
        log::warn!("failed to emit tray usage update: {error}");
    }
}

#[tauri::command]
pub async fn get_tray_usage_snapshot(
    state: State<'_, AppState>,
) -> Result<TrayUsageSnapshot, AppError> {
    Ok(state.tray_usage_service.cached_snapshot().await)
}

#[tauri::command]
pub async fn refresh_tray_usage(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<TrayUsageSnapshot, AppError> {
    Ok(state.tray_usage_service
        .refresh_at(Local::now(), |snapshot| publish_tray_usage(&app, snapshot))
        .await)
}

#[tauri::command]
pub async fn set_provider_daily_budget(
    app: AppHandle,
    state: State<'_, AppState>,
    provider_id: String,
    daily_budget_usd: Option<String>,
) -> Result<UsageProviderView, AppError> {
    let view = state.db.set_provider_daily_budget(
        &provider_id,
        daily_budget_usd.as_deref(),
    )?;
    let snapshot = state.tray_usage_service
        .rebuild_from_persisted_at(Local::now(), |snapshot| {
            publish_tray_usage(&app, snapshot)
        }).await;
    crate::usage_events::notify_dashboard_invalidated();
    debug_assert_eq!(snapshot.status, state.tray_usage_service.cached_snapshot().await.status);
    Ok(view)
}
```

Expose a `set_provider_daily_budget_test_hook` containing the body above and
accepting `impl Fn(&TrayUsageSnapshot)`; the Tauri command delegates to it with
`publish_tray_usage`. Tests never need a mock `AppHandle`.

Export/register all three commands in `commands/mod.rs` and `generate_handler!`.

- [ ] **Step 6: Wire every rebuild trigger**

Change `notify_dashboard_invalidated` and the debounced log path to schedule one async persisted-state rebuild through `AppHandle.state::<AppState>()`. Add missing notifications after:

- `save_dashboard_module` and `set_dashboard_module_visibility`;
- `save_usage_provider` and `set_usage_provider_enabled`;
- all binding mutations; and
- the budget mutation.

Change the quota scheduler to accept an after-cycle callback:

```rust
pub type QuotaCycleCallback =
    Arc<dyn Fn(QuotaSchedulerCycle) + Send + Sync + 'static>;

pub fn start_scheduler(
    self: Arc<Self>,
    after_cycle: QuotaCycleCallback,
) -> QuotaSchedulerHandle {
    // Existing interval/select loop remains unchanged.
    // After every completed cycle, invoke after_cycle(cycle) exactly once.
}
```

The production callback marks the tray snapshot stale when `cycle.errors` is non-empty; otherwise it rebuilds from persisted data. Move `start_quota_scheduler` until after `app.manage(app_state)` so the callback can resolve state safely.

Start one cancellable midnight task. It recomputes `TrayUsageWindows::from_local_now(Local::now())` at least every 15 minutes, sleeps only until the lesser of the next midnight or 15 minutes, rebuilds when `now >= next_local_midnight_at`, and derives the next boundary again. Never sleep a fixed 86,400 seconds.

- [ ] **Step 7: Build the persisted snapshot at startup**

Immediately after state is managed and `usage_events::init` has an `AppHandle`:

```rust
let handle = app.handle().clone();
tauri::async_runtime::spawn(async move {
    let state = handle.state::<AppState>();
    state.tray_usage_service
        .rebuild_from_persisted_at(Local::now(), |snapshot| {
            publish_tray_usage(&handle, snapshot)
        })
        .await;
});
```

The initial bundled gray icon remains until this task completes; no startup path removes the tray item.

- [ ] **Step 8: Run and commit command/event/icon integration**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml commands::tray_usage::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml tray_status::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml usage_events::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml usage::quota::tests --lib -- --nocapture
git add src-tauri/src/commands/tray_usage.rs src-tauri/src/commands/mod.rs src-tauri/src/tray_status.rs src-tauri/icons/tray/macos/status_green.png src-tauri/icons/tray/macos/status_yellow.png src-tauri/icons/tray/macos/status_red.png src-tauri/icons/tray/macos/status_unknown.png src-tauri/src/usage_events.rs src-tauri/src/usage/quota.rs src-tauri/src/store.rs src-tauri/src/commands/usage_dashboard.rs src-tauri/src/lib.rs
git commit -m "feat: publish tray usage status"
```

Expected: PASS; icon asset tests decode all four 18×18 RGBA files.

---

### Task 6: Implement the macOS custom popover and two-window lifecycle

**Files:**

- Create: `src-tauri/src/tray_popover.rs`
- Create: `src-tauri/capabilities/tray-popover.json`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock` (only the feature-resolved graph produced by the Rust command)
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/tray.rs`
- Modify: `src-tauri/src/commands/tray_usage.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Test: `src-tauri/src/tray_popover.rs`
- Test: `src-tauri/src/lib.rs`

**Interfaces:**

- Produces: `TRAY_POPOVER_LABEL`, `classify_tray_click`, `calculate_popover_position`, `toggle`, `hide`.
- Produces serializable `MainWindowDestination::{Usage, ProviderBudget}`.
- Produces commands `hide_tray_popover`, `open_main_from_tray`, `take_pending_main_window_destination`, and `quit_from_tray`.
- Preserves the existing right-click `create_tray_menu` and all current menu IDs/actions.

- [ ] **Step 1: Write click and physical-position tests**

```rust
#[test]
fn only_left_down_toggles_and_right_down_hides() {
    assert_eq!(
        classify_tray_click(MouseButton::Left, MouseButtonState::Down),
        TrayClickAction::TogglePopover,
    );
    assert_eq!(
        classify_tray_click(MouseButton::Left, MouseButtonState::Up),
        TrayClickAction::Ignore,
    );
    assert_eq!(
        classify_tray_click(MouseButton::Right, MouseButtonState::Down),
        TrayClickAction::HidePopover,
    );
}

#[test]
fn popover_is_centered_below_anchor_and_clamped_to_work_area() {
    let position = calculate_popover_position(
        physical_rect(1900, 0, 22, 24),
        PhysicalSize::new(760, 1040),
        physical_rect(0, 0, 1920, 1080),
    );
    assert_eq!(position.x, 1160);
    assert_eq!(position.y, 32);
}
```

Add left/right/top/bottom clamps, negative secondary-monitor origin, 1× and 2× scale, and an anchor wider than the popup.

- [ ] **Step 2: Run the popover filter and confirm red**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml tray_popover::tests --lib -- --nocapture
```

Expected: FAIL because `tray_popover` does not exist.

- [ ] **Step 3: Implement click classification and monitor-safe positioning**

```rust
pub const TRAY_POPOVER_LABEL: &str = "tray-popover";
const POPOVER_WIDTH: f64 = 380.0;
const POPOVER_HEIGHT: f64 = 520.0;
const POPOVER_GAP_PHYSICAL: i32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayClickAction {
    TogglePopover,
    HidePopover,
    Ignore,
}

pub fn classify_tray_click(
    button: MouseButton,
    state: MouseButtonState,
) -> TrayClickAction {
    match (button, state) {
        (MouseButton::Left, MouseButtonState::Down) => {
            TrayClickAction::TogglePopover
        }
        (MouseButton::Right, MouseButtonState::Down) => {
            TrayClickAction::HidePopover
        }
        _ => TrayClickAction::Ignore,
    }
}

pub fn calculate_popover_position(
    anchor: PhysicalRect<i32, u32>,
    popup_size: PhysicalSize<u32>,
    work_area: PhysicalRect<i32, u32>,
) -> PhysicalPosition<i32> {
    let centered_x = anchor.position.x
        + (anchor.size.width as i32 - popup_size.width as i32) / 2;
    let below_y = anchor.position.y
        + anchor.size.height as i32
        + POPOVER_GAP_PHYSICAL;
    let min_x = work_area.position.x;
    let min_y = work_area.position.y;
    let max_x = min_x + work_area.size.width as i32 - popup_size.width as i32;
    let max_y = min_y + work_area.size.height as i32 - popup_size.height as i32;
    PhysicalPosition::new(
        centered_x.clamp(min_x, max_x.max(min_x)),
        below_y.clamp(min_y, max_y.max(min_y)),
    )
}
```

Keep event rectangle and final `set_position` in physical pixels. Use logical size only when constructing the 380×520 window.

- [ ] **Step 4: Lazily create, position, show, and hide the webview**

Enable Tauri's existing `macos-private-api` feature (no new crate/version) so
the borderless webview can have genuinely transparent rounded corners:

```toml
tauri = { version = "2.11.5", features = [
  "tray-icon",
  "protocol-asset",
  "image-png",
  "macos-private-api"
] }
```

```rust
pub fn ensure_window(app: &AppHandle) -> Result<WebviewWindow, AppError> {
    if let Some(window) = app.get_webview_window(TRAY_POPOVER_LABEL) {
        return Ok(window);
    }
    WebviewWindowBuilder::new(
        app,
        TRAY_POPOVER_LABEL,
        WebviewUrl::App("index.html".into()),
    )
    .title("LLM Usage Bar")
    .inner_size(POPOVER_WIDTH, POPOVER_HEIGHT)
    .visible(false)
    .transparent(true)
    .decorations(false)
    .resizable(false)
    .always_on_top(true)
    .accept_first_mouse(true)
    .build()
    .map_err(AppError::from)
}

pub fn toggle(app: &AppHandle, anchor: Rect) -> Result<(), AppError> {
    let window = ensure_window(app)?;
    if window.is_visible()? {
        return hide(app);
    }
    let provisional_scale = app.primary_monitor()?
        .map(|monitor| monitor.scale_factor())
        .unwrap_or(1.0);
    let provisional_position =
        anchor.position.to_physical::<f64>(provisional_scale);
    let provisional_size =
        anchor.size.to_physical::<f64>(provisional_scale);
    let anchor_center = PhysicalPosition::new(
        provisional_position.x + provisional_size.width / 2.0,
        provisional_position.y + provisional_size.height / 2.0,
    );
    let monitor = app.monitor_from_point(anchor_center.x, anchor_center.y)?
        .or(app.primary_monitor()?)
        .ok_or_else(|| AppError::Message("tray_monitor_unavailable".to_string()))?;
    let scale = monitor.scale_factor();
    let popup = PhysicalSize::new(
        (POPOVER_WIDTH * scale).round() as u32,
        (POPOVER_HEIGHT * scale).round() as u32,
    );
    let physical_anchor = PhysicalRect {
        position: anchor.position.to_physical::<i32>(scale),
        size: anchor.size.to_physical::<u32>(scale),
    };
    window.set_position(calculate_popover_position(
        physical_anchor,
        popup,
        monitor.work_area(),
    ))?;
    window.show()?;
    window.set_focus()?;
    let _ = app.emit_to(TRAY_POPOVER_LABEL, "tray-popover-shown", ());
    Ok(())
}

pub fn hide(app: &AppHandle) -> Result<(), AppError> {
    if let Some(window) = app.get_webview_window(TRAY_POPOVER_LABEL) {
        window.hide()?;
    }
    Ok(())
}
```

Creation failure is logged and returned from left click, but the tray/native menu remains alive.

- [ ] **Step 5: Route tray events without breaking other platforms**

On macOS set `show_menu_on_left_click(false)`. Match only `Click` and only `Down`:

```rust
#[cfg(target_os = "macos")]
match classify_tray_click(button, button_state) {
    TrayClickAction::TogglePopover => {
        if let Err(error) = tray_popover::toggle(app, rect) {
            log::warn!("failed to toggle tray popover: {error}");
        }
    }
    TrayClickAction::HidePopover => {
        let _ = tray_popover::hide(app);
        // Do not consume or replace the existing right-click native menu.
    }
    TrayClickAction::Ignore => {}
}
```

Windows/Linux keep `show_menu_on_left_click(true)` and the existing event path. Retain hover-triggered soft usage refresh only if it does not open a window.

- [ ] **Step 6: Separate `tray-popover` window events from `main`**

At the top of the global handler:

```rust
if window.label() == TRAY_POPOVER_LABEL {
    match event {
        WindowEvent::Focused(false) => {
            let _ = window.hide();
        }
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            let _ = window.hide();
        }
        _ => {}
    }
    return;
}
```

Only `main` can enter the existing close-to-tray/exit logic. Configure window-state persistence with `with_denylist(&[TRAY_POPOVER_LABEL])` so the popup never overwrites main geometry.

- [ ] **Step 7: Add minimum capability and typed main navigation**

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "tray-popover",
  "description": "Minimum capability for the tray usage popover",
  "windows": ["tray-popover"],
  "permissions": ["core:default"]
}
```

Define the shared Rust destination and pending slot:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MainWindowDestination {
    Usage { agent_module_id: Option<String> },
    ProviderBudget { provider_id: Option<String> },
}

static PENDING_MAIN_DESTINATION: OnceLock<Mutex<Option<MainWindowDestination>>> =
    OnceLock::new();

pub fn open_main_window(
    app: &AppHandle,
    destination: MainWindowDestination,
) -> Result<(), AppError> {
    hide(app)?;
    PENDING_MAIN_DESTINATION
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| AppError::Message("main_navigation_unavailable".to_string()))?
        .replace(destination.clone());
    crate::tray::apply_tray_policy(app, true);
    crate::lightweight::exit_lightweight_mode(app)
        .map_err(AppError::Message)?;
    let main = app.get_webview_window("main")
        .ok_or_else(|| AppError::Message("main_window_unavailable".to_string()))?;
    main.show()?;
    main.unminimize()?;
    main.set_focus()?;
    let _ = app.emit_to("main", "main-window-navigate", ());
    Ok(())
}
```

`take_pending_main_window_destination` atomically `take()`s the slot. `quit_from_tray` calls `app.exit(0)`. `hide_tray_popover` only hides. Register all commands.

- [ ] **Step 8: Make macOS launch menu-bar-first and restore Accessory policy**

On a standard macOS launch, keep `main` hidden and apply Accessory policy after setup, regardless of legacy `silent_startup=false`. Existing explicit native `show_main`, macOS reopen, `Open details`, and `Settings` use the unified `open_main_window` path.

For main close, keep the existing close-to-tray behavior and apply Accessory policy. Start a lightweight 300ms main-window visibility monitor only while policy is Regular:

```rust
if main.is_minimized().unwrap_or(false) {
    let _ = main.hide();
    let _ = crate::tray::apply_tray_policy(&app, false);
}
```

This covers macOS native minimize because Tauri 2.11.5 has no public `Minimized` window event. Stop the monitor at application exit; it must not poll in tests or on Windows/Linux.

- [ ] **Step 9: Run backend lifecycle tests and commit**

```bash
pnpm rust -- test --manifest-path src-tauri/Cargo.toml tray_popover::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml tray::tests --lib -- --nocapture
pnpm rust -- test --manifest-path src-tauri/Cargo.toml tests::no_code_keeps_app_alive_in_tray --lib -- --nocapture
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/tray_popover.rs src-tauri/capabilities/tray-popover.json src-tauri/src/lib.rs src-tauri/src/tray.rs src-tauri/src/commands/tray_usage.rs src-tauri/src/commands/mod.rs
git commit -m "feat: add macOS tray usage popover window"
```

Expected: PASS; tests prove left-down/right-down separation, focus-loss hiding, main/popover event routing, and destination serialization.

---

### Task 7: Add the dedicated renderer surface and cache/event data bridge

**Files:**

- Create: `src/types/trayUsage.ts`
- Create: `src/lib/api/trayUsage.ts`
- Create: `src/lib/query/trayUsage.ts`
- Create: `src/windowSurface.tsx`
- Create: `src/windowSurface.test.tsx`
- Create: `src/lib/query/trayUsage.test.tsx`
- Modify: `src/main.tsx`
- Modify: `tests/msw/tauriMocks.ts`
- Modify: `tests/msw/handlers.ts`

**Interfaces:**

- Consumes: Rust `TrayUsageSnapshot` and command/event names from Tasks 5–6.
- Produces: exact TypeScript DTO mirror and typed IPC actions.
- Produces: `useTrayUsageSnapshot`, `useRefreshTrayUsage`, and `useTrayUsageEventBridge`.
- Produces: `WindowSurface` that renders only `TrayUsagePopover` for `tray-popover`.

- [ ] **Step 1: Write failing DTO/API/query event tests**

Add a complete MSW fixture with one subscription and one metered Provider. Assert command names/arguments and direct cache replacement:

```tsx
it("publishes an event payload directly into the snapshot cache", async () => {
  const wrapper = createQueryWrapper();
  const { result } = renderHook(
    () => {
      useTrayUsageEventBridge();
      return useTrayUsageSnapshot();
    },
    { wrapper },
  );
  await waitFor(() => expect(result.current.data?.status).toBe("green"));

  emitTauriEvent("tray-usage-updated", {
    ...trayUsageSnapshotFixture,
    status: "red",
    generatedAt: 2_000,
  });

  await waitFor(() => expect(result.current.data?.status).toBe("red"));
  expect(tauriInvoke).toHaveBeenCalledTimes(1);
});

it("deduplicates refresh while the mutation or snapshot is refreshing", async () => {
  const { result } = renderHook(() => useRefreshTrayUsage(), {
    wrapper: createQueryWrapper(),
  });
  result.current.mutate();
  result.current.mutate();
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(commandCalls("refresh_tray_usage")).toHaveLength(1);
});
```

The fixture serialization must use decimal strings for percentages/USD and contain no `quotaConfig` property.

- [ ] **Step 2: Write failing window-surface tests**

```tsx
it("renders only the popover surface for tray-popover", () => {
  render(<WindowSurface windowLabel="tray-popover" />);
  expect(screen.getByTestId("tray-usage-popover")).toBeInTheDocument();
  expect(screen.queryByTestId("main-app")).not.toBeInTheDocument();
});

it("renders the existing app for main and unknown labels", () => {
  const { rerender } = render(<WindowSurface windowLabel="main" />);
  expect(screen.getByTestId("main-app")).toBeInTheDocument();
  rerender(<WindowSurface windowLabel="future-main-label" />);
  expect(screen.getByTestId("main-app")).toBeInTheDocument();
});
```

Mock the two surfaces as test components in `windowSurface.test.tsx`; the pure branch must not require a global Tauri window mock.

- [ ] **Step 3: Run the renderer tests and confirm red**

```bash
pnpm test:unit src/lib/query/trayUsage.test.tsx
pnpm test:unit src/windowSurface.test.tsx
```

Expected: FAIL because tray types/query/surface do not exist.

- [ ] **Step 4: Define the exact TypeScript contract**

```ts
export type TrayUsageStatus = "green" | "yellow" | "red" | "unknown";
export type TrayCostQuality =
  | "complete"
  | "estimated"
  | "partial"
  | "unavailable";

export interface TrayUsageSnapshot {
  status: TrayUsageStatus;
  generatedAt: number;
  lastSuccessAt: number | null;
  stale: boolean;
  refreshError: string | null;
  refreshInProgress: boolean;
  agents: TrayAgentUsageView[];
}

export interface TrayAgentUsageView {
  agentModuleId: string;
  name: string;
  sortOrder: number;
  status: TrayUsageStatus;
  providers: TrayProviderUsageView[];
}

export interface TrayProviderUsageView {
  providerId: string;
  providerName: string;
  systemPresetKey: string | null;
  billingKind: "subscription" | "metered";
  status: TrayUsageStatus;
  warningReason: string | null;
  subscription: TraySubscriptionUsageView | null;
  metered: TrayMeteredUsageView | null;
}

export interface TrayQuotaWindowView {
  kind: "five_hour" | "seven_day";
  usedPercent: string | null;
  remainingPercent: string | null;
  resetsAt: string | null;
  status: TrayUsageStatus;
  unavailableReason: string | null;
}

export interface TraySubscriptionUsageView {
  planLabel: string | null;
  windows: TrayQuotaWindowView[];
}

export interface TrayMeteredUsageView {
  todayCostUsd: string | null;
  rolling30DayCostUsd: string | null;
  dailyBudgetUsd: string | null;
  budgetConsumedPercent: string | null;
  totalTokens: number;
  costQuality: TrayCostQuality;
}

export type MainWindowDestination =
  | { kind: "usage"; agentModuleId: string | null }
  | { kind: "providerBudget"; providerId: string | null };
```

Property names must match Rust serde output exactly; do not add a frontend-only alternate snapshot shape.

- [ ] **Step 5: Implement typed IPC and React Query**

```ts
export const getTrayUsageSnapshot = () =>
  invoke<TrayUsageSnapshot>("get_tray_usage_snapshot");

let refreshInFlight: Promise<TrayUsageSnapshot> | null = null;
export const refreshTrayUsage = () => {
  if (!refreshInFlight) {
    refreshInFlight = invoke<TrayUsageSnapshot>("refresh_tray_usage")
      .finally(() => {
        refreshInFlight = null;
      });
  }
  return refreshInFlight;
};

export const hideTrayPopover = () =>
  invoke<void>("hide_tray_popover");

export const openMainFromTray = (destination: MainWindowDestination) =>
  invoke<void>("open_main_from_tray", { destination });

export const takePendingMainWindowDestination = () =>
  invoke<MainWindowDestination | null>(
    "take_pending_main_window_destination",
  );

export const quitFromTray = () => invoke<void>("quit_from_tray");
```

```ts
export const trayUsageKeys = {
  all: ["tray-usage"] as const,
  snapshot: () => [...trayUsageKeys.all, "snapshot"] as const,
};

export function useTrayUsageSnapshot() {
  return useQuery({
    queryKey: trayUsageKeys.snapshot(),
    queryFn: getTrayUsageSnapshot,
    staleTime: Infinity,
    refetchOnWindowFocus: false,
  });
}

export function useTrayUsageEventBridge() {
  const queryClient = useQueryClient();
  useTauriEvent<TrayUsageSnapshot>("tray-usage-updated", (snapshot) => {
    queryClient.setQueryData(trayUsageKeys.snapshot(), snapshot);
  });
}
```

`useRefreshTrayUsage` keeps one in-flight mutation and writes the returned snapshot into the same key. It never clears cached data on error.

- [ ] **Step 6: Split the window bootstrap before main-only initialization**

```tsx
export function WindowSurface({
  windowLabel,
}: {
  windowLabel: string;
}) {
  return windowLabel === "tray-popover" ? <TrayUsagePopover /> : <App />;
}
```

In `bootstrap()`, after confirming Tauri runtime, read `getCurrentWebviewWindow().label`. For `tray-popover`:

```tsx
document.documentElement.dataset.windowSurface = "tray-popover";
root.render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <ThemeProvider
        defaultTheme="system"
        storageKey="llm-usage-bar:theme"
      >
        <WindowSurface windowLabel="tray-popover" />
      </ThemeProvider>
    </QueryClientProvider>
  </React.StrictMode>,
);
return;
```

Do not initialize `DatabaseUpgrade`, updater UI, main window controls, Settings, or main configuration-error recovery on the popover path. Main/non-Tauri preview behavior remains unchanged.

- [ ] **Step 7: Run and commit the data/surface slice**

```bash
pnpm test:unit src/lib/query/trayUsage.test.tsx
pnpm test:unit src/windowSurface.test.tsx
pnpm typecheck
git add src/types/trayUsage.ts src/lib/api/trayUsage.ts src/lib/query/trayUsage.ts src/windowSurface.tsx src/windowSurface.test.tsx src/lib/query/trayUsage.test.tsx src/main.tsx tests/msw/tauriMocks.ts tests/msw/handlers.ts
git commit -m "feat: add tray popover renderer surface"
```

Expected: PASS; `main` still renders the existing `App` and the popover performs no main-only initialization.

---

### Task 8: Recreate the compact reference hierarchy as a functional popover

**Files:**

- Create: `src/components/tray-usage/TrayUsagePopover.tsx`
- Create: `src/components/tray-usage/TrayUsagePopoverView.tsx`
- Create: `src/components/tray-usage/TrayAgentTabs.tsx`
- Create: `src/components/tray-usage/TraySubscriptionSection.tsx`
- Create: `src/components/tray-usage/TrayApiSpendingSection.tsx`
- Create: `src/components/tray-usage/TrayUsageProgress.tsx`
- Create: `src/components/tray-usage/TrayUsageFooter.tsx`
- Create: `src/components/tray-usage/trayUsagePresentation.ts`
- Create: `src/components/tray-usage/TrayUsagePopover.test.tsx`
- Create: `src/components/tray-usage/trayUsagePresentation.test.ts`
- Modify: `src/index.css`
- Modify: `src/i18n/locales/en.json`
- Modify: `src/i18n/locales/zh.json`
- Modify: `src/i18n/locales/zh-TW.json`
- Modify: `src/i18n/locales/ja.json`
- Modify: `src/i18n/dashboardCopy.test.ts`

**Interfaces:**

- Consumes: `useTrayUsageSnapshot`, `useRefreshTrayUsage`, `TrayUsageSnapshot`.
- Produces a controlled pure view with `selectedAgentId: "overview" | string`.
- Produces formatter/status helpers shared by all visual states.
- Emits typed detail/settings/refresh/hide/quit actions; no UI control is static chrome.

- [ ] **Step 1: Write presentation utility tests**

```ts
expect(formatUsd("279.84", "en-US")).toBe("$279.84");
expect(formatUsd(null, "en-US")).toBe("—");
expect(formatPercent("88")).toBe("88%");
expect(clampPercentForProgress("120")).toBe(100);
expect(clampPercentForProgress("-2")).toBe(0);
expect(formatResetTime("2026-07-15T21:00:00Z", now, "en-US"))
  .toEqual({ text: "Pending refresh", pending: true });
expect(statusLabel("red", t)).toBe("Critical");
```

Cover invalid timestamp/number, reset future duration, all four statuses, all four cost qualities, and no negative countdown.

- [ ] **Step 2: Write the complete popover state tests**

Render `TrayUsagePopoverView` with deterministic props and assert:

```tsx
expect(screen.getByRole("tab", { name: "Overview" }))
  .toHaveAttribute("aria-selected", "true");
expect(screen.getByText("ChatGPT Plus/Pro")).toBeInTheDocument();
expect(screen.getByText("50% remaining")).toBeInTheDocument();
expect(screen.getByText("$8.00 of $10.00")).toBeInTheDocument();
expect(screen.getByText("Critical")).toBeInTheDocument();
expect(screen.getByText("Partial data")).toBeInTheDocument();
expect(screen.getByRole("progressbar", { name: /7-day allowance/i }))
  .toHaveAttribute("aria-valuenow", "50");
```

Then click an Agent tab and prove rows are filtered without another backend call. Separate tests cover:

- initial loading skeleton;
- empty Agent/source state;
- unknown/no usable data;
- stale plus last-success time and sanitized error copy;
- refreshing disabled action;
- missing budget with `Set daily budget` action;
- unavailable cost with details action;
- light/dark token compatibility;
- keyboard tab order;
- Escape and browser blur invoking hide once;
- Open details, Settings, Refresh, and Quit callbacks.

- [ ] **Step 3: Run visual-component tests and confirm red**

```bash
pnpm test:unit src/components/tray-usage/trayUsagePresentation.test.ts
pnpm test:unit src/components/tray-usage/TrayUsagePopover.test.tsx
pnpm test:unit src/i18n/dashboardCopy.test.ts
```

Expected: FAIL because components and `trayUsage` copy do not exist.

- [ ] **Step 4: Implement the controller and selected-Agent behavior**

```tsx
export function TrayUsagePopover() {
  const { data, isLoading } = useTrayUsageSnapshot();
  const refresh = useRefreshTrayUsage();
  const [selectedAgentId, setSelectedAgentId] =
    useState<"overview" | string>("overview");
  const didRequestInitialRefresh = useRef(false);

  useTrayUsageEventBridge();
  useEffect(() => {
    if (didRequestInitialRefresh.current) return;
    didRequestInitialRefresh.current = true;
    refresh.mutate();
  }, [refresh.mutate]);
  useTauriEvent("tray-popover-shown", () => {
    if (!refresh.isPending && !data?.refreshInProgress) {
      refresh.mutate();
    }
  });

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") void hideTrayPopover();
    };
    const onBlur = () => void hideTrayPopover();
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("blur", onBlur);
    };
  }, []);

  return (
    <TrayUsagePopoverView
      snapshot={data ?? null}
      loading={isLoading}
      selectedAgentId={selectedAgentId}
      onSelectAgent={setSelectedAgentId}
      refreshing={refresh.isPending || Boolean(data?.refreshInProgress)}
      onRefresh={() => refresh.mutate()}
      onOpenDetails={(agentModuleId) =>
        void openMainFromTray({ kind: "usage", agentModuleId })
      }
      onOpenSettings={(providerId) =>
        void openMainFromTray({ kind: "providerBudget", providerId })
      }
      onQuit={() => void quitFromTray()}
    />
  );
}
```

On mount, show the cache query immediately and request one deduplicated refresh.
`tray-popover-shown` repeats that request on later hide/show cycles; the shared
in-flight Promise prevents the first mount and first show event from creating two
backend cycles.

- [ ] **Step 5: Implement the fixed-frame hierarchy from the screenshot**

`TrayUsagePopoverView` uses this structure:

```tsx
<main
  data-testid="tray-usage-popover"
  className="tray-popover-frame flex h-[520px] w-[380px] flex-col overflow-hidden"
>
  <TrayAgentTabs
    agents={snapshot?.agents ?? []}
    selectedAgentId={selectedAgentId}
    onSelect={onSelectAgent}
  />
  <header className="shrink-0 border-b px-4 py-3">
    <div className="flex items-start justify-between gap-3">
      <div>
        <h1 className="text-[15px] font-semibold">{title}</h1>
        <p className="mt-1 text-xs text-muted-foreground">{freshnessText}</p>
      </div>
      <StatusBadge status={selectedStatus} />
    </div>
  </header>
  <ScrollArea className="min-h-0 flex-1">
    <div className="space-y-4 px-4 py-3">
      <TraySubscriptionSection providers={subscriptionProviders} />
      <TrayApiSpendingSection providers={meteredProviders} />
      {emptyState}
    </div>
  </ScrollArea>
  <TrayUsageFooter
    refreshing={refreshing}
    onOpenDetails={openSelectedDetails}
    onRefresh={onRefresh}
    onOpenSettings={openSelectedSettings}
    onQuit={onQuit}
  />
</main>
```

Match the supplied reference's hierarchy, not every row: compact horizontal Agent tabs, strong account header, restrained separators, dense metric pairs, and persistent footer. Use existing `ScrollArea`, `Button`, `Badge`, `ProviderIcon`/brand assets, and existing installed UI icons. Do not make new SVGs or emoji.

- [ ] **Step 6: Implement semantic progress and status treatment**

Use the native semantic `progress` element rather than drawing a bar:

```tsx
export function TrayUsageProgress({
  label,
  value,
  status,
}: {
  label: string;
  value: number;
  status: TrayUsageStatus;
}) {
  return (
    <progress
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={value}
      value={value}
      max={100}
      className={cn("tray-usage-progress", "is-" + status)}
    />
  );
}
```

The subscription bar displays consumed percent but text foregrounds remaining allowance. API progress displays budget consumed. Every colored state also renders `Healthy`, `Warning`, `Critical`, or `Data unavailable` text.

- [ ] **Step 7: Scope the surface CSS without changing main-window geometry**

```css
html[data-window-surface="tray-popover"],
html[data-window-surface="tray-popover"] body,
html[data-window-surface="tray-popover"] #root {
  width: 380px;
  height: 520px;
  overflow: hidden;
  background: transparent;
}

.tray-popover-frame {
  border: 1px solid hsl(var(--border));
  border-radius: 14px;
  background: hsl(var(--popover));
  color: hsl(var(--popover-foreground));
  box-shadow: 0 18px 44px rgb(0 0 0 / 0.34);
}

.tray-usage-progress {
  width: 100%;
  height: 6px;
  overflow: hidden;
  border: 0;
  border-radius: 999px;
  color: #5ac8fa;
  background: hsl(var(--muted));
}
.tray-usage-progress.is-yellow { color: #ffcc00; }
.tray-usage-progress.is-red { color: #ff3b30; }
.tray-usage-progress.is-unknown { color: #8e8e93; }
.tray-usage-progress::-webkit-progress-bar {
  border-radius: inherit;
  background: hsl(var(--muted));
}
.tray-usage-progress::-webkit-progress-value {
  border-radius: inherit;
  background: currentColor;
}
```

Use solid colors only; do not add gradients. Confirm existing light and dark tokens meet contrast/focus rules.

- [ ] **Step 8: Add complete copy in all supported locales**

Add an identical-shape top-level `trayUsage` object to English, Simplified Chinese, Traditional Chinese, and Japanese. It includes:

```json
{
  "overview": "Overview",
  "updated": "Updated {{time}}",
  "neverUpdated": "No usable data yet",
  "stale": "Stale",
  "healthy": "Healthy",
  "warning": "Warning",
  "critical": "Critical",
  "unknown": "Data unavailable",
  "subscription": "Subscription",
  "apiSpending": "API spending",
  "remaining": "{{percent}} remaining",
  "resets": "Resets {{time}}",
  "pendingRefresh": "Pending refresh",
  "today": "Today",
  "rolling30Days": "Last 30 days",
  "tokens": "Tokens",
  "ofBudget": "{{cost}} of {{budget}}",
  "setDailyBudget": "Set daily budget",
  "completeData": "Complete",
  "estimatedData": "Estimated",
  "partialData": "Partial data",
  "unavailableData": "Unavailable",
  "openDetails": "Open details",
  "refresh": "Refresh",
  "settings": "Settings",
  "quit": "Quit",
  "empty": "No visible usage sources"
}
```

Translate values naturally; keys must be identical. Extend `dashboardCopy.test.ts` to require every key in all four locales.

- [ ] **Step 9: Run and commit the visual slice**

```bash
pnpm test:unit src/components/tray-usage/trayUsagePresentation.test.ts
pnpm test:unit src/components/tray-usage/TrayUsagePopover.test.tsx
pnpm test:unit src/i18n/dashboardCopy.test.ts
pnpm typecheck
git add src/components/tray-usage src/index.css src/i18n/locales/en.json src/i18n/locales/zh.json src/i18n/locales/zh-TW.json src/i18n/locales/ja.json src/i18n/dashboardCopy.test.ts
git commit -m "feat: build compact tray usage popover"
```

Expected: PASS for overview, Agent filter, loading/stale/partial/unavailable, keyboard, and all footer interactions.

---

### Task 9: Add the dedicated budget editor and reliable main-window destinations

**Files:**

- Create: `src/components/settings/ProviderDailyBudgetField.tsx`
- Create: `src/components/settings/ProviderDailyBudgetField.test.tsx`
- Create: `src/hooks/useMainWindowNavigation.ts`
- Create: `src/hooks/useMainWindowNavigation.test.tsx`
- Modify: `src/types/usageDashboard.ts`
- Modify: `src/lib/api/trayUsage.ts`
- Modify: `src/lib/query/trayUsage.ts`
- Modify: `src/App.tsx`
- Modify: `src/App.usage-dashboard.test.tsx`
- Modify: `src/components/settings/SystemProviderCard.tsx`
- Modify: `src/components/settings/SystemProviderCard.test.tsx`
- Modify: `src/components/settings/UsageProvidersSettings.tsx`
- Modify: `src/components/settings/UsageProvidersSettings.system.test.tsx`
- Modify: `src/components/settings/SettingsPage.tsx`
- Modify: `src/components/usage-dashboard/UsageProviderDialog.tsx`
- Modify: `src/components/usage-dashboard/UsageProviderDialog.test.tsx`
- Modify: `src/i18n/locales/en.json`
- Modify: `src/i18n/locales/zh.json`
- Modify: `src/i18n/locales/zh-TW.json`
- Modify: `src/i18n/locales/ja.json`
- Modify: `tests/msw/handlers.ts`

**Interfaces:**

- Consumes: `UsageProviderView.dailyBudgetUsd` and `MainWindowDestination`.
- Produces: `useSetProviderDailyBudget`.
- Produces: `SettingsPage.defaultProviderId?: string` and `UsageProvidersSettings.targetProviderId?: string`.
- Removes renderer ownership of `UsageProviderInput.quotaConfig` and the free-form quota credential textarea.

- [ ] **Step 1: Write failing budget-field tests**

```tsx
it("saves a positive budget and can clear it", async () => {
  const onSave = vi.fn().mockResolvedValue(undefined);
  render(
    <ProviderDailyBudgetField
      providerId="system-openai-api"
      providerName="OpenAI API"
      value="10"
      onSave={onSave}
    />,
  );
  const input = screen.getByRole("spinbutton", {
    name: /openai api.*daily budget/i,
  });
  await user.clear(input);
  await user.type(input, "25.50");
  await user.click(screen.getByRole("button", { name: /save budget/i }));
  expect(onSave).toHaveBeenCalledWith("25.50");

  await user.click(screen.getByRole("button", { name: /clear budget/i }));
  expect(onSave).toHaveBeenLastCalledWith(null);
});

it.each(["0", "-1"])("rejects invalid budget %s", async (value) => {
  const onSave = vi.fn().mockResolvedValue(undefined);
  render(
    <ProviderDailyBudgetField
      providerId="metered"
      providerName="Metered"
      value={null}
      onSave={onSave}
    />,
  );
  const input = screen.getByRole("spinbutton", {
    name: /metered.*daily budget/i,
  });
  await user.type(input, value);
  await user.click(screen.getByRole("button", { name: /save budget/i }));
  expect(screen.getByRole("alert")).toHaveTextContent(/positive amount/i);
  expect(onSave).not.toHaveBeenCalled();
});
```

Add pending/disabled, backend error, success reset, and targeted autofocus tests.

- [ ] **Step 2: Write failing main-navigation tests**

Test live event and cold/pending destination. Agent navigation must wait until the visible Agent query contains the requested stable ID:

```tsx
setPendingMainDestination({
  kind: "usage",
  agentModuleId: "opencode",
});
emitTauriEvent("main-window-navigate", undefined);
await waitFor(() =>
  expect(screen.getByRole("tab", { name: "OpenCode" }))
    .toHaveAttribute("aria-selected", "true"),
);

setPendingMainDestination({
  kind: "providerBudget",
  providerId: "system-openrouter-api",
});
render(<App />);
await waitFor(() =>
  expect(screen.getByRole("spinbutton", {
    name: /openrouter.*daily budget/i,
  })).toHaveFocus(),
);
```

This protects lightweight mode, where `main` may be destroyed/recreated and a live event can arrive before mount.

- [ ] **Step 3: Run focused settings/navigation tests and confirm red**

```bash
pnpm test:unit src/components/settings/ProviderDailyBudgetField.test.tsx
pnpm test:unit src/hooks/useMainWindowNavigation.test.tsx
pnpm test:unit src/App.usage-dashboard.test.tsx
```

Expected: FAIL because the field and hook do not exist.

- [ ] **Step 4: Add the budget mutation and field**

```ts
export const setProviderDailyBudget = (
  providerId: string,
  dailyBudgetUsd: string | null,
) =>
  invoke<UsageProviderView>("set_provider_daily_budget", {
    providerId,
    dailyBudgetUsd,
  });

export function useSetProviderDailyBudget() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      providerId,
      dailyBudgetUsd,
    }: {
      providerId: string;
      dailyBudgetUsd: string | null;
    }) => setProviderDailyBudget(providerId, dailyBudgetUsd),
    onSuccess: (provider) => {
      queryClient.setQueryData<UsageProviderView[]>(
        usageDashboardKeys.providers(),
        (current = []) =>
          current.map((item) => item.id === provider.id ? provider : item),
      );
    },
  });
}
```

`ProviderDailyBudgetField` is a real `form` with `type="number"`, `min="0.000001"`, `step="0.01"`, USD suffix, Save, and Clear. Client validation rejects empty-on-save, non-finite, and `<=0`; backend validation remains authoritative. The wrapper ID is `provider-budget-<providerId>` and exposes a forwarded input ref.

- [ ] **Step 5: Place the field only on metered Provider cards**

For fixed metered Providers, render it after authentication content and before `SystemProviderAgentBindings`:

```tsx
{provider.billingKind === "metered" ? (
  <ProviderDailyBudgetField
    providerId={provider.id}
    providerName={provider.name}
    value={provider.dailyBudgetUsd}
    targeted={targetProviderId === provider.id}
  />
) : null}
```

For custom metered rows, render the same component as a separate full-width row inside that Provider container. Subscription Providers never render it. Add `dailyBudgetUsd: string | null` to `UsageProviderView` only.

- [ ] **Step 6: Remove raw quota-config ownership from the renderer**

Delete `quotaConfig` from the TypeScript `UsageProviderInput`, remove the state/parser/property/`Textarea` from `UsageProviderDialog`, remove `usageDashboard.quotaConfig` and `invalidQuotaConfig` copy from all locales, and update its tests:

```tsx
expect(
  screen.queryByLabelText(/quota credential json/i),
).not.toBeInTheDocument();
await user.click(screen.getByRole("button", { name: /save/i }));
expect(onSave).toHaveBeenCalledWith(
  expect.not.objectContaining({ quotaConfig: expect.anything() }),
);
```

Do not remove Rust's internal `UsageProviderStored.quota_config`; quota collectors may still need protected configuration. The boundary is that the renderer cannot read or write it.

- [ ] **Step 7: Implement event plus pending main navigation**

```ts
export function useMainWindowNavigation({
  agents,
  selectAgent,
  openProviderSettings,
}: {
  agents: AgentModuleView[];
  selectAgent: (id: string) => void;
  openProviderSettings: (providerId: string | null) => void;
}) {
  const [pending, setPending] =
    useState<MainWindowDestination | null>(null);

  const drainPending = useCallback(() => {
    void takePendingMainWindowDestination().then((destination) => {
      if (destination) setPending(destination);
    });
  }, []);

  useTauriEvent<void>("main-window-navigate", drainPending);

  useEffect(() => {
    drainPending();
  }, [drainPending]);

  useEffect(() => {
    if (!pending) return;
    if (pending.kind === "usage") {
      if (
        pending.agentModuleId &&
        !agents.some((agent) => agent.id === pending.agentModuleId)
      ) return;
      if (pending.agentModuleId) selectAgent(pending.agentModuleId);
    } else {
      openProviderSettings(pending.providerId);
    }
    setPending(null);
  }, [agents, openProviderSettings, pending, selectAgent]);
}
```

In `App`, replace the settings boolean-only target with:

```ts
const [settingsTarget, setSettingsTarget] = useState<{
  open: boolean;
  tab: string;
  providerId: string | null;
}>({ open: false, tab: "agents", providerId: null });
```

Pass `defaultTab={settingsTarget.tab}` and `defaultProviderId={settingsTarget.providerId ?? undefined}`. The Provider settings target uses `document.getElementById("provider-budget-" + providerId)?.scrollIntoView({ block: "center" })`, then focuses its input. Preserve the existing close-button/Agent-order work if this branch is later reconciled with the dirty root; do not overwrite it during merge.

- [ ] **Step 8: Run all affected settings tests**

```bash
pnpm test:unit src/components/settings/ProviderDailyBudgetField.test.tsx
pnpm test:unit src/components/settings/SystemProviderCard.test.tsx
pnpm test:unit src/components/settings/UsageProvidersSettings.system.test.tsx
pnpm test:unit src/components/usage-dashboard/UsageProviderDialog.test.tsx
pnpm test:unit src/hooks/useMainWindowNavigation.test.tsx
pnpm test:unit src/App.usage-dashboard.test.tsx
pnpm test:unit src/i18n/dashboardCopy.test.ts
pnpm typecheck
```

Expected: PASS; no rendered or serialized frontend input contains `quotaConfig`.

- [ ] **Step 9: Commit settings and navigation**

```bash
git add src/components/settings/ProviderDailyBudgetField.tsx src/components/settings/ProviderDailyBudgetField.test.tsx src/hooks/useMainWindowNavigation.ts src/hooks/useMainWindowNavigation.test.tsx src/types/usageDashboard.ts src/lib/api/trayUsage.ts src/lib/query/trayUsage.ts src/App.tsx src/App.usage-dashboard.test.tsx src/components/settings/SystemProviderCard.tsx src/components/settings/SystemProviderCard.test.tsx src/components/settings/UsageProvidersSettings.tsx src/components/settings/UsageProvidersSettings.system.test.tsx src/components/settings/SettingsPage.tsx src/components/usage-dashboard/UsageProviderDialog.tsx src/components/usage-dashboard/UsageProviderDialog.test.tsx src/i18n/locales/en.json src/i18n/locales/zh.json src/i18n/locales/zh-TW.json src/i18n/locales/ja.json tests/msw/handlers.ts
git commit -m "feat: configure daily provider budgets"
```

---

### Task 10: Run full verification, real macOS interaction QA, and visual comparison

**Files:**

- Create: `design-qa.md`
- Modify only when verification exposes a reproducible defect: the owning Task 1–9 file and its focused test.

**Interfaces:**

- Consumes the approved screenshot at `/var/folders/th/28ml58qj663d_8tbmcz417t00000gn/T/TemporaryItems/NSIRD_screencaptureui_qyMxAq/截屏2026-07-15 22.49.02.png`.
- Produces: passing automated verification, a real Tauri capture, and `design-qa.md` with `final result: passed`.
- Produces no merge into the dirty root worktree; branch integration is a separate user decision.

- [ ] **Step 1: Run format, focused regression, and full suites**

```bash
pnpm rust -- fmt --manifest-path src-tauri/Cargo.toml -- --check
pnpm rust -- clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
pnpm rust -- test --manifest-path src-tauri/Cargo.toml --lib
pnpm test:unit --exclude '.worktrees/**'
pnpm typecheck
pnpm build:renderer
pnpm build
```

Expected: every command exits 0. If a command fails, use `superpowers:systematic-debugging`, add the smallest regression test, fix it, and rerun the exact failed command before continuing.

- [ ] **Step 2: Inspect the final security boundary**

```bash
rg -n "quotaConfig|quota_config|apiKey|api_key|accessToken|access_token|fingerprint" src/types/trayUsage.ts src/lib/api/trayUsage.ts src/lib/query/trayUsage.ts src/components/tray-usage src-tauri/src/commands/tray_usage.rs src-tauri/src/usage/tray_snapshot.rs
rg -n "daily_budget_usd" src-tauri/src/usage/system_provider_migration.rs src-tauri/src/database/dao/usage_providers.rs src-tauri/src/database/backup.rs
```

Expected: the first command finds no sensitive DTO/payload fields; `quota_config` may appear only in a negative/redaction test. The second confirms system reconciliation/ordinary saves never update the budget and backup explicitly preserves it.

- [ ] **Step 3: Launch a real isolated macOS app**

Read `computer-use:computer-use` before controlling the Mac. Use a disposable app home:

```bash
mkdir -p /private/tmp/llm-usage-bar-popover-qa
HOME=/private/tmp/llm-usage-bar-popover-qa LLM_USAGE_BAR_TEST_HOME=/private/tmp/llm-usage-bar-popover-qa pnpm dev
```

Wait for the tray item; do not use the user's production database or change real Provider credentials.

- [ ] **Step 4: Seed deterministic disposable QA data**

After the app creates schema v18, keep it running and insert only non-secret rows into its disposable database:

```bash
/usr/bin/sqlite3 /private/tmp/llm-usage-bar-popover-qa/.llm-usage-bar/llm-usage-bar.db "PRAGMA foreign_keys=ON;
INSERT OR REPLACE INTO usage_providers (
  id, name, billing_kind, product_group_id, token_sources,
  quota_source, quota_interval_seconds, route_app_type, route_config,
  quota_config, enabled, needs_review, created_at, updated_at,
  system_preset_key, daily_budget_usd
) VALUES (
  'qa-metered', 'QA Metered', 'metered', 'qa-metered', '[\"proxy\"]',
  NULL, NULL, NULL, NULL, NULL, 1, 0,
  CAST(strftime('%s','now') AS INTEGER),
  CAST(strftime('%s','now') AS INTEGER),
  NULL, '25'
);
INSERT OR REPLACE INTO agent_provider_bindings (
  id, agent_module_id, provider_id, enabled, api_key_fingerprint,
  credential_slot, credential_version, created_at, updated_at, route_protocol
) VALUES (
  'qa-codex-metered', 'codex', 'qa-metered', 1, NULL,
  NULL, 0, CAST(strftime('%s','now') AS INTEGER),
  CAST(strftime('%s','now') AS INTEGER), NULL
);
INSERT OR IGNORE INTO usage_events (
  event_id, source, provider_id, product_group_id, occurred_at,
  model, input_tokens, output_tokens, cache_read_tokens,
  cache_creation_tokens, total_cost_usd, cost_source, created_at,
  agent_module_id
) VALUES (
  'qa-today-10-usd', 'proxy', 'qa-metered', 'qa-metered',
  CAST(strftime('%s','now') AS INTEGER) - 60,
  'qa-model', 1000, 500, 0, 0, '10', 'upstream',
  CAST(strftime('%s','now') AS INTEGER), 'codex'
);
INSERT OR IGNORE INTO quota_snapshots (
  snapshot_id, provider_id, fetched_at,
  five_hour_utilization_percent, five_hour_resets_at,
  seven_day_utilization_percent, seven_day_resets_at,
  manual_resets_remaining, raw_payload, created_at
) VALUES (
  'qa-subscription-green', 'system-chatgpt-subscription',
  CAST(strftime('%s','now') AS INTEGER),
  '49', datetime('now', '+5 hours'), NULL, NULL,
  NULL, '{}', CAST(strftime('%s','now') AS INTEGER)
);"
```

Click Refresh to rebuild through the normal command. Use the budget editor to change `25`→`15`→`10` for green/yellow/red metered states and Clear for the missing-budget state. For subscription thresholds, insert later snapshots with unique IDs/current `fetched_at` and values `50`, `80`, and `81`, clicking Refresh after each. Do not add a production debug override.

- [ ] **Step 5: Verify the complete interaction contract**

Using the actual status item and window:

1. Startup shows a colored/gray dot but no main window or Dock icon.
2. First left click shows the 380×520 popover below the item; Dock remains hidden.
3. Second left click, Escape, and click-away each hide it once.
4. Right click retains the complete Provider/Profile, lightweight, website, and quit menu.
5. Primary and secondary-display edge placement stays within the active work area.
6. Repeated Refresh starts one cycle, cache stays visible, and failure preserves the alert with stale copy.
7. Overview/Agent tabs filter locally.
8. Missing-budget action opens Provider settings; main/Dock appear and the correct input is focused.
9. Open details selects and focuses the requested Agent.
10. Closing and minimizing main hides it and restores Accessory policy.
11. Keyboard order, progress names, status text, light appearance, and dark appearance are legible.
12. DevTools and Rust logs show no error or secret-bearing diagnostic.

- [ ] **Step 6: Capture and run blocking design QA**

Read `product-design:design-qa` and its QA rubric. Capture the real popover at 380×520 in the same dark state/content density as the supplied reference. Put source and implementation side by side in one visible comparison using the in-app browser or two tiled Preview windows, then evaluate:

- typography, weight, and line height;
- frame, padding, separators, radius, and scroll/fixed regions;
- dark/light tokens and semantic colors;
- Provider/Agent asset fidelity and sharpness;
- copy density and hierarchy;
- loading/stale/partial/empty states;
- focus and interaction affordances.

Write `design-qa.md` with source path, implementation capture path, viewport, state, full-view comparison, focused regions, findings, iteration history, and exactly:

```text
final result: passed
```

Any P0/P1/P2 finding blocks completion: fix it, capture the same state again, update comparison history, and repeat. P3 polish may remain as follow-up.

- [ ] **Step 7: Request two-stage code review and fix findings**

Use `superpowers:requesting-code-review`. Review first against this plan/spec, then for code quality/security/regressions. Every actionable finding gets a focused test and fix; rerun the relevant focused test plus Step 1's full gate.

- [ ] **Step 8: Run verification-before-completion and inspect Git scope**

Use `superpowers:verification-before-completion`, then:

```bash
git status --short
git diff --check
git log --oneline 3d847d65..HEAD
git diff --stat 3d847d65..HEAD
pnpm cargo:cache -- status
```

Expected: only planned feature/report files are present; no `.pnpm-store/`, root-worktree edits, credentials, QA database, or temporary screenshots are staged.

- [ ] **Step 9: Commit the passing QA report if it changed**

```bash
git add design-qa.md
git commit -m "docs: record menu bar popover design QA"
```

Skip this commit only when `design-qa.md` is already included in the immediately preceding fix commit with `final result: passed`.

- [ ] **Step 10: Hand off the isolated branch**

Use `superpowers:finishing-a-development-branch`. Report the branch/worktree, exact passing commands, real interaction results, QA capture/report, and any P3 notes. Do not merge, overwrite, or clean the user's dirty root worktree without a new explicit integration choice.

---

## Plan Self-Review Results

- **Spec coverage:** Tasks 1–10 map to all ten acceptance criteria, every rebuild trigger, four status assets, left/right input, two-window Dock policy, stale retention, budget persistence, safe DTOs, frontend states, and real macOS QA.
- **Instruction quality:** Every code-changing task names exact files, interfaces, red/green commands, implementation shape, and scoped commit. No deferred implementation markers remain.
- **Type consistency:** Rust and TypeScript use the same camelCase DTO properties, status/cost-quality strings, event/command names, window label, and destination tags.
- **Dirty-worktree safety:** Execution begins in an isolated worktree at `3d847d65` and never stages root edits or `.pnpm-store/`.
- **Execution rule:** Each slice begins with a failing focused test, ends with passing focused tests, and is committed before the next architectural layer.
