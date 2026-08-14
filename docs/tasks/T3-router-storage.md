# T3:路由的存储层 + schema 迁移

> **先读 [`README.md`](README.md) 的铁律。** 依赖:无。可与 T1、T2 并行。
> **T4、T5、T7 都等着这个任务,优先做。**

---

## 目标

给本地路由建三张表,并提供读写它们的 DAO。**只做存储,不做任何路由逻辑** ——
决策在 T4,转发在 T6。

---

## 1. 建表

在 `src-tauri/src/database/schema.rs` 里加一个迁移函数,并把
`src-tauri/src/database/mod.rs:58` 的 `SCHEMA_VERSION` 从 `26` 改成 `27`。

照抄现有 `migrate_vN_to_vN1` 的写法(文件里有十几个例子),函数名
`migrate_v26_to_v27`,并在版本分发的地方接上。**不要发明新的迁移机制。**

### 表 1:`router_providers` —— 有哪些上游

```sql
CREATE TABLE IF NOT EXISTS router_providers (
    id            TEXT PRIMARY KEY,      -- 稳定标识，如 "packyapi"、"official"
    display_name  TEXT NOT NULL,
    base_url      TEXT NOT NULL,
    wire_api      TEXT NOT NULL,         -- "responses" | "chat_completions"
    priority      INTEGER NOT NULL,      -- 越小越优先，全局顺序（v1 只有全局）
    enabled       INTEGER NOT NULL DEFAULT 1,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);
```

**注意**:凭据**不存这里**。凭据怎么取由 provider 模块负责(设计文档 §3 的划线)。
这张表只描述「有哪些上游、怎么连、什么顺序」。

### 表 2:`router_model_map` —— 哪家有哪个模型,叫什么

```sql
CREATE TABLE IF NOT EXISTS router_model_map (
    provider_id     TEXT NOT NULL,
    logical_model   TEXT NOT NULL,       -- Codex 发来的那个名字，如 "gpt-5.6-sol"
    upstream_model  TEXT NOT NULL,       -- 该家的真实 ID，可能与 logical 相同
    created_at      INTEGER NOT NULL,
    PRIMARY KEY (provider_id, logical_model),
    FOREIGN KEY (provider_id) REFERENCES router_providers(id) ON DELETE CASCADE
);
```

**这张表是必须的,不是可选的**:故障转移切过去的那家未必有这个模型(官方就没有
`gpt-5.6-sol`),不知道就只是换个地方 400。见设计文档决定 22。

### 表 3:`router_attempts` —— 每次路由决策的记录

```sql
CREATE TABLE IF NOT EXISTS router_attempts (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at     INTEGER NOT NULL,     -- epoch 毫秒
    logical_model  TEXT NOT NULL,
    provider_id    TEXT NOT NULL,
    outcome        TEXT NOT NULL,        -- "success" | "failed" | "skipped"
    failure_kind   TEXT,                 -- 失败时填，取值见 T5
    http_status    INTEGER,
    input_tokens   INTEGER,
    output_tokens  INTEGER,
    duration_ms    INTEGER
);

CREATE INDEX IF NOT EXISTS idx_router_attempts_started
    ON router_attempts(started_at);
CREATE INDEX IF NOT EXISTS idx_router_attempts_provider
    ON router_attempts(provider_id, started_at);
```

**为什么要记 token**:会话日志把 Codex 的用量整体绑到一个 provider,不区分实际
走了哪家;router 是全 app 唯一同时知道「多少 token」和「哪一家」的地方。
详见设计文档 §5.1。

**这张表与 `usage_events` 语义不同,不要相加** —— 那张管总量,这张管分账。

---

## 2. DAO

新建 `src-tauri/src/database/dao/router.rs`,并在
`src-tauri/src/database/dao/mod.rs` 里挂上(照抄旁边现有 DAO 的挂载写法)。

**照抄现有 DAO 的风格**(看 `dao/usage_providers.rs`):用 `lock_conn!` 宏拿连接,
错误用 `AppError`,不要自己造错误类型。

需要的方法,**签名照写,不要改**:

```rust
impl Database {
    /// 全部启用的 provider，按 priority 升序。
    pub fn list_router_providers(&self) -> Result<Vec<RouterProvider>, AppError>;

    /// 某个逻辑模型在各家的映射，按 provider 的 priority 升序。
    /// 返回的每一项都保证 provider 是 enabled 的。
    pub fn list_model_routes(
        &self,
        logical_model: &str,
    ) -> Result<Vec<ModelRoute>, AppError>;

    pub fn upsert_router_provider(&self, p: &RouterProvider) -> Result<(), AppError>;
    pub fn delete_router_provider(&self, id: &str) -> Result<(), AppError>;

    pub fn upsert_model_route(&self, r: &ModelRoute) -> Result<(), AppError>;
    pub fn delete_model_routes_for_provider(&self, provider_id: &str)
        -> Result<(), AppError>;

    pub fn record_router_attempt(&self, a: &RouterAttempt) -> Result<i64, AppError>;

    /// 按 provider 汇总某段时间的用量，供分账面板使用。
    pub fn sum_router_usage_by_provider(
        &self,
        start_at: i64,
        end_at: i64,
    ) -> Result<Vec<RouterUsageSummary>, AppError>;
}
```

类型定义放在 `src-tauri/src/database/dao/router.rs` 顶部:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct RouterProvider {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub wire_api: WireApi,
    pub priority: i64,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireApi {
    Responses,
    ChatCompletions,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelRoute {
    pub provider_id: String,
    pub logical_model: String,
    pub upstream_model: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouterAttempt {
    pub started_at: i64,
    pub logical_model: String,
    pub provider_id: String,
    pub outcome: AttemptOutcome,
    pub failure_kind: Option<String>,
    pub http_status: Option<u16>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptOutcome {
    Success,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouterUsageSummary {
    pub provider_id: String,
    pub attempts: i64,
    pub failures: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
}
```

`WireApi` 与 `AttemptOutcome` 存进库时用字符串(`"responses"` / `"chat_completions"`,
`"success"` / `"failed"` / `"skipped"`),读出来解析回枚举。**解析失败不要 panic**,
返回 `AppError`。

---

## 3. 测试(必须写,而且必须过)

在 `dao/router.rs` 末尾加 `#[cfg(test)] mod tests`,用 `Database::memory()`
(仓库里到处是这个用法,照抄)。**至少覆盖**:

1. `list_router_providers` 按 priority 升序返回,且**跳过 enabled = 0 的**
2. `list_model_routes` 只返回该 logical_model 的行,且**按 provider 的 priority 排序**
3. `list_model_routes` 对一个不存在的模型返回空 Vec,**不是错误**
4. `delete_router_provider` 会连带删掉它在 `router_model_map` 里的行(外键级联)
5. `record_router_attempt` 写入后能被 `sum_router_usage_by_provider` 正确汇总,
   且**时间范围之外的记录不计入**
6. `WireApi` / `AttemptOutcome` 存进去再读出来是同一个值(往返)
7. 库里存了非法的 wire_api 字符串时,读取返回 `Err`,**不 panic**

---

## 4. 明确不要做的事

- ❌ 不要写任何路由决策逻辑(那是 T4)
- ❌ 不要写任何 HTTP 相关代码(那是 T6)
- ❌ 不要碰 `config.toml`(那是 T7)
- ❌ 不要改 `usage_events` 或任何现有的表
- ❌ 不要在这三张表里存凭据、token、API key
- ❌ 不要加新依赖

---

## 5. 完成的标准

- `SCHEMA_VERSION` 已改为 27,迁移函数已接上版本分发
- 三张表 + 两个索引建出来了
- 上面列的 DAO 方法全部实现,签名与本文一致
- 七条测试全部通过
- 六项检查全绿(README §0 第 6 条),并把 `test result:` 行贴进报告
