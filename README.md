<div align="center">

# LLM Usage Bar

### See what your AI coding tools are actually costing you — subscription quota and API spend, in one menu bar

[![Platform](https://img.shields.io/badge/platform-macOS%2012%2B%20%7C%20Windows%2010%2B-lightgrey.svg)](#install)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

English | [中文](README_ZH.md) | [日本語](README_JA.md) | [Deutsch](README_DE.md) | [Changelog](CHANGELOG.md)

*Forked from [CC Switch](https://github.com/farion1231/cc-switch) and rebuilt around usage tracking — see [Credits](#credits).*

</div>

## What it does

If you code with Claude Code and Codex, your usage is split across places that never add up: a five-hour window and a weekly window on the Claude subscription, an OAuth quota on Codex, and a separate pile of API keys billed by the token. Each has its own page, its own reset clock, and no shared total.

LLM Usage Bar reads all of it locally and puts one answer in the menu bar: **how much is left, and how fast you are going through it.**

Monitoring reads local session logs and quota files, and calls provider billing endpoints with keys you supply. Optional local routing is configured separately in Settings; only an explicit Connect Codex confirmation changes your CLI configuration.

## Two things it tracks

**Subscription quota** — the percentage-based windows on plans you already pay for.

- **Claude** — both the five-hour and weekly windows, read from Claude Desktop's local plan history and Claude Code's status-line bridge. The reset instant is latched so it survives whichever source refreshes last, and stays bound to the account that supplied it.
- **Codex** — quota from your existing OAuth session.
- **Coding plans** — Kimi For Coding, Zhipu GLM (personal and team), MiniMax, and Volcano Ark. Plans that grant usage-limit resets show how many are left and when they expire.

**API spend** — money, per key.

- Give a Provider a list of **named API keys** and each one reports its own daily and monthly spend, remaining budget, and when the figures were last fetched. Providers with more than one key also show a combined total.
- Spend is read from the provider's own billing endpoint. OpenRouter is wired up today; other presets need their own endpoint before they report anything.
- Set a daily budget per Provider, or an overall API budget, and the bar tells you when you are outrunning it.

## Optional Codex routing

Settings → Local routing provides provider priority, saved API-key selection, model mappings, automatic failover or a manual provider, and per-provider request/token totals. Configure a Responses-compatible endpoint and map each requested model before connecting. Connection changes are confirmed and backed up; Restore direct connection keeps your other current Codex settings. Restart Codex after either action and keep this app running while connected.

## The usage light

The menu bar shows one indicator, not a number you have to interpret. It projects your current burn rate against the time left on the reset clock:

- **Healthy** — at this pace you finish the window with room to spare
- **Warning** — you are on track to run out before the reset
- **Critical** — you are past that point

Open the popover for the detail behind the call: the pace it measured, what it projects, and how long until the window rolls over.

## Where the numbers come from

| Source                           | What it provides                      | How                                                           |
| -------------------------------- | ------------------------------------- | ------------------------------------------------------------- |
| Claude Code / Codex session logs | Tokens, models, per-request cost      | Imported from the JSONL files the CLIs write locally          |
| Claude Desktop plan history      | Five-hour and weekly percentages      | Local JSON, refreshed by the app itself                       |
| Claude Code status line          | Percentages **and** the reset instant | Local cache, written while a session renders its status line  |
| Codex OAuth                      | Subscription quota                    | Request signed with your existing OAuth session               |
| Coding-plan endpoints            | Plan quota and remaining resets       | Kimi, GLM, MiniMax by API key; Volcano Ark by AK/SK signature |
| Provider billing endpoints       | Per-key spend and limits              | Direct call with the key you saved                            |

Monitoring works independently of optional routing. Routed request totals count only traffic and usage actually observed by the router; they are a lower bound, not an account balance.

## Breakdowns

Three tabs over the same time range — today, 7 days, 30 days, or a year:

- **Providers** — spend and tokens per Provider, with a 12-month daily activity heatmap and an hourly or daily trend chart
- **Models** — which models the money actually went to
- **Agents** — which tool spent it, with explicit Agent-to-Provider bindings for traffic that would otherwise be unattributed

Every request is inspectable, and cost is recomputed from pricing you control: refresh the official price list, or override any model's rate per Provider.

## Cross-platform architecture

The supported development direction is **React + TypeScript for the UI, Tauri 2 + Rust for the desktop backend**. macOS and Windows share the same interface and business logic, exposed through the macOS menu bar or Windows system tray. The former Swift / SwiftUI migration is discontinued; its historical branches are archives.

Use Node.js 24 LTS, pnpm 11 and stable Rust. macOS requires Xcode Command Line Tools. Windows 10/11 requires Visual Studio C++ Build Tools with the Windows SDK and WebView2.

```sh
pnpm install --frozen-lockfile
pnpm dev
# Build on the target operating system
pnpm build
```

macOS builds app / DMG bundles; Windows builds NSIS / MSI installers. Output is under `release/tauri-target/release/bundle/`. Windows CI checks compilation, credential storage, and routing connection/restore. Release builds also install the Windows NSIS package and check app startup, database initialization, and the local router. Windows tray interaction still needs manual validation. Claude Desktop local data sources are platform-dependent and may not be available on Windows.

## Install

Download macOS and Windows installers from [GitHub Releases](https://github.com/Xr810/LLM-Usage-Bar/releases/latest). The macOS app is ad-hoc signed and not notarized; Windows installers are unsigned. See the release notes for installation steps and SHA-256 checksums. To build from source, use the cross-platform commands above. On macOS 12 or later, this additional helper signs a local build:

```bash
pnpm install && pnpm build:local:mac
```

That builds and signs the app at `release/tauri-target/release/bundle/macos/LLM Usage Bar.app` and stops. Drop `--build-only` — run `./scripts/build_and_run.sh` — to also install it to `/Applications` and launch it, with a verification pass that restores the previous app if the new bundle fails to start.

> **One-way upgrade.** The app migrates its database forward on first launch and takes an automatic backup first. Once migrated, an older build can no longer open it — the version ceiling refuses rather than risking the data. Install deliberately.

## Your data stays local

| Path                                | Contents                                                   |
| ----------------------------------- | ---------------------------------------------------------- |
| `~/.llm-usage-bar/llm-usage-bar.db` | SQLite — usage events, providers, pricing, quota snapshots |
| `~/.llm-usage-bar/settings.json`    | Device-level UI preferences                                |
| `~/.llm-usage-bar/backups/`         | Automatic pre-migration backups, 10 most recent by default |
| `~/.llm-usage-bar/logs/`            | Application log                                            |

API keys are stored in macOS Keychain or the current user’s Windows Credential Manager, never in the database and never in the logs. macOS debug builds keep credential storage disabled. Spend figures are never written to the log file.

Optional sync — the database can be kept in a custom config directory (iCloud, Dropbox, OneDrive, NAS) or pushed to WebDAV or S3-compatible storage. Off by default.

## FAQ

<details>
<summary><strong>Do I need to change how I run Claude Code or Codex?</strong></summary>

Monitoring needs no CLI changes. Optional routing requires explicit confirmation and a Codex restart. Restore direct connection in Settings before uninstalling if you enabled routing.

</details>

<details>
<summary><strong>Why does a Provider show no spend?</strong></summary>

Because spend comes from the provider's own billing endpoint, and only presets wired to one can report it. OpenRouter is wired up (`GET /api/v1/key`, which is scoped to the key that authenticates the call). Others need their endpoint added first. A Provider with no endpoint and no session-log attribution will legitimately show nothing.

</details>

<details>
<summary><strong>Why is a key's total flagged as belonging to a replaced credential?</strong></summary>

Because it does. Replacing a key does not retroactively reassign the spend the old one accrued, so those figures are called out rather than folded silently into the current total.

</details>

<details>
<summary><strong>Claude shows a percentage but no reset time. Why?</strong></summary>

Only one of Claude's two local sources carries a reset instant — the Claude Code status-line bridge, which writes while a terminal session renders its status line. Claude Desktop's plan history has the percentages but never the reset. The app latches the reset once it sees one and keeps serving it until it passes, but if the bridge has never run there is nothing to latch, and the app says so instead of showing a placeholder.

</details>

<details>
<summary><strong>Can it track a subscription used on another machine?</strong></summary>

No. Everything is read from this machine's local files and from key-scoped endpoints. A key used elsewhere is invisible to a per-key endpoint, and another machine's session logs are not here to import.

</details>

<details>
<summary><strong>Which languages does the interface support?</strong></summary>

English, 简体中文, 繁體中文, and 日本語.

</details>

## Documentation

- [Changelog](CHANGELOG.md)
- [Contributing](CONTRIBUTING.md) · [Security policy](SECURITY.md) · [Support](SUPPORT.md)

> `docs/user-manual/` still describes the provider-switching, proxy, MCP, prompts and skills features that were removed — it is not linked here until it has been rewritten.

## Built with

[Tauri 2](https://tauri.app/) · Rust · React 19 · TypeScript · SQLite

## Credits

LLM Usage Bar began as a fork of [CC Switch](https://github.com/farion1231/cc-switch) by Jason Young, and still builds on its provider, storage, sync and metering foundations. CC Switch's provider-switching, proxy, MCP, prompts and skills features have been removed; the quota, ingestion, aggregation, dashboard, routing and native-bridge layers are new to this project.

## License

[MIT](LICENSE)
