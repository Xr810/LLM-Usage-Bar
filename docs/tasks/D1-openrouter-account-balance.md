# D1:OpenRouter 账户余额模块(管理 key → `/api/v1/credits`)

> 给外部 AI agent 的实施任务书。**先读 `docs/tasks/README.md` 的铁律**,再读本文。
> 这是纯后端 Rust 任务。**不要碰 `src/`(前端)——面板由项目所有者自己做。**

---

## 0. 背景:面板要什么

设计文档 `docs/design/2026-08-14-local-routing-design.md` §5.3(已批准)规定,按量计费
provider 的卡片有上下两条量表:

```
今日花费    $3.20 / $10.00      ← 上条:本机记账(router/会话日志),本任务不负责
余额        $61.40 / $100.00    ← 下条:本任务负责
上次充值 $100 · 2 分钟前更新
```

决定 25:余额**优先问 provider 的 API**;决定 26/27:没有余额 API 才用估算(下界,视觉可辨)。

**本任务只做 OpenRouter 的「余额 + 参考额度(充值总额)」读取与暴露**,不做 UI、
不做上条、不做估算。

## 1. 上游 API(已调研,照此实现;响应字段名以 OpenRouter 官方文档为准)

- **凭据**:OpenRouter **Management API key**(管理 key),在 openrouter.ai 的
  Management API Keys 页创建。**它不能调模型,只能做管理/查询**——这正是本任务要的。
  普通 `sk-or-...` 模型 key 调 `/api/v1/credits` 会得到
  `403 {"error":{"code":403,"message":"Only management keys can perform this operation"}}`。
- **端点**:`GET https://openrouter.ai/api/v1/credits`
  - 头:`Authorization: Bearer <management key>`
  - 成功(200):
    ```json
    { "data": { "total_credits": 100.5, "total_usage": 25.75 } }
    ```
  - 语义:`total_credits` = 累计充值总额(参考额度,下条分母);
    `total_usage` = 累计消费;`余额 = total_credits − total_usage`。
  - 401 = key 无效/缺失;403 = 不是管理 key(两种都映射为明确的错误码,别混)。
  - OpenRouter 侧该数据有约 60 秒缓存延迟,属正常,不必重试。
- **已有且不要动的实现**:`system_provider_connection.rs` 的
  `refresh_key_usage` 已经用**普通模型 key** 抓 `/api/v1/key`(usage/limit/
  limit_remaining)。那一条继续管「每把 key 的花费」;**本任务新增的是账户级余额**,
  两条不互相替代。

### 1.1 ⚠️ 这个端点仓库里**已经实现了**(2026-08-15 核实,本任务因此改为扩展)

**`services/balance.rs:287` 的 `query_openrouter` 已经在打
`GET https://openrouter.ai/api/v1/credits`,并解析 `data.total_credits` /
`data.total_usage`、算出 `remaining = total_credits - total_usage`。**
本任务书早先版本没发现它,写成了「新建一个文件从零实现」——**那会造成同一个端点
两份实现、两套四舍五入口径。**

已有实现与本任务要求的差距,**这三条才是真正要做的事**:

| | 现状(`balance.rs`) | 本任务要求 |
| --- | --- | --- |
| 数值类型 | `f64`,`parse_f64_field(...)` | `rust_decimal::Decimal` → 十进制字符串,金额不 round |
| 缺字段 | `.unwrap_or(0.0)` —— **静默当 0** | `invalid_response` 明确报错(仓库 invariant:无价 ≠ 免费) |
| 凭据 | 用 provider 已存的模型 key | **专用管理 key**,固定 slot(§3) |
| 快照与调度 | 无,每次现抓 | 15 分钟调度 + 快照 + 失败退避(§4) |

**所以本任务是「把 `query_openrouter` 升级并接上管理 key + 调度」,不是新建模块。**
改造后 `balance.rs` 里其余四家(DeepSeek / StepFun / SiliconFlow / Novita)的行为
**必须逐字节不变** —— 它们不在本任务范围内。

## 2. 先决条件(开工前确认)

> **⚠️ 2026-08-15 路径已刷新。** 本任务书原写于 `b9a4d18a`,而 **T17 把后端整个重排过**
> (`database/`→`store/`、`credentials/`→`secrets/`、`commands/`→`api/commands/`,
> 且**根目录的 `store.rs` 已改名 `app_state.rs`,`store` 现在指数据库**)。
> 下面用的是重排后的真实路径,**照本文写的路径走,不要照旧记忆**。
>
> 落点还取决于 [`T21-provider-modules-and-sync.md`](T21-provider-modules-and-sync.md):
> 它会把 `services/balance.rs` 搬到 `providers/balance/mod.rs`。
> **T21 若已合并,就在新位置改;若未合并,就在 `services/balance.rs` 原地改** ——
> 两种情况都不要自己新建目录。

- 工作基线:`main` 分支(T17 合并后,顶端 `d6eb9f49` 或更新),`SCHEMA_VERSION = 28`。
- 仓库约定:所有 Rust/Tauri 命令走包装器 `pnpm rust -- <cargo 参数>`;
  **不要**直接 `cargo build/test/clippy`;注释和提交信息用中文。
- 六项交付检查(缺一不算完成,输出里贴 `test result:` 行):
  ```
  pnpm rust -- fmt --check
  pnpm rust -- clippy -- -D warnings
  pnpm rust -- test
  pnpm typecheck
  pnpm format:check
  pnpm test:unit
  ```
- 与 D2(PackyCode)并行开发;两个任务**唯一会碰到的共同文件**是
  `src-tauri/src/lib.rs` 和 `src-tauri/src/app_state.rs`(各自加自己的 mod/注册/启停,
  都是几行,合并时按行解决即可)。**不要去改 D2 的文件。**

## 3. 存储方案:**方案 A,已锁定,不再二选一**

> **2026-08-15 决定。** 原任务书让接手者在 A / B 之间选,现在不选了 —— 见
> [`README.md`](README.md) 铁律第 9 条:**优先不建表**。迁移一旦发布到用户机器上就
> 撤不回来,而本任务要存的只是单账号的一个快照,用不着一张表。
>
> **原方案 B(建表 + `migrate_v28_to_v29`)作废,不要实现。** 若你认为非建表不可,
> **停下来在报告里说明,不要自己动 `SCHEMA_VERSION`。**

- 管理 key 明文只进 OS 钥匙串:用现有 `crate::secrets::CredentialStore`
  (`src-tauri/src/secrets/mod.rs:27`,put/get/delete 接口),slot 名固定
  `"openrouter-management-key"`(单账号,固定 slot 即可,不需要 staging/journal 那套
  ——那是给 provider_api_keys 多 key 轮换用的)。
- 非机密状态存 **settings 表**(`src-tauri/src/store/dao/settings.rs` 的 `get_setting`/
  `set_setting`,键值表已存在):
  - `openrouter_account_balance` = JSON
    `{"total_credits_usd":"100.5","total_usage_usd":"25.75","balance_usd":"74.75","fetched_at":123}`
  - 值一律**十进制字符串**,不存浮点(仓库约定:金额不 round)。
- **不建表、不动 SCHEMA_VERSION、无迁移。**
- 管理 key 是否有配置的判断:调用 `credential_store.get("openrouter-management-key")`
  返回 `Some` 且非空。

## 4. 目标结构与改动清单

### 改造 `balance.rs` 的 OpenRouter 那一支(**不是新建文件**,见 §1.1)

位置:T21 未合并 → `src-tauri/src/services/balance.rs`;
T21 已合并 → `src-tauri/src/providers/balance/mod.rs`。

照 `official_pricing.rs` 的分层(网络函数与调度器可测),内容:

1. **网络层**(要能注入,便于单测;不要直接用真实 reqwest 写死):
   - 把现有 `query_openrouter`(`balance.rs:287`)升级:接受**管理 key**,
     返回 `Result<CreditsPayload, ...>`。
   - 解析只认 `data.total_credits` / `data.total_usage`,用 `rust_decimal::Decimal`
     转十进制字符串;**缺失/非数字/负数 → 明确错误,绝不 `unwrap_or(0.0)`**
     (现有代码正是这么写的,这是本任务要修掉的第一件事)。
   - 错误码区分:`authentication_failed`(401/403)、`connection_failed`(网络/超时,
     8 秒超时)、`invalid_response`(结构不符)、`upstream_rejected`(其他 4xx/5xx)。
   - **同文件里其余四家(DeepSeek / StepFun / SiliconFlow / Novita)一个字符都不许动。**
2. **存储层**:按 §3(方案 A)读写(两个函数:`load_snapshot` / `save_snapshot`)。
3. **调度器**(照 `official_pricing.rs:287` `start_scheduler` 的成熟模板):
   - `REFRESH_INTERVAL = 15 分钟`(与 key usage 调度一致);
   - 失败退避 `[30s, 60s, 300s, 600s, 1800s]`,**失败保留上次快照不动**;
   - 无管理 key 时调度器空转(不报错、不请求);
   - 成功后调 `crate::usage_events::notify_dashboard_invalidated()`;
   - 轻量模式(lightweight)不跑,参考 `provider_key_usage_scheduler.rs` 的判断。
4. **对外视图结构**:
   ```rust
   pub struct OpenRouterAccountBalanceView {
       pub total_credits_usd: Option<String>, // 参考额度
       pub total_usage_usd: Option<String>,
       pub balance_usd: Option<String>,       // total_credits - total_usage
       pub fetched_at: Option<i64>,
       pub has_management_key: bool,
   }
   ```
   金额字段全 `Option<String>`,无快照时全 `None`(面板自己决定怎么显示),
   **绝不编造 0**。

### 新建 `src-tauri/src/api/commands/openrouter_balance.rs`

> T16/T17 之后 `commands/` 已并入 `api/commands/`。**分层照 `api/router.rs` 的做法**:
> 业务编排放在 `api/` 顶层的与传输无关的模块里(不 import tauri),
> `api/commands/` 只放 tauri 薄壳。这样将来 socket 面板能直接调同一份编排。
>
> **注**:`api/mod.rs` 顶部那句「本目录任何文件不得 import 任何 tauri 类型」
> 写于 T16,当时 `commands/` 还在外面;T17 把它搬进来后这句话已经不准确
> (`api/commands/` 下 14 个文件 import tauri)。**约定的实质没变** ——
> 不 import tauri 的是 `api/` 顶层那些编排模块。不要因为这句注释就把
> tauri 命令放到 `api/` 外面去。

三个 tauri 命令(只暴露视图,不暴露明文):

- `set_openrouter_management_key(state, key: String) -> Result<(), AppError>`
  —— trim 后存钥匙串;空串拒绝;成功/失败不清已有值(参考现有 provider key 的
  set 语义:验证后才替换)。**日志、错误串、返回体不得含 key 明文**。
- `clear_openrouter_management_key(state) -> Result<(), AppError>`
  —— 删钥匙串条目 + 清余额快照。
- `get_openrouter_account_balance(state) -> Result<OpenRouterAccountBalanceView, AppError>`
  —— 只读 settings/表,不发请求。

### 接线(小改)

- `src-tauri/src/app_state.rs`(**T17 前叫 `store.rs`;现在的 `store` 是数据库,别搞混**):
  `AppState` 加调度器 handle 字段 + `start_/take_` 方法 —— 照
  `official_pricing_scheduler` 的三处写法(`app_state.rs:36`、`:121`、`:185`)。
- `src-tauri/src/lib.rs`:`mod` 声明、`invoke_handler` 注册三个命令、启动/退出路径
  挂 start/stop(照 official_pricing 的挂法)。

## 5. 明确不做

- ❌ 不碰前端 `src/`、不写 React 组件
- ❌ 不动 `/api/v1/key` 那条已有链路(那是 per-key 花费,语义不同)
- ❌ 不做"没有管理 key 时的估算/回退"(用户已拍板:只用管理 key)
- ❌ 不读/不存用户的普通模型 key 当管理 key(403 会告诉你区别,测试里断言这个错误路径)
- ❌ 不加新 crate;HTTP 用 `crate::http_client::get()` 的共享 reqwest client
- ❌ 不改 D2 的文件
- ❌ **不建表、不动 `SCHEMA_VERSION`、不写迁移**(§3 已锁定方案 A)
- ❌ **不新建 `openrouter_balance.rs`** —— 端点已在 `balance.rs` 实现,本任务是改造它(§1.1)
- ❌ 不动 `balance.rs` 里 DeepSeek / StepFun / SiliconFlow / Novita 四家的行为
- ❌ 不做 T21 的搬运 —— 若 `balance.rs` 还在 `services/`,就在原地改,不要顺手建 `providers/`

## 6. 测试(必须)

1. 200 正常解析:`total_credits/total_usage` → balance 正确(含"余额=充-用"的减法断言);
2. 401/403 → `authentication_failed`,且**不写快照**;
3. 网络错误 → `connection_failed`,旧快照保留(settings 里旧值不动);
   **另加一条回归**:改造后 DeepSeek / StepFun / SiliconFlow / Novita 四家的既有
   行为不变(现有测试若已覆盖就跑通它们,没覆盖就补一条最小断言);
4. 响应缺字段/负数/非数字 → `invalid_response`,绝不落 0;
5. 调度:成功一次后 15 分钟内不再请求;失败按退避阶梯重试;取消(cancel)即时退出
   (照 `provider_key_usage_scheduler.rs` 现有测试的写法);
6. 无管理 key:调度空转、`get_openrouter_account_balance` 返回 `has_management_key: false`、
   其余 `None`;
7. set/clear 命令:set 后钥匙串有值;clear 后钥匙串与快照都空;错误路径不泄 key 明文
   (扫描返回体/日志字符串)。

## 7. 交付格式

照 `docs/tasks/README.md` §3:文件清单、六项检查的 `test result:` 原文、做了任务书
之外的事、想改但忍住没改的事。

另外必须报告:

- **`balance.rs` 改造前后的 diff**,并说明其余四家为什么没被影响
- 你是在 `services/balance.rs` 还是 `providers/balance/mod.rs` 上改的(取决于 T21)
