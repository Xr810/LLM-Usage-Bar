//! Billing-identity classification (BE-2, ADR-010).
//!
//! Runs on the inbound header map before any auth header is dropped. The
//! result is a class plus a salted fingerprint. The raw credential never
//! leaves this module.

use axum::http::HeaderMap;
use sha2::{Digest, Sha256};

use crate::policy::BillingIdentity;

const FINGERPRINT_SALT: &[u8] = b"llm-usage-bar-billing-fp-v1";
const SESSION_HEADER: &str = "x-claude-code-session-id";
const ACCOUNT_HEADER: &str = "chatgpt-account-id";

/// Approved C22 key; credential detection never creates consent.
pub const SUBSCRIPTION_TERMS_KEY: &str = "subscription_terms_ack";

/// Numeric ordering of BE-17's current terms version string "1".
pub const CURRENT_SUBSCRIPTION_TERMS_VERSION: u32 = 1;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubscriptionTermsAck {
    version: String,
    acked_at: i64,
}

pub fn subscription_terms_acknowledged(record: Option<&str>, now: i64) -> bool {
    record
        .and_then(|value| serde_json::from_str::<SubscriptionTermsAck>(value).ok())
        .is_some_and(|ack| {
            !ack.version.is_empty()
                && ack.version.bytes().all(|byte| byte.is_ascii_digit())
                && ack
                    .version
                    .parse::<u32>()
                    .is_ok_and(|version| version >= CURRENT_SUBSCRIPTION_TERMS_VERSION)
                && ack.acked_at > 0
                && ack.acked_at <= now
        })
}

/// Exact HTTPS host allowlist. Extension is empty in production until an
/// explicit trusted-subscription-relay setting is approved; never use model routes.
pub fn subscription_url_allowed(url: &str, trusted_hosts: &[String]) -> bool {
    let Ok(url) = url::Url::parse(url) else {
        return false;
    };
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && url.host_str().is_some_and(|host| {
            host == "api.anthropic.com" || trusted_hosts.iter().any(|trusted| trusted == host)
        })
}

pub fn is_anthropic_subscription(headers: &HeaderMap) -> bool {
    bearer_token(headers).is_some_and(|token| token.starts_with("sk-ant-oat"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedRequest {
    pub identity: BillingIdentity,
    pub key_fp: Option<String>,
    pub session_id: Option<String>,
    /// Subscription traffic keeps the client's auth headers. Every other
    /// class drops them and uses the stored provider credential.
    pub passthrough: bool,
}

impl ClassifiedRequest {
    /// Fields safe to write to a log line. This is the credential class and
    /// the fingerprint, never the credential itself.
    pub fn log_label(&self) -> String {
        format!(
            "billing_identity={} key_fp={}",
            identity_label(self.identity),
            self.key_fp.as_deref().unwrap_or("-")
        )
    }
}

pub fn identity_label(identity: BillingIdentity) -> &'static str {
    match identity {
        BillingIdentity::ApiKey => "api_key",
        BillingIdentity::SubscriptionOauth => "subscription_oauth",
        BillingIdentity::Unknown => "unknown",
    }
}

/// `codex_path` is true for `/v1/responses`. A bare JWT on that path is
/// subscription traffic. The same JWT on the Anthropic path stays unknown.
pub fn classify_headers(headers: &HeaderMap, codex_path: bool) -> ClassifiedRequest {
    let session_id = header_str(headers, SESSION_HEADER).map(str::to_string);
    let beta = header_str(headers, "anthropic-beta").unwrap_or("");
    let account = header_str(headers, ACCOUNT_HEADER).is_some();
    let api_key = header_str(headers, "x-api-key");
    let bearer = bearer_token(headers);

    // No credential is unknown, including an oauth-like beta header with nothing to classify.
    let (identity, secret) = if let Some(api_key) = api_key {
        (BillingIdentity::ApiKey, Some(api_key))
    } else if let Some(token) = bearer {
        (
            classify_bearer(token, beta, account, codex_path),
            Some(token),
        )
    } else {
        (BillingIdentity::Unknown, None)
    };

    ClassifiedRequest {
        passthrough: identity == BillingIdentity::SubscriptionOauth,
        key_fp: secret.map(fingerprint),
        identity,
        session_id,
    }
}

/// Headers forwarded for a subscription request. Stored provider credentials
/// are not added. API-key and unknown requests keep using the whitelist in
/// `build_upstream_headers`.
pub fn passthrough_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for name in [
        "content-type",
        "accept",
        "authorization",
        "anthropic-beta",
        "anthropic-version",
        ACCOUNT_HEADER,
    ] {
        if let Some(value) = header_str(headers, name) {
            out.push((name.to_string(), value.to_string()));
        }
    }
    out
}

fn classify_bearer(token: &str, beta: &str, account: bool, codex_path: bool) -> BillingIdentity {
    if token.starts_with("sk-ant-oat") {
        return BillingIdentity::SubscriptionOauth;
    }
    if token.starts_with("sk-") {
        return BillingIdentity::ApiKey;
    }
    if looks_like_jwt(token) {
        return if account || codex_path {
            BillingIdentity::SubscriptionOauth
        } else {
            BillingIdentity::Unknown
        };
    }
    if beta_looks_like_oauth(beta) {
        return BillingIdentity::Unknown;
    }
    BillingIdentity::ApiKey
}

fn beta_looks_like_oauth(beta: &str) -> bool {
    beta.to_ascii_lowercase().contains("oauth")
}

fn looks_like_jwt(token: &str) -> bool {
    let mut parts = token.split('.');
    let first = parts.next().filter(|part| !part.is_empty());
    let second = parts.next().filter(|part| !part.is_empty());
    let third = parts.next().filter(|part| !part.is_empty());
    first.is_some() && second.is_some() && third.is_some() && parts.next().is_none()
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    header_str(headers, "authorization").and_then(|value| {
        value
            .strip_prefix("Bearer ")
            .or_else(|| value.strip_prefix("bearer "))
            .filter(|token| !token.is_empty())
    })
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn fingerprint(secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(FINGERPRINT_SALT);
    hasher.update(secret.as_bytes());
    let digest = hasher.finalize();
    digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderName, HeaderValue};

    #[test]
    fn subscription_host_and_terms_are_explicit_and_fail_closed() {
        assert!(subscription_url_allowed(
            "https://api.anthropic.com/v1/messages",
            &[]
        ));
        for url in [
            "http://api.anthropic.com/v1/messages",
            "https://api.anthropic.com.evil.test/v1/messages",
            "https://api.anthropic.com@evil.test/v1/messages",
            "https://user@api.anthropic.com/v1/messages",
            "https://api.anthropic.com:444/v1/messages",
            "https://relay.test/v1/messages",
            "https://chatgpt.com/backend-api/codex/responses",
        ] {
            assert!(!subscription_url_allowed(url, &[]), "{url}");
        }
        assert!(subscription_url_allowed(
            "https://relay.test/v1/messages",
            &["relay.test".to_string()]
        ));
        assert!(!subscription_terms_acknowledged(None, 100));
        for record in [
            "true",
            "{}",
            "invalid",
            r#"{"version":2,"accepted":true,"acceptedAt":1}"#,
            r#"{"version":1,"accepted":false,"acceptedAt":1}"#,
            r#"{"version":1,"accepted":true,"acceptedAt":0}"#,
            r#"{"version":1,"accepted":true,"acceptedAt":101}"#,
        ] {
            assert!(!subscription_terms_acknowledged(Some(record), 100));
        }
        assert!(!subscription_terms_acknowledged(
            Some(r#"{"version":1,"accepted":true,"acceptedAt":100}"#),
            100
        ));
        assert!(!subscription_terms_acknowledged(
            Some(r#"{"version":1,"ackedAt":100}"#),
            100
        ));
        for record in [
            r#"{"version":"0","ackedAt":100}"#,
            r#"{"version":"r16-2026-10-25","ackedAt":100}"#,
            r#"{"version":"1","ackedAt":0}"#,
            r#"{"version":"1","ackedAt":101}"#,
            r#"{"version":"1","ackedAt":"100"}"#,
        ] {
            assert!(!subscription_terms_acknowledged(Some(record), 100));
        }
        for record in [
            r#"{"version":"1","ackedAt":100}"#,
            r#"{"version":"2","ackedAt":1}"#,
        ] {
            assert!(subscription_terms_acknowledged(Some(record), 100));
        }
    }

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                HeaderName::try_from(*name).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    #[test]
    fn claude_and_codex_samples_cover_the_three_classes() {
        let api_key = "sk-ant-api03-example-not-a-real-key";
        let oat = "sk-ant-oat01-example-not-a-real-token";
        let gateway = "gateway-token-example";
        let jwt = "aaa.bbb.ccc";

        let cases = [
            (
                headers(&[("x-api-key", api_key)]),
                false,
                BillingIdentity::ApiKey,
            ),
            (
                headers(&[
                    ("x-api-key", api_key),
                    ("authorization", &format!("Bearer {oat}")),
                ]),
                false,
                BillingIdentity::ApiKey,
            ),
            (
                headers(&[("authorization", &format!("Bearer {api_key}"))]),
                false,
                BillingIdentity::ApiKey,
            ),
            (
                headers(&[("authorization", &format!("Bearer {gateway}"))]),
                false,
                BillingIdentity::ApiKey,
            ),
            (
                headers(&[("authorization", "Bearer sk-proj-example")]),
                true,
                BillingIdentity::ApiKey,
            ),
            (
                headers(&[("authorization", &format!("Bearer {oat}"))]),
                false,
                BillingIdentity::SubscriptionOauth,
            ),
            (
                headers(&[
                    ("authorization", &format!("Bearer {jwt}")),
                    ("chatgpt-account-id", "acct_example"),
                ]),
                false,
                BillingIdentity::SubscriptionOauth,
            ),
            (
                headers(&[("authorization", &format!("Bearer {jwt}"))]),
                true,
                BillingIdentity::SubscriptionOauth,
            ),
            (
                headers(&[("authorization", &format!("Bearer {jwt}"))]),
                false,
                BillingIdentity::Unknown,
            ),
            (
                headers(&[
                    ("authorization", "Bearer not-a-known-prefix"),
                    ("anthropic-beta", "oauth-2025-04-20"),
                ]),
                false,
                BillingIdentity::Unknown,
            ),
            (
                headers(&[("anthropic-beta", "oauth-2025-04-20")]),
                false,
                BillingIdentity::Unknown,
            ),
            (headers(&[]), false, BillingIdentity::Unknown),
        ];

        for (index, (map, codex_path, expected)) in cases.into_iter().enumerate() {
            let classified = classify_headers(&map, codex_path);
            assert_eq!(classified.identity, expected, "case {index}");
            assert_eq!(
                classified.passthrough,
                expected == BillingIdentity::SubscriptionOauth,
                "case {index}"
            );
        }
    }

    #[test]
    fn fingerprint_and_log_label_do_not_contain_the_credential() {
        let secret = "sk-ant-api03-example-not-a-real-key";
        let classified = classify_headers(&headers(&[("x-api-key", secret)]), false);
        let fp = classified.key_fp.as_deref().unwrap();
        assert_ne!(fp, secret);
        assert!(!fp.contains("sk-ant"));
        assert_eq!(fp.len(), 16);
        let label = classified.log_label();
        assert!(!label.contains(secret));
        assert!(label.contains("billing_identity=api_key"));
        assert!(label.contains(fp));
    }

    #[test]
    fn passthrough_keeps_subscription_headers_and_drops_cookies() {
        let map = headers(&[
            ("authorization", "Bearer sk-ant-oat01-example"),
            ("anthropic-beta", "oauth-2025-04-20"),
            ("anthropic-version", "2023-06-01"),
            ("chatgpt-account-id", "acct_example"),
            ("content-type", "application/json"),
            ("cookie", "session=secret"),
            ("x-api-key", "should-not-be-forwarded-on-this-list"),
        ]);
        let forwarded = passthrough_headers(&map);
        let names: Vec<&str> = forwarded.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "content-type",
                "authorization",
                "anthropic-beta",
                "anthropic-version",
                "chatgpt-account-id"
            ]
        );
        assert!(!names.contains(&"cookie"));
        assert!(!names.contains(&"x-api-key"));
    }

    #[test]
    fn session_id_comes_from_the_claude_code_header() {
        let classified = classify_headers(
            &headers(&[
                ("x-claude-code-session-id", "sess-1"),
                ("x-api-key", "sk-ant-api03-example"),
            ]),
            false,
        );
        assert_eq!(classified.session_id.as_deref(), Some("sess-1"));
    }
}
