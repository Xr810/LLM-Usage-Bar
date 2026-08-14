//! 本地 router 的 Tauri 命令(T10):把 T3–T9 造好的零件暴露给前端,
//! 并承载启动时的指针缺口标记逻辑。
//!
//! 安全面:输入结构体里**绝不出现凭据明文**——`credential_key_id` 只是指向
//! `provider_api_keys.id` 的引用,真正的 key 走仓库既有的凭据命令;命令返回值
//! 里也绝不回传凭据明文。指针的读写在 `router::pointer` 里实现,这里只是它的
//! 调用方。

use serde::{Deserialize, Serialize};

use crate::database::{lock_conn, Database, ModelRoute, RouterAuthKind, RouterProvider, WireApi};
use crate::error::AppError;
use crate::router::pointer::PointerState;
use crate::store::AppState;

/// router 默认监听端口(settings 里没有 `router.port` 或解析不了时用)。
pub const DEFAULT_ROUTER_PORT: u16 = 8788;

/// 指针缺口标记的 settings 键(决定 36):从这个 epoch 毫秒起,用量可能没经过
/// router,受影响 provider 的估算不完整。
pub const POINTER_GAP_KEY: &str = "router.pointer_gap_since";

/// 供设置界面编辑的 provider 行视图。`auth_kind` / `credential_key_id` 只是
/// 「凭据从哪来」的描述,不是凭据本身。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterProviderView {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub wire_api: String,
    pub priority: i64,
    pub enabled: bool,
    pub auth_kind: String,
    pub credential_key_id: Option<String>,
}

/// 前端提交的 provider 行。**绝不能加 api_key / token / secret 这类字段**:
/// 前端存 key 走仓库既有的凭据命令,router 这边只认 `credential_key_id` 引用。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
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

/// 一条模型映射(全量替换的输入单元)。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRouteInput {
    pub logical_model: String,
    pub upstream_model: String,
}

/// 指针当前状态,供界面显示「已接管 / 未接管 / 读不到」。
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

fn wire_api_from_input(raw: &str) -> Result<WireApi, AppError> {
    match raw {
        "responses" => Ok(WireApi::Responses),
        "chat_completions" => Ok(WireApi::ChatCompletions),
        other => Err(AppError::InvalidInput(format!("非法 wire_api: {other}"))),
    }
}

fn auth_kind_from_input(raw: &str) -> Result<RouterAuthKind, AppError> {
    match raw {
        "chatgpt_oauth" => Ok(RouterAuthKind::ChatgptOauth),
        "bearer_key" => Ok(RouterAuthKind::BearerKey),
        "none" => Ok(RouterAuthKind::None),
        other => Err(AppError::InvalidInput(format!("非法 auth_kind: {other}"))),
    }
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

/// provider 行是否存在(不区分启用状态)。`set_router_mode` 与 `set_model_routes`
/// 的写侧校验共用。
fn router_provider_exists(db: &Database, id: &str) -> Result<bool, AppError> {
    let conn = lock_conn!(db.conn);
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM router_providers WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .map_err(AppError::from)?;
    Ok(count > 0)
}

/// 列出全部 provider(含未启用的)。
///
/// DAO 的 `list_router_providers` 是给转发层用的、过滤 `enabled = 1`;
/// 设置界面要编辑未启用的行,所以这里直接查表,不借那个入口。
pub(crate) fn list_router_providers_impl(
    db: &Database,
) -> Result<Vec<RouterProviderView>, AppError> {
    let conn = lock_conn!(db.conn);
    let mut statement = conn.prepare(
        "SELECT id, display_name, base_url, wire_api, priority, enabled,
                auth_kind, credential_key_id
         FROM router_providers
         ORDER BY priority ASC, id ASC",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(RouterProviderView {
                id: row.get(0)?,
                display_name: row.get(1)?,
                base_url: row.get(2)?,
                wire_api: row.get(3)?,
                priority: row.get(4)?,
                enabled: row.get(5)?,
                auth_kind: row.get(6)?,
                credential_key_id: row.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 新增或更新一个 provider。`wire_api` / `auth_kind` 在入口处校验,非法值进不了库。
pub(crate) fn upsert_router_provider_impl(
    db: &Database,
    input: &RouterProviderInput,
) -> Result<(), AppError> {
    db.upsert_router_provider(&RouterProvider {
        id: input.id.clone(),
        display_name: input.display_name.clone(),
        base_url: input.base_url.clone(),
        wire_api: wire_api_from_input(&input.wire_api)?,
        priority: input.priority,
        enabled: input.enabled,
        auth_kind: auth_kind_from_input(&input.auth_kind)?,
        credential_key_id: input.credential_key_id.clone(),
    })
}

/// 某个 provider 的模型映射全量替换:先清空再插入(任务书 §3)。
///
/// provider 不存在直接 Err:外键约束只会给一句生硬的 constraint 错误,
/// 写侧明确校验,错误信息才对得上(任务书 §3.2「写侧要严」)。
pub(crate) fn set_model_routes_impl(
    db: &Database,
    provider_id: &str,
    routes: &[ModelRouteInput],
) -> Result<(), AppError> {
    if !router_provider_exists(db, provider_id)? {
        return Err(AppError::InvalidInput(format!(
            "router provider 不存在: {provider_id}"
        )));
    }
    db.delete_model_routes_for_provider(provider_id)?;
    for route in routes {
        db.upsert_model_route(&ModelRoute {
            provider_id: provider_id.to_string(),
            logical_model: route.logical_model.clone(),
            upstream_model: route.upstream_model.clone(),
        })?;
    }
    Ok(())
}

/// 读 `router.mode`。与 T6 转发层的读侧同样宽容:读不到或解析不了都当 `"auto"`,
/// 返回给前端的永远是合法形态(垃圾值由读侧容错,写侧见 `set_router_mode_impl`)。
pub(crate) fn get_router_mode_impl(db: &Database) -> Result<String, AppError> {
    let raw = db.get_setting("router.mode")?;
    Ok(match raw.as_deref() {
        Some("auto") | None => "auto".to_string(),
        Some(manual) => match manual.strip_prefix("manual:") {
            Some(provider_id) if !provider_id.is_empty() => manual.to_string(),
            _ => "auto".to_string(),
        },
    })
}

/// 写 `router.mode`,写侧要严(任务书 §3.2):
/// - `"auto"` 直接存;
/// - `"manual:<id>"` 要 `<id>` 在 `router_providers` 里存在;
/// - 其余一律 Err,垃圾进不了库。
pub(crate) fn set_router_mode_impl(db: &Database, mode: &str) -> Result<(), AppError> {
    if mode == "auto" {
        return db.set_setting("router.mode", "auto");
    }
    match mode.strip_prefix("manual:") {
        Some(provider_id) if !provider_id.is_empty() => {
            if !router_provider_exists(db, provider_id)? {
                return Err(AppError::InvalidInput(format!(
                    "manual 模式指向的 router provider 不存在: {provider_id}"
                )));
            }
            db.set_setting("router.mode", mode)
        }
        _ => Err(AppError::InvalidInput(format!("非法 router.mode: {mode}"))),
    }
}

/// 把指针状态落成 settings 缺口标记(决定 36):
/// - `NotOurs` / `Unreadable` → 记 `router.pointer_gap_since` = 发现时的 epoch
///   毫秒;**已有值不覆盖**(保留最早那次缺口);
/// - `OursAndCurrent` → 删掉标记。
///
/// 只在启动时调用(启动只读指针、绝不写,决定 34/36);`enable_router_pointer`
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

/// 列出全部 router provider(含未启用的,供设置界面编辑)。
#[tauri::command]
pub async fn list_router_providers(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<RouterProviderView>, String> {
    list_router_providers_impl(state.db.as_ref()).map_err(|e| e.to_string())
}

/// 新增或更新一个 provider。
#[tauri::command]
pub async fn upsert_router_provider(
    state: tauri::State<'_, AppState>,
    input: RouterProviderInput,
) -> Result<(), String> {
    upsert_router_provider_impl(state.db.as_ref(), &input).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_router_provider(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state
        .db
        .delete_router_provider(&id)
        .map_err(|e| e.to_string())
}

/// 某个 provider 的模型映射,全量替换。
#[tauri::command]
pub async fn set_model_routes(
    state: tauri::State<'_, AppState>,
    provider_id: String,
    routes: Vec<ModelRouteInput>,
) -> Result<(), String> {
    set_model_routes_impl(state.db.as_ref(), &provider_id, &routes).map_err(|e| e.to_string())
}

/// 读路由模式。值是 `"auto"` 或 `"manual:<provider_id>"`。
#[tauri::command]
pub async fn get_router_mode(state: tauri::State<'_, AppState>) -> Result<String, String> {
    get_router_mode_impl(state.db.as_ref()).map_err(|e| e.to_string())
}

/// 写路由模式。写侧要严:`manual:<id>` 必须指向存在的 provider。
#[tauri::command]
pub async fn set_router_mode(
    state: tauri::State<'_, AppState>,
    mode: String,
) -> Result<(), String> {
    set_router_mode_impl(state.db.as_ref(), &mode).map_err(|e| e.to_string())
}

/// 指针当前状态,供界面显示「已接管 / 未接管 / 读不到」。只读,绝不写。
#[tauri::command]
pub async fn inspect_router_pointer() -> Result<PointerStateView, String> {
    Ok(match crate::router::pointer::inspect_pointer() {
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
    })
}

/// 写指针前的守卫判定(纯函数,不读全局状态)。
///
/// `listening` 由调用方传入:进程级的 LISTENING_PORT 是全局状态,测试之间会
/// 互相污染,判定抽成纯函数才能用纯输入测。两种情况分别给用户能执行的下一步:
/// - 没监听:最常见的原因是端口被占(上一次 app 没退干净),换端口或重启 app;
/// - 端口不一致:改过 router.port 但没重启 app,重启后实际监听才对得上。
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

/// 用户显式点「启用」时调用:写一次指针(决定 34,之后永不再改)。
/// 写之前先确认 router 真的在监听——端口被占时 router 没起来,把指针写进去
/// 会让 Codex 每个请求都变成「连接被拒」,而指针按设计不能再改回去。
/// 写入成功后清掉缺口标记——指针刚被自己接管,缺口到此结束。
#[tauri::command]
pub async fn enable_router_pointer(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let port = read_router_port(state.db.as_ref());
    pointer_enable_guard(port, crate::router::server::listening_port())?;
    crate::router::pointer::point_codex_at_router(port).map_err(|e| e.to_string())?;
    if let Err(error) = apply_pointer_gap_marker(state.db.as_ref(), &PointerState::OursAndCurrent) {
        // 指针已经写成功,标记清不掉只是 UI 上缺口提示还在,不把成功改成失败。
        log::warn!("指针已写入,但清除缺口标记失败: {error}");
    }
    Ok(())
}

/// 最近的路由尝试,按 provider 汇总,供分账面板。时间范围是半开区间。
#[tauri::command]
pub async fn recent_router_attempts(
    state: tauri::State<'_, AppState>,
    start_at: i64,
    end_at: i64,
) -> Result<Vec<RouterUsageSummaryView>, String> {
    state
        .db
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
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> Database {
        Database::memory().unwrap()
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

        set_router_mode_impl(&db, "auto").unwrap();
        assert_eq!(
            db.get_setting("router.mode").unwrap().as_deref(),
            Some("auto")
        );

        assert!(set_router_mode_impl(&db, "manual:nonexistent").is_err());
        assert!(set_router_mode_impl(&db, "垃圾").is_err());
        assert!(set_router_mode_impl(&db, "manual:").is_err());
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

        set_router_mode_impl(&db, "manual:packyapi").unwrap();

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
        upsert_router_provider_impl(&db, &input).unwrap();

        let mut disabled = input.clone();
        disabled.id = "backup".to_string();
        disabled.priority = 5;
        disabled.enabled = false;
        disabled.auth_kind = "chatgpt_oauth".to_string();
        disabled.credential_key_id = None;
        upsert_router_provider_impl(&db, &disabled).unwrap();

        let listed = list_router_providers_impl(&db).unwrap();
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
        upsert_provider(&db, "packyapi");

        set_model_routes_impl(
            &db,
            "packyapi",
            &[route("m1", "u1"), route("m2", "u2"), route("m3", "u3")],
        )
        .unwrap();
        assert_eq!(route_count(&db, "packyapi"), 3);

        set_model_routes_impl(&db, "packyapi", &[route("m1", "u1b")]).unwrap();
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
        assert!(upsert_router_provider_impl(&db, &bad_wire).is_err());

        let mut bad_auth = base;
        bad_auth.auth_kind = "password".to_string();
        assert!(upsert_router_provider_impl(&db, &bad_auth).is_err());

        assert!(list_router_providers_impl(&db).unwrap().is_empty());
    }

    // 写侧要严:给不存在的 provider 设模型映射,直接 Err,不落半截数据。
    #[test]
    fn set_model_routes_rejects_missing_provider() {
        let db = memory_db();

        let result = set_model_routes_impl(&db, "ghost", &[route("m1", "u1")]);

        assert!(result.is_err());
        assert_eq!(route_count(&db, "ghost"), 0);
    }

    // get_router_mode 读侧宽容:垃圾值当 auto 返回,不给前端抛错误。
    #[test]
    fn get_router_mode_tolerates_garbage() {
        let db = memory_db();

        assert_eq!(get_router_mode_impl(&db).unwrap(), "auto");
        db.set_setting("router.mode", "whatever").unwrap();
        assert_eq!(get_router_mode_impl(&db).unwrap(), "auto");
        db.set_setting("router.mode", "manual:packyapi").unwrap();
        assert_eq!(get_router_mode_impl(&db).unwrap(), "manual:packyapi");
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
