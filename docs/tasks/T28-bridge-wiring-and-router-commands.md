# T28:bridge 启动接线 + 路由命令接入(含补上的读命令)

创建:2026-08-24
分支:`feat/native-bridge-replant`(接在 T27 之后)
前置:`T27-native-bridge-replant.md`、`docs/design/2026-08-23-router-panel-visual-direction.md`

---

## 0. 做了什么

1. bridge 启动接线
2. 九个路由命令接进 bridge 分发
3. **补第十个命令 `listModelRoutes`** —— 原来那九个只能写映射、读不回来
4. Swift 侧 `RouterRepository`(DTO + actor 实现)
5. 两侧共用的路由契约 fixture

## 1. 生命周期:仍然锁在 bridge-only 闸门后

`start()` 只在 `LLM_USAGE_BAR_NATIVE_BRIDGE_ONLY` 置位时调用。理由:bridge 是
SwiftUI 壳与 Rust 核心之间唯一的传输通道,而那个壳目前只在该模式下运行 —— 给一个
尚无 UI 的功能常驻开一个 Unix socket,只会白白增加本地攻击面和空闲功耗(§11 那条线)。
**等 Swift 壳成为默认外壳时,这道闸门才该拿掉。**

**没有接 `stop()`**,这是有意的:退出路径有 #3998 的死锁史(异步清理任务与插件的
`RunEvent::Exit` 钩子互等同一把锁),不值得为一个能自愈的清理去碰它 ——
`prepare_socket_path` 下次启动会删掉无人监听的旧 socket;若仍有人监听则报错中止,
那说明另一个实例在跑,本来就该拦。

## 2. 两条与 tauri 那条路不同的契约(都是有意的)

| # | 差异 | 为什么 |
| --- | --- | --- |
| 1 | **路由错误按原文回传**,不走 `write_app_result` 的统一脱敏(另写了 `write_router_result`) | 面板要把「非法 wire_api」「模式指向不存在的 provider」钉在窗口顶部给用户看。`api::router` 本身已是脱敏面,tauri 那条路也是 `e.to_string()` 原样给前端 |
| 2 | **变更命令返回变更后的新状态**,不是 `()` | bridge 响应里 `result: null` 会被 Swift 侧判成畸形响应(`BridgeResponse.result` 是 `Result?`);顺带返回新状态,面板不用回查,也没有读到旧值的窗口 |

变更命令各自返回:upsert / delete → 新的 provider 列表;setModelRoutes → 新的映射列表;
setRouterMode → 新的模式;enableRouterPointer → 新的指针状态。

## 3. 补的第十个命令

**原来九个命令里 `set_model_routes` 只能写,没有任何一条能读回来** ——
`RouterProviderView` 没有 routes 字段,DAO 只有 `list_model_routes(logical_model)`
(按逻辑模型查、且只要 `enabled = 1`,那是路由决策路径)。结果是**模型映射那一屏
没法显示用户已配好的映射**,而那正是被 T21 探针钉死要第一个做的屏。

补法(刻意不动 `RouterProviderView`,它已被 tauri 那条路在用):

- `store/dao/router.rs`:`list_all_model_routes()` —— 不过滤 enabled,按
  `provider.priority` 升序(与「尝试顺序」一致),组内按 logical_model
- `api/router.rs`:`ModelRouteView`(读侧,带 `provider_id`)+ `RouterApi::list_model_routes()`
- `api/commands/router.rs`:tauri 命令 `list_model_routes`,并在 `lib.rs` 注册(与另外九个保持对称)
- `native_bridge.rs`:`listModelRoutes` 分发臂

**HANDOFF §18.3 记的「九个 tauri 命令」现在是十个。**

## 4. 顺手修掉的两笔 T27 遗留

1. **T27 的验证不完整。** 当时只跑了 `cargo check`(只编 lib、不编 test),所以
   我写在 `#[cfg(test)]` 里的 `AppError::Unknown`(不存在的变体)没被发现。
2. **T27「先接受」的非原子设置写入站不住,有测试盯着** ——
   `concurrent_settings_mutations_conflict_inside_the_settings_lock` 要求两个并发
   变更只成功一个,那一版两个都成了。

   已真修,不是改断言:`config/settings.rs` 加了 `update_settings_checked`,把写锁
   持到整段「读 → 准备 → 副作用 → 落盘 → 提交」上,恢复 compare-and-swap 语义。
   函数上写了注释说明它有唯一调用方、不是死代码 —— T22 那次精简把它的前身当死代码
   清掉过一次,别再来一次。

## 5. 验证

- `cargo test --lib`:**1285 passed · 0 failed · 2 ignored**
- `cd native && swift test`:**28 passed**(新增 5 个路由契约测试)
- 新增共用 fixture `native/Tests/UsageCoreTests/Fixtures/router-contract-v1.json`:
  Rust 侧断言真实视图**序列化出来**就是它,Swift 侧断言 DTO 能把它**解回来**。
  两边都不改这个文件,才说明协议没跟后端脱节 —— 这套机制这一轮已经两次逮到真问题

## 6. 遗留

- `TODO(T28)` 那条(Settings 交接目的地)**没有恢复**:`MainWindowDestination` 现在
  只剩 `Usage` / `ProviderBudget`,新设置界面写出来之前这个目的地无处可去。已从
  `capabilities.shutdownDestinations` 里撤掉声明(分发臂仍安全收下并退化为「不指定
  目的地」),对应的单元测试改为断言这条退化
- Swift 侧只有 repository 与 DTO,**没有任何视图** —— 视图按视觉方向文档从零写,是下一步
- bridge 的端到端脚本 `native/script/bridge_e2e.mjs` 未覆盖新命令
