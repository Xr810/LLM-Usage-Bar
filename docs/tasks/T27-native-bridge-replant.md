# T27:把 native bridge 移栽到当前 main

创建:2026-08-23
分支:`feat/native-bridge-replant`(已从 main 开好,文件已捞好,**未提交**)
前置:`docs/design/2026-08-23-router-panel-visual-direction.md`

---

## 0. 一句话

把 `feat/swift-native-shell` 上的 **bridge 层**(Swift `UsageCore` + Rust
`native_bridge.rs`)搬到当前 main 上,让它编译、让契约测试过。
**14 个 SwiftUI 视图文件不要**,它们会按新视觉方向重写。

## 1. 为什么不是 rebase

那条分支落后 main **113 个 commit**,而它 7547 行里最值钱的是 bridge,最不值钱的
是视图 —— 视图本来就要全部重画(HIG 计划 P1–P4 + 新视觉方向已取代)。为了保住
注定被删的代码去 rebase 一次跨后端重排的分支,是纯亏。

## 2. 现状(开工前先确认)

这些文件**已经在工作区**,从分支上 `git archive` 出来的,原样未改:

```
native/Package.swift          native/Sources/UsageCore/      (10 个 .swift)
native/.gitignore             native/Tests/UsageCoreTests/   (4 个测试 + 3 个 Fixture)
native/Configuration/         native/script/                 (bridge_e2e.mjs, build_and_run.sh)
src-tauri/src/native_bridge.rs   (1889 行 / 67 KB)
```

**没有捞的**:`native/Sources/LLMUsageBarNative/`(14 个视图)、`.xcodeproj`。

## 3. 要做的四件事

### 3.1 修 `native_bridge.rs` 的 import 漂移

T17 那次「按职责分模块」把类型挪了位置,**但没有改名**。已核实的全部四处:

| 文件里现在写的 | 改成 |
| --- | --- |
| `crate::usage::domain::{BillingKind, CostSourceCounts, ProviderMonitoringDashboardView, ProviderUsageView, QuotaFetchState, QuotaStatusView, TokenSource, UsageTrendBucketView, UsageTrendGranularity}` | `crate::model::domain::{同名九个}` |
| `crate::settings::{AppSettings, LocalMigrations, S3SyncSettings, WebDavSyncSettings}` | `crate::config::settings::{同名四个}` |
| `crate::store::AppState` | `crate::app_state::AppState` |
| `crate::usage::tray_snapshot::TrayUsageSnapshot` | **不变** |
| `crate::error::AppError` | **不变** |

改完编译,**残余漂移(方法签名、字段增删)由编译器报出来,逐条修**。
修的时候**跟着 main 的现状走,不要去改 main 上的既有文件** —— 只有 3.2 那一行例外。

### 3.2 在 `lib.rs` 里注册模块

加 `mod native_bridge;`,位置按字母序放进现有的模块声明块。
启动接线**这一轮不做**(bridge server 什么时候起、由谁起,留到 T28)。

### 3.3 `Package.swift` 去掉可执行 target

现在它声明了 `LLMUsageBarNative` 可执行 target,但那些源文件我们没捞。
改成**只留 `UsageCore` 库 + `UsageCoreTests` 测试**,`products` 里同步删掉
`.executable(...)`。可执行 target 等新视图写出来时再加回去。

### 3.4 跑通验证

```
cd native && swift build && swift test
```

## 4. 验收

1. `cargo check`(在 `src-tauri/`)零错误 —— 这是主判据
2. `cd native && swift test` 全绿,**契约测试一行不改**
   (`TrayUsageContractTests`、`NativeSettingsContractTests` 是判断 bridge
   协议有没有跟后端脱节的唯一凭据,改它们等于把判据关掉)
3. `git status` 里除了本任务涉及的文件,没有别的改动

## 5. 约束

- **不要跑任何 git 命令**(sandbox 写不了索引)。文件已经在工作区,直接改就行;
  不要 commit、不要 `git mv`、不要切分支
- **shell 断网**。本任务不需要联网:分支上 `Cargo.toml` / `Cargo.lock` 一行没改过,
  `Package.swift` 也没有外部包依赖。**如果发现需要新 crate,停下来报告,不要试图拉**
- **不要碰任何 UI / 前端文件**,`src/` 整个不动
- **不要动 main 上的既有 Rust 文件**,唯一例外是 `lib.rs` 加一行 `mod native_bridge;`
- 不要为了让编译过而删功能。改不动的地方留 `TODO(T28)` 注释并在报告里列出来

## 6. 交回来的时候要说清

- 除了表里那四处,还额外修了哪些漂移(这是判断后端重排影响面的真实数据)
- 有没有留下 `TODO(T28)`,分别是什么
- `swift test` 和 `cargo check` 的真实输出

---

## 7. 完成记录(2026-08-23,Claude 自己做的,未派)

**结果:`cargo check` 退出 0 / 0 error;`cd native && swift test` 23 个测试全绿,
契约测试一行未改。** 契约测试原样通过是这次最有价值的信号 —— 说明 bridge 协议
跟重排后的后端数据形状仍然对得上。

### 除了 §3.1 表里预测的四处,实际还多修了五处

| # | 漂移 | 处理 |
| --- | --- | --- |
| 1 | `crate::commands::` → `crate::api::commands::` | 5 处,纯改路径 |
| 2 | `get_provider_usage_activity_test_hook` **已删** —— 拆进独立文件时并成了 `#[tauri::command]` 版本(收 `State` 包装,bridge 拿不到) | 改为直接调它内部那行 `usage::aggregation::aggregate_enabled_provider_trend` |
| 3 | `config::settings::update_settings_checked_with_hooks` **已删** | 用公开的 `get_settings` / `update_settings` 就地重建同一顺序 |
| 4 | `config::settings::update_settings_in_store_with_hooks`(`#[cfg(test)]`)**已删** | 对着显式传入的 store 与 path 重建,配 `config::atomic_write` |
| 5 | `MainWindowDestination::GeneralSettings` 变体**已删**(现只剩 `Usage` / `ProviderBudget`) | Settings 交接退化为「开主窗口不指定目的地」,留 `TODO(T28)` |

第 3、4 两条大概率是 T22 精简时当死代码清掉的 —— 它们此前只有分支上的 bridge 在用。

### 一处行为回退,需要知情

重建后的设置写入**不再是原子的**:原 helper 在设置写锁内完成读改写,新版
`get_settings()` 与 `update_settings()` 之间不持锁,理论上存在 read-modify-write 竞态。
设置写入都由用户显式触发、并发极低,先接受。要恢复原子性得把 helper 放回
`config::settings`,那属于动 SSOT,留给 T28 定夺。

### 遗留

- `TODO(T28)`:Settings 交接目的地(见上表第 5 条)
- 启动接线未做(spec §3.2 就是这么定的):`mod native_bridge;` 已注册,但没有任何
  调用方,所以 `cargo check` 会报一整片 `never used` 警告 —— **这是预期的**,
  接线在 T28
- `native/LLMUsageBarNative.xcodeproj` 没有移栽(它引用那 14 个视图文件)
