# Swift 原生迁移路线

项目采用“绞杀者”方式逐步迁移：Tauri 版本在原生版本达到同等能力之前继续作为可发布应用，避免一次性重写造成数据丢失或功能倒退。

## 第一阶段（当前实现）

- `native/LLMUsageBarNative.xcodeproj` 是正式 macOS App 工程，包含 `Native Preview` 和 `Native Production` 两个共享 scheme。Preview 身份固定为 `com.llmusagebar.native.preview`；Production 在切换前仍使用隔离的 `com.llmusagebar.native.production-candidate`，最终验收通过后才改成 `com.llmusagebar.desktop`。
- `Native Preview` 默认使用仅在 Preview/Debug 编译的内存样例，不连接 socket、不读取 Keychain、不访问文件或数据库。样例保留 subscription/metered 两个独立账号、unknown/null、三个模型、两个 Agent、趋势以及 65 条可分页事件；界面会明确显示“样例数据”。
- `native/Package.swift` 继续提供 `UsageCore` 本地模块和 Swift Testing 入口。旧的简化 `UsageSnapshot` 只保留给早期草案测试；生产界面改用完整镜像 Rust DTO 的 `TrayUsageSnapshotV1`。
- 原生场景已经固定为 `MenuBarExtra(.window)`、`Window(id: "main")` 和 `Settings`。Preview Run 会切换为 regular、自动显示单例主窗口并保留菜单栏入口；Production Candidate 仍以 accessory 启动且不自动开窗。AppKit 只负责应用激活、重开和 macOS 13 的设置窗口 responder fallback。
- 菜单栏已展示共享 API 预算、Provider 多账号、订阅窗口、API 花费、pace、预测耗尽、reset、刷新错误和 stale 状态。
- 原生主窗口已经接入版本化的 Provider、Model、Agent 和 usage-event 查询。侧栏以 Provider 账号 ID 稳定选择，工具栏提供 24 小时、7 天和 30 天范围；Provider 详情包含额度、花费、记录数及分页事件。
- General 设置已经接入独立的安全投影和稀疏 patch。Rust 仍在现有 `AppState` 内独占数据库、settings 文件、Keychain 与刷新调度器；Swift 不直接读取或写入这些存储，也不会接触同步、备份、Provider 或认证秘密。

## Native bridge v1

- Socket：`~/.llm-usage-bar/runtime/native-bridge-v1.sock`，runtime 目录 `0700`，socket `0600`，服务端通过 `getpeereid` 校验客户端 UID。
- 协议：一行一个 JSON 对象的 NDJSON，最大 1 MiB；每个 request/response 都包含 `protocolVersion` 和 `id`。
- 方法：`hello`、`capabilities`、`getTrayUsageSnapshot`、`refreshTrayUsage`、`getRuntimeStatus`、`getNativeSettings`、`setNativeSettings`、`getProviderDashboard`、`getProviderUsageActivity`、`getModelDashboard`、`getAgentBreakdown`、`getUsageEvents`、`shutdown`。Settings、Dashboard 与分页响应各自带 `schemaVersion: 1`。
- `hello` 如实返回 `readOnly: false` 与 `mutationSchemaVersion: 1`，协议版本仍为 1。Swift 对缺少或不认识 mutation schema 的旧服务保持只读连接，只有 mutation 调用会返回 unavailable；`capabilities.mutations` 明确列出 `setNativeSettings`。
- Preview 连接后，旧 Tauri 菜单栏临时隐藏；连接释放后恢复。`shutdown` 可携带 `usage` 或 `settings` 目的地：服务先关闭 socket、结束 Preview lease，再由同一个 Rust 进程恢复正式 Tauri UI，避免退出/重启之间争抢数据库运行锁。
- Rust 每次发布菜单栏状态时原子更新 `tray-snapshot-v1.json`。Swift 仅在 bridge 不可用时读取它，并无条件标记 `stale`。
- 共享契约样例位于 `native/Tests/UsageCoreTests/Fixtures/tray-usage-snapshot-v1.json`，由 Rust、TypeScript 和 Swift 三端测试共同读取。未知 JSON 字段向前兼容，未知 envelope schema 在解码 snapshot 前拒绝。
- Dashboard 共享样例位于 `native/Tests/UsageCoreTests/Fixtures/dashboard-contract-v1.json`，同样由三端读取，并覆盖 Provider、Model、Agent 与 usage event 中的 unknown/null 语义。
- Settings 共享样例位于 `native/Tests/UsageCoreTests/Fixtures/native-settings-v1.json`，由 Rust、TypeScript 和 Swift 三端读取；未知 JSON 字段向前兼容，未知 settings envelope schema 会被 Swift 拒绝。
- Provider dashboard 使用独立的最小安全投影：只传账号 ID、显示名、计费类型、产品组、启用状态和系统 preset key；路由地址、Keychain/API key 状态、Agent binding 与认证元数据不会进入 Swift。quota 原始错误也会归一成稳定的 `quota_refresh_failed` 代码。
- Settings 同样使用显式的最小安全投影，只包含 General pane 的十个 allowlist 字段。`revision` 是该投影确定性 JSON 的 SHA-256 摘要；`setNativeSettings` 只接受稀疏 patch，并在 settings 写锁内复核 revision、验证完整的 patch 后状态，再经 Rust 现有 settings 持久化路径写入。冲突返回当前 revision 和安全投影，未知 key 与非法值分别返回稳定的 `unknown_setting`、`invalid_setting`。
- `launchOnStartup` 的系统 auto-launch 切换与 settings 持久化作为一个 mutation 执行；系统调用或文件写入失败时会恢复原状态并且不更新内存 settings。语言、阈值、预算 decimal string 与刷新间隔均由 Rust 做最终验证。
- `native/script/bridge_e2e.mjs` 会启动真实 Tauri 进程并验证 runtime/socket 权限、hello gate、协议不匹配、并发客户端、半包、settings get/set/get、冲突、非法 patch、线上无秘密、dashboard 查询与 shutdown。它只在短路径 `/private/tmp/lub-bridge-*` 下运行，并通过双环境标志禁用 Keychain、同步器和外部网络。
- 当前没有可复用的 settings-change event。Swift 修改后，共享预算会触发既有 tray/dashboard 重建，但已经打开的 Tauri Settings 页面不会自动刷新其他 General 字段；重新打开或重新加载该页面后才读取新值。

构建与测试：

```bash
cd native
swift test
xcodebuild \
  -project LLMUsageBarNative.xcodeproj \
  -scheme "Native Preview" \
  -configuration Preview \
  -derivedDataPath .build/xcode-preview \
  CODE_SIGNING_ALLOWED=NO build
```

在 macOS 上运行开发版本：

```bash
cd native
./script/build_and_run.sh
```

默认命令等价于 `--preview-data=fixture`，启动后会自动显示主窗口。需要连接真实 Rust bridge 时改用：

```bash
./script/build_and_run.sh --live
```

`--live` 等价于 `--preview-data=bridge`。Bridge 模式在启动已安装可执行文件前会检查其是否包含 `--native-bridge-server` 能力标记；较旧的正式版不会被误启动，界面会改用 stale snapshot 并显示 bridge 不可用。安装或显式指定包含 bridge 的 Tauri 构建后才会进入实时模式。

在 Xcode 中选择 `Native Preview > My Mac` 后按 Run 即可使用 fixture。Scheme Editor 中默认启用 `--preview-data=fixture`，并保留一个默认禁用的 `--preview-data=bridge` 参数；两者只应启用一个。Canvas 入口集中在 `Sources/LLMUsageBarNative/NativeViewPreviews.swift`，包含菜单栏 healthy/stale/refresh error、主窗口浅色/深色和 Settings diagnostics 六个预览。Canvas 模型直接预加载，不启动进程或 bridge。

从仓库根目录执行完整 bridge 进程验收：

```bash
./native/script/build_and_run.sh --bridge-e2e
```

若正式 Tauri APP 未安装在 `/Applications/LLM Usage Bar.app`，可给 Preview 指定可执行文件：

```bash
LLM_USAGE_BAR_BRIDGE_EXECUTABLE=/absolute/path/to/llm-usage-bar \
  ./script/build_and_run.sh --live
```

## 后续阶段

1. **完整只读菜单栏验收**：用真实去敏数据做连续运行、四语言、浅深色、键盘、VoiceOver、handoff、崩溃和 stale 对比。
2. **原生只读主窗口验收**：契约和第一版 UI 已接入；下一步补 Activity 网格、详情层次、真实去敏数据库结构化对比和 UI 自动化。
3. **扩展原生设置与安全写操作**：General allowlist 已通过带 revision 的 Rust bridge mutation 完成；Provider、定价、认证、备份与同步仍按独立安全投影逐项迁移。
4. **Swift 核心替换**：按纯逻辑、GRDB 只读、导入调度、采集器、写操作、Keychain/备份/migration 的固定顺序切换；每个接口独立对比且禁止双写。
5. **正式切换和清理**：全部活跃接口切换后才启用 Production；首版保持 schema v26。稳定版验收后再引入 v27，最后删除 React、Node、Tauri 和 Rust 发布依赖。

## 迁移约束

- 原生应用不得直接写现有 SQLite 数据库，直到 Rust 与 Swift 对 migration、备份和版本上限具有同一套契约测试。
- API key 继续只保存在 macOS 钥匙串；快照、协议错误和日志不得包含凭据。
- Preview 必须使用独立 bundle identity；只有生产切换阶段才能采用正式身份和数据目录。
- 当前 Rust 仍是正式版本和回退基准。功能与证据矩阵见 [native-feature-matrix.md](native-feature-matrix.md)。
