//! 上游请求失败分类(纯函数,无 IO)。
//!
//! T6 把一次上游尝试的结果提取成 [`AttemptResult`] 后交给 [`classify`],
//! 得到「算不算失败、要不要换下一家、要不要拉黑、拉黑谁、拉多久」的判定。
//! 本模块不读数据库、不发网络、不读系统时间:输入全是参数,输出只是一个判定。

/// 一次上游尝试的结果，由调用方(T6)从真实响应里提取。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttemptResult {
    /// 拿到了 HTTP 响应。
    Http { status: u16, body_snippet: String },
    /// 连不上、DNS 失败、TLS 失败。
    ConnectFailed,
    /// 连上了但迟迟没有第一个字节。
    FirstByteTimeout,
    /// 流已经开始吐字之后断了。
    StreamBroken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Connect,
    Timeout,
    RateLimited,
    ServerError,
    ModelNotFound,
    /// 认证错、参数错、内容被拒等——换一家也是同样的错。
    RequestRejected,
    StreamBroken,
}

/// 拉黑的作用范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlacklistScope {
    /// 只拉黑 (这个 provider, 这个模型) 这一行。
    ThisRoute,
    /// 拉黑整个 provider——只用于与模型无关的故障。
    WholeProvider,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureVerdict {
    pub kind: FailureKind,
    /// 是否应当继续尝试队列里的下一家。
    pub try_next: bool,
    /// 是否拉黑；None 表示不拉黑。
    pub blacklist: Option<BlacklistScope>,
    /// 拉黑时长（秒）。仅当 blacklist 为 Some 时有意义。
    pub cooldown_secs: i64,
}

/// 成功返回 None，失败返回判定。
///
/// `already_streaming`：本次请求是否已经向客户端吐过字。
pub fn classify(result: &AttemptResult, already_streaming: bool) -> Option<FailureVerdict> {
    // 判定表按编号顺序匹配，先命中先返回：429 和 404 都落在 4xx 区间里，
    // 顺序错了就归错类。
    let (kind, try_next, blacklist, cooldown) = match result {
        AttemptResult::ConnectFailed => {
            // 连不上与模型无关，拉黑整个 provider。
            (
                FailureKind::Connect,
                true,
                Some(BlacklistScope::WholeProvider),
                60,
            )
        }
        AttemptResult::FirstByteTimeout => {
            // 超时同样与模型无关，拉黑整个 provider。
            (
                FailureKind::Timeout,
                true,
                Some(BlacklistScope::WholeProvider),
                60,
            )
        }
        AttemptResult::StreamBroken => {
            // 已经吐字之后断流：错误只能透传给客户端，换家会让用户看到两截拼
            // 在一起的话；这也不是「这家坏了」的证据，不拉黑。
            (FailureKind::StreamBroken, false, None, 0)
        }
        AttemptResult::Http {
            status,
            body_snippet,
        } => {
            // 2xx 全算成功。
            if (200..=299).contains(status) {
                return None;
            }
            if *status == 429 {
                (
                    FailureKind::RateLimited,
                    true,
                    Some(BlacklistScope::ThisRoute),
                    60,
                )
            } else if *status == 404 || (*status == 400 && body_hints_model_not_found(body_snippet))
            {
                // 「这家没有这个模型」是稳定事实，拉黑 600 秒而不是 60 秒，
                // 否则每分钟重试等于反复撞同一堵墙；但也不永久拉黑——中转
                // 随时可能上新模型。
                (
                    FailureKind::ModelNotFound,
                    true,
                    Some(BlacklistScope::ThisRoute),
                    600,
                )
            } else if (400..=499).contains(status) {
                // 认证错、参数错、内容被拒——换一家只是把同一个错再挨一遍，
                // 还多等一轮（设计文档决定 14）。
                (FailureKind::RequestRejected, false, None, 0)
            } else if (500..=599).contains(status) {
                (
                    FailureKind::ServerError,
                    true,
                    Some(BlacklistScope::ThisRoute),
                    60,
                )
            } else {
                // 1xx / 3xx / ≥600：上游正常工作时不该出现。3xx 漏到这里说明
                // 配置错了（base_url 指到了一个会重定向的地方），换一家不会
                // 更好，拉黑还会掩盖问题，所以归到「换一家也是同样的错」，
                // 原样透传给客户端让用户看见。
                (FailureKind::RequestRejected, false, None, 0)
            }
        }
    };

    Some(FailureVerdict {
        kind,
        // 决定 15 的全局压制：已经吐过字就不能换下一家，否则用户会看到半句
        // 加另一句拼在一起。这只压制 try_next，拉黑照常——下一次请求应当
        // 跳过这家。
        try_next: try_next && !already_streaming,
        blacklist,
        cooldown_secs: cooldown,
    })
}

/// body 是否带有「这家没有这个模型」的迹象。
///
/// 只在 status 是 400 或 404 时才调用（判定表第 6 行）；大小写不敏感。
/// 关键词清单见任务书 §3.1，全部是小写或大小写无关的中文，因此只把 body
/// 转小写一次再逐个比对。
fn body_hints_model_not_found(body: &str) -> bool {
    let lower = body.to_lowercase();
    [
        "model_not_found",
        "does not exist",
        "unknown model",
        "no such model",
        "不存在",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(status: u16, body: &str) -> AttemptResult {
        AttemptResult::Http {
            status,
            body_snippet: body.to_string(),
        }
    }

    /// 未吐字时的判定（`already_streaming = false`）。
    fn classify_normal(result: AttemptResult) -> Option<FailureVerdict> {
        classify(&result, false)
    }

    /// 已吐字时的判定（`already_streaming = true`）。
    fn classify_streaming(result: AttemptResult) -> Option<FailureVerdict> {
        classify(&result, true)
    }

    #[test]
    fn http_200_is_success() {
        assert_eq!(classify_normal(http(200, "ok")), None);
    }

    #[test]
    fn http_204_is_success() {
        // 边界：2xx 全算成功，204 不带 body 也一样。
        assert_eq!(classify_normal(http(204, "")), None);
    }

    #[test]
    fn connect_failed_blacklists_whole_provider_for_60s() {
        let verdict = classify_normal(AttemptResult::ConnectFailed).expect("连接失败应有判定");
        assert_eq!(verdict.kind, FailureKind::Connect);
        assert!(verdict.try_next);
        assert_eq!(verdict.blacklist, Some(BlacklistScope::WholeProvider));
        assert_eq!(verdict.cooldown_secs, 60);
    }

    #[test]
    fn first_byte_timeout_blacklists_whole_provider_for_60s() {
        let verdict = classify_normal(AttemptResult::FirstByteTimeout).expect("首字节超时应有判定");
        assert_eq!(verdict.kind, FailureKind::Timeout);
        assert!(verdict.try_next);
        assert_eq!(verdict.blacklist, Some(BlacklistScope::WholeProvider));
        assert_eq!(verdict.cooldown_secs, 60);
    }

    #[test]
    fn http_429_is_rate_limited_and_blacklists_this_route() {
        let verdict = classify_normal(http(429, "")).expect("429 应有判定");
        assert_eq!(verdict.kind, FailureKind::RateLimited);
        assert!(verdict.try_next);
        assert_eq!(verdict.blacklist, Some(BlacklistScope::ThisRoute));
    }

    #[test]
    fn http_500_and_503_are_server_errors_and_blacklist_this_route() {
        for status in [500, 503] {
            let verdict = classify_normal(http(status, "")).expect("5xx 应有判定");
            assert_eq!(verdict.kind, FailureKind::ServerError);
            assert!(verdict.try_next);
            assert_eq!(verdict.blacklist, Some(BlacklistScope::ThisRoute));
        }
    }

    #[test]
    fn http_404_is_model_not_found_blacklisting_this_route_for_600s() {
        let verdict = classify_normal(http(404, "")).expect("404 应有判定");
        assert_eq!(verdict.kind, FailureKind::ModelNotFound);
        assert!(verdict.try_next);
        assert_eq!(verdict.blacklist, Some(BlacklistScope::ThisRoute));
        assert_eq!(verdict.cooldown_secs, 600);
    }

    #[test]
    fn http_400_with_model_not_found_body_is_model_not_found() {
        let verdict = classify_normal(http(400, r#"{"error": "model_not_found"}"#))
            .expect("400 + 模型不存在 body 应有判定");
        assert_eq!(verdict.kind, FailureKind::ModelNotFound);
    }

    #[test]
    fn http_400_with_mixed_case_does_not_exist_body_is_model_not_found() {
        // 大小写混杂也必须命中（大小写不敏感）。
        let verdict = classify_normal(http(400, "The model does NOT Exist"))
            .expect("大小写混杂的模型不存在 body 应有判定");
        assert_eq!(verdict.kind, FailureKind::ModelNotFound);
    }

    #[test]
    fn http_400_with_unrelated_body_is_request_rejected() {
        let verdict = classify_normal(http(400, r#"{"error": "bad request"}"#))
            .expect("400 + 其他错误 body 应有判定");
        assert_eq!(verdict.kind, FailureKind::RequestRejected);
        assert!(!verdict.try_next);
        assert_eq!(verdict.blacklist, None);
    }

    #[test]
    fn http_401_is_request_rejected_without_failover() {
        let verdict = classify_normal(http(401, "")).expect("401 应有判定");
        assert_eq!(verdict.kind, FailureKind::RequestRejected);
        assert!(!verdict.try_next);
    }

    #[test]
    fn http_403_is_request_rejected_without_failover() {
        let verdict = classify_normal(http(403, "")).expect("403 应有判定");
        assert_eq!(verdict.kind, FailureKind::RequestRejected);
        assert!(!verdict.try_next);
    }

    #[test]
    fn http_500_while_streaming_does_not_fail_over_but_still_blacklists() {
        let verdict = classify_streaming(http(500, "")).expect("500 应有判定");
        assert!(!verdict.try_next);
        assert_eq!(verdict.blacklist, Some(BlacklistScope::ThisRoute));
    }

    #[test]
    fn http_429_while_streaming_does_not_fail_over_but_still_blacklists() {
        let verdict = classify_streaming(http(429, "")).expect("429 应有判定");
        assert!(!verdict.try_next);
        assert_eq!(verdict.blacklist, Some(BlacklistScope::ThisRoute));
    }

    #[test]
    fn stream_broken_does_not_fail_over_or_blacklist() {
        let verdict = classify_normal(AttemptResult::StreamBroken).expect("断流应有判定");
        assert_eq!(verdict.kind, FailureKind::StreamBroken);
        assert!(!verdict.try_next);
        assert_eq!(verdict.blacklist, None);
    }

    #[test]
    fn http_404_is_model_not_found_even_without_keywords_in_body() {
        // 404 本身就够，不要求 body 里有任何关键词。
        let verdict =
            classify_normal(http(404, r#"{"error": "not found"}"#)).expect("404 应有判定");
        assert_eq!(verdict.kind, FailureKind::ModelNotFound);
    }

    #[test]
    fn http_302_is_request_rejected_without_failover_or_blacklist() {
        // 判定表第 9 行：3xx 漏到这里说明配置错了，换一家不会更好。
        let verdict = classify_normal(http(302, "")).expect("302 应有判定");
        assert_eq!(verdict.kind, FailureKind::RequestRejected);
        assert!(!verdict.try_next);
        assert_eq!(verdict.blacklist, None);
    }

    #[test]
    fn http_100_is_request_rejected_without_failover_or_blacklist() {
        // 判定表第 9 行同样覆盖 1xx。
        let verdict = classify_normal(http(100, "")).expect("100 应有判定");
        assert_eq!(verdict.kind, FailureKind::RequestRejected);
        assert!(!verdict.try_next);
        assert_eq!(verdict.blacklist, None);
    }

    #[test]
    fn connect_failed_while_streaming_does_not_fail_over_but_still_blacklists() {
        // already_streaming 的压制是全局的，不只作用于 HTTP 分支：
        // 已经吐字就不能换下一家，但该拉黑还是要拉黑。
        let verdict = classify_streaming(AttemptResult::ConnectFailed).expect("连接失败应有判定");
        assert!(!verdict.try_next);
        assert_eq!(verdict.blacklist, Some(BlacklistScope::WholeProvider));
        assert_eq!(verdict.cooldown_secs, 60);
    }
}
