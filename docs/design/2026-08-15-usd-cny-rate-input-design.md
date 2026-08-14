# 填价格表支持 USD / CNY 切换:汇率只读,自动换算成 USD 落库

**日期:** 2026-08-15
**状态:** 方案已确认,未实现。API 源待定(不影响其余部分开工)。
**范围:** 只影响「填价格表」这一个输入环节。数据库、计价、显示、历史数据**一律不动**。

---

## 1. 为什么做

用户的中转商(relay)以人民币报价。现在填价格表(`ProviderModelPricingSection`)
只能填美元,用户得自己把 CNY 报价换算成 USD 再填,容易算错、且换一次填一次。

目标:填的时候可以切到人民币,提交前**按汇率自动换算成 USD 存库**。

**已确认的边界**(用户拍板,不再讨论):

- 只影响填价格表的输入;**库里永远存 USD**。
- 汇率**不需要手动可改**:后端抓取 + 只读展示,前端不提供修改入口。
- 汇率 **30 天刷新一次**;抓取失败用上次存的值兜底。

## 2. 现状:整个系统是纯 USD,没有币种概念

| 环节 | 现状 |
| --- | --- |
| 填价格表(`src/components/settings/ProviderModelPricingSection.tsx`) | 四个输入框,label 写死 "USD / 1M tokens",填纯数字 |
| 存库(`provider_model_pricing` 表,`database/dao/provider_model_pricing.rs:27` `canonicalize_price`) | 四列 `TEXT`,裸数字,无币种字段 |
| 计价(`usage/metering/calculator.rs:109`) | 直接把填的数字当美元乘 token 数,产出 `total_cost_usd` |
| 下游 | `usage_events` / `usage_daily_rollups` 全为 `*_usd` 列;官方价目表(`services/official_pricing.rs`,源 `https://models.dev/api.json`)为 USD;上游 API 报的 cost 也按 USD 处理 |
| 显示(`src/components/settings/providerSpendFormat.ts`) | `formatUsd` 硬编码 `currency: "USD"` |

**推论**:「切换单位」如果是想全局改显示币种,那是给系统加一层货币维度的大改动
(要动数据库语义、所有显示函数、历史数据按哪天汇率换算)。**本设计明确不做这个**。

## 3. 方案

### 3.1 后端:新建 `src-tauri/src/services/currency_rate.rs`(独立小模块)

完全照抄 `services/official_pricing.rs` 的成熟模式(启动查过期 → 定时刷新 →
失败退避),只改数据源和语义:

- **抓取**:GET 汇率 API(源待定,见 §5),解析出 USD→CNY 汇率,存 settings 表。
- **存储**:settings 表两个键(复用 `database/dao/settings.rs:17/49` 的
  `get_setting` / `set_setting`,**无数据库迁移**):
  - `usd_cny_rate` — 汇率值,如 `"7.20"`,存字符串不存浮点,与仓库「金额用十进制字符串」
    的约定一致
  - `usd_cny_rate_updated_at` — 上次成功抓取时间戳
- **调度**:`STARTUP_STALE_AFTER = 30 天`;失败按
  `FAILURE_RETRY_DELAYS`(30s→1800s 五级退避)重试,**失败时保留上次存的值不动**,
  与官方价目刷新的失败语义一致(`official_pricing.rs:287` `start_scheduler`)。
- **对外只读接口**:
  - `current_rate(db) -> Option<RateInfo>` — 返回 `{ rate, updated_at }`,供前端展示
  - 调度器 handle 挂到 `AppState`(`store.rs` 已有 `official_pricing_scheduler`
    同款字段,照抄一份 `currency_rate_scheduler`)

### 3.2 前端:填价格表加 USD/CNY 切换

`src/components/settings/ProviderModelPricingSection.tsx`:

- 表单头部加一个 USD/CNY 切换(segmented control 或 select)。
- 切到 CNY 时:
  - 四个输入框的 label 显示 "CNY / 1M tokens"(i18n key,四语言,与仓库多语言约定一致);
  - 已存价格行摘要仍按 USD 显示(库里就是 USD,不做二次换算,避免"按哪天汇率"歧义);
  - 提交前把四个值**除以汇率**转成 USD,再走现有 `updateProviderModelPricing` 提交路径
    (后端一行不用改)。
- 切换控件旁边显示一行只读信息:
  `按 1 USD = 7.20 CNY 换算 · 最后更新于 8月1日`,来自 §3.1 的只读接口。
  无汇率时(从未抓取成功)提示「汇率暂不可用,请先以 USD 填写」,不允许提交 CNY。
- 换算发生在提交那一瞬间,用**当时拿到的汇率**,提交后即固化(USD),不保留"这个价格
  当时是按哪个汇率填的"——因为价格存的是 USD,汇率只是输入时的一次性换算工具。

### 3.3 明确不做的事

- ❌ 不做全局币种切换(仪表盘 / 托盘 / 历史花费仍全 USD)
- ❌ 不改数据库 schema、不加迁移
- ❌ 不改计价 `calculator.rs`、不改任何 `*_usd` 列语义
- ❌ 汇率不做手动编辑入口(用户已拍板)
- ❌ 已存价格不按汇率回显成 CNY

## 4. 这是什么样的模块

**独立小服务模块**,归模块化蓝图(§9.2)里的 **config** 类——和官方价目刷新
(`services/official_pricing.rs`)平行,不是包含关系:

- 两者唯一的相似点是「都要定时从网上抓点东西」,所以汇率抓取**复用官方价格那套
  调度写法**(模板),但代码完全独立、互不耦合;
- 它不依赖任何 provider,不碰 provider 模块;
- 它不碰数据库 schema(只用 settings 键值),所以**没有迁移、没有 SCHEMA_VERSION 问题**;
- 生命周期照 `AppState` 现有 scheduler 的模式接入(`store.rs:184` 同款),启动/退出
  处理与官方价目刷新一致。

## 5. 未决事项:API 源

- 候选:
  - 腾讯行情接口 `https://qt.gtimg.cn/q=usdcny` — 国内直连快、稳定,返回格式简单
  - `open.er-api.com` — 国际通用、无需 key,国内直连可能慢
  - 双源回退 — 先试腾讯,失败试 open.er-api.com,再失败用上次存的值
- 用户尚未拍板(2026-08-15)。**只有「抓取那一小段」依赖它**,§3 其余部分
  (存储、调度、前端换算、只读展示)都不依赖具体源,可先开工。

## 6. 改动清单

| 改动 | 文件 | 规模 |
| --- | --- | --- |
| 汇率抓取/存储/调度/只读接口 | 新建 `src-tauri/src/services/currency_rate.rs` | ~150 行(照抄 official_pricing 模式) |
| 调度器 handle + 启动/停止 | `src-tauri/src/store.rs`、`src-tauri/src/lib.rs` | 各几行 |
| 只读命令暴露给前端 | `src-tauri/src/commands/usage.rs`(或新建 commands 文件) | 一个 `#[tauri::command]` |
| 表单切换 + 换算 + 汇率展示 | `src/components/settings/ProviderModelPricingSection.tsx` | 一个切换控件 + 提交换算 |
| 文案 | 四语言 locale(参照 `usageDashboard.customPricing*` 现有 key 的加法) | 每条 +2~3 个 key |
| 测试 | 后端 `currency_rate.rs` 内嵌单测(换算/过期/失败兜底);前端 `ProviderModelPricingSection.test.tsx` 已有,补 CNY 提交换算用例 | — |

**完成标准**:六项检查全绿(Rust fmt/clippy/test、tsc、prettier、vitest);
填 CNY 提交后库里存的四个值是 `CNY ÷ 汇率` 的 USD;失败时显示上次汇率;无汇率时
禁止 CNY 提交。
