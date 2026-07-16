# Managed ChatGPT Quota Integration Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` and `superpowers:test-driven-development`. Implement each task test-first and stop for independent review after each commit.

**Goal:** Make the fixed `ChatGPT Plus/Pro` Provider refresh quota through the ChatGPT account already connected in LLM Usage Bar, while keeping Claude subscription quota explicitly unavailable and preventing repeated identical dashboard errors.

**Confirmed failure:** The managed Codex OAuth account and Codex Agent binding both exist, but fixed-Provider reconciliation writes `quota_source = NULL`. `refresh_provider_quota` therefore fails before credential lookup or any network request with `quota source is not configured`.

**Architecture:** The fixed Provider catalog owns canonical quota metadata. Only `system-chatgpt-subscription` receives source `codex_oauth` and a 300-second interval. A production `QuotaCollector` reuses the exact same `Arc<RwLock<CodexOAuthManager>>` that backs the existing Tauri OAuth state and the existing `query_codex_quota` / `wham/usage` protocol. `AppState` accepts the production quota service through a real constructor so tests can inject a fake collector without credentials or network. Claude remains `quota_source = NULL`; no Claude OAuth/token path is added.

**Relationship to the approved feature:** This is a prerequisite correction for [`2026-07-15-menu-bar-usage-popover-implementation.md`](./2026-07-15-menu-bar-usage-popover-implementation.md). The authoritative tray snapshot must not classify or display a false ChatGPT "unavailable" state caused by missing internal wiring.

## Non-negotiable boundaries

- Never read a second OAuth file, the Codex CLI token, or a Claude OAuth token for this path.
- Never expose access tokens, refresh tokens, account IDs, raw upstream bodies, or credential fingerprints to the renderer, logs, test output, or fixtures.
- Preserve the existing `claude`, `codex`, and `coding_plan` collectors for custom/legacy Providers; `codex_oauth` is a distinct managed source.
- Reconciliation must repair canonical ChatGPT metadata on every startup and must not overwrite user-owned `daily_budget_usd`.
- Tests use an injected fake collector. They must not make network requests or depend on the developer's real account.
- Run Rust only through `pnpm rust -- ...`; focused Vitest is `pnpm test:unit <path>` without an inserted `--`.

---

### Task 1: Route the fixed ChatGPT Provider through managed Codex OAuth

**Files:**

- Modify: `src-tauri/src/usage/system_providers.rs`
- Modify: `src-tauri/src/usage/system_provider_migration.rs`
- Modify: `src-tauri/src/usage/quota.rs`
- Modify: `src-tauri/src/services/subscription.rs`
- Modify: `src-tauri/src/commands/codex_oauth.rs`
- Modify: `src-tauri/src/store.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/database/tests.rs`
- Test: `src-tauri/src/commands/usage_dashboard.rs`

**Interfaces:**

- `SystemProviderDefinition` gains canonical `quota_source` and `quota_interval_seconds` metadata.
- `MANAGED_CODEX_QUOTA_SOURCE` is exactly `"codex_oauth"`.
- A shared managed-OAuth helper resolves the requested/default account, obtains a valid token from `CodexOAuthManager`, drops the outer manager guard, and then reuses `query_codex_quota`.
- `QuotaService::production(db, codex_oauth_manager)` adds the managed collector while keeping existing collectors.
- A production-used `AppState` constructor accepts an injected `Arc<QuotaService>`; existing test constructors keep deterministic defaults.

- [ ] **Step 1: Write RED catalog reconciliation tests**

Extend the idempotent system catalog test to assert, before and after a second reconciliation:

```rust
assert_eq!(chatgpt.quota_source.as_deref(), Some("codex_oauth"));
assert_eq!(chatgpt.quota_interval_seconds, Some(300));
assert_eq!(claude.quota_source, None);
assert_eq!(claude.quota_interval_seconds, None);
```

Also set `daily_budget_usd` on a metered fixed Provider before reconciliation and retain the existing preservation assertion.

- [ ] **Step 2: Run the catalog test and confirm RED**

```bash
pnpm --config.verify-deps-before-run=false rust -- test --manifest-path src-tauri/Cargo.toml database::tests::system_catalog_reconciliation_is_idempotent_and_never_restores_a_deleted_default_binding --lib -- --nocapture
```

Expected: FAIL because ChatGPT currently reconciles to `quota_source = NULL`.

- [ ] **Step 3: Persist canonical quota metadata**

Add `quota_source: Option<&'static str>` and `quota_interval_seconds: Option<u64>` to the server-owned catalog. Set only ChatGPT to `Some("codex_oauth")` / `Some(300)`. Bind those fields in both INSERT and conflict UPDATE. Keep Claude and all fixed API Providers at `None`.

Do not reuse `route_app_type` as quota metadata and do not put any quota source in `quota_config`.

- [ ] **Step 4: Write a RED Provider-command behavior test**

At the `refresh_provider_quota_test_hook` boundary:

1. create an in-memory database and reconcile fixed Providers;
2. inject a fake collector whose source is `codex_oauth` and whose result contains stable 5-hour and 7-day windows;
3. create `AppState` with that injected `QuotaService`;
4. refresh `system-chatgpt-subscription` and assert a persisted snapshot/fetch state;
5. reconcile the system catalog again, refresh again, and assert the fake collector was called twice and the source still resolves.

This test is the proof that restart reconciliation cannot recreate the user's current failure. It must contain no real OAuth value or HTTP call.

- [ ] **Step 5: Run the command test and confirm RED**

```bash
pnpm --config.verify-deps-before-run=false rust -- test --manifest-path src-tauri/Cargo.toml commands::usage_dashboard::tests::fixed_chatgpt_refresh_survives_catalog_reconciliation --lib -- --nocapture
```

Expected: FAIL because `AppState` cannot yet accept the injected production quota service and the catalog source is absent.

- [ ] **Step 6: Implement the shared managed collector and production wiring**

Extract the existing managed-account/default-account/token lookup used by `get_codex_oauth_quota` into a shared helper; keep the command as a thin wrapper. Add a collector that owns a clone of the same manager `Arc` and reports source `codex_oauth`.

In Tauri setup:

1. create one `CodexOAuthManager` after the database is ready;
2. pass the same `Arc` into `QuotaService::production` and `CodexOAuthState`;
3. inject that quota service into `AppState` through the production-used constructor;
4. remove the later duplicate manager construction;
5. start the quota scheduler only after both managed states are registered.

Keep `QuotaService::new` for deterministic tests and existing custom/legacy collectors. Do not change Claude auth behavior.

- [ ] **Step 7: Run focused backend verification**

```bash
pnpm --config.verify-deps-before-run=false rust -- test --manifest-path src-tauri/Cargo.toml fixed_chatgpt_refresh_survives_catalog_reconciliation --lib -- --nocapture
pnpm --config.verify-deps-before-run=false rust -- test --manifest-path src-tauri/Cargo.toml system_catalog_reconciliation_is_idempotent_and_never_restores_a_deleted_default_binding --lib -- --nocapture
pnpm --config.verify-deps-before-run=false rust -- test --manifest-path src-tauri/Cargo.toml usage::quota::tests --lib -- --nocapture
pnpm --config.verify-deps-before-run=false rust -- fmt --manifest-path src-tauri/Cargo.toml --all -- --check
```

Expected: PASS, with no outbound request in tests.

- [ ] **Step 8: Commit and request independent review**

```bash
git add src-tauri/src/usage/system_providers.rs src-tauri/src/usage/system_provider_migration.rs src-tauri/src/usage/quota.rs src-tauri/src/services/subscription.rs src-tauri/src/commands/codex_oauth.rs src-tauri/src/store.rs src-tauri/src/lib.rs src-tauri/src/database/tests.rs src-tauri/src/commands/usage_dashboard.rs
git commit -m "fix: connect managed ChatGPT quota source"
```

Review must verify the single-manager invariant, no Claude OAuth path, no secret/error-body exposure, reconciliation idempotence, and scheduler initialization order.

---

### Task 2: Make quota availability and refresh feedback truthful

**Files:**

- Modify: `src/components/usage-dashboard/UsageDashboardPage.tsx`
- Modify: `src/components/usage-dashboard/UsageDashboardPage.test.tsx`
- Modify: `src/components/usage-dashboard/SubscriptionProviderCard.tsx`
- Modify: `src/components/usage-dashboard/SubscriptionProviderCard.test.tsx`
- Modify: `tests/msw/handlers.ts`

- [ ] **Step 1: Write RED duplicate-error lifecycle test**

Queue two identical rejected refreshes followed by one success. After each rejection, assert exactly one alert with that message. After success, assert the action error is gone. Keep query errors independent and deduplicate the combined rendered list.

- [ ] **Step 2: Write RED quota-capability card tests**

Assert a Provider with `quotaSource: null`:

- renders both quota windows as unavailable;
- does not render a `Refresh quota` button;
- still renders `Sync sessions` when session logs apply.

Assert ChatGPT with `quotaSource: "codex_oauth"` renders Refresh and invokes it with the Provider ID. Do not add OAuth behavior to the Claude card.

- [ ] **Step 3: Run focused tests and confirm RED**

```bash
pnpm --config.verify-deps-before-run=false test:unit src/components/usage-dashboard/UsageDashboardPage.test.tsx
pnpm --config.verify-deps-before-run=false test:unit src/components/usage-dashboard/SubscriptionProviderCard.test.tsx
```

- [ ] **Step 4: Implement replacement, clearing, and capability-gated actions**

On refresh failure, replace the prior action error instead of appending. On successful refresh, clear the scoped action error if the active Agent still matches. Deduplicate the combined query/action error list before rendering.

Treat `Boolean(usage.provider.quotaSource)` as the Refresh capability. When absent, ignore any stale quota/fetch-state payload and show the existing unavailable-window copy. Keep session sync independent.

Update the MSW fixed ChatGPT fixture from `codex` to `codex_oauth` so renderer tests match the Rust catalog rather than hiding the production mismatch.

- [ ] **Step 5: Verify, commit, and request independent review**

```bash
pnpm --config.verify-deps-before-run=false test:unit src/components/usage-dashboard/UsageDashboardPage.test.tsx
pnpm --config.verify-deps-before-run=false test:unit src/components/usage-dashboard/SubscriptionProviderCard.test.tsx
pnpm --config.verify-deps-before-run=false typecheck
git add src/components/usage-dashboard/UsageDashboardPage.tsx src/components/usage-dashboard/UsageDashboardPage.test.tsx src/components/usage-dashboard/SubscriptionProviderCard.tsx src/components/usage-dashboard/SubscriptionProviderCard.test.tsx tests/msw/handlers.ts
git commit -m "fix: clarify subscription quota feedback"
```

Review must confirm identical alerts do not accumulate, success clears only action feedback, query errors are preserved, null-source subscriptions never trigger refresh, and ChatGPT uses `codex_oauth` consistently.

---

## Integration gate

After both tasks are independently approved:

```bash
pnpm --config.verify-deps-before-run=false rust -- test --manifest-path src-tauri/Cargo.toml database::tests --lib
pnpm --config.verify-deps-before-run=false rust -- test --manifest-path src-tauri/Cargo.toml commands::usage_dashboard::tests --lib
pnpm --config.verify-deps-before-run=false test:unit src/components/usage-dashboard/UsageDashboardPage.test.tsx
pnpm --config.verify-deps-before-run=false test:unit src/components/usage-dashboard/SubscriptionProviderCard.test.tsx
pnpm --config.verify-deps-before-run=false typecheck
git diff --check
```

Then return to Task 2 of the approved menu-bar popover implementation plan. Real-account verification is performed only during final isolated macOS QA; automated tests never touch the user's credentials.
