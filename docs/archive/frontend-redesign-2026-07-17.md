# Frontend Redesign — "Obsidian & Paper"

Date: 2026-07-17
Branch: `feat/frontend-redesign`
Scope: renderer only (`src/`), no Rust changes. All 612 frontend tests and
`tsc --noEmit` pass; production renderer build passes.

## Design goals

The previous UI was a stock shadcn/zinc + system-blue look: flat bordered
cards, equal-weight stat walls, generic blue accent, and per-window status
badges repeated inside the tray popover. The redesign introduces a coherent
design language instead of restyling screen by screen.

## Token system (`src/index.css`, `tailwind.config.cjs`)

- **Light**: warm paper background (`40 9% 97%`), pure white elevated cards,
  hairline warm-gray borders, indigo accent (`243 75% 59%`).
- **Dark**: layered obsidian surfaces (bg `240 7% 7%` → card `240 6% 11%`),
  low-contrast borders, brighter indigo (`239 84% 68%`).
- **Semantic status tokens** (`--success/--warning/--danger` + foregrounds)
  flip per theme and now drive quota meters, tray progress bars, badges, and
  dots. This fixes the old tray bug where the healthy state rendered cyan
  (`#5ac8fa`) instead of green.
- New shadow scale (`shadow-xs/sm/md/lg/card/pop`), radius scale up to `2xl`,
  `.metric` utility for tabular numerals, tighter global letter-spacing,
  indigo-tinted `::selection`, token-driven focus rings.

## Primitives (`src/components/ui`)

- **Button**: token-driven variants (no more hard-coded `bg-blue-500`),
  subtle press feedback (`active:scale-[0.98]`), soft outline style.
- **Card**: `rounded-xl` + `shadow-card`, `p-5` rhythm.
- **Badge**: pill with new soft `success/warning/danger/info` variants.
- **Tabs**: macOS-style segmented thumb (raised card on muted track)
  replacing the blue pill.
- **Alert/Input/Dialog**: soft tinted variants, `rounded-xl` dialog with
  `shadow-pop`.

## Main window

- **App header**: brand mark (indigo tile + chart icon), slimmer bar,
  ghost settings button.
- **Dashboard controls**: preset ranges moved into a reusable
  `SegmentedControl` (`components/common/SegmentedControl.tsx`).
- **Subscription cards**: provider brand icon (preset map → fuzzy inference
  via `iconInference`, extended with openai/chatgpt/codex/gpt/gemini),
  `QuotaMeter` component showing **remaining %** (matches tray semantics)
  with status-colored bar and localized reset countdown, compact token strip
  with compact-notation values, quiet ghost footer actions.
- **Metered cards + overview**: stat strips instead of bordered boxes,
  cost-quality badges, mono model names in recent requests.
- **Sections**: uppercase micro-headings with counts; softer empty states.
- Missing copy (`providerMonitoring`, `meteredProviders`, `remainingPercent`,
  window labels, empty states…) added to all four locales — the UI no longer
  mixes English fallback into the zh interface.

## Tray popover (380×520)

- One status indicator per provider (dot + tiny label) instead of a badge
  per quota window; window status is carried by bar color alone.
- Provider rows are soft `bg-card` tiles with 22px brand icons.
- Footer is now a single row of four compact icon actions.
- `TrayUsageStatusBadge` restyled to borderless dot+label.

## Settings

- System Provider cards: brand icon header, borderless auth status row
  (status dot + actions right-aligned) for Claude CLI / Codex OAuth /
  API-key variants; custom Provider rows show icons and ghost actions.

## Compatibility notes

- `ProviderIcon` strips inner SVG `<title>` nodes (outer span already
  provides `title`), keeping text queries and screen readers clean.
- `vitest.config.ts` now excludes `.pnpm-store/` and `.worktrees/` — stale
  project copies there were previously collected when filtering by path.
- `productIdentityCompatibilityManifest.json` line numbers were realigned
  after locale insertions (context text unchanged).
- Two tests updated for intentional copy changes: "Metered Provider
  accounts" → "Metered accounts"; "25% used" → "75% left" (remaining-based
  quota display).

## Evidence

Before/after captures (mock-backed browser harness, both themes):
`docs/images/redesign/`
