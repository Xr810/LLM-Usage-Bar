# T9:router 的凭据解析(`UpstreamAuth` 的真实实现)

> **先读 [`README.md`](README.md) 的铁律。** 依赖:T3、T6 已合并。
> **不能与 T10 并行** —— 你改 schema 和 `RouterProvider`,T10 要把新字段暴露给前端。
>
> **这是这批里唯一碰凭据的任务。写错了会泄露用户的订阅账号凭据或把他从 Codex 踢下线。**
> §5「安全铁律」先读完再动手,那七条一条都不能破。

---

## 目标

T6 定义了 `UpstreamAuth` trait 但没有实现它 —— 现在整个 crate 里只有测试里的
`FakeAuth`。也就是说 **router 起来了也发不出一个能通过认证的请求**。

你要做的是:让每个 `router_providers` 行知道自己的凭据从哪来,并写出真正的
`UpstreamAuth` 实现。

---

## 1. 先看清楚已经有什么(动手前必读)

**不要自己造凭据存储。** 仓库里已经有一整套,而且是加密 + 钥匙串 + 指纹校验的:

| 你需要的 | 已经有的 |
| --- | --- |
| 取一把已存的 API key(明文) | `CredentialService::resolve_provider_api_key(key_id, expected_version)`,见 `src-tauri/src/credentials/service.rs:442` |
| 用法范例 | `src-tauri/src/services/system_provider_connection.rs:392` 附近,三处调用照抄 |
| ChatGPT 的 OAuth 凭据 | `crate::services::subscription::codex::read_codex_credentials()`(读钥匙串,失败回落读文件) |
| 秘密的内存表示 | `Zeroizing<Vec<u8>>` / `SecretString`,见 `credentials/mod.rs` |

`resolve_provider_api_key` **已经做了**版本校验、指纹常数时间比对、以及取回后的
二次一致性检查。**照用,不要绕过它自己去读 `CredentialStore`。**

---

## 2. schema v28:给 `router_providers` 加两列

把 `SCHEMA_VERSION` 从 `27` 改成 `28`,加 `migrate_v27_to_v28`,照抄 v26→v27 的写法。

```sql
ALTER TABLE router_providers ADD COLUMN auth_kind TEXT NOT NULL DEFAULT 'none';
ALTER TABLE router_providers ADD COLUMN credential_key_id TEXT;
```

- `auth_kind`:`"chatgpt_oauth"` | `"bearer_key"` | `"none"`
- `credential_key_id`:**只在 `auth_kind = "bearer_key"` 时有意义**,值是
  `provider_api_keys.id`。**它是一个引用,不是凭据** —— T3「这三张表不存凭据」的
  规矩仍然成立,这里存的是「去哪把凭据取出来」,不是凭据本身。

### 2.1 改 SCHEMA_VERSION 的连带改动(已授权)

与 T3 §1.0 同款,这几处必须一起改,不改测试必挂:

| 文件 | 怎么改 |
| --- | --- |
| `src-tauri/src/lib.rs` | `SCHEMA_VERSION != 27` → `!= 28`,并在注释块末尾追加一句 `// Reviewed for schema v28: v27 -> v28 只给 router_providers 加两个可空/带默认的列，与 v13 基线无关。` |
| `src-tauri/src/database/tests.rs` | 断言 `get_user_version == 27` 的地方全部改成 `28`(先 grep 确认有几处,一处不漏) |

**除这两个文件外,不要碰任何其他清单外文件。**

### 2.2 DAO 跟着改

`RouterProvider` 结构体加两个字段:

```rust
pub auth_kind: RouterAuthKind,
pub credential_key_id: Option<String>,
```

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouterAuthKind {
    ChatgptOauth,
    BearerKey,
    None,
}
```

存库用字符串,读出来解析回枚举,**非法值返回 `AppError` 不 panic** —— 与
`WireApi` 完全同款,照抄它的 `wire_api_to_db` / `wire_api_from_db`。
`list_router_providers` 与 `upsert_router_provider` 都要带上这两列。

---

## 3. 新建 `src-tauri/src/router/auth.rs`

在 `router/mod.rs` 里 `pub mod auth;`。

```rust
/// `UpstreamAuth` 的真实实现：按 provider 行上记的 auth_kind 去取凭据。
///
/// **本结构体不缓存任何凭据**——每次 `headers_for` 都重新取。凭据的生命周期
/// 越短越好，缓存换来的那点性能不值得多一个泄露面。
pub struct RouterUpstreamAuth {
    db: Arc<Database>,
    credentials: Arc<CredentialService>,
}

impl RouterUpstreamAuth {
    pub fn new(db: Arc<Database>, credentials: Arc<CredentialService>) -> Self;
}

impl UpstreamAuth for RouterUpstreamAuth {
    fn headers_for(&self, provider_id: &str) -> Result<Vec<(String, String)>, AppError>;
}
```

### 3.1 三个分支怎么做

| `auth_kind` | 做什么 |
| --- | --- |
| `None` | 返回 `Ok(vec![])` |
| `BearerKey` | `credential_key_id` 为 `None` → `Err`;否则用 `resolve_provider_api_key` 取回,返回 `[("Authorization", format!("Bearer {key}"))]` |
| `ChatgptOauth` | 用 `read_codex_credentials()` 取 access token,返回 `[("Authorization", format!("Bearer {token}"))]` |

**`provider_id` 在 `router_providers` 里查不到 → 返回 `Err`,不要静默返回空头。**
空头会让请求以「未认证」的形态打到上游,拿回一个 401,然后被 T5 判成
`RequestRejected` —— 用户看到的错误会指向完全错误的方向。

### 3.2 `headers_for` 是同步的,但取凭据是 async

`UpstreamAuth::headers_for` 的签名是同步的(T6 定的,不要改)。
`resolve_provider_api_key` 是 `async`。**不要用 `block_on`** —— 那会在 tokio
worker 线程上阻塞,router 正跑在上面。

用 `tauri::async_runtime::block_on` 也不行,理由同上。

**做法**:在 `RouterUpstreamAuth::new` 里不做任何事;在 `headers_for` 里用
`futures::executor::block_on` **也不行**。

正确做法是**把 async 挡在外面**:`headers_for` 里只做同步的事 ——
`read_codex_credentials()` 本来就是同步的;`bearer_key` 那条改用
**同步路径**取 key。如果 `CredentialService` 没有同步取 key 的入口,
**停下来在报告里说明**,不要自己造一个绕过版本校验和指纹比对的读法。

> 这一条是本任务最可能卡住的地方。**卡住就停下来问,不要发明**。
> 宁可交一个「`bearer_key` 分支返回 `Err("待接同步凭据入口")`」的半成品,
> 也不要为了跑通而绕过凭据体系。

---

## 4. 测试(必须写,而且必须过)

用 `Database::memory()`,凭据侧用假的 `CredentialStore`(仓库里
`credentials/tests.rs` 有现成写法,照抄)。

| # | 场景 | 期望 |
| --- | --- | --- |
| 1 | `auth_kind = none` | `Ok(vec![])` |
| 2 | `auth_kind = bearer_key` 且 key 存在 | 恰好一个头,值是 `Bearer <明文>` |
| 3 | `auth_kind = bearer_key` 但 `credential_key_id` 是 `None` | `Err` |
| 4 | `auth_kind = bearer_key` 但那把 key 已被删 | `Err`,**不 panic** |
| 5 | `provider_id` 在表里不存在 | `Err`,**不是** `Ok(vec![])` |
| 6 | 库里 `auth_kind` 是非法字符串 | 读取返回 `Err`,不 panic |
| 7 | 往返:三个枚举值存进去读出来是同一个 | 相等 |
| 8 | **错误信息里不含凭据明文**:构造一把已知内容的 key,让第 4 条那样失败,断言 `err.to_string()` 里搜不到那个明文 |

第 8 条是安全测试,**必须有**。

---

## 5. 安全铁律(破任何一条,成果作废)

1. **绝不 `log::` 任何凭据明文**,也不要打印它的长度、前缀、后缀
2. **绝不把凭据写进任何错误信息**(第 8 条测试就是盯这个)
3. **绝不把凭据写进数据库**(包括 `router_attempts` 的任何列)
4. **绝不缓存凭据**到结构体字段或全局
5. **绝不读写 `~/.codex/auth.json`** —— 取 ChatGPT 凭据只走
   `read_codex_credentials()` 这个既有入口,它自己知道该读哪
6. **绝不绕过 `resolve_provider_api_key`** 直接碰 `CredentialStore`
7. **绝不把客户端来的头当作凭据回落** —— 取不到就 `Err`,不要「那就用客户端的」

报告里逐条确认这七条。

---

## 6. 明确不要做的事

- ❌ 不要接线到启动流程(那是 T10)
- ❌ 不要写 tauri 命令(那是 T10)
- ❌ 不要改 `router/server.rs`(T6 的地盘,`UpstreamAuth` trait 的定义不许动)
- ❌ 不要碰 `src/`(前端)
- ❌ 不要加新依赖

---

## 7. 完成的标准

- schema 28,迁移接上,§2.1 两处连带改动已完成
- `RouterProvider` 带上两个新字段,`RouterAuthKind` 往返可解析、非法值返 `Err`
- `router/auth.rs` 建好并挂上,`RouterUpstreamAuth` 实现 `UpstreamAuth`
- 8 条测试全过(第 8 条是安全测试)
- §5 七条逐条确认
- 六项检查全绿,**外加 `clippy --all-targets -- -D warnings` 也要绿**
  (这批的整合分支目前是这个水平,别退化)
