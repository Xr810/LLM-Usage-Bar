# Windows acceptance — aa828178

## Scope and provenance

- Repository: Xr810/LLM-Usage-Bar.
- Executed on the Windows Desktop runner on 2026-10-08, not inferred from CI.
- Verified source commit: [aa828178c59bfd7db9358ef5df912e369602f99d](https://github.com/Xr810/LLM-Usage-Bar/commit/aa828178c59bfd7db9358ef5df912e369602f99d).
- Direct parent / watcher fix: [9c48788d406e86e234c4d46e5fa3f91bfad22e97](https://github.com/Xr810/LLM-Usage-Bar/commit/9c48788d406e86e234c4d46e5fa3f91bfad22e97).
- No existing project checkout was found in the inspected user directories. An isolated exact-commit checkout was created; its status was clean before testing. Existing user work and application data were not repurposed. Runtime source was unchanged throughout testing; the only later tracked change was a local HANDOFF evidence entry, which is not included in this report transfer.
- Overall result: **NOT PASSING**. Release-mode unsigned packages build and run, but native failures and slow in-flight ingestion exit remain. Successful focused checks do not supersede these failures.
- This is a sanitized, text-only report for a documentation branch. No database, credential payload or identifier, personal log, screenshot, installer, or full evidence archive is included. Only explicitly selected synthetic-test excerpts are reproduced below.
- Original evidence remains on the Windows runner. Evidence basenames below are provenance identifiers, **not paths accessible from an Orb**. The report is transferred separately to the coordinator, which owns documentation-only commit/push. No main merge, repair-source push, or release is authorized by this report.

## Environment and initial blocker

Windows x64; Node 24.21.0; pnpm 11.11.0; pinned Rust 1.95.0; portable Git 2.56.0.windows.2; WebView2 154.0.4258.62. Portable tools were added to individual command environments, not the system PATH.

Initially MSVC and Windows SDK were missing. The first native command exited 101 with `linker link.exe not found`, and the first release build exited 1 without producing packages. These were unexecuted test cases, not native failures or passes. Two elevation attempts were canceled. Following explicit user approval, Visual Studio Build Tools 2022 17.14.37710.0, MSVC and Windows SDK 10.0.26100.0 installed successfully; no reboot was required. Subsequent results below were actually executed after installation. The approved toolchain was retained.

## Commands and outcomes

All commands ran from the isolated checkout. Rust and Tauri used the repository wrappers.

| Command | Actual outcome |
| --- | --- |
| `pnpm install --frozen-lockfile` | Passed |
| `pnpm typecheck` | Passed |
| `pnpm format:check` | Passed |
| `pnpm test:unit --maxWorkers=2` | 54 files, 352 tests passed |
| `pnpm build:renderer` | Passed; Vite configuration-loading / bundle-size warnings retained |
| `pnpm rust -- fmt --manifest-path src-tauri/Cargo.toml --check` | Passed |
| `pnpm rust -- test --locked --manifest-path src-tauri/Cargo.toml` | Exit 101; library: 1230 passed, 19 failed, 2 ignored; integrations did not run in this invocation |
| `pnpm rust -- test --locked --manifest-path src-tauri/Cargo.toml --lib app_state:: -- --nocapture` | 10 passed; measured in-flight transaction shutdown 3.1528ms; intentional worker panic was part of a passing test |
| `pnpm rust -- test --locked --manifest-path src-tauri/Cargo.toml --lib usage::watcher::tests:: -- --nocapture` | 7 passed, including real Windows file events and callback release |
| `pnpm rust -- test --locked --manifest-path src-tauri/Cargo.toml --lib secrets::windows::tests -- --nocapture` | 2 passed with actual Windows Credential Manager roundtrip / overwrite / deletion |
| `pnpm rust -- test --locked --manifest-path src-tauri/Cargo.toml --lib route::pointer::tests -- --nocapture` | 10 passed |
| `node --test scripts/cargo-cache-lib.test.mjs` | 39 passed, 1 failed, 6 skipped |
| `pnpm tauri -- build --target x86_64-pc-windows-msvc` | Exit 0 after toolchain installation; optimized release compilation 6m 01s; 2 bundles |

Integrations were separately executed with `pnpm rust -- test --locked --manifest-path src-tauri/Cargo.toml --test <target> -- --nocapture`:

| Target | Outcome |
| --- | --- |
| `app_type_parse` | 2 passed |
| `coexistence_identity` | 1 passed |
| `extension_seam_guards` | 2 passed |
| `usage_dashboard_commands` | 4 passed |
| `router_probe` | 1 ignored; no probe pass claimed |
| `app_config_load` | 0 passed, 6 failed; migration rejection caused mutex poisoning in the other 5 cases |

## Failures and diagnostic bypasses

The full native run failed in these 19 cases, transcribed from its failure summary:

```text
cli::tests::anchored_upgrade_windows::npm_windows_default_branch
cli::tests::anchored_upgrade_windows::pnpm_windows_uses_pnpm_add
cli::tests::anchored_upgrade_windows::volta_windows_uses_volta_install
cli::tests::anchored_upgrade_windows::windows_full_batch_line_for_percent_path_uses_quadruple_escape
cli::tests::anchored_upgrade_windows::windows_no_sibling_uses_cli_update_without_package_fallback
cli::tests::anchored_upgrade_windows::windows_path_with_space_is_double_quoted
store::identity_migration::tests::backup_captures_committed_uncheckpointed_wal
store::identity_migration::tests::concurrent_callers_share_one_no_clobber_migration_result
store::identity_migration::tests::crash_residue_wal_is_preserved_in_independent_new_and_archive_snapshots
store::identity_migration::tests::migrates_real_v13_database_and_archives_source
store::identity_migration::tests::migration_hook_proves_post_snapshot_competing_commit_cannot_succeed
store::identity_migration::tests::post_unlink_directory_sync_failure_keeps_committed_publications
store::identity_migration::tests::source_failing_quick_check_is_rejected_without_publication
store::identity_migration::tests::source_write_barrier_allows_backup_but_rejects_competing_commit
store::identity_migration::tests::successful_publication_leaves_no_hidden_snapshot_or_quarantine_name
store::migrations::cursor_migration::tests::classifies_only_paths_below_resolved_external_roots
store::migrations::cursor_migration::tests::migration_archives_v13_rows_and_builds_conservative_source_cursors
store::tests::schema_v14_cursor_migration_tests::migration_v13_to_current_archives_line_state_and_sets_version_once
store::tests::schema_v14_cursor_migration_tests::v14_migration_failure_rolls_back_archive_cursor_and_version
```

- Six CLI failures compare direct npm/pnpm/Volta install plans against `codex update || fallback` expectations; a no-sibling case returns None. This report does not decide whether product behavior or expectations should change.
- Nine identity migration failures include Windows explicitly refusing safe retirement of the old database, and live-WAL backup PermissionDenied. Four cursor/schema failures include `legacy` versus expected `claude`, and rollback expectations. These are current failing results, not a resolved environment blocker like the initially missing compiler.
- The original `app_config_load` run failed all six. Diagnostic command `pnpm rust -- test --locked --manifest-path src-tauri/Cargo.toml --test app_config_load -- --skip database_identity_migrates_v13_and_threads_authoritative_runtime_paths` passed the remaining five. The skip is disclosed and does not make the original suite pass.
- The cargo-cache failure was main checkout / linked worktree bucket equality. Portable Git defaulted to `core.autocrlf=true`; inspected test Cargo.lock bytes were LF in the main checkout and CRLF in the linked worktree, so hashes differed. A focused subprocess-only `core.autocrlf=false` diagnostic passed. No global Git configuration or runtime source was changed, and the default-run failure remains.
- First 10000-event pressure fixture: the read observer hit SQLITE_BUSY before sending quit. That attempt did not test exit. A second independent fixture was used. A later fixed-time observer saw incomplete ingestion and failed its premature count assertion; eventual natural exit and the independently reopened final database were checked separately. Failed observations were retained, not rewritten into passing results.

Selected raw native summary:

```text
test result: FAILED. 1230 passed; 19 failed; 2 ignored; 0 measured; 0 filtered out; finished in 37.45s
error: test failed, to rerun pass `--lib`
```

## Release packages and installation

Both packages are version 3.17.0, x64, built by the release command above. Authenticode status was **NotSigned** for both. No signing credential was provided; this is not a signed release or SmartScreen trust validation.

| Package basename | Bytes | SHA256 |
| --- | ---: | --- |
| `LLM Usage Bar_3.17.0_x64-setup.exe` | 6386259 | `15FA2D8FAE02F939F26DF73AE16026BF94C21E6C030ED785DBA7A7BF006A4633` |
| `LLM Usage Bar_3.17.0_x64_en-US.msi` | 8761344 | `E327BF84E9A8390204D9C73295F0933056FC170559F84ADC59095558623F1872` |

NSIS installed with `/S /D=<QA_ROOT>\installed-nsis` and uninstalled with `uninstall.exe /S`; both exit 0. MSI installed with `msiexec /i <MSI> /passive /norestart INSTALLDIR=<QA_ROOT>\installed-msi /L*v <QA_LOG>` under elevation and uninstalled with `msiexec /x <MSI> /passive /norestart /L*v <QA_LOG>`; both exit 0. The per-machine MSI actually reused the prior NSIS directory, as verified below. An administrative extraction with `msiexec /a <MSI> /qn TARGETDIR=<QA_ROOT>\msi-extracted` exited 0; its executable and the MSI-installed executable matched SHA256 `B0450EE98E9C15CB85370A7ACA7FA84DA51FF46674C37DEA7808AFBE85F55935`. Comparing that executable with the pre-packaging build executable was not used to infer a version mismatch: bundling patches bundle-type information.

### Confirmed MSI AppSearch override of explicit INSTALLDIR

Source evidence is the actual `msi-install.log`, not a hypothesized MSI default. Personal absolute directory prefix is replaced consistently with `<QA_ROOT>` in this excerpt; timestamps and property changes are preserved:

```text
MSI (s) (C4:C8) [23:43:52:496]: PROPERTY CHANGE: Adding INSTALLDIR property. Its value is '<QA_ROOT>\installed-msi'.
MSI (s) (C4:C8) [23:43:52:561]: Doing action: AppSearch
Action start 23:43:52: AppSearch.
MSI (s) (C4:C8) [23:43:52:562]: PROPERTY CHANGE: Modifying INSTALLDIR property. Its current value is '<QA_ROOT>\installed-msi'. Its new value: '<QA_ROOT>\installed-nsis'.
Action ended 23:43:52: AppSearch. Return value 1.
MSI (s) (C4:C8) [23:43:52:565]: PROPERTY CHANGE: Modifying INSTALLDIR property. Its current value is '<QA_ROOT>\installed-nsis'. Its new value: '<QA_ROOT>\installed-nsis\'.
MSI (s) (C4:C8) [23:43:52:566]: PROPERTY CHANGE: Adding ARPINSTALLLOCATION property. Its value is '<QA_ROOT>\installed-nsis\'.
Property(S): INSTALLDIR = <QA_ROOT>\installed-nsis\
```

Actual generated WiX, `release/tauri-target/x86_64-pc-windows-msvc/release/wix/x64/main.wxs`, lines 59–65 (blank lines/comments omitted; attributes unchanged):

```xml
<Property Id="INSTALLDIR">
  <RegistrySearch Id="PrevInstallDirNoName" Root="HKCU" Key="Software\llmusagebar\LLM Usage Bar" Type="raw" />
  <RegistrySearch Id="PrevInstallDirWithName" Root="HKCU" Key="Software\llmusagebar\LLM Usage Bar" Name="InstallDir" Type="raw" />
</Property>
```

Generated comments say the NSIS default-value search is first and the MSI `InstallDir` search second so the latter has priority when both exist. Lines 111–113 also write `[INSTALLDIR]` to HKCU `Software\llmusagebar\LLM Usage Bar`, value `InstallDir`. The directly assigned search property and observed AppSearch transition corroborate the override in this particular install. No no-prior-key, dual-key precedence, or interactive-directory-selection matrix was executed; this report does not claim those cases fail or fixes this behavior.

## Installed application acceptance

Application runs used process-only `LLM_USAGE_BAR_TEST_HOME`, `WEBVIEW2_USER_DATA_FOLDER`, and local WebView CDP arguments (`--remote-debugging-port=9222 --remote-debugging-address=127.0.0.1`). Fixtures were generated synthetic Claude JSONL, not personal sessions. Real user database and real API keys were not used. The renderer was driven through CDP and production IPC; native tray interaction was driven through Windows UIAutomation plus the app-owned native HMENU, not simulated renderer tray clicks.

- Both installed packages launched and rendered. Inspected screenshots remain private local evidence and are deliberately excluded from this transfer.
- Initial JSONL ingestion preserved token tuple `[2,1,48719,2061]` and excluded a zero-usage row. Appending `[11,7,13,17]` triggered real Windows watcher ingestion without manual sync, exactly once. MSI append `[19,23,29,31]` produced exactly three retained events; independent SQLite assertions checked values and `integrity_check = ok` after exit.
- A disposable synthetic credential was created through production IPC and stored in real Windows Credential Manager. Process restart restored configured/version 1; replacement gave version 2; clearing gave missing/version 3; deletion removed metadata. Independent database checks found no plaintext synthetic credential, and a targeted credential listing after cleanup returned none. No actual credential payload, target identifier, provider connection, or paid request is included or claimed.
- Native window close hid the window while leaving the application alive; actual tray Open restored it. Actual tray Quit exited naturally. MSI tray invocation plus process wait took 483ms, exit 0; this includes UI-driver overhead, not only shutdown work.
- A normal MSI cold launch followed by production quit took 2406ms, exit 0. It was not demonstrated to overlap the unowned credential-restoration initializer and is not evidence that precise race is solved.
- The unconfigured router returned HTTP 503 to a local synthetic request, without upstream credentials; this validates the unconfigured smoke path, not provider routing success.
- Second 10000-row fixture: with window hidden and 2 new rows observed in the database, three consecutive production `quit_from_tray` requests were sent. Application remained alive at 120 seconds, still writing. It eventually exited 0 naturally, with observed elapsed upper bound 175.6648448 seconds; no force kill was used. All 20002 rows from two bulk fixtures plus the initial events were retained; every bulk tuple was independently checked as `[3,5,7,11]`.
- Final application/owned WebView count, listeners on 8788/9222 and UIAutomation tray icon count were all 0. Databases reopened with `integrity_check = ok`, and original database files opened exclusively with FileShare.None after exit. No ghost-icon hover was needed. Test installs, disposable credential, test-created roaming window-state file and administrative extraction were removed; build tools and private evidence were retained.

Selected synthetic-app raw excerpts (no personal log paths or session contents):

```text
[2026-10-08][23:39:40][INFO][llm_usage_bar_lib] 收到用户主动退出请求 (code=Some(0))，开始清理...
[2026-10-08][23:42:35][INFO][llm_usage_bar_lib::ingest] [SESSION-SYNC] 同步完成: 导入 10000 条, 跳过 0 条, 扫描 3 个文件
[2026-10-08][23:42:35][INFO][llm_usage_bar_lib] 已显式从系统托盘移除图标
[2026-10-08][23:42:35][INFO][llm_usage_bar_lib] 清理完成，退出应用
eventual_graceful_exit_observed=true elapsed_seconds_upper_bound=175.6648448 exit_code=0
{"TrayIcons":0,"Names":[],"AppProcesses":[]}
```

## Limits and next verification

Do not close the overall Windows acceptance gate until current native failures and the slow ingestion-exit behavior are addressed and natively rerun. The 3.1528ms small-transaction test and 483ms idle desktop result do not establish a global exit deadline. Precise exit interleavings with unowned startup credential restoration / backup tasks, permanently blocked I/O, valid provider credentials and online quota were not verified. Unsigned packages do not establish signed distribution or SmartScreen behavior.

This runner did not execute macOS tests. Any macOS isolated-HOME keychain failure must be classified by its own runner using actual keychain/signing/permission evidence; a reported `credential_unavailable` alone cannot distinguish a product defect from temporary signing, isolation or authorization limits. No cross-platform causal conclusion is asserted here.

## Private evidence index (not transferred)

| Basenames / groups | What they establish |
| --- | --- |
| `verified-commit.txt` | Exact checked source commit |
| `dependencies.log`, `typecheck.log`, `format.log`, `frontend-tests.log`, `renderer-build.log`, `rust-fmt.log` | Frontend and formatting execution |
| `native-full-tests.log`, `native-app_state.log`, `native-usage-watcher-tests.log`, `native-secrets-windows-tests.log`, `native-route-pointer-tests.log` | Full native failure and focused actual native results |
| `integration-*.log` | Original integration outcomes and separately labeled migration-skip diagnostic |
| `cargo-cache-tests.log`, `cache-lf-diagnostic.log` | Default cache failure and LF diagnostic |
| `release-build.log`, `release-build-after-toolchain.log`, `package-manifest.json` | Initial blocked build, successful release build, hashes/signature status |
| `install-uninstall.log`, `msi-install.log`, `msi-extract.log`, `msi-uninstall.log`, `msi-uninstall-result.txt` | Installer execution, actual AppSearch transition, extraction and uninstall |
| Generated `main.wxs` at the relative build path above | Actual WiX search-property wiring; only the safe fragment is in this report |
| `nsis-initial-db.log`, `nsis-watcher-db.log`, `msi-watcher-db.log`, `msi-after-exit-db.log` | Independent synthetic-event values, no duplicate ingestion, DB integrity |
| `nsis-credential-created.json`, `nsis-credential-restart-clear.json`, `nsis-credential-deleted.log` | Disposable credential lifecycle; identifiers deliberately omitted here |
| `nsis-close-to-tray.json`, `nsis-tray-open.log`, `nsis-tray-quit.log`, `msi-tray-quit.log`, `msi-startup-quick-quit.*` | Actual window/tray/normal-startup interactions |
| `nsis-inflight-driver.log`, `nsis-inflight-observation.log`, `nsis-inflight-quit.*`, `nsis-inflight-final-db.log` | Pressure requests, failed premature observation, eventual exit and final data |
| `final-residue.json`, `final-tray-presence.log`, archived DB check logs | Final process/listener/tray absence and independent reopened DB assertions |

These files, private synthetic fixture databases, screenshots, installers, drivers and the full evidence archive are intentionally **not** part of the documentation submission. Only this sanitized Markdown is transferred. The existing local HANDOFF edit is left untouched and is not submitted with this document.
