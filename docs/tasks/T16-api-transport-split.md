# T16:让 `api` 与传输无关

> **先读 [`README.md`](README.md) 的铁律。** 依赖:**T14、T15 已合并**。
> **不能与 T17 并行** —— T17 要把你产出的 `api` 层搬进新目录。

---

## 目标

模块化设计 §9.3 第三条:

> `commands/` 一万行是 Tauri 命令,而 SwiftUI 面板走的是 socket。
> **若契约直接长在 Tauri 命令上,socket 那边就得再写一遍。**

现在正是这个状态:每个 `#[tauri::command]` 函数里既有参数解析、又有业务编排、
又有错误脱敏。**换一个传输就得整个重写一遍。**

---

## 1. 目标形状

```
api/            与传输无关:有哪些查询、哪些命令、返回什么、错误怎么脱敏
 ↑
commands/       Tauri 的薄壳:解析参数 → 调 api → 返回
```

**薄壳的意思是:一个 `#[tauri::command]` 函数体应当只有几行** ——
取 state、调 `api` 里的对应函数、把 `Result<T, AppError>` 转成
`Result<T, String>`。任何 `if`、任何编排、任何 SQL 都不该留在壳里。

---

## 2. 范围:先只做 router 那一组

`commands/` 有 20 个文件一万行,**一次全拆风险太高**。本任务**只拆
`commands/router.rs` 那九个命令**,把形状立起来,后续再照着搬别的。

新建 `src-tauri/src/api/mod.rs` 与 `src-tauri/src/api/router.rs`,
在 `lib.rs` 加 `pub mod api;`。

```rust
// api/router.rs —— 与传输无关。不 import 任何 tauri 类型。
pub struct RouterApi { db: Arc<Database> }

impl RouterApi {
    pub fn list_providers(&self) -> Result<Vec<RouterProviderView>, AppError>;
    pub fn upsert_provider(&self, input: RouterProviderInput) -> Result<(), AppError>;
    // …九个命令一一对应
}
```

**硬要求:`api/` 下不允许出现 `use tauri::` 的任何一行。** 这是「与传输无关」
唯一可验证的判据,报告里贴 `grep -rn "use tauri" src-tauri/src/api/` 的输出
(应为空)。

视图/输入结构体从 `commands/router.rs` 搬到 `api/router.rs`,
`commands/router.rs` 里 `pub use` 转出保持前端类型不变。

---

## 3. 保持不变的东西(违反即失败)

- ✅ **九个 tauri 命令的名字、参数、返回类型一个字不变** —— 前端还没写,
  但契约已经定了,改了就是让前端重来
- ✅ 错误脱敏行为不变:`api` 返回 `AppError`,壳里转 `String`,
  **转换逻辑照抄现在的**
- ✅ T10 写的 9 条测试全部继续通过,断言不改
- ✅ `lib.rs` 的 `invoke_handler` 注册列表不变

---

## 4. 测试

1. T10 的 9 条测试搬到 `api/router.rs` 后继续通过(测 `RouterApi`,不再经 tauri)
2. `commands/router.rs` 里每个函数体不超过 5 行(**人工确认,报告里说明**)
3. `grep -rn "use tauri" src-tauri/src/api/` 为空

---

## 5. 明确不要做的事

- ❌ 不要拆 `commands/` 下的其他 19 个文件 —— 本任务只立形状
- ❌ 不要改任何命令的签名或名字
- ❌ 不要重排目录(那是 T17)
- ❌ 不要碰 `src/`、不要加新依赖

---

## 6. 完成的标准

- `api/router.rs` 存在,不含任何 tauri 引用
- `commands/router.rs` 是薄壳,每个函数体 ≤ 5 行
- 九个命令的对外契约一字未变
- 三条验收项都有实际输出贴进报告
- 六项检查全绿,**外加 `clippy --all-targets -- -D warnings`**
