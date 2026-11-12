# Swift 原生迁移功能矩阵

状态含义：`基准` 为正式 Tauri 行为；`已接入` 为可在 Native Preview 试用；`骨架` 为原生导航/UI 已存在但数据契约尚未迁移；`未开始` 不应被误解为可用。

| 能力 | Tauri 输入与输出基准 | Native Preview 当前状态 | 当前验收证据 | 下一道门槛 |
| --- | --- | --- | --- | --- |
| 菜单栏状态灯 | `TrayUsageSnapshot.status` → 圆形红黄绿/unknown 图标 | 已接入 | Rust/Swift contract test；Xcode universal build | 四语言 VoiceOver UI test 与日常运行 |
| 订阅 Provider | 账号级 quota snapshot → 5h/7d、pace、reset、manual resets | 已接入，只读且不合并账号 | 共享 fixture 保留 unknown/null；Swift DTO 使用 provider ID | 真实去敏数据库结构化对比 |
| 计费 Provider | usage events + pricing → 今日/30 日花费、预算、tokens | 已接入，只读 | Decimal string contract；budget/metered 菜单卡 | 多 key 与 partial/unavailable 实库对比 |
| 刷新与错误 | Rust collectors/schedulers → persisted state + tray projection | 已接入，刷新仍由 Rust 落盘 | bridge `refreshTrayUsage`；Rust 单服务实例；隔离进程 E2E | 真实数据连续运行、timeout 与强制崩溃恢复 |
| stale 回退 | 最近一次成功 tray projection | 已接入 | 原子 `tray-snapshot-v1.json`；Swift 强制 stale；0700/0600 进程测试 | 真实进程崩溃后的 stale socket/snapshot 恢复 |
| Providers 主窗口 | dashboard query、activity、events | 已接入，只读；账号稳定选择 | `getProviderDashboard` 最小安全投影、`getUsageEvents`、24h/7d/30d、分页 Table | 真实去敏数据库结构化对比与 Activity 网格 |
| Models 主窗口 | model aggregation、pricing projection | 已接入，只读 | `getModelDashboard`、原生列表与总量 | 真实去敏数据库对比和按产品组展开 |
| Agents 主窗口 | agent breakdown、stable binding | 已接入，只读 | `getAgentBreakdown`、原生列表 | 不恢复切换/代理功能；补 Agent 详情 |
| General 设置 | locale、托盘/启动行为、usage 阈值与共享预算标量 | 写入通道已接入；界面仍是只读骨架 | `NativeSettingsV1` 显式 allowlist；三端共享 fixture；锁内 sparse patch/revision 冲突测试；auto-launch 回滚；真实进程 E2E 脚本覆盖 | 先补 SwiftUI 设置面板与冲突 UI，再做四语言、并发冲突和双 UI 刷新体验验收 |
| Provider/预算/定价写入 | DB transaction + credential binding + version check | 未开始 | 仅 General 的共享预算模式/标量进入 allowlist；Provider 级预算、定价和凭据未开放 | 独立安全投影、bridge mutation 与冲突 UI |
| Keychain/OAuth | Rust credential service 与兼容 service/account 名称 | 未开始；Swift 正常启动不读取秘密 | bridge snapshot DTO 不含 credential 字段 | 隔离 Keychain 服务的 CRUD/冲突测试 |
| 备份/恢复/同步 | SQLite backup、WebDAV/S3、自定义数据目录 | 未开始 | 无 Swift 写入 | Rust bridge 表单后再迁 Swift core |
| 通知/登录启动/文件面板 | Tauri plugins/现有系统集成 | 未开始 | 无行为替换 | UserNotifications、SMAppService、NSOpenPanel |
| 数据库 migration | Rust v13→v26 | 未开始；Rust 独占数据库 | Preview 复用同一 AppState，不双写 | 历史 fixture、quick_check、FK、rollback |
| 签名与发布 | Developer ID、ZIP/DMG、notarization | Xcode 配置已建立，不可发布 | Preview/Production candidate 均为隔离身份；hardened runtime 预留 | 最终切换正式 identity 后 archive、notarize、staple、Gatekeeper |

## 阶段验收规则

- `unknown`、`unavailable` 和 `null` 不得格式化为零。
- Provider 多账号必须以稳定 `providerId` 分开显示。
- Swift Preview 除 `setNativeSettings` 的 General 稀疏 allowlist 外只能调用只读 bridge 方法；所有持久化与刷新写入仍由 Rust 核心执行。
- Settings bridge 不得序列化完整 `AppSettings`，也不得接收整对象替换；秘密字段名和值在请求、响应、错误和日志中都不得出现。
- Bridge 不得复用完整 Web Provider DTO；路由、认证、Keychain/API key 和 binding 元数据不得进入 Swift 进程，quota 错误只暴露稳定代码。
- 每个后续接口单独切换，禁止 Rust/Swift 双写同一数据。
- Production scheme 在完整切换前仅用于编译检查，不得替换已安装正式应用。
- `native/script/bridge_e2e.mjs` 必须使用短的 `/private/tmp` 测试 home；macOS Unix socket 路径受 `SUN_LEN` 限制。
- isolated bridge E2E 必须同时具备 `LLM_USAGE_BAR_TEST_HOME` 和 `LLM_USAGE_BAR_NATIVE_BRIDGE_ISOLATED_TEST`，并禁用 Keychain、同步器和外部网络。
