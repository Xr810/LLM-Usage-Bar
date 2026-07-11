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

fn provider_view(provider: &UsageProviderStored) -> UsageProviderView {
    let route_base_url = provider.route_config.as_ref().and_then(|config| {
        config
            .get("base_url")
            .or_else(|| config.get("baseUrl"))
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    let has_route_credentials = provider.route_config.as_ref().is_some_and(|config| {
        [
            "api_key",
            "apiKey",
            "token",
            "access_token",
            "accessToken",
            "auth_token",
            "authToken",
        ]
        .iter()
        .any(|key| config.get(key).is_some_and(has_non_empty_value))
    });

    UsageProviderView {
        id: provider.id.clone(),
        name: provider.name.clone(),
        billing_kind: provider.billing_kind,
        product_group_id: provider.product_group_id.clone(),
        token_sources: provider.token_sources.clone(),
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

impl Database {
    pub fn list_usage_providers(&self) -> Result<Vec<UsageProviderView>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut statement = conn.prepare(&format!(
            "SELECT {PROVIDER_COLUMNS} FROM usage_providers ORDER BY id"
        ))?;
        let providers = statement
            .query_map([], provider_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(providers.iter().map(provider_view).collect())
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

        let conn = lock_conn!(self.conn);
        conn.execute(
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

        let stored = conn.query_row(
            &format!("SELECT {PROVIDER_COLUMNS} FROM usage_providers WHERE id = ?1"),
            [&input.id],
            provider_from_row,
        )?;
        Ok(provider_view(&stored))
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
}

#[cfg(test)]
mod tests {
    use crate::database::Database;
    use crate::usage::domain::{BillingKind, TokenSource, UsageProviderInput};
    use serde_json::json;

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
}
