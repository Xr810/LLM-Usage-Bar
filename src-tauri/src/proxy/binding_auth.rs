use crate::credentials::SecretString;
use http::{header::AUTHORIZATION, uri::PathAndQuery, HeaderMap, HeaderValue, Uri};
use subtle::ConstantTimeEq;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BindingAuthProtocol {
    Claude,
    Codex,
    Gemini,
    ClaudeDesktop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("binding authorization failed")]
pub(crate) struct BindingAuthorizationFailed;

pub(crate) struct InboundBindingCredentials {
    pub(crate) binding_key: SecretString,
    pub(crate) gateway_token: Option<SecretString>,
}

pub(crate) fn extract_and_strip_binding_auth(
    protocol: BindingAuthProtocol,
    headers: &mut HeaderMap,
    uri: &mut Uri,
    endpoint: &mut String,
) -> Result<InboundBindingCredentials, BindingAuthorizationFailed> {
    let mut locations = CredentialLocations::take(headers, uri, endpoint);

    match protocol {
        BindingAuthProtocol::Claude => {
            if locations.invalid || locations.x_goog_api_key.is_present() {
                return Err(BindingAuthorizationFailed);
            }
            locations.bearer.append(&mut locations.x_api_key);
            locations.bearer.append(&mut locations.query_key);
            Ok(InboundBindingCredentials {
                binding_key: locations.bearer.into_unique()?,
                gateway_token: None,
            })
        }
        BindingAuthProtocol::Codex => {
            if locations.invalid
                || locations.x_api_key.is_present()
                || locations.x_goog_api_key.is_present()
            {
                return Err(BindingAuthorizationFailed);
            }
            locations.bearer.append(&mut locations.query_key);
            Ok(InboundBindingCredentials {
                binding_key: locations.bearer.into_unique()?,
                gateway_token: None,
            })
        }
        BindingAuthProtocol::Gemini => {
            if locations.invalid || locations.x_api_key.is_present() {
                return Err(BindingAuthorizationFailed);
            }
            locations.bearer.append(&mut locations.x_goog_api_key);
            locations.bearer.append(&mut locations.query_key);
            Ok(InboundBindingCredentials {
                binding_key: locations.bearer.into_unique()?,
                gateway_token: None,
            })
        }
        BindingAuthProtocol::ClaudeDesktop => {
            if locations.invalid
                || locations.x_goog_api_key.is_present()
                || locations.query_key.is_present()
            {
                return Err(BindingAuthorizationFailed);
            }
            Ok(InboundBindingCredentials {
                binding_key: locations.x_api_key.into_unique()?,
                gateway_token: Some(locations.bearer.into_unique()?),
            })
        }
    }
}

#[derive(Default)]
struct SecretCandidates {
    values: Vec<SecretString>,
}

impl SecretCandidates {
    fn push(&mut self, value: String) {
        self.values.push(SecretString::new(value));
    }

    fn is_present(&self) -> bool {
        !self.values.is_empty()
    }

    fn append(&mut self, other: &mut Self) {
        self.values.append(&mut other.values);
    }

    fn into_unique(mut self) -> Result<SecretString, BindingAuthorizationFailed> {
        if self.values.is_empty() {
            return Err(BindingAuthorizationFailed);
        }

        let first = self.values.remove(0);
        if first.expose_bytes().is_empty()
            || self.values.iter().any(|candidate| {
                candidate.expose_bytes().is_empty()
                    || first
                        .expose_bytes()
                        .ct_eq(candidate.expose_bytes())
                        .unwrap_u8()
                        != 1
            })
        {
            return Err(BindingAuthorizationFailed);
        }

        Ok(first)
    }
}

struct CredentialLocations {
    bearer: SecretCandidates,
    x_api_key: SecretCandidates,
    x_goog_api_key: SecretCandidates,
    query_key: SecretCandidates,
    invalid: bool,
}

impl CredentialLocations {
    fn take(headers: &mut HeaderMap, uri: &mut Uri, endpoint: &mut String) -> Self {
        let (bearer, bearer_invalid) = take_bearer_headers(headers);
        let (x_api_key, x_api_key_invalid) = take_plain_headers(headers, "x-api-key");
        let (x_goog_api_key, x_goog_invalid) = take_plain_headers(headers, "x-goog-api-key");
        strip_non_binding_credential_headers(headers);

        let mut query_key = strip_query_key_from_endpoint(endpoint);
        let (mut uri_query_key, uri_invalid) = strip_query_key_from_uri(uri);
        query_key.append(&mut uri_query_key);

        Self {
            bearer,
            x_api_key,
            x_goog_api_key,
            query_key,
            invalid: bearer_invalid || x_api_key_invalid || x_goog_invalid || uri_invalid,
        }
    }
}

fn take_bearer_headers(headers: &mut HeaderMap) -> (SecretCandidates, bool) {
    let mut candidates = SecretCandidates::default();
    let mut invalid = false;

    for value in headers.get_all(AUTHORIZATION).iter() {
        match parse_bearer(value) {
            Some(token) => candidates.push(token.to_string()),
            None => invalid = true,
        }
    }
    headers.remove(AUTHORIZATION);

    (candidates, invalid)
}

fn parse_bearer(value: &HeaderValue) -> Option<&str> {
    let value = value.to_str().ok()?;
    let mut parts = value.split_ascii_whitespace();
    let scheme = parts.next()?;
    let token = parts.next()?;
    if !scheme.eq_ignore_ascii_case("bearer") || token.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(token)
}

fn take_plain_headers(headers: &mut HeaderMap, name: &'static str) -> (SecretCandidates, bool) {
    let mut candidates = SecretCandidates::default();
    let mut invalid = false;

    for value in headers.get_all(name).iter() {
        match value
            .to_str()
            .ok()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(value) => candidates.push(value.to_string()),
            None => invalid = true,
        }
    }
    headers.remove(name);

    (candidates, invalid)
}

fn strip_query_key_from_uri(uri: &mut Uri) -> (SecretCandidates, bool) {
    let Some(query) = uri.query() else {
        return (SecretCandidates::default(), false);
    };
    let (sanitized_query, candidates) = strip_credential_query(query);
    let Some(sanitized_query) = sanitized_query else {
        return (candidates, false);
    };

    let sanitized_path_and_query = if sanitized_query.is_empty() {
        uri.path().to_string()
    } else {
        format!("{}?{sanitized_query}", uri.path())
    };
    let Ok(sanitized_path_and_query) = sanitized_path_and_query.parse::<PathAndQuery>() else {
        *uri = Uri::from_static("/");
        return (candidates, true);
    };

    let mut parts = std::mem::replace(uri, Uri::from_static("/")).into_parts();
    parts.path_and_query = Some(sanitized_path_and_query);
    let Ok(sanitized_uri) = Uri::from_parts(parts) else {
        return (candidates, true);
    };
    *uri = sanitized_uri;

    (candidates, false)
}

fn strip_query_key_from_endpoint(endpoint: &mut String) -> SecretCandidates {
    let Some((path, query)) = endpoint.split_once('?') else {
        return SecretCandidates::default();
    };
    let (sanitized_query, candidates) = strip_credential_query(query);
    let Some(sanitized_query) = sanitized_query else {
        return candidates;
    };

    *endpoint = if sanitized_query.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{sanitized_query}")
    };
    candidates
}

fn normalized_query_name(name: &str) -> String {
    name.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub(crate) fn is_credential_query_name(name: &str) -> bool {
    let name = normalized_query_name(name);
    matches!(
        name.as_str(),
        "key"
            | "apikey"
            | "token"
            | "authtoken"
            | "accesstoken"
            | "refreshtoken"
            | "bearertoken"
            | "auth"
            | "authorization"
            | "credential"
            | "credentials"
            | "secret"
            | "password"
            | "signature"
            | "sig"
            | "xapikey"
            | "xgoogapikey"
    ) || name.ends_with("apikey")
        || name.ends_with("token")
        || name.ends_with("secret")
        || name.ends_with("password")
        || name.ends_with("credential")
        || name.ends_with("authorization")
        || name.ends_with("signature")
        || name.ends_with("privatekey")
        || name.ends_with("signingkey")
        || name.ends_with("secretkey")
        || name.ends_with("accesskeyid")
        || name.ends_with("credentials")
        || name.ends_with("subscriptionkey")
        || name.ends_with("auth")
        || name.contains("bearertoken")
        || name.contains("clientsecret")
        || name.contains("credential")
}

pub(crate) fn is_credential_header_name(name: &str) -> bool {
    matches!(
        normalized_query_name(name).as_str(),
        "cookie"
            | "setcookie"
            | "proxyauthenticate"
            | "xauth"
            | "xauthtoken"
            | "ocpapimsubscriptionkey"
    ) || is_credential_query_name(name)
}

fn strip_non_binding_credential_headers(headers: &mut HeaderMap) {
    let names = headers
        .keys()
        .filter(|name| is_credential_header_name(name.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    for name in names {
        headers.remove(name);
    }
}

/// Returns `Some(sanitized_query)` when any credential-shaped query parameter
/// was removed. Only `key` is accepted as a binding-key candidate; all other
/// credential parameters are discarded. Safe segments remain byte-for-byte
/// unchanged.
fn strip_credential_query(query: &str) -> (Option<String>, SecretCandidates) {
    let mut retained = Vec::new();
    let mut candidates = SecretCandidates::default();
    let mut removed = false;

    for segment in query.split('&') {
        let credential = url::form_urlencoded::parse(segment.as_bytes())
            .next()
            .filter(|(name, _)| is_credential_query_name(name));
        if let Some((name, value)) = credential {
            if name.eq_ignore_ascii_case("key") {
                candidates.push(value.into_owned());
            }
            removed = true;
        } else {
            retained.push(segment);
        }
    }

    (removed.then(|| retained.join("&")), candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{header::AUTHORIZATION, HeaderName, HeaderValue};

    const X_API_KEY: HeaderName = HeaderName::from_static("x-api-key");
    const X_GOOG_API_KEY: HeaderName = HeaderName::from_static("x-goog-api-key");

    fn extract(
        protocol: BindingAuthProtocol,
        headers: &mut HeaderMap,
        uri: &mut Uri,
        endpoint: &mut String,
    ) -> Result<InboundBindingCredentials, BindingAuthorizationFailed> {
        extract_and_strip_binding_auth(protocol, headers, uri, endpoint)
    }

    fn expect_failure(
        result: Result<InboundBindingCredentials, BindingAuthorizationFailed>,
        message: &str,
    ) -> BindingAuthorizationFailed {
        match result {
            Ok(_) => panic!("{message}"),
            Err(error) => error,
        }
    }

    #[test]
    fn claude_accepts_bearer_and_strips_authorization() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer claude-binding-secret"),
        );
        headers.insert(
            HeaderName::from_static("x-relay-auth"),
            HeaderValue::from_static("old-route-secret"),
        );
        let mut uri: Uri = "/v1/messages?beta=true".parse().unwrap();
        let mut endpoint = "/v1/messages?beta=true".to_string();

        let credentials = extract(
            BindingAuthProtocol::Claude,
            &mut headers,
            &mut uri,
            &mut endpoint,
        )
        .expect("Claude Bearer should be accepted");

        assert_eq!(
            credentials.binding_key.expose_bytes(),
            b"claude-binding-secret"
        );
        assert!(credentials.gateway_token.is_none());
        assert!(!headers.contains_key(AUTHORIZATION));
        assert!(!headers.contains_key("x-relay-auth"));
        assert_eq!(uri.to_string(), "/v1/messages?beta=true");
        assert_eq!(endpoint, "/v1/messages?beta=true");
    }

    #[test]
    fn claude_accepts_matching_bearer_and_repeated_x_api_key() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("bearer same-key"));
        headers.append(X_API_KEY, HeaderValue::from_static("same-key"));
        headers.append(X_API_KEY, HeaderValue::from_static("same-key"));
        let mut uri: Uri = "/v1/messages".parse().unwrap();
        let mut endpoint = "/v1/messages".to_string();

        let credentials = extract(
            BindingAuthProtocol::Claude,
            &mut headers,
            &mut uri,
            &mut endpoint,
        )
        .expect("matching values in one role should be accepted");

        assert_eq!(credentials.binding_key.expose_bytes(), b"same-key");
        assert!(!headers.contains_key(AUTHORIZATION));
        assert!(!headers.contains_key(X_API_KEY));
    }

    #[test]
    fn codex_rejects_non_native_key_headers_and_strips_every_known_credential_header() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer codex-key"));
        headers.insert(X_API_KEY, HeaderValue::from_static("wrong-protocol-key"));
        headers.insert(
            X_GOOG_API_KEY,
            HeaderValue::from_static("wrong-protocol-google-key"),
        );
        let mut uri: Uri = "/v1/responses".parse().unwrap();
        let mut endpoint = "/responses".to_string();

        let error = expect_failure(
            extract(
                BindingAuthProtocol::Codex,
                &mut headers,
                &mut uri,
                &mut endpoint,
            ),
            "protocol-mismatched native headers must fail closed",
        );

        assert_eq!(error, BindingAuthorizationFailed);
        assert!(!headers.contains_key(AUTHORIZATION));
        assert!(!headers.contains_key(X_API_KEY));
        assert!(!headers.contains_key(X_GOOG_API_KEY));
    }

    #[test]
    fn codex_accepts_bearer_without_a_protocol_native_key_header() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer codex-key"));
        let mut uri: Uri = "/v1/responses".parse().unwrap();
        let mut endpoint = "/responses".to_string();

        let credentials = extract(
            BindingAuthProtocol::Codex,
            &mut headers,
            &mut uri,
            &mut endpoint,
        )
        .expect("OpenAI/Codex native authentication is Bearer");

        assert_eq!(credentials.binding_key.expose_bytes(), b"codex-key");
        assert!(!headers.contains_key(AUTHORIZATION));
    }

    #[test]
    fn ordinary_claude_and_codex_accept_and_strip_query_key() {
        for protocol in [BindingAuthProtocol::Claude, BindingAuthProtocol::Codex] {
            let mut headers = HeaderMap::new();
            let mut uri: Uri = "/v1/request?key=ordinary-query-key&beta=true"
                .parse()
                .unwrap();
            let mut endpoint = "/v1/request?key=ordinary-query-key&beta=true".to_string();

            let credentials = extract(protocol, &mut headers, &mut uri, &mut endpoint)
                .expect("ordinary routes should accept sanitized query keys");

            assert_eq!(
                credentials.binding_key.expose_bytes(),
                b"ordinary-query-key"
            );
            assert_eq!(uri.to_string(), "/v1/request?beta=true");
            assert_eq!(endpoint, "/v1/request?beta=true");
        }
    }

    #[test]
    fn non_binding_credential_query_parameters_are_always_stripped() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer binding-key"),
        );
        let mut uri: Uri = concat!(
            "/v1/messages?token=inbound-secret&access_token=also-secret",
            "&client_secret=client-secret&session_token=session-secret",
            "&X-Amz-Security-Token=aws-secret&X-Amz-Credential=aws-credential",
            "&trace=safe"
        )
        .parse()
        .unwrap();
        let mut endpoint = uri.to_string();

        let credentials = extract(
            BindingAuthProtocol::Claude,
            &mut headers,
            &mut uri,
            &mut endpoint,
        )
        .expect("Bearer remains the binding credential");

        assert_eq!(credentials.binding_key.expose_bytes(), b"binding-key");
        assert_eq!(uri.to_string(), "/v1/messages?trace=safe");
        assert_eq!(endpoint, "/v1/messages?trace=safe");
    }

    #[test]
    fn gemini_accepts_bearer_and_x_goog_api_key_when_they_agree() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer gemini-key"));
        headers.insert(X_GOOG_API_KEY, HeaderValue::from_static("gemini-key"));
        let mut uri: Uri = "/v1beta/models/gemini:generateContent".parse().unwrap();
        let mut endpoint = "/v1beta/models/gemini:generateContent".to_string();

        let credentials = extract(
            BindingAuthProtocol::Gemini,
            &mut headers,
            &mut uri,
            &mut endpoint,
        )
        .expect("matching Gemini credential locations should be accepted");

        assert_eq!(credentials.binding_key.expose_bytes(), b"gemini-key");
        assert!(!headers.contains_key(AUTHORIZATION));
        assert!(!headers.contains_key(X_GOOG_API_KEY));
    }

    #[test]
    fn gemini_decodes_and_strips_query_key_from_uri_and_endpoint() {
        let mut headers = HeaderMap::new();
        let mut uri: Uri =
            "/v1beta/models/gemini:generateContent?alt=sse&key=query%2Dsecret&raw=a%2Bb"
                .parse()
                .unwrap();
        let mut endpoint =
            "/v1beta/models/gemini:generateContent?alt=sse&key=query%2Dsecret&raw=a%2Bb"
                .to_string();

        let credentials = extract(
            BindingAuthProtocol::Gemini,
            &mut headers,
            &mut uri,
            &mut endpoint,
        )
        .expect("Gemini query key should be accepted");

        assert_eq!(credentials.binding_key.expose_bytes(), b"query-secret");
        assert_eq!(
            uri.to_string(),
            "/v1beta/models/gemini:generateContent?alt=sse&raw=a%2Bb"
        );
        assert_eq!(
            endpoint,
            "/v1beta/models/gemini:generateContent?alt=sse&raw=a%2Bb"
        );
    }

    #[test]
    fn gemini_query_key_in_endpoint_only_is_still_sanitized() {
        let mut headers = HeaderMap::new();
        let mut uri: Uri = "/v1beta/models/gemini:generateContent".parse().unwrap();
        let mut endpoint =
            "/v1beta/models/gemini:generateContent?alt=json&key=endpoint-key".to_string();

        let credentials = extract(
            BindingAuthProtocol::Gemini,
            &mut headers,
            &mut uri,
            &mut endpoint,
        )
        .expect("endpoint query key should be accepted");

        assert_eq!(credentials.binding_key.expose_bytes(), b"endpoint-key");
        assert_eq!(uri.to_string(), "/v1beta/models/gemini:generateContent");
        assert_eq!(endpoint, "/v1beta/models/gemini:generateContent?alt=json");
    }

    #[test]
    fn query_key_name_is_case_insensitive_and_never_survives_sanitization() {
        let mut headers = HeaderMap::new();
        let mut uri: Uri = "/v1beta/models/gemini?Key=case-key&alt=sse"
            .parse()
            .unwrap();
        let mut endpoint = "/v1beta/models/gemini?KEY=case-key&alt=sse".to_string();

        let credentials = extract(
            BindingAuthProtocol::Gemini,
            &mut headers,
            &mut uri,
            &mut endpoint,
        )
        .expect("query credential names should be case-insensitive");

        assert_eq!(credentials.binding_key.expose_bytes(), b"case-key");
        assert_eq!(uri.to_string(), "/v1beta/models/gemini?alt=sse");
        assert_eq!(endpoint, "/v1beta/models/gemini?alt=sse");
    }

    #[test]
    fn repeated_binding_role_values_must_agree_and_errors_have_no_payload() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer first-secret"),
        );
        headers.insert(X_API_KEY, HeaderValue::from_static("second-secret"));
        let mut uri: Uri = "/v1/messages".parse().unwrap();
        let mut endpoint = "/v1/messages".to_string();

        let error = expect_failure(
            extract(
                BindingAuthProtocol::Claude,
                &mut headers,
                &mut uri,
                &mut endpoint,
            ),
            "one binding role cannot contain conflicting values",
        );

        assert_eq!(format!("{error:?}"), "BindingAuthorizationFailed");
        assert_eq!(error.to_string(), "binding authorization failed");
        assert!(!format!("{error:?}").contains("first-secret"));
        assert!(!error.to_string().contains("second-secret"));
        assert!(!headers.contains_key(AUTHORIZATION));
        assert!(!headers.contains_key(X_API_KEY));
    }

    #[test]
    fn gemini_conflicting_query_and_header_values_fail_after_sanitizing_both() {
        let mut headers = HeaderMap::new();
        headers.insert(X_GOOG_API_KEY, HeaderValue::from_static("header-secret"));
        let mut uri: Uri = "/v1beta/models/gemini?key=query-secret&alt=sse"
            .parse()
            .unwrap();
        let mut endpoint = "/v1beta/models/gemini?key=query-secret&alt=sse".to_string();

        let error = expect_failure(
            extract(
                BindingAuthProtocol::Gemini,
                &mut headers,
                &mut uri,
                &mut endpoint,
            ),
            "Gemini binding credential locations must agree",
        );

        assert_eq!(error, BindingAuthorizationFailed);
        assert!(!headers.contains_key(X_GOOG_API_KEY));
        assert_eq!(uri.to_string(), "/v1beta/models/gemini?alt=sse");
        assert_eq!(endpoint, "/v1beta/models/gemini?alt=sse");
    }

    #[test]
    fn claude_desktop_keeps_gateway_and_binding_roles_distinct() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer gateway-secret"),
        );
        headers.insert(X_API_KEY, HeaderValue::from_static("binding-secret"));
        let mut uri: Uri = "/claude-desktop/v1/models".parse().unwrap();
        let mut endpoint = "/v1/models".to_string();

        let credentials = extract(
            BindingAuthProtocol::ClaudeDesktop,
            &mut headers,
            &mut uri,
            &mut endpoint,
        )
        .expect("different gateway and binding tokens are valid");

        assert_eq!(credentials.binding_key.expose_bytes(), b"binding-secret");
        assert_eq!(
            credentials
                .gateway_token
                .as_ref()
                .expect("gateway token")
                .expose_bytes(),
            b"gateway-secret"
        );
        assert!(!headers.contains_key(AUTHORIZATION));
        assert!(!headers.contains_key(X_API_KEY));
    }

    #[test]
    fn claude_desktop_requires_each_role_to_be_in_its_own_location() {
        for (authorization, x_api_key) in [
            (Some("Bearer gateway-only"), None),
            (None, Some("binding-only")),
        ] {
            let mut headers = HeaderMap::new();
            if let Some(value) = authorization {
                headers.insert(AUTHORIZATION, HeaderValue::from_str(value).unwrap());
            }
            if let Some(value) = x_api_key {
                headers.insert(X_API_KEY, HeaderValue::from_str(value).unwrap());
            }
            let mut uri: Uri = "/claude-desktop/v1/messages".parse().unwrap();
            let mut endpoint = "/v1/messages".to_string();

            let error = expect_failure(
                extract(
                    BindingAuthProtocol::ClaudeDesktop,
                    &mut headers,
                    &mut uri,
                    &mut endpoint,
                ),
                "gateway and binding roles are both required",
            );

            assert_eq!(error, BindingAuthorizationFailed);
            assert!(!headers.contains_key(AUTHORIZATION));
            assert!(!headers.contains_key(X_API_KEY));
        }
    }

    #[test]
    fn claude_desktop_rejects_query_key_as_binding_and_strips_it() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer gateway-secret"),
        );
        headers.insert(X_API_KEY, HeaderValue::from_static("binding-secret"));
        let mut uri: Uri = "/claude-desktop/v1/messages?key=wrong-location"
            .parse()
            .unwrap();
        let mut endpoint = "/v1/messages?key=wrong-location".to_string();

        let error = expect_failure(
            extract(
                BindingAuthProtocol::ClaudeDesktop,
                &mut headers,
                &mut uri,
                &mut endpoint,
            ),
            "Desktop binding key must only use x-api-key",
        );

        assert_eq!(error, BindingAuthorizationFailed);
        assert_eq!(uri.to_string(), "/claude-desktop/v1/messages");
        assert_eq!(endpoint, "/v1/messages");
        assert!(!headers.contains_key(AUTHORIZATION));
        assert!(!headers.contains_key(X_API_KEY));
    }

    #[test]
    fn malformed_or_missing_credentials_fail_with_the_same_generic_error() {
        for authorization in [None, Some("Basic raw-secret"), Some("Bearer ")] {
            let mut headers = HeaderMap::new();
            if let Some(value) = authorization {
                headers.insert(AUTHORIZATION, HeaderValue::from_str(value).unwrap());
            }
            let mut uri: Uri = "/v1/responses".parse().unwrap();
            let mut endpoint = "/responses".to_string();

            let error = expect_failure(
                extract(
                    BindingAuthProtocol::Codex,
                    &mut headers,
                    &mut uri,
                    &mut endpoint,
                ),
                "malformed and missing credentials both fail closed",
            );

            assert_eq!(error, BindingAuthorizationFailed);
            assert!(!headers.contains_key(AUTHORIZATION));
        }
    }
}
