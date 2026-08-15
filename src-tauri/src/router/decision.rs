//! 路由决策:纯函数,无 IO。
//!
//! 给定「这个模型的候选列表」+「当前的拉黑状态」+「当前模式」,算出应该按什么
//! 顺序去试哪几家。本模块不读数据库、不发网络、不读系统时间:所有输入都是参数,
//! `now_ms` 也由调用方传入,便于测试。

use crate::store::ModelRoute;

/// 路由模式。自动:按顺序试、失败换下一家。手动:只用指定那家,不换。
///
/// **模式从哪来不归本任务管**——调用方(T6)从 `settings` 表的 `router.mode`
/// 读出字符串再转成这个枚举。本文件里**不要**出现任何读配置的代码。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteMode {
    Auto,
    Manual { provider_id: String },
}

/// 一条被拉黑的记录:某个 (provider, 模型) 在 until_ms 之前不再尝试。
/// provider 级别的拉黑用 logical_model = None 表示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blacklisted {
    pub provider_id: String,
    pub logical_model: Option<String>,
    pub until_ms: i64,
}

/// 一个候选:去哪家、把模型名换成什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub provider_id: String,
    pub upstream_model: String,
    /// 仅用于 UI 展示:这一家当前是否处于拉黑期。
    /// 手动模式下会出现 true(手动不受拉黑约束,但要让用户看见)。
    pub blacklisted: bool,
}

/// 给定模型的候选队列。**注意返回的是队列,不是"当前的 provider"**。
///
/// 这个签名是硬约束(设计文档决定 23):v1 虽然只有全局顺序,但签名必须以模型为
/// 入口。写成 `fn current_provider() -> Provider` 的话,到 v2 支持"每个模型不同
/// 顺序"时每个调用点都要改。
///
/// - `routes`:该模型在各家的映射,**调用方保证已按 priority 升序**
/// - `blacklist`:当前所有拉黑记录(不必预筛)
/// - `now_ms`:当前时刻,由调用方传入(便于测试)
pub fn candidates_for(
    routes: &[ModelRoute],
    blacklist: &[Blacklisted],
    mode: &RouteMode,
    now_ms: i64,
) -> Vec<Candidate> {
    match mode {
        RouteMode::Auto => auto_candidates(routes, blacklist, now_ms),
        RouteMode::Manual { provider_id } => {
            manual_candidates(routes, blacklist, provider_id, now_ms)
        }
    }
}

/// 一条拉黑记录此刻是否对 (provider, logical_model) 生效。
///
/// 「生效」要同时满足:未过期(`until_ms > now_ms`,等于即已过期)且命中该
/// provider——命中分两种,精确到行的(logical_model 相同)或整个 provider 的
/// (logical_model 为 None)。
fn blacklist_entry_active(
    entry: &Blacklisted,
    provider_id: &str,
    logical_model: &str,
    now_ms: i64,
) -> bool {
    entry.until_ms > now_ms
        && entry.provider_id == provider_id
        && entry
            .logical_model
            .as_deref()
            .is_none_or(|model| model == logical_model)
}

/// 自动模式:按 routes 给定的顺序返回所有不在拉黑期内的候选。
fn auto_candidates(
    routes: &[ModelRoute],
    blacklist: &[Blacklisted],
    now_ms: i64,
) -> Vec<Candidate> {
    routes
        .iter()
        .filter(|route| {
            !blacklist.iter().any(|entry| {
                blacklist_entry_active(entry, &route.provider_id, &route.logical_model, now_ms)
            })
        })
        .map(|route| Candidate {
            provider_id: route.provider_id.clone(),
            upstream_model: route.upstream_model.clone(),
            // 自动模式下拉黑的都被跳过了,留在队列里的必然是可用状态。
            blacklisted: false,
        })
        .collect()
}

/// 手动模式:只用指定的那一家,拉黑不生效,但把真实拉黑状态带给 UI 展示。
fn manual_candidates(
    routes: &[ModelRoute],
    blacklist: &[Blacklisted],
    provider_id: &str,
    now_ms: i64,
) -> Vec<Candidate> {
    let Some(route) = routes.iter().find(|route| route.provider_id == provider_id) else {
        // 该 provider 在这个模型上没有映射,没有候选可给。
        return Vec::new();
    };
    let blacklisted = blacklist
        .iter()
        .any(|entry| blacklist_entry_active(entry, provider_id, &route.logical_model, now_ms));
    vec![Candidate {
        provider_id: route.provider_id.clone(),
        upstream_model: route.upstream_model.clone(),
        blacklisted,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    // —— 测试夹具 ——

    fn route(provider_id: &str, logical_model: &str, upstream_model: &str) -> ModelRoute {
        ModelRoute {
            provider_id: provider_id.to_string(),
            logical_model: logical_model.to_string(),
            upstream_model: upstream_model.to_string(),
        }
    }

    fn row_blacklist(provider_id: &str, logical_model: &str, until_ms: i64) -> Blacklisted {
        Blacklisted {
            provider_id: provider_id.to_string(),
            logical_model: Some(logical_model.to_string()),
            until_ms,
        }
    }

    fn provider_blacklist(provider_id: &str, until_ms: i64) -> Blacklisted {
        Blacklisted {
            provider_id: provider_id.to_string(),
            logical_model: None,
            until_ms,
        }
    }

    fn candidate(provider_id: &str, upstream_model: &str, blacklisted: bool) -> Candidate {
        Candidate {
            provider_id: provider_id.to_string(),
            upstream_model: upstream_model.to_string(),
            blacklisted,
        }
    }

    fn manual_mode(provider_id: &str) -> RouteMode {
        RouteMode::Manual {
            provider_id: provider_id.to_string(),
        }
    }

    /// 固定夹具:gpt-5.6-sol 在三个 provider 上的映射,按 priority 升序。
    fn standard_routes() -> Vec<ModelRoute> {
        vec![
            route("official", "gpt-5.6-sol", "gpt-5.6-sol"),
            route("packyapi", "gpt-5.6-sol", "sol"),
            route("fallback", "gpt-5.6-sol", "sol-fb"),
        ]
    }

    fn standard_candidates() -> Vec<Candidate> {
        vec![
            candidate("official", "gpt-5.6-sol", false),
            candidate("packyapi", "sol", false),
            candidate("fallback", "sol-fb", false),
        ]
    }

    // —— 自动模式 ——

    #[test]
    fn auto_mode_without_blacklist_returns_all_candidates_in_given_order() {
        let routes = standard_routes();

        let candidates = candidates_for(&routes, &[], &RouteMode::Auto, 1_000);

        // 无拉黑:原样返回全部,顺序不变。
        assert_eq!(candidates, standard_candidates());
    }

    #[test]
    fn auto_mode_skips_provider_with_active_row_level_blacklist() {
        let routes = standard_routes();
        let blacklist = vec![row_blacklist("official", "gpt-5.6-sol", 2_000)];

        let candidates = candidates_for(&routes, &blacklist, &RouteMode::Auto, 1_000);

        // 第一家被行级拉黑,第二家排第一。
        assert_eq!(
            candidates,
            vec![
                candidate("packyapi", "sol", false),
                candidate("fallback", "sol-fb", false),
            ]
        );
    }

    #[test]
    fn auto_mode_skips_provider_with_active_provider_level_blacklist() {
        let routes = standard_routes();
        let blacklist = vec![provider_blacklist("official", 2_000)];

        let candidates = candidates_for(&routes, &blacklist, &RouteMode::Auto, 1_000);

        // provider 级拉黑(logical_model = None)同样跳过整个 provider。
        assert_eq!(
            candidates,
            vec![
                candidate("packyapi", "sol", false),
                candidate("fallback", "sol-fb", false),
            ]
        );
    }

    #[test]
    fn auto_mode_keeps_provider_whose_blacklist_expires_exactly_at_now() {
        let routes = standard_routes();
        let blacklist = vec![row_blacklist("official", "gpt-5.6-sol", 1_000)];

        let candidates = candidates_for(&routes, &blacklist, &RouteMode::Auto, 1_000);

        // 边界:until_ms == now_ms 视为已过期,不跳过。
        assert_eq!(candidates, standard_candidates());
    }

    #[test]
    fn auto_mode_skips_provider_whose_blacklist_expires_one_millisecond_after_now() {
        let routes = standard_routes();
        let blacklist = vec![row_blacklist("official", "gpt-5.6-sol", 1_001)];

        let candidates = candidates_for(&routes, &blacklist, &RouteMode::Auto, 1_000);

        // 还差 1 毫秒到期:仍在拉黑期内,跳过。
        assert_eq!(
            candidates,
            vec![
                candidate("packyapi", "sol", false),
                candidate("fallback", "sol-fb", false),
            ]
        );
    }

    #[test]
    fn auto_mode_returns_empty_queue_when_every_provider_is_blacklisted() {
        let routes = standard_routes();
        let blacklist = vec![
            row_blacklist("official", "gpt-5.6-sol", 2_000),
            row_blacklist("packyapi", "gpt-5.6-sol", 2_000),
            provider_blacklist("fallback", 2_000),
        ];

        let candidates = candidates_for(&routes, &blacklist, &RouteMode::Auto, 1_000);

        // 全部被拉黑:返回空 Vec,不是错误。
        assert!(candidates.is_empty());
    }

    #[test]
    fn auto_mode_returns_empty_queue_when_there_are_no_routes() {
        let candidates = candidates_for(&[], &[], &RouteMode::Auto, 1_000);

        assert!(candidates.is_empty());
    }

    // —— 手动模式 ——

    #[test]
    fn manual_mode_returns_only_the_selected_provider_when_it_is_not_blacklisted() {
        let routes = standard_routes();

        let candidates = candidates_for(&routes, &[], &manual_mode("packyapi"), 1_000);

        // 只返回选中的那一家,其余两家不进队列。
        assert_eq!(candidates, vec![candidate("packyapi", "sol", false)]);
    }

    #[test]
    fn manual_mode_still_returns_the_selected_provider_while_blacklisted_and_flags_it() {
        let routes = standard_routes();
        let blacklist = vec![row_blacklist("packyapi", "gpt-5.6-sol", 2_000)];

        let candidates = candidates_for(&routes, &blacklist, &manual_mode("packyapi"), 1_000);

        // 手动模式拉黑只显示不生效:仍然返回,但 blacklisted 为 true 供 UI 展示。
        assert_eq!(candidates, vec![candidate("packyapi", "sol", true)]);
    }

    #[test]
    fn manual_mode_returns_empty_queue_when_selected_provider_has_no_mapping_for_this_model() {
        let routes = standard_routes();

        let candidates = candidates_for(&routes, &[], &manual_mode("nobody"), 1_000);

        assert!(candidates.is_empty());
    }

    // —— 拉黑范围 ——

    #[test]
    fn auto_mode_ignores_blacklist_entries_for_other_logical_models() {
        let routes = standard_routes();
        // 官方在 gpt-5.5 上被拉黑,不影响它在 gpt-5.6-sol 上的可用性(决定 13)。
        let blacklist = vec![row_blacklist("official", "gpt-5.5", 2_000)];

        let candidates = candidates_for(&routes, &blacklist, &RouteMode::Auto, 1_000);

        assert_eq!(candidates, standard_candidates());
    }
}
