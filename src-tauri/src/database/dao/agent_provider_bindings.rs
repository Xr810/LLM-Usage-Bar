use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::provider::{Provider, ProviderMeta};
use crate::usage::domain::{
    AgentProviderBindingInput, AgentProviderBindingView, BillingKind, BindingCredentialStatus,
    TokenSource,
};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BindingAuthMode {
    SessionOnly,
    DirectApiKey,
    ManagedAuth,
    Unsupported,
}

pub(crate) struct BindingRecord {
    pub(crate) id: String,
    pub(crate) agent_module_id: String,
    pub(crate) provider_id: String,
    pub(crate) enabled: bool,
    pub(crate) fingerprint: Option<Vec<u8>>,
    pub(crate) credential_slot: Option<String>,
    pub(crate) credential_version: i64,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) provider_enabled: bool,
    billing_kind: BillingKind,
    token_sources: Vec<TokenSource>,
    pub(crate) route_app_type: Option<String>,
    pub(crate) route_config: Option<Value>,
    pub(crate) quota_config: Option<Value>,
    legacy_settings_config: Option<Value>,
    legacy_meta: Option<Value>,
    pub(crate) agent_archived_at: Option<i64>,
    pub(crate) provider_name: String,
    pub(crate) product_group_id: String,
    pub(crate) legacy_migration_linked: bool,
    pub(crate) legacy_provider_id: Option<String>,
    pub(crate) legacy_provider: Option<Provider>,
}

fn public_error(code: &'static str) -> AppError {
    AppError::Message(code.to_string())
}

fn now_timestamp() -> Result<i64, AppError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|error| AppError::Database(format!("system clock before unix epoch: {error}")))
}

fn parse_billing_kind(raw: String, column: usize) -> rusqlite::Result<BillingKind> {
    match raw.as_str() {
        "subscription" => Ok(BillingKind::Subscription),
        "metered" => Ok(BillingKind::Metered),
        _ => Err(rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            "invalid billing kind".into(),
        )),
    }
}

fn parse_json<T: serde::de::DeserializeOwned>(
    raw: Option<String>,
    column: usize,
) -> rusqlite::Result<Option<T>> {
    raw.map(|value| {
        serde_json::from_str(&value).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                column,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
    })
    .transpose()
}

pub(crate) fn binding_record_from_row(row: &Row<'_>) -> rusqlite::Result<BindingRecord> {
    let token_sources = serde_json::from_str(&row.get::<_, String>(11)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(11, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let legacy_settings_config: Option<Value> = parse_json(row.get(15)?, 15)?;
    let legacy_meta: Option<Value> = parse_json(row.get(16)?, 16)?;
    let legacy_provider_id: Option<String> = row.get(19)?;
    let legacy_provider = legacy_provider_id
        .map(|id| {
            let settings_config = legacy_settings_config.clone().ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    15,
                    rusqlite::types::Type::Null,
                    "legacy provider settings missing".into(),
                )
            })?;
            let meta = legacy_meta
                .clone()
                .map(serde_json::from_value::<ProviderMeta>)
                .transpose()
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        16,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
            Ok::<Provider, rusqlite::Error>(Provider {
                id,
                name: row.get(20)?,
                settings_config,
                website_url: row.get(21)?,
                category: row.get(22)?,
                created_at: row.get(23)?,
                sort_index: row.get(24)?,
                notes: row.get(25)?,
                icon: row.get(26)?,
                icon_color: row.get(27)?,
                meta,
                in_failover_queue: row.get(28)?,
            })
        })
        .transpose()?;
    Ok(BindingRecord {
        id: row.get(0)?,
        agent_module_id: row.get(1)?,
        provider_id: row.get(2)?,
        enabled: row.get(3)?,
        fingerprint: row.get(4)?,
        credential_slot: row.get(5)?,
        credential_version: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        provider_enabled: row.get(9)?,
        billing_kind: parse_billing_kind(row.get(10)?, 10)?,
        token_sources,
        route_app_type: row.get(12)?,
        route_config: parse_json(row.get(13)?, 13)?,
        quota_config: parse_json(row.get(14)?, 14)?,
        legacy_settings_config,
        legacy_meta,
        agent_archived_at: row.get(17)?,
        provider_name: row.get(18)?,
        legacy_migration_linked: row.get(29)?,
        legacy_provider_id: row.get(30)?,
        product_group_id: row.get(31)?,
        legacy_provider,
    })
}

pub(crate) const BINDING_RECORD_QUERY: &str =
    "SELECT binding.id, binding.agent_module_id, binding.provider_id,
            binding.enabled, binding.api_key_fingerprint, binding.credential_slot,
            binding.credential_version, binding.created_at, binding.updated_at,
            provider.enabled, provider.billing_kind, provider.token_sources,
            provider.route_app_type, provider.route_config, provider.quota_config,
            legacy.settings_config, legacy.meta, agent.archived_at, provider.name,
            legacy.id, legacy.name, legacy.website_url, legacy.category,
            legacy.created_at, legacy.sort_index, legacy.notes, legacy.icon,
            legacy.icon_color, legacy.in_failover_queue,
            (provider.legacy_app_type IS NOT NULL
             AND provider.legacy_provider_id IS NOT NULL),
            provider.legacy_provider_id, provider.product_group_id
     FROM agent_provider_bindings AS binding
     JOIN usage_providers AS provider ON provider.id = binding.provider_id
     JOIN agent_modules AS agent ON agent.id = binding.agent_module_id
     LEFT JOIN providers AS legacy
       ON legacy.id = provider.legacy_provider_id
      AND legacy.app_type = provider.legacy_app_type";

fn normalized_auth_marker(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub(crate) fn resolve_direct_api_format(route_config: &Value) -> Option<&'static str> {
    let object = route_config.as_object()?;
    let mut resolved = None;
    for value in object
        .iter()
        .filter(|(key, _)| normalized_auth_marker(key) == "apiformat")
        .map(|(_, value)| value)
    {
        let candidate = match normalized_auth_marker(value.as_str()?).as_str() {
            "anthropic" => "anthropic",
            "openaichat" => "openai_chat",
            "openairesponses" => "openai_responses",
            "gemininative" => "gemini_native",
            _ => return None,
        };
        match resolved {
            Some(existing) if existing != candidate => return None,
            Some(_) => {}
            None => resolved = Some(candidate),
        }
    }
    resolved
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectCredentialPlacement {
    AuthorizationBearer,
    XApiKey,
    XGoogApiKey,
}

#[derive(Default)]
struct AuthInspection {
    managed: bool,
    unsupported: bool,
    explicit_placements: Vec<DirectCredentialPlacement>,
    credential_field_placements: Vec<DirectCredentialPlacement>,
    route_hosts: Vec<String>,
}

fn is_managed_auth_marker(value: &str) -> bool {
    matches!(
        normalized_auth_marker(value).as_str(),
        "oauth"
            | "googleoauth"
            | "oauthpersonal"
            | "managedaccount"
            | "githubcopilot"
            | "codexoauth"
            | "geminioauth"
            | "chatgpt"
    )
}

fn default_direct_placement(protocol: &str) -> Option<DirectCredentialPlacement> {
    match protocol {
        "claude" | "claude-desktop" => Some(DirectCredentialPlacement::XApiKey),
        "codex" => Some(DirectCredentialPlacement::AuthorizationBearer),
        "gemini" => Some(DirectCredentialPlacement::XGoogApiKey),
        _ => None,
    }
}

fn direct_marker_placement(
    marker: &str,
    default: DirectCredentialPlacement,
) -> Option<DirectCredentialPlacement> {
    match normalized_auth_marker(marker).as_str() {
        "apikey" | "direct" | "directapikey" | "providerconfig" => Some(default),
        "bearer" | "beareronly" | "token" | "openaiapikey" => {
            Some(DirectCredentialPlacement::AuthorizationBearer)
        }
        "xapikey" | "anthropicapikey" => Some(DirectCredentialPlacement::XApiKey),
        "xgoogapikey" | "geminiapikey" | "googleapikey" => {
            Some(DirectCredentialPlacement::XGoogApiKey)
        }
        _ => None,
    }
}

fn record_auth_marker(
    marker: &str,
    default: DirectCredentialPlacement,
    inspection: &mut AuthInspection,
) {
    if is_managed_auth_marker(marker) {
        inspection.managed = true;
    } else if let Some(placement) = direct_marker_placement(marker, default) {
        inspection.explicit_placements.push(placement);
    } else {
        inspection.unsupported = true;
    }
}

fn string_is_truthy(value: &Value) -> bool {
    match value {
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_i64().is_some_and(|value| value != 0),
        Value::String(value) => matches!(
            normalized_auth_marker(value).as_str(),
            "1" | "true" | "yes" | "on" | "enabled"
        ),
        _ => false,
    }
}

fn inspect_auth_container(
    value: &Value,
    default: DirectCredentialPlacement,
    inspection: &mut AuthInspection,
) {
    match value {
        // Historical Codex route documents use an empty auth object while the
        // actual API-key placement is implied by the protocol.
        Value::Object(object) if object.is_empty() => {}
        Value::Object(_) => {
            let managed_before = inspection.managed;
            let explicit_before = inspection.explicit_placements.len();
            let credential_fields_before = inspection.credential_field_placements.len();
            let unsupported_before = inspection.unsupported;

            inspect_auth_value(value, true, default, inspection);

            let proves_supported_auth = (!managed_before && inspection.managed)
                || inspection.explicit_placements.len() > explicit_before
                || inspection.credential_field_placements.len() > credential_fields_before;
            if !proves_supported_auth && inspection.unsupported == unsupported_before {
                inspection.unsupported = true;
            }
        }
        Value::String(marker) if !marker.trim().is_empty() => {
            record_auth_marker(marker, default, inspection);
        }
        _ => inspection.unsupported = true,
    }
}

fn inspect_auth_value(
    value: &Value,
    auth_context: bool,
    default: DirectCredentialPlacement,
    inspection: &mut AuthInspection,
) {
    let Value::Object(object) = value else {
        if let Value::Array(values) = value {
            for value in values {
                inspect_auth_value(value, auth_context, default, inspection);
            }
        }
        return;
    };

    for (key, value) in object {
        let key = normalized_auth_marker(key);
        let is_auth_container = matches!(key.as_str(), "auth" | "authentication" | "authbinding");

        if is_auth_container {
            inspect_auth_container(value, default, inspection);
            continue;
        }
        let child_auth_context = auth_context;

        if key == "partnerpromotionkey"
            && value
                .as_str()
                .is_some_and(|marker| normalized_auth_marker(marker) == "googleofficial")
        {
            inspection.managed = true;
        }

        if key == "providertype" {
            match value
                .as_str()
                .map(normalized_auth_marker)
                .filter(|marker| !marker.is_empty())
            {
                Some(marker) if is_managed_auth_marker(&marker) => inspection.managed = true,
                Some(marker) if matches!(marker.as_str(), "newapi" | "customgateway") => {
                    inspection.explicit_placements.push(default);
                }
                _ => inspection.unsupported = true,
            }
        }

        let is_auth_marker = key == "authmode"
            || key == "authtype"
            || key == "authprovider"
            || key == "selectedtype"
            || (child_auth_context
                && matches!(
                    key.as_str(),
                    "type" | "strategy" | "scheme" | "method" | "mode" | "source"
                ));
        if is_auth_marker {
            match value.as_str().filter(|marker| !marker.trim().is_empty()) {
                Some(marker) => record_auth_marker(marker, default, inspection),
                None => inspection.unsupported = true,
            }
        }

        if key == "type"
            && value
                .as_str()
                .is_some_and(|marker| normalized_auth_marker(marker) == "serviceaccount")
        {
            inspection.unsupported = true;
        }

        if key == "apikeyfield" {
            if let Some(field) = value.as_str().filter(|field| !field.trim().is_empty()) {
                match normalized_auth_marker(field).as_str() {
                    "anthropicauthtoken" => inspection
                        .explicit_placements
                        .push(DirectCredentialPlacement::AuthorizationBearer),
                    "anthropicapikey" => inspection
                        .explicit_placements
                        .push(DirectCredentialPlacement::XApiKey),
                    "geminiapikey" | "googleapikey" => inspection
                        .explicit_placements
                        .push(DirectCredentialPlacement::XGoogApiKey),
                    "openaiapikey" => inspection
                        .explicit_placements
                        .push(DirectCredentialPlacement::AuthorizationBearer),
                    _ => inspection.unsupported = true,
                }
            }
        }

        match key.as_str() {
            "anthropicauthtoken" => inspection
                .credential_field_placements
                .push(DirectCredentialPlacement::AuthorizationBearer),
            "anthropicapikey" => inspection
                .credential_field_placements
                .push(DirectCredentialPlacement::XApiKey),
            "openaiapikey" => inspection
                .credential_field_placements
                .push(DirectCredentialPlacement::AuthorizationBearer),
            "geminiapikey" | "googleapikey" => inspection
                .credential_field_placements
                .push(DirectCredentialPlacement::XGoogApiKey),
            "claudecodeusebedrock" | "claudecodeusevertex" if string_is_truthy(value) => {
                inspection.unsupported = true;
            }
            "awsaccesskeyid"
            | "awssecretaccesskey"
            | "awssessiontoken"
            | "awsbearertokenbedrock"
            | "googleapplicationcredentials"
            | "serviceaccountcredentials" => inspection.unsupported = true,
            "baseurl" | "anthropicbaseurl" | "googlegeminibaseurl" => {
                if let Some(url) = value.as_str() {
                    if let Ok(url) = reqwest::Url::parse(url) {
                        if let Some(host) = url.host_str() {
                            inspection.route_hosts.push(host.to_ascii_lowercase());
                        }
                    }
                }
            }
            _ => {}
        }

        inspect_auth_value(value, child_auth_context, default, inspection);
    }
}

fn inspect_auth(value: &Value, default: DirectCredentialPlacement) -> AuthInspection {
    let mut inspection = AuthInspection::default();
    inspect_auth_value(value, false, default, &mut inspection);
    if inspection.route_hosts.iter().any(|host| {
        host == "githubcopilot.com"
            || host.ends_with(".githubcopilot.com")
            || host == "chatgpt.com"
            || host.ends_with(".chatgpt.com")
    }) {
        inspection.managed = true;
    }
    inspection
}

fn value_has_unsupported_explicit_auth(value: &Value) -> bool {
    inspect_auth(value, DirectCredentialPlacement::XApiKey).unsupported
}

fn value_proves_managed_auth(value: &Value) -> bool {
    inspect_auth(value, DirectCredentialPlacement::XApiKey).managed
}

pub(crate) fn resolve_direct_credential_placement(
    protocol: &str,
    route_config: &Value,
) -> Option<DirectCredentialPlacement> {
    let default = default_direct_placement(protocol)?;
    let has_top_level_api_format = route_config.as_object().is_some_and(|object| {
        object
            .keys()
            .any(|key| normalized_auth_marker(key) == "apiformat")
    });
    let api_format = resolve_direct_api_format(route_config);
    if has_top_level_api_format && api_format.is_none() {
        return None;
    }
    let inspection = inspect_auth(route_config, default);
    if inspection.managed || inspection.unsupported {
        return None;
    }

    let mut explicit_placements = inspection.explicit_placements;
    if api_format == Some("gemini_native") {
        explicit_placements.push(DirectCredentialPlacement::XGoogApiKey);
    }

    // Explicit route semantics (authMode/apiKeyField/apiFormat) are
    // authoritative. Credential-bearing env fields are legacy-shaped storage
    // that will be scrubbed before forwarding, so they are only placement
    // evidence when no explicit route marker exists.
    let candidates = if explicit_placements.is_empty() {
        inspection.credential_field_placements
    } else {
        explicit_placements
    };
    let mut placement = None;
    for candidate in candidates {
        match placement {
            Some(existing) if existing != candidate => return None,
            Some(_) => {}
            None => placement = Some(candidate),
        }
    }
    if placement.is_none()
        && matches!(protocol, "claude" | "claude-desktop")
        && inspection
            .route_hosts
            .iter()
            .any(|host| host == "openrouter.ai" || host.ends_with(".openrouter.ai"))
    {
        placement = Some(DirectCredentialPlacement::AuthorizationBearer);
    }
    Some(placement.unwrap_or(default))
}

pub(crate) fn binding_auth_mode(record: &BindingRecord) -> BindingAuthMode {
    let has_session = record.token_sources.contains(&TokenSource::SessionLog);
    let has_proxy = record.token_sources.contains(&TokenSource::Proxy);
    let is_managed_auth = [
        record.route_config.as_ref(),
        record.quota_config.as_ref(),
        record.legacy_settings_config.as_ref(),
        record.legacy_meta.as_ref(),
    ]
    .into_iter()
    .flatten()
    .any(value_proves_managed_auth);
    if is_managed_auth {
        return BindingAuthMode::ManagedAuth;
    }
    let has_unsupported_explicit_auth = [
        record.route_config.as_ref(),
        record.quota_config.as_ref(),
        record.legacy_settings_config.as_ref(),
        record.legacy_meta.as_ref(),
    ]
    .into_iter()
    .flatten()
    .any(value_has_unsupported_explicit_auth);
    if has_unsupported_explicit_auth {
        return BindingAuthMode::Unsupported;
    }
    if !has_proxy {
        return if record.billing_kind == BillingKind::Subscription && has_session {
            BindingAuthMode::SessionOnly
        } else {
            BindingAuthMode::Unsupported
        };
    }
    if record
        .route_app_type
        .as_deref()
        .zip(record.route_config.as_ref())
        .and_then(|(protocol, route_config)| {
            resolve_direct_credential_placement(protocol, route_config)
        })
        .is_some()
    {
        BindingAuthMode::DirectApiKey
    } else {
        BindingAuthMode::Unsupported
    }
}

fn credential_status(
    record: &BindingRecord,
    auth_mode: BindingAuthMode,
) -> BindingCredentialStatus {
    match auth_mode {
        BindingAuthMode::SessionOnly | BindingAuthMode::ManagedAuth => {
            if record.fingerprint.is_none() && record.credential_slot.is_none() {
                BindingCredentialStatus::NotRequired
            } else {
                BindingCredentialStatus::Unavailable
            }
        }
        BindingAuthMode::DirectApiKey => {
            if record.fingerprint.is_none() && record.credential_slot.is_none() {
                BindingCredentialStatus::Missing
            } else {
                // Task 3 upgrades this to Configured only after the protected item
                // has been loaded and its fingerprint verified.
                BindingCredentialStatus::Unavailable
            }
        }
        BindingAuthMode::Unsupported => BindingCredentialStatus::Unavailable,
    }
}

fn binding_view(record: &BindingRecord) -> Result<AgentProviderBindingView, AppError> {
    let auth_mode = binding_auth_mode(record);
    let credential_status = credential_status(record, auth_mode);
    let credential_version = u64::try_from(record.credential_version)
        .map_err(|_| AppError::Database("negative credential version".to_string()))?;
    // `effective_enabled` is proxy routability. Until Task 3 can verify the
    // protected credential, no binding is effectively routable.
    let effective_enabled = record.enabled
        && record.provider_enabled
        && record.agent_archived_at.is_none()
        && auth_mode == BindingAuthMode::DirectApiKey
        && credential_status == BindingCredentialStatus::Configured;
    Ok(AgentProviderBindingView {
        id: record.id.clone(),
        agent_module_id: record.agent_module_id.clone(),
        provider_id: record.provider_id.clone(),
        enabled: record.enabled,
        effective_enabled,
        credential_status,
        credential_version,
        created_at: record.created_at,
        updated_at: record.updated_at,
    })
}

fn query_binding_records(
    conn: &Connection,
    predicate: &str,
    parameter: Option<&str>,
) -> Result<Vec<BindingRecord>, AppError> {
    let sql = format!(
        "{BINDING_RECORD_QUERY}{predicate}
         ORDER BY binding.agent_module_id, binding.provider_id, binding.id"
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = match parameter {
        Some(value) => statement
            .query_map([value], binding_record_from_row)?
            .collect::<Result<Vec<_>, _>>()?,
        None => statement
            .query_map([], binding_record_from_row)?
            .collect::<Result<Vec<_>, _>>()?,
    };
    Ok(rows)
}

pub(crate) fn bindings_for_provider_on_conn(
    conn: &Connection,
    provider_id: &str,
) -> Result<Vec<AgentProviderBindingView>, AppError> {
    query_binding_records(conn, " WHERE binding.provider_id = ?1", Some(provider_id))?
        .iter()
        .map(binding_view)
        .collect()
}

fn binding_record_by_id(
    conn: &Connection,
    binding_id: &str,
) -> Result<Option<BindingRecord>, AppError> {
    Ok(
        query_binding_records(conn, " WHERE binding.id = ?1", Some(binding_id))?
            .into_iter()
            .next(),
    )
}

#[allow(dead_code)] // Consumed by the protected credential lifecycle in Task 3.
pub(crate) fn binding_auth_mode_for_id_on_conn(
    conn: &Connection,
    binding_id: &str,
) -> Result<Option<BindingAuthMode>, AppError> {
    binding_record_by_id(conn, binding_id).map(|record| record.as_ref().map(binding_auth_mode))
}

fn provider_context_for_new_binding(
    conn: &Connection,
    agent_module_id: &str,
    provider_id: &str,
) -> Result<BindingRecord, AppError> {
    let row = conn
        .query_row(
            "SELECT provider.enabled, provider.billing_kind, provider.token_sources,
                    provider.route_app_type, provider.route_config, provider.quota_config,
                    (SELECT legacy.settings_config FROM providers AS legacy
                     WHERE legacy.id = provider.legacy_provider_id
                       AND legacy.app_type = provider.legacy_app_type),
                    (SELECT legacy.meta FROM providers AS legacy
                     WHERE legacy.id = provider.legacy_provider_id
                       AND legacy.app_type = provider.legacy_app_type),
                    agent.archived_at,
                    (provider.legacy_app_type IS NOT NULL
                     AND provider.legacy_provider_id IS NOT NULL),
                    provider.legacy_provider_id, provider.product_group_id
             FROM usage_providers AS provider
             JOIN agent_modules AS agent ON agent.id = ?1
             WHERE provider.id = ?2",
            params![agent_module_id, provider_id],
            |row| {
                let token_sources =
                    serde_json::from_str(&row.get::<_, String>(2)?).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            2,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?;
                Ok(BindingRecord {
                    id: String::new(),
                    agent_module_id: agent_module_id.to_string(),
                    provider_id: provider_id.to_string(),
                    enabled: false,
                    fingerprint: None,
                    credential_slot: None,
                    credential_version: 0,
                    created_at: 0,
                    updated_at: 0,
                    provider_enabled: row.get(0)?,
                    billing_kind: parse_billing_kind(row.get(1)?, 1)?,
                    token_sources,
                    route_app_type: row.get(3)?,
                    route_config: parse_json(row.get(4)?, 4)?,
                    quota_config: parse_json(row.get(5)?, 5)?,
                    legacy_settings_config: parse_json(row.get(6)?, 6)?,
                    legacy_meta: parse_json(row.get(7)?, 7)?,
                    agent_archived_at: row.get(8)?,
                    provider_name: String::new(),
                    product_group_id: row.get(11)?,
                    legacy_migration_linked: row.get(9)?,
                    legacy_provider_id: row.get(10)?,
                    legacy_provider: None,
                })
            },
        )
        .optional()?
        .ok_or_else(|| public_error("invalid_binding"))?;
    if row.agent_archived_at.is_some() {
        return Err(public_error("invalid_binding"));
    }
    Ok(row)
}

fn validate_requested_enabled(record: &BindingRecord, enabled: bool) -> Result<(), AppError> {
    if !enabled {
        return Ok(());
    }
    match binding_auth_mode(record) {
        BindingAuthMode::SessionOnly => Ok(()),
        BindingAuthMode::ManagedAuth if record.token_sources.contains(&TokenSource::SessionLog) => {
            Ok(())
        }
        BindingAuthMode::DirectApiKey => {
            if record.fingerprint.is_none() {
                Err(public_error("credential_required"))
            } else {
                // `enabled` is the requested state. Protected-store verification
                // happens asynchronously in BindingCredentialService and controls
                // `effective_enabled`; the synchronous DAO never opens Keychain.
                Ok(())
            }
        }
        BindingAuthMode::ManagedAuth | BindingAuthMode::Unsupported => {
            Err(public_error("invalid_binding"))
        }
    }
}

impl Database {
    pub fn list_agent_provider_bindings(
        &self,
        agent_module_id: Option<&str>,
    ) -> Result<Vec<AgentProviderBindingView>, AppError> {
        let conn = lock_conn!(self.conn);
        let records = match agent_module_id {
            Some(agent_module_id) => query_binding_records(
                &conn,
                " WHERE binding.agent_module_id = ?1",
                Some(agent_module_id),
            )?,
            None => query_binding_records(&conn, "", None)?,
        };
        records.iter().map(binding_view).collect()
    }

    pub fn save_agent_provider_binding(
        &self,
        input: &AgentProviderBindingInput,
    ) -> Result<AgentProviderBindingView, AppError> {
        if input.agent_module_id.trim().is_empty() || input.provider_id.trim().is_empty() {
            return Err(public_error("invalid_binding"));
        }
        let now = now_timestamp()?;
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction()?;
        let id = if let Some(id) = input.id.as_deref() {
            let record = binding_record_by_id(&transaction, id)?
                .ok_or_else(|| public_error("binding_not_found"))?;
            if record.agent_module_id != input.agent_module_id
                || record.provider_id != input.provider_id
                || record.agent_archived_at.is_some()
            {
                return Err(public_error("invalid_binding"));
            }
            validate_requested_enabled(&record, input.enabled)?;
            if transaction.execute(
                "UPDATE agent_provider_bindings SET enabled = ?2, updated_at = ?3
                 WHERE id = ?1",
                params![id, input.enabled, now],
            )? != 1
            {
                return Err(public_error("binding_not_found"));
            }
            id.to_string()
        } else {
            let record = provider_context_for_new_binding(
                &transaction,
                &input.agent_module_id,
                &input.provider_id,
            )?;
            validate_requested_enabled(&record, input.enabled)?;
            let duplicate: bool = transaction.query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM agent_provider_bindings
                     WHERE agent_module_id = ?1 AND provider_id = ?2
                 )",
                params![input.agent_module_id, input.provider_id],
                |row| row.get(0),
            )?;
            if duplicate {
                return Err(public_error("invalid_binding"));
            }
            let id = uuid::Uuid::new_v4().to_string();
            transaction.execute(
                "INSERT INTO agent_provider_bindings (
                     id, agent_module_id, provider_id, enabled,
                     api_key_fingerprint, credential_slot, credential_version,
                     created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, NULL, NULL, 0, ?5, ?5)",
                params![
                    id,
                    input.agent_module_id,
                    input.provider_id,
                    input.enabled,
                    now
                ],
            )?;
            id
        };
        let record = binding_record_by_id(&transaction, &id)?
            .ok_or_else(|| public_error("binding_not_found"))?;
        transaction.execute(
            "UPDATE agent_modules SET ever_bound = 1 WHERE id = ?1",
            [&record.agent_module_id],
        )?;
        let view = binding_view(&record)?;
        transaction.commit()?;
        Ok(view)
    }

    pub fn delete_agent_provider_binding_metadata(
        &self,
        binding_id: &str,
        expected_version: u64,
    ) -> Result<(), AppError> {
        let expected_version =
            i64::try_from(expected_version).map_err(|_| public_error("credential_conflict"))?;
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction()?;
        let state = transaction
            .query_row(
                "SELECT credential_version, api_key_fingerprint, credential_slot,
                        EXISTS(
                            SELECT 1 FROM agent_credential_operations
                            WHERE binding_id = agent_provider_bindings.id
                        )
                 FROM agent_provider_bindings WHERE id = ?1",
                [binding_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<Vec<u8>>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, bool>(3)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| public_error("binding_not_found"))?;
        if state.0 != expected_version || state.1.is_some() || state.2.is_some() || state.3 {
            return Err(public_error("credential_conflict"));
        }
        if transaction.execute(
            "DELETE FROM agent_provider_bindings
             WHERE id = ?1 AND credential_version = ?2",
            params![binding_id, expected_version],
        )? != 1
        {
            return Err(public_error("credential_conflict"));
        }
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::database::Database;
    use crate::usage::domain::{
        AgentModuleInput, AgentProviderBindingInput, BindingCredentialStatus,
    };
    use rusqlite::params;
    use serde_json::Value;

    #[allow(clippy::too_many_arguments)]
    fn insert_provider(
        db: &Database,
        id: &str,
        billing_kind: &str,
        token_sources: &str,
        route_app_type: Option<&str>,
        enabled: bool,
        route_config: Option<&str>,
        quota_config: Option<&str>,
    ) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO usage_providers (
                 id, name, billing_kind, product_group_id, token_sources,
                 route_app_type, route_config, quota_config, enabled, needs_review,
                 created_at, updated_at
             ) VALUES (?1, ?1, ?2, 'test', ?3, ?4, ?5, ?6, ?7, 0, 10, 10)",
            params![
                id,
                billing_kind,
                token_sources,
                route_app_type,
                route_config,
                quota_config,
                enabled
            ],
        )
        .unwrap();
    }

    fn input(agent_module_id: &str, provider_id: &str, enabled: bool) -> AgentProviderBindingInput {
        AgentProviderBindingInput {
            id: None,
            agent_module_id: agent_module_id.to_string(),
            provider_id: provider_id.to_string(),
            enabled,
        }
    }

    #[test]
    fn session_only_binding_may_be_requested_enabled_without_a_credential() {
        let db = Database::memory().unwrap();
        insert_provider(
            &db,
            "session",
            "subscription",
            "[\"session_log\"]",
            None,
            true,
            None,
            None,
        );
        let binding = db
            .save_agent_provider_binding(&input("claude-code", "session", true))
            .unwrap();
        assert!(binding.enabled);
        assert!(!binding.effective_enabled);
        assert_eq!(
            binding.credential_status,
            BindingCredentialStatus::NotRequired
        );
        assert_eq!(binding.credential_version, 0);
    }

    #[test]
    fn direct_api_binding_requires_a_protected_credential_before_enabling() {
        let db = Database::memory().unwrap();
        insert_provider(
            &db,
            "direct",
            "metered",
            "[\"proxy\"]",
            Some("claude"),
            true,
            Some("{\"baseUrl\":\"https://api.example\"}"),
            None,
        );
        assert_eq!(
            db.save_agent_provider_binding(&input("codex", "direct", true))
                .unwrap_err()
                .to_string(),
            "credential_required"
        );
        assert!(db
            .list_agent_provider_bindings(Some("codex"))
            .unwrap()
            .is_empty());

        let binding = db
            .save_agent_provider_binding(&input("codex", "direct", false))
            .unwrap();
        assert_eq!(binding.credential_status, BindingCredentialStatus::Missing);
        assert!(!binding.effective_enabled);
    }

    #[test]
    fn bindings_are_many_to_many_but_pairs_are_unique_and_stable() {
        let db = Database::memory().unwrap();
        for id in ["one", "two"] {
            insert_provider(
                &db,
                id,
                "subscription",
                "[\"session_log\"]",
                None,
                true,
                None,
                None,
            );
        }
        let first = db
            .save_agent_provider_binding(&input("codex", "one", true))
            .unwrap();
        db.save_agent_provider_binding(&input("claude-code", "one", true))
            .unwrap();
        db.save_agent_provider_binding(&input("codex", "two", true))
            .unwrap();
        assert_eq!(db.list_agent_provider_bindings(None).unwrap().len(), 3);
        assert_eq!(
            db.save_agent_provider_binding(&input("codex", "one", true))
                .unwrap_err()
                .to_string(),
            "invalid_binding"
        );

        let error = db
            .save_agent_provider_binding(&AgentProviderBindingInput {
                id: Some(first.id),
                agent_module_id: "claude-code".to_string(),
                provider_id: "one".to_string(),
                enabled: true,
            })
            .unwrap_err();
        assert_eq!(error.to_string(), "invalid_binding");
    }

    #[test]
    fn fingerprint_constraint_is_global_and_public_view_contains_no_private_hints() {
        let db = Database::memory().unwrap();
        for id in ["one", "two"] {
            insert_provider(
                &db,
                id,
                "metered",
                "[\"proxy\"]",
                Some("claude"),
                true,
                None,
                None,
            );
        }
        let one = db
            .save_agent_provider_binding(&input("codex", "one", false))
            .unwrap();
        let two = db
            .save_agent_provider_binding(&input("codex", "two", false))
            .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            let fingerprint = vec![7_u8; 32];
            conn.execute(
                "UPDATE agent_provider_bindings
                 SET api_key_fingerprint = ?2, credential_slot = 'slot-one',
                     credential_version = 1 WHERE id = ?1",
                params![one.id, fingerprint],
            )
            .unwrap();
            assert!(conn
                .execute(
                    "UPDATE agent_provider_bindings
                     SET api_key_fingerprint = ?2, credential_slot = 'slot-two',
                         credential_version = 1 WHERE id = ?1",
                    params![two.id, vec![7_u8; 32]],
                )
                .is_err());
        }
        let view = db
            .list_agent_provider_bindings(Some("codex"))
            .unwrap()
            .into_iter()
            .find(|binding| binding.id == one.id)
            .unwrap();
        assert_eq!(view.credential_status, BindingCredentialStatus::Unavailable);
        let json = serde_json::to_value(view).unwrap();
        let object = json.as_object().unwrap();
        for forbidden in [
            "apiKeyFingerprint",
            "credentialSlot",
            "fingerprint",
            "maskedKey",
            "keyHint",
        ] {
            assert!(!object.contains_key(forbidden));
        }
        assert!(!serde_json::to_string(&json).unwrap().contains("slot-one"));
    }

    #[test]
    fn managed_auth_is_display_only_and_never_accepts_binding_key_enablement() {
        let db = Database::memory().unwrap();
        insert_provider(
            &db,
            "managed",
            "subscription",
            "[\"session_log\",\"proxy\"]",
            Some("codex"),
            true,
            Some("{\"env\":{\"ANTHROPIC_BASE_URL\":\"https://chatgpt.com/backend-api/codex\"}}"),
            Some("{\"providerType\":\"codex_oauth\"}"),
        );
        let binding = db
            .save_agent_provider_binding(&input("codex", "managed", true))
            .unwrap();
        assert_eq!(
            binding.credential_status,
            BindingCredentialStatus::NotRequired
        );
        assert!(!binding.effective_enabled);
    }

    #[test]
    fn session_only_managed_provider_keeps_managed_auth_classification() {
        let db = Database::memory().unwrap();
        insert_provider(
            &db,
            "managed-session-only",
            "subscription",
            "[\"session_log\"]",
            Some("codex"),
            true,
            None,
            Some("{\"providerType\":\"codex_oauth\"}"),
        );
        let binding = db
            .save_agent_provider_binding(&input("codex", "managed-session-only", true))
            .unwrap();
        let conn = db.conn.lock().unwrap();
        assert_eq!(
            super::binding_auth_mode_for_id_on_conn(&conn, &binding.id).unwrap(),
            Some(super::BindingAuthMode::ManagedAuth)
        );
    }

    #[test]
    fn managed_account_auth_binding_wins_for_enterprise_urls_without_provider_type() {
        let db = Database::memory().unwrap();
        insert_provider(
            &db,
            "enterprise-managed",
            "subscription",
            "[\"session_log\",\"proxy\"]",
            Some("claude"),
            true,
            Some("{\"env\":{\"ANTHROPIC_BASE_URL\":\"https://llm.example.corp/v1\"}}"),
            Some(
                "{\"authMode\":\"api_key\",\"authBinding\":{\"source\":\"managed_account\",\"authProvider\":\"github_copilot\"}}",
            ),
        );
        let binding = db
            .save_agent_provider_binding(&input("claude-code", "enterprise-managed", true))
            .unwrap();
        assert_eq!(
            binding.credential_status,
            BindingCredentialStatus::NotRequired
        );
        assert!(!binding.effective_enabled);

        let conn = db.conn.lock().unwrap();
        assert_eq!(
            super::binding_auth_mode_for_id_on_conn(&conn, &binding.id).unwrap(),
            Some(super::BindingAuthMode::ManagedAuth)
        );
    }

    #[test]
    fn codex_and_gemini_repository_oauth_markers_are_managed_on_enterprise_urls() {
        for (provider_id, route_app_type, route_config) in [
            (
                "codex-enterprise-oauth",
                "codex",
                "{\"baseUrl\":\"https://codex.example.corp/v1\",\"auth_mode\":\"chatgpt\"}",
            ),
            (
                "gemini-enterprise-oauth",
                "gemini",
                "{\"baseUrl\":\"https://gemini.example.corp/v1\",\"security\":{\"auth\":{\"selectedType\":\"oauth-personal\"}}}",
            ),
        ] {
            let db = Database::memory().unwrap();
            insert_provider(
                &db,
                provider_id,
                "subscription",
                "[\"session_log\",\"proxy\"]",
                Some(route_app_type),
                true,
                Some(route_config),
                None,
            );
            let binding = db
                .save_agent_provider_binding(&input("codex", provider_id, true))
                .unwrap();
            assert_eq!(
                binding.credential_status,
                BindingCredentialStatus::NotRequired,
                "{provider_id}"
            );
            let conn = db.conn.lock().unwrap();
            assert_eq!(
                super::binding_auth_mode_for_id_on_conn(&conn, &binding.id).unwrap(),
                Some(super::BindingAuthMode::ManagedAuth),
                "{provider_id}"
            );
        }
    }

    #[test]
    fn explicit_direct_api_key_auth_markers_remain_routable() {
        for (provider_id, route_app_type, route_config) in [
            (
                "claude-direct-bearer",
                "claude",
                "{\"baseUrl\":\"https://api.example\",\"auth_mode\":\"bearer_only\"}",
            ),
            (
                "codex-direct-key",
                "codex",
                "{\"baseUrl\":\"https://api.example\",\"authMode\":\"api_key\"}",
            ),
            (
                "gemini-direct-key",
                "gemini",
                "{\"baseUrl\":\"https://api.example\",\"security\":{\"auth\":{\"selectedType\":\"gemini-api-key\"}}}",
            ),
            (
                "provider-config-binding",
                "claude",
                "{\"baseUrl\":\"https://api.example\",\"authBinding\":{\"source\":\"provider_config\"}}",
            ),
        ] {
            let db = Database::memory().unwrap();
            insert_provider(
                &db,
                provider_id,
                "metered",
                "[\"proxy\"]",
                Some(route_app_type),
                true,
                Some(route_config),
                None,
            );
            let binding = db
                .save_agent_provider_binding(&input("codex", provider_id, false))
                .unwrap();
            let conn = db.conn.lock().unwrap();
            assert_eq!(
                super::binding_auth_mode_for_id_on_conn(&conn, &binding.id).unwrap(),
                Some(super::BindingAuthMode::DirectApiKey),
                "{provider_id}"
            );
        }
    }

    #[test]
    fn unsupported_explicit_auth_markers_fail_closed() {
        for (provider_id, route_config) in [
            (
                "basic-auth",
                "{\"baseUrl\":\"https://api.example\",\"auth_mode\":\"basic\"}",
            ),
            (
                "aws-sigv4",
                "{\"baseUrl\":\"https://api.example\",\"authMode\":\"aws_sigv4\"}",
            ),
            (
                "future-oauth",
                "{\"baseUrl\":\"https://api.example\",\"auth\":{\"strategy\":\"future_oauth\"}}",
            ),
            (
                "case-variant-basic-auth",
                "{\"baseUrl\":\"https://api.example\",\"AuthMode\":\"basic\"}",
            ),
            (
                "nested-auth-method",
                "{\"baseUrl\":\"https://api.example\",\"authentication\":{\"method\":\"future_auth\"}}",
            ),
            (
                "bedrock-multi-field-auth",
                "{\"env\":{\"ANTHROPIC_BASE_URL\":\"https://bedrock.example\",\"CLAUDE_CODE_USE_BEDROCK\":\"1\",\"AWS_ACCESS_KEY_ID\":\"old\",\"AWS_SECRET_ACCESS_KEY\":\"old\"}}",
            ),
        ] {
            let db = Database::memory().unwrap();
            insert_provider(
                &db,
                provider_id,
                "metered",
                "[\"proxy\"]",
                Some("claude"),
                true,
                Some(route_config),
                None,
            );
            let binding = db
                .save_agent_provider_binding(&input("claude-code", provider_id, false))
                .unwrap();
            let conn = db.conn.lock().unwrap();
            assert_eq!(
                super::binding_auth_mode_for_id_on_conn(&conn, &binding.id).unwrap(),
                Some(super::BindingAuthMode::Unsupported),
                "{provider_id}"
            );
            drop(conn);
            assert_eq!(
                db.save_agent_provider_binding(&AgentProviderBindingInput {
                    id: Some(binding.id),
                    agent_module_id: "claude-code".to_string(),
                    provider_id: provider_id.to_string(),
                    enabled: true,
                })
                .unwrap_err()
                .to_string(),
                "invalid_binding",
                "{provider_id}"
            );
        }
    }

    #[test]
    fn scalar_authentication_marker_fails_closed() {
        let route_config = serde_json::json!({
            "baseUrl": "https://api.example",
            "authentication": "basic"
        });

        assert_eq!(
            super::resolve_direct_credential_placement("claude", &route_config),
            None
        );
    }

    #[test]
    fn authentication_mode_marker_cannot_be_ignored() {
        let route_config = serde_json::json!({
            "baseUrl": "https://api.example",
            "authentication": {"mode": "basic"}
        });

        assert_eq!(
            super::resolve_direct_credential_placement("claude", &route_config),
            None
        );
    }

    #[test]
    fn structured_auth_marker_cannot_hide_an_unsupported_strategy() {
        let route_config = serde_json::json!({
            "baseUrl": "https://api.example",
            "auth": {"strategy": {"kind": "basic"}}
        });

        assert_eq!(
            super::resolve_direct_credential_placement("claude", &route_config),
            None
        );
    }

    #[test]
    fn opaque_nonempty_auth_container_fails_closed() {
        let route_config = serde_json::json!({
            "baseUrl": "https://api.example",
            "authBinding": {"opaque": {"nested": "value"}}
        });

        assert_eq!(
            super::resolve_direct_credential_placement("claude", &route_config),
            None
        );
    }

    #[test]
    fn malformed_auth_containers_fail_closed() {
        for malformed in [
            Value::Null,
            Value::Bool(true),
            serde_json::json!(7),
            serde_json::json!([]),
            serde_json::json!(["bearer"]),
            serde_json::json!(""),
        ] {
            let route_config = serde_json::json!({
                "baseUrl": "https://api.example",
                "auth": malformed
            });

            assert_eq!(
                super::resolve_direct_credential_placement("claude", &route_config),
                None,
                "{route_config}"
            );
        }
    }

    #[test]
    fn unknown_provider_type_fails_closed() {
        let route_config = serde_json::json!({
            "baseUrl": "https://api.example",
            "providerType": "future_gateway"
        });

        assert_eq!(
            super::resolve_direct_credential_placement("claude", &route_config),
            None
        );
    }

    #[test]
    fn empty_auth_object_keeps_the_legacy_direct_default() {
        let route_config = serde_json::json!({
            "baseUrl": "https://api.example",
            "auth": {}
        });

        assert_eq!(
            super::resolve_direct_credential_placement("codex", &route_config),
            Some(super::DirectCredentialPlacement::AuthorizationBearer)
        );
    }

    #[test]
    fn auth_type_marker_fails_closed() {
        let route_config = serde_json::json!({
            "baseUrl": "https://api.example",
            "authType": "aws_sigv4"
        });

        assert_eq!(
            super::resolve_direct_credential_placement("claude", &route_config),
            None
        );
    }

    #[test]
    fn managed_cloud_provider_types_fail_closed() {
        for provider_type in ["bedrock", "vertex"] {
            let route_config = serde_json::json!({
                "baseUrl": "https://api.example",
                "providerType": provider_type
            });

            assert_eq!(
                super::resolve_direct_credential_placement("claude", &route_config),
                None,
                "{provider_type}"
            );
        }
    }

    #[test]
    fn bare_service_account_document_fails_closed() {
        let route_config = serde_json::json!({
            "baseUrl": "https://api.example",
            "type": "service_account",
            "project_id": "project",
            "private_key": "private",
            "client_email": "service@example.invalid",
            "token_uri": "https://oauth2.googleapis.com/token"
        });

        assert_eq!(
            super::resolve_direct_credential_placement("gemini", &route_config),
            None
        );
    }

    #[test]
    fn nested_api_format_cannot_change_credential_placement() {
        let route_config = serde_json::json!({
            "baseUrl": "https://api.example",
            "transport": {
                "apiFormat": "gemini_native"
            }
        });

        assert_eq!(
            super::resolve_direct_credential_placement("claude", &route_config),
            Some(super::DirectCredentialPlacement::XApiKey)
        );
    }

    #[test]
    fn unknown_top_level_api_format_fails_closed() {
        let route_config = serde_json::json!({
            "baseUrl": "https://api.example",
            "apiFormat": "future_wire_format"
        });

        assert_eq!(
            super::resolve_direct_credential_placement("claude", &route_config),
            None
        );
    }

    #[test]
    fn supported_top_level_api_formats_are_canonicalized() {
        for (raw, expected) in [
            ("ANTHROPIC", "anthropic"),
            ("OpenAI-Chat", "openai_chat"),
            ("openai_responses", "openai_responses"),
            ("GEMINI_NATIVE", "gemini_native"),
        ] {
            let route_config = serde_json::json!({"API_FORMAT": raw});

            assert_eq!(
                super::resolve_direct_api_format(&route_config),
                Some(expected),
                "{raw}"
            );
        }
    }

    #[test]
    fn google_official_promotion_marker_is_managed_auth() {
        let db = Database::memory().unwrap();
        insert_provider(
            &db,
            "google-official",
            "subscription",
            "[\"session_log\",\"proxy\"]",
            Some("gemini"),
            true,
            Some("{\"baseUrl\":\"https://generativelanguage.googleapis.com\"}"),
            Some("{\"partnerPromotionKey\":\"google-official\"}"),
        );
        let binding = db
            .save_agent_provider_binding(&input("opencode", "google-official", false))
            .unwrap();
        let conn = db.conn.lock().unwrap();
        assert_eq!(
            super::binding_auth_mode_for_id_on_conn(&conn, &binding.id).unwrap(),
            Some(super::BindingAuthMode::ManagedAuth)
        );
    }

    #[test]
    fn unknown_explicit_auth_provider_markers_fail_closed() {
        for (provider_id, route_config) in [
            (
                "unknown-auth-provider-type",
                "{\"baseUrl\":\"https://api.example\",\"providerType\":\"aws_sigv4\"}",
            ),
            (
                "unknown-auth-binding-provider",
                "{\"baseUrl\":\"https://api.example\",\"authBinding\":{\"authProvider\":\"future_auth_service\"}}",
            ),
        ] {
            let db = Database::memory().unwrap();
            insert_provider(
                &db,
                provider_id,
                "metered",
                "[\"proxy\"]",
                Some("claude"),
                true,
                Some(route_config),
                None,
            );
            let binding = db
                .save_agent_provider_binding(&input("claude-code", provider_id, false))
                .unwrap();
            let conn = db.conn.lock().unwrap();
            assert_eq!(
                super::binding_auth_mode_for_id_on_conn(&conn, &binding.id).unwrap(),
                Some(super::BindingAuthMode::Unsupported),
                "{provider_id}"
            );
        }
    }

    #[test]
    fn archived_agent_rejects_new_or_updated_bindings() {
        let db = Database::memory().unwrap();
        insert_provider(
            &db,
            "session",
            "subscription",
            "[\"session_log\"]",
            None,
            true,
            None,
            None,
        );
        let custom = db
            .save_agent_module(&AgentModuleInput {
                id: None,
                name: "Archived".to_string(),
                sort_order: 10,
                visible: true,
            })
            .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE agent_modules SET archived_at = 20, visible = 0 WHERE id = ?1",
                [&custom.id],
            )
            .unwrap();
        }
        assert_eq!(
            db.save_agent_provider_binding(&input(&custom.id, "session", true))
                .unwrap_err()
                .to_string(),
            "invalid_binding"
        );
    }

    #[test]
    fn metadata_delete_is_versioned_and_refuses_credential_or_journal_state() {
        let db = Database::memory().unwrap();
        insert_provider(
            &db,
            "direct",
            "metered",
            "[\"proxy\"]",
            Some("claude"),
            true,
            None,
            None,
        );
        let binding = db
            .save_agent_provider_binding(&input("codex", "direct", false))
            .unwrap();
        assert_eq!(
            db.delete_agent_provider_binding_metadata(&binding.id, 1)
                .unwrap_err()
                .to_string(),
            "credential_conflict"
        );
        db.delete_agent_provider_binding_metadata(&binding.id, 0)
            .unwrap();
        assert!(db.list_agent_provider_bindings(None).unwrap().is_empty());
    }

    #[test]
    fn provider_and_agent_deletion_are_restricted_while_a_binding_exists() {
        let db = Database::memory().unwrap();
        insert_provider(
            &db,
            "session",
            "subscription",
            "[\"session_log\"]",
            None,
            true,
            None,
            None,
        );
        let custom = db
            .save_agent_module(&AgentModuleInput {
                id: None,
                name: "Restricted".to_string(),
                sort_order: 10,
                visible: true,
            })
            .unwrap();
        db.save_agent_provider_binding(&input(&custom.id, "session", true))
            .unwrap();
        let conn = db.conn.lock().unwrap();
        assert!(conn
            .execute("DELETE FROM usage_providers WHERE id = 'session'", [])
            .is_err());
        assert!(conn
            .execute("DELETE FROM agent_modules WHERE id = ?1", [&custom.id])
            .is_err());
    }

    #[test]
    fn binding_view_uses_public_camel_case_and_snake_case_status() {
        let db = Database::memory().unwrap();
        insert_provider(
            &db,
            "session",
            "subscription",
            "[\"session_log\"]",
            None,
            true,
            None,
            None,
        );
        let view = db
            .save_agent_provider_binding(&input("codex", "session", true))
            .unwrap();
        let value: Value = serde_json::to_value(view).unwrap();
        assert_eq!(value["agentModuleId"], "codex");
        assert_eq!(value["providerId"], "session");
        assert_eq!(value["credentialStatus"], "not_required");
        assert!(value.get("agent_module_id").is_none());
    }
}
