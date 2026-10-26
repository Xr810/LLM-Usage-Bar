//! 契约 C1：`PolicyGate`。契约 C7 的 `ReqMeta` 是 admit 的入参。

use serde_json::Value;

/// 经代理请求的计费身份。枚举值用 snake_case（契约 C7）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BillingIdentity {
    ApiKey,
    SubscriptionOauth,
    Unknown,
}

/// 代理在丢认证头之前交给策略门的请求元数据（契约 C7）。
///
/// 字段保持契约里的 snake_case。这是 Rust trait 的入参，不是 command 载荷；
/// 真正过 IPC 的载荷按 R-2 用 camelCase。`key_fp` 只放加盐指纹，不放 Key 明文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReqMeta {
    pub req_id: String,
    pub app_type: String,
    pub provider_id: String,
    pub model: String,
    pub billing_identity: BillingIdentity,
    pub key_fp: Option<String>,
    pub session_id: Option<String>,
    /// 预估费用，微美元整数（ADR §6.3）。command 载荷里的金额仍用十进制字符串。
    pub est_cost_micro_usd: i64,
    pub via_subscription_optin: bool,
}

/// [`PolicyGate::settle`] 使用的已结算用量。
///
/// 只表达计量结果。订阅伪美元不进这个结构。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cost_micro_usd: i64,
}

/// 策略门的决定（契约 C1）。没有单独的 Warn 变体：社区版的「只告警」是
/// 恒为 [`Decision::Allow`]，告警仍走现有预算告警，不在这个枚举里加变体。
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Allow,
    Deny {
        rule: String,
        status: u16,
        body: Value,
    },
    /// BE-30 fails closed immediately with HTTP 429 / GUARD_HOLD; no waiting.
    /// PRO-2 may extend this response with interactive Hold handling later.
    Hold {
        reason: String,
        /// Optional retry delay in seconds, also emitted as Retry-After.
        retry_after_seconds: Option<u64>,
    },
}

/// 请求转发前的策略门（契约 C1）。
///
/// `admit` 借用 [`ReqMeta`]，`settle` 借用 [`Usage`]，这样 trait 可以放进
/// `Box<dyn PolicyGate>`。社区实现是 [`AllowAllGate`]。拒绝、挂起和 Kill
/// 由私有 crate 实现，不写在这里。
pub trait PolicyGate: Send + Sync {
    /// 放行、拒绝或挂起。社区实现恒为 [`Decision::Allow`]。
    fn admit(&self, req: &ReqMeta) -> Decision;

    /// 用响应里的用量结算 `req_id`。社区实现不落库。
    /// Record only: the request has already been sent. Settlement must never
    /// replace or otherwise alter the upstream response.
    fn settle(&self, req_id: &str, usage: &Usage);
}

/// 社区构建的策略门：只放行，不拒绝、不挂起、不写状态。
///
/// 预算告警留在现有告警路径。这个类型没有字段，`admit` / `settle` 不能记录
/// Key、不能改配置、也不能改数据库。
#[derive(Debug, Default, Clone, Copy)]
pub struct AllowAllGate;

impl PolicyGate for AllowAllGate {
    fn admit(&self, _req: &ReqMeta) -> Decision {
        Decision::Allow
    }

    fn settle(&self, _req_id: &str, _usage: &Usage) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(identity: BillingIdentity) -> ReqMeta {
        ReqMeta {
            req_id: "req-1".to_string(),
            app_type: "claude".to_string(),
            provider_id: "anthropic".to_string(),
            model: "claude-sonnet".to_string(),
            billing_identity: identity,
            key_fp: Some("fp-not-a-key".to_string()),
            session_id: None,
            est_cost_micro_usd: 1_000_000,
            via_subscription_optin: false,
        }
    }

    #[test]
    fn allow_all_gate_admits_every_billing_identity_without_state() {
        let gate = AllowAllGate;
        let usage = Usage {
            input_tokens: 3,
            output_tokens: 4,
            cache_read_tokens: 1,
            cache_creation_tokens: 0,
            cost_micro_usd: 12,
        };
        for identity in [
            BillingIdentity::ApiKey,
            BillingIdentity::SubscriptionOauth,
            BillingIdentity::Unknown,
        ] {
            let req = sample(identity);
            assert_eq!(gate.admit(&req), Decision::Allow);
            gate.settle(&req.req_id, &usage);
            assert_eq!(gate.admit(&req), Decision::Allow);
        }
    }

    #[test]
    fn decision_keeps_deny_and_hold_for_the_private_impl() {
        let deny = Decision::Deny {
            rule: "daily".to_string(),
            status: 429,
            body: serde_json::json!({ "error": "budget_exceeded" }),
        };
        let hold = Decision::Hold {
            reason: "hard_cap".to_string(),
            retry_after_seconds: None,
        };
        assert_ne!(deny, Decision::Allow);
        assert_ne!(hold, Decision::Allow);
    }
}
