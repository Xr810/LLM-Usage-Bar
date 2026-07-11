use crate::database::{lock_conn, to_json_string, Database};
use crate::error::AppError;
use crate::usage::domain::{
    BillingKind, RouteBinding, TokenSource, UsageProviderInput, UsageProviderStored,
    UsageProviderView, UsageSourceBinding,
};
use rusqlite::{params, types::Type, OptionalExtension, Row};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

const SUPPORTED_SESSION_SOURCES: [&str; 2] = ["claude", "codex"];

fn now_timestamp() -> Result<i64, AppError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|error| AppError::Database(format!("system clock before unix epoch: {error}")))
}

fn billing_kind_value(kind: BillingKind) -> &'static str {
    match kind {
        BillingKind::Subscription => "subscription",
        BillingKind::Metered => "metered",
    }
}

fn parse_json_column<T: DeserializeOwned>(value: String, column: usize) -> rusqlite::Result<T> {
    serde_json::from_str(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(error))
    })
}

fn optional_json_column<T: DeserializeOwned>(
    value: Option<String>,
    column: usize,
) -> rusqlite::Result<Option<T>> {
    value.map(|raw| parse_json_column(raw, column)).transpose()
}

fn provider_from_row(row: &Row<'_>) -> rusqlite::Result<UsageProviderStored> {
    let billing_kind = parse_json_column(format!("\"{}\"", row.get::<_, String>(2)?), 2)?;
    let token_sources = parse_json_column(row.get(4)?, 4)?;
    Ok(UsageProviderStored {
        id: row.get(0)?,
        name: row.get(1)?,
        billing_kind,
        product_group_id: row.get(3)?,
        token_sources,
        quota_source: row.get(5)?,
        quota_interval_seconds: row.get::<_, Option<i64>>(6)?.map(|value| value as u64),
        route_app_type: row.get(7)?,
        route_config: optional_json_column(row.get(8)?, 8)?,
        quota_config: optional_json_column(row.get(9)?, 9)?,
        enabled: row.get(10)?,
        needs_review: row.get(11)?,
        legacy_app_type: row.get(12)?,
        legacy_provider_id: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

const PROVIDER_COLUMNS: &str = "id, name, billing_kind, product_group_id, token_sources,
    quota_source, quota_interval_seconds, route_app_type, route_config, quota_config,
    enabled, needs_review, legacy_app_type, legacy_provider_id, created_at, updated_at";

fn has_non_empty_value(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(value) => !value.trim().is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
    }
}

fn public_route_base_url(raw: String) -> Option<String> {
    let mut url = Url::parse(&raw).ok()?;
    url.set_username("").ok()?;
    url.set_password(None).ok()?;
    url.set_query(None);
    url.set_fragment(None);
    let root_path = url.path() == "/";
    let sanitized = url.to_string();
    Some(if root_path {
        sanitized.trim_end_matches('/').to_string()
    } else {
        sanitized
    })
}

fn provider_view(
    provider: &UsageProviderStored,
    session_source_bindings: Vec<String>,
) -> UsageProviderView {
    let route_base_url = provider.route_config.as_ref().and_then(|config| {
        let direct_or_env = config
            .get("base_url")
            .or_else(|| config.get("baseUrl"))
            .or_else(|| config.get("baseURL"))
            .and_then(Value::as_str)
            .or_else(|| {
                config
                    .pointer("/env/ANTHROPIC_BASE_URL")
                    .and_then(Value::as_str)
            })
            .or_else(|| {
                config
                    .pointer("/env/GOOGLE_GEMINI_BASE_URL")
                    .and_then(Value::as_str)
            });
        direct_or_env
            .map(str::to_string)
            .or_else(|| {
                config
                    .get("config")
                    .and_then(Value::as_str)
                    .and_then(crate::codex_config::extract_codex_base_url)
            })
            .and_then(public_route_base_url)
    });
    let has_route_credentials = provider.route_config.as_ref().is_some_and(|config| {
        let has_direct = [
            "api_key",
            "apiKey",
            "token",
            "access_token",
            "accessToken",
            "auth_token",
            "authToken",
        ]
        .iter()
        .any(|key| config.get(key).is_some_and(has_non_empty_value));
        let has_env = [
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_API_KEY",
            "OPENROUTER_API_KEY",
            "OPENAI_API_KEY",
            "GEMINI_API_KEY",
        ]
        .iter()
        .any(|key| {
            config
                .get("env")
                .and_then(|env| env.get(key))
                .is_some_and(has_non_empty_value)
        });
        let config_text = config.get("config").and_then(Value::as_str);
        let has_codex = crate::codex_config::extract_codex_api_key(config.get("auth"), config_text)
            .is_some()
            || config.get("config").is_some_and(|nested| {
                ["api_key", "apiKey", "token"]
                    .iter()
                    .any(|key| nested.get(key).is_some_and(has_non_empty_value))
            });
        has_direct || has_env || has_codex
    });

    UsageProviderView {
        id: provider.id.clone(),
        name: provider.name.clone(),
        billing_kind: provider.billing_kind,
        product_group_id: provider.product_group_id.clone(),
        token_sources: provider.token_sources.clone(),
        session_source_bindings,
        quota_source: provider.quota_source.clone(),
        quota_interval_seconds: provider.quota_interval_seconds,
        route_app_type: provider.route_app_type.clone(),
        enabled: provider.enabled,
        needs_review: provider.needs_review,
        created_at: provider.created_at,
        updated_at: provider.updated_at,
        route_base_url,
        has_route_credentials,
    }
}

fn validate_session_source_bindings(
    input: &UsageProviderInput,
) -> Result<Option<Vec<String>>, AppError> {
    let Some(requested) = input.session_source_bindings.as_ref() else {
        return Ok(None);
    };
    for source in requested {
        if !SUPPORTED_SESSION_SOURCES.contains(&source.as_str()) {
            return Err(AppError::Message(format!(
                "unsupported usage source: {source}"
            )));
        }
    }
    if !requested.is_empty() && !input.token_sources.contains(&TokenSource::SessionLog) {
        return Err(AppError::Message(
            "session source bindings require session_log token support".to_string(),
        ));
    }
    Ok(Some(
        SUPPORTED_SESSION_SOURCES
            .iter()
            .filter(|source| requested.iter().any(|requested| requested == **source))
            .map(|source| (*source).to_string())
            .collect(),
    ))
}

fn source_bindings_for_provider(
    conn: &rusqlite::Connection,
    provider_id: &str,
) -> Result<Vec<String>, AppError> {
    let mut statement = conn.prepare(
        "SELECT source FROM usage_source_bindings
         WHERE provider_id = ?1 ORDER BY source",
    )?;
    let bindings = statement
        .query_map([provider_id], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(AppError::from)?;
    Ok(bindings)
}

impl Database {
    pub fn list_usage_providers(&self) -> Result<Vec<UsageProviderView>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut statement = conn.prepare(&format!(
            "SELECT {PROVIDER_COLUMNS} FROM usage_providers ORDER BY id"
        ))?;
        let providers = statement
            .query_map([], provider_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        providers
            .iter()
            .map(|provider| {
                Ok(provider_view(
                    provider,
                    source_bindings_for_provider(&conn, &provider.id)?,
                ))
            })
            .collect()
    }

    pub fn get_usage_provider(&self, id: &str) -> Result<Option<UsageProviderStored>, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            &format!("SELECT {PROVIDER_COLUMNS} FROM usage_providers WHERE id = ?1"),
            [id],
            provider_from_row,
        )
        .optional()
        .map_err(AppError::from)
    }

    pub fn save_usage_provider(
        &self,
        input: &UsageProviderInput,
    ) -> Result<UsageProviderView, AppError> {
        input.validate().map_err(AppError::Message)?;
        let requested_session_sources = validate_session_source_bindings(input)?;
        let now = now_timestamp()?;
        let token_sources = to_json_string(&input.token_sources)?;
        let route_config = input
            .route_config
            .as_ref()
            .map(to_json_string)
            .transpose()?;
        let quota_config = input
            .quota_config
            .as_ref()
            .map(to_json_string)
            .transpose()?;
        let quota_interval_seconds = input
            .quota_interval_seconds
            .map(i64::try_from)
            .transpose()
            .map_err(|_| AppError::Message("quota interval is too large".to_string()))?;

        let _operation_guard = lock_conn!(self.usage_source_binding_operation);
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction()?;
        transaction.execute(
            "INSERT INTO usage_providers (
                id, name, billing_kind, product_group_id, token_sources, quota_source,
                quota_interval_seconds, route_app_type, route_config, quota_config,
                enabled, needs_review, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0, ?12, ?12)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                billing_kind = excluded.billing_kind,
                product_group_id = excluded.product_group_id,
                token_sources = excluded.token_sources,
                quota_source = excluded.quota_source,
                quota_interval_seconds = excluded.quota_interval_seconds,
                route_app_type = excluded.route_app_type,
                route_config = COALESCE(excluded.route_config, usage_providers.route_config),
                quota_config = COALESCE(excluded.quota_config, usage_providers.quota_config),
                enabled = excluded.enabled,
                updated_at = excluded.updated_at",
            params![
                input.id,
                input.name,
                billing_kind_value(input.billing_kind),
                input.product_group_id,
                token_sources,
                input.quota_source,
                quota_interval_seconds,
                input.route_app_type,
                route_config,
                quota_config,
                input.enabled,
                now,
            ],
        )?;

        if let Some(requested) = requested_session_sources.as_ref() {
            for source in SUPPORTED_SESSION_SOURCES {
                if requested.iter().any(|requested| requested == source) {
                    transaction.execute(
                        "INSERT INTO usage_source_bindings (source, provider_id, updated_at)
                         VALUES (?1, ?2, ?3)
                         ON CONFLICT(source) DO UPDATE SET
                            provider_id = excluded.provider_id,
                            updated_at = excluded.updated_at",
                        params![source, input.id, now],
                    )?;
                } else {
                    transaction.execute(
                        "DELETE FROM usage_source_bindings
                         WHERE source = ?1 AND provider_id = ?2",
                        params![source, input.id],
                    )?;
                }
            }
        }

        let stored = transaction.query_row(
            &format!("SELECT {PROVIDER_COLUMNS} FROM usage_providers WHERE id = ?1"),
            [&input.id],
            provider_from_row,
        )?;
        let bindings = source_bindings_for_provider(&transaction, &input.id)?;
        transaction.commit()?;
        Ok(provider_view(&stored, bindings))
    }

    pub fn set_usage_provider_enabled(&self, id: &str, enabled: bool) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        let updated = conn.execute(
            "UPDATE usage_providers SET enabled = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, enabled, now_timestamp()?],
        )?;
        if updated == 0 {
            return Err(AppError::Message("usage provider not found".to_string()));
        }
        Ok(())
    }

    pub fn get_route_bindings(&self) -> Result<Vec<RouteBinding>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut statement = conn.prepare(
            "SELECT protocol, provider_id, updated_at FROM route_bindings ORDER BY protocol",
        )?;
        let bindings = statement
            .query_map([], |row| {
                Ok(RouteBinding {
                    protocol: row.get(0)?,
                    provider_id: row.get(1)?,
                    updated_at: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)?;
        Ok(bindings)
    }

    pub fn set_route_binding(
        &self,
        protocol: &str,
        provider_id: &str,
    ) -> Result<RouteBinding, AppError> {
        let conn = lock_conn!(self.conn);
        let provider = conn
            .query_row(
                "SELECT billing_kind, enabled FROM usage_providers WHERE id = ?1",
                [provider_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
            )
            .optional()?;
        let Some((billing_kind, enabled)) = provider else {
            return Err(AppError::Message("route provider not found".to_string()));
        };
        if billing_kind != "metered" {
            return Err(AppError::Message(
                "route provider must be metered".to_string(),
            ));
        }
        if !enabled {
            return Err(AppError::Message(
                "route provider must be enabled".to_string(),
            ));
        }

        let updated_at = now_timestamp()?;
        conn.execute(
            "INSERT INTO route_bindings (protocol, provider_id, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(protocol) DO UPDATE SET
                provider_id = excluded.provider_id,
                updated_at = excluded.updated_at",
            params![protocol, provider_id, updated_at],
        )?;
        Ok(RouteBinding {
            protocol: protocol.to_string(),
            provider_id: provider_id.to_string(),
            updated_at,
        })
    }

    pub fn get_usage_source_binding(
        &self,
        source_key: &str,
    ) -> Result<Option<UsageSourceBinding>, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT source, provider_id, updated_at
             FROM usage_source_bindings WHERE source = ?1",
            [source_key],
            |row| {
                Ok(UsageSourceBinding {
                    source: row.get(0)?,
                    provider_id: row.get(1)?,
                    updated_at: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(AppError::from)
    }

    pub fn set_usage_source_binding(
        &self,
        source_key: &str,
        provider_id: &str,
    ) -> Result<UsageSourceBinding, AppError> {
        if !SUPPORTED_SESSION_SOURCES.contains(&source_key) {
            return Err(AppError::Message(format!(
                "unsupported usage source: {source_key}"
            )));
        }
        let _operation_guard = lock_conn!(self.usage_source_binding_operation);
        let conn = lock_conn!(self.conn);
        let token_sources = conn
            .query_row(
                "SELECT token_sources FROM usage_providers WHERE id = ?1",
                [provider_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(token_sources) = token_sources else {
            return Err(AppError::Message("source provider not found".to_string()));
        };
        let token_sources: Vec<TokenSource> =
            serde_json::from_str(&token_sources).map_err(|error| {
                AppError::Database(format!("invalid provider token_sources: {error}"))
            })?;
        if !token_sources.contains(&TokenSource::SessionLog) {
            return Err(AppError::Message(
                "source provider must support session_log".to_string(),
            ));
        }

        let updated_at = now_timestamp()?;
        conn.execute(
            "INSERT INTO usage_source_bindings (source, provider_id, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(source) DO UPDATE SET
                provider_id = excluded.provider_id,
                updated_at = excluded.updated_at",
            params![source_key, provider_id, updated_at],
        )?;
        Ok(UsageSourceBinding {
            source: source_key.to_string(),
            provider_id: provider_id.to_string(),
            updated_at,
        })
    }

    pub(crate) fn with_bound_usage_source<T>(
        &self,
        source_key: &str,
        provider_id: &str,
        operation: impl FnOnce() -> Result<T, AppError>,
    ) -> Result<Option<T>, AppError> {
        if !SUPPORTED_SESSION_SOURCES.contains(&source_key) {
            return Err(AppError::Message(format!(
                "unsupported usage source: {source_key}"
            )));
        }
        let _operation_guard = lock_conn!(self.usage_source_binding_operation);
        let binding_matches = {
            let conn = lock_conn!(self.conn);
            conn.query_row(
                "SELECT provider_id FROM usage_source_bindings WHERE source = ?1",
                [source_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .is_some_and(|bound_provider_id| bound_provider_id == provider_id)
        };
        if !binding_matches {
            return Ok(None);
        }
        operation().map(Some)
    }
}

#[cfg(test)]
mod tests {
    use crate::database::Database;
    use crate::usage::domain::{BillingKind, TokenSource, UsageProviderInput};
    use serde_json::json;
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    fn provider(
        id: &str,
        billing_kind: BillingKind,
        token_sources: Vec<TokenSource>,
    ) -> UsageProviderInput {
        UsageProviderInput {
            id: id.to_string(),
            name: format!("Provider {id}"),
            billing_kind,
            product_group_id: "claude".to_string(),
            token_sources,
            session_source_bindings: None,
            quota_source: Some("official".to_string()),
            quota_interval_seconds: Some(300),
            route_app_type: Some("claude".to_string()),
            route_config: Some(json!({
                "baseUrl": "https://metered.example/v1",
                "apiKey": "secret-value"
            })),
            quota_config: Some(json!({"access_token": "quota-secret"})),
            enabled: true,
        }
    }

    #[test]
    fn provider_id_is_global_and_public_views_redact_route_secrets() {
        let db = Database::memory().unwrap();
        let first = provider("shared", BillingKind::Metered, vec![TokenSource::Proxy]);
        let public = db.save_usage_provider(&first).unwrap();

        assert_eq!(
            public.route_base_url.as_deref(),
            Some("https://metered.example/v1")
        );
        assert!(public.has_route_credentials);
        let serialized = serde_json::to_string(&public).unwrap();
        assert!(!serialized.contains("secret-value"));
        assert!(!serialized.contains("quota-secret"));

        let mut same_id = provider(
            "shared",
            BillingKind::Subscription,
            vec![TokenSource::SessionLog],
        );
        same_id.name = "Updated globally".to_string();
        db.save_usage_provider(&same_id).unwrap();

        let providers = db.list_usage_providers().unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].name, "Updated globally");
        assert_eq!(
            db.get_usage_provider("shared")
                .unwrap()
                .unwrap()
                .billing_kind,
            BillingKind::Subscription
        );
    }

    #[test]
    fn provider_upsert_preserves_omitted_route_and_quota_credentials() {
        let db = Database::memory().unwrap();
        let original = provider("metered", BillingKind::Metered, vec![TokenSource::Proxy]);
        db.save_usage_provider(&original).unwrap();

        let mut edit = original;
        edit.name = "Edited".to_string();
        edit.route_config = None;
        edit.quota_config = None;
        let view = db.save_usage_provider(&edit).unwrap();

        assert_eq!(view.name, "Edited");
        assert_eq!(
            view.route_base_url.as_deref(),
            Some("https://metered.example/v1")
        );
        assert!(view.has_route_credentials);
        let stored = db.get_usage_provider("metered").unwrap().unwrap();
        assert_eq!(
            stored.route_config.unwrap()["apiKey"],
            json!("secret-value")
        );
        assert_eq!(
            stored.quota_config.unwrap()["access_token"],
            json!("quota-secret")
        );
    }

    #[test]
    fn redaction_supports_base_url_and_all_documented_credential_keys() {
        for (index, key) in [
            "api_key",
            "apiKey",
            "token",
            "access_token",
            "accessToken",
            "auth_token",
            "authToken",
        ]
        .iter()
        .enumerate()
        {
            let db = Database::memory().unwrap();
            let mut input = provider(
                &format!("provider-{index}"),
                BillingKind::Metered,
                vec![TokenSource::Proxy],
            );
            input.route_config = Some(json!({"base_url": "https://snake.example", (*key): "x"}));
            let view = db.save_usage_provider(&input).unwrap();
            assert_eq!(
                view.route_base_url.as_deref(),
                Some("https://snake.example")
            );
            assert!(view.has_route_credentials, "credential key {key}");
        }

        let db = Database::memory().unwrap();
        let mut empty = provider("empty", BillingKind::Metered, vec![TokenSource::Proxy]);
        empty.route_config = Some(json!({"baseUrl": "https://camel.example", "token": ""}));
        let view = db.save_usage_provider(&empty).unwrap();
        assert_eq!(
            view.route_base_url.as_deref(),
            Some("https://camel.example")
        );
        assert!(!view.has_route_credentials);
    }

    #[test]
    fn redaction_recognizes_migrated_nested_route_shapes() {
        for (index, route_config, expected_base_url) in [
            (
                0,
                json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://claude.example",
                        "ANTHROPIC_AUTH_TOKEN": "claude-secret"
                    }
                }),
                "https://claude.example",
            ),
            (
                1,
                json!({
                    "env": {
                        "GOOGLE_GEMINI_BASE_URL": "https://gemini.example",
                        "GEMINI_API_KEY": "gemini-secret"
                    }
                }),
                "https://gemini.example",
            ),
            (
                2,
                json!({
                    "auth": {"OPENAI_API_KEY": "codex-secret"},
                    "config": "model_provider = \"custom\"\n[model_providers.custom]\nbase_url = \"https://codex.example\""
                }),
                "https://codex.example",
            ),
        ] {
            let db = Database::memory().unwrap();
            let mut input = provider(
                &format!("nested-{index}"),
                BillingKind::Metered,
                vec![TokenSource::Proxy],
            );
            input.route_config = Some(route_config);
            let view = db.save_usage_provider(&input).unwrap();
            assert_eq!(view.route_base_url.as_deref(), Some(expected_base_url));
            assert!(view.has_route_credentials);
            let serialized = serde_json::to_string(&view).unwrap();
            assert!(!serialized.contains("secret"));
        }
    }

    #[test]
    fn public_route_base_url_strips_userinfo_query_and_fragment() {
        let db = Database::memory().unwrap();
        let mut input = provider("url-secret", BillingKind::Metered, vec![TokenSource::Proxy]);
        input.route_config = Some(json!({
            "baseUrl": "https://user:password@example.com/v1?api_key=query-secret#fragment-secret",
            "apiKey": "header-secret"
        }));

        let view = db.save_usage_provider(&input).unwrap();
        assert_eq!(
            view.route_base_url.as_deref(),
            Some("https://example.com/v1")
        );
        let serialized = serde_json::to_string(&view).unwrap();
        for secret in [
            "user",
            "password",
            "query-secret",
            "fragment-secret",
            "header-secret",
        ] {
            assert!(!serialized.contains(secret));
        }
    }

    #[test]
    fn route_binding_requires_an_enabled_metered_provider() {
        let db = Database::memory().unwrap();
        let subscription = provider(
            "subscription-id",
            BillingKind::Subscription,
            vec![TokenSource::SessionLog],
        );
        db.save_usage_provider(&subscription).unwrap();
        assert_eq!(
            db.set_route_binding("claude", "subscription-id")
                .unwrap_err()
                .to_string(),
            "route provider must be metered"
        );

        let metered = provider("metered-id", BillingKind::Metered, vec![TokenSource::Proxy]);
        db.save_usage_provider(&metered).unwrap();
        db.set_usage_provider_enabled("metered-id", false).unwrap();
        assert_eq!(
            db.set_route_binding("claude", "metered-id")
                .unwrap_err()
                .to_string(),
            "route provider must be enabled"
        );

        db.set_usage_provider_enabled("metered-id", true).unwrap();
        let binding = db.set_route_binding("claude", "metered-id").unwrap();
        assert_eq!(binding.provider_id, "metered-id");
        assert_eq!(db.get_route_bindings().unwrap(), vec![binding]);
    }

    #[test]
    fn source_binding_requires_session_log_support() {
        let db = Database::memory().unwrap();
        let proxy_only = provider("proxy-only", BillingKind::Metered, vec![TokenSource::Proxy]);
        db.save_usage_provider(&proxy_only).unwrap();
        assert_eq!(
            db.set_usage_source_binding("claude", "proxy-only")
                .unwrap_err()
                .to_string(),
            "source provider must support session_log"
        );

        let session = provider(
            "session",
            BillingKind::Subscription,
            vec![TokenSource::SessionLog],
        );
        db.save_usage_provider(&session).unwrap();
        let binding = db.set_usage_source_binding("claude", "session").unwrap();
        assert_eq!(binding.provider_id, "session");
        assert_eq!(
            db.get_usage_source_binding("claude").unwrap(),
            Some(binding)
        );
    }

    #[test]
    fn provider_save_atomically_binds_unbinds_and_takes_over_session_sources() {
        let db = Database::memory().unwrap();
        let mut first = provider(
            "first",
            BillingKind::Subscription,
            vec![TokenSource::SessionLog],
        );
        first.session_source_bindings = Some(vec!["claude".to_string(), "codex".to_string()]);

        let saved = db.save_usage_provider(&first).unwrap();
        assert_eq!(saved.session_source_bindings, vec!["claude", "codex"]);

        let mut second = provider(
            "second",
            BillingKind::Subscription,
            vec![TokenSource::SessionLog],
        );
        second.session_source_bindings = Some(vec!["claude".to_string()]);
        let saved = db.save_usage_provider(&second).unwrap();
        assert_eq!(saved.session_source_bindings, vec!["claude"]);
        assert_eq!(
            db.get_usage_source_binding("claude")
                .unwrap()
                .unwrap()
                .provider_id,
            "second"
        );

        first.session_source_bindings = Some(vec![]);
        let saved = db.save_usage_provider(&first).unwrap();
        assert!(saved.session_source_bindings.is_empty());
        assert_eq!(
            db.get_usage_source_binding("claude")
                .unwrap()
                .unwrap()
                .provider_id,
            "second",
            "deselecting another provider's binding must not remove it"
        );
        assert!(db.get_usage_source_binding("codex").unwrap().is_none());

        let listed = db.list_usage_providers().unwrap();
        assert_eq!(listed[0].session_source_bindings, Vec::<String>::new());
        assert_eq!(listed[1].session_source_bindings, vec!["claude"]);
    }

    #[test]
    fn provider_save_rejects_unknown_or_incapable_session_bindings() {
        let db = Database::memory().unwrap();
        let mut incapable = provider("incapable", BillingKind::Metered, vec![TokenSource::Proxy]);
        incapable.session_source_bindings = Some(vec!["claude".to_string()]);
        assert_eq!(
            db.save_usage_provider(&incapable).unwrap_err().to_string(),
            "session source bindings require session_log token support"
        );

        let mut unknown = provider(
            "unknown",
            BillingKind::Subscription,
            vec![TokenSource::SessionLog],
        );
        unknown.session_source_bindings = Some(vec!["other".to_string()]);
        assert_eq!(
            db.save_usage_provider(&unknown).unwrap_err().to_string(),
            "unsupported usage source: other"
        );
    }

    #[test]
    fn provider_and_session_bindings_roll_back_as_one_transaction() {
        let db = Database::memory().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute_batch(
                "CREATE TRIGGER fail_session_binding
                 BEFORE INSERT ON usage_source_bindings
                 BEGIN SELECT RAISE(FAIL, 'forced binding failure'); END;",
            )
            .unwrap();
        }
        let mut input = provider(
            "atomic",
            BillingKind::Subscription,
            vec![TokenSource::SessionLog],
        );
        input.session_source_bindings = Some(vec!["claude".to_string()]);

        assert!(db.save_usage_provider(&input).is_err());
        assert!(db.get_usage_provider("atomic").unwrap().is_none());
        assert!(db.get_usage_source_binding("claude").unwrap().is_none());
    }

    #[test]
    fn binding_mutation_waits_for_in_flight_bound_operation() {
        let db = Arc::new(Database::memory().unwrap());
        let mut original = provider(
            "original",
            BillingKind::Subscription,
            vec![TokenSource::SessionLog],
        );
        original.session_source_bindings = Some(vec!["claude".to_string()]);
        db.save_usage_provider(&original).unwrap();
        let replacement = provider(
            "replacement",
            BillingKind::Subscription,
            vec![TokenSource::SessionLog],
        );
        db.save_usage_provider(&replacement).unwrap();

        let (operation_started_tx, operation_started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let operation_db = db.clone();
        let operation = std::thread::spawn(move || {
            operation_db
                .with_bound_usage_source("claude", "original", || {
                    operation_started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                })
                .unwrap()
        });
        operation_started_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        let (mutation_done_tx, mutation_done_rx) = mpsc::channel();
        let mutation_db = db.clone();
        let mutation = std::thread::spawn(move || {
            mutation_db
                .set_usage_source_binding("claude", "replacement")
                .unwrap();
            mutation_done_tx.send(()).unwrap();
        });
        assert!(mutation_done_rx
            .recv_timeout(Duration::from_millis(50))
            .is_err());

        release_tx.send(()).unwrap();
        assert_eq!(operation.join().unwrap(), Some(()));
        mutation.join().unwrap();
        mutation_done_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        assert_eq!(
            db.get_usage_source_binding("claude")
                .unwrap()
                .unwrap()
                .provider_id,
            "replacement"
        );
    }
}
