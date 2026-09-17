LLM Usage Bar now uses one React / TypeScript frontend with Tauri 2 and Rust on **macOS and Windows**. Licensed under MIT; built on CC Switch's foundations.

- Subscription quota, API spend, model/provider breakdowns, budgets and diagnostics.
- Optional Codex routing: provider priorities, model mappings, saved credentials, automatic/manual selection, and request/token totals.
- Explicit connection and restore controls. Restart Codex after either action; keep LLM Usage Bar running while routing is connected.
- Windows credentials stored in the current user's Credential Manager; macOS uses Keychain.
- Model mapping changes are transactional: a failed save keeps the previous mappings.

Downloads:

- **macOS 12+ (Apple Silicon and Intel):** universal `.dmg` or `.zip`.
- **Windows 10/11 x64:** NSIS `.exe` or `.msi`. Requires WebView2; the installer can install the runtime.
- **SHA256SUMS.txt:** SHA-256 checksums for all packages.

Signing: the macOS app is ad-hoc signed, **not Apple notarized**. After moving it to Applications, use System Settings → Privacy & Security → Open Anyway if macOS blocks it. Windows installers are **unsigned** and may show a SmartScreen warning. Verify the source and checksum before choosing More info → Run anyway. No automatic in-app updater is enabled.

Your database is backed up automatically before schema upgrades. Older builds may not open an upgraded database. Restore direct connection before uninstalling if you enabled Codex routing. Provider capabilities depend on their available billing endpoints and local data; Claude Desktop data differs by platform. Routed totals omit interrupted or unreported usage and are not an account balance.

中文：统一前端现支持 macOS / Windows，新增完整本地路由设置、模型映射、API key 选择、自动/手动切换、显式连接/恢复和分账。Mac 为临时签名、未公证；Windows 未签名。连接或恢复后请重启 Codex；路由期间保持应用运行，卸载前恢复直连。升级前自动备份数据库，下载校验值见 SHA256SUMS.txt。
