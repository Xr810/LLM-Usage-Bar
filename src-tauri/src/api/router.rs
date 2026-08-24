//! 本地 router 的业务 api 层(T16):与传输无关,不 import 任何 tauri 类型。
//!
//! 模块化设计 §9.3 第三条:契约若直接长在 tauri 命令上,socket 面板就得再写一遍。
//! 这一层把「有哪些查询、哪些命令、返回什么、错误怎么脱敏」收成 `RouterApi`,
//! `commands/router.rs` 退化成薄壳;启动接线与将来的 socket 端也直接调这里。
//!
//! 安全面:输入结构体里**绝不出现凭据明文**——`credential_key_id` 只是指向
//! `provider_api_keys.id` 的引用,真正的 key 走仓库既有的凭据命令;返回值
//! 里也绝不回传凭据明文。指针的读写在 `router::pointer` 里实现,这里只是它的
//! 调用方。

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::route::pointer::PointerState;
use crate::store::{lock_conn, Database, ModelRoute, RouterChainEntry};

/// router 默认监听端口(settings 里没有 `router.port` 或解析不了时用)。
pub const DEFAULT_ROUTER_PORT: u16 = 8788;

/// 指针缺口标记的 settings 键(决定 36):从这个 epoch 毫秒起,用量可能没经过
/// router,受影响 provider 的估算不完整。
pub const POINTER_GAP_KEY: &str = "router.pointer_gap_since";

/// 链上的一家。
///
/// 除了 `priority` / `enabled` 之外**全部是派生值** —— 它们来自被引用的那家
/// usage provider,面板只读不写。想改 base_url 或凭据,要去那家 provider 自己的
/// 设置里改,不在路由面板里
/// (见 docs/design/2026-08-24-router-references-usage-providers.md)。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterProviderView {
    /// = `usage_providers.id`
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub wire_api: String,
    pub priority: i64,
    pub enabled: bool,
    pub auth_kind: String,
    /// 那家有没有可用凭据。**只是一个布尔** —— 连 key 的 id 都不回传,
    /// 面板没有任何理由知道它。
    pub has_credential: bool,
}

/// 可以加进链、但还没加的一家。「添加 Provider」在新模型下是**从名单里挑**,
/// 不是再填一遍连接信息。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterCandidateView {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub wire_api: String,
    pub auth_kind: String,
    pub has_credential: bool,
    /// 按量 / 订阅。面板用它解释「这家为什么没有余额可看」之类。
    pub billing_kind: String,
}

/// 前端提交的一条链上项。
///
/// **只有引用和顺序。** 连接方式与凭据都不在这里 —— 那些是被引用那家自己的属性,
/// 路由重复存一份正是上一版凭据存不进去的根因。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterChainInput {
    /// → `usage_providers.id`
    pub provider_id: String,
    pub priority: i64,
    pub enabled: bool,
}

/// 一条模型映射(全量替换的输入单元)。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRouteInput {
    pub logical_model: String,
    pub upstream_model: String,
}

/// 指针当前状态,供界面显示「已接管 / 未接管 / 读不到」。
/// 一条已存在的映射(读侧)。写侧是 `ModelRouteInput`,不带 provider —— 那边的语义是
/// 「给这个 provider 全量替换」;读侧一次返回所有 provider 的,所以必须带 `provider_id`。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRouteView {
    pub provider_id: String,
    pub logical_model: String,
    pub upstream_model: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PointerStateView {
    /// `"ours"` | `"not_ours"` | `"unreadable"`。
    pub state: String,
    /// NotOurs 时当前指向的 provider id;从未配置过 model_provider 是 None。
    pub current: Option<String>,
}

/// 某段时间内按 provider 汇总的路由尝试,供分账面板。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterUsageSummaryView {
    pub provider_id: String,
    pub attempts: i64,
    pub failures: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

/// 模式按 agent 分开存 —— Codex 走手动、Claude 走自动,是完全正常的组合。
fn mode_key(agent: &str) -> String {
    format!("router.mode.{agent}")
}

/// 读 `router.port`:读不到或解析不了就用默认 8788,不报错、不写回(任务书 §1.3)。
/// 端口被占是常见情况,启动接线那边对绑定失败也只记日志,这里不放大成错误。
pub(crate) fn read_router_port(db: &Database) -> u16 {
    match db.get_setting("router.port") {
        Ok(Some(raw)) => raw.trim().parse::<u16>().unwrap_or(DEFAULT_ROUTER_PORT),
        Ok(None) => DEFAULT_ROUTER_PORT,
        Err(_) => DEFAULT_ROUTER_PORT,
    }
}

/// 这家在不在某个 agent 的链上(不区分启用状态)。`set_mode` 的写侧校验用。
fn router_provider_exists(db: &Database, agent: &str, id: &str) -> Result<bool, AppError> {
    let conn = lock_conn!(db.conn);
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM router_chain WHERE agent = ?2 AND provider_id = ?1",
            rusqlite::params![id, agent],
            |row| row.get(0),
        )
        .map_err(AppError::from)?;
    Ok(count > 0)
}

/// 把指针状态落成 settings 缺口标记(决定 36):
/// - `NotOurs` / `Unreadable` → 记 `router.pointer_gap_since` = 发现时的 epoch
///   毫秒;**已有值不覆盖**(保留最早那次缺口);
/// - `OursAndCurrent` → 删掉标记。
///
/// 只在启动时调用(启动只读指针、绝不写,决定 34/36);`enable_pointer`
/// 写指针成功后也会调一次 OursAndCurrent 分支来清标记。
pub(crate) fn apply_pointer_gap_marker(
    db: &Database,
    state: &PointerState,
) -> Result<(), AppError> {
    match state {
        PointerState::OursAndCurrent => delete_setting(db, POINTER_GAP_KEY),
        PointerState::NotOurs { .. } | PointerState::Unreadable => {
            if db.get_setting(POINTER_GAP_KEY)?.is_none() {
                db.set_setting(POINTER_GAP_KEY, &now_millis().to_string())
            } else {
                Ok(())
            }
        }
    }
}

fn delete_setting(db: &Database, key: &str) -> Result<(), AppError> {
    let conn = lock_conn!(db.conn);
    conn.execute("DELETE FROM settings WHERE key = ?1", [key])
        .map_err(AppError::from)?;
    Ok(())
}

/// epoch 毫秒。缺口标记只需要「最早发现时刻」这一个用途,不值得为它引入
/// 时间库——仓库里的现成实现都在别的模块里,几行写在这里最直白。
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

/// 写指针前的守卫判定(纯函数,不读全局状态)。
///
/// `listening` 由调用方传入:进程级的 LISTENING_PORT 是全局状态,测试之间会
/// 互相污染,判定抽成纯函数才能用纯输入测。两种情况分别给用户能执行的下一步:
/// - 没监听:最常见的原因是端口被占(上一次 app 没退干净),换端口或重启 app;
/// - 端口不一致:改过 router.port 但没重启 app,重启后实际监听才对得上。
///
/// 错误类型是裸 `String` 而不是 `AppError`:这些文案就是最终给用户看的
/// 下一步指引,包进任何 AppError 变体都会在壳的 `to_string()` 里多出前缀,
/// 破坏脱敏面的字节一致性。
fn pointer_enable_guard(configured_port: u16, listening: Option<u16>) -> Result<(), String> {
    match listening {
        None => Err(format!(
            "router 没能在端口 {configured_port} 上启动,最常见的原因是端口被占用;\
             请换一个端口或重启 app 后再启用"
        )),
        Some(actual) if actual != configured_port => Err(format!(
            "配置的 router 端口是 {configured_port},实际监听的是 {actual};\
             修改端口后需要重启 app 才能生效,重启后再启用"
        )),
        Some(_) => Ok(()),
    }
}

/// 与传输无关的 router 业务入口:Tauri 命令、将来的 socket 端都只调它,
/// 传输特有类型不进入这一层。
pub struct RouterApi {
    db: Arc<Database>,
}

/// 从一家 usage provider 推出「当转发目标时」的连接方式。
///
/// 与 DAO 里那个 `derive_connection` 同一套规则,只是输入形态不同:那边拿的是
/// 数据库原始列,这边拿的是已经组装好的视图。规则本身只有一处真相 ——
/// 订阅且没有 route base = OAuth;有 base 就看 authMode。
fn derive_candidate_connection(
    provider: &crate::model::domain::UsageProviderView,
) -> (String, String, String) {
    match provider
        .route_base_url
        .as_deref()
        .or(provider.canonical_endpoint.as_deref())
    {
        Some(base) if !base.is_empty() => {
            let auth = if provider.has_route_credentials || !provider.api_keys.is_empty() {
                "bearer_key"
            } else {
                "none"
            };
            (base.to_string(), "chat_completions".to_string(), auth.to_string())
        }
        // 没有 base 的订阅那家 = Codex 自己的 OAuth 路。
        _ => (
            "https://chatgpt.com/backend-api".to_string(),
            "responses".to_string(),
            "chatgpt_oauth".to_string(),
        ),
    }
}

impl RouterApi {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// 这个 agent 链上的全部 provider(**含未启用的** —— 设置界面要编辑它们)。
    ///
    /// 除 priority / enabled 外都是派生值:显示名、base_url、协议、认证方式
    /// 来自被引用那家 usage provider,凭据只回一个布尔。
    pub fn list_providers(&self, agent: &str) -> Result<Vec<RouterProviderView>, AppError> {
        let chain = self.db.list_router_chain(agent)?;
        if chain.is_empty() {
            return Ok(Vec::new());
        }
        let catalog: std::collections::HashMap<String, _> = self
            .db
            .list_usage_providers()?
            .into_iter()
            .map(|provider| (provider.id.clone(), provider))
            .collect();

        Ok(chain
            .into_iter()
            .filter_map(|entry| {
                // 链上引用的那家被删了 —— 外键是 ON DELETE CASCADE,正常不会发生;
                // 真发生了就跳过,不要拿一行空壳去糊弄界面。
                let provider = catalog.get(&entry.provider_id)?;
                let (base_url, wire_api, auth_kind) = derive_candidate_connection(provider);
                Some(RouterProviderView {
                    id: entry.provider_id,
                    display_name: provider.name.clone(),
                    base_url,
                    wire_api,
                    priority: entry.priority,
                    enabled: entry.enabled,
                    auth_kind,
                    has_credential: provider.has_route_credentials
                        || !provider.api_keys.is_empty(),
                })
            })
            .collect())
    }

    /// 把一家加进链,或改它的顺序 / 启用位。
    ///
    /// 写侧要严:引用的那家必须**存在于 usage_providers,且它的 `route_app_type`
    /// 就是这个 agent**。否则会出现「Codex 的链上挂着一家只能给 Claude 用的
    /// provider」,转发时才炸,错误指向完全相反的方向。
    pub fn upsert_chain_entry(
        &self,
        agent: &str,
        input: RouterChainInput,
    ) -> Result<(), AppError> {
        let candidate = self
            .db
            .list_usage_providers()?
            .into_iter()
            .find(|provider| provider.id == input.provider_id)
            .ok_or_else(|| {
                AppError::InvalidInput(format!("provider 不存在: {}", input.provider_id))
            })?;

        if candidate.route_app_type.as_deref() != Some(agent) {
            return Err(AppError::InvalidInput(format!(
                "{} 不能用于 {agent}(它标的是 {})",
                input.provider_id,
                candidate.route_app_type.as_deref().unwrap_or("未标注")
            )));
        }

        self.db.upsert_router_chain_entry(&RouterChainEntry {
            agent: agent.to_string(),
            provider_id: input.provider_id,
            priority: input.priority,
            enabled: input.enabled,
        })
    }

    /// 把一家移出链。**不动那家 provider 本身** —— 用户只是不再路由到它,
    /// 不代表不再监控它。这正是新模型和旧模型最大的语义差别。
    pub fn remove_from_chain(&self, agent: &str, provider_id: &str) -> Result<(), AppError> {
        self.db.remove_from_router_chain(agent, provider_id)
    }

    /// 还没加进链、但可以加的那些。「添加 Provider」在新模型下是**从名单里挑**。
    pub fn list_candidates(&self, agent: &str) -> Result<Vec<RouterCandidateView>, AppError> {
        let on_chain: std::collections::HashSet<String> = self
            .db
            .list_router_chain(agent)?
            .into_iter()
            .map(|entry| entry.provider_id)
            .collect();

        Ok(self
            .db
            .list_usage_providers()?
            .into_iter()
            .filter(|provider| provider.route_app_type.as_deref() == Some(agent))
            .filter(|provider| !on_chain.contains(&provider.id))
            .map(|provider| {
                let (base_url, wire_api, auth_kind) = derive_candidate_connection(&provider);
                RouterCandidateView {
                    id: provider.id,
                    display_name: provider.name,
                    base_url,
                    wire_api,
                    auth_kind,
                    has_credential: provider.has_route_credentials
                        || !provider.api_keys.is_empty(),
                    billing_kind: format!("{:?}", provider.billing_kind).to_lowercase(),
                }
            })
            .collect())
    }

    /// 某家在这个 agent 上的模型映射,全量替换。
    ///
    /// 那家必须已经在链上:外键只会给一句生硬的 constraint 错误,写侧明确校验,
    /// 错误信息才对得上。
    pub fn set_model_routes(
        &self,
        agent: &str,
        provider_id: &str,
        routes: Vec<ModelRouteInput>,
    ) -> Result<(), AppError> {
        let on_chain = self
            .db
            .list_router_chain(agent)?
            .into_iter()
            .any(|entry| entry.provider_id == provider_id);
        if !on_chain {
            return Err(AppError::InvalidInput(format!(
                "{provider_id} 还没有加进 {agent} 的路由链"
            )));
        }
        self.db.delete_model_routes_for_provider(agent, provider_id)?;
        for route in routes {
            self.db.upsert_model_route(&ModelRoute {
                agent: agent.to_string(),
                provider_id: provider_id.to_string(),
                logical_model: route.logical_model,
                upstream_model: route.upstream_model,
            })?;
        }
        Ok(())
    }

    /// 这个 agent 的全部映射(含已停用那家的),供设置界面渲染。
    pub fn list_model_routes(&self, agent: &str) -> Result<Vec<ModelRouteView>, AppError> {
        Ok(self
            .db
            .list_all_model_routes(agent)?
            .into_iter()
            .map(|route| ModelRouteView {
                provider_id: route.provider_id,
                logical_model: route.logical_model,
                upstream_model: route.upstream_model,
            })
            .collect())
    }

    /// 读 `router.mode`。与 T6 转发层的读侧同样宽容:读不到或解析不了都当 `"auto"`,
    /// 返回给前端的永远是合法形态(垃圾值由读侧容错,写侧见 `set_mode`)。
    pub fn get_mode(&self, agent: &str) -> Result<String, AppError> {
        let raw = self.db.get_setting(&mode_key(agent))?;
        Ok(match raw.as_deref() {
            Some("auto") | None => "auto".to_string(),
            Some(manual) => match manual.strip_prefix("manual:") {
                Some(provider_id) if !provider_id.is_empty() => manual.to_string(),
                _ => "auto".to_string(),
            },
        })
    }

    /// 写 `router.mode.<agent>`,写侧要严(任务书 §3.2):
    /// - `"auto"` 直接存;
    /// - `"manual:<id>"` 要 `<id>` **在这个 agent 的链上**;
    /// - 其余一律 Err,垃圾进不了库。
    pub fn set_mode(&self, agent: &str, mode: &str) -> Result<(), AppError> {
        let key = mode_key(agent);
        if mode == "auto" {
            return self.db.set_setting(&key, "auto");
        }
        match mode.strip_prefix("manual:") {
            Some(provider_id) if !provider_id.is_empty() => {
                if !router_provider_exists(self.db.as_ref(), agent, provider_id)? {
                    return Err(AppError::InvalidInput(format!(
                        "manual 模式指向的 provider 不在 {agent} 的链上: {provider_id}"
                    )));
                }
                self.db.set_setting(&key, mode)
            }
            _ => Err(AppError::InvalidInput(format!("非法 router.mode: {mode}"))),
        }
    }

    /// 指针当前状态,供界面显示「已接管 / 未接管 / 读不到」。只读,绝不写。
    pub fn inspect_pointer() -> PointerStateView {
        match crate::route::pointer::inspect_pointer() {
            PointerState::OursAndCurrent => PointerStateView {
                state: "ours".to_string(),
                current: None,
            },
            PointerState::NotOurs { current } => PointerStateView {
                state: "not_ours".to_string(),
                current,
            },
            PointerState::Unreadable => PointerStateView {
                state: "unreadable".to_string(),
                current: None,
            },
        }
    }

    /// 用户显式点「启用」时调用:写一次指针(决定 34,之后永不再改)。
    /// 写之前先确认 router 真的在监听——端口被占时 router 没起来,把指针写进去
    /// 会让 Codex 每个请求都变成「连接被拒」,而指针按设计不能再改回去。
    /// 写入成功后清掉缺口标记——指针刚被自己接管,缺口到此结束。
    pub fn enable_pointer(&self) -> Result<(), AppError> {
        let port = read_router_port(self.db.as_ref());
        pointer_enable_guard(port, crate::route::server::listening_port())
            .map_err(AppError::Message)?;
        crate::route::pointer::point_codex_at_router(port)?;
        if let Err(error) =
            apply_pointer_gap_marker(self.db.as_ref(), &PointerState::OursAndCurrent)
        {
            // 指针已经写成功,标记清不掉只是 UI 上缺口提示还在,不把成功改成失败。
            log::warn!("指针已写入,但清除缺口标记失败: {error}");
        }
        Ok(())
    }

    /// 最近的路由尝试,按 provider 汇总,供分账面板。时间范围是半开区间。
    pub fn recent_attempts(
        &self,
        start_at: i64,
        end_at: i64,
    ) -> Result<Vec<RouterUsageSummaryView>, AppError> {
        self.db
            .sum_router_usage_by_provider(start_at, end_at)
            .map(|rows| {
                rows.into_iter()
                    .map(|row| RouterUsageSummaryView {
                        provider_id: row.provider_id,
                        attempts: row.attempts,
                        failures: row.failures,
                        input_tokens: row.input_tokens,
                        output_tokens: row.output_tokens,
                    })
                    .collect()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> Arc<Database> {
        Arc::new(Database::memory().unwrap())
    }

    fn upsert_provider(db: &Database, id: &str) {
        db.upsert_router_provider(&RouterProvider {
            id: id.to_string(),
            display_name: id.to_string(),
            base_url: format!("https://{id}.example/v1"),
            wire_api: WireApi::Responses,
            priority: 1,
            enabled: true,
            auth_kind: RouterAuthKind::None,
            credential_key_id: None,
        })
        .unwrap();
    }

    fn route(logical: &str, upstream: &str) -> ModelRouteInput {
        ModelRouteInput {
            logical_model: logical.to_string(),
            upstream_model: upstream.to_string(),
        }
    }

    fn route_count(db: &Database, provider_id: &str) -> i64 {
        let conn = db.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM router_model_map WHERE provider_id = ?1",
            [provider_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    // 任务书 §5.1:端口读取。
    #[test]
    fn router_port_defaults_and_parses() {
        let db = memory_db();
        assert_eq!(read_router_port(&db), DEFAULT_ROUTER_PORT);

        db.set_setting("router.port", "9000").unwrap();
        assert_eq!(read_router_port(&db), 9000);

        db.set_setting("router.port", "abc").unwrap();
        assert_eq!(read_router_port(&db), DEFAULT_ROUTER_PORT);
    }

    // 任务书 §5.2:auto 成功;manual:nonexistent 与垃圾一律 Err。
    #[test]
    fn set_router_mode_rejects_unknown_manual_target_and_garbage() {
        let db = memory_db();
        let api = RouterApi::new(db.clone());

        api.set_mode("auto").unwrap();
        assert_eq!(
            db.get_setting("router.mode").unwrap().as_deref(),
            Some("auto")
        );

        assert!(api.set_mode("manual:nonexistent").is_err());
        assert!(api.set_mode("垃圾").is_err());
        assert!(api.set_mode("manual:").is_err());
        // 库里的值不被失败的写入污染,仍是上一次成功的 auto。
        assert_eq!(
            db.get_setting("router.mode").unwrap().as_deref(),
            Some("auto")
        );
    }

    // 任务书 §5.3:manual:<存在的 id> 成功,且存进 settings 的值逐字节是 "manual:<id>"。
    #[test]
    fn set_router_mode_manual_stores_exact_value() {
        let db = memory_db();
        upsert_provider(&db, "packyapi");
        let api = RouterApi::new(db.clone());

        api.set_mode("manual:packyapi").unwrap();

        assert_eq!(
            db.get_setting("router.mode").unwrap().as_deref(),
            Some("manual:packyapi")
        );
    }

    // 任务书 §5.4:upsert 之后 list 读回全部字段(含 auth_kind 与 credential_key_id),
    // 未启用的行也要出现在列表里(设置界面要能编辑它们)。
    #[test]
    fn upsert_then_list_reads_back_every_field_including_disabled_rows() {
        let db = memory_db();
        let api = RouterApi::new(db.clone());
        let input = RouterProviderInput {
            id: "packyapi".to_string(),
            display_name: "PackyAPI".to_string(),
            base_url: "https://www.packyapi.ai/v1".to_string(),
            wire_api: "responses".to_string(),
            priority: 3,
            enabled: true,
            auth_kind: "bearer_key".to_string(),
            credential_key_id: Some("key-1".to_string()),
        };
        api.upsert_provider(input.clone()).unwrap();

        let mut disabled = input;
        disabled.id = "backup".to_string();
        disabled.priority = 5;
        disabled.enabled = false;
        disabled.auth_kind = "chatgpt_oauth".to_string();
        disabled.credential_key_id = None;
        api.upsert_provider(disabled).unwrap();

        let listed = api.list_providers().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, "packyapi");
        assert_eq!(listed[0].display_name, "PackyAPI");
        assert_eq!(listed[0].base_url, "https://www.packyapi.ai/v1");
        assert_eq!(listed[0].wire_api, "responses");
        assert_eq!(listed[0].priority, 3);
        assert!(listed[0].enabled);
        assert_eq!(listed[0].auth_kind, "bearer_key");
        assert_eq!(listed[0].credential_key_id.as_deref(), Some("key-1"));

        assert_eq!(listed[1].id, "backup");
        assert!(!listed[1].enabled);
        assert_eq!(listed[1].auth_kind, "chatgpt_oauth");
        assert_eq!(listed[1].credential_key_id, None);
    }

    // 任务书 §5.5:set_model_routes 是全量替换——先 3 条再 1 条,库里只剩 1 条。
    #[test]
    fn set_model_routes_replaces_the_whole_set() {
        let db = memory_db();
        let api = RouterApi::new(db.clone());
        upsert_provider(&db, "packyapi");

        api.set_model_routes(
            "packyapi",
            vec![route("m1", "u1"), route("m2", "u2"), route("m3", "u3")],
        )
        .unwrap();
        assert_eq!(route_count(&db, "packyapi"), 3);

        api.set_model_routes("packyapi", vec![route("m1", "u1b")])
            .unwrap();
        assert_eq!(route_count(&db, "packyapi"), 1);
        let upstream: String = {
            let conn = db.conn.lock().unwrap();
            conn.query_row(
                "SELECT upstream_model FROM router_model_map
                 WHERE provider_id = 'packyapi' AND logical_model = 'm1'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(upstream, "u1b", "替换后必须是新值,不是旧行残留");
    }

    // 任务书 §5.6:NotOurs → 写入 router.pointer_gap_since;再来一次 NotOurs
    // 不覆盖原值;变成 OursAndCurrent → 键被删除。
    #[test]
    fn pointer_gap_marker_writes_once_and_is_cleared_when_ours() {
        let db = memory_db();

        apply_pointer_gap_marker(
            &db,
            &PointerState::NotOurs {
                current: Some("packyapi".to_string()),
            },
        )
        .unwrap();
        let first = db
            .get_setting(POINTER_GAP_KEY)
            .unwrap()
            .expect("NotOurs 必须留下缺口标记");
        assert!(!first.is_empty());

        // 再来一次 NotOurs(以及 Unreadable)都不覆盖,保留最早那次。
        apply_pointer_gap_marker(
            &db,
            &PointerState::NotOurs {
                current: Some("openai".to_string()),
            },
        )
        .unwrap();
        assert_eq!(
            db.get_setting(POINTER_GAP_KEY).unwrap().as_deref(),
            Some(first.as_str())
        );
        apply_pointer_gap_marker(&db, &PointerState::Unreadable).unwrap();
        assert_eq!(
            db.get_setting(POINTER_GAP_KEY).unwrap().as_deref(),
            Some(first.as_str())
        );

        // 恢复成 OursAndCurrent → 键被删除。
        apply_pointer_gap_marker(&db, &PointerState::OursAndCurrent).unwrap();
        assert!(db.get_setting(POINTER_GAP_KEY).unwrap().is_none());
    }

    // 写侧要严:非法的 wire_api / auth_kind 进不了库。
    #[test]
    fn upsert_rejects_invalid_wire_api_and_auth_kind() {
        let db = memory_db();
        let api = RouterApi::new(db.clone());
        let base = RouterProviderInput {
            id: "packyapi".to_string(),
            display_name: "PackyAPI".to_string(),
            base_url: "https://www.packyapi.ai/v1".to_string(),
            wire_api: "responses".to_string(),
            priority: 1,
            enabled: true,
            auth_kind: "none".to_string(),
            credential_key_id: None,
        };
        let mut bad_wire = base.clone();
        bad_wire.wire_api = "grpc".to_string();
        assert!(api.upsert_provider(bad_wire).is_err());

        let mut bad_auth = base;
        bad_auth.auth_kind = "password".to_string();
        assert!(api.upsert_provider(bad_auth).is_err());

        assert!(api.list_providers().unwrap().is_empty());
    }

    // 写侧要严:给不存在的 provider 设模型映射,直接 Err,不落半截数据。
    #[test]
    fn set_model_routes_rejects_missing_provider() {
        let db = memory_db();
        let api = RouterApi::new(db.clone());

        let result = api.set_model_routes("ghost", vec![route("m1", "u1")]);

        assert!(result.is_err());
        assert_eq!(route_count(&db, "ghost"), 0);
    }

    // get_router_mode 读侧宽容:垃圾值当 auto 返回,不给前端抛错误。
    #[test]
    fn get_router_mode_tolerates_garbage() {
        let db = memory_db();
        let api = RouterApi::new(db.clone());

        assert_eq!(api.get_mode().unwrap(), "auto");
        db.set_setting("router.mode", "whatever").unwrap();
        assert_eq!(api.get_mode().unwrap(), "auto");
        db.set_setting("router.mode", "manual:packyapi").unwrap();
        assert_eq!(api.get_mode().unwrap(), "manual:packyapi");
    }

    // T12 §3:守卫判定是纯函数,直接测三种输入,不读进程级全局状态。

    /// 1. 没在监听:Err,信息里必须含配置的端口号。
    #[test]
    fn pointer_enable_guard_rejects_when_not_listening() {
        let error = pointer_enable_guard(8788, None).unwrap_err();
        assert!(
            error.contains("8788"),
            "错误信息必须含配置的端口号,实际是: {error}"
        );
    }

    /// 2. 实际监听与配置一致:Ok。
    #[test]
    fn pointer_enable_guard_passes_when_port_matches() {
        assert!(pointer_enable_guard(8788, Some(8788)).is_ok());
    }

    /// 3. 实际监听与配置不一致:Err,且信息里两个端口都必须出现——用户要能看出「配的是这个、实际是那个」。
    #[test]
    fn pointer_enable_guard_rejects_when_port_mismatches() {
        let error = pointer_enable_guard(8788, Some(9000)).unwrap_err();
        assert!(
            error.contains("8788") && error.contains("9000"),
            "错误信息必须同时含配置端口与实际监听端口,实际是: {error}"
        );
    }
}
