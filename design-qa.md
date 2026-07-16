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
