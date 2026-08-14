# T7:指针写入 + 逃生命令

> **先读 [`README.md`](README.md) 的铁律。** 依赖:**T3 完成并合并**。
> 可与 T6 并行(不碰同一个文件)。
>
> **这个任务碰用户的真实配置文件。写坏了他的 Codex 就用不了。谨慎。**

---

## 背景:为什么这件事敏感

用户当初就是因为 CC Switch **反复复写 `~/.codex/config.toml` 把配置写坏**才要换方案。
所以这次的规矩是:

> **指针只写一次,之后永不再改。**(设计文档决定 34)

---

## 1. 要做的三件事

1. 一个函数:把 Codex 的 `model_provider` 指向本地 router(**只写这一次**)
2. 一个函数:启动时检查指针**是不是自己写的**,不是就别动它
3. 一个**不依赖 app 存活**的命令行回退方式

---

## 2. 新建文件

`src-tauri/src/router/pointer.rs`,在 `router/mod.rs` 里 `pub mod pointer;`。

---

## 3. 写入(照做)

```rust
/// 把 Codex 的 model_provider 指向本地 router。
///
/// 只改**顶层的 model_provider 这一个键**，以及新增一个
/// [model_providers.llm_usage_bar_router] 段。文件里其余内容——MCP 服务器、
/// 已信任的项目、用户自己的注释——**一个字节都不能动**。
pub fn point_codex_at_router(port: u16) -> Result<(), AppError>;
```

### 3.1 硬要求

- **写之前先备份**:同目录下 `config.toml.bak-<yyyymmdd-HHMMSS>-before-router`
- **原子写入**:写临时文件再 rename,不要直接截断原文件
- **写完必须读回校验**:能被 TOML 解析、且 `model_provider` 确实是新值。
  校验失败就**用备份还原**并返回 `Err`
- **绝不整体重写文件。** 用 `toml_edit`(已在依赖里,`0.25`)—— 它就是为
  「保留注释和格式的就地改写」造的。**不要用 `toml` crate 反序列化再序列化回去**,
  那会丢掉全部注释和键序
- 只允许出现两种改动:替换顶层 `model_provider` 那一个键的值,
  和追加一个 `[model_providers.llm_usage_bar_router]` 段。**别的一律不动**

### 3.2 写入内容

```toml
model_provider = "llm_usage_bar_router"

[model_providers.llm_usage_bar_router]
name = "LLM Usage Bar Router"
base_url = "http://127.0.0.1:<port>/v1"
wire_api = "responses"
```

**`base_url` 末尾的 `/v1` 是硬契约**(README §2.2):Codex 会往它后面接
`/responses`,而 T6 注册的正是 `POST /v1/responses`。**少写或多写这一段,
两边就对不上了**,而且表现为一个本地 404,很难查。

**注意 `requires_openai_auth` 与 `experimental_bearer_token` 不写在这里** ——
那两个键是"直连某一家"时才需要的(见 `HANDOFF.md` §13),经由 router 时认证由
router 侧处理。

---

## 4. 启动时的检查(照做)

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PointerState {
    /// 指向我们的 router，一切正常。
    OursAndCurrent,
    /// 指向别处——用户手动改过（逃生），或从未配置过。
    NotOurs { current: Option<String> },
    /// 文件读不到或解析不了。
    Unreadable,
}

pub fn inspect_pointer() -> PointerState;
```

**发现 `NotOurs` 时的处理(设计文档决定 36):**

- ❌ **不要静默覆盖**
- ✅ 返回状态,由上层提示用户
- ✅ 记一个标记:「这段时间的用量没有经过 router,受影响 provider 的估算不完整」

**这个函数只读不写。**

---

## 5. 逃生命令(最容易被忽略的一条)

设计文档**决定 37**:

> **逃生门不能只存在于「需要 app 活着才能用」的地方。**
> app 都挂了,app 里的按钮点不了。

所以要提供一个**独立于 app 的**回退方式。做法:

在 `scripts/` 下新建 `scripts/codex-unroute.sh`:

- 找到 `~/.codex/config.toml`
- 找到最近一个 `config.toml.bak-*-before-router` 备份并还原
- 若没有备份,则把顶层 `model_provider` 改回参数指定的值
  (用法:`./scripts/codex-unroute.sh packyapi`)
- **同样要先备份当前文件再动手**
- 用 `bash` 写(这是唯一允许用 shell 的地方——它必须在 app 不可用时能跑)

并在 `HANDOFF.md` 里加一句,写明这个脚本的位置和用法。

---

## 6. 测试

**不要在测试里碰用户真实的 `~/.codex/config.toml`。** 把路径做成参数,测试用
`tempfile` 造临时文件(`tempfile` 已是 dev-dependency)。

必测:

| # | 场景 | 期望 |
| --- | --- | --- |
| 1 | 一个含 MCP 段、注释、多个 provider 的配置 | 写入后**除了 `model_provider` 那行和新增的段,其余逐字节相同** |
| 2 | 已经有 `model_provider` 的 | 就地替换,不追加第二个 |
| 3 | 没有 `model_provider` 的 | 插在第一个 `[table]` 之前 |
| 4 | 写入后读回校验失败(喂一个会导致解析失败的场景) | 用备份还原,返回 `Err` |
| 5 | `inspect_pointer` 对我们写的 → `OursAndCurrent` |  |
| 6 | `inspect_pointer` 对用户手改成 `packyapi` 的 → `NotOurs { current: Some("packyapi") }` |  |
| 7 | `inspect_pointer` 对损坏的 TOML → `Unreadable`,**不 panic** |  |

第 1 条是这个任务的核心:**逐字节比对**,不是"看起来差不多"。

---

## 7. 明确不要做的事

- ❌ **不要碰 `~/.codex/auth.json`** —— 一个字节都不要读写。那是 Codex 的官方登录,
  动它会把用户从 Codex 踢下线
- ❌ 不要在 app 退出时改指针(设计文档决定 35 已撤销这个设计)
- ❌ 不要持续监听 config.toml
- ❌ 不要整体重写配置文件
- ❌ 不要在没有备份的情况下写入
- ❌ 不要加新依赖

---

## 8. 完成的标准

- 三个函数按签名实现
- `scripts/codex-unroute.sh` 可用,并已写进 `HANDOFF.md`
- 7 条测试通过,其中第 1 条是逐字节比对
- 全程未读写 `auth.json`(在报告里明确声明)
- 六项检查全绿,`test result:` 行贴进报告
