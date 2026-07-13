# Dynamic Dashboard Frontend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Each implementation task is delegated through the Kimi Coding MCP only after `kimi_delegation_status` reports ready; Codex must read every patch page in order, apply it with the review token, verify it in this worktree, then accept or roll it back.

**Goal:** Replace the mixed monitoring/configuration home page with database-driven dashboard-module navigation, separate subscription and API usage views, and a three-section settings surface while keeping the work frontend-only.

**Status:** Complete on `main`: Task 1 (`efa01503`), Task 2 (`42099f57`), Task 3 (`7aefa078`), Task 4 (`af5ea79c`), and Task 5 (`04d1107d`) are implemented and covered by the accompanying frontend tests. The dashboard-module backend commands subsequently landed in `87b19ebe`, `bde56533`, and `4d6c899f`.

**Architecture:** Add a typed React Query boundary for the future dashboard-module Tauri commands and the Provider-to-module field. The app shell owns module selection; pure projection helpers flatten the existing Provider-aware dashboard by stable Provider ID, then module-specific pages render either one subscription account at a time or all metered Providers. Configuration mutations live only under Settings, and a browser-only Vite run renders one explicit preview notice without invoking desktop APIs.

**Tech Stack:** React 18, TypeScript, TanStack React Query, Radix Tabs through `src/components/ui/tabs.tsx`, Tailwind CSS, i18next, Tauri `invoke`, Vitest, Testing Library, MSW.

## Global Constraints

- This plan is frontend-only. Do not modify `src-tauri/**`, database migrations, command registration, authentication, routing authority, or dependency versions.
- Dashboard modules come only from `list_dashboard_modules`; do not synthesize them from Provider names, `productGroupId`, quota sources, or a frontend constant.
- Module names are display text, never business keys. All selection and Provider membership use stable IDs.
- Clicking a dashboard module changes visible UI only. It must not call Provider enable/switch, RouteBinding, proxy, CLI configuration, or live-takeover mutations.
- Subscription quota snapshots remain status data and never enter Token, request, or cost totals.
- API totals use only enabled metered Provider rows. A missing cost remains unknown/partial and is never treated as numeric zero.
- Historical dashboard rows are merged by stable `provider.id`; `productGroupId` remains event attribution and is not a module key.
- Provider, static route, and proxy controls must not render on the home page; they remain available under Settings.
- Focused Vitest commands are `pnpm test:unit <path>` with no `--` before the path.
- Do not run `tauri dev` or a desktop build against the real user home. Renderer tests, typecheck, format checks, and `pnpm build:renderer` are sufficient for this frontend phase.
- Browser-only Vite startup must render exactly one friendly preview notice and must not invoke Tauri APIs.
- The four locale files `en.json`, `zh.json`, `zh-TW.json`, and `ja.json` must remain structurally aligned for every new key.
- The backend commands introduced by this frontend contract do not exist yet. In the desktop runtime this branch must surface a recoverable module-read error until the data-layer phase implements them; do not add a compatibility fallback that violates the approved design.

## File Responsibility Map

- `src/types/usageDashboard.ts`: dashboard-module and Provider membership contracts.
- `src/lib/api/usageDashboard.ts`: exact Tauri command names and payloads.
- `src/lib/query/usageDashboard.ts`: module queries/mutations and cache invalidation.
- `src/components/usage-dashboard/usageDashboardProjection.ts`: pure stable-ID projection and exact decimal aggregation.
- `src/components/usage-dashboard/DashboardModuleSwitcher.tsx`: accessible top-level module Tabs.
- `src/components/usage-dashboard/useDashboardModuleSelection.ts`: persisted selection and fallback rules.
- `src/components/usage-dashboard/SubscriptionModulePage.tsx`: one subscription Provider instance at a time.
- `src/components/usage-dashboard/ApiUsagePage.tsx`: all enabled metered Providers plus partial-cost status.
- `src/components/usage-dashboard/UsageDashboardPage.tsx`: range clock, query orchestration, errors, and module page dispatch only.
- `src/components/settings/DashboardModulesSettings.tsx`: module create/rename/reorder/show/hide/delete controls.
- `src/components/settings/UsageProvidersSettings.tsx`: Provider list and Provider dialog.
- `src/components/settings/ProxyRoutingSettings.tsx`: proxy status/control and RouteBinding UI.
- `src/components/settings/SettingsPage.tsx`: three-section settings dialog.
- `src/lib/platform.ts` and `src/main.tsx`: desktop-runtime detection and browser preview boundary.
- `src/i18n/locales/*.json`: user-visible copy.

---

### Task 1: Add the Dashboard Module Frontend Contract

**Files:**
- Modify: `src/types/usageDashboard.ts`
- Modify: `src/lib/api/usageDashboard.ts`
- Modify: `src/lib/query/usageDashboard.ts`
- Test: `src/lib/query/usageDashboard.test.tsx`

**Interfaces:**
- Consumes: existing `UsageProviderInput`, `UsageProviderView`, `usageDashboardApi`, and `usageDashboardKeys`.
- Produces: `DashboardModuleKind`, `DashboardModuleView`, `DashboardModuleInput`, `useDashboardModules`, and five mutation hooks used by Tasks 2 and 4.

- [x] **Step 1: Write failing command and cache-invalidation tests**

Add tests that mock Tauri `invoke` and assert these exact calls:

```ts
await usageDashboardApi.listDashboardModules();
expect(invoke).toHaveBeenCalledWith("list_dashboard_modules");

await usageDashboardApi.saveDashboardModule({
  id: null,
  name: "Gemini",
  kind: "subscription",
  sortOrder: 4,
  visible: true,
});
expect(invoke).toHaveBeenCalledWith("save_dashboard_module", {
  input: {
    id: null,
    name: "Gemini",
    kind: "subscription",
    sortOrder: 4,
    visible: true,
  },
});

await usageDashboardApi.reorderDashboardModules(["codex", "api"]);
expect(invoke).toHaveBeenCalledWith("reorder_dashboard_modules", {
  moduleIds: ["codex", "api"],
});

await usageDashboardApi.setDashboardModuleVisibility("codex", false);
expect(invoke).toHaveBeenCalledWith("set_dashboard_module_visibility", {
  moduleId: "codex",
  visible: false,
});

await usageDashboardApi.deleteDashboardModule("gemini");
expect(invoke).toHaveBeenCalledWith("delete_dashboard_module", {
  moduleId: "gemini",
});
```

Render the mutation hooks in a `QueryClientProvider`, complete each mutation, and assert invalidation of `usageDashboardKeys.modules()`, `providers()`, and `dashboards()` where membership or visibility can change.

- [x] **Step 2: Run the focused test and observe the missing-contract failure**

Run:

```bash
pnpm test:unit src/lib/query/usageDashboard.test.tsx
```

Expected: FAIL because the five API methods, module query key, and hooks do not exist.

- [x] **Step 3: Add the exact TypeScript contracts**

Add:

```ts
export type DashboardModuleKind = "subscription" | "api";

export interface DashboardModuleView {
  id: string;
  name: string;
  kind: DashboardModuleKind;
  sortOrder: number;
  visible: boolean;
  isSystem: boolean;
  providerCount: number;
}

export interface DashboardModuleInput {
  id: string | null;
  name: string;
  kind: DashboardModuleKind;
  sortOrder: number;
  visible: boolean;
}
```

Add `dashboardModuleId?: string | null` to `UsageProviderInput` and `dashboardModuleId: string | null` to `UsageProviderView`. The field is required on the returned view so missing backend serialization is visible at the contract boundary.

- [x] **Step 4: Implement API methods and React Query hooks**

Add the five API methods using the exact command names and camelCase payloads from Step 1. Add these keys and hooks:

```ts
modules: () => [...usageDashboardKeys.all, "modules"] as const,

export function useDashboardModules() {
  return useQuery({
    queryKey: usageDashboardKeys.modules(),
    queryFn: usageDashboardApi.listDashboardModules,
  });
}
```

The save/reorder/visibility/delete mutation hooks must invalidate modules, providers, and dashboards after success. Extend Provider-save invalidation to include modules because `providerCount` and membership can change.

- [x] **Step 5: Verify Task 1**

Run:

```bash
pnpm test:unit src/lib/query/usageDashboard.test.tsx
pnpm typecheck
pnpm format:check
```

Expected: all commands exit 0.

- [x] **Step 6: Commit Task 1**

```bash
git add src/types/usageDashboard.ts src/lib/api/usageDashboard.ts src/lib/query/usageDashboard.ts src/lib/query/usageDashboard.test.tsx
git commit -m "feat(ui): add dashboard module contract"
```

### Task 2: Add Dynamic Module Selection and Accessible Top-Level Navigation

**Files:**
- Create: `src/components/usage-dashboard/useDashboardModuleSelection.ts`
- Create: `src/components/usage-dashboard/useDashboardModuleSelection.test.tsx`
- Create: `src/components/usage-dashboard/DashboardModuleSwitcher.tsx`
- Create: `src/components/usage-dashboard/DashboardModuleSwitcher.test.tsx`
- Modify: `src/App.tsx`
- Test: `src/App.usage-dashboard.test.tsx`

**Interfaces:**
- Consumes: `DashboardModuleView` and `useDashboardModules()` from Task 1.
- Produces: `selectedModule: DashboardModuleView | null`, `selectModule(id)`, and `DashboardModuleSwitcher` for Task 3.

- [x] **Step 1: Write failing selection tests**

Cover all selection rules with data whose names are deliberately arbitrary:

```ts
const modules = [
  { id: "b", name: "Second", kind: "subscription", sortOrder: 2, visible: true, isSystem: false, providerCount: 1 },
  { id: "api-id", name: "Metered", kind: "api", sortOrder: 4, visible: true, isSystem: true, providerCount: 2 },
  { id: "a", name: "First", kind: "subscription", sortOrder: 1, visible: true, isSystem: false, providerCount: 1 },
];
```

Assert that modules render in `sortOrder`, a persisted visible ID remains selected, a hidden/deleted ID falls back to the first visible module, only the API module remains selectable when all subscriptions are hidden, and no visible modules produce `null`. Assert the chosen ID is stored under `llm-usage-bar:last-dashboard-module-id`.

- [x] **Step 2: Write failing interaction and App-shell tests**

Render `DashboardModuleSwitcher` with five modules and assert:

```ts
expect(screen.getAllByRole("tab")).toHaveLength(5);
expect(screen.getByRole("tab", { name: "Fifth" })).toBeVisible();
fireEvent.keyDown(screen.getByRole("tab", { name: "First" }), { key: "ArrowRight" });
expect(onSelect).toHaveBeenCalledWith("second-id");
```

In `App.usage-dashboard.test.tsx`, mock only module and dashboard queries, then assert the module Tabs sit in the header before Settings. Clicking a module must not call Provider, RouteBinding, proxy, or live configuration mutations.

- [x] **Step 3: Run focused tests and observe missing components**

Run:

```bash
pnpm test:unit src/components/usage-dashboard/useDashboardModuleSelection.test.tsx
pnpm test:unit src/components/usage-dashboard/DashboardModuleSwitcher.test.tsx
pnpm test:unit src/App.usage-dashboard.test.tsx
```

Expected: FAIL because the hook, switcher, and App wiring are absent.

- [x] **Step 4: Implement persisted selection**

Implement the hook with these invariants:

```ts
const visibleModules = [...modules]
  .filter((module) => module.visible)
  .sort((left, right) => left.sortOrder - right.sortOrder);
const selectedModule =
  visibleModules.find((module) => module.id === selectedId) ??
  visibleModules[0] ??
  null;
```

Synchronize the selected ID and local storage in effects. Never select by module name. If the current module disappears, replace the stored value with the fallback ID.

- [x] **Step 5: Implement the switcher with Radix Tabs**

Use `Tabs`, `TabsList`, and `TabsTrigger` from `src/components/ui/tabs.tsx`. Give the list `aria-label={t("dashboardModules.navigation")}`, place it in an `overflow-x-auto` wrapper, keep labels readable with `whitespace-nowrap`, and call only `onSelect(value)` from `onValueChange`. When more than four modules are visible, also render an accessible `More modules` dropdown that lists the same stable IDs; it supplements horizontal scrolling and never replaces keyboard-accessible Tabs.

- [x] **Step 6: Wire navigation into App**

`App` owns `useDashboardModules`, selection, and the Settings dialog. Remove the duplicate subtitle from the header. Render module errors once with a Retry button. Pass `selectedModule` and `onOpenSettings={() => setSettingsOpen(true)}` to `UsageDashboardPage`.

If the query succeeds with zero visible modules, render a module-management empty state whose action opens Settings; do not render an empty dashboard.

- [x] **Step 7: Verify Task 2**

Run the three focused tests from Step 3, then:

```bash
pnpm typecheck
pnpm format:check
```

Expected: all commands exit 0.

- [x] **Step 8: Commit Task 2**

```bash
git add src/App.tsx src/App.usage-dashboard.test.tsx src/components/usage-dashboard/DashboardModuleSwitcher.tsx src/components/usage-dashboard/DashboardModuleSwitcher.test.tsx src/components/usage-dashboard/useDashboardModuleSelection.ts src/components/usage-dashboard/useDashboardModuleSelection.test.tsx
git commit -m "feat(ui): add dynamic dashboard navigation"
```

### Task 3: Project Stable Provider Data into Subscription and API Pages

**Files:**
- Create: `src/components/usage-dashboard/usageDashboardProjection.ts`
- Create: `src/components/usage-dashboard/usageDashboardProjection.test.ts`
- Create: `src/components/usage-dashboard/SubscriptionModulePage.tsx`
- Create: `src/components/usage-dashboard/SubscriptionModulePage.test.tsx`
- Create: `src/components/usage-dashboard/ApiUsagePage.tsx`
- Create: `src/components/usage-dashboard/ApiUsagePage.test.tsx`
- Modify: `src/components/usage-dashboard/UsageDashboardPage.tsx`
- Modify: `src/components/usage-dashboard/UsageDashboardPage.test.tsx`
- Modify: `src/components/usage-dashboard/SubscriptionProviderCard.tsx`
- Modify: `src/components/usage-dashboard/MeteredProviderCard.tsx`

**Interfaces:**
- Consumes: selected `DashboardModuleView`, `UsageDashboardView`, stable `UsageProviderView.dashboardModuleId`, current quota/session mutations, range helpers, and event bridge.
- Produces: `projectDashboardModule()` returning one discriminated subscription/API projection.

- [x] **Step 1: Write failing pure projection tests**

Create fixtures where the same Provider ID appears under two different historical `productGroupId` rows. Assert it is merged once, all four Token counts and event counts are summed, and the newest quota/fetch-state is retained. Assert subscription membership uses `dashboardModuleId` and API membership uses `billingKind === "metered"`, independent of names.

Test partial cost semantics:

```ts
expect(projected.kind).toBe("api");
expect(projected.totalCostUsd).toBe("1.25");
expect(projected.costStatus).toBe("partial");
```

Use one metered Provider with cost `"1.25"` and one with `null`/unavailable. Also test exact string addition such as `"0.1" + "0.2" === "0.3"` without floating-point conversion. Explicit `"0"` must remain known zero.

- [x] **Step 2: Write failing page behavior tests**

Subscription page assertions:

- one Provider renders details directly;
- two Providers render a second, separately labelled Tab list;
- switching instances shows only that Provider's quota and Token values;
- missing 5-hour or 7-day windows render `usageDashboard.quotaWindowUnavailable`, not `0%`;
- quota reset renders both absolute time and a relative countdown;
- Token range controls do not alter quota values.

API page assertions:

- only enabled metered Providers appear;
- subscription cards never appear;
- total Token and request counts use metered rows only;
- unknown cost shows a partial-data label and is not formatted as `$0`;
- upstream and estimated source badges remain visible per Provider.

Home-page assertion: Provider config, RouteBinding, Add Provider, enable/disable, and proxy buttons are absent.

- [x] **Step 3: Run focused tests and observe the missing projection/pages**

Run:

```bash
pnpm test:unit src/components/usage-dashboard/usageDashboardProjection.test.ts
pnpm test:unit src/components/usage-dashboard/SubscriptionModulePage.test.tsx
pnpm test:unit src/components/usage-dashboard/ApiUsagePage.test.tsx
pnpm test:unit src/components/usage-dashboard/UsageDashboardPage.test.tsx
```

Expected: FAIL because the new units do not exist and the current page still renders configuration controls.

- [x] **Step 4: Implement exact stable-ID projection**

Implement `projectDashboardModule(module, dashboard)` by flattening each product group's subscription and metered arrays, merging rows by `usage.provider.id`, and selecting:

```ts
module.kind === "subscription"
  ? merged.filter((usage) =>
      usage.provider.enabled &&
      usage.provider.billingKind === "subscription" &&
      usage.provider.dashboardModuleId === module.id)
  : merged.filter((usage) =>
      usage.provider.enabled && usage.provider.billingKind === "metered");
```

Implement decimal-string addition using `BigInt` plus a common decimal scale. Return `costStatus: "complete" | "estimated" | "partial" | "unavailable"`; any unavailable metered event/provider makes the result at least partial, and no known values yields unavailable.

- [x] **Step 5: Implement the subscription page**

Use a second Radix Tabs list with an accessible label distinct from the top-level list. Maintain selected Provider by stable ID and fall back when membership changes. Reuse `SubscriptionProviderCard`, but extend it to display input, output, cache read, cache creation, total Token, data source, last successful refresh, two separate quota-window cards, reset absolute time/countdown, and manual reset count.

Do not render USD for subscription Providers. The dashboard range controls affect Token/events only; quota values come unchanged from the latest snapshot.

- [x] **Step 6: Implement the API page**

Render an API summary card for total Token, request count, known USD cost, and cost status. Render every projected metered Provider with `MeteredProviderCard`. Preserve per-Provider source badges and recent-request errors. Because the current backend lacks a cross-Provider model-distribution and recent-request aggregate, render one localized `aggregateUnavailable` note instead of inventing totals from truncated Provider event pages.

- [x] **Step 7: Reduce UsageDashboardPage to monitoring orchestration**

Keep range state, 30-second advancement, `useUsageEventBridge`, quota refresh, Session sync, warning/error handling, and `useUsageDashboard`. Remove Provider-save/enable, bindings, proxy state/mutations, `RouteBindingsPanel`, Provider list, and `UsageProviderDialog`. Dispatch to `SubscriptionModulePage` or `ApiUsagePage` using `selectedModule.kind`.

- [x] **Step 8: Verify Task 3**

Run the four focused tests from Step 3, then:

```bash
pnpm test:unit src/App.usage-dashboard.test.tsx
pnpm typecheck
pnpm format:check
```

Expected: all commands exit 0.

- [x] **Step 9: Commit Task 3**

```bash
git add src/components/usage-dashboard
git commit -m "feat(ui): split subscription and api usage pages"
```

### Task 4: Move Module, Provider, Proxy, and Route Configuration into Settings

**Files:**
- Create: `src/components/settings/DashboardModulesSettings.tsx`
- Create: `src/components/settings/DashboardModulesSettings.test.tsx`
- Create: `src/components/settings/UsageProvidersSettings.tsx`
- Create: `src/components/settings/UsageProvidersSettings.test.tsx`
- Create: `src/components/settings/ProxyRoutingSettings.tsx`
- Create: `src/components/settings/ProxyRoutingSettings.test.tsx`
- Modify: `src/components/settings/SettingsPage.tsx`
- Modify: `src/components/usage-dashboard/UsageProviderDialog.tsx`
- Modify: `src/components/usage-dashboard/UsageProviderDialog.test.tsx`
- Test: `tests/components/SettingsDialog.test.tsx`
- Test: `tests/integration/SettingsDialog.test.tsx`

**Interfaces:**
- Consumes: Task 1 module/query mutations and existing Provider, RouteBinding, and proxy hooks.
- Produces: settings-only configuration surfaces; home-page code imports none of them.

- [x] **Step 1: Write failing module-management tests**

Assert ordered module rows, Provider counts, create/rename, Move Up/Move Down payload order, visibility mutation, and delete restrictions. The API system module must have no delete control and no kind editor. A custom module with `providerCount > 0` must explain that Providers need moving or disabling and must not call delete.

- [x] **Step 2: Write failing Provider form tests**

Update fixtures with `dashboardModuleId`. Assert:

- subscription billing requires a subscription module selection;
- only `kind === "subscription"` modules appear in the selector;
- quick create calls `save_dashboard_module`, selects the returned stable ID, then saves the Provider with that ID;
- editing can move the Provider by changing only `dashboardModuleId` while retaining Provider ID;
- metered billing hides the selector and saves `dashboardModuleId: null`.

- [x] **Step 3: Write failing settings integration tests**

Open Settings and assert three Tabs named Usage modules, Provider, and Proxy & routing. Provider controls appear only under Provider. Proxy start/stop and RouteBinding controls appear only under Proxy & routing. Closing Settings returns to the same selected dashboard module.

- [x] **Step 4: Run focused tests and observe the old single-page dialog failure**

Run:

```bash
pnpm test:unit src/components/settings/DashboardModulesSettings.test.tsx
pnpm test:unit src/components/settings/UsageProvidersSettings.test.tsx
pnpm test:unit src/components/settings/ProxyRoutingSettings.test.tsx
pnpm test:unit src/components/usage-dashboard/UsageProviderDialog.test.tsx
pnpm test:unit tests/components/SettingsDialog.test.tsx
pnpm test:unit tests/integration/SettingsDialog.test.tsx
```

Expected: FAIL because the three settings sections and module-aware Provider form do not exist.

- [x] **Step 5: Implement DashboardModulesSettings**

Render module name, type, Provider count, visibility, and order controls. Save edits through `useSaveDashboardModule`; reorder by sending the complete ordered stable-ID list; visibility through its dedicated mutation; delete only after `ConfirmDialog` confirmation and only when the client-side preconditions allow it. Always surface backend rejection text because the backend remains authoritative.

- [x] **Step 6: Implement UsageProvidersSettings and the module-aware dialog**

Move the existing Provider list and Add/Edit/Enable actions into this settings section. Pass subscription modules and an async quick-create callback into `UsageProviderDialog`. Validate a non-empty stable module ID for enabled subscriptions before `onSave`. Keep existing quota credentials, Session bindings, route config, and secret-preservation behavior unchanged.

- [x] **Step 7: Implement ProxyRoutingSettings**

Move `useIsProxyRunning`, start/stop mutations, and `RouteBindingsPanel` here. Change visible copy from `Static routes` to `Proxy forwarding targets`, with explanatory text that this determines where requests are sent and is not a dashboard filter.

- [x] **Step 8: Implement the three-section SettingsPage**

Use Radix Tabs and preserve the dialog shell. Default to `modules`, honor `defaultTab` when it is one of `modules | providers | proxy`, and map historical/unknown values to `modules` rather than mounting the dashboard inside Settings.

- [x] **Step 9: Verify Task 4**

Run all six focused commands from Step 4, then:

```bash
pnpm test:unit src/App.usage-dashboard.test.tsx
pnpm typecheck
pnpm format:check
```

Expected: all commands exit 0.

- [x] **Step 10: Commit Task 4**

```bash
git add src/components/settings src/components/usage-dashboard/UsageProviderDialog.tsx src/components/usage-dashboard/UsageProviderDialog.test.tsx tests/components/SettingsDialog.test.tsx tests/integration/SettingsDialog.test.tsx
git commit -m "feat(ui): move usage configuration into settings"
```

### Task 5: Add Preview Safety, Localized Copy, and Full Frontend Regression Coverage

**Files:**
- Modify: `src/lib/platform.ts`
- Modify: `src/main.tsx`
- Modify: `src/i18n/locales/en.json`
- Modify: `src/i18n/locales/zh.json`
- Modify: `src/i18n/locales/zh-TW.json`
- Modify: `src/i18n/locales/ja.json`
- Modify: `tests/msw/handlers.ts`
- Modify: `tests/setupGlobals.ts`
- Test: `tests/integration/App.test.tsx`
- Test: `src/App.usage-dashboard.test.tsx`

**Interfaces:**
- Consumes: completed App/Settings render tree and module API contract.
- Produces: a non-Tauri startup guard and complete localized integration fixtures.

- [x] **Step 1: Write a failing browser-preview test**

Remove `window.__TAURI_INTERNALS__`, render the startup boundary, and assert exactly one instance of:

```text
Current UI preview. Open LLM Usage Bar desktop to read local usage.
```

Assert no raw `window.__TAURI_INTERNALS__` text, no Tauri invoke error, and no Provider/module query call.

- [x] **Step 2: Extend MSW/Tauri fixtures and App integration tests**

Return five modules from `list_dashboard_modules`, including a renamed subscription module, and Providers with stable `dashboardModuleId` fields. Assert the fifth module appears without component changes, clicking it renders only its Provider, the API module shows Azure and OpenRouter, and Settings contains all configuration controls that are absent from home.

- [x] **Step 3: Run focused tests and observe missing preview/fixture behavior**

Run:

```bash
pnpm test:unit tests/integration/App.test.tsx
pnpm test:unit src/App.usage-dashboard.test.tsx
```

Expected: FAIL until runtime detection, fixtures, and App behavior are implemented.

- [x] **Step 4: Add the startup boundary**

Export:

```ts
export function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}
```

In `main.tsx`, render the normal providers/App only when this returns true. Otherwise render one localized, styled preview notice without constructing QueryClient consumers or theme/settings hooks that invoke Tauri.

- [x] **Step 5: Add aligned locale keys**

Add keys for module navigation, module/provider Tabs, settings sections, empty states, missing quota windows, absolute reset/countdown, partial/estimated/unavailable cost, aggregate-unavailable explanation, proxy-forwarding copy, retry, and preview notice. Use natural translations in all four locale files; do not rely on `defaultValue` for newly introduced production copy.

- [x] **Step 6: Run the complete frontend gate**

Run:

```bash
pnpm test:unit
pnpm typecheck
pnpm format:check
pnpm build:renderer
git diff --check
```

Expected: all commands exit 0. Record total Vitest file/test counts in the task handoff.

- [x] **Step 7: Commit Task 5**

```bash
git add src/lib/platform.ts src/main.tsx src/i18n/locales tests src/App.usage-dashboard.test.tsx
git commit -m "test(ui): cover dynamic dashboard frontend"
```

## Plan Self-Review

- Spec coverage: Tasks 1–5 cover dynamic module navigation, stable Provider membership, subscription instance switching, unified metered view, settings relocation, empty/error/preview states, accessibility, persistence, and regression tests.
- Intentional backend boundary: database defaults/migration, command implementation, true module CRUD persistence, plan labels, cross-Provider model distribution, and cross-Provider recent-request aggregation remain in the data-layer phase. The frontend exposes honest unavailable/error states instead of fabricating those values.
- Placeholder scan: every code-writing step names exact files, interfaces, commands, behavior, and verification commands.
- Type consistency: all later tasks consume `DashboardModuleView`, `DashboardModuleInput`, `dashboardModuleId`, and the Task 1 hook names exactly as declared.
