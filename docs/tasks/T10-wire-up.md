# T10:把 router 接进 app,并给前端一组命令

> **先读 [`README.md`](README.md) 的铁律。** 依赖:**T9 必须已完成并合并**。
> 不能与 T9 并行(它改 `RouterProvider` 的字段,你要把这些字段暴露出去)。
>
> **这个任务碰用户的真实 `~/.codex/config.toml`。** 写坏了他的 Codex 就用不了。

---

## 目标

到目前为止零件都造好了,但**一个都没装上**:

```
router::server::start()      没有任何调用方 → router 从不启动
point_codex_at_router()      没有任何调用方 → 指针从不写
inspect_pointer()            没有任何调用方 → 指针被改了也没人知道
router_providers 三张表      没有任何生产代码写入 → 永远是空的
```

你要做的是把它们装起来,并给前端一组命令。**装完之后,手动灌一条 provider
记录,Codex 应该能真的走一次本地 router。**

---

## 1. 启动接线

### 1.1 在哪接

`src-tauri/src/lib.rs` 的 Tauri `setup` 流程里。**先通读那一段**,找到数据库和
`CredentialService` 已经就绪、但还没开始做重活的位置。

**必须在启动流程的早期**(设计文档决定 5②:先把监听端口开起来,再做其余初始化)。

### 1.2 怎么接

```rust
let auth = Arc::new(crate::router::auth::RouterUpstreamAuth::new(
    db.clone(),
    credentials.clone(),
));
tauri::async_runtime::spawn(async move {
    if let Err(error) = crate::router::server::start(db, port, auth).await {
        log::error!("[ROUTER] 启动失败: {error}");
    }
});
```

### 1.3 端口从哪来

存在 `settings` 表,键 **`router.port`**,默认 **`8788`**。
读不到或解析不了就用默认值,**不要报错、不要写回**。

### 1.4 绑定失败不能拖垮 app

端口被占是常见情况(上次没退干净、别的软件占了)。**绑定失败只记 `log::error!`
并让 app 继续启动** —— 不要 `panic!`、不要中止 setup。用户至少还能打开界面
看到出了什么事。

---

## 2. 指针的状态机(本任务最需要小心的部分)

设计文档决定 34:**指针只写一次,之后永不再改**。所以启动时**只读不写**:

```
启动 → inspect_pointer()
   ├─ OursAndCurrent → 什么都不做
   ├─ NotOurs { current } → 记一个标记,不覆盖(决定 36)
   └─ Unreadable        → 记一个标记,不覆盖
```

**`point_codex_at_router()` 只在用户显式点「启用」时调**,由 §3 的命令触发,
**绝不在启动时自动调**。

### 2.1 `NotOurs` 要留下什么

在 `settings` 里记一个键 **`router.pointer_gap_since`**,值是发现时的 epoch 毫秒
(已经有值就不覆盖,保留最早那次)。语义是「从这个时刻起,用量可能没经过 router,
受影响 provider 的估算不完整」(决定 36)。

指针恢复成 `OursAndCurrent` 时**删掉这个键**。

**本任务只负责记这个标记,不负责在 UI 上显示** —— 显示是前端的活。

---

## 3. Tauri 命令

新建 `src-tauri/src/commands/router.rs`,在 `commands/mod.rs` 挂上,并在 `lib.rs`
的 `invoke_handler` 列表里注册(照抄旁边现有命令的写法)。

**照抄仓库现有命令的风格**:返回 `Result<T, String>`,错误经既有的脱敏路径,
不要把内部错误原样抛给前端。

```rust
/// 列出全部 router provider（含未启用的，供设置界面编辑）。
pub async fn list_router_providers() -> Result<Vec<RouterProviderView>, String>;

/// 新增或更新一个 provider。
pub async fn upsert_router_provider(input: RouterProviderInput) -> Result<(), String>;

pub async fn delete_router_provider(id: String) -> Result<(), String>;

/// 某个 provider 的模型映射，全量替换。
pub async fn set_model_routes(provider_id: String, routes: Vec<ModelRouteInput>)
    -> Result<(), String>;

/// 读/写路由模式。值是 "auto" 或 "manual:<provider_id>"（README §2.2）。
pub async fn get_router_mode() -> Result<String, String>;
pub async fn set_router_mode(mode: String) -> Result<(), String>;

/// 指针当前状态，供界面显示「已接管 / 未接管 / 读不到」。
pub async fn inspect_router_pointer() -> Result<PointerStateView, String>;

/// 用户显式点「启用」时调用：写一次指针。
pub async fn enable_router_pointer() -> Result<(), String>;

/// 最近的路由尝试，供分账面板。
pub async fn recent_router_attempts(start_at: i64, end_at: i64)
    -> Result<Vec<RouterUsageSummaryView>, String>;
```

### 3.1 `RouterProviderInput` 绝不接受凭据

```rust
pub struct RouterProviderInput {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub wire_api: String,
    pub priority: i64,
    pub enabled: bool,
    pub auth_kind: String,
    /// 指向 provider_api_keys.id 的引用。**不是 key 本身。**
    pub credential_key_id: Option<String>,
}
```

**这个结构体里绝不能出现 `api_key` / `token` / `secret` 这类字段。**
前端存 key 走仓库既有的凭据命令,router 这边只认引用。

### 3.2 `set_router_mode` 的校验

- `"auto"` 直接存
- `"manual:<id>"` 要校验 `<id>` 在 `router_providers` 里存在,不存在返回 `Err`
- 其余一律 `Err`

(注意:T6 的**读**侧对垃圾值一律退回 auto —— 那是运行时的容错。
**写**侧要严,别让垃圾进库。)

---

## 4. 明确不要做的事

- ❌ **不要在启动时自动写指针** —— 只在 `enable_router_pointer` 里写
- ❌ 不要持续监听 `config.toml`
- ❌ 不要碰 `~/.codex/auth.json`
- ❌ 不要改 `router/server.rs`、`router/pointer.rs`、`router/auth.rs` 的任何逻辑
  (你是它们的调用方,不是维护者)
- ❌ 不要在命令里返回任何凭据明文
- ❌ 不要碰 `src/`(前端)—— 界面由项目所有者自己写
- ❌ 不要加新依赖

---

## 5. 测试

启动接线不好做纯单测,但下面这些必须有:

1. 端口读取:`router.port` 不存在 → 8788;是 `"9000"` → 9000;是 `"abc"` → 8788
2. `set_router_mode("auto")` 成功;`set_router_mode("manual:nonexistent")` → `Err`;
   `set_router_mode("垃圾")` → `Err`
3. `manual:<存在的 id>` 成功,且存进 `settings` 的值逐字节是 `"manual:<id>"`
4. `upsert_router_provider` 之后 `list_router_providers` 能读回全部字段(含
   `auth_kind` 与 `credential_key_id`)
5. `set_model_routes` 是**全量替换**:先设 3 条再设 1 条,库里只剩 1 条
6. 指针标记:`NotOurs` → 写入 `router.pointer_gap_since`;再来一次 `NotOurs`
   **不覆盖原值**;变成 `OursAndCurrent` → 键被删除
7. `RouterProviderInput` 里没有凭据字段(编译期就保证了,在报告里声明即可)

指针相关的测试**不要碰真实的 `~/.codex/config.toml`**,路径做成参数,用 `tempfile`。

---

## 6. 完成的标准

- `start()` 在启动早期被调用,绑定失败只记日志不拖垮 app
- 启动时只读指针不写;`point_codex_at_router` 只由 `enable_router_pointer` 触发
- 上面九个命令实现并注册,前端能调到
- `NotOurs` 会在 `settings` 留下 `router.pointer_gap_since`,恢复时删除
- 七条测试通过
- 六项检查全绿,**外加 `clippy --all-targets -- -D warnings` 也要绿**
- 报告里回答:**装完之后,用户手动灌一条 provider 记录,Codex 能不能真的走通一次?**
  如果不能,还差什么,逐条列出
