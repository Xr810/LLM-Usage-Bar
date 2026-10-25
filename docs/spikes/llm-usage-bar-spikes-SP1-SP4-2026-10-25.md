# LLM Usage Bar：Spike SP-1 … SP-4

**日期**：2026-10-25　**分支**：`cursor/llm-usage-bar-spikes-sp1-sp4-d246`　**状态**：验证记录，不合并、不发布

**输入**：技术方案 v2（Q2、Q5、ADR-001 … ADR-017、§7.0 SP-1 … SP-4）与 PRD A+P3。固定约束：订阅（OAuth）流量只告警、放行；硬停和 Kill 只作用于 API Key 流量；代理崩溃或 License 失效默认 fail-closed，用户可改成放行。

**本文件没有产品改动。** 可执行验证在 `spikes/llm-usage-bar/sp1_sp4_harness.py`，原始输出在 `spikes/llm-usage-bar/sp1_sp4_harness_output.json`。没有真实 API Key，没有登录任何 Provider。

证据分三档，后文每条结论都带一档：

| 档 | 含义 |
|---|---|
| **实测** | 本机跑了一次性验证程序，对着 mock 或纯函数 |
| **文档/源码推断** | 读了官方文档、Codex 公开源码或本仓库源码，没有在本机对着那个二进制或那个账号复现 |
| **未实测** | 需要真实凭据、真实 CLI 登录或真实账单，本次明确不做 |

---

## 0. 结论先行

| Spike | 结论 | 一句话 |
|---|---|---|
| SP-1 | **部分可行** | 入站请求上，API Key 和订阅 OAuth 可以分开，但只看「有没有 `Authorization: Bearer`」会把网关 Key 当成订阅。现有 `route/` 在转发前丢掉认证头，分类必须发生在丢头之前。真实 Claude Code 流量未抓。 |
| SP-2 | **部分可行** | Codex 的 API Key 路径可以进本地 `POST /v1/responses`。ChatGPT 订阅默认走 `wss://chatgpt.com/backend-api/codex/responses`，不进本地代理。要让订阅请求碰到代理，必须另建非保留 provider id，并关掉 WebSocket；代理还得原样转发到 Codex 后端。这一跳未用真实登录验证。 |
| SP-3 | **部分可行** | mock 上，客户端中途断开后，终态 `usage` 不到，已生成的输出 token 已经发生。现有解析器只认 `response.completed`，断开时记成 0。真实账单差额未测。 |
| SP-4 | **部分可行** | 火山 Agent Plan 的 `GetAFPUsage` 有官方文档。Kimi、智谱额度端点、仓库里的 MiniMax 路径、`GetCodingPlanUsage`、Claude / Codex / Gemini 的额度接口，本次都对不上公开的官方字段文档。端点本身未用真凭据调用。 |

任务书里「两种认证各抓 ≥20 个真实请求」和「断开后 Provider 实际计费」这两条验收，本次都标 **未实测**，不记为通过。

---

## 1. 怎么测的

```bash
python3 spikes/llm-usage-bar/sp1_sp4_harness.py
```

程序做三件事：

1. 用公开文档里的头名和 token 前缀造请求，每类 24 条，在进程内分类，再经 `127.0.0.1` 上的 mock 代理走一遍 HTTP。
2. 按 Codex 公开源码 `ModelProviderInfo::to_api_provider` 和 `merge_configured_model_providers` 的规则，计算给定 `config.toml` 形状会打到哪个 URL、走 HTTP 还是 WebSocket。没有运行 Codex 二进制。
3. mock 上游按 Anthropic SSE 和 OpenAI Responses SSE 吐流。客户端读到 5 个内容增量就关连接，对比「上游已经生成的 token」和「终态 usage 解析器看见的 token」。

合成 token 只有形状，例如 `sk-ant-api03-synthetic-01`、`sk-ant-oat01-synthetic-01`、`eyJ…` 三段 JWT。输出文件里没有秘密。

---

## 2. SP-1　认证头能否分开 API Key 和订阅

**结论：部分可行。**

能分开的前提是：在入站 `HeaderMap` 上同时看头名、token 前缀和 `anthropic-beta`，并且把「认不出」单列成第三态。只看 Bearer 还是 `x-api-key`，不可行。

### 2.1 实测：清晰形状，每类 24 条，分类器 100%

分类器规则（实现见 harness 的 `classify_claude` / `classify_openai_codex` / `classify_gemini` / `classify_coding_plan_key`）：

| 流量 | 判为 `api_key`（可硬停） | 判为 `subscription_oauth`（只告警） | 判为 `unknown`（不放行成订阅，也不当成已确认的 Key） |
|---|---|---|---|
| Claude Code | 有 `x-api-key`；或 `Authorization: Bearer` 的值以 `sk-ant-api` 开头；或其他非 `sk-ant-oat` 的 Bearer（网关凭据） | Bearer 以 `sk-ant-oat` 开头 | 没有认证头；或 beta 里有 `oauth-2025-04-20` 但前缀不是已知形状 |
| Codex / OpenAI | `sk-` / `sk-proj-` / `sk-svcacct-` / `sk-admin-`，放在 Bearer 或 `x-api-key` | JWT（`eyJ` 开头、三段）并且带 `ChatGPT-Account-Id`，或路径含 `/backend-api/codex` | 只有 JWT、没有账号头、路径也不是 Codex 后端 |
| Gemini CLI | `x-goog-api-key`，或 Bearer 以 `AIza` 开头 | Bearer 以 `ya29.` 开头 | 其他 Bearer、空认证 |
| Kimi / 智谱 / MiniMax / 火山 Coding Plan | 有 Bearer，或智谱那种不带 Bearer 前缀的 `Authorization` | 不把这些 Key 当成 OAuth | 没有认证材料 |

`python3 spikes/llm-usage-bar/sp1_sp4_harness.py` 的 `suites`：

| suite | 正确 / 总数 |
|---|---|
| `claude_api_key` | 24/24 |
| `claude_oauth` | 24/24 |
| `codex_api_key` | 24/24 |
| `codex_oauth` | 24/24 |
| `gemini_api_key` | 24/24 |
| `gemini_oauth` | 24/24 |
| `coding_plan_kimi` / `zhipu` / `minimax` / `volcengine` | 各 24/24 |

同一批 Claude / Codex 请求再打到 mock 代理 `POST`（96 次）：入站分类 `api_key` 48、`subscription_oauth` 48。这是 **实测**，对象是 mock，不是 Claude Code 或 Codex 进程。

对抗样本（空头、两种头同时出现、OAuth beta 配上陌生前缀、JWT 落在 `/v1/responses` 而不是 Codex 后端）也按上表预期命中，见输出里的 `adversarial`。两种头同时出现时判 `api_key`：官方写明网关凭据会替换订阅登录，这时放行成「订阅」会造成漏杀。

### 2.2 实测：ADR-010 的草图会误判

技术方案 Q6 / ADR-010 写的是：Anthropic OAuth 看 `Authorization: Bearer`，API Key 看 `x-api-key`。用这个草图跑 24 条 Claude API Key 夹具，**8 条被标成订阅**。这 8 条是 `ANTHROPIC_AUTH_TOKEN` 对应的 Bearer：值是 `sk-ant-api03-…` 或网关 token，并没有 `x-api-key`。

这是漏杀方向的错误。硬停只打 API Key，被标成订阅的 Key 会在 Kill 和日顶之下继续花钱。

官方依据（**文档/源码推断**）：

- [Connect Claude Code to an LLM gateway](https://code.claude.com/docs/en/llm-gateway-connect)：`ANTHROPIC_AUTH_TOKEN` 进 `Authorization: Bearer`，`ANTHROPIC_API_KEY` 进 `x-api-key`，`apiKeyHelper` 两个头都写。凭据放错头会 401。
- [Other LLM gateways](https://code.claude.com/docs/en/llm-gateway) 与 [Run Claude Code through a gateway](https://code.claude.com/docs/en/gateways)：只要设置了网关凭据变量，请求就不再带 claude.ai 订阅登录，用量按该凭据计费。只设置 `ANTHROPIC_BASE_URL`、不设置凭据变量时，订阅登录仍是有效凭据，流量仍经过网关。
- [Gateway compatibility](https://code.claude.com/docs/en/llm-gateway-protocol)：`anthropic-beta` 在 claude.ai 登录时带有 OAuth capability，删掉它会 401。头要原样转发。同时给出 `x-claude-code-session-id`，会话熔断可以读这个头，不必解析正文。

`sk-ant-oat01-` 与 `sk-ant-api03-` 的前缀来自第三方说明（LiteLLM 的 Claude 订阅教程、ARC 的 Claude Code 笔记），**不是**上面那几页官方文档里的原文。前缀稳定性按技术方案自己的标注，仍是未验证。分类器把「beta 像 OAuth、前缀却不认识」收成 `unknown`，就是为了不把一条没见过的 Bearer 自动豁免。

OpenAI 侧：API Key 的 `sk-` / `sk-proj-` 是平台公开形状。ChatGPT 访问令牌是 JWT，本仓库 `providers/codex/subscription.rs` 把它放进 `Authorization: Bearer`，并加 `ChatGPT-Account-Id`。JWT 单独出现时本 spike 判 `unknown`，避免一条来历不明的 Bearer 被豁免。

Coding Plan 的 Key 不是 OAuth。它们走套餐额度，但认证材料是 Key。按 ADR-014，硬停适用于它们；不要因为「套餐」两个字就只告警。

### 2.3 实测：现有 `route/` 转发前把分类信号丢掉

`src-tauri/src/route/server.rs` 的 `build_upstream_headers` 只保留 `content-type` 和 `accept`。`authorization`、`cookie`、`x-api-key`、`openai-organization` 都丢掉，再由 `UpstreamAuth` 按**路由表里选中的那一家**注入凭据（`route/auth.rs`：`RouterAuthKind::ChatgptOauth` 或 `BearerKey`）。

harness 对 96 条本来可分的请求做了同样的白名单：**96/96 在丢头之后变成 `unknown`**。mock 代理上也是 `forwarded_kept_authorization = 0`。

所以：

- 分类做得到，但必须在 `handle_responses` 读入站头的那一刻做，不能在转发出去的头上做。
- 现有路由器是「换上游凭据」的故障转移，不是「按客户端计费身份透传」的计费代理。订阅透传（ADR-014）需要一条新路径：OAuth 请求保留客户端的 `Authorization`、`anthropic-beta`、`ChatGPT-Account-Id`，不要换成表里的 Key。
- 换凭据本身会把订阅请求变成上游那把 Key 的账单。那和「订阅流量不阻断、也不改记到 API Key」是冲突的。

### 2.4 未实测

- 没有抓 Claude Code、Codex、Gemini CLI 的真实进程。任务书「各 ≥20 个真实请求」不成立。
- 没有验证 `sk-ant-oat` 前缀在当前 Claude Code 版本里是否仍是唯一订阅形状。
- 没有验证 Claude 的证书锁定。官方把 `ANTHROPIC_BASE_URL` 写成网关接入方式，LiteLLM 文档用 `http://localhost:4000` 配订阅登录，这和「锁死 api.anthropic.com 证书、本地 HTTP 代理根本连不上」不相容，但是 **未实测**。

### 2.5 对 Q2 / Q5 的影响

- **Q2，Claude Code + API Key**：维持可在请求前拦截。分类不要用「Bearer = 订阅」。
- **Q2，Claude Code + 订阅**：官方文档支持「只改 `ANTHROPIC_BASE_URL`，订阅凭据仍生效，请求经过网关」。拦截点存在，策略仍是只告警。这比「能不能进代理还不知道」更具体；真实客户端仍 **未实测**。
- **Q6 / ADR-010**：规则表要升级成「头名 + 前缀 + beta」，并保留 `unknown`。前缀表按技术方案做成可更新的，不要写死在匹配代码里当永久事实。
- **Q5**：SP-1 不改变额度接口能不能读到。它只确定代理上看得到哪一种计费身份。

### 2.6 风险

- 把网关 Bearer 当成订阅：Kill 和日顶漏杀。
- 把 `sk-ant-oat` 当成 API Key 去硬停：违反 ADR-014。
- `unknown` 如果默认放行成订阅，同样漏杀。和 fail-closed 一致的做法是：`unknown` 不享受订阅豁免。硬停是否直接 429，还是先挂起问用户（BE-9 的身份询问），留给实现；本 spike 只要求它不要被标成已确认的订阅。
- 条款（R2）：Anthropic 自己的网关文档允许「只改 base URL、仍用订阅登录」。这不是法律意见。透传时必须原样转发 `anthropic-beta`，否则官方写明会 401，计量也会断。

---

## 3. SP-2　Codex 的 API Key 和 ChatGPT 订阅能不能进代理

**结论：部分可行。** API Key 路径可以进现在的本地 `POST /v1/responses`。ChatGPT 订阅的默认路径不进。把订阅请求赶到本地 HTTP 代理，源码上有一条配置，真实登录未跑。

### 3.1 文档/源码推断：Codex 怎么选 URL

公开源码（2026-10-25 读到的 `main`）：

- `codex-rs/model-provider-info/src/lib.rs`
  - `CHATGPT_CODEX_BASE_URL = "https://chatgpt.com/backend-api/codex"`
  - `to_api_provider`：认证模式是 `Chatgpt` / `ChatgptAuthTokens` / `Headers` / `AgentIdentity` / `PersonalAccessToken` 时，默认 base 是上面的 Codex 后端；否则默认 `https://api.openai.com/v1`。**provider 自己的 `base_url` 若是 `Some`，覆盖这个默认值。**
  - `merge_configured_model_providers`：保留 id `openai`、`ollama`、`lmstudio` 不能被 `config.toml` 里的同名表替换（Bedrock 例外）。OpenAI 员工在 [openai/codex#11698](https://github.com/openai/codex/issues/11698)（2026-02-13）写过：不能覆盖内置 provider，要换一个 id，或只用 `OPENAI_BASE_URL`。
  - 内置 `openai` provider：`requires_openai_auth = true`，`supports_websockets = true`。自定义 provider 的 `supports_websockets` 默认 false。
  - `AuthMode::uses_codex_backend`（`codex-rs/protocol/src/auth.rs`）：ChatGPT、外部注入的 ChatGPT token、headers、Agent Identity、PAT 走 Codex 后端；API Key 不走。
- WebSocket 是订阅流量的默认传输。问题单里的实际 URL 是 `wss://chatgpt.com/backend-api/codex/responses`（[openai/codex#13041](https://github.com/openai/codex/issues/13041)、[#46489](https://github.com/openai/codex/issues/46489)、[#38638](https://github.com/openai/codex/issues/38638)）。关掉 WebSocket 的写法是**另一个** provider id，而不是改内置 `openai`：

```toml
model_provider = "openai_https"

[model_providers.openai_https]
name = "OpenAI"
wire_api = "responses"
requires_openai_auth = true
supports_websockets = false
base_url = "https://chatgpt.com/backend-api/codex"
```

把 `base_url` 换成 `http://127.0.0.1:<port>/v1`，按 `to_api_provider` 请求会变成 `POST http://127.0.0.1:<port>/v1/responses`，认证仍是 ChatGPT 登录。这是源码推断，不是本机 Codex 的抓包。

- 证书锁定：读到的源码没有 pin。企业代理走 `CODEX_CA_CERTIFICATE` / `SSL_CERT_FILE`（[openai/codex#14239](https://github.com/openai/codex/pull/14239)）。WebSocket 曾经不吃自定义 CA（[#46489](https://github.com/openai/codex/issues/46489)），HTTP 会吃。本地代理用 `http://127.0.0.1` 时不经过 TLS，pin 和自定义 CA 都碰不到。`supports_websockets = true` 再指到 localhost，则会去连 `ws://`，现有只注册了 `POST /v1/responses` 的路由器接不住。
- [openai/codex#34608](https://github.com/openai/codex/issues/34608)（2026-07-21，作者自述 Codex CLI 0.144.3）：ChatGPT 登录加自定义 `base_url`，透明代理把客户端凭据原样转到 `api.openai.com/v1/responses`，返回 `401`，缺 `api.responses.write`。这说明两件事：自定义 `base_url` 在那次试验里确实接到了请求；**同一枚 ChatGPT token 不能当成 API Key 去打 `api.openai.com`**。要维持订阅计费，上游只能是 `https://chatgpt.com/backend-api/codex`。该 issue 是别人的报告，本仓库没有复现，标 **文档/源码推断**，不是本机实测。

环境变量 `OPENAI_BASE_URL` 盖不过已经写了 `model_providers.<id>.base_url` 的配置（[openai/codex#44453](https://github.com/openai/codex/discussions/44453)，作者称 2026-08-31 看过请求行）。本仓库设计文档 `docs/design/2026-08-14-local-routing-design.md` §2.1 的抓包结论是另一条配置：`requires_openai_auth = true` 再加 `experimental_bearer_token` 时，线上的 `Authorization` 是中转 Key，不是 ChatGPT access token。那条路径是 API Key 计费，不是订阅透传。

### 3.2 实测：把上述规则做成纯函数，再对本地端口发请求

harness 的 `codex_resolution_matrix`（规则复刻，不是 Codex 进程）：

| 场景 | 选中的传输 | 解析出的 HTTP URL | 打到本机代理？ |
|---|---|---|---|
| 不改配置，ChatGPT 登录，内置 `openai` | WebSocket | `https://chatgpt.com/backend-api/codex/responses` | 否 |
| `config.toml` 里写 `[model_providers.openai] base_url = 127.0.0.1` | WebSocket，覆盖被丢掉 | 同上，官方 Codex 后端 | 否 |
| 本仓库 `pointer.rs` 的形状：`model_provider = "llm_usage_bar_router"`，只有 `base_url` 和 `wire_api` | HTTP | `http://127.0.0.1:43123/v1/responses` | URL 是本机 |
| 自定义 id，`requires_openai_auth = true`，`supports_websockets = false`，`base_url` 指本机 | HTTP | 本机 `/v1/responses` | URL 是本机 |
| 自定义 id，`env_key = "OPENAI_API_KEY"`，`requires_openai_auth = false` | HTTP | 本机 `/v1/responses` | URL 是本机 |

`pointer.rs` 写入的段是：

```163:166:src-tauri/src/route/pointer.rs
    section["name"] = value("LLM Usage Bar Router");
    section["base_url"] = value(format!("http://127.0.0.1:{port}/v1"));
    section["wire_api"] = value("responses");
```

没有 `requires_openai_auth`，没有 `supports_websockets`，没有 `experimental_bearer_token`。按 Codex 源码，`requires_openai_auth` 默认 false 时登录界面被跳过，Key 来自 `env_key`。这段配置的 URL 会指向本机，但 **ChatGPT token 不会因此自动附上**。现有路由器再按自己的 `auth_kind` 换上游凭据。所以今天的「Connect Codex」是 API Key / 中转路径，不是订阅透传。

另一次 **实测**：向只实现了 `POST` 的 mock 发 WebSocket Upgrade，`GET /v1/responses` 返回 **404**，`RECORDER` 里没有一条入站分类。HTTP-only 的 `route/` 看不到订阅默认的那条 WebSocket。

### 3.3 哪条路径可以拦截

| 路径 | 请求前能否碰到本地代理 | 硬停？ | 证据档 |
|---|---|---|---|
| Codex + API Key / 兼容上游，`base_url = http://127.0.0.1:<port>/v1`，`wire_api = "responses"`，`requires_openai_auth = false` | 能，`POST /v1/responses` | 能，这是 API Key | URL 规则 **实测**（复刻）；真实 Codex **未实测**。本仓库指针已经按这个形状写 |
| 内置 `openai` + ChatGPT 登录，不改 provider id | 不能。WebSocket 去 `chatgpt.com` | 不允许硬停 | **文档/源码推断** |
| 非保留 id + `requires_openai_auth = true` + `supports_websockets = false` + 本机 `base_url` | URL 会到本机 HTTP | 不允许硬停。代理必须把原头转到 `https://chatgpt.com/backend-api/codex/responses` | URL **实测**（复刻）；chatgpt.com 是否接受这次转发 **未实测** |
| 把 ChatGPT token 转到 `api.openai.com` | 请求可以进代理，上游 401，订阅额度不生效 | 不适用 | **文档/源码推断**（#34608） |
| 现有 `route/` 换凭据 | 客户端的请求进了本机，但订阅 token 被丢掉 | 若换上 API Key，账单性质变了，不能当成「订阅只告警」 | **实测**（白名单）+ 源码 |

复现步骤（仍不登录）：

```bash
python3 spikes/llm-usage-bar/sp1_sp4_harness.py
# 看 codex_resolution_matrix 与 live_mock_proxy.websocket_upgrade_status
```

真实 Codex 的复现（本次没做，写成步骤只为以后）：准备一个非保留 provider，`supports_websockets = false`，`requires_openai_auth = true`，`base_url` 指 loopback；用已有 ChatGPT 登录起 CLI；在代理上只记录头名和 token 形状（JWT 对比 `sk-`），不落盘明文；上游只允许 `chatgpt.com` 的 Codex 后端。没有这些条件就不要宣称订阅流量已经被拦截。

### 3.4 未实测

- 本机没有跑 `codex` 二进制，没有 `codex login`。
- 没有验证当前发行版是否仍拒绝覆盖内置 `openai`。源码和 2026-02 的员工回复一致，但是发行版可能滞后或超前。
- 没有验证 `wss://` 在 `supports_websockets = false` 时是否还会被模型元数据的 `prefer_websockets` 重新选中。#13041 的评论说只关 feature flag 不够，要换 HTTP-only provider。这条评论之后又有 #38638 说 opt-out 被拿掉、要失败 5 次才退回 HTTPS。**传输选择以用户机器上的 Codex 版本为准，本次不能盖章。**
- 证书锁定未对二进制做实验。源码层面没看到 pin。

### 3.5 对 Q2 / Q5 的影响

Q2 建议改成下面这张，MVP 列先不动（订阅仍不是硬停对象）：

| 客户端 | 请求前能否碰到代理 | MVP 硬停 |
|---|---|---|
| Codex + API Key / 兼容端点 | 能。非保留 provider，`base_url` 指 `http://127.0.0.1:<port>/v1`，`wire_api = "responses"` | 是 |
| Codex + ChatGPT 订阅 | 默认不能。内置 `openai` 走 WebSocket 到 `chatgpt.com/backend-api/codex`。只有非保留 id、`requires_openai_auth = true`、`supports_websockets = false`、本机 `base_url` 这条配置，URL 才会到本地 HTTP。转发目标必须是 Codex 后端，不能是 `api.openai.com`。真实登录 **未实测** | 否。即使以后透传成功，也只告警 |
| Gemini API Key | `GOOGLE_GEMINI_BASE_URL` 官方写明只在 `gemini-api-key` 认证下生效，允许 `localhost` / `127.0.0.1` 用 HTTP（[Gemini CLI configuration](https://geminicli.com/docs/reference/configuration/)） | 不在原 MVP 清单里。若要加，这是 API Key，可以硬停 |
| Gemini 订阅 / Code Assist | `GOOGLE_GEMINI_BASE_URL` **不**覆盖这条认证。Code Assist 的主机在 CLI 源码里是 `CODE_ASSIST_ENDPOINT`（默认云端 Code Assist）。订阅流量不能靠 Gemini 的 API base URL 进代理 | 否 |

Q5 的 Codex 额度行不因为 SP-2 变成「官方接口」。`wham/usage` 仍然是本仓库在用的非公开路径（见 SP-4）。订阅流量若没进代理，伪美元只能继续靠这个额度接口和 JSONL，不能靠代理计量。

### 3.6 风险

- 把 #34608 那种 401 当成「订阅已经在计费」。token 到了 `api.openai.com` 就是失败，不是订阅账单。
- 现有路由器给订阅请求换上 API Key：用户以为还在吃 ChatGPT 额度，实际在烧 Key。这比「拦不到」更差。
- WebSocket 默认传输让「指针写了 base_url」成为假安全感。要在文档里写明必须 `supports_websockets = false`，并在启动时读回配置，发现仍是内置 `openai` 就标「仅事后」。
- 转发 ChatGPT token 到本机之外的中转，和本仓库 2026-08-14 设计里「官方 token 不外泄」的目标相反。透传只应发生在 loopback，上游固定 `chatgpt.com`。
- fail-closed：`pointer.rs` 退出时不把指针改回直连（设计文档决定 35；`disconnect_codex` 是显式动作）。代理进程没了，Codex 仍打 `127.0.0.1`，请求失败。这和 ADR-013 的默认 closed 一致，但是 **未实测** 杀掉 App 后的 CLI 行为。open 模式要另做「把 base_url 改回官方」的恢复，现有指针没有这条。SIGKILL 之后只能等下次启动，ADR-013 已经写了这个缺口。

---

## 4. SP-3　流式中途断开，usage 和「已生成」差多少

**结论：部分可行。** mock 上差额测到了。真实 Provider 账单 **未实测**，官方文档没有给出一个可以抄进账本的数字。

### 4.1 实测：mock 上游

上游计划：输入 1000 token，输出 20 个内容增量，终态 usage 写 `output_tokens = 20`。客户端在看到 5 个内容增量后关连接。完整读完的对照组也跑了。

| 场景 | 客户端看见的内容增量 | 终态 usage | 现有 `usage_from_completed_event` | 上游计数器在连接断开时写到的输出 token |
|---|---|---|---|---|
| Anthropic 形状，中途断开 | 5 | `message_delta.usage.output_tokens` 没有到达。`message_start` 里的 `input_tokens = 1000` 已经到达 | `None`（这个解析器本来就不认 Anthropic 事件） | 7 |
| Anthropic 形状，读完 | 20 | `message_delta` 输出 20 | `None` | 20，终态已发出 |
| OpenAI Responses 形状，中途断开 | 5 | `response.completed` 没有到达 | `None` | 7 |
| OpenAI Responses 形状，读完 | 20 | `response.completed` 的 usage 是 `(1000, 20)` | `(1000, 20)` | 20，终态已发出 |

差额（mock 的「已生成」减去客户端解析到的终态输出；解析不到按 0）：

- 断开时，终态解析器：**7 − 0 = 7**。
- 断开时，若按已经看见的内容增量记账：**7 − 5 = 2**。多出来的 2 是上游已经 `write` 成功、客户端不再读、内核缓冲被丢掉的增量。这是这个 harness 的写缓冲行为，不是 Provider 公布的常数。
- 读完时，终态 usage 和计划的 20 一致，差额 0。

对应代码：`src-tauri/src/route/server.rs` 的 `usage_from_completed_event` 只在 `type == "response.completed"` 且 `response.usage` 里两个整数字段都在时回填。断开的 OpenAI 流在这条解析器下是空账。Anthropic 的 `message_start` / `message_delta` 今天根本不进 `route/`（只注册了 `POST /v1/responses`）。

### 4.2 文档/源码推断：真实账单

- OpenAI 社区，员工账号 OpenAI_Support，2026-04-06，[Cancelling Stream Does Not Show Usage](https://community.openai.com/t/cancelling-stream-does-not-show-usage/1377768)：流被取消时，最后的 usage 不会出现在响应里；**后端仍然记账**。帖子没有给「生成了 N、账单是 M」的差。
- Anthropic 的 [Streaming messages](https://platform.claude.com/docs/en/build-with-claude/streaming) 写了事件顺序：`message_start` 带初始 usage，`message_delta` 的 usage 是累计输出，然后 `message_stop`。没有写「客户端提前关连接时发票上的输出 token 等于什么」。第三方文章说按已生成的输出计费，**不当作官方数字**。
- 因此：真实差额的验收标准本次 **未实测**。不能写成「Anthropic 断开后多收 2 个 token」这种句子。

### 4.3 对 Q2 / Q5 的影响

- 不改变 Q2 的「谁能拦截」。
- 影响 Q3 / ADR-007 的结算，间接影响 Q5 的伪美元：伪美元若用代理上的 usage，断开的请求会少记输出。订阅窗百分比来自额度接口，不来自这次流，所以 **Q5 的「能不能读到窗」不受影响**；「代理计量和窗百分比是否对得上」会受影响。
- 和 fail-closed 的关系：客户端断开时如果把预留全部 `release`，账本会小于 Provider 已经生成的量，后面的请求可能被放行（漏记，方向和熔断偏早相反）。断开且没有终态 usage 时，应至少按已见增量结算，预留不要清成 0；多出来的在途部分仍由「单请求预留上限 1 美元」罩住。这是建议，不是本次改动的代码。

### 4.4 风险

- 只信 `response.completed`：中途熔断和用户按停止都会少记，日顶偏松。
- 断开后继续把上游读完（为了拿到终态 usage）会让 Provider 把剩下的输出也生成完。OpenAI 员工的说法是取消后后端仍记账，但是「继续读会不会多生成」本次 **未实测**。在没测清楚之前，不要为了账本去把用户已经取消的流读完。
- mock 的 +2 不能写进对外的超额承诺。

---

## 5. SP-4　Coding Plan 与订阅额度端点的文档状态

**结论：部分可行。** 表填完了。没有任何一条用真实凭据打过。

「官方」指本次抓到的厂商公开文档里有这个 Action 或这条 URL。「非官方」指仓库在用、公开文档对不上，或只有社区逆向。「官方目录有名字」不等于字段表已经核对。

| Provider | 仓库现在打的端点 | 文档状态 | 链接 | 和 Q5 的关系 |
|---|---|---|---|---|
| Claude Pro/Max | `GET https://api.anthropic.com/api/oauth/usage`，beta `oauth-2025-04-20`（`usage/claude_oauth.rs`） | **非官方**。本次官方网关文档只要求转发 `anthropic-beta` 里的 OAuth capability，没有把这条 usage URL 写成公开 API | 仓库源码。官方网关页见 SP-1 | 维持「能读百分比、非官方」。Windows 上钥匙串路径本就不可用，SP-4 没有新证据 |
| Codex / ChatGPT | `GET {chatgpt_base}/wham/usage`，缺省 `https://chatgpt.com/backend-api/wham/usage`；中转形态是 `{base}/api/codex/usage`（`providers/codex/subscription.rs`） | **非官方**。Codex 配置文档有 `chatgpt_base_url`，没有公开 wham 额度字段 | 仓库源码；配置样本 [learn.chatgpt.com config sample](https://learn.chatgpt.com/docs/config-file/config-sample.md) | 维持「能读、非官方」。订阅流量默认不进代理时，这仍是窗数据的来源 |
| Kimi For Coding | `GET https://api.kimi.com/coding/v1/usages`，Bearer | **非官方**。`platform.kimi.com` 快速开始写的是 `api.moonshot.cn` 的对话 API，没有这条 usages | 社区逆向笔记：[OpenTokenUsage kimi.md](https://github.com/PowerUserZ/OpenTokenUsage/blob/main/docs/providers/kimi.md) 自标 undocumented | Q5「能读」只表示仓库已接。文档状态改为明确的非官方 |
| 智谱 GLM | `GET https://open.bigmodel.cn/api/monitor/usage/quota/limit`（国际站换 `api.z.ai`），`Authorization` **不加** Bearer | **额度端点非官方**。官方有 Coding Plan 的 Base URL 和 5 小时 / 周积分说明，用量查看写的是控制台和 Claude Code 插件，不是这条 URL | [套餐概览](https://docs.bigmodel.cn/cn/coding-plan/overview)、[用量查询插件](https://docs.bigmodel.cn/cn/coding-plan/extension/usage-query-plugin)、[FAQ](https://docs.bigmodel.cn/cn/coding-plan/faq) | 窗的存在是官方的（5 小时 + 周）。读取路径不是官方文档。另外 FAQ 写明套餐只在指定 Base URL 和指定工具里扣额度，本地代理若改掉客户端看到的 Base URL，套餐是否仍生效是 **未实测** 的条款/产品风险 |
| MiniMax | 仓库：`GET https://api.minimaxi.com/v1/api/openplatform/coding_plan/remains`（国际 `api.minimax.io`），Bearer | **仓库路径本次未在官方 FAQ 出现**。官方 FAQ 给的是另一条：`GET https://www.minimax.cn/v1/token_plan/remains`，Bearer | [MiniMax 开放平台 FAQ](https://platform.minimax.cn/docs/m-plan/faq) | Q5 不要把仓库 URL 写成「官方文档已覆盖」。官方确实提供了用量 API，路径要以 FAQ 那条为准再对一次字段。字段对比 **未实测** |
| 火山 Agent Plan | `POST https://open.volcengineapi.com/?Action=GetAFPUsage&Version=2024-01-01`，AK/SK 签名 | **Action 官方**。文档摘要里的主机是 `ark.cn-beijing.volcengineapi.com`，和仓库的 `open.volcengineapi.com` 不是同一个字符串 | [获取套餐 AFP 额度](https://docs.volcengine.com/docs/ark/get-afp-usage-api?lang=zh)，目录页 [API 文档中心 ark 2024-01-01](https://api.volcengine.com/api-docs/?serviceCode=ark&version=2024-01-01) | Q5「能读」可以保留，但要核对主机。返回里有 5 小时 / 日 / 周 / 月；仓库跳过日窗。完整字段表本次没有逐项对照 |
| 火山 Coding Plan | 仓库：`Action=GetCodingPlanUsage` | **这次官方目录里没有这个 Action**。目录里的 Coding Plan 用量是 `GetSeatInfoUsage`（团队个人席位）和 `ListSeatInfoUsages` | 同上目录页。仓库注释写明逐字段规格不是官方文档，样例来自 2026-06-21 的用户响应 | Q5 的火山行要拆开：Agent Plan 有官方 `GetAFPUsage`；Coding Plan 不应再把 `GetCodingPlanUsage` 当成官方。改挂 `GetSeatInfoUsage` 之前，字段 **未实测** |
| ZenMux | `GET` 用户配置的 `base_url` 本身（`query_zenmux`） | **未找到官方额度文档** | 仓库源码 | 不进 Q5 的「能读」清单，除非补到文档 |
| Gemini 订阅 | `POST https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist` 然后 `retrieveUserQuota` | **非官方公共 API**。端点在 [gemini-cli `code_assist/server.ts`](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/code_assist/server.ts)，可用 `CODE_ASSIST_ENDPOINT` 改主机。Google 写明 2026-06-18 起，面向个人的 Code Assist / AI Pro / AI Ultra 登录不再服务 Gemini CLI | [Gemini Code Assist consumer accounts](https://developers.google.com/gemini-code-assist/docs/deprecations/code-assist-individuals) | Q5「Gemini 订阅未验证、不进 MVP」维持，并加上个人 OAuth 已弃用。API Key 路径不靠这个端点 |
| Cursor | 无个人额度 API | 技术方案已标不能读 | — | 不变 |
| DeepSeek 余额 | `GET https://api.deepseek.com/user/balance`（`providers/balance/mod.rs`） | 这是余额不是 Coding Plan 窗。本次没有重抓 DeepSeek 文档，不新标官方 | 仓库注释 | 不改 Q5 订阅表 |

### 5.1 未实测

整张表的 HTTP 调用都没发生。官方页面是否在抓取之后改版，以再抓一次为准。火山 `GetAFPUsage` 只核对了目录名和文档摘要里的主机与窗口描述，没有对照 JSON 样例的每一个字段。

### 5.2 对 Q5 的影响

建议把 Q5 的「状态」列收成：

| Provider | Spike 之后的状态 |
|---|---|
| Claude 5h / 周窗 | 仍是非官方 usage URL。代理能看见订阅请求（SP-1，文档），不等于这个额度 URL 变成官方 |
| Codex 订阅窗 | 仍是非官方 `wham/usage`。默认流量不进代理（SP-2），窗数据继续靠这条接口 |
| Kimi / 智谱 | 非官方额度端点。智谱的窗规则（5 小时 + 周）本身有官方说明 |
| MiniMax | 官方有用量 API，URL 与仓库不一致，先不要标成已核实 |
| 火山 | `GetAFPUsage` 官方，主机字符串要核对；`GetCodingPlanUsage` 从「能读」里拿掉，改评估 `GetSeatInfoUsage` |
| Gemini 订阅 | 仍不进 MVP；个人 Google 登录这条产品线官方已弃用 |
| Cursor | 仍不支持 |

伪美元的观测法（ADR-009）依赖这些百分比。百分比来源被标成非官方的，UI 必须能显示「数据过期 / 来源非官方」，这和 PRD M4 已经写的过期态一致。

### 5.3 风险

- 智谱、火山的套餐条款要求固定 Base URL 和指定工具。本地代理改 Base URL 之后，套餐 Key 可能被当成普通 API 甚至违规。硬停在技术上做得到，不表示套餐仍按套餐计费。
- 把 `GetCodingPlanUsage` 的百分比当官方字段：上游改名就不会报错，只会变成空窗，伪美元失真（R3）。
- MiniMax 两条 URL 如果混用，国际站和国内站会读到不同的账户或直接 401。

---

## 6. 固定约束，对照这次的证据

| 约束 | 这次能说的 |
|---|---|
| 订阅只告警，硬停和 Kill 只打 API Key | 分类器把 `subscription_oauth` 和 `api_key` 分成两态。Coding Plan 的 Key 归 API Key。`unknown` 不能升级成订阅豁免。现有路由器不看客户端身份，做不到这条 |
| 代理崩溃默认 fail-closed | Codex 指针在进程退出后仍指向 `127.0.0.1`（`pointer.rs`，决定 35）。这是 fail-closed 的形状。没有杀掉 App 再跑 CLI。Claude 的 `ANTHROPIC_BASE_URL` 同样会在代理挂掉时连接失败，前提是用户没有另备直连；本次没有跑 Claude Code。open 模式要主动改回官方 URL，现有代码没有 |
| License 失效默认 fail-closed | 仓库里没有 `on_license_invalid`。无可测行为，**未实测**。和分类无关：失效时不应把已有规则降成「全部放行」，除非用户选了 open |

---

## 7. 不在本次范围里的事

没有改 `route/`，没有加 `PolicyGate`，没有碰发布流水线，没有合并到 `main`。harness 不进 Tauri 构建。真实账单、真实 OAuth 登录、证书锁定的二进制实验，都留在 **未实测**。
