# T25:删掉 ingest 的四个转发层文件

> **先读 [`README.md`](README.md) 的铁律。** 依赖:T17、T22 已合并。
> **与 T24 都改 `lib.rs`**(不同区段),可并行但合并时留意。
>
> **纯机械改动,零逻辑。** 改完行为必须一字不变。

---

## 0. 背景

T1 与 T14b 把四家的会话日志读取搬进 `ingest/` 时,**为了让每一步能单独复核**,
把原来的四个文件留成了转发层。T22 查实:

- 它们注释里说要保的旧路径 `crate::services::session_usage*` **已经零用户** ——
  T17 之后全仓库都改成了 `crate::ingest::session_usage*`
- 现在引用的是**转发文件本身**,共 **16 处调用点 + 4 行 mod 声明**

也就是说:**这四个文件现在只是一层没有理由存在的间接。**

---

## 1. 十六处调用点(T22 已逐个定位)

| 转发文件 | 调用点 |
| --- | --- |
| `ingest/session_usage.rs` | `usage/session.rs:102,536`、`usage/usage_stats.rs:3374`、`api/commands/usage.rs:340,342,393,394` |
| `ingest/session_usage_codex.rs` | `usage/session.rs:105,545`、`api/commands/usage.rs:345` |
| `ingest/session_usage_gemini.rs` | `lib.rs:1241,1261`、`api/commands/usage.rs:359` |
| `ingest/session_usage_opencode.rs` | `lib.rs:1245,1267`、`api/commands/usage.rs:373` |

**行号会漂**(T24 可能同时在改 `lib.rs`),以**符号**为准,不要按行号盲改。

---

## 2. 做法

1. 把这 16 处的路径换成 `ingest/` 下真正的实现模块
   (`crate::ingest::{claude,codex,gemini,opencode}` 里对应的那个函数)
2. 删掉 `ingest/mod.rs` 里那 4 行 `pub mod session_usage*;`
3. 删掉那 4 个转发文件

**函数名不要改。** 如果转发层导出的名字与实现模块里的不同,
**在报告里列出对应关系**,不要顺手统一命名 —— 那是另一件事。

---

## 3. 硬要求

- ✅ **零逻辑改动。** 只换路径、删文件
- ✅ 所有测试断言一字不改,测试数仍是 **1256**
- ✅ 五个对外函数(`sync_claude_session_logs` 等)的**行为**不变
- ✅ 删完之后 `grep -rn "session_usage" src-tauri/src` 只应剩下
  `ingest/` 内部的实现和测试引用 —— 报告里贴这条的输出

**如果某个转发层导出的东西在实现模块里找不到对应物,停下来报告** ——
那说明转发层不只是转发,里面有东西。

---

## 4. 明确不要做的事

- ❌ 不要改任何函数体
- ❌ 不要重命名任何函数或类型
- ❌ 不要动 `ingest/` 下四个真正的解析器(`claude.rs` / `codex.rs` /
  `gemini.rs` / `opencode.rs`)的逻辑
- ❌ 不要碰 `src/`、不要加新依赖

---

## 5. 完成的标准

- 四个转发文件不复存在,`ingest/mod.rs` 少 4 行
- 16 处调用点全部换成直接路径
- 测试数 1256,断言未改
- 六项检查全绿,**外加 `clippy --all-targets -- -D warnings`**
- §3 那条 grep 的实际输出贴进报告
