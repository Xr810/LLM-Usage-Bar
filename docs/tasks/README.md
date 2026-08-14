# 实施任务清单:模块化 + 本地路由

给外部 AI agent 执行的任务书。**先读完本文,再读你被分配的那一份。**

> **状态:2026-08-14 已批准落地。** 设计已定稿,任务书已按一次实机核对修订
> (补齐了 T1 的解析器接口、T2 的搬运清单、T3 的连带改动、T5 的状态码缺口、
> T6 的凭据入口与路径前缀)。**照本文和你那份任务书执行即可。**

---

## 0. 铁律(违反任何一条,成果直接作废)

1. **语言是 Rust。** 后端一切用 Rust 写。**不要用 Python、Node、Shell 写业务逻辑**,
   不要引入脚本做本该由 Rust 做的事。
2. **不要添加任何新的 crate 依赖。** 需要的全都已在 `src-tauri/Cargo.toml` 里:
   `axum 0.8`、`hyper 1.0`、`hyper-util`、`tower`、`tower-http`、`reqwest 0.12`、
   `tokio 1`、`futures 0.3`、`rusqlite 0.40`、`serde`、`serde_json`、`thiserror 2.0`、
   `chrono`、`uuid`、`log`。**如果你觉得需要新依赖,停下来在报告里说明,不要自己加。**
3. **只改你那份任务书里明确列出的文件。** 碰到别的文件想改,停下来在报告里写明,
   不要顺手改。多个 agent 在并行工作,越界就是冲突。
4. **不要重构任务书没要求的东西。** 看到旁边有难看的代码,忍住。
5. **跑本地命令必须用包装器**(仓库约定,见 `AGENTS.md`):
   - `pnpm rust -- <cargo 参数>`,例如 `pnpm rust -- test`
   - **不要**直接 `cargo build` / `cargo test` / `cargo clippy`
6. **交付前必须全部通过这六项**,一项不过就不算完成:

   ```
   pnpm rust -- fmt --check
   pnpm rust -- clippy -- -D warnings
   pnpm rust -- test
   pnpm typecheck
   pnpm format:check
   pnpm test:unit
   ```

   注意 `clippy` 是 `-D warnings`,**任何警告都算失败**。
7. **注释和提交信息用中文**,与仓库现有风格一致。注释要解释**为什么**,不要复述代码
   在做什么。
8. **不要动 UI / 前端组件。** `src/` 下的 React 代码由项目所有者自己写。你只在
   `src-tauri/` 里工作(除非任务书明确说了要改前端类型)。

---

## 1. 走最短的路

**这条最重要**:本项目已经有大量可复用的东西。**动手前先找,别自己造。**

| 你可能想造的 | 已经有的,直接用 |
| --- | --- |
| HTTP 客户端 | `crate::http_client::get()` —— 全局共享的 reqwest client,带连接池 |
| 错误类型 | `crate::error::AppError` |
| 数据库访问 | `crate::database::Database`,DAO 在 `src-tauri/src/database/dao/` |
| 时间戳 | `chrono`;仓库里到处是 `now_timestamp()` / `now_millis()` 的现成实现 |
| 日志 | `log::warn!` / `log::error!`(已初始化) |
| 配置读写 | `crate::settings::*` |
| Codex 路径 | `crate::agent_paths::get_codex_auth_path()` 等 |

**反面例子(不要这样)**:为了拿一个时间戳去引入新 crate;为了发一个 HTTP 请求自己
`reqwest::Client::new()`(应该用 `http_client::get()`,否则每次都新建连接池)。

**如果一件事看起来需要绕很大的圈子才能做到,大概率是你没找到现成的东西。停下来问。**

---

## 2. 任务依赖与并行分组

```
第一批（三个可同时开，互不碰同一个文件）
  ├─ T1  会话日志共享流水线
  ├─ T2  subscription.rs 按 provider 拆分
  └─ T3  路由的存储层 + schema 迁移

第二批（都依赖 T3 完成并合并）
  ├─ T4  路由决策（纯函数，无 IO）
  └─ T5  失败分类（纯函数，无 IO）
        ↑ T4 T5 可同时开

第三批（依赖 T4 + T5）
  ├─ T6  HTTP 转发层
  └─ T7  指针写入 + 逃生命令
        ↑ T6 T7 可同时开（不碰同一个文件）
```

| 任务 | 文件 | 依赖 | 能否并行 |
| --- | --- | --- | --- |
| [T1](T1-ingest-pipeline.md) | 会话日志共享流水线 | 无 | ✅ 与 T2 T3 |
| [T2](T2-subscription-split.md) | `subscription.rs` 拆分 | 无 | ✅ 与 T1 T3 |
| [T3](T3-router-storage.md) | 路由存储 + 迁移 | 无 | ✅ 与 T1 T2 |
| [T4](T4-router-decision.md) | 路由决策纯函数 | T3 | ✅ 与 T5 |
| [T5](T5-failure-classify.md) | 失败分类纯函数 | T3 | ✅ 与 T4 |
| [T6](T6-router-forward.md) | HTTP 转发层 | T4 T5 | ✅ 与 T7 |
| [T7](T7-pointer-escape.md) | 指针写入 + 逃生命令 | T3 | ✅ 与 T6 |

**UI 不在此列** —— 设置界面、菜单栏状态由项目所有者自己实现。

### 2.1 并行时的硬要求

每个并行任务**必须在自己的 git 分支上做**,分支名用任务号:

```bash
git switch -c task/T1-ingest-pipeline main
```

**不要**多个任务在同一个工作区里并行改动。完成后各自提交,由项目所有者合并。

---

## 2.2 跨任务契约(T3–T7 全体必读,这几条不许各自发挥)

并行的几个任务在这几个点上必须**完全一致**,谁都不能自己定:

| 契约 | 值 | 谁依赖 |
| --- | --- | --- |
| Codex 打到本地 router 的完整路径 | **`POST /v1/responses`**(base_url 含 `/v1`) | T6 注册这个路由;T7 把 base_url 写成 `http://127.0.0.1:<port>/v1` |
| 路由模式存哪 | 现有 `settings` 表,键 **`router.mode`**,值 `"auto"` 或 `"manual:<provider_id>"` | T6 用 `db.get_setting("router.mode")` 读;读不到或解析不了**一律当 `auto`** |
| 监听地址 | 固定 `127.0.0.1`,**绝不** `0.0.0.0` | T6 |
| 拉黑状态存哪 | **内存,不落库**(60–600 秒的短时状态) | T6 |
| 上游凭据谁给 | **router 自己不持有**,由启动方注入一个闭包,见 T6 §3 | T6 |

`settings` 表已经存在,`get_setting` / `set_setting` 在
`src-tauri/src/database/dao/settings.rs`。**不要为 `router.mode` 建新表、不要动 schema**
——那是 T3 的地盘,而 T3 也不建这个。

---

## 3. 交付格式

每个任务完成后,在报告里给出:

1. **改了哪些文件**(逐个列出,不要漏)
2. **六项检查的实际输出**(粘贴 `test result:` 那几行,不要只说"通过了")
3. **你做了但任务书没要求的事**(如果有)——必须主动说明
4. **你想做但忍住没做的事**(如果有)——写下来,由项目所有者判断

**不要在报告里说"应该可以工作"。** 跑过就是跑过,没跑过就说没跑过。

---

## 4. 背景阅读(不长,但会省掉你很多弯路)

- `HANDOFF.md` —— 项目现状,**顶部有文档地图**
- `docs/design/2026-08-14-local-routing-design.md` —— 路由的完整设计与决策表
  (33 条,编号排到 37;9/17/19/20 已撤销,编号有意留空)。
  **T3–T7 的执行者必须读 §3、§4**
- `docs/design/2026-08-14-modular-core-and-providers.md` —— 模块化的划线原则。
  **T1、T2 的执行者必须读 §3、§4、§9**
- `AGENTS.md` —— 本地命令的包装器约定

**注意**:设计文档是背景,用来理解**为什么**这么设计;**任务书(本目录)才是要执行的
东西**。两者冲突时以任务书为准,并在报告里指出冲突在哪。
