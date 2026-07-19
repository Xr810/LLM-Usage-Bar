# Codex Manual Weekly Reset Credits — Design QA

Date: 2026-07-19
Branch: `codex/manual-reset-credits`

Source visual truth:

- `/var/folders/th/28ml58qj663d_8tbmcz417t00000gn/T/TemporaryItems/NSIRD_screencaptureui_dbMT8k/截屏2026-07-19 23.30.57.png`

Implementation comparison:

- `qa-artifacts/manual-reset-credits-comparison.png`

## Result

The corrected implementation presents Codex reset credits as manually consumable
weekly-limit resets, not as additional quota windows. The collapsed row shows
`使用限额重置` and the authoritative `可用 3 次` count. Expanding it reveals three
`Full reset` entries with localized expiry dates of July 27, August 1, and
August 13. The existing secondary subscription window remains labeled `周额度`.

No actionable P0, P1, or P2 visual, interaction, copy, or accessibility issue
remains. The implementation intentionally uses the existing Provider card and
design tokens instead of cloning the Codex modal. The screenshot's destructive
`使用重置` buttons are intentionally absent because this feature is read-only and
must not consume a reset credit.

## Evidence

- The supplied Codex screenshot and the expanded LLM Usage Bar card were
  rendered side by side in the same `1280 × 720` dark-theme comparison input.
- The collapsible control exposes the accessible name
  `展开或收起 3 次使用限额重置` and correctly reports the expanded state.
- DOM verification confirmed all three `Full reset` rows and their exact
  localized expiry dates.
- The initial keyboard-focus treatment was visually too prominent inside the
  clipped card. It was replaced by the existing muted-surface focus treatment,
  then recaptured and rechecked.

final result: passed

---

# Menu Bar Usage Popover — Final Design QA

Date: 2026-07-16  
Branch: `codex/menu-bar-usage-popover`  
Logical viewport: `380 × 520` (`760 × 1040` Retina capture)

## Result

The approved traffic-light menu-bar popover is visually and functionally ready on
the feature branch. The implementation follows the supplied CodexBar hierarchy
while remaining intentionally smaller: horizontally scrollable Agent navigation,
freshness and worst-state summary, subscription windows, API spend and budget,
and a fixed 2 × 2 action footer.

No P0, P1, or P2 visual, interaction, data-safety, or accessibility issue remains.

## Visual evidence

All captures below came from the disposable QA home and database at
`/private/tmp/llm-usage-bar-popover-final-qa-20260716`. The QA app used the real
Tauri `tray-popover` window, real backend snapshot commands, and the production
renderer. A temporary delayed auto-open hook was used only because this Mac's
status-item row is saturated and macOS placed the QA item beneath the notch/menu
overflow; the hook was reverted immediately after each QA build.

| Evidence | Capture |
| --- | --- |
| Reference and implementation in one comparison input | `captures/comparison-dark.png` |
| Final dark critical overview | `captures/final-latest-top.png` |
| Real internal scroll with API cost, 30-day cost, token total, and daily budget | `captures/final-latest-scrolled.png` |
| Agent-local filtering after a real Codex tab click | `captures/final-latest-agent.png` |
| Exact 50% remaining warning boundary | `captures/final-dark-yellow-50.png` |
| Exact 20% remaining warning boundary | `captures/final-dark-yellow-20-settled.png` |
| Missing API budget as a normal green/unknown mixed state with setup action | `captures/final-missing-budget-scrolled.png` |
| Failed refresh preserving the last critical state and marking it stale | `captures/final-failure-stale-red.png` |
| Light appearance with readable API metrics and fixed footer | `captures/final-light-red-scrolled.png` |
| Main usage window after Open details | `captures/final-latest-details.png` |
| Main Provider settings after Settings | `captures/final-latest-settings.png` |

The combined comparison confirms the intended reference relationship: near-black
surface, blue selected Agent, compact information hierarchy, subtle dividers,
progress emphasis, and persistent footer actions. Differences are deliberate:
the implementation uses per-Provider cards and explicit textual statuses instead
of copying CodexBar's account-specific chart and purchase rows.

## State and interaction matrix

- Green: greater than 50% subscription remaining; an unconfigured optional API
  budget stays unclassified and does not erase the valid green state.
- Yellow: exactly 50% and exactly 20% remaining both render Warning.
- Red: 19% remaining and 100% daily budget consumption render Critical.
- Stale: a failed local fixture refresh retains the last red snapshot, last
  success time, sanitized failure copy, and red aggregate state.
- Agent navigation: a native mouse event selected Codex and filtered the existing
  snapshot locally; no backend refresh was needed for the filter.
- Scroll: native wheel events moved only the summary viewport. Agent navigation
  and the action footer remained fixed and reachable.
- Escape: the real popover hid and the QA process reported zero visible popup
  windows for that interaction cycle.
- Open details and Settings: each hid the popup and revealed the corresponding
  main-window destination. The captured main usage and Provider settings surfaces
  confirm both routes.
- Right-click/native menu: AXPress reached the existing native menu during the
  macOS run. Physical pointer testing of the status item itself was unavailable
  only because macOS placed the item under the crowded menu-bar overflow/notch;
  the production left/right/down classifier and monitor-edge placement paths are
  covered by the already-passing Rust suite.

## Accessibility and visual corrections

The QA review found one P2 issue before sign-off: inactive Agent labels inherited
an extra `opacity-60`, reducing dark-theme text contrast to about 3:1. The tray
tabs now override inactive opacity to 100%; a regression assertion covers both
light and dark appearances. Selected tabs, progress boundaries, status badges,
keyboard traversal, accessible progress names/values, focus restoration, Escape,
and footer labels are covered by the frontend suite.

The QA run also exposed a startup race where a popup could observe
`refreshInProgress=true` before attaching its terminal event listener. The query
now polls the cache-only snapshot until the backend reaches a terminal state and
stops polling afterward. Its regression test proves both recovery and interval
shutdown.

## Isolation and security boundary

- Disposable HOME, config, database, renderer port, application proxy port, and
  bundle identifier were used for every launch.
- Only `qa-subscription` and `qa-metered` were enabled. The fixture contained no
  production Provider, credential slot, fingerprint, route credential, usage
  event, or auth path.
- All quota traffic was restricted to the loopback guard. Clean scenarios passed
  both prelaunch and final process isolation assertions with no forbidden outbound
  request.
- Opening the full Provider settings page caused the application to perform its
  normal credential lookup. The fail-closed QA `security` shim blocked four such
  subprocesses before macOS Keychain access; this evidence is archived as
  `logs/security.settings-navigation-blocked.log`. No production credential was
  read. Navigation was therefore verified without weakening the isolation guard.
- A final sensitive-field scan found `quota_config`, API-key, and fingerprint
  strings only in negative/redaction tests and their sentinels, never in the tray
  frontend DTO or serialized production snapshot.

## Automated verification

Final checks after the last TypeScript and styling fixes:

- Focused tray query and popover regressions: 33 passed.
- Full frontend suite, limited to two workers: 103 files, 649 tests passed.
- TypeScript: `tsc --noEmit` passed.
- Renderer production build: passed.
- `git diff --check`: passed.

Earlier in this same implementation run, before the two final frontend-only
fixes, the full Rust library suite passed with 2,385 tests, Clippy passed with
`-D warnings`, and the macOS App/DMG build completed. At the user's explicit
request, no further Rust, Clippy, Cargo test, or Tauri rebuild was run after the
frontend-only changes. The latest QA debug bundles did include both frontend
fixes and were used for the final dark and light captures.

## Remaining environmental note

The QA fixture intentionally used very distant reset timestamps to exercise
overflow-safe localized formatting. Those values are not proposed production
copy. The only runtime limitation was macOS menu-bar saturation hiding the QA
status item under the notch/overflow; it does not affect the popup layout, status
model, native-menu preservation, or verified click classifier.

final result: passed

---

# Provider Usage Statistics — Design QA

Date: 2026-07-19

Source visual truth:

- Main usage statistics: `/Users/max/Library/Application Support/CleanShot/media/media_TLZaGI07E8/CleanShot 2026-07-19 at 21.49.38@2x.png`
- Provider recent usage: `/var/folders/th/28ml58qj663d_8tbmcz417t00000gn/T/TemporaryItems/NSIRD_screencaptureui_EqUMHT/截屏2026-07-19 21.45.34.png`

Implementation screenshots:

- Main: `qa-artifacts/usage-main.png`
- Tray: `qa-artifacts/usage-tray.png`
- Combined main comparison: `qa-artifacts/usage-main-comparison.png`
- Combined tray comparison: `qa-artifacts/usage-tray-comparison.png`

Viewport and state:

- Main: `1280 × 800`, dark theme, all enabled Providers combined, populated hourly buckets.
- Tray: `760 × 1200` capture with a `348 px` Provider card, dark theme, ChatGPT quota plus populated 30-day usage.

## Findings

No actionable P0, P1, or P2 differences remain. The tray implementation closely
preserves the supplied two-column metric hierarchy, amber bars, and most-used
model line while intentionally omitting the explanatory local-log sentence. The
main implementation uses the reference's summary hierarchy and multi-series
trend, with stacked bars retained because the requested product behavior calls
for hourly/daily bar charts.

## Fidelity surfaces

- Fonts and typography: existing application typography and tabular metric
  numerals preserve the reference hierarchy at both main-window and popover
  densities.
- Spacing and layout rhythm: the main summary, five token-breakdown cells, chart,
  and compact tray block remain aligned without overflow at the tested sizes.
- Colors and visual tokens: surfaces, borders, muted copy, state green, amber tray
  bars, token-series colors, and estimated-cost line use the existing dark-theme
  tokens or the supplied chart palette.
- Image and asset fidelity: no illustrative assets are needed; both charts use
  the project's existing Recharts implementation and render sharply.
- Copy and content: total tokens, requests, estimated cost, input/output/cache
  values, cache hit rate, and most-used model are visible. The prohibited local
  estimation explanation is absent. Tooltip cost copy is localized.

## Comparison history

- Initial rendered capture: the chart animation left the bars visually empty on
  first paint, a P2 mismatch for a glanceable monitoring screen.
- Fix: disabled Recharts animation for the main stacked bars, cost line, and tray
  bars so real data is visible immediately.
- Post-fix evidence: both combined comparison images show populated bars at the
  same dark-theme state. No P0/P1/P2 issue remains.

## Focused evidence and interactions

The full combined images are readable, so a separate focused crop was not
needed. Hovering the main 14:12 bar exposed localized input, output, cache write,
cache read, and total-cost values. Both preview states reported zero browser
console errors.

final result: passed

---

# Main Window Usage Trend — Design QA

Date: 2026-07-19

Source visual truth: `/var/folders/th/28ml58qj663d_8tbmcz417t00000gn/T/TemporaryItems/NSIRD_screencaptureui_c8Bj7M/截屏2026-07-19 20.55.14.png`

Hourly implementation screenshot: `/private/tmp/llm-usage-trend-preview-crop.png`

Daily implementation screenshot: `/private/tmp/llm-usage-trend-daily.png`

Comparison viewport: `620 × 330` content crop, dark theme

State: populated Provider-wide token trend; hourly and daily buckets

## Findings

No actionable P0, P1, or P2 differences remain. The implementation preserves the
reference hierarchy: compact summary, low-noise dark card, zero-filled time
buckets, and a single dominant bar series. Indigo replaces the reference amber
as an intentional use of the existing Obsidian & Paper primary token. The
reference's cost/model text is not duplicated inside this component because the
main dashboard already presents cost and per-Provider model/request evidence in
the surrounding sections.

## Fidelity surfaces

- Fonts and typography: existing application font, metric numerals, hierarchy,
  wrapping, and compact labels are consistent with the main window.
- Spacing and layout rhythm: the chart fits a 620 px content width without
  overflow; header, plot, and axis spacing remain readable for both 21 hourly
  and 30 daily buckets.
- Colors and visual tokens: card, border, muted text, popover, and bar colors all
  use the existing dark-theme tokens with sufficient contrast.
- Image and asset fidelity: no raster asset is required; the data visualization
  is rendered by the project's existing Recharts dependency.
- Copy and content: localized title, hourly/daily label, total, accessible chart
  name, and tooltip values match the selected aggregation state.

## Evidence and interactions

The source and hourly implementation were opened together in one comparison
input. The full component was legible, so a separate focused crop was not needed.
Hovering the 16:00 hourly bar exposed `3,200,000 Token · 40 次请求`. The daily
state rendered 30 distinct buckets with readable sampled dates. Browser console
errors and warnings: none.

No P0/P1/P2 visual fix iteration was required after the settled chart animation
was captured. Automated coverage also verifies the hourly-to-daily range switch.

final result: passed
