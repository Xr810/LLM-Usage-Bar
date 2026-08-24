# 路由改为引用 provider 名单,而不是自己再存一份

创建:2026-08-24
取代:`2026-08-14-local-routing-design.md` 中「router 自带一份 provider 名单」的前提
(决定 7 / 8 / 21 / 22 受影响,见 §4)

---

## 0. 一句话

**provider 名单只有一份**(`usage_providers`,也就是用户加进来监控的那些);
**路由只是从这份名单里挑一个有序子集**,按 agent 分。参照 LiteLLM。

## 1. 为什么改

现在是两份名单:`usage_providers`(监控)与 `router_providers`(路由)。后者把
id、显示名、base_url、协议、认证方式**又存了一遍**。

这个重复的第一个症状是**凭据存不进去**:`provider_api_keys.provider_id` 有外键
`REFERENCES usage_providers(id)`,而路由用的是另一张表里的 id,所以「给路由里的
某一家绑一把 key」在数据库层面就不成立。

不改的话,后面每加一个跟 provider 有关的能力(余额、连通性测试、模型清单拉取、
密钥轮换)都要在两份名单之间再对一次账。

## 2. 骨架其实已经在了

`usage_providers` 每行自带:

| 列 | 内容 | 例 |
| --- | --- | --- |
| `route_app_type` | 这家能给哪个 agent 用 | `codex` / `claude` |
| `route_config` | 连接方式 | `{"base_url":"…","apiFormat":"openai_chat","authMode":"bearer"}` |

实测(用户库):18 家 `codex`、1 家 `claude`;`apiFormat` 只有 `openai_chat`(bearer)
与 `anthropic`(x_api_key)两种。还有一张 `route_bindings(protocol, provider_id)`,
语义就是「这个 agent 现在走哪一家」—— 目前为空。

**所以「一份名单 + 按 agent 挑」这套结构本来就存在,是后做的本地路由没有用它。**

## 3. 新模型

一条路由项 = **对 `usage_providers` 的引用** + 顺序 + 启用位,按 agent 分组:

```
router_chain(agent, provider_id, priority, enabled)
router_model_map(agent, provider_id, logical_model, upstream_model)
```

派生出来的、不再重复存的:

| 转发链要的 | 从哪来 |
| --- | --- |
| `base_url` | `route_config.base_url`(用户覆盖过的以凭据里的 canonical endpoint 为准) |
| `wire_api` | 由 `route_config.apiFormat` 推:`openai_chat` → `chat_completions` |
| 认证 | 那家 usage provider 自己的凭据(`authMode` 决定 bearer 还是 x-api-key) |
| 显示名 | `usage_providers.name` |

**凭据问题自动消失** —— key 本来就挂在那家名下,路由不再需要 `credential_key_id`。

## 4. 对已批准决定的影响

| 决定 | 状态 |
| --- | --- |
| 7(用各家内置模型 ID) | **不变** |
| 8(先加 provider 再排顺序) | **语义变了**:不再「加」provider,而是从名单里**挑**;排顺序不变 |
| 21(v1 只有一个全局顺序) | **被取代**:顺序天然按 agent 分,因为 `route_app_type` 就是按 agent 的 |
| 22(要知道哪家有哪个模型) | **不变**,而且更容易 —— 模型清单能从那家的 `model_list_path` 拉 |
| 10/13/14/15/16(顺序、拉黑、降级) | **不变**,只是作用在新结构上 |
| 28–33(模式、手动、拉黑显示) | **不变** |
| 34/36(指针、缺口标记) | **不变** |

## 5. 影响面

- **Rust**:`router_providers` 表与 DAO、`api::router` 的视图与方法、十个 bridge 命令
  的入参出参、`route/auth.rs` 的取 key 路径、`route/server.rs` 的 base_url/wire_api 来源
- **Swift**:`RouterProviderV1` 等 DTO、`RouterRepository`、`RouterPanelModel`
- **UI**:「添加 Provider」变成「从名单里挑」;凭据那一节从「绑 key」变成「显示那家的
  凭据状态」,绑 key 回到 provider 自己的设置里;模式与分账按 agent 分
- 已落的五个提交**不回退**,在其上继续改

## 6. 迁移

`router_providers` 目前**在用户库里还不存在**(实测:装着的构建早于路由功能,
`sqlite_master` 里没有这张表)。所以这次不需要写数据迁移 —— 直接改表定义即可。
若将来有用户已经建过表,再补一段把旧行按 id 匹配到 `usage_providers` 的迁移。
