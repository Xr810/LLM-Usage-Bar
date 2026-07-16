# Agent Provider Switching Frontend and Native Integration Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose verified per-Agent Provider switching, Custom Agent setup, model editing, guarded mutations, sync review, and native-menu fallback in the App without coupling switching to usage history or changing the usage-only left-click tray popover.

**Architecture:** One typed `AgentSwitchView` is the renderer's coherent observation frame; backend-authored `selection` and `switchability` remain the sole source of truth. A controller renders above usage empty states, Settings separates Bound/Available/Current, the Custom Agent wizard persists backend drafts, and the post-menu main-window destination channel carries right-click attention to the correct controller when native quick apply cannot safely proceed.

**Tech Stack:** React 18, TypeScript, TanStack Query, Tauri invoke/events, Radix/shadcn components, Tailwind, i18next, Vitest, Testing Library, MSW/Tauri mocks.

**Depends on:** Gate 3 in [`2026-07-16-agent-provider-switching-custom-agents.md`](2026-07-16-agent-provider-switching-custom-agents.md) and the post-menu renderer baseline recorded by the master preflight.

---

## Frontend Invariants

- Renderer never derives Current or switchability from enabled/effective/credential/model/health fields.
- `AgentSwitchView` carries state and binding rows from one backend observation frame. Do not join separate query timestamps to decide “In use.”
- Preview tokens exist only in transient component/native-attention state. Never put them in Query cache, localStorage, URL, persisted store, telemetry, or error text.
- Usage history can be empty while the current-Provider controller remains fully usable.
- Changing Agent tab, sorting/hiding Agent, opening Settings, refreshing usage, or using the left-click popover causes zero switch IPC.
- Pending state is scoped by Agent/operation. Agent A switching does not freeze Agent B or navigation.
- Current/uncertain mutation buttons may be disabled for clarity, but backend preview/apply guards remain mandatory.
- Custom path/file safety, ownership, editor compatibility, and switchability are backend decisions. Frontend form checks only required shape.
- Existing root renderer WIP and menu-popover WIP must already be committed/merged as separate baseline work. Never reintroduce their old diff as part of these commits.
- `TrayUsagePopover*`, `windowSurface` routing, `show_menu_on_left_click(false)`, and left-click usage-only behavior remain intact.

## File Responsibility Map

### Create

- `src/types/agentSwitching.ts`
- `src/lib/api/agentSwitching.ts`
- `src/lib/query/agentSwitching.ts`
- `src/lib/query/agentSwitching.test.tsx`
- `src/components/agent-switching/{AgentProviderController.tsx,AgentProviderController.test.tsx,SwitchPreviewDialog.tsx,SwitchPreviewDialog.test.tsx,SwitchStateSummary.tsx,DefaultModelEditor.tsx,DefaultModelEditor.test.tsx,GuardedMutationDialog.tsx,GuardedMutationDialog.test.tsx,LegacyCredentialUpgradeDialog.tsx,LegacyCredentialUpgradeDialog.test.tsx}`
- `src/components/settings/custom-agent-adapter/{CustomAgentAdapterWizard.tsx,CustomAgentAdapterWizard.test.tsx,AdapterTargetStep.tsx,AdapterMappingStep.tsx,AdapterReviewStep.tsx,adapterDraftForm.ts,adapterDraftForm.test.ts}`
- `src/components/settings/sync-review/{SwitchSensitiveSyncBanner.tsx,SwitchSensitiveSyncDialog.tsx,SwitchSensitiveSyncItem.tsx,SwitchSensitiveSyncDialog.test.tsx}`
- `src/i18n/{agentSwitchingCopy.ts,agentSwitchingCopy.test.ts}`
- `src/components/settings/AgentProviderBindingRow.test.tsx`

### Modify

- `src/types/{usageDashboard.ts,trayUsage.ts}`
- `src/lib/api/usageDashboard.ts`
- `src/lib/query/{usageDashboard.ts,usageDashboard.test.tsx}`
- `tests/msw/{handlers.ts,state.ts,tauriMocks.ts}`
- `src/components/usage-dashboard/{UsageDashboardPage.tsx,UsageDashboardPage.test.tsx,UsageProviderDialog.tsx,UsageProviderDialog.test.tsx}`
- `src/components/settings/{DashboardModulesSettings.tsx,DashboardModulesSettings.test.tsx,AgentProviderBindingRow.tsx,UsageProvidersSettings.tsx,UsageProvidersSettings.test.tsx,UsageProvidersSettings.system.test.tsx,SystemProviderCard.tsx,SystemProviderCard.test.tsx,SystemProviderAgentBindings.tsx,SystemProviderAgentBindings.test.tsx,SettingsPage.tsx}`
- Post-menu: `src/hooks/{useMainWindowNavigation.ts,useMainWindowNavigation.test.tsx}`
- `src/{App.tsx,App.usage-dashboard.test.tsx}`
- `tests/components/SettingsDialog.test.tsx`
- `tests/integration/SettingsDialog.test.tsx`
- `src/i18n/locales/{en.json,zh.json,zh-TW.json,ja.json}`
- `src/i18n/dashboardCopy.test.ts`

## Task 1: Add the exact TypeScript/API/query contract and coherent fixtures

**Files:**

- Create: `src/types/agentSwitching.ts`
- Create: `src/lib/api/agentSwitching.ts`
- Create: `src/lib/query/{agentSwitching.ts,agentSwitching.test.tsx}`
- Modify: `src/types/usageDashboard.ts`
- Modify: `src/lib/api/usageDashboard.ts`
- Modify: `src/lib/query/{usageDashboard.ts,usageDashboard.test.tsx}`
- Modify: `tests/msw/{handlers.ts,state.ts,tauriMocks.ts}`

**Produces:** a stable renderer mirror of the already-green backend DTOs and IPC commands.

- [ ] **Step 1: Write failing exact-command and coherent-frame tests**

Mock every command and assert exact `{ input }` shape, camelCase response mapping, query keys, invalidation scope, and error behavior. Prove:

- `useAgentSwitchView(agentId)` returns one state/bindings frame;
- apply never sends projection values, paths, model derived by UI, or token outside its input;
- preview token is not placed in Query data by mutation success handlers;
- Agent A mutation state is distinguishable from Agent B;
- success/failure invalidates Agent frame, relevant binding list, Providers, and Agents only when needed;
- a mismatched `agentModuleId` response rejects rather than rendering another Agent;
- safe fixtures contain no credential/fingerprint/digest/slot/recovery data.

- [ ] **Step 2: Confirm red**

```bash
pnpm test:unit src/lib/query/agentSwitching.test.tsx
pnpm test:unit src/lib/query/usageDashboard.test.tsx
```

- [ ] **Step 3: Define exhaustive types**

Implement:

```ts
export type AgentConfigState =
  | "in_sync"
  | "drifted"
  | "missing"
  | "setup_required"
  | "unmanaged"
  | "partial";

export type AgentRouteHealth =
  | "usable"
  | "unknown"
  | "proxy_down"
  | "auth_missing"
  | "credential_unavailable"
  | "upstream_unhealthy";

export type BindingCredentialMode =
  "official" | "provider_split" | "legacy_combined";

export interface AgentSwitchView {
  agentModuleId: string;
  state: AgentSwitchStateView;
  bindings: AgentProviderBindingView[];
  observedAt: number;
}
```

Also define `AgentSwitchActivity`, `SwitchabilityState`, all stable reason/action unions from the master plan, `EvaluatedSwitchVersions`, `BindingSwitchabilityView`, `AgentBindingSwitchPreview`, `AgentBindingSwitchInput`, `AgentBindingSwitchResult`, guarded mutation DTOs, affected-Agent view, Custom draft/validation DTOs, and switch-sensitive sync DTOs.

Extend `AgentProviderBindingView` with explicit `defaultModel`, `upstreamModel`, `switchConfigVersion`, `credentialMode`, `selection: "current" | "last_verified" | "none"`, adapter/protocol summary, and backend-authored `switchability`. Preserve `enabled`, `effectiveEnabled`, and credential status as different concepts.

- [ ] **Step 4: Implement API and query hooks**

Wire these exact command names:

```text
get_agent_switch_view
refresh_agent_switch_state
preview_agent_binding_switch
get_agent_binding_switch_preview
apply_agent_binding_switch
preview_agent_disconnect
apply_agent_disconnect
preview_guarded_agent_mutation
apply_guarded_agent_mutation
get_custom_agent_adapter_draft
save_custom_agent_adapter_draft
discard_custom_agent_adapter_draft
validate_custom_agent_adapter_draft
promote_custom_agent_adapter_draft
list_switch_sensitive_sync_mutations
preview_switch_sensitive_sync_mutation
resolve_switch_sensitive_sync_mutation
preview_legacy_credential_upgrade
apply_legacy_credential_upgrade
```

Create hooks `useAgentSwitchView`, `useRefreshAgentSwitchState`, `usePreviewAgentBindingSwitch`, `useApplyAgentBindingSwitch`, `useAgentDisconnectFlow`, `useGuardedAgentMutation`, `useCustomAgentAdapterDraft`, `useCustomAgentAdapterDraftActions`, `useSwitchSensitiveSyncMutations`, `useResolveSwitchSensitiveSyncMutation`, and `useAgentSwitchEventBridge`.

- [ ] **Step 5: Add coherent MSW/Tauri state fixtures**

Store one `AgentSwitchView` per stable Agent ID. A simulated switch updates current/last/state/version/binding selection atomically before emitting an invalidation event. Add helpers for drift, stale preview, rollback, partial, unhealthy current, setup-required, sync conflict, and guarded affected-Agent results.

- [ ] **Step 6: Run and commit**

```bash
pnpm test:unit src/lib/query/agentSwitching.test.tsx
pnpm test:unit src/lib/query/usageDashboard.test.tsx
pnpm typecheck
git add src/types src/lib/api src/lib/query tests/msw
git diff --cached --check
git commit -m "feat: add agent switching frontend contracts"
```

## Task 2: Add exhaustive localized state/reason/action copy

**Files:**

- Create: `src/i18n/{agentSwitchingCopy.ts,agentSwitchingCopy.test.ts}`
- Modify: `src/i18n/locales/{en.json,zh.json,zh-TW.json,ja.json}`
- Modify: `src/i18n/dashboardCopy.test.ts`

**Produces:** typed labels and help text for every backend enum and flow.

**Kimi eligibility:** This task may be delegated only after Task 1 unions are committed and `kimi_delegation_status` permits it. Codex must review all four locales and exhaustive tests, then apply/retest under `AGENTS.md`.

- [ ] **Step 1: Write failing exhaustive-copy tests**

Use `satisfies Record<Union, string>` for config state, route health, switchability, all reason codes, repair actions, sync actions, and draft format/protocol labels. Load four locales and assert each required key is non-empty, not raw backend English/code, and contains no credential/token placeholder.

- [ ] **Step 2: Confirm red**

```bash
pnpm test:unit src/i18n/agentSwitchingCopy.test.ts
```

- [ ] **Step 3: Implement typed copy maps and locales**

Keep reason and action copy separate so the UI can explain why and label what happens next. Add complete Custom wizard, preview, ownership, Disconnect, legacy upgrade, affected-Agent guard, recovery, and sync-review strings.

- [ ] **Step 4: Run and commit**

```bash
pnpm test:unit src/i18n/agentSwitchingCopy.test.ts
pnpm test:unit src/i18n/dashboardCopy.test.ts
pnpm format:check
git add src/i18n
git diff --cached --check
git commit -m "feat: localize agent provider switching"
```

## Task 3: Render the current-Provider controller above usage history

**Files:**

- Create: `src/components/agent-switching/{AgentProviderController.tsx,AgentProviderController.test.tsx,SwitchPreviewDialog.tsx,SwitchPreviewDialog.test.tsx,SwitchStateSummary.tsx,DefaultModelEditor.tsx,DefaultModelEditor.test.tsx}`
- Modify: `src/components/usage-dashboard/{UsageDashboardPage.tsx,UsageDashboardPage.test.tsx}`
- Modify: `src/App.tsx`
- Modify: `src/App.usage-dashboard.test.tsx`

**Produces:** dashboard selection/status/switch/reapply/repair UI independent of history.

**Kimi eligibility:** `SwitchStateSummary` and purely presentational redacted preview rows may be delegated after props/fixtures are fixed. Command flow, token lifecycle, pending scope, and state transitions remain Codex-owned.

- [ ] **Step 1: Write failing controller placement/state/action tests**

Cover:

- controller appears for a new Agent with no usage history and before loading/empty branches;
- verified current + default model + config state + route health render separately;
- unhealthy route retains `In use` and offers backend repair action;
- drift/missing/unmanaged/setup-required/partial shows Current unknown, removes `In use`, and may show last verified as non-authoritative;
- `effectiveEnabled=true` plus backend blocked remains blocked;
- ready non-current → Switch, ready current → Reapply, confirmation → preview, blocked → exact repair;
- Agent A pending allows Agent B navigation/action;
- tab switch/sort/hide/usage refresh emits zero switch IPC;
- success updates only after returned/refetched verified frame;
- rollback retains old current; partial clears managed current; stale preview is discarded and regenerated;
- existing post-menu `UsageDashboardPage` error deduplication remains.

- [ ] **Step 2: Confirm red**

```bash
pnpm test:unit src/components/agent-switching/AgentProviderController.test.tsx
pnpm test:unit src/components/agent-switching/SwitchPreviewDialog.test.tsx
pnpm test:unit src/components/usage-dashboard/UsageDashboardPage.test.tsx
```

- [ ] **Step 3: Implement controller and transient preview flow**

`UsageDashboardPage` renders `AgentProviderController` immediately after its Agent header and before any usage projection branch. The controller trusts `binding.selection`/`binding.switchability`, keeps preview token only in dialog state, re-reads prepared preview by token, and refreshes state on all terminal results.

- [ ] **Step 4: Implement default model editing**

Non-current binding sends guarded metadata Save. Current binding displays `Save & reapply`: preview/apply freezes the model version and publishes metadata only with verified live reapply. Missing/invalid model focuses the editor from `edit_model` repair. When the detector supplies one exact redacted model candidate, expose a separate `Adopt detected model` action that sends that explicit value/version to the backend; never auto-populate or publish a guessed model.

- [ ] **Step 5: Run and commit**

```bash
pnpm test:unit src/components/agent-switching/AgentProviderController.test.tsx
pnpm test:unit src/components/agent-switching/SwitchPreviewDialog.test.tsx
pnpm test:unit src/components/agent-switching/DefaultModelEditor.test.tsx
pnpm test:unit src/components/usage-dashboard/UsageDashboardPage.test.tsx
pnpm test:unit src/App.usage-dashboard.test.tsx
git add src/components/agent-switching src/components/usage-dashboard \
  src/App.tsx src/App.usage-dashboard.test.tsx
git diff --cached --check
git commit -m "feat: add agent provider dashboard controller"
```

## Task 4: Replace name-only Custom Agent creation with the draft wizard

**Files:**

- Create: `src/components/settings/custom-agent-adapter/{CustomAgentAdapterWizard.tsx,CustomAgentAdapterWizard.test.tsx,AdapterTargetStep.tsx,AdapterMappingStep.tsx,AdapterReviewStep.tsx,adapterDraftForm.ts,adapterDraftForm.test.ts}`
- Modify: `src/components/settings/{DashboardModulesSettings.tsx,DashboardModulesSettings.test.tsx,SettingsPage.tsx}`

**Produces:** initial/create and version-bound edit draft UX over backend-authoritative validation/promotion.

**Kimi eligibility:** pure DTO↔form mapping/tests and visual step components may be delegated after contracts/fixtures are fixed. Saving, promotion, path-picker integration, preview confirmation, state/version handling, and merge with baseline WIP remain Codex-owned.

- [ ] **Step 1: Write failing form mapping and wizard lifecycle tests**

Cover initial Cancel with zero IPC/Agent, Save draft returning backend Agent ID, close/resume at exact step, discard leaving setup-required, existing setup-required migration, generic Custom icon, protocol/four-format choices, multi-target mappings and required logical roles, file picker/directory picker string collection, backend validation messages, missing-target confirmation/cancel zero writes, redacted preview secret sentinel absence, promotion failure staying non-switchable, edit base-version conflict, discard edits preserving active adapter/current, and active edit requiring migration preview or Disconnect.

- [ ] **Step 2: Confirm red**

```bash
pnpm test:unit src/components/settings/custom-agent-adapter/adapterDraftForm.test.ts
pnpm test:unit src/components/settings/custom-agent-adapter/CustomAgentAdapterWizard.test.tsx
pnpm test:unit src/components/settings/DashboardModulesSettings.test.tsx
```

- [ ] **Step 3: Implement initial and edit flows**

Preserve the baseline `AgentsSettings`/Agent sorter. Replace the name-only create action with wizard open. Before Save, keep state local. After Save, use backend ID/draft version. `Resume setup`, `Discard draft`, `Edit adapter`, and `Discard edits` always refetch backend draft/Agent state.

- [ ] **Step 4: Keep safety decisions in backend**

Reuse `settingsApi.openFileDialog()` and `settingsApi.pickDirectory()` only for selection. Renderer must not label a path safe, infer preservability, calculate ownership, parse credentials, or mark switchable. Show backend redacted target summary/validation/confirmation reasons verbatim through localized presentation.

- [ ] **Step 5: Run and commit**

```bash
pnpm test:unit src/components/settings/custom-agent-adapter/adapterDraftForm.test.ts
pnpm test:unit src/components/settings/custom-agent-adapter/CustomAgentAdapterWizard.test.tsx
pnpm test:unit src/components/settings/DashboardModulesSettings.test.tsx
git add src/components/settings/custom-agent-adapter \
  src/components/settings/DashboardModulesSettings.tsx \
  src/components/settings/DashboardModulesSettings.test.tsx \
  src/components/settings/SettingsPage.tsx
git diff --cached --check
git commit -m "feat: add custom agent adapter wizard"
```

## Task 5: Separate Bound, Available, Switchability, and Current in Agents Settings

**Files:**

- Create: `src/components/settings/AgentProviderBindingRow.test.tsx`
- Create: `src/components/agent-switching/{GuardedMutationDialog.tsx,GuardedMutationDialog.test.tsx,LegacyCredentialUpgradeDialog.tsx,LegacyCredentialUpgradeDialog.test.tsx}`
- Modify: `src/components/settings/{AgentProviderBindingRow.tsx,DashboardModulesSettings.tsx,DashboardModulesSettings.test.tsx}`

**Produces:** explicit relationship/availability/current/model/upgrade/Disconnect semantics.

- [ ] **Step 1: Write failing binding-row and guard tests**

Assert separate Bound/Unbind, Available toggle, effective label, backend switchability/reason/action, current/last/none selection, default/upstream model, Save versus Save & reapply, current/uncertain disable/unbind/model/credential clear requiring guard, affected Agent list, Custom setup/edit/validation/target summary/Disconnect, and legacy-combined upgrade requirements. A disabled button alone is not accepted as command protection; mocks prove guarded preview is called.

- [ ] **Step 2: Confirm red**

```bash
pnpm test:unit src/components/settings/AgentProviderBindingRow.test.tsx
pnpm test:unit src/components/agent-switching/GuardedMutationDialog.test.tsx
pnpm test:unit src/components/agent-switching/LegacyCredentialUpgradeDialog.test.tsx
```

- [ ] **Step 3: Implement binding semantics and guarded flows**

Do not reuse `src/lib/query/mutations.ts::useSwitchProviderMutation`; that remains unmanaged legacy-only. All managed switch/reapply/Disconnect/upgrade paths use `agentSwitching` hooks and fresh backend previews. The current binding cannot be disabled/unbound/cleared; uncertain last-verified references also require the backend's allowed path.

- [ ] **Step 4: Run and commit**

```bash
pnpm test:unit src/components/settings/AgentProviderBindingRow.test.tsx
pnpm test:unit src/components/agent-switching/GuardedMutationDialog.test.tsx
pnpm test:unit src/components/agent-switching/LegacyCredentialUpgradeDialog.test.tsx
pnpm test:unit src/components/settings/DashboardModulesSettings.test.tsx
git add src/components/settings src/components/agent-switching
git diff --cached --check
git commit -m "feat: expose live provider state in agent settings"
```

## Task 6: Guard Provider-side global mutations by affected Agents

**Files:**

- Modify: `src/components/settings/{UsageProvidersSettings.tsx,UsageProvidersSettings.test.tsx,UsageProvidersSettings.system.test.tsx,SystemProviderCard.tsx,SystemProviderCard.test.tsx,SystemProviderAgentBindings.tsx,SystemProviderAgentBindings.test.tsx}`
- Modify: `src/components/usage-dashboard/{UsageProviderDialog.tsx,UsageProviderDialog.test.tsx}`

**Produces:** Provider page Bound/Available/Current rows plus mandatory affected-Agent previews for global changes.

- [ ] **Step 1: Write failing Provider-side semantics/guard tests**

Cover real Agent names including Custom (remove static `AGENT_NAMES`), Bound checkbox separate from availability, effective/switchability/current per Agent, and backend affected-Agent guard for Provider disable/delete, upstream credential clear/replace, official logout, binding unbind/disable, and legacy upgrade. Prove `canClearUpstreamCredential=false` is presentation only, not the only protection; proof mismatch offers separate account and never merge. New custom API Provider setup asks for one Provider-scoped upstream credential and shows that each binding gets an independently generated local key; it never asks the user to reuse one secret for both roles.

- [ ] **Step 2: Confirm red**

```bash
pnpm test:unit src/components/settings/SystemProviderAgentBindings.test.tsx
pnpm test:unit src/components/settings/SystemProviderCard.test.tsx
pnpm test:unit src/components/settings/UsageProvidersSettings.system.test.tsx
pnpm test:unit src/components/settings/UsageProvidersSettings.test.tsx
```

- [ ] **Step 3: Implement Provider-side guard flows**

Resolve Agent labels from Agent query/backend affected view. Every sensitive action previews first, lists current/uncertain Agents and blockers, and invokes only allowed apply/separate-account/Disconnect path. Ordinary safe display edits continue normally.

- [ ] **Step 4: Run and commit**

```bash
pnpm test:unit src/components/settings/SystemProviderAgentBindings.test.tsx
pnpm test:unit src/components/settings/SystemProviderCard.test.tsx
pnpm test:unit src/components/settings/UsageProvidersSettings.system.test.tsx
pnpm test:unit src/components/settings/UsageProvidersSettings.test.tsx
pnpm test:unit src/components/usage-dashboard/UsageProviderDialog.test.tsx
git add src/components/settings src/components/usage-dashboard/UsageProviderDialog.tsx \
  src/components/usage-dashboard/UsageProviderDialog.test.tsx
git diff --cached --check
git commit -m "feat: guard provider mutations by affected agents"
```

## Task 7: Add the shared switch-sensitive sync review queue

**Files:**

- Create: `src/components/settings/sync-review/{SwitchSensitiveSyncBanner.tsx,SwitchSensitiveSyncDialog.tsx,SwitchSensitiveSyncItem.tsx,SwitchSensitiveSyncDialog.test.tsx}`
- Modify: `src/components/settings/SettingsPage.tsx`
- Modify: `tests/components/SettingsDialog.test.tsx`
- Modify: `tests/integration/SettingsDialog.test.tsx`

**Produces:** one Settings-level queue shared by Agents and Providers tabs.

- [ ] **Step 1: Write failing allowed-action/durability tests**

Assert source installation label, causal version, redacted field summary, affected Agents, blockers, and backend `allowedActions`. Render only `apply_remote`, `keep_local`, or `create_separate_account` when allowed. Prove no merge on account mismatch, keep-local sends mutation ID + causal version, apply remote re-detects/re-previews, stale token cannot apply, incoming change did not alter current before review, and item disappears only after durable success + refetch.

- [ ] **Step 2: Confirm red**

```bash
pnpm test:unit src/components/settings/sync-review/SwitchSensitiveSyncDialog.test.tsx
pnpm test:unit tests/components/SettingsDialog.test.tsx
pnpm test:unit tests/integration/SettingsDialog.test.tsx
```

- [ ] **Step 3: Implement one shared banner/dialog**

Mount the query/banner at Settings level so Agents/Providers do not maintain competing local queues. Apply reuses normal guarded preview/reapply/Disconnect; it never calls raw metadata save. Preserve the baseline Settings close/navigation behavior.

- [ ] **Step 4: Run and commit**

```bash
pnpm test:unit src/components/settings/sync-review/SwitchSensitiveSyncDialog.test.tsx
pnpm test:unit tests/components/SettingsDialog.test.tsx
pnpm test:unit tests/integration/SettingsDialog.test.tsx
git add src/components/settings/sync-review src/components/settings/SettingsPage.tsx \
  tests/components/SettingsDialog.test.tsx tests/integration/SettingsDialog.test.tsx
git diff --cached --check
git commit -m "feat: review switch-sensitive sync changes"
```

## Task 8: Route native right-click attention into the main controller and complete Gate 4

**Files:**

- Modify: post-menu `src/types/trayUsage.ts`
- Modify: post-menu `src/hooks/{useMainWindowNavigation.ts,useMainWindowNavigation.test.tsx}`
- Modify: `src/{App.tsx,App.usage-dashboard.test.tsx}`
- Modify: `src/components/agent-switching/AgentProviderController.tsx`
- Test: post-menu `src/windowSurface.test.tsx`
- Test: post-menu `src/components/tray-usage/TrayUsagePopover.test.tsx`

**Produces:** one-shot Agent switch destination/attention; quick success stays native, confirmation/error goes to the correct Agent controller.

- [ ] **Step 1: Write failing cold/warm navigation and popover regression tests**

Cover:

- `MainWindowDestination` union adds `{ kind: "agentSwitch", agentModuleId, bindingId, previewToken }`;
- destination waits until stable Agent ID exists, never selects a fallback Agent when hidden/removed;
- valid prepared token is fetched; stale token is discarded and freshly previewed;
- no-token error/partial refetches state and shows backend reason;
- attention is consumed once in warm event and cold pending-destination paths;
- quick success does not open main;
- left-click continues to render only `TrayUsagePopover` and invokes no switch command;
- right-click fallback does not change `show_menu_on_left_click(false)` or `windowSurface` choice.

- [ ] **Step 2: Confirm red**

```bash
pnpm test:unit src/hooks/useMainWindowNavigation.test.tsx
pnpm test:unit src/App.usage-dashboard.test.tsx
pnpm test:unit src/windowSurface.test.tsx
pnpm test:unit src/components/tray-usage/TrayUsagePopover.test.tsx
```

- [ ] **Step 3: Extend the existing post-menu destination channel**

Do not create a second event/pending store. Extend Rust/TypeScript union already used by `main-window-navigate` and `take_pending_main_window_destination`. App selects the exact Agent only after query hydration, then passes one-shot attention to `AgentProviderController`, which consumes and clears it after opening/refetching the intended flow.

- [ ] **Step 4: Run Gate 4**

```bash
pnpm test:unit src/lib/query/agentSwitching.test.tsx
pnpm test:unit src/components/agent-switching/AgentProviderController.test.tsx
pnpm test:unit src/components/settings/custom-agent-adapter/CustomAgentAdapterWizard.test.tsx
pnpm test:unit src/components/settings/AgentProviderBindingRow.test.tsx
pnpm test:unit src/components/settings/UsageProvidersSettings.test.tsx
pnpm test:unit src/components/settings/sync-review/SwitchSensitiveSyncDialog.test.tsx
pnpm test:unit src/hooks/useMainWindowNavigation.test.tsx
pnpm test:unit src/components/tray-usage/TrayUsagePopover.test.tsx
pnpm typecheck
pnpm format:check
pnpm build:renderer
pnpm test:unit --exclude '.worktrees/**'
```

- [ ] **Step 5: Commit and record Gate 4 evidence**

```bash
git add src/types/trayUsage.ts src/hooks src/App.tsx src/App.usage-dashboard.test.tsx \
  src/components/agent-switching/AgentProviderController.tsx
git diff --cached --check
git commit -m "feat: route native switch attention to the main controller"
```

Before moving to release acceptance, inspect `git log --stat` and prove the original root WIP and menu-popover WIP are present only through their own baseline commits, not silently folded into one of these feature commits.
