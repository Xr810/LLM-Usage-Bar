# Menu Bar Usage Status and Popover Design

**Date:** 2026-07-15
**Status:** Approved

## Summary

LLM Usage Bar will become a menu-bar-first macOS application whose persistent
presence is one colored status dot. The dot reports the most severe known usage
state across enabled, visible Agent and Provider combinations. A left click opens
a compact custom usage popover inspired by the supplied CodexBar reference. A
right click preserves the existing native Provider/Profile management menu.

The popover summarizes subscription allowance, API spending, reset timing, and
data freshness. It does not reveal the full application or its Dock icon. Only an
explicit “Open details” or “Settings” action opens the main window and changes the
macOS activation policy so the Dock icon becomes visible.

## Context

The current application already has several pieces that this design can reuse:

- `src-tauri/src/tray.rs` builds a native Tauri tray menu, refreshes usage labels,
  and applies macOS Dock policy at runtime.
- `src-tauri/src/lib.rs` creates the tray icon and currently opens its native menu
  on a left click.
- Subscription quota snapshots expose used percentages for five-hour and
  seven-day windows plus reset timestamps.
- The Agent-centric usage dashboard aggregates per-Provider tokens and USD cost
  over an arbitrary time range.
- Metered Providers already have an opaque `quota_config` JSON column, but it can
  contain fields that must not be projected wholesale to the frontend. Fixed
  Provider reconciliation also intentionally clears that column, so a durable
  user-owned budget cannot be stored there.

The current tray summary uses emoji with 70% and 90% utilization thresholds. It
does not provide a custom popover, a single worst-state icon, or a configurable
daily API budget. The new design replaces those presentation rules without
changing how external Agents choose Providers.

## Goals

- Make the menu bar communicate overall usage health without text.
- Give subscription users an early warning when remaining allowance reaches 50%
  and a critical warning below 20%.
- Give API users equivalent warnings against a per-Provider daily USD budget.
- Provide a compact, high-information popover that visually follows the supplied
  reference while remaining smaller and focused on this product’s Agent model.
- Keep the menu bar as the normal persistent macOS presence and reveal the Dock
  only after an explicit main-window action.
- Preserve existing native tray actions and Provider/Profile switching.
- Make the icon and popover consume one authoritative snapshot so their states
  cannot disagree.
- Preserve the last known alert across transient refresh failures.

## Non-goals

- Replacing the full usage dashboard.
- Automatically buying quota, changing a subscription, or disabling an API Key.
- Automatically switching an external Agent’s active Provider.
- Adding weekly or monthly API budget enforcement in this release.
- Treating a missing optional API budget as an error.
- Reworking Windows or Linux tray behavior beyond keeping it functional.
- Replacing existing quota collection, session ingestion, or cost estimation
  pipelines.

## Considered Approaches

### 1. Custom Tauri popover window

Create a dedicated, borderless webview window that is positioned beneath the
macOS status item and renders a small React application. Keep the native menu for
right-click management actions.

This is the selected approach. It can reproduce the supplied layout, reuse the
existing frontend component system, and keep the main window and Dock lifecycle
separate from menu-bar inspection.

### 2. Expanded native tray menu

Continue using Tauri native menu items and add more labels. This is operationally
simple, but native menus cannot express the reference design’s progress bars,
grouped metrics, Agent tabs, hierarchy, or freshness notices.

### 3. Temporary mini mode for the main window

Resize and reposition the main window when the tray icon is clicked. This avoids a
second webview, but it couples the popover to saved main-window geometry, Dock
visibility, title-bar behavior, and restore state. Those interactions make it
fragile and contradict the desired two-step workflow.

## Status Model

### Status values

The shared status type has four values:

- `green`: monitored usage is below its warning boundary;
- `yellow`: at least one monitored value has reached its warning boundary;
- `red`: at least one monitored value has reached its critical boundary; and
- `unknown`: no monitored source has ever produced a usable value.

When several values are available, severity is aggregated as `red > yellow >
green`. `unknown` is used only when no value can be classified. Missing data for
one Provider does not erase a valid alert from another Provider.

### Subscription allowance

Subscription quota inputs are used percentages. For every Provider:

1. Parse each available five-hour and seven-day utilization value.
2. Clamp a valid value to `0...100` for display and classification.
3. Select the highest used percentage, because it represents the window with the
   least remaining allowance.
4. Compute `remaining = 100 - highest_used`.

The boundaries are exact:

| Remaining allowance | Status |
| --- | --- |
| Greater than 50% | Green |
| 20% through 50%, inclusive | Yellow |
| Less than 20% | Red |

At exactly 50% remaining the status is yellow. At exactly 20% remaining it is
still yellow.

### Metered API spending

Every enabled metered Provider may have one optional positive daily USD budget.
Today’s cost is computed from the Mac’s local midnight through the current time.
The boundaries are exact:

| Daily budget consumed | Status |
| --- | --- |
| Less than 50% | Green |
| At least 50% and less than 80% | Yellow |
| At least 80% | Red |

An unconfigured budget excludes that Provider from status calculation but the
popover still shows today’s cost and an “Set daily budget” action. A zero,
negative, non-finite, or malformed budget is rejected rather than treated as
unlimited.

Upstream and estimated numeric costs can participate in classification. A partial
numeric cost can prove that a threshold has already been crossed, so it may
escalate to yellow or red. A partial cost below the yellow threshold cannot prove
green and is shown as incomplete instead. A fully unavailable cost is not
classified.

The rolling 30-day cost is display-only and never changes the dot.

### Freshness

A failed refresh does not replace a previously successful classification with a
lower severity. The most recent successful snapshot remains visible and the
popover marks it as stale with the last successful time and refresh error. The
icon becomes gray only before any subscription or budgeted API source has ever
produced a classifiable value.

## Tray Usage Snapshot

The backend owns one `TrayUsageSnapshot` projection used by both the tray icon and
the popover. It contains:

- aggregate status, generation time, last-success time, and stale state;
- visible Agent identities and their Provider rows;
- per-Provider subscription windows, remaining percentages, and reset times;
- today’s metered cost, daily budget, budget percentage, cost quality, token
  totals, and rolling 30-day cost;
- per-source warning or unavailable reasons; and
- whether a refresh is currently running.

The projection exposes presentation-safe values only. It never contains API Keys,
OAuth tokens, raw `quota_config`, route credentials, credential fingerprints, or
raw upstream payloads.

The backend supplies two focused commands:

1. `get_tray_usage_snapshot` returns the latest cache-backed projection quickly.
2. `refresh_tray_usage` refreshes eligible quota/session sources, rebuilds the
   projection, updates the icon, and emits a `tray-usage-updated` event.

The popup shows the cache-backed snapshot immediately, then requests an async
refresh. Existing debounce rules prevent repeated clicks from creating refresh
storms.

## Daily Budget Persistence

Add a nullable `daily_budget_usd TEXT` column to `usage_providers`. The value is a
canonical positive decimal string owned by the user, independent of upstream
quota collection configuration. A dedicated column is required because system
Provider reconciliation currently resets `quota_config` to canonical null values
at startup; storing the budget there would lose the user’s setting.

The additive schema migration leaves existing rows null. Fixed Provider catalog
reconciliation and ordinary Provider saves must preserve the budget unless the
dedicated budget command changes it.

The frontend must not read or write the whole `quota_config` object. That object
may contain unrelated sensitive collection settings. Instead:

- the safe Provider view exposes only `dailyBudgetUsd` from the dedicated column;
- a dedicated `set_provider_daily_budget` command validates and canonicalizes a
  decimal string;
- clearing the budget sets only `daily_budget_usd` to null; and
- command arguments, logs, diagnostics, exports, and events never include raw
  `quota_config`.

Budget editing lives in the main Provider settings experience, not inside a
free-form JSON editor. The popover may deep-link to that control.

## Update Lifecycle

The app rebuilds the snapshot and icon at these points:

- after application state and persisted quota snapshots are available at startup;
- after a scheduled quota refresh succeeds or fails;
- after new metered usage is ingested, with a short debounce;
- after a Provider budget, enabled state, binding, or visible Agent changes;
- after the user requests refresh from the popover; and
- at local midnight, when today’s API cost window resets.

Snapshot generation must be side-effect free. Refreshing data and projecting data
remain separate so the application can always repaint from persisted state even
when upstream services are offline.

## macOS Tray and Window Behavior

### Status dot

The macOS tray uses four bundled PNG assets: green, yellow, red, and neutral gray.
Each asset is an 18-by-18 transparent canvas containing one approximately 10-pixel
circle. The colored assets are not marked as macOS template images, because a
template image would discard the status color. The tooltip names the application
and current state for accessibility.

### Mouse behavior

- Left click toggles the custom popover and never opens the native menu.
- Right click keeps the existing native menu and all current Provider, Profile,
  lightweight-mode, website, and quit actions.
- A second left click, Escape, or loss of focus hides the popover.

### Popover window

The backend lazily creates a window labeled `tray-popover` with these macOS
properties:

- approximately 380 pixels wide and at most 520 pixels tall;
- undecorated, non-resizable, always on top, and absent from the task switcher;
- hidden at startup and excluded from saved main-window geometry; and
- positioned beneath the clicked status item and clamped to the active monitor’s
  work area.

Long content scrolls inside the summary region while Agent navigation and footer
actions remain reachable within the fixed window bounds.

Opening or closing this window does not change the application’s Accessory
activation policy and does not show the Dock icon.

The frontend entrypoint renders `TrayUsagePopover` when the current window label is
`tray-popover`; the normal `App` remains unchanged for the `main` label. The two
window surfaces share DTO types and presentation utilities but do not share
window-local React state.

### Main-window actions

“Open details” hides the popover, opens the main usage dashboard for the selected
Agent, applies the regular activation policy, and focuses the main window.
“Settings” follows the same sequence but opens the Provider budget control.

Closing or minimizing the main window back to the tray restores Accessory policy
according to the existing runtime tray policy. The menu-bar dot remains available
throughout.

## Popover Information Architecture

The reference image establishes the visual hierarchy rather than a requirement to
copy every row. The compact popover contains:

1. **Agent navigation:** Overview followed by visible fixed and custom Agents in a
   horizontally scrollable tab row.
2. **Account header:** selected Agent or overview title, last successful update,
   and stale/partial badges.
3. **Subscription section:** Provider name, plan label when available, a progress
   bar for each available window, remaining percentage, and reset time.
4. **API spending section:** today’s USD cost, daily budget progress, rolling
   30-day USD cost, token total, and complete/estimated/partial status.
5. **Footer actions:** Open details, Refresh, Settings, and Quit.

Overview aggregates all visible Agents but keeps Provider rows distinct enough to
explain which source caused yellow or red. Selecting an Agent filters the same
snapshot without triggering a new backend query.

The visual treatment follows macOS dark popover conventions: a near-black surface,
subtle border, rounded corners, restrained separators, compact typography, and
muted cyan/orange progress accents. It uses the existing icon libraries and
product/Agent assets rather than emoji or improvised CSS icons. The layout must
also remain legible in the application’s supported light appearance.

## Failure and Edge-case Behavior

- An invalid quota percentage is ignored and reported in the popover; it cannot
  create a false green state.
- A reset timestamp in the past is shown as pending refresh rather than a negative
  duration.
- A missing daily budget is a normal configuration state.
- A Provider with a budget but no usable cost shows unavailable and links to
  details; it does not assume zero spend.
- A refresh already in progress disables repeated refresh actions.
- Upstream failure preserves the last successful alert and records a sanitized
  error message.
- If the status icon asset cannot be loaded, the app retains the previous icon and
  logs a non-secret diagnostic rather than removing the status item.
- If the custom window cannot be created, right-click native actions remain
  available and the failure is logged.
- Multi-monitor positioning clamps both axes and never persists popover position
  as main-window state.
- Local-midnight recomputation handles time-zone and daylight-saving changes by
  deriving the next boundary from the current local calendar, not by adding a
  fixed 24 hours.

## Accessibility

- Color is not the only indicator inside the popover: every status includes text,
  percentages, and progress labels.
- The tray tooltip exposes Green, Warning, Critical, or Data unavailable.
- All actions are keyboard reachable; Escape closes the window and focus starts on
  the selected Agent tab or first summary heading.
- Progress bars expose accessible names and numeric values.
- Text and controls meet the existing application contrast and focus-ring rules.

## Testing Strategy

Implementation follows test-driven development.

### Pure status tests

- subscription boundaries immediately above/below and exactly at 20% and 50%
  remaining;
- API boundaries immediately above/below and exactly at 50% and 80% consumed;
- worst-state aggregation across windows, Providers, and Agents;
- malformed percentages, invalid budgets, missing budgets, complete/estimated/
  partial/unavailable costs, and stale snapshots; and
- local-midnight and rolling-30-day query boundaries.

### Backend tests

- the additive budget migration preserves all existing Provider data;
- system Provider reconciliation and ordinary Provider saves preserve the budget;
- safe views and events never serialize raw `quota_config` or credentials;
- snapshot projection uses the same classification returned to the tray icon;
- refresh failure retains the last successful snapshot;
- icon mapping and event emission occur after relevant mutations; and
- click handling distinguishes left and right mouse buttons.

### Frontend tests

- overview and Agent filters render the supplied snapshot correctly;
- subscription and API rows expose status text, reset times, budget progress, and
  data-quality badges;
- loading, refreshing, stale, partial, unavailable, and empty states;
- Open details and Settings target the correct main-window destination; and
- Escape and blur request popover dismissal.

### Runtime verification

On macOS, build and relaunch the real Tauri application, then verify:

- green, yellow, red, and gray dot assets at menu-bar scale in light and dark menu
  bars;
- left-click toggle, right-click native menu, Escape, and click-away behavior;
- positioning on the primary display and at least one secondary-display edge;
- the Dock remains hidden while inspecting the popover;
- Open details and Settings reveal and focus the main window and Dock icon;
- status changes after quota, spend, budget, and local-day changes; and
- no browser-console or Rust errors occur during the primary interaction flow.

## Acceptance Criteria

The feature is complete only when all of the following are true:

1. macOS shows one colored circular status item with no text or legacy glyph.
2. Subscription and API thresholds match the exact approved boundaries.
3. The overall dot always reflects the most severe classifiable visible source.
4. A left click opens the compact custom popover without showing the Dock.
5. A right click preserves the full existing native tray menu.
6. The popover shows subscription remaining allowance, reset timing, today’s API
   cost, daily budget progress, rolling 30-day cost, and freshness.
7. API budgets persist per metered Provider across restart and system catalog
   reconciliation without exposing or modifying `quota_config`.
8. Refresh failure preserves the last successful warning or critical state and
   marks it stale.
9. Open details and Settings are the only popover actions that reveal the main
   window and Dock.
10. Automated status, persistence, backend projection, and frontend rendering
    tests pass, followed by successful real macOS interaction and visual QA.
