# D2:PackyCode 账户用量模块(NewAPI 管理接口 → 余额/花费)

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

决定 25:余额**优先问 provider 的 API**。PackyCode 是 NewAPI 搭建的中转
(与 openai-api 等按量 provider 不同:它的余额/花费只有**管理接口**能读,
模型 key 读不到)。

**本任务只做 PackyCode 的「余额 + 已用 + 参考额度(累计充值)」读取与暴露,**
不做 UI、不做上条、不做估算。

## 1. 上游 API(已调研,照此实现)

### 1.1 凭据:系统访问令牌 + 用户 ID(注意:不是模型 key)

PackyAPI 官方文档(用量查询配置页)原文:

> 用量查询需要使用 PackyApi 的**系统访问令牌**和**用户 ID**,
> **不是**配置供应商时使用的 API Key。

- **系统访问令牌**(System Access Token):登录 `www.packyapi.ai` 控制台,
  个人设置 → 安全设置 → 生成系统访问令牌。这是**机密**,待遇同密钥。
- **用户 ID**:个人设置页顶部可见。**不是机密**(但也不要到处打日志)。
- 请求地址(默认,须可配置):`https://cf.api.fan`

### 1.2 NewAPI 管理接口(路径/字段源自 new-api 开源项目与 CC Switch 官方模板)

NewAPI 管理 API 与 OpenAI 兼容转发 API 是**两套**;下面用到的全部挂 base 下:

| 端点 | 用途 |
| --- | --- |
| `GET {base}/api/status` | 公开(无需认证):`version`、`quota_per_unit`(单位换算,默认 **500000 = 1 USD**,运营方可改)、`display_in_currency`、`custom_currency_symbol`、`system_name` |
| `GET {base}/api/user/self` | 账户余额:返回 `{ "success": true, "message": "", "data": {...} }` |
| `GET {base}/api/log/self/stat?type=2&start_timestamp=&end_timestamp=` | 时间段消费统计(可选做;type=2=消费),返回 `{"quota": 85000, "rpm": 12, "tpm": 3400}` |

`/api/user/self` 的 `data` 关键字段:

| 字段 | 含义 |
| --- | --- |
| `quota` | **剩余**额度(整数单位) |
| `used_quota` | 累计已用(整数单位) |
| `request_count` | 累计请求数 |
| `group` | 用户组(如 `default`、`vip`) |
| `username` / `display_name` | 账号名 |
| `role` | 1=普通 / 10=管理员 / 100=root(不必用) |
| `status` | 1 启用 / 2 禁用 |

**请求头**(照 CC Switch 官方 NewAPI 模板写;new-api 对 `Authorization` 会剥掉
`Bearer ` 前缀,所以带不带都能过,统一带):

```
Authorization: Bearer <系统访问令牌>
New-Api-User: <用户ID>
User-Agent: <本项目自己的 UA 字符串>
Accept: application/json
```

### 1.3 单位换算(重要,别写死)

- `quota` 是整数单位,**钱 = quota / quota_per_unit**;`quota_per_unit` 默认 500000
  (=1 USD),但运营方可改。
- 所以:先 `GET /api/status` 读 `quota_per_unit`;**读不到或请求失败时回退 500000**。
- PackyAPI 充值比例 1:1(1 元人民币 = 1 美元额度),所以换算出的就是 USD。
- 余额 USD = `quota / quota_per_unit`;已用 USD = `used_quota / quota_per_unit`;
  参考额度(累计充值)USD = `(quota + used_quota) / quota_per_unit`。
- 用 `rust_decimal::Decimal` 算(仓库已有该依赖),结果存十进制字符串,不存浮点。

### 1.4 实测注意(2026-08-15 从数据中心 IP 探测)

- `https://cf.api.fan` 前面有 **Cloudflare 机器人防护**:未带真实凭据的探测
  得到过 `404 "not found"`(纯文本)与 403 挑战页(HTML)。**这不是端点不存在**——
  是防护在拦。实现必须:
  - 任何非 JSON / 非 2xx 响应都按错误码处理,绝不 panic;
  - 解析失败给出明确 `invalid_response`,不要试图从 HTML 里抠字段;
  - 真实验收必须在用户本机用真实令牌跑(用户会提供)。
- 端点路径可能有 `/v1` 前缀变体(探测 `/v1/api/user/self` 与 `/api/user/self`
  行为不同)。**base_url 做成可配置字段**(默认 `https://cf.api.fan`),存进
  settings;抓取用 `{base}/api/user/self`,若 404 可让配置改填
  `https://cf.api.fan/v1`。**不要把路径前缀写死。**

## 2. 先决条件(开工前确认)

> **⚠️ 2026-08-15 路径已刷新。** 本任务书原写于 `b9a4d18a`,而 **T17 把后端整个重排过**
> (`database/`→`store/`、`credentials/`→`secrets/`、`commands/`→`api/commands/`,
> 且**根目录的 `store.rs` 已改名 `app_state.rs`,`store` 现在指数据库**)。
> 照本文写的路径走,不要照旧记忆。
>
> **本任务确认是全新实现**(2026-08-15 核实:仓库里没有任何 NewAPI 管理端点的代码,
> `packyapi` 只作为路由决策测试的夹具名出现)。这一点与 D1 不同 ——
> D1 的端点已经存在,那份任务书已改成「改造」。
>
> 落点取决于 [`T21-provider-modules-and-sync.md`](T21-provider-modules-and-sync.md):
> T21 已合并 → 新建 `providers/packycode/mod.rs`;
> **T21 未合并 → 新建 `services/packycode_usage.rs`,不要自己建 `providers/` 目录。**

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
- 与 D1(OpenRouter)并行开发;两个任务**唯一会碰到的共同文件**是
  `src-tauri/src/lib.rs` 和 `src-tauri/src/app_state.rs`(各自加自己的 mod/注册/启停,
  都是几行,合并时按行解决即可)。**不要去改 D1 的文件** —— D1 改的是
  `balance.rs` 里 OpenRouter 那一支。

## 3. 存储方案:**方案 A,已锁定,不再二选一**

> **2026-08-15 决定。** 原任务书让接手者在 A / B 之间选,现在不选了 —— 见
> [`README.md`](README.md) 铁律第 9 条:**优先不建表**。迁移一旦发布到用户机器上就
> 撤不回来,而本任务要存的只是单账号的一个快照,用不着一张表。
>
> **原方案 B(建表 + `migrate_v28_to_v29`)作废,不要实现。** 因此 D1/D2 之间
> **不再存在「谁拥有迁移」的协调问题** —— 两边都不建表。若你认为非建表不可,
> **停下来在报告里说明,不要自己动 `SCHEMA_VERSION`。**

- **系统访问令牌**只进 OS 钥匙串:用现有 `crate::secrets::CredentialStore`
  (`src-tauri/src/secrets/mod.rs:27`,put/get/delete),slot 名
  `"packycode-usage-token"`(单账号,固定 slot,不需要 staging/journal 那套)。
- 非机密状态存 **settings 表**(`src-tauri/src/store/dao/settings.rs` 的 `get_setting`/
  `set_setting`):
  - `packycode_usage_user_id` = 用户 ID 字符串(非机密)
  - `packycode_usage_base_url` = base_url(默认 `https://cf.api.fan`)
  - `packycode_account_usage` = JSON
    `{"balance_usd":"61.40","used_usd":"38.60","reference_usd":"100.00","quota_per_unit":"500000","fetched_at":123}`
  - 金额一律十进制字符串,不存浮点。
- **不建表、不动 SCHEMA_VERSION、无迁移。**

## 4. 目标结构与改动清单

### 新建 packycode 模块(位置见 §2:`providers/packycode/mod.rs` 或 `services/packycode_usage.rs`)

照 `official_pricing.rs` 的分层(网络函数与调度器可测),内容:

1. **网络层**(要能注入,便于单测):
   - `fetch_status(client, base_url) -> Result<Option<String>, ...>`
     —— 读 `quota_per_unit`;404/非 JSON/超时 → `None`(回退 500000),不视为硬失败;
   - `fetch_self(client, base_url, token, user_id) -> Result<SelfPayload, ...>`
     —— 两头发齐;`success == false` → `upstream_rejected`(带上 `message`,
     **message 可能含运营方文案,记日志时截断到 200 字符**)。
   - 错误码区分:`authentication_failed`(401/403)、`connection_failed`(网络/超时,
     8 秒超时)、`invalid_response`(非 JSON/缺字段)、`upstream_rejected`。
   - 解析只认 §1.2 字段;缺失/负数/非整数 → `invalid_response`,绝不静默当 0。
2. **换算层**(纯函数,必须有单测):`fn quota_units_to_usd(units, quota_per_unit)
   -> String`,用 `rust_decimal::Decimal`,输出 normalize 过的十进制字符串;
   `quota_per_unit <= 0` → 按 500000。
3. **存储层**:按 §3(方案 A)读写三个 settings 键。
4. **调度器**(照 `official_pricing.rs:287` `start_scheduler` 的模板):
   - `REFRESH_INTERVAL = 15 分钟`(与 key usage 调度一致);
   - 失败退避 `[30s, 60s, 300s, 600s, 1800s]`,**失败保留上次快照不动**;
   - 未配置令牌时调度器空转(不报错、不请求);
   - 成功后调 `crate::usage_events::notify_dashboard_invalidated()`;
   - 轻量模式不跑,参考 `provider_key_usage_scheduler.rs` 的判断。
5. **对外视图结构**:
   ```rust
   pub struct PackyCodeAccountUsageView {
       pub balance_usd: Option<String>,      // 剩余
       pub used_usd: Option<String>,         // 累计已用
       pub reference_usd: Option<String>,    // 累计充值(quota+used)
       pub quota_per_unit: Option<String>,   // 展示换算口径,便于诊断
       pub fetched_at: Option<i64>,
       pub has_credentials: bool,
   }
   ```
   金额字段全 `Option<String>`,无快照时全 `None`,**绝不编造 0**。

### 新建 `src-tauri/src/api/commands/packycode_usage.rs`

> T16/T17 之后 `commands/` 已并入 `api/commands/`。**分层照 `api/router.rs` 的做法**:
> 业务编排放在 `api/` 顶层的与传输无关的模块里(不 import tauri),
> `api/commands/` 只放 tauri 薄壳,将来 socket 面板能直接调同一份编排。
>
> **注**:`api/mod.rs` 顶部那句「本目录任何文件不得 import 任何 tauri 类型」
> 写于 T16(当时 `commands/` 还在外面),T17 搬进来后这句话已不准确。
> 约定的实质没变 —— 不 import tauri 的是 `api/` 顶层那些编排模块。

四个 tauri 命令(只暴露视图,不暴露令牌明文):

- `set_packycode_usage_credentials(state, access_token: String,
  user_id: String, base_url: Option<String>) -> Result<(), AppError>`
  —— trim 后:令牌进钥匙串、user_id/base_url 进 settings;空令牌/空 user_id 拒绝;
  写入前可以先 `fetch_self` 验证一次(成功才算配好),失败报
  `authentication_failed` 且**不落盘**。**日志、错误串、返回体不得含令牌明文**。
- `clear_packycode_usage_credentials(state) -> Result<(), AppError>`
  —— 删钥匙串条目 + 清三个 settings 键。
- `get_packycode_account_usage(state) -> Result<PackyCodeAccountUsageView, AppError>`
  —— 只读,不发请求。
- `refresh_packycode_account_usage(state) -> Result<PackyCodeAccountUsageView, AppError>`
  —— 立即抓一次(手动刷新入口;60 秒内重复调用复用上次结果,防连点)。

### 接线(小改)

- `src-tauri/src/app_state.rs`(**T17 前叫 `store.rs`;现在的 `store` 是数据库,别搞混**):
  `AppState` 加调度器 handle 字段 + `start_/take_` 方法 —— 照
  `official_pricing_scheduler` 的三处写法(`app_state.rs:36`、`:121`、`:185`)。
- `src-tauri/src/lib.rs`:`mod` 声明、`invoke_handler` 注册四个命令、启动/退出路径
  挂 start/stop(照 official_pricing 的挂法)。

## 5. 明确不做

- ❌ 不碰前端 `src/`、不写 React 组件
- ❌ 不做「今日花费」上条(`/api/log/self/stat` 可选,默认不做;上条走本机记账)
- ❌ 不把模型 key(`sk-...`)当管理凭据用——那是两套凭据,互相不通用
- ❌ 不加新 crate;HTTP 用 `crate::http_client::get()` 的共享 reqwest client
- ❌ 不改 D1 的文件(D1 改的是 `balance.rs` 里 OpenRouter 那一支)
- ❌ **不建表、不动 `SCHEMA_VERSION`、不写迁移**(§3 已锁定方案 A)
- ❌ 不做 T21 的搬运 —— 若 `providers/` 还不存在,就放 `services/`,不要自己建目录

## 6. 测试(必须)

1. `/api/user/self` 正常解析:quota/used_quota 换算正确(用非默认 `quota_per_unit`
   断言换算公式,证明没写死 500000);
2. `quota_per_unit` 缺失/为 0 → 回退 500000;`/api/status` 404 → 同样回退;
3. `success: false` → `upstream_rejected`,**不写快照**;非 JSON(Cloudflare HTML)
   → `invalid_response`,不 panic;
4. 401/403 → `authentication_failed`,且 set 命令不落盘(断言钥匙串/settings 无变化);
5. 网络错误 → `connection_failed`,旧快照保留;
6. 调度:成功后 15 分钟内不重复请求;失败退避;cancel 即时退出;
7. 未配置凭据:调度空转、`get_...` 返回 `has_credentials: false` 其余 `None`;
8. set/clear 命令:set 后钥匙串 + settings 有值;clear 后全空;错误路径不泄令牌明文
   (扫描返回体/日志字符串)。

## 7. 交付格式

照 `docs/tasks/README.md` §3:文件清单、六项检查的 `test result:` 原文、做了任务书
之外的事、想改但忍住没改的事。

另外必须报告:

- **真实端点探测结论**(如果你用真实凭据验过 `/api/user/self` 的路径前缀)
- 模块最终落在 `providers/packycode/` 还是 `services/`(取决于 T21)
