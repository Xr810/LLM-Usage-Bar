# macOS 原生验收记录：aa828178

验收日期：2026-10-08（Asia/Singapore）。本文为脱敏文档，不包含安装包、数据库、凭据、个人日志或截图。原始证据保留在执行机器上；证据索引仅以文件名及行号标识，不表示这些文件已在仓库或 Orb 中可访问。

## 1. 版本、范围与结论

- 验证源码：[aa828178c59bfd7db9358ef5df912e369602f99d](https://github.com/Xr810/LLM-Usage-Bar/commit/aa828178c59bfd7db9358ef5df912e369602f99d)，产品版本 3.17.0；已通过 `git merge-base --is-ancestor 9c48788d HEAD` 确认包含 watcher 路径别名修复。
- 原 checkout HEAD 为 [c7771c62c303580ae0e364253f1387ca311db226](https://github.com/Xr810/LLM-Usage-Bar/commit/c7771c62c303580ae0e364253f1387ca311db226)，验收前后干净且未切换。验证在独立 detached worktree 执行，未借用旧 CI 绿灯。
- 平台：Apple Silicon、macOS 27.0.1；Rust 1.95.0、Node 22.23.2、pnpm 11.11.0。发行 workflow 使用 macOS 14 / Node 24，不能把本机结果说成完全相同环境下的 CI 结果。
- 应用源码未修改。原应用和真实数据库验收前后的摘要一致；不公开真实数据库摘要或个人路径。
- 原生回归、带构建覆盖的 universal 安装包、安装后真实 Codex 日志摄取通过；**完整托盘及受管理后台退出验收未完成，不满足全量发行验收门。**
- 实际启动为 ARM Mac 的正常原生启动。Intel slice 仅交叉编译、架构检查，未作 Intel 真机运行验收；本机没有进行 Windows 验收。

本文中的 `<QA_WORKTREE>`、`<QA_HOME>`、`<REAL_CODEX_ROOT>`、`<INSTALLED_QA_APP>` 和 `<WINDOWS_TEST_ROOT>` 是脱敏占位符，不是 Orb 路径。包与原始本机证据未上传或发布。

## 2. 实际执行命令与结果

所有 Rust/Tauri 命令使用仓库包装器。

| 命令 | 本次结果 | 证据 |
| --- | --- | --- |
| `pnpm install --frozen-lockfile` | 通过 | M01 |
| `pnpm rust -- test --locked --manifest-path src-tauri/Cargo.toml` | 1296 通过、0 失败、3 ignored | M02 |
| `pnpm rust -- test --locked --manifest-path src-tauri/Cargo.toml --lib app_state:: -- --nocapture` | 10 通过；事务退出等待 3.253583 ms，测试失败界限 5 秒 | M03 |
| `pnpm rust -- test --locked --manifest-path src-tauri/Cargo.toml --lib usage::watcher::tests:: -- --nocapture` | 8 通过，包含原生 FSEvents、目录别名及停止后回调释放 | M04 |
| `pnpm rust -- clippy --locked --manifest-path src-tauri/Cargo.toml -- -D warnings` | 通过 | M05 |
| `pnpm rust -- fmt --manifest-path src-tauri/Cargo.toml --check` | 通过 | M05 |
| `pnpm typecheck`、`pnpm format:check` | 通过 | M05 |
| `pnpm test:unit` | 351 通过，范围查询测试 1 项 5 秒超时 | M06 |
| `pnpm test:unit src/components/usage-dashboard/UsageDashboardPage.test.tsx` | 2 通过，同项超时复现 | M06 |
| `pnpm test:unit --maxWorkers=1` | 原超时未改，54 文件、352 项通过 | M06 |
| `node --test scripts/cargo-cache-lib.test.mjs` | runner 重连后缺 Cargo PATH 的尝试为 45/1；补齐 PATH 后 46/0 | M07 |
| 原发行 universal 构建（下文） | 失败，dyld 拒绝加载 strip 后的宏动态库 | M08 |
| 同发行构建仅设置 `CARGO_PROFILE_RELEASE_STRIP=none` | release APP / DMG 构建退出码 0 | M09 |
| `codesign --verify --deep --strict`、DMG checksum、两个架构分别检查 | 通过 | M10 |
| `spctl --assess --type execute --verbose=4` / `xcrun stapler validate` | 拒绝（3）/ 没有 stapled ticket（65） | M11 |

完整 Rust 测试执行期间 runner 重连，原命令的进程跟踪丢失；保留日志含所有 suite 的完成结果。本表合计为 lib 1281 加集成 6 + 2 + 1 + 2 + 4，共 1296，不声称取到了丢失进程的退出码。三个 ignored 为两项 live S3 测试及 `router_end_to_end_probe`，不计为通过。依赖守卫两项包含在本次完整测试中。

聚焦后台测试中的 intentional worker panic 是预期测试输入，对应测试通过，不是生产崩溃。5 秒仅为测试界限，不是生产强杀或退出期限。

聚焦原生测试的原文摘录（M03/M04）：

```text
in-flight transaction shutdown: 3.253583ms (limit 5s)
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 1273 filtered out; finished in 1.52s
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1275 filtered out; finished in 0.38s
```

## 3. 构建失败、绕过与包校验和

默认发行构建命令：

```sh
CI=true APPLE_SIGNING_IDENTITY=- \
  pnpm tauri -- build --target universal-apple-darwin
```

本机失败摘录（替换个人路径并省略重复的 dlopen 搜索候选，M08:158）：

```text
error: <QA_WORKTREE>/release/tauri-target/release/deps/libserde_derive-28a60c1659a95c40.dylib: dlopen(...): ... (mis-aligned LINKEDIT string pool, fileOffset=0x00223404)
```

仅改变构建环境，未改 Cargo.toml 或应用源码：

```sh
export PATH="$HOME/.cargo/bin:$HOME/.nvm/versions/node/v22.23.2/bin:$PATH"
CI=true APPLE_SIGNING_IDENTITY=- CARGO_PROFILE_RELEASE_STRIP=none \
  pnpm tauri -- build --target universal-apple-darwin
```

此命令生成 universal release APP、DMG，随后按 workflow 用 `ditto -c -k --sequesterRsrc --keepParent` 生成 ZIP。**覆盖后的成功不是默认 workflow 原样成功；尚未定位工具链/strip 交互的完整根因，不能宣称已修复。**

本机 `lipo <binary> -verify_arch arm64 x86_64` 报 `-verify_arch requires exactly one input file`；分别验证 `arm64`、`x86_64` 均成功，`lipo -info` 显示两种架构。DMG 校验为 VALID。安装后可执行文件与 DMG 中的副本逐字节一致。

签名摘录（M10）：

```text
Format=app bundle with Mach-O universal (x86_64 arm64)
Signature=adhoc
TeamIdentifier=not set
```

构建明确因缺少 Apple 签名/公证环境凭据而跳过 notarization。临时签名结构验证成功不等于 Apple Developer 签名或公证成功；Gatekeeper 拒绝也未被绕过后算成通过。未验证互联网下载后带 quarantine 的信任安装流程。

| 本机构建的文件名（仅记录，未附包） | SHA-256 |
| --- | --- |
| `LLM-Usage-Bar-3.17.0-aa828178-macOS-universal.dmg` | `2042179a6e56e0e0fead239ad293b153149b0918fe728aff3acffa87a326782c` |
| `LLM-Usage-Bar-3.17.0-aa828178-macOS-universal.zip` | `eba9e7df4b690b69acdd397ce602b12a2d5b9a52fb62ab9473dfb0535d4f7f23` |

## 4. 安装版摄取、监听、退出与残留

使用 `hdiutil verify`、只读挂载、`ditto` 安装独立 QA APP，安装后验证签名结构及二进制一致性，再卸载 DMG。未替换原应用。直接启动 `<INSTALLED_QA_APP>/Contents/MacOS/llm-usage-bar`，环境设置如下；测试工具不是应用修复：

```text
HOME=<QA_HOME>
LLM_USAGE_BAR_TEST_HOME=<QA_HOME>
XDG_DATA_HOME=<QA_HOME>/.local/share
HERMES_HOME=<QA_HOME>/.hermes
```

`.codex` 是到 `<REAL_CODEX_ROOT>` 的 symlink，fixture 为人工生成的无凭据 JSONL：

1. 启动路由对未配置模型的 POST 请求返回预期 503，并确认监听属于 QA 进程。
2. 初始累计 input/output/cache 为 101/23/17，落库完全一致。
3. 在初始摄取后等待 90 秒，跨过生产 60 秒最小同步间隔，再追加累计 135/34/20；329 ms 内落库增量 34/11/3，没有调用手动同步。
4. 再写相同累计值，等待 75 秒后仍只有两条记录，不重复计费。
5. 运行中及退出重开后的 `PRAGMA integrity_check` 均为 `ok`，两条记录保留。
6. 向运行中的 QA APP 发送 AppleEvent quit，957 ms 内退出码 0，没有强杀。之后无该应用进程、8788 监听或该进程窗口。

安装版生产代码在人工 fixture 上的 FSEvents/摄取日志摘录（M13:103–104，仅替换路径）：

```text
[2026-10-08][23:41:47][TRACE][notify::fsevent] FSEvent: path = `<REAL_CODEX_ROOT>/sessions/2026/10/08/rollout-qa.jsonl`, flag = StreamFlags(ITEM_MODIFIED | IS_FILE)
[2026-10-08][23:41:47][INFO][llm_usage_bar_lib::ingest::codex] [CODEX-SYNC] 同步完成: 导入 1 条, 跳过 0 条, 扫描 1 个文件, 剪枝 0 个文件
```

最终验收 JSON 的无个人字段摘录（M12）：

```json
{
  "router503": true,
  "appendLatencyMs": 329,
  "integrityWhileRunning": "ok",
  "appleEventQuitSent": true,
  "appleEventWaitMs": 957,
  "finalExit": { "code": 0, "signal": null },
  "integrityAfterExit": "ok"
}
```

退出日志仅见 `applicationWillTerminate`（M13:108–109），未见托盘退出分支的开始清理、清理完成及移除托盘日志。**无残留或数据库完整性不能替代受管理后台任务取消/等待、正在写库时安全退出或托盘 Quit 路径的证明。**

第一次验收脚本错用 canonical `usage_events.session_id` 查询，该字段按设计为空；核对实际数据与 INSERT 后改用 fixture `request_id`、新隔离目录重跑成功。首次 harness 失败及其 SIGTERM 清理不计为产品失败或正常退出证据（M15）。

## 5. 未完成项：产品缺陷与环境限制的区分

| 观察 | 分类与边界 |
| --- | --- |
| `AXIsProcessTrusted=false`，屏幕捕获允许 | **确认的权限限制**。需在执行机器的辅助功能设置允许 runner 宿主 Amp.app；托盘可见性截图无法可靠归属目标，点击、Quit、残留未完成，不算通过。 |
| 没有 Apple 凭据；签名为 ad-hoc，Gatekeeper 拒绝 | **确认的发行环境限制**，不属于已签名/公证发行。 |
| 隔离 HOME 下三次 `credential_unavailable`，初始摄取约延迟 25 秒 | **未定位的验收失败，不能确认是实际产品缺陷，也不能直接归为纯环境限制**。本次同时改变 HOME/应用数据路径并使用临时签名，没有默认 HOME、独立受控钥匙串或有效 Apple 签名的对照；日志没有底层 OSStatus。 |
| fixture 没有真实账户，quota scheduler 刷新失败 | **确认的测试范围限制**。不证明真实在线额度或凭据流程成功，也不足以认定额度产品缺陷。 |
| 默认前端并发超时，单 worker 全量通过 | 两种结果都保留，未定位根因；不归因于并发/机器负载后便宣称修复。 |
| 默认 strip 构建失败，关闭 strip 成功 | 确认失败及有效绕过，未定位完整工具链根因，未提交修复。 |

凭据失败摘录（M13:91–96）：

```text
[2026-10-08][23:40:00][ERROR][llm_usage_bar_lib::secrets::service] credential store put failed
[2026-10-08][23:40:00][ERROR][llm_usage_bar_lib::secrets::service] fixed API binding local credential generation failed: credential_unavailable
```

后续凭据复验应在独立测试账户或受控钥匙串下固定版本/签名，分别对比默认 HOME 与隔离 HOME，记录非敏感 OSStatus 和授权结果；不使用真实账户数据库/凭据，不把默认 HOME 指向生产状态后直接试写。此对照本次未执行。

实际启动初始化、同步写库中退出、连续退出请求、托盘退出/残留仍未完成。10 项 AppState 测试不能替代完整桌面关键交错；真实初始化仍有未受管理 spawn，慢 I/O/永久阻塞的生产退出时限未验证。原生 watcher 测试和安装版目录别名事件已通过，但不能据此重写修复前 CI 的超时因果结论。

## 6. 证据索引（原始文件未附）

| ID | 原始证据文件名/位置 | 用途 |
| --- | --- | --- |
| M01 | `install.log` | frozen lockfile 安装 |
| M02 | `rust-tests.log`:1292–1356 | 全部 suite 结果、合计及 ignored |
| M03 | `background-focused.log`:18,22 | 事务等待、后台 10 项结果 |
| M04 | `watcher-focused.log`:15 | watcher 8 项结果 |
| M05 | `clippy.log`、`fmt.log`、`typecheck.log`、`frontend-format.log` | 静态检查 |
| M06 | `frontend-tests.log`:60,73–74；`frontend-targeted.log`:54,67–68；`frontend-tests-single-worker.log` | 默认/定向失败与单 worker 成功 |
| M07 | `cargo-cache-tests.log`:180,294–295；`cargo-cache-tests-with-path.log`:284–285 | 缺 PATH 与修正后结果 |
| M08 | `release-build.log`:158–179 | 原构建失败 |
| M09 | `release-build-unstripped.log`:827,835 | 跳过公证、构建退出码 |
| M10 | `package-verification.log` | ad-hoc、双架构、DMG 校验、安装一致性；一致性 cmp 成功由执行工具退出码确认 |
| M11 | `gatekeeper.log`、`notarization.log` | Gatekeeper 拒绝和无 ticket |
| M12 | `installed-acceptance.json`、`installed-acceptance.mjs` | 人工 fixture 验收结果及最终执行脚本 |
| M13 | `native-app.log`:91–109 | 凭据失败、真实 FSEvents、摄取和 AppleEvent 退出 |
| M14 | `permissions-final.log`、`post-exit-state.log`、`original-state-before.log` | 权限、残留和原状态未变；个人路径及真实 DB 摘要不附 |
| M15 | `installed-acceptance-harness-error.json`、`installed-runtime-first-run.log` | 首次工具查询错误和信号清理，排除误报 |
| M16 | `SHA256SUMS.txt` | 本文的两个包摘要 |

本节索引的原始文件只在 Mac runner；不要将证据文件名或占位符视为 Orb 可访问的文件。本文包含经人工核对的有限脱敏摘录，不上传整份日志。

## 7. Windows MSI 旁证核对（不是本机 Windows 执行结果）

通过读取 Windows 原生验收线程的既有工具输出，核对到 commit 同为 `aa828178c59bfd7db9358ef5df912e369602f99d`。以下来自 `msi-install.log` 实际读取输出；此 Mac 没有读取该 Windows 文件系统，也没有重新安装 MSI。

仅将个人目录前缀替换为一致的 `<WINDOWS_TEST_ROOT>`；其余属性变化内容保持原摘录：

```text
MSI (s) (C4:C8) [23:43:52:496]: PROPERTY CHANGE: Adding INSTALLDIR property. Its value is '<WINDOWS_TEST_ROOT>\installed-msi'.
MSI (s) (C4:C8) [23:43:52:561]: Doing action: AppSearch
Action start 23:43:52: AppSearch.
MSI (s) (C4:C8) [23:43:52:562]: PROPERTY CHANGE: Modifying INSTALLDIR property. Its current value is '<WINDOWS_TEST_ROOT>\installed-msi'. Its new value: '<WINDOWS_TEST_ROOT>\installed-nsis'.
Action ended 23:43:52: AppSearch. Return value 1.
MSI (s) (C4:C8) [23:43:52:565]: PROPERTY CHANGE: Modifying INSTALLDIR property. Its current value is '<WINDOWS_TEST_ROOT>\installed-nsis'. Its new value: '<WINDOWS_TEST_ROOT>\installed-nsis\'.
Property(S): INSTALLDIR = <WINDOWS_TEST_ROOT>\installed-nsis\
```

同一输出的 `Command Line` 行包含 `INSTALLDIR=<WINDOWS_TEST_ROOT>\installed-msi`。这足以确认**该次运行的显式 INSTALLDIR 被 AppSearch 覆盖**，不是仅从模板推测。

已生成 `release/tauri-target/x86_64-pc-windows-msvc/release/wix/x64/main.wxs`:59–65 的实际读取片段（只去掉工具路径/行号前缀、合并终端折行）：

```xml
        <Property Id="INSTALLDIR">
          <!-- First attempt: Search for the default key value (this is how the nsis installer stores the path) -->
          <RegistrySearch Id="PrevInstallDirNoName" Root="HKCU" Key="Software\llmusagebar\LLM Usage Bar" Type="raw" />

          <!-- Second attempt: Search for "InstallDir" which takes priority if found (this is how the msi installer stores the path) -->
          <RegistrySearch Id="PrevInstallDirWithName" Root="HKCU" Key="Software\llmusagebar\LLM Usage Bar" Name="InstallDir" Type="raw" />
        </Property>
```

证据 W01：Windows `verified-commit.txt`、`msi-install.log` 的既有工具输出；W02：生成 `main.wxs` 的读取输出及 `release-build-after-toolchain.log`。没有附 Windows 原日志或注册表导出。

**结论边界**：日志未指明哪个 RegistrySearch ID 命中；不能断言具体命中项。现有摘录没有 `WIX_UPGRADE_DETECTED` 或 `InstallScope` 原文，不能确认其值，不能将“摘录未见”说成“完整日志不存在”，也不据 HKCU 搜索推断安装范围。没有执行注册表清理、MSI 修复或复验。

## 8. 文档交付边界

本次只提交此脱敏 Markdown 到专门文档分支，不合并 main、不推送应用修复，也不附原 QA worktree 中含个人路径的 HANDOFF 记录。应用安装包和原始证据仍保留在执行机器上；协调 Orb 收到的是本文，不是对本机路径的访问权限。后续修复需按其新 commit 重新原生验收，不能继承 aa828178 的通过结果。
