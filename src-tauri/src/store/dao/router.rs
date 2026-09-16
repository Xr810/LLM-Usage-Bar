use crate::error::AppError;
use crate::store::{lock_conn, Database};
use rusqlite::params;
use std::time::{SystemTime, UNIX_EPOCH};

/// 转发链要用的一家 provider,**这是派生出来的视图,不是一张表**。
///
/// 2026-08-24 起路由不再自带 provider 名单:`router_chain` 只存「引用 + 顺序」,
/// 下面这些字段全部从被引用的 `usage_providers` 那行推出来
/// (见 docs/design/2026-08-24-router-references-usage-providers.md)。
///
/// 形状保持不变,是为了让转发链(`route/server.rs`、`route/auth.rs`)不受影响 ——
/// 变的是「这份数据从哪来」,不是「转发时需要什么」。
#[derive(Debug, Clone, PartialEq)]
pub struct RouterProvider {
    /// = `usage_providers.id`
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub wire_api: WireApi,
    pub priority: i64,
    pub enabled: bool,
    /// 从 `billing_kind` 与 `route_config.authMode` 推出来。
    pub auth_kind: RouterAuthKind,
    /// 那家 usage provider 的主 key(`provider_api_keys` 按 sort_order 的第一条)。
    /// 只是「去哪把凭据取出来」,不是凭据本身。
    pub credential_key_id: Option<String>,
}

/// 一家 usage provider「当转发目标时」的样子。
///
/// 链上项与候选项(面板「从名单里挑」那一屏)都由它派生 —— 推导规则只有
/// `derive_connection` 一处。上一版就是因为 api 层自己又推了一遍,
/// base_url、协议、认证在面板与转发链之间会慢慢分叉。
#[derive(Debug, Clone, PartialEq)]
pub struct RouterProviderRef {
    /// = `usage_providers.id`
    pub id: String,
    pub display_name: String,
    /// `subscription` | `metered`,原样来自 `usage_providers`。
    pub billing_kind: String,
    /// 推不出来时是**空串** —— 这家当不了转发目标。转发链直接过滤掉它,
    /// 面板则要显示成「缺连接信息」,所以这里不提前丢。
    pub base_url: String,
    pub wire_api: WireApi,
    pub auth_kind: RouterAuthKind,
    /// 那家的主 key。只是「去哪把凭据取出来」,不是凭据本身。
    pub credential_key_id: Option<String>,
}

/// 存下来的那一条:引用 + 顺序。**只有这个是真表。**
#[derive(Debug, Clone, PartialEq)]
pub struct RouterChainEntry {
    /// `codex` | `claude`,对应 `usage_providers.route_app_type`
    pub agent: String,
    /// → `usage_providers.id`
    pub provider_id: String,
    pub priority: i64,
    pub enabled: bool,
}

/// Codex 官方订阅那条路没有 `route_config`,base_url 从这里来。
const CHATGPT_OFFICIAL_BASE_URL: &str = "https://chatgpt.com/backend-api";

/// 从 usage_providers 那一行推出连接方式。
///
/// 实测取值(2026-08-24,用户库):`apiFormat` 只有 `openai_chat`(bearer)与
/// `anthropic`(x_api_key);订阅那家没有 route_config。
fn derive_connection(
    billing_kind: &str,
    route_config: Option<&str>,
) -> (String, WireApi, RouterAuthKind) {
    let config: Option<serde_json::Value> =
        route_config.and_then(|raw| serde_json::from_str(raw).ok());

    let base_url = config
        .as_ref()
        .and_then(|c| c.get("base_url"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);

    let api_format = config
        .as_ref()
        .and_then(|c| c.get("apiFormat"))
        .and_then(serde_json::Value::as_str);
    let auth_mode = config
        .as_ref()
        .and_then(|c| c.get("authMode"))
        .and_then(serde_json::Value::as_str);

    match (billing_kind, base_url) {
        // 订阅且没有 route_config = Codex 自己那条 OAuth 路。
        ("subscription", None) => (
            CHATGPT_OFFICIAL_BASE_URL.to_string(),
            WireApi::Responses,
            RouterAuthKind::ChatgptOauth,
        ),
        (_, Some(url)) => {
            let wire = match api_format {
                // Codex 原生说 responses;其余 OpenAI 兼容家都是 chat completions。
                Some("openai_responses") => WireApi::Responses,
                _ => WireApi::ChatCompletions,
            };
            let auth = match auth_mode {
                Some("bearer") | Some("x_api_key") => RouterAuthKind::BearerKey,
                _ => RouterAuthKind::None,
            };
            (url, wire, auth)
        }
        // 既不是订阅、又没有 base_url —— 这家没法当转发目标,交给上层过滤。
        (_, None) => (String::new(), WireApi::ChatCompletions, RouterAuthKind::None),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireApi {
    Responses,
    ChatCompletions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouterAuthKind {
    ChatgptOauth,
    BearerKey,
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelRoute {
    /// 映射按 agent 分 —— Codex 挑的那几家和 Claude 挑的那几家互不相干。
    pub agent: String,
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

fn now_timestamp() -> Result<i64, AppError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|error| AppError::Database(format!("system clock before unix epoch: {error}")))
}

pub(crate) fn wire_api_to_db(api: WireApi) -> &'static str {
    match api {
        WireApi::Responses => "responses",
        WireApi::ChatCompletions => "chat_completions",
    }
}

fn wire_api_from_db(raw: &str) -> Result<WireApi, AppError> {
    match raw {
        "responses" => Ok(WireApi::Responses),
        "chat_completions" => Ok(WireApi::ChatCompletions),
        other => Err(AppError::Database(format!("非法 wire_api 值: {other}"))),
    }
}

pub(crate) fn auth_kind_to_db(kind: RouterAuthKind) -> &'static str {
    match kind {
        RouterAuthKind::ChatgptOauth => "chatgpt_oauth",
        RouterAuthKind::BearerKey => "bearer_key",
        RouterAuthKind::None => "none",
    }
}

fn auth_kind_from_db(raw: &str) -> Result<RouterAuthKind, AppError> {
    match raw {
        "chatgpt_oauth" => Ok(RouterAuthKind::ChatgptOauth),
        "bearer_key" => Ok(RouterAuthKind::BearerKey),
        "none" => Ok(RouterAuthKind::None),
        other => Err(AppError::Database(format!("非法 auth_kind 值: {other}"))),
    }
}

/// 读回 attempt 行时把 outcome 字符串转回枚举(`outcome_to_db` 的反方向)。
///
/// T3 误把这份放进了 tests 里,生产代码取不到;T6 按任务书 §4.3.1 提到生产区。
/// 生产侧目前只有写没有读——读回方是后续的面板分账任务,落地后删掉这个 allow。
#[allow(dead_code)]
fn outcome_from_db(raw: &str) -> Result<AttemptOutcome, AppError> {
    match raw {
        "success" => Ok(AttemptOutcome::Success),
        "failed" => Ok(AttemptOutcome::Failed),
        "skipped" => Ok(AttemptOutcome::Skipped),
        other => Err(AppError::Database(format!(
            "非法 attempt outcome 值: {other}"
        ))),
    }
}

fn outcome_to_db(outcome: AttemptOutcome) -> &'static str {
    match outcome {
        AttemptOutcome::Success => "success",
        AttemptOutcome::Failed => "failed",
        AttemptOutcome::Skipped => "skipped",
    }
}

fn model_route_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<ModelRoute> {
    Ok(ModelRoute {
        agent: row.get(0)?,
        provider_id: row.get(1)?,
        logical_model: row.get(2)?,
        upstream_model: row.get(3)?,
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl Database {
    /// 某个 agent 的转发链,按 priority 升序,**只含启用且可用的那些**。
    ///
    /// 这是一次 JOIN:链上只存引用与顺序,连接方式(base_url / 协议 / 认证)
    /// 由被引用的 usage_providers 那一行推出来。凭据取那家的主 key
    /// (`provider_api_keys` 按 sort_order 的第一条)。
    pub fn list_router_providers(&self, agent: &str) -> Result<Vec<RouterProvider>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut statement = conn.prepare(
            "SELECT chain.provider_id, up.name, up.billing_kind, up.route_config,
                    chain.priority, chain.enabled,
                    (SELECT k.id FROM provider_api_keys k
                      WHERE k.provider_id = chain.provider_id
                      ORDER BY k.sort_order, k.created_at, k.id LIMIT 1)
             FROM router_chain chain
             JOIN usage_providers up ON up.id = chain.provider_id
             WHERE chain.agent = ?1 AND chain.enabled = 1
             ORDER BY chain.priority ASC, chain.provider_id ASC",
        )?;
        let rows = statement
            .query_map([agent], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, bool>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows
            .into_iter()
            .map(
                |(id, name, billing, config, priority, enabled, key_id)| {
                    let (base_url, wire_api, auth_kind) =
                        derive_connection(&billing, config.as_deref());
                    RouterProvider {
                        id,
                        display_name: name,
                        base_url,
                        wire_api,
                        priority,
                        enabled,
                        auth_kind,
                        credential_key_id: key_id,
                    }
                },
            )
            // base_url 推不出来的那家当不了转发目标,直接不进链 —— 让它带着空
            // base_url 往下走,只会在拼 URL 时变成一个看不懂的 404。
            .filter(|provider| !provider.base_url.is_empty())
            .collect())
    }

    /// 链上存的原始行(含未启用的),供设置界面编辑。
    pub fn list_router_chain(&self, agent: &str) -> Result<Vec<RouterChainEntry>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut statement = conn.prepare(
            "SELECT agent, provider_id, priority, enabled FROM router_chain
             WHERE agent = ?1 ORDER BY priority ASC, provider_id ASC",
        )?;
        let rows = statement
            .query_map([agent], |row| {
                Ok(RouterChainEntry {
                    agent: row.get(0)?,
                    provider_id: row.get(1)?,
                    priority: row.get(2)?,
                    enabled: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)?;
        Ok(rows)
    }

    /// 能给这个 agent 当转发目标的**全部** provider,不论在不在链上,按 id 升序。
    ///
    /// 「添加 Provider」在新模型下是从这份名单里挑(设计文档 §4 决定 8),
    /// 面板显示链上那几家时也用它来取派生字段 —— 两个用途共用一份推导,
    /// 面板看到的连接方式与转发时真正用的必然一致。
    pub fn router_provider_refs(&self, agent: &str) -> Result<Vec<RouterProviderRef>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut statement = conn.prepare(
            "SELECT up.id, up.name, up.billing_kind, up.route_config,
                    (SELECT k.id FROM provider_api_keys k
                      WHERE k.provider_id = up.id
                      ORDER BY k.sort_order, k.created_at, k.id LIMIT 1)
             FROM usage_providers up
             WHERE up.route_app_type = ?1
             ORDER BY up.id ASC",
        )?;
        let rows = statement
            .query_map([agent], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)?;

        Ok(rows
            .into_iter()
            .map(|(id, name, billing, config, key_id)| {
                let (base_url, wire_api, auth_kind) = derive_connection(&billing, config.as_deref());
                RouterProviderRef {
                    id,
                    display_name: name,
                    billing_kind: billing,
                    base_url,
                    wire_api,
                    auth_kind,
                    credential_key_id: key_id,
                }
            })
            .collect())
    }

    /// 某个逻辑模型在这个 agent 各家上的映射，按 priority 升序。
    /// 返回的每一项都保证那家在链上且启用。
    pub fn list_model_routes(
        &self,
        agent: &str,
        logical_model: &str,
    ) -> Result<Vec<ModelRoute>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut statement = conn.prepare(
            "SELECT map.agent, map.provider_id, map.logical_model, map.upstream_model
             FROM router_model_map map
             JOIN router_chain chain
               ON chain.agent = map.agent AND chain.provider_id = map.provider_id
             WHERE map.agent = ?1 AND map.logical_model = ?2 AND chain.enabled = 1
             ORDER BY chain.priority ASC, chain.provider_id ASC",
        )?;
        let routes = statement
            .query_map([agent, logical_model], model_route_from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)?;
        Ok(routes)
    }

    /// 供设置界面用:某个 agent 的**全部**映射,含已停用那些。
    ///
    /// 与上面是两个用途,别合并:那个是转发决策路径(单模型、只要启用的),
    /// 这个是编辑界面的读路径,要把用户配过的原样显示出来。
    pub fn list_all_model_routes(&self, agent: &str) -> Result<Vec<ModelRoute>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut statement = conn.prepare(
            "SELECT map.agent, map.provider_id, map.logical_model, map.upstream_model
             FROM router_model_map map
             JOIN router_chain chain
               ON chain.agent = map.agent AND chain.provider_id = map.provider_id
             WHERE map.agent = ?1
             ORDER BY chain.priority ASC, chain.provider_id ASC, map.logical_model ASC",
        )?;
        let routes = statement
            .query_map([agent], model_route_from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)?;
        Ok(routes)
    }

    /// 把一家加进链、或改它的顺序与启用位。
    pub fn upsert_router_chain_entry(&self, entry: &RouterChainEntry) -> Result<(), AppError> {
        let now = now_ms();
        let conn = lock_conn!(self.conn);
        conn.execute(
            "INSERT INTO router_chain
                 (agent, provider_id, priority, enabled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(agent, provider_id) DO UPDATE SET
                 priority   = excluded.priority,
                 enabled    = excluded.enabled,
                 updated_at = excluded.updated_at",
            params![
                entry.agent,
                entry.provider_id,
                entry.priority,
                entry.enabled as i64,
                now
            ],
        )?;
        Ok(())
    }

    /// 把一家移出链。**不动那家 usage provider 本身** —— 用户只是不再路由到它,
    /// 不代表不再监控它。
    pub fn remove_from_router_chain(
        &self,
        agent: &str,
        provider_id: &str,
    ) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "DELETE FROM router_chain WHERE agent = ?1 AND provider_id = ?2",
            [agent, provider_id],
        )?;
        Ok(())
    }

    pub fn upsert_model_route(&self, r: &ModelRoute) -> Result<(), AppError> {
        let now = now_ms();
        let conn = lock_conn!(self.conn);
        conn.execute(
            "INSERT INTO router_model_map
                 (agent, provider_id, logical_model, upstream_model, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(agent, provider_id, logical_model) DO UPDATE SET
                 upstream_model = excluded.upstream_model",
            params![r.agent, r.provider_id, r.logical_model, r.upstream_model, now],
        )?;
        Ok(())
    }

    pub fn delete_model_routes_for_provider(
        &self,
        agent: &str,
        provider_id: &str,
    ) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "DELETE FROM router_model_map WHERE agent = ?1 AND provider_id = ?2",
            [agent, provider_id],
        )?;
        Ok(())
    }

    pub fn record_router_attempt(&self, a: &RouterAttempt) -> Result<i64, AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "INSERT INTO router_attempts (
                 started_at, logical_model, provider_id, outcome, failure_kind,
                 http_status, input_tokens, output_tokens, duration_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                a.started_at,
                a.logical_model,
                a.provider_id,
                outcome_to_db(a.outcome),
                a.failure_kind,
                a.http_status.map(i64::from),
                a.input_tokens,
                a.output_tokens,
                a.duration_ms,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// 把一次尝试的 token 数回填上去。行不存在时不报错——那一行可能因为
    /// 写库失败根本没进去，为此让一个已经成功的请求失败不划算。
    pub fn update_router_attempt_tokens(
        &self,
        id: i64,
        input_tokens: i64,
        output_tokens: i64,
    ) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "UPDATE router_attempts
             SET input_tokens = ?2, output_tokens = ?3
             WHERE id = ?1",
            params![id, input_tokens, output_tokens],
        )?;
        Ok(())
    }

    /// 按 provider 汇总某段时间的用量，供分账面板使用。
    ///
    /// 时间范围是半开区间 `[start_at, end_at)`，按 `started_at`（epoch 毫秒）过滤。
    pub fn sum_router_usage_by_provider(
        &self,
        start_at: i64,
        end_at: i64,
    ) -> Result<Vec<RouterUsageSummary>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut statement = conn.prepare(
            "SELECT provider_id,
                    COUNT(*) AS attempts,
                    COALESCE(SUM(CASE WHEN outcome = 'failed' THEN 1 ELSE 0 END), 0) AS failures,
                    COALESCE(SUM(input_tokens), 0) AS input_tokens,
                    COALESCE(SUM(output_tokens), 0) AS output_tokens
             FROM router_attempts
             WHERE started_at >= ?1 AND started_at < ?2
             GROUP BY provider_id
             ORDER BY provider_id",
        )?;
        let summaries = statement
            .query_map(params![start_at, end_at], |row| {
                Ok(RouterUsageSummary {
                    provider_id: row.get(0)?,
                    attempts: row.get(1)?,
                    failures: row.get(2)?,
                    input_tokens: row.get(3)?,
                    output_tokens: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)?;
        Ok(summaries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Database;

    fn provider(id: &str, priority: i64, enabled: bool) -> RouterProvider {
        RouterProvider {
            id: id.to_string(),
            display_name: format!("Provider {id}"),
            base_url: format!("https://{id}.example/v1"),
            wire_api: WireApi::Responses,
            priority,
            enabled,
            auth_kind: RouterAuthKind::None,
            credential_key_id: None,
        }
    }

    fn route(provider_id: &str, logical_model: &str, upstream_model: &str) -> ModelRoute {
        ModelRoute {
            provider_id: provider_id.to_string(),
            logical_model: logical_model.to_string(),
            upstream_model: upstream_model.to_string(),
        }
    }

    fn attempt(
        started_at: i64,
        provider_id: &str,
        outcome: AttemptOutcome,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
    ) -> RouterAttempt {
        RouterAttempt {
            started_at,
            logical_model: "gpt-5.6-sol".to_string(),
            provider_id: provider_id.to_string(),
            outcome,
            failure_kind: if outcome == AttemptOutcome::Failed {
                Some("timeout".to_string())
            } else {
                None
            },
            http_status: None,
            input_tokens,
            output_tokens,
            duration_ms: None,
        }
    }

    #[test]
    fn list_router_providers_orders_by_priority_and_skips_disabled() {
        let db = Database::memory().unwrap();
        db.upsert_router_provider(&provider("a", 20, true)).unwrap();
        db.upsert_router_provider(&provider("b", 10, true)).unwrap();
        db.upsert_router_provider(&provider("c", 30, false))
            .unwrap();

        let listed = db.list_router_providers().unwrap();

        assert_eq!(
            listed.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            vec!["b", "a"],
        );
    }

    #[test]
    fn list_model_routes_filters_by_logical_model_orders_by_priority_and_skips_disabled() {
        let db = Database::memory().unwrap();
        db.upsert_router_provider(&provider("a", 20, true)).unwrap();
        db.upsert_router_provider(&provider("b", 10, true)).unwrap();
        db.upsert_router_provider(&provider("c", 5, false)).unwrap();
        db.upsert_model_route(&route("a", "gpt-5.6-sol", "sol"))
            .unwrap();
        db.upsert_model_route(&route("b", "gpt-5.6-sol", "gpt-5.6-sol"))
            .unwrap();
        db.upsert_model_route(&route("c", "gpt-5.6-sol", "hidden"))
            .unwrap();
        db.upsert_model_route(&route("a", "other-model", "other"))
            .unwrap();

        let routes = db.list_model_routes("gpt-5.6-sol").unwrap();

        // c 的 priority 最小但 enabled = 0，必须被跳过；结果按 priority 升序。
        assert_eq!(
            routes,
            vec![
                route("b", "gpt-5.6-sol", "gpt-5.6-sol"),
                route("a", "gpt-5.6-sol", "sol"),
            ],
        );
    }

    #[test]
    fn list_all_model_routes_keeps_disabled_providers_and_orders_by_priority() {
        let db = Database::memory().unwrap();
        db.upsert_router_provider(&provider("a", 20, true)).unwrap();
        db.upsert_router_provider(&provider("b", 10, true)).unwrap();
        db.upsert_router_provider(&provider("c", 5, false)).unwrap();
        db.upsert_model_route(&route("a", "gpt-5.6-sol", "sol"))
            .unwrap();
        db.upsert_model_route(&route("a", "gpt-5.6", "a-56"))
            .unwrap();
        db.upsert_model_route(&route("b", "gpt-5.6-sol", "gpt-5.6-sol"))
            .unwrap();
        db.upsert_model_route(&route("c", "gpt-5.6-sol", "local"))
            .unwrap();

        let routes = db.list_all_model_routes().unwrap();

        // 与 list_model_routes 的关键差别:c 已停用,但编辑界面照样要显示它的映射。
        // 排序按 provider.priority 升序,组内按 logical_model。
        assert_eq!(
            routes,
            vec![
                route("c", "gpt-5.6-sol", "local"),
                route("b", "gpt-5.6-sol", "gpt-5.6-sol"),
                route("a", "gpt-5.6", "a-56"),
                route("a", "gpt-5.6-sol", "sol"),
            ],
        );
    }

    #[test]
    fn list_all_model_routes_is_empty_when_nothing_configured() {
        let db = Database::memory().unwrap();

        assert_eq!(db.list_all_model_routes().unwrap(), Vec::new());
    }

    #[test]
    fn list_model_routes_returns_empty_vec_for_unknown_model() {
        let db = Database::memory().unwrap();

        assert_eq!(db.list_model_routes("nonexistent").unwrap(), Vec::new());
    }

    #[test]
    fn delete_router_provider_cascades_to_model_map() {
        let db = Database::memory().unwrap();
        db.upsert_router_provider(&provider("a", 1, true)).unwrap();
        db.upsert_router_provider(&provider("b", 2, true)).unwrap();
        db.upsert_model_route(&route("a", "m1", "u1")).unwrap();
        db.upsert_model_route(&route("b", "m1", "u2")).unwrap();

        db.delete_router_provider("a").unwrap();

        assert_eq!(
            db.list_model_routes("m1").unwrap(),
            vec![route("b", "m1", "u2")],
        );
        let remaining: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM router_model_map", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn record_router_attempt_feeds_provider_summary_within_time_range() {
        let db = Database::memory().unwrap();
        db.record_router_attempt(&attempt(
            1000,
            "a",
            AttemptOutcome::Success,
            Some(10),
            Some(20),
        ))
        .unwrap();
        db.record_router_attempt(&attempt(
            1500,
            "a",
            AttemptOutcome::Failed,
            Some(1),
            Some(2),
        ))
        .unwrap();
        db.record_router_attempt(&attempt(
            2000,
            "b",
            AttemptOutcome::Success,
            Some(30),
            Some(40),
        ))
        .unwrap();
        db.record_router_attempt(&attempt(2500, "b", AttemptOutcome::Failed, None, None))
            .unwrap();
        db.record_router_attempt(&attempt(
            3000,
            "a",
            AttemptOutcome::Success,
            Some(100),
            Some(200),
        ))
        .unwrap();

        let summary = db.sum_router_usage_by_provider(1500, 2500).unwrap();

        // [1500, 2500) 半开区间:1000 与 3000 在范围外,2500 是开边界,都不计入。
        assert_eq!(
            summary,
            vec![
                RouterUsageSummary {
                    provider_id: "a".to_string(),
                    attempts: 1,
                    failures: 1,
                    input_tokens: 1,
                    output_tokens: 2,
                },
                RouterUsageSummary {
                    provider_id: "b".to_string(),
                    attempts: 1,
                    failures: 0,
                    input_tokens: 30,
                    output_tokens: 40,
                },
            ],
        );
    }

    #[test]
    fn wire_api_and_attempt_outcome_round_trip_through_storage() {
        let db = Database::memory().unwrap();
        let mut stored_provider = provider("packyapi", 1, true);
        stored_provider.wire_api = WireApi::ChatCompletions;
        db.upsert_router_provider(&stored_provider).unwrap();

        let listed = db.list_router_providers().unwrap();
        assert_eq!(listed[0].wire_api, WireApi::ChatCompletions);

        for (outcome, expected) in [
            (AttemptOutcome::Success, "success"),
            (AttemptOutcome::Failed, "failed"),
            (AttemptOutcome::Skipped, "skipped"),
        ] {
            db.record_router_attempt(&attempt(1, "packyapi", outcome, None, None))
                .unwrap();
            let raw: String = {
                let conn = db.conn.lock().unwrap();
                conn.query_row(
                    "SELECT outcome FROM router_attempts ORDER BY id DESC LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap()
            };
            assert_eq!(raw, expected);
            assert_eq!(outcome_from_db(&raw).unwrap(), outcome);
        }
    }

    #[test]
    fn invalid_wire_api_in_db_returns_error_instead_of_panicking() {
        let db = Database::memory().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO router_providers (
                     id, display_name, base_url, wire_api, priority, enabled,
                     created_at, updated_at
                 ) VALUES ('broken', 'Broken', 'https://broken.example', 'bogus', 1, 1, 0, 0)",
                [],
            )
            .unwrap();
        }

        let error = db.list_router_providers().unwrap_err();

        assert!(error.to_string().contains("wire_api"));
    }

    #[test]
    fn upserts_overwrite_provider_and_route_fields() {
        let db = Database::memory().unwrap();
        let mut updated = provider("packyapi", 5, true);
        db.upsert_router_provider(&updated).unwrap();
        updated.display_name = "PackyAPI v2".to_string();
        updated.wire_api = WireApi::ChatCompletions;
        updated.priority = 7;
        updated.enabled = false;
        db.upsert_router_provider(&updated).unwrap();

        let conn = db.conn.lock().unwrap();
        let (name, wire, priority, enabled): (String, String, i64, i64) = conn
            .query_row(
                "SELECT display_name, wire_api, priority, enabled
                 FROM router_providers WHERE id = 'packyapi'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        drop(conn);
        assert_eq!(name, "PackyAPI v2");
        assert_eq!(wire, "chat_completions");
        assert_eq!((priority, enabled), (7, 0));
        assert!(db.list_router_providers().unwrap().is_empty());

        db.upsert_model_route(&route("packyapi", "gpt-5.6-sol", "sol"))
            .unwrap();
        db.upsert_model_route(&route("packyapi", "gpt-5.6-sol", "sol-2"))
            .unwrap();
        let (count, upstream): (i64, String) = {
            let conn = db.conn.lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*), MAX(upstream_model)
                 FROM router_model_map
                 WHERE provider_id = 'packyapi' AND logical_model = 'gpt-5.6-sol'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
        };
        assert_eq!(count, 1);
        assert_eq!(upstream, "sol-2");
    }
}
