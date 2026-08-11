//! 数据库备份和恢复
//!
//! 提供 SQL 导出/导入和二进制快照备份功能。

use super::{lock_conn, Database};
use crate::error::AppError;
use chrono::{Local, Utc};
use rusqlite::backup::Backup;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

const LLM_USAGE_BAR_SQL_EXPORT_HEADER: &str = "-- LLM Usage Bar SQLite export";
const LEGACY_CC_SWITCH_SQL_EXPORT_HEADER: &str = "-- CC Switch SQLite 导出";
const REDACTED_BACKUP_MARKER: &str = "-- sensitive fields: redacted";
const REDACTED_BACKUP_SETTING: &str = "backup_redaction_version";
const REDACTED_BACKUP_VERSION: &str = "1";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ProtectedBindingState {
    binding_id: String,
    agent_module_id: String,
    provider_id: String,
    binding_enabled: bool,
    fingerprint: Option<Vec<u8>>,
    credential_slot: Option<String>,
    credential_version: i64,
    route_protocol: Option<String>,
    provider_enabled: bool,
    billing_kind: String,
    token_sources: String,
    route_app_type: Option<String>,
    route_config: Option<String>,
    quota_config: Option<String>,
    legacy_app_type: Option<String>,
    legacy_provider_id: Option<String>,
    legacy_settings_config: Option<String>,
    legacy_meta: Option<String>,
    agent_archived_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProtectedProviderRouteState {
    provider_id: String,
    provider_enabled: bool,
    billing_kind: String,
    token_sources: String,
    route_app_type: Option<String>,
    route_config: Option<String>,
    quota_config: Option<String>,
    legacy_app_type: Option<String>,
    legacy_provider_id: Option<String>,
    legacy_settings_config: Option<String>,
    legacy_meta: Option<String>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct CredentialJournalState {
    operation_id: String,
    binding_id: String,
    generation: i64,
    operation_kind: String,
    status: String,
    staging_slot: Option<String>,
    previous_slot: Option<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct ProtectedProviderCredentialState {
    key_id: String,
    provider_id: String,
    label: String,
    fingerprint: Option<Vec<u8>>,
    credential_slot: Option<String>,
    credential_version: i64,
    last_test_at: Option<i64>,
    last_test_status: Option<String>,
    last_test_error_code: Option<String>,
    sort_order: i64,
    created_at: i64,
    updated_at: i64,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct ProviderCredentialJournalState {
    operation_id: String,
    key_id: String,
    generation: i64,
    operation_kind: String,
    status: String,
    staging_slot: Option<String>,
    previous_slot: Option<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct SystemProviderCanonicalState {
    id: String,
    system_preset_key: String,
    name: String,
    billing_kind: String,
    product_group_id: String,
    token_sources: String,
    quota_source: Option<String>,
    quota_interval_seconds: Option<i64>,
    route_app_type: Option<String>,
    route_config: Option<String>,
    quota_config: Option<String>,
    needs_review: bool,
    legacy_app_type: Option<String>,
    legacy_provider_id: Option<String>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct SchemaObjectIdentity {
    object_type: String,
    name: String,
    table_name: String,
}

/// Tables whose data rows are skipped when exporting for WebDAV sync.
const SYNC_SKIP_TABLES: &[&str] = &[
    "proxy_request_logs",
    "usage_daily_rollups",
    "usage_light_predictions",
    "provider_api_keys",
    "provider_key_usage_snapshots",
    "agent_credential_operations",
    "provider_credential_operations",
];

/// Tables whose local data is preserved (restored from local snapshot) during WebDAV import.
/// Device-local usage history is restored after sync imports.
const SYNC_PRESERVE_TABLES: &[&str] = &[
    "proxy_request_logs",
    "usage_daily_rollups",
    "usage_light_predictions",
    "provider_api_keys",
    "provider_key_usage_snapshots",
];

fn normalized_secret_key(key: &str) -> String {
    key.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_secret_json_key(key: &str) -> bool {
    let key = normalized_secret_key(key);
    key.contains("apikey")
        || key.contains("apitoken")
        || key.contains("accesstoken")
        || key.contains("refreshtoken")
        || key.contains("idtoken")
        || key.contains("authtoken")
        || key == "token"
        || key.ends_with("token")
        || key.ends_with("bearertoken")
        || key.contains("clientsecret")
        || key.contains("secret")
        || key.contains("credential")
        || key.contains("accesskeyid")
        || key.contains("secretaccesskey")
        || key.contains("privatekey")
        || key.contains("password")
        || key.contains("passwd")
        || key.ends_with("pwd")
        || key.contains("passphrase")
        || key.ends_with("authorization")
        || key.contains("cookie")
        || matches!(
            key.as_str(),
            "key"
                | "sig"
                | "signature"
                | "pat"
                | "jwt"
                | "assertion"
                | "auth"
                | "oauth"
                | "code"
                | "headers"
                | "env"
                | "args"
                | "command"
                | "body"
        )
}

fn redact_url_secrets(raw: &str) -> Option<String> {
    let mut url = url::Url::parse(raw).ok()?;
    let had_userinfo = !url.username().is_empty() || url.password().is_some();
    if had_userinfo {
        url.set_username("").ok()?;
        url.set_password(None).ok()?;
    }

    // Custom endpoints can embed bearer or bot tokens in their paths. Portable
    // backups keep only the network origin; same-device restore compares this
    // redacted projection before rehydrating any local-only credential state.
    let removed_path = url.host().is_some() && url.path() != "/";
    if removed_path {
        url.set_path("/");
    }
    let removed_query = url.query().is_some();
    if removed_query {
        url.set_query(None);
    }
    let removed_fragment = url.fragment().is_some();
    if removed_fragment {
        url.set_fragment(None);
    }

    (had_userinfo || removed_path || removed_query || removed_fragment).then(|| url.into())
}

fn redact_toml_secrets(value: &mut toml::Value) {
    match value {
        toml::Value::Table(table) => {
            table.retain(|key, value| {
                if is_secret_json_key(key) {
                    false
                } else {
                    redact_toml_secrets(value);
                    true
                }
            });
        }
        toml::Value::Array(values) => {
            for value in values {
                redact_toml_secrets(value);
            }
        }
        toml::Value::String(raw) => {
            if let Some(redacted) = redact_url_secrets(raw) {
                *raw = redacted;
            }
        }
        _ => {}
    }
}

fn redact_json_secrets(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.retain(|key, value| {
                if is_secret_json_key(key) {
                    false
                } else {
                    redact_json_secrets(value);
                    true
                }
            });
        }
        serde_json::Value::Array(values) => {
            for value in values {
                redact_json_secrets(value);
            }
        }
        serde_json::Value::String(raw) => {
            if let Ok(mut nested) = serde_json::from_str::<serde_json::Value>(raw) {
                redact_json_secrets(&mut nested);
                if let Ok(redacted) = serde_json::to_string(&nested) {
                    *raw = redacted;
                }
            } else if let Ok(mut nested) = toml::from_str::<toml::Value>(raw) {
                redact_toml_secrets(&mut nested);
                if let Ok(redacted) = toml::to_string(&nested) {
                    *raw = redacted;
                }
            } else if let Some(redacted) = redact_url_secrets(raw) {
                *raw = redacted;
            }
        }
        _ => {}
    }
}

fn sanitized_json_backup_text(
    raw: Option<&str>,
    invalid_fallback: Option<&str>,
) -> Result<Option<String>, AppError> {
    match raw {
        Some(raw) => match serde_json::from_str::<serde_json::Value>(raw) {
            Ok(mut value) => {
                redact_json_secrets(&mut value);
                Ok(Some(serde_json::to_string(&value).map_err(|_| {
                    AppError::Database("failed to sanitize backup JSON".to_string())
                })?))
            }
            Err(_) => Ok(invalid_fallback.map(str::to_string)),
        },
        None => Ok(None),
    }
}

fn redacted_json_matches(
    local: Option<&str>,
    incoming: Option<&str>,
    invalid_fallback: Option<&str>,
) -> Result<bool, AppError> {
    let projected_local = sanitized_json_backup_text(local, invalid_fallback)?;
    let projected_incoming = sanitized_json_backup_text(incoming, invalid_fallback)?;
    match (projected_local.as_deref(), projected_incoming.as_deref()) {
        (None, None) => Ok(true),
        (Some(projected), Some(incoming)) => {
            let projected = serde_json::from_str::<serde_json::Value>(projected)
                .map_err(|_| Database::credential_conflict())?;
            let incoming = serde_json::from_str::<serde_json::Value>(incoming)
                .map_err(|_| Database::credential_conflict())?;
            Ok(projected == incoming)
        }
        _ => Ok(false),
    }
}

fn json_backup_value_is_redacted(raw: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return false;
    };
    let mut redacted = value.clone();
    redact_json_secrets(&mut redacted);
    redacted == value
}

/// A database backup entry for the UI
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupEntry {
    pub filename: String,
    pub size_bytes: u64,
    pub created_at: String, // ISO 8601
}

impl Database {
    fn credential_conflict() -> AppError {
        AppError::Message("credential_conflict".to_string())
    }

    fn protected_binding_state(
        conn: &Connection,
    ) -> Result<BTreeSet<ProtectedBindingState>, AppError> {
        if !Self::table_exists(conn, "agent_provider_bindings")? {
            return Ok(BTreeSet::new());
        }

        let mut statement = conn.prepare(
            "SELECT binding.id, binding.agent_module_id, binding.provider_id,
                    binding.enabled, binding.api_key_fingerprint,
                    binding.credential_slot, binding.credential_version,
                    binding.route_protocol,
                    provider.enabled, provider.billing_kind, provider.token_sources,
                    provider.route_app_type, provider.route_config, provider.quota_config,
                    provider.legacy_app_type, provider.legacy_provider_id,
                    (SELECT legacy.settings_config FROM providers AS legacy
                     WHERE legacy.id = provider.legacy_provider_id
                       AND legacy.app_type = provider.legacy_app_type),
                    (SELECT legacy.meta FROM providers AS legacy
                     WHERE legacy.id = provider.legacy_provider_id
                       AND legacy.app_type = provider.legacy_app_type),
                    agent.archived_at
             FROM agent_provider_bindings AS binding
             JOIN usage_providers AS provider ON provider.id = binding.provider_id
             JOIN agent_modules AS agent ON agent.id = binding.agent_module_id
             WHERE binding.api_key_fingerprint IS NOT NULL
                OR binding.credential_slot IS NOT NULL
                OR EXISTS (
                    SELECT 1 FROM provider_api_keys AS provider_key
                    WHERE provider_key.provider_id = provider.id
                      AND (provider_key.api_key_fingerprint IS NOT NULL
                           OR provider_key.credential_slot IS NOT NULL)
                )
             ORDER BY binding.id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(ProtectedBindingState {
                binding_id: row.get(0)?,
                agent_module_id: row.get(1)?,
                provider_id: row.get(2)?,
                binding_enabled: row.get(3)?,
                fingerprint: row.get(4)?,
                credential_slot: row.get(5)?,
                credential_version: row.get(6)?,
                route_protocol: row.get(7)?,
                provider_enabled: row.get(8)?,
                billing_kind: row.get(9)?,
                token_sources: row.get(10)?,
                route_app_type: row.get(11)?,
                route_config: row.get(12)?,
                quota_config: row.get(13)?,
                legacy_app_type: row.get(14)?,
                legacy_provider_id: row.get(15)?,
                legacy_settings_config: row.get(16)?,
                legacy_meta: row.get(17)?,
                agent_archived_at: row.get(18)?,
            })
        })?;

        rows.collect::<Result<BTreeSet<_>, _>>()
            .map_err(AppError::from)
    }

    fn protected_provider_route_state(
        conn: &Connection,
        provider_id: &str,
    ) -> Result<Option<ProtectedProviderRouteState>, AppError> {
        conn.query_row(
            "SELECT provider.id, provider.enabled, provider.billing_kind,
                    provider.token_sources, provider.route_app_type,
                    provider.route_config, provider.quota_config,
                    provider.legacy_app_type, provider.legacy_provider_id,
                    (SELECT legacy.settings_config FROM providers AS legacy
                     WHERE legacy.id = provider.legacy_provider_id
                       AND legacy.app_type = provider.legacy_app_type),
                    (SELECT legacy.meta FROM providers AS legacy
                     WHERE legacy.id = provider.legacy_provider_id
                       AND legacy.app_type = provider.legacy_app_type)
             FROM usage_providers AS provider
             WHERE provider.id = ?1",
            [provider_id],
            |row| {
                Ok(ProtectedProviderRouteState {
                    provider_id: row.get(0)?,
                    provider_enabled: row.get(1)?,
                    billing_kind: row.get(2)?,
                    token_sources: row.get(3)?,
                    route_app_type: row.get(4)?,
                    route_config: row.get(5)?,
                    quota_config: row.get(6)?,
                    legacy_app_type: row.get(7)?,
                    legacy_provider_id: row.get(8)?,
                    legacy_settings_config: row.get(9)?,
                    legacy_meta: row.get(10)?,
                })
            },
        )
        .optional()
        .map_err(AppError::from)
    }

    fn credential_journal_state(
        conn: &Connection,
    ) -> Result<BTreeSet<CredentialJournalState>, AppError> {
        if !Self::table_exists(conn, "agent_credential_operations")? {
            return Ok(BTreeSet::new());
        }

        let mut statement = conn.prepare(
            "SELECT operation_id, binding_id, generation, operation_kind, status,
                    staging_slot, previous_slot, created_at, updated_at
             FROM agent_credential_operations
             ORDER BY operation_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(CredentialJournalState {
                operation_id: row.get(0)?,
                binding_id: row.get(1)?,
                generation: row.get(2)?,
                operation_kind: row.get(3)?,
                status: row.get(4)?,
                staging_slot: row.get(5)?,
                previous_slot: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        })?;

        rows.collect::<Result<BTreeSet<_>, _>>()
            .map_err(AppError::from)
    }

    fn protected_provider_credential_state(
        conn: &Connection,
    ) -> Result<BTreeSet<ProtectedProviderCredentialState>, AppError> {
        if !Self::table_exists(conn, "provider_api_keys")? {
            return Ok(BTreeSet::new());
        }
        let mut statement = conn.prepare(
            "SELECT id, provider_id, label, api_key_fingerprint, credential_slot,
                    credential_version, last_test_at, last_test_status,
                    last_test_error_code, sort_order, created_at, updated_at
             FROM provider_api_keys ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(ProtectedProviderCredentialState {
                key_id: row.get(0)?,
                provider_id: row.get(1)?,
                label: row.get(2)?,
                fingerprint: row.get(3)?,
                credential_slot: row.get(4)?,
                credential_version: row.get(5)?,
                last_test_at: row.get(6)?,
                last_test_status: row.get(7)?,
                last_test_error_code: row.get(8)?,
                sort_order: row.get(9)?,
                created_at: row.get(10)?,
                updated_at: row.get(11)?,
            })
        })?;
        rows.collect::<Result<BTreeSet<_>, _>>()
            .map_err(AppError::from)
    }

    fn provider_credential_journal_state(
        conn: &Connection,
    ) -> Result<BTreeSet<ProviderCredentialJournalState>, AppError> {
        if !Self::table_exists(conn, "provider_credential_operations")? {
            return Ok(BTreeSet::new());
        }
        let mut statement = conn.prepare(
            "SELECT operation_id, key_id, generation, operation_kind, status,
                    staging_slot, previous_slot, created_at, updated_at
             FROM provider_credential_operations ORDER BY operation_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(ProviderCredentialJournalState {
                operation_id: row.get(0)?,
                key_id: row.get(1)?,
                generation: row.get(2)?,
                operation_kind: row.get(3)?,
                status: row.get(4)?,
                staging_slot: row.get(5)?,
                previous_slot: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        })?;
        rows.collect::<Result<BTreeSet<_>, _>>()
            .map_err(AppError::from)
    }

    fn system_provider_canonical_state(
        conn: &Connection,
    ) -> Result<BTreeSet<SystemProviderCanonicalState>, AppError> {
        if !Self::has_column(conn, "usage_providers", "system_preset_key")? {
            return Ok(BTreeSet::new());
        }
        let mut statement = conn.prepare(
            "SELECT id, system_preset_key, name, billing_kind, product_group_id,
                    token_sources, quota_source, quota_interval_seconds,
                    route_app_type, route_config, quota_config, needs_review,
                    legacy_app_type, legacy_provider_id
             FROM usage_providers
             WHERE system_preset_key IS NOT NULL
             ORDER BY system_preset_key",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(SystemProviderCanonicalState {
                id: row.get(0)?,
                system_preset_key: row.get(1)?,
                name: row.get(2)?,
                billing_kind: row.get(3)?,
                product_group_id: row.get(4)?,
                token_sources: row.get(5)?,
                quota_source: row.get(6)?,
                quota_interval_seconds: row.get(7)?,
                route_app_type: row.get(8)?,
                route_config: row.get(9)?,
                quota_config: row.get(10)?,
                needs_review: row.get(11)?,
                legacy_app_type: row.get(12)?,
                legacy_provider_id: row.get(13)?,
            })
        })?;
        rows.collect::<Result<BTreeSet<_>, _>>()
            .map_err(AppError::from)
    }

    fn preserve_system_provider_seed_marker(
        local: &Connection,
        incoming: &Connection,
    ) -> Result<(), AppError> {
        let marker: Option<String> = local
            .query_row(
                "SELECT value FROM settings
                 WHERE key = 'system_provider_default_bindings_v1_seeded'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(marker) = marker else {
            return Err(Self::credential_conflict());
        };
        incoming.execute(
            "INSERT INTO settings (key, value)
             VALUES ('system_provider_default_bindings_v1_seeded', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [marker],
        )?;
        Ok(())
    }

    fn ever_bound_agent_ids(conn: &Connection) -> Result<BTreeSet<String>, AppError> {
        if !Self::table_exists(conn, "agent_modules")? {
            return Ok(BTreeSet::new());
        }
        let mut statement =
            conn.prepare("SELECT id FROM agent_modules WHERE ever_bound = 1 ORDER BY id")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        rows.collect::<Result<BTreeSet<_>, _>>()
            .map_err(AppError::from)
    }

    fn schema_object_identities(
        conn: &Connection,
    ) -> Result<BTreeSet<SchemaObjectIdentity>, AppError> {
        let mut statement = conn.prepare(
            "SELECT type, name, tbl_name
             FROM sqlite_schema
             WHERE type IN ('table', 'index', 'trigger', 'view')
               AND sql IS NOT NULL
             ORDER BY type, name",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(SchemaObjectIdentity {
                object_type: row.get(0)?,
                name: row.get(1)?,
                table_name: row.get(2)?,
            })
        })?;
        rows.collect::<Result<BTreeSet<_>, _>>()
            .map_err(AppError::from)
    }

    fn trigger_definitions(conn: &Connection) -> Result<BTreeMap<String, String>, AppError> {
        let mut statement = conn.prepare(
            "SELECT name, sql FROM sqlite_schema
             WHERE type = 'trigger'
             ORDER BY name",
        )?;
        let rows = statement.query_map([], |row| {
            let name: String = row.get(0)?;
            let sql: String = row.get(1)?;
            Ok((name, sql.split_whitespace().collect::<Vec<_>>().join(" ")))
        })?;
        rows.collect::<Result<BTreeMap<_, _>, _>>()
            .map_err(AppError::from)
    }

    /// Imported DDL is untrusted executable SQLite state. Compare the migrated
    /// object namespace with a database created by this exact build and require
    /// every trigger body to be canonical before any Backup can reach the live
    /// connection. This rejects extra dormant triggers as well as a malicious
    /// body hidden behind a legitimate trigger name.
    fn validate_import_schema_allowlist(incoming: &Connection) -> Result<(), AppError> {
        let canonical = Database::memory().map_err(|_| Self::credential_conflict())?;
        let canonical_conn = lock_conn!(canonical.conn);
        let incoming_objects =
            Self::schema_object_identities(incoming).map_err(|_| Self::credential_conflict())?;
        let canonical_objects = Self::schema_object_identities(&canonical_conn)
            .map_err(|_| Self::credential_conflict())?;
        if incoming_objects != canonical_objects {
            return Err(Self::credential_conflict());
        }
        let incoming_triggers =
            Self::trigger_definitions(incoming).map_err(|_| Self::credential_conflict())?;
        let canonical_triggers =
            Self::trigger_definitions(&canonical_conn).map_err(|_| Self::credential_conflict())?;
        if incoming_triggers != canonical_triggers {
            return Err(Self::credential_conflict());
        }
        Ok(())
    }

    fn ensure_protected_credentials_preserved(
        current: &Connection,
        incoming: &Connection,
    ) -> Result<(), AppError> {
        let current_bindings = Self::protected_binding_state(current)?;
        let incoming_bindings = Self::protected_binding_state(incoming)?;
        // Incoming snapshots may add protected bindings whose local store will
        // fail closed, but they must not orphan any slot already owned here.
        if !current_bindings.is_subset(&incoming_bindings) {
            return Err(Self::credential_conflict());
        }

        let current_journal = Self::credential_journal_state(current)?;
        let incoming_journal = Self::credential_journal_state(incoming)?;
        // Journal rows are device-local deletion intent. Neither dropping local
        // work nor importing another device's cleanup work is safe.
        if current_journal != incoming_journal {
            return Err(Self::credential_conflict());
        }

        let current_provider_credentials = Self::protected_provider_credential_state(current)?;
        let incoming_provider_credentials = Self::protected_provider_credential_state(incoming)?;
        if !current_provider_credentials.is_subset(&incoming_provider_credentials) {
            return Err(Self::credential_conflict());
        }

        let current_provider_journal = Self::provider_credential_journal_state(current)?;
        let incoming_provider_journal = Self::provider_credential_journal_state(incoming)?;
        if current_provider_journal != incoming_provider_journal {
            return Err(Self::credential_conflict());
        }

        if Self::system_provider_canonical_state(current)?
            != Self::system_provider_canonical_state(incoming)?
        {
            return Err(Self::credential_conflict());
        }

        // `ever_bound` is a one-way tombstone: once a local Agent has owned a
        // binding, importing an older snapshot must not make it hard-deletable
        // again after that binding is removed.
        let current_ever_bound = Self::ever_bound_agent_ids(current)?;
        let incoming_ever_bound = Self::ever_bound_agent_ids(incoming)?;
        if !current_ever_bound.is_subset(&incoming_ever_bound) {
            return Err(Self::credential_conflict());
        }

        Ok(())
    }

    fn preserve_local_credentials_for_sync(
        local: &Connection,
        incoming: &Connection,
    ) -> Result<(), AppError> {
        // A journal is device-local deletion intent. Current sync exports omit
        // it; reject stale or foreign snapshots instead of executing them here.
        if !Self::credential_journal_state(local)?.is_empty()
            || !Self::credential_journal_state(incoming)?.is_empty()
            || !Self::provider_credential_journal_state(local)?.is_empty()
            || !Self::provider_credential_journal_state(incoming)?.is_empty()
        {
            return Err(Self::credential_conflict());
        }

        for state in Self::protected_binding_state(local)? {
            let updated = incoming.execute(
                "UPDATE agent_provider_bindings
                 SET api_key_fingerprint = ?2, credential_slot = ?3,
                     credential_version = ?4
                 WHERE id = ?1 AND agent_module_id = ?5 AND provider_id = ?6",
                rusqlite::params![
                    state.binding_id,
                    state.fingerprint,
                    state.credential_slot,
                    state.credential_version,
                    state.agent_module_id,
                    state.provider_id,
                ],
            );
            match updated {
                Ok(1) => {}
                Ok(_) | Err(_) => return Err(Self::credential_conflict()),
            }
        }
        for agent_id in Self::ever_bound_agent_ids(local)? {
            match incoming.execute(
                "UPDATE agent_modules SET ever_bound = 1 WHERE id = ?1",
                [&agent_id],
            ) {
                Ok(1) => {}
                Ok(_) | Err(_) => return Err(Self::credential_conflict()),
            }
        }
        for state in Self::protected_provider_credential_state(local)? {
            match incoming.execute(
                "UPDATE provider_api_keys
                 SET api_key_fingerprint = ?2, credential_slot = ?3,
                     credential_version = ?4, last_test_at = ?5,
                     last_test_status = ?6, last_test_error_code = ?7,
                     created_at = ?8, updated_at = ?9
                 WHERE id = ?1 AND provider_id = ?10",
                rusqlite::params![
                    state.key_id,
                    state.fingerprint,
                    state.credential_slot,
                    state.credential_version,
                    state.last_test_at,
                    state.last_test_status,
                    state.last_test_error_code,
                    state.created_at,
                    state.updated_at,
                    state.provider_id,
                ],
            ) {
                Ok(1) => {}
                Ok(_) | Err(_) => return Err(Self::credential_conflict()),
            }
        }
        Self::preserve_system_provider_seed_marker(local, incoming)?;
        Ok(())
    }

    fn redacted_backup_version(conn: &Connection) -> Result<bool, AppError> {
        if !Self::table_exists(conn, "settings")? {
            return Ok(false);
        }
        let version = conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [REDACTED_BACKUP_SETTING],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        match version.as_deref() {
            None => Ok(false),
            Some(REDACTED_BACKUP_VERSION) => Ok(true),
            Some(_) => Err(Self::credential_conflict()),
        }
    }

    fn remove_redacted_backup_marker(conn: &Connection) -> Result<(), AppError> {
        conn.execute(
            "DELETE FROM settings WHERE key = ?1",
            [REDACTED_BACKUP_SETTING],
        )?;
        Ok(())
    }

    fn validate_redacted_json_column(
        conn: &Connection,
        table: &str,
        column: &str,
    ) -> Result<(), AppError> {
        if !Self::table_exists(conn, table)? || !Self::has_column(conn, table, column)? {
            return Err(Self::credential_conflict());
        }
        let mut statement = conn.prepare(&format!(
            "SELECT \"{column}\" FROM \"{table}\" WHERE \"{column}\" IS NOT NULL"
        ))?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            if !json_backup_value_is_redacted(&row?) {
                return Err(Self::credential_conflict());
            }
        }
        Ok(())
    }

    fn validate_redacted_settings(conn: &Connection) -> Result<(), AppError> {
        let mut statement = conn.prepare("SELECT key, value FROM settings")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        for row in rows {
            let (key, value) = row?;
            if key != REDACTED_BACKUP_SETTING && is_secret_json_key(&key) {
                return Err(Self::credential_conflict());
            }
            let normalized = normalized_secret_key(&key);
            if matches!(key.as_str(), "global_proxy_url" | "universal_providers")
                || (key.starts_with("common_config_")
                    && !key.ends_with("_cleared")
                    && normalized != "commonconfiglegacymigratedv1")
            {
                return Err(Self::credential_conflict());
            }
            let Some(value) = value else {
                continue;
            };
            if serde_json::from_str::<serde_json::Value>(&value).is_ok()
                && !json_backup_value_is_redacted(&value)
            {
                return Err(Self::credential_conflict());
            }
            if redact_url_secrets(&value).is_some() {
                return Err(Self::credential_conflict());
            }
        }
        Ok(())
    }

    fn validate_redacted_url_column(
        conn: &Connection,
        table: &str,
        column: &str,
    ) -> Result<(), AppError> {
        let mut statement = conn.prepare(&format!(
            "SELECT \"{column}\" FROM \"{table}\" WHERE \"{column}\" IS NOT NULL"
        ))?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            if redact_url_secrets(&row?).is_some() {
                return Err(Self::credential_conflict());
            }
        }
        Ok(())
    }

    fn validate_redacted_backup_shape(conn: &Connection) -> Result<(), AppError> {
        if !Self::credential_journal_state(conn)?.is_empty()
            || !Self::provider_credential_journal_state(conn)?.is_empty()
        {
            return Err(Self::credential_conflict());
        }

        let binding_secrets: i64 = conn.query_row(
            "SELECT COUNT(*) FROM agent_provider_bindings
             WHERE api_key_fingerprint IS NOT NULL OR credential_slot IS NOT NULL",
            [],
            |row| row.get(0),
        )?;
        let provider_key_secrets = if Self::table_exists(conn, "provider_api_keys")? {
            conn.query_row(
                "SELECT COUNT(*) FROM provider_api_keys
                 WHERE api_key_fingerprint IS NOT NULL OR credential_slot IS NOT NULL",
                [],
                |row| row.get::<_, i64>(0),
            )?
        } else {
            0
        };
        // v26 intentionally retains this table for rollback/data recovery. Its
        // old slot pointers are retired, but a redacted backup must still prove
        // that it did not carry them across devices.
        let retired_provider_secrets = if Self::table_exists(conn, "provider_api_credentials")? {
            conn.query_row(
                "SELECT COUNT(*) FROM provider_api_credentials
                     WHERE api_key_fingerprint IS NOT NULL OR credential_slot IS NOT NULL",
                [],
                |row| row.get::<_, i64>(0),
            )?
        } else {
            0
        };
        let quota_secrets: i64 = conn.query_row(
            "SELECT COUNT(*) FROM usage_providers WHERE quota_config IS NOT NULL",
            [],
            |row| row.get(0),
        )?;
        if binding_secrets != 0
            || provider_key_secrets != 0
            || retired_provider_secrets != 0
            || quota_secrets != 0
        {
            return Err(Self::credential_conflict());
        }

        let nonempty_meta: i64 = conn.query_row(
            "SELECT COUNT(*) FROM providers WHERE meta != '{}'",
            [],
            |row| row.get(0),
        )?;
        if nonempty_meta != 0 {
            return Err(Self::credential_conflict());
        }

        for (table, column) in [
            ("providers", "settings_config"),
            ("providers", "meta"),
            ("usage_providers", "route_config"),
            ("quota_snapshots", "raw_payload"),
        ] {
            Self::validate_redacted_json_column(conn, table, column)?;
        }
        Self::validate_redacted_settings(conn)?;
        Self::validate_redacted_url_column(conn, "provider_endpoints", "url")?;
        Self::validate_redacted_url_column(conn, "providers", "website_url")?;
        for (table, column, empty_allowed) in [
            ("quota_fetch_state", "last_error", false),
            ("proxy_request_logs", "error_message", false),
        ] {
            if !Self::table_exists(conn, table)? || !Self::has_column(conn, table, column)? {
                continue;
            }
            let condition = if empty_allowed {
                format!("\"{column}\" != ''")
            } else {
                format!("\"{column}\" IS NOT NULL")
            };
            let unsafe_rows: i64 = conn.query_row(
                &format!("SELECT COUNT(*) FROM \"{table}\" WHERE {condition}"),
                [],
                |row| row.get(0),
            )?;
            if unsafe_rows != 0 {
                return Err(Self::credential_conflict());
            }
        }
        Ok(())
    }

    fn redacted_binding_context_matches(
        current: &ProtectedBindingState,
        incoming: &ProtectedBindingState,
    ) -> Result<bool, AppError> {
        if !redacted_json_matches(
            current.route_config.as_deref(),
            incoming.route_config.as_deref(),
            None,
        )? || !redacted_json_matches(
            current.legacy_settings_config.as_deref(),
            incoming.legacy_settings_config.as_deref(),
            Some("{}"),
        )? {
            return Ok(false);
        }

        let mut projected = current.clone();
        projected.route_config = incoming.route_config.clone();
        projected.quota_config = incoming.quota_config.clone();
        projected.legacy_settings_config = incoming.legacy_settings_config.clone();
        projected.legacy_meta = incoming.legacy_meta.clone();
        Ok(projected == *incoming)
    }

    fn redacted_provider_context_matches(
        current: &ProtectedProviderRouteState,
        incoming: &ProtectedProviderRouteState,
    ) -> Result<bool, AppError> {
        if !redacted_json_matches(
            current.route_config.as_deref(),
            incoming.route_config.as_deref(),
            None,
        )? || !redacted_json_matches(
            current.legacy_settings_config.as_deref(),
            incoming.legacy_settings_config.as_deref(),
            Some("{}"),
        )? {
            return Ok(false);
        }

        let mut projected = current.clone();
        projected.route_config = incoming.route_config.clone();
        projected.quota_config = incoming.quota_config.clone();
        projected.legacy_settings_config = incoming.legacy_settings_config.clone();
        projected.legacy_meta = incoming.legacy_meta.clone();
        Ok(projected == *incoming)
    }

    fn restore_local_provider_auth_context(
        local: &Connection,
        incoming: &Connection,
        provider_id: &str,
    ) -> Result<(), AppError> {
        let state = Self::protected_provider_route_state(local, provider_id)?
            .ok_or_else(Self::credential_conflict)?;
        match incoming.execute(
            "UPDATE usage_providers
             SET route_config = ?2, quota_config = ?3
             WHERE id = ?1",
            rusqlite::params![state.provider_id, state.route_config, state.quota_config],
        ) {
            Ok(1) => {}
            Ok(_) | Err(_) => return Err(Self::credential_conflict()),
        }
        if let (Some(legacy_app_type), Some(legacy_provider_id)) =
            (state.legacy_app_type, state.legacy_provider_id)
        {
            match incoming.execute(
                "UPDATE providers
                 SET settings_config = ?3, meta = ?4
                 WHERE app_type = ?1 AND id = ?2",
                rusqlite::params![
                    legacy_app_type,
                    legacy_provider_id,
                    state.legacy_settings_config,
                    state.legacy_meta,
                ],
            ) {
                Ok(1) => {}
                Ok(_) | Err(_) => return Err(Self::credential_conflict()),
            }
        }
        Ok(())
    }

    fn rehydrate_local_credentials_for_redacted_backup(
        local: &Connection,
        incoming: &Connection,
    ) -> Result<(), AppError> {
        if !Self::credential_journal_state(local)?.is_empty()
            || !Self::credential_journal_state(incoming)?.is_empty()
            || !Self::provider_credential_journal_state(local)?.is_empty()
            || !Self::provider_credential_journal_state(incoming)?.is_empty()
        {
            return Err(Self::credential_conflict());
        }

        let current_bindings = Self::protected_binding_state(local)?;
        // Unlike the retired one-row-per-Provider table, v26 has no seeded
        // empty placeholders. An empty key row is user-created local
        // bookkeeping and must be preserved along with configured keys.
        let current_provider_credentials = Self::protected_provider_credential_state(local)?
            .into_iter()
            .collect::<Vec<_>>();

        for state in &current_bindings {
            let has_binding_credential =
                state.fingerprint.is_some() || state.credential_slot.is_some();
            let updated = if has_binding_credential {
                incoming.execute(
                    "UPDATE agent_provider_bindings
                     SET api_key_fingerprint = ?2, credential_slot = ?3,
                         credential_version = ?4
                     WHERE id = ?1 AND agent_module_id = ?5 AND provider_id = ?6",
                    rusqlite::params![
                        state.binding_id,
                        state.fingerprint,
                        state.credential_slot,
                        state.credential_version,
                        state.agent_module_id,
                        state.provider_id,
                    ],
                )
            } else {
                // System bindings are seeded with per-database IDs. A Provider-level
                // Keychain credential follows the logical Agent + Provider route, so
                // preserve the local ID only after that exact route has matched.
                incoming.execute(
                    "UPDATE agent_provider_bindings
                     SET id = ?1, api_key_fingerprint = NULL,
                         credential_slot = NULL, credential_version = ?2
                     WHERE agent_module_id = ?3 AND provider_id = ?4",
                    rusqlite::params![
                        state.binding_id,
                        state.credential_version,
                        state.agent_module_id,
                        state.provider_id,
                    ],
                )
            };
            match updated {
                Ok(1) => {}
                Ok(_) | Err(_) => return Err(Self::credential_conflict()),
            }
        }
        for state in &current_provider_credentials {
            match incoming.execute(
                "INSERT INTO provider_api_keys (
                     id, provider_id, label, api_key_fingerprint,
                     credential_slot, credential_version, last_test_at,
                     last_test_status, last_test_error_code, sort_order,
                     created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(id) DO UPDATE SET
                     label = excluded.label,
                     api_key_fingerprint = excluded.api_key_fingerprint,
                     credential_slot = excluded.credential_slot,
                     credential_version = excluded.credential_version,
                     last_test_at = excluded.last_test_at,
                     last_test_status = excluded.last_test_status,
                     last_test_error_code = excluded.last_test_error_code,
                     sort_order = excluded.sort_order,
                     created_at = excluded.created_at,
                     updated_at = excluded.updated_at
                 WHERE provider_api_keys.provider_id = excluded.provider_id",
                rusqlite::params![
                    state.key_id,
                    state.provider_id,
                    state.label,
                    state.fingerprint,
                    state.credential_slot,
                    state.credential_version,
                    state.last_test_at,
                    state.last_test_status,
                    state.last_test_error_code,
                    state.sort_order,
                    state.created_at,
                    state.updated_at,
                ],
            ) {
                Ok(1) => {}
                Ok(_) | Err(_) => return Err(Self::credential_conflict()),
            }
        }

        let incoming_bindings = Self::protected_binding_state(incoming)?;
        let mut protected_provider_ids = BTreeSet::new();
        for current in &current_bindings {
            let Some(candidate) = incoming_bindings
                .iter()
                .find(|candidate| candidate.binding_id == current.binding_id)
            else {
                return Err(Self::credential_conflict());
            };
            if !Self::redacted_binding_context_matches(current, candidate)? {
                return Err(Self::credential_conflict());
            }
            protected_provider_ids.insert(current.provider_id.clone());
        }

        for credential in &current_provider_credentials {
            let current = Self::protected_provider_route_state(local, &credential.provider_id)?
                .ok_or_else(Self::credential_conflict)?;
            let incoming = Self::protected_provider_route_state(incoming, &credential.provider_id)?
                .ok_or_else(Self::credential_conflict)?;
            if !Self::redacted_provider_context_matches(&current, &incoming)? {
                return Err(Self::credential_conflict());
            }
            protected_provider_ids.insert(credential.provider_id.clone());
        }

        for provider_id in protected_provider_ids {
            Self::restore_local_provider_auth_context(local, incoming, &provider_id)?;
        }
        for agent_id in Self::ever_bound_agent_ids(local)? {
            match incoming.execute(
                "UPDATE agent_modules SET ever_bound = 1 WHERE id = ?1",
                [&agent_id],
            ) {
                Ok(1) => {}
                Ok(_) | Err(_) => return Err(Self::credential_conflict()),
            }
        }
        Self::remove_redacted_backup_marker(incoming)?;
        Ok(())
    }

    fn authoritative_backup_dir(&self) -> Result<Option<PathBuf>, AppError> {
        let Some(database_path) = self.database_path() else {
            return Ok(None);
        };
        let parent = database_path
            .parent()
            .ok_or_else(|| AppError::Config("无效的数据库路径".to_string()))?;
        Ok(Some(parent.join("backups")))
    }

    fn sanitize_json_backup_column(
        conn: &Connection,
        table: &str,
        column: &str,
        invalid_fallback: Option<&str>,
    ) -> Result<(), AppError> {
        if !Self::table_exists(conn, table)? || !Self::has_column(conn, table, column)? {
            return Ok(());
        }

        let sql = format!("SELECT rowid, \"{column}\" FROM \"{table}\"");
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        let rows = rows.collect::<Result<Vec<_>, _>>()?;
        drop(statement);

        for (rowid, raw) in rows {
            let sanitized = sanitized_json_backup_text(raw.as_deref(), invalid_fallback)?;
            conn.execute(
                &format!("UPDATE \"{table}\" SET \"{column}\" = ?1 WHERE rowid = ?2"),
                rusqlite::params![sanitized, rowid],
            )?;
        }
        Ok(())
    }

    fn sanitize_url_backup_column(
        conn: &Connection,
        table: &str,
        column: &str,
    ) -> Result<(), AppError> {
        if !Self::table_exists(conn, table)? || !Self::has_column(conn, table, column)? {
            return Ok(());
        }
        let mut statement = conn.prepare(&format!(
            "SELECT rowid, \"{column}\" FROM \"{table}\" WHERE \"{column}\" IS NOT NULL"
        ))?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        let rows = rows.collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        for (rowid, raw) in rows {
            if let Some(redacted) = redact_url_secrets(&raw) {
                conn.execute(
                    &format!("UPDATE \"{table}\" SET \"{column}\" = ?1 WHERE rowid = ?2"),
                    rusqlite::params![redacted, rowid],
                )?;
            }
        }
        Ok(())
    }

    fn sanitize_backup_settings(conn: &Connection) -> Result<(), AppError> {
        if !Self::table_exists(conn, "settings")? {
            return Ok(());
        }
        let mut statement = conn.prepare("SELECT key, value FROM settings")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        let rows = rows.collect::<Result<Vec<_>, _>>()?;
        drop(statement);

        for (key, value) in rows {
            let normalized = normalized_secret_key(&key);
            if is_secret_json_key(&key) {
                conn.execute("DELETE FROM settings WHERE key = ?1", [&key])?;
                continue;
            }
            let is_common_config = key.starts_with("common_config_")
                && !key.ends_with("_cleared")
                && normalized != "commonconfiglegacymigratedv1";
            let Some(value) = value else {
                continue;
            };
            if is_common_config {
                conn.execute("DELETE FROM settings WHERE key = ?1", [&key])?;
                continue;
            }
            if matches!(key.as_str(), "global_proxy_url" | "universal_providers") {
                conn.execute("DELETE FROM settings WHERE key = ?1", [&key])?;
                continue;
            }
            if let Some(redacted) = sanitized_json_backup_text(Some(&value), None)? {
                conn.execute(
                    "UPDATE settings SET value = ?2 WHERE key = ?1",
                    rusqlite::params![key, redacted],
                )?;
            } else if let Some(redacted) = redact_url_secrets(&value) {
                conn.execute(
                    "UPDATE settings SET value = ?2 WHERE key = ?1",
                    rusqlite::params![key, redacted],
                )?;
            }
        }
        Ok(())
    }

    fn sanitize_backup_snapshot(conn: &Connection) -> Result<(), AppError> {
        conn.execute_batch("PRAGMA secure_delete=ON;")?;
        conn.execute_batch("SAVEPOINT sanitize_backup_snapshot;")?;
        let result = (|| {
            Self::sanitize_json_backup_column(conn, "providers", "settings_config", Some("{}"))?;
            if Self::table_exists(conn, "providers")? {
                conn.execute("UPDATE providers SET meta = '{}'", [])?;
            }
            Self::sanitize_json_backup_column(conn, "usage_providers", "route_config", None)?;
            Self::sanitize_url_backup_column(conn, "provider_endpoints", "url")?;
            Self::sanitize_url_backup_column(conn, "providers", "website_url")?;
            Self::sanitize_backup_settings(conn)?;

            if Self::table_exists(conn, "usage_providers")? {
                conn.execute("UPDATE usage_providers SET quota_config = NULL", [])?;
            }
            if Self::table_exists(conn, "quota_snapshots")? {
                let append_only_trigger = conn
                    .query_row(
                        "SELECT sql FROM sqlite_master
                         WHERE type = 'trigger'
                           AND name = 'quota_snapshots_append_only_update'",
                        [],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?;
                conn.execute(
                    "DROP TRIGGER IF EXISTS quota_snapshots_append_only_update",
                    [],
                )?;
                conn.execute("UPDATE quota_snapshots SET raw_payload = '{}'", [])?;
                if let Some(trigger_sql) = append_only_trigger {
                    conn.execute_batch(&format!("{trigger_sql};"))?;
                }
            }
            conn.execute(
                "UPDATE settings SET value = 'false' WHERE key LIKE 'proxy_takeover_%'",
                [],
            )?;
            for (table, column) in [
                ("quota_fetch_state", "last_error"),
                ("proxy_request_logs", "error_message"),
            ] {
                if Self::table_exists(conn, table)? && Self::has_column(conn, table, column)? {
                    conn.execute(&format!("UPDATE \"{table}\" SET \"{column}\" = NULL"), [])?;
                }
            }
            if Self::table_exists(conn, "agent_provider_bindings")? {
                conn.execute(
                    "UPDATE agent_provider_bindings
                     SET api_key_fingerprint = NULL, credential_slot = NULL",
                    [],
                )?;
            }
            if Self::table_exists(conn, "provider_api_credentials")? {
                conn.execute(
                    "UPDATE provider_api_credentials
                     SET api_key_fingerprint = NULL, credential_slot = NULL",
                    [],
                )?;
            }
            if Self::table_exists(conn, "provider_api_keys")? {
                conn.execute(
                    "UPDATE provider_api_keys
                     SET api_key_fingerprint = NULL, credential_slot = NULL",
                    [],
                )?;
            }
            for table in [
                "agent_credential_operations",
                "provider_credential_operations",
            ] {
                if Self::table_exists(conn, table)? {
                    conn.execute(&format!("DELETE FROM \"{table}\""), [])?;
                }
            }
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![REDACTED_BACKUP_SETTING, REDACTED_BACKUP_VERSION],
            )?;
            Ok(())
        })();

        match result {
            Ok(()) => {
                conn.execute_batch("RELEASE sanitize_backup_snapshot;")?;
                Ok(())
            }
            Err(error) => {
                let _ = conn.execute_batch(
                    "ROLLBACK TO sanitize_backup_snapshot; RELEASE sanitize_backup_snapshot;",
                );
                Err(error)
            }
        }
    }

    fn sanitized_snapshot_for_backup(&self) -> Result<Connection, AppError> {
        let snapshot = self.snapshot_to_memory()?;
        Self::sanitize_backup_snapshot(&snapshot)?;
        snapshot.execute_batch("VACUUM;")?;
        Ok(snapshot)
    }

    /// 导出为 SQLite 兼容的 SQL 文本（内存字符串，完整导出）
    pub fn export_sql_string(&self) -> Result<String, AppError> {
        let snapshot = self.sanitized_snapshot_for_backup()?;
        Self::dump_sql(&snapshot, &[])
    }

    /// Export SQL for sync (WebDAV), skipping local-only tables' data
    pub fn export_sql_string_for_sync(&self) -> Result<String, AppError> {
        let snapshot = self.sanitized_snapshot_for_backup()?;
        Self::dump_sql(&snapshot, SYNC_SKIP_TABLES)
    }

    /// 导出为 SQLite 兼容的 SQL 文本
    pub fn export_sql(&self, target_path: &Path) -> Result<(), AppError> {
        let dump = self.export_sql_string()?;

        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
        }

        crate::config::atomic_write(target_path, dump.as_bytes())
    }

    /// 从 SQL 文件导入，返回生成的备份 ID（若无备份则为空字符串）
    pub fn import_sql(&self, source_path: &Path) -> Result<String, AppError> {
        if !source_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "SQL 文件不存在: {}",
                source_path.display()
            )));
        }

        let sql_raw = fs::read_to_string(source_path).map_err(|e| AppError::io(source_path, e))?;
        let sql_content = sql_raw.trim_start_matches('\u{feff}');
        self.import_sql_string(sql_content)
    }

    /// 从 SQL 字符串导入，返回生成的备份 ID（若无备份则为空字符串）
    pub fn import_sql_string(&self, sql_raw: &str) -> Result<String, AppError> {
        self.import_sql_string_inner(sql_raw, &[])
    }

    /// Import SQL generated for sync, then restore local-only tables from the
    /// current device snapshot before replacing the main database.
    pub(crate) fn import_sql_string_for_sync(&self, sql_raw: &str) -> Result<String, AppError> {
        self.import_sql_string_inner(sql_raw, SYNC_PRESERVE_TABLES)
    }

    fn import_sql_string_inner(
        &self,
        sql_raw: &str,
        preserve_tables: &[&str],
    ) -> Result<String, AppError> {
        let sql_content = sql_raw.trim_start_matches('\u{feff}');
        Self::validate_llm_usage_bar_sql_export(sql_content)?;

        // 导入前备份现有数据库
        let backup_path = self.backup_database_file()?;

        let local_snapshot = if preserve_tables.is_empty() {
            None
        } else {
            Some(self.snapshot_to_memory()?)
        };

        // 在临时数据库执行导入，确保失败不会污染主库
        let temp_file = NamedTempFile::new().map_err(|e| AppError::IoContext {
            context: "创建临时数据库文件失败".to_string(),
            source: e,
        })?;
        let temp_path = temp_file.path().to_path_buf();
        let temp_conn =
            Connection::open(&temp_path).map_err(|e| AppError::Database(e.to_string()))?;

        temp_conn
            .execute_batch(sql_content)
            .map_err(|e| AppError::Database(format!("执行 SQL 导入失败: {e}")))?;
        let redacted_backup = Self::redacted_backup_version(&temp_conn)?;
        if redacted_backup {
            Self::validate_redacted_backup_shape(&temp_conn)?;
        }

        // 补齐缺失表/索引并进行基础校验
        Self::create_tables_on_conn(&temp_conn)?;
        if Self::get_user_version(&temp_conn)? == crate::database::SCHEMA_VERSION {
            crate::usage::system_provider_migration::reconcile_system_provider_catalog(&temp_conn)?;
        }
        Self::apply_schema_migrations_on_conn(&temp_conn)?;
        crate::usage::system_provider_migration::reconcile_system_provider_catalog(&temp_conn)?;
        crate::usage::system_provider_migration::validate_schema_v17_complete(&temp_conn)?;
        crate::usage::budget_migration::normalize_daily_budgets(&temp_conn)?;
        Self::validate_basic_state(&temp_conn)?;
        Self::validate_import_schema_allowlist(&temp_conn)?;
        if let Some(local_snapshot) = local_snapshot.as_ref() {
            Self::restore_tables(local_snapshot, &temp_conn, preserve_tables)?;
            if !redacted_backup {
                Self::preserve_local_credentials_for_sync(local_snapshot, &temp_conn)?;
            }
        }

        // 使用 Backup 将临时库原子写回主库
        {
            let mut main_conn = lock_conn!(self.conn);
            if redacted_backup {
                Self::rehydrate_local_credentials_for_redacted_backup(&main_conn, &temp_conn)?;
            }
            Self::preserve_system_provider_seed_marker(&main_conn, &temp_conn)?;
            Self::ensure_protected_credentials_preserved(&main_conn, &temp_conn)?;
            let backup = Backup::new(&temp_conn, &mut main_conn)
                .map_err(|e| AppError::Database(e.to_string()))?;
            backup
                .step(-1)
                .map_err(|e| AppError::Database(e.to_string()))?;
        }

        let backup_id = backup_path
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
            .unwrap_or_default();

        Ok(backup_id)
    }

    /// 创建内存快照以避免长时间持有数据库锁
    pub(crate) fn snapshot_to_memory(&self) -> Result<Connection, AppError> {
        let conn = lock_conn!(self.conn);
        let mut snapshot =
            Connection::open_in_memory().map_err(|e| AppError::Database(e.to_string()))?;

        {
            let backup =
                Backup::new(&conn, &mut snapshot).map_err(|e| AppError::Database(e.to_string()))?;
            backup
                .step(-1)
                .map_err(|e| AppError::Database(e.to_string()))?;
        }

        Ok(snapshot)
    }

    fn validate_llm_usage_bar_sql_export(sql: &str) -> Result<(), AppError> {
        let first_line = sql.trim_start().lines().next().unwrap_or_default();
        if matches!(
            first_line,
            LLM_USAGE_BAR_SQL_EXPORT_HEADER | LEGACY_CC_SWITCH_SQL_EXPORT_HEADER
        ) {
            return Ok(());
        }

        Err(AppError::localized(
            "backup.sql.invalid_format",
            "仅支持导入由 LLM Usage Bar 或旧版 CC Switch 导出的 SQL 备份文件。",
            "Only SQL backups exported by LLM Usage Bar or legacy CC Switch are supported.",
        ))
    }

    fn restore_tables(
        source_conn: &Connection,
        target_conn: &Connection,
        tables: &[&str],
    ) -> Result<(), AppError> {
        for table in tables {
            if !Self::table_exists(source_conn, table)? || !Self::table_exists(target_conn, table)?
            {
                continue;
            }

            let columns = Self::get_table_columns(source_conn, table)?;
            if columns.is_empty() {
                continue;
            }

            target_conn
                .execute(&format!("DELETE FROM \"{table}\""), [])
                .map_err(|e| AppError::Database(format!("清空表 {table} 失败: {e}")))?;

            let placeholders = (1..=columns.len())
                .map(|idx| format!("?{idx}"))
                .collect::<Vec<_>>()
                .join(", ");
            let cols = columns
                .iter()
                .map(|column| format!("\"{column}\""))
                .collect::<Vec<_>>()
                .join(", ");
            let insert_sql = format!("INSERT INTO \"{table}\" ({cols}) VALUES ({placeholders})");

            let mut stmt = source_conn
                .prepare(&format!("SELECT * FROM \"{table}\""))
                .map_err(|e| AppError::Database(format!("读取表 {table} 失败: {e}")))?;
            let mut rows = stmt
                .query([])
                .map_err(|e| AppError::Database(format!("查询表 {table} 数据失败: {e}")))?;

            while let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
                let mut values = Vec::with_capacity(columns.len());
                for idx in 0..columns.len() {
                    values.push(
                        row.get::<_, rusqlite::types::Value>(idx)
                            .map_err(|e| AppError::Database(e.to_string()))?,
                    );
                }

                target_conn
                    .execute(&insert_sql, rusqlite::params_from_iter(values.iter()))
                    .map_err(|e| AppError::Database(format!("恢复表 {table} 数据失败: {e}")))?;
            }
        }

        Ok(())
    }

    /// Periodic backup: create a new backup if the latest one is older than the configured interval
    pub(crate) fn periodic_backup_if_needed(&self) -> Result<(), AppError> {
        let interval_hours = crate::settings::effective_backup_interval_hours();
        if interval_hours > 0 {
            if let Some(backup_dir) = self.authoritative_backup_dir()? {
                let latest = fs::read_dir(&backup_dir).ok().and_then(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .filter(|e| e.path().extension().map(|ext| ext == "db").unwrap_or(false))
                        .filter_map(|e| e.metadata().ok().and_then(|m| m.modified().ok()))
                        .max()
                });

                let interval_secs = u64::from(interval_hours) * 3600;
                let needs_backup = match latest {
                    None => true,
                    Some(last_modified) => {
                        last_modified.elapsed().unwrap_or_default()
                            > std::time::Duration::from_secs(interval_secs)
                    }
                };

                if needs_backup {
                    log::info!(
                        "Periodic backup: latest backup is older than {interval_hours} hours, creating new backup"
                    );
                    self.backup_database_file()?;
                }
            }
        }

        // Periodic maintenance is always enabled, regardless of auto-backup settings.
        let mut reclaimed_rows = 0u64;
        match self.rollup_and_prune(30) {
            Ok(deleted) => {
                reclaimed_rows += deleted;
            }
            Err(e) => {
                log::warn!("Periodic rollup_and_prune failed: {e}");
            }
        }
        if reclaimed_rows > 0 {
            let conn = lock_conn!(self.conn);
            if let Err(e) = conn.execute_batch("PRAGMA incremental_vacuum;") {
                log::warn!("Periodic incremental vacuum failed: {e}");
            }
        }

        Ok(())
    }

    /// 生成一致性快照备份，返回备份文件路径（不存在主库时返回 None）
    pub(crate) fn backup_database_file(&self) -> Result<Option<PathBuf>, AppError> {
        let Some(db_path) = self.database_path() else {
            return Ok(None);
        };
        if !db_path.exists() {
            return Ok(None);
        }

        let backup_dir = self
            .authoritative_backup_dir()?
            .ok_or_else(|| AppError::Config("内存数据库没有可写入的备份目录".to_string()))?;

        fs::create_dir_all(&backup_dir).map_err(|e| AppError::io(&backup_dir, e))?;

        let base_id = format!("db_backup_{}", Local::now().format("%Y%m%d_%H%M%S"));
        let mut backup_id = base_id.clone();
        let mut backup_path = backup_dir.join(format!("{backup_id}.db"));
        let mut counter = 1;
        while backup_path.exists() {
            backup_id = format!("{base_id}_{counter}");
            backup_path = backup_dir.join(format!("{backup_id}.db"));
            counter += 1;
        }

        {
            let snapshot = self.sanitized_snapshot_for_backup()?;
            let mut dest_conn =
                Connection::open(&backup_path).map_err(|e| AppError::Database(e.to_string()))?;
            let backup = Backup::new(&snapshot, &mut dest_conn)
                .map_err(|e| AppError::Database(e.to_string()))?;
            backup
                .step(-1)
                .map_err(|e| AppError::Database(e.to_string()))?;
        }

        Self::cleanup_db_backups(&backup_dir)?;
        Ok(Some(backup_path))
    }

    /// 清理旧的数据库备份，保留最新的 N 个
    fn cleanup_db_backups(dir: &Path) -> Result<(), AppError> {
        let retain = crate::settings::effective_backup_retain_count();
        let entries = match fs::read_dir(dir) {
            Ok(iter) => iter
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    entry
                        .path()
                        .extension()
                        .map(|ext| ext == "db")
                        .unwrap_or(false)
                })
                .collect::<Vec<_>>(),
            Err(_) => return Ok(()),
        };

        if entries.len() <= retain {
            return Ok(());
        }

        let remove_count = entries.len().saturating_sub(retain);
        let mut sorted = entries;
        sorted.sort_by_key(|entry| entry.metadata().and_then(|m| m.modified()).ok());

        for entry in sorted.into_iter().take(remove_count) {
            if let Err(err) = fs::remove_file(entry.path()) {
                log::warn!("删除旧数据库备份失败 {}: {}", entry.path().display(), err);
            }
        }
        Ok(())
    }

    /// 基础状态校验
    fn validate_basic_state(conn: &Connection) -> Result<(), AppError> {
        let provider_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM providers", [], |row| row.get(0))
            .map_err(|e| AppError::Database(e.to_string()))?;

        if provider_count == 0 {
            return Err(AppError::Config(
                "导入的 SQL 未包含有效的供应商数据".to_string(),
            ));
        }
        Ok(())
    }

    /// 导出数据库为 SQL 文本
    fn dump_sql(conn: &Connection, skip_tables: &[&str]) -> Result<String, AppError> {
        let mut output = String::new();
        let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let user_version: i64 = conn
            .query_row("PRAGMA user_version;", [], |row| row.get(0))
            .unwrap_or(0);

        output.push_str(&format!(
            "{LLM_USAGE_BAR_SQL_EXPORT_HEADER}\n{REDACTED_BACKUP_MARKER}\n-- 生成时间: {timestamp}\n-- user_version: {user_version}\n"
        ));
        output.push_str("PRAGMA foreign_keys=OFF;\n");
        output.push_str(&format!("PRAGMA user_version={user_version};\n"));
        output.push_str("BEGIN TRANSACTION;\n");

        // 导出 schema
        let mut stmt = conn
            .prepare(
                "SELECT type, name, tbl_name, sql
                 FROM sqlite_master
                 WHERE sql NOT NULL AND type IN ('table','index','trigger','view')
                 ORDER BY type='table' DESC, name",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut tables = Vec::new();
        let mut rows = stmt
            .query([])
            .map_err(|e| AppError::Database(e.to_string()))?;
        while let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
            let obj_type: String = row.get(0).map_err(|e| AppError::Database(e.to_string()))?;
            let name: String = row.get(1).map_err(|e| AppError::Database(e.to_string()))?;
            let sql: String = row.get(3).map_err(|e| AppError::Database(e.to_string()))?;

            // 跳过 SQLite 内部对象（如 sqlite_sequence）
            if name.starts_with("sqlite_") {
                continue;
            }

            output.push_str(&sql);
            output.push_str(";\n");

            if obj_type == "table" && !name.starts_with("sqlite_") {
                tables.push(name);
            }
        }

        // 导出数据
        for table in tables {
            if skip_tables.iter().any(|t| *t == table) {
                continue;
            }
            let columns = Self::get_table_columns(conn, &table)?;
            if columns.is_empty() {
                continue;
            }

            let mut stmt = conn
                .prepare(&format!("SELECT * FROM \"{table}\""))
                .map_err(|e| AppError::Database(e.to_string()))?;
            let mut rows = stmt
                .query([])
                .map_err(|e| AppError::Database(e.to_string()))?;

            while let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
                let mut values = Vec::with_capacity(columns.len());
                for idx in 0..columns.len() {
                    let value = row
                        .get_ref(idx)
                        .map_err(|e| AppError::Database(e.to_string()))?;
                    values.push(Self::format_sql_value(value)?);
                }

                let cols = columns
                    .iter()
                    .map(|c| format!("\"{c}\""))
                    .collect::<Vec<_>>()
                    .join(", ");
                output.push_str(&format!(
                    "INSERT INTO \"{table}\" ({cols}) VALUES ({});\n",
                    values.join(", ")
                ));
            }
        }

        output.push_str("COMMIT;\nPRAGMA foreign_keys=ON;\n");
        Ok(output)
    }

    /// 获取表的列名列表
    fn get_table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, AppError> {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info(\"{table}\")"))
            .map_err(|e| AppError::Database(e.to_string()))?;
        let iter = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut columns = Vec::new();
        for col in iter {
            columns.push(col.map_err(|e| AppError::Database(e.to_string()))?);
        }
        Ok(columns)
    }

    /// 格式化 SQL 值
    fn format_sql_value(value: ValueRef<'_>) -> Result<String, AppError> {
        match value {
            ValueRef::Null => Ok("NULL".to_string()),
            ValueRef::Integer(i) => Ok(i.to_string()),
            ValueRef::Real(f) => Ok(f.to_string()),
            ValueRef::Text(t) => {
                let text = std::str::from_utf8(t)
                    .map_err(|e| AppError::Database(format!("文本字段不是有效的 UTF-8: {e}")))?;
                let escaped = text.replace('\'', "''");
                Ok(format!("'{escaped}'"))
            }
            ValueRef::Blob(bytes) => {
                let mut s = String::from("X'");
                for b in bytes {
                    use std::fmt::Write;
                    let _ = write!(&mut s, "{b:02X}");
                }
                s.push('\'');
                Ok(s)
            }
        }
    }

    /// List all database backup files, sorted by creation time (newest first)
    pub fn list_backups(&self) -> Result<Vec<BackupEntry>, AppError> {
        let Some(backup_dir) = self.authoritative_backup_dir()? else {
            return Ok(vec![]);
        };
        if !backup_dir.exists() {
            return Ok(vec![]);
        }

        let mut entries: Vec<BackupEntry> = fs::read_dir(&backup_dir)
            .map_err(|e| AppError::io(&backup_dir, e))?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|ext| ext == "db").unwrap_or(false))
            .filter_map(|e| {
                let metadata = e.metadata().ok()?;
                let filename = e.file_name().to_string_lossy().to_string();
                let size_bytes = metadata.len();
                let created_at = metadata
                    .modified()
                    .ok()
                    .map(|t| {
                        let dt: chrono::DateTime<Utc> = t.into();
                        dt.to_rfc3339()
                    })
                    .unwrap_or_default();
                Some(BackupEntry {
                    filename,
                    size_bytes,
                    created_at,
                })
            })
            .collect();

        // Sort by created_at descending (newest first)
        entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(entries)
    }

    /// Restore database from a backup file. Returns the safety backup ID.
    pub fn restore_from_backup(&self, filename: &str) -> Result<String, AppError> {
        // Security: validate filename to prevent path traversal
        if filename.contains("..")
            || filename.contains('/')
            || filename.contains('\\')
            || !filename.ends_with(".db")
        {
            return Err(AppError::InvalidInput(
                "Invalid backup filename".to_string(),
            ));
        }

        let backup_dir = self
            .authoritative_backup_dir()?
            .ok_or_else(|| AppError::Config("内存数据库没有可恢复的备份目录".to_string()))?;
        let backup_path = backup_dir.join(filename);

        if !backup_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "Backup file not found: {filename}"
            )));
        }

        // Step 1: Create safety backup of current database
        let safety_backup = self.backup_database_file()?;
        let safety_id = safety_backup
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
            .unwrap_or_default();

        // Step 2: Stage and migrate the backup before it can replace the main database.
        // Besides making old backups safe to inspect, this keeps a failed migration
        // from partially overwriting the live connection.
        let source_conn =
            Connection::open(&backup_path).map_err(|e| AppError::Database(e.to_string()))?;
        let staged_file = NamedTempFile::new().map_err(|e| AppError::IoContext {
            context: "创建临时数据库文件失败".to_string(),
            source: e,
        })?;
        let mut staged_conn =
            Connection::open(staged_file.path()).map_err(|e| AppError::Database(e.to_string()))?;
        {
            let backup = Backup::new(&source_conn, &mut staged_conn)
                .map_err(|e| AppError::Database(e.to_string()))?;
            backup
                .step(-1)
                .map_err(|e| AppError::Database(e.to_string()))?;
        }
        let redacted_backup = Self::redacted_backup_version(&staged_conn)?;
        if redacted_backup {
            Self::validate_redacted_backup_shape(&staged_conn)?;
        }
        Self::create_tables_on_conn(&staged_conn)?;
        if Self::get_user_version(&staged_conn)? == crate::database::SCHEMA_VERSION {
            crate::usage::system_provider_migration::reconcile_system_provider_catalog(
                &staged_conn,
            )?;
        }
        Self::apply_schema_migrations_on_conn(&staged_conn)?;
        crate::usage::system_provider_migration::reconcile_system_provider_catalog(&staged_conn)?;
        crate::usage::system_provider_migration::validate_schema_v17_complete(&staged_conn)?;
        crate::usage::budget_migration::normalize_daily_budgets(&staged_conn)?;
        Self::validate_import_schema_allowlist(&staged_conn)?;

        // Step 3: Preserve device-local credential lifecycle state, then restore.
        {
            let mut main_conn = lock_conn!(self.conn);
            if redacted_backup {
                Self::rehydrate_local_credentials_for_redacted_backup(&main_conn, &staged_conn)?;
            }
            Self::preserve_system_provider_seed_marker(&main_conn, &staged_conn)?;
            Self::ensure_protected_credentials_preserved(&main_conn, &staged_conn)?;
            let backup = Backup::new(&staged_conn, &mut main_conn)
                .map_err(|e| AppError::Database(e.to_string()))?;
            backup
                .step(-1)
                .map_err(|e| AppError::Database(e.to_string()))?;
        }

        // Step 4: Re-run idempotent initialization on the live connection.
        self.create_tables()?;
        self.apply_schema_migrations()?;
        self.reconcile_system_providers()?;
        self.ensure_model_pricing_seeded()?;

        log::info!("Database restored from backup: {filename}, safety backup: {safety_id}");
        Ok(safety_id)
    }

    /// Rename a backup file. Returns the new filename.
    pub fn rename_backup(&self, old_filename: &str, new_name: &str) -> Result<String, AppError> {
        // Validate old filename (path traversal + .db suffix)
        if old_filename.contains("..")
            || old_filename.contains('/')
            || old_filename.contains('\\')
            || !old_filename.ends_with(".db")
        {
            return Err(AppError::InvalidInput(
                "Invalid backup filename".to_string(),
            ));
        }

        // Clean new name
        let trimmed = new_name.trim();
        if trimmed.is_empty() {
            return Err(AppError::InvalidInput(
                "New name cannot be empty".to_string(),
            ));
        }

        // Length limit (without .db suffix)
        let name_part = trimmed.strip_suffix(".db").unwrap_or(trimmed);
        if name_part.len() > 100 {
            return Err(AppError::InvalidInput(
                "Name too long (max 100 characters)".to_string(),
            ));
        }

        // Prevent path traversal in new name
        if name_part.contains("..")
            || name_part.contains('/')
            || name_part.contains('\\')
            || name_part.contains('\0')
        {
            return Err(AppError::InvalidInput(
                "Invalid characters in new name".to_string(),
            ));
        }

        let new_filename = format!("{name_part}.db");

        let backup_dir = self
            .authoritative_backup_dir()?
            .ok_or_else(|| AppError::Config("内存数据库没有可重命名的备份目录".to_string()))?;
        let old_path = backup_dir.join(old_filename);
        let new_path = backup_dir.join(&new_filename);

        if !old_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "Backup file not found: {old_filename}"
            )));
        }

        if new_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "A backup named '{new_filename}' already exists"
            )));
        }

        fs::rename(&old_path, &new_path).map_err(|e| AppError::io(&old_path, e))?;
        log::info!("Renamed backup: {old_filename} -> {new_filename}");
        Ok(new_filename)
    }

    /// Delete a backup file permanently.
    pub fn delete_backup(&self, filename: &str) -> Result<(), AppError> {
        // Validate filename (path traversal + .db suffix)
        if filename.contains("..")
            || filename.contains('/')
            || filename.contains('\\')
            || !filename.ends_with(".db")
        {
            return Err(AppError::InvalidInput(
                "Invalid backup filename".to_string(),
            ));
        }

        let backup_dir = self
            .authoritative_backup_dir()?
            .ok_or_else(|| AppError::Config("内存数据库没有可删除的备份目录".to_string()))?;
        let backup_path = backup_dir.join(filename);
        if !backup_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "Backup file not found: {filename}"
            )));
        }

        fs::remove_file(&backup_path).map_err(|e| AppError::io(&backup_path, e))?;
        log::info!("Deleted backup: {filename}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        redact_url_secrets, Database, LEGACY_CC_SWITCH_SQL_EXPORT_HEADER,
        LLM_USAGE_BAR_SQL_EXPORT_HEADER,
    };
    use crate::error::AppError;
    use crate::product_identity::DATABASE_FILE;
    use crate::settings::{update_settings, AppSettings};
    use crate::usage::domain::{BillingKind, TokenSource, UsageProviderInput};
    use rusqlite::params;
    use serial_test::serial;
    use std::ffi::OsString;

    const BACKUP_SECRET_SENTINELS: &[&str] = &[
        "legacy-api-secret",
        "legacy-oauth-secret",
        "route-api-secret",
        "quota-api-secret",
        "quota-aws-secret",
        "raw-upstream-secret",
        "binding-slot-secret",
        "provider-slot-secret",
        "toml-bearer-secret",
        "url-password-secret",
        "url-code-secret",
        "url-path-secret",
        "legacy-path-secret",
        "route-path-secret",
        "mcp-pat-secret",
        "mcp-arg-secret",
        "live-refresh-secret",
        "gateway-setting-secret",
        "proxy-password-secret",
        "proxy-query-secret",
        "common-config-secret",
        "universal-provider-secret",
        "opaque-setting-secret",
        "endpoint-password-secret",
        "endpoint-query-secret",
        "endpoint-path-secret",
        "quota-error-secret",
        "health-error-secret",
        "stream-error-secret",
        "request-error-secret",
    ];

    struct TestHomeRestore(Option<OsString>);

    impl Drop for TestHomeRestore {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => std::env::set_var("LLM_USAGE_BAR_TEST_HOME", value),
                None => std::env::remove_var("LLM_USAGE_BAR_TEST_HOME"),
            }
        }
    }

    fn insert_protected_credential_state(
        db: &Database,
        binding_id: &str,
        slot: &str,
        version: i64,
        include_journal: bool,
    ) -> Result<(), AppError> {
        let conn = crate::database::lock_conn!(db.conn);
        conn.execute(
            "INSERT OR IGNORE INTO providers (id, app_type, name, settings_config, meta)
             VALUES ('backup-provider', 'claude', 'Backup Provider', '{}', '{}')",
            [],
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO usage_providers (
                 id, name, billing_kind, product_group_id, token_sources,
                 route_app_type, route_config, enabled, needs_review,
                 created_at, updated_at
             ) VALUES (
                 'backup-usage-provider', 'Backup Usage Provider', 'metered', 'backup',
                 '[\"proxy\"]', 'claude', '{}', 1, 0, 10, 10
             )",
            [],
        )?;
        conn.execute(
            "INSERT INTO agent_provider_bindings (
                 id, agent_module_id, provider_id, enabled,
                 api_key_fingerprint, credential_slot, credential_version,
                 created_at, updated_at
             ) VALUES (?1, 'claude-code', 'backup-usage-provider', 1, ?2, ?3, ?4, 10, 10)",
            params![binding_id, vec![0x5a_u8; 32], slot, version],
        )?;
        if include_journal {
            conn.execute(
                "INSERT INTO agent_credential_operations (
                     operation_id, binding_id, generation, operation_kind,
                     status, staging_slot, previous_slot, created_at, updated_at
                 ) VALUES (
                     'backup-cleanup-operation', ?1, ?2, 'replace',
                     'cleanup', ?3, 'orphaned-previous-slot', 11, 12
                 )",
                params![binding_id, version, slot],
            )?;
        }
        Ok(())
    }

    fn protected_tuple(
        db: &Database,
        binding_id: &str,
    ) -> Result<(Vec<u8>, String, i64), AppError> {
        let conn = crate::database::lock_conn!(db.conn);
        conn.query_row(
            "SELECT api_key_fingerprint, credential_slot, credential_version
             FROM agent_provider_bindings WHERE id = ?1",
            [binding_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(AppError::from)
    }

    type OptionalProtectedTuple = (Option<Vec<u8>>, Option<String>, i64);

    fn optional_protected_tuple(
        db: &Database,
        binding_id: &str,
    ) -> Result<OptionalProtectedTuple, AppError> {
        let conn = crate::database::lock_conn!(db.conn);
        conn.query_row(
            "SELECT api_key_fingerprint, credential_slot, credential_version
             FROM agent_provider_bindings WHERE id = ?1",
            [binding_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(AppError::from)
    }

    fn set_provider_credential_state(
        db: &Database,
        key_id: &str,
        fingerprint_byte: u8,
        slot: &str,
        version: i64,
    ) -> Result<(), AppError> {
        let conn = crate::database::lock_conn!(db.conn);
        conn.execute(
            "INSERT INTO provider_api_keys (
                 id, provider_id, label, api_key_fingerprint, credential_slot,
                 credential_version, last_test_at, last_test_status,
                 last_test_error_code, sort_order, created_at, updated_at
             )
             SELECT ?1, provider.id, provider.name, ?2, ?3, ?4, 91,
                    'success', NULL, 0, 90, 92
             FROM usage_providers AS provider WHERE provider.id = ?1
             ON CONFLICT(id) DO UPDATE SET
                 api_key_fingerprint = excluded.api_key_fingerprint,
                 credential_slot = excluded.credential_slot,
                 credential_version = excluded.credential_version,
                 last_test_at = excluded.last_test_at,
                 last_test_status = excluded.last_test_status,
                 last_test_error_code = excluded.last_test_error_code,
                 updated_at = excluded.updated_at",
            params![key_id, vec![fingerprint_byte; 32], slot, version],
        )?;
        Ok(())
    }

    type ProviderCredentialTuple = (Vec<u8>, String, i64, Option<i64>, Option<String>);

    fn provider_credential_tuple(
        db: &Database,
        key_id: &str,
    ) -> Result<ProviderCredentialTuple, AppError> {
        let conn = crate::database::lock_conn!(db.conn);
        conn.query_row(
            "SELECT api_key_fingerprint, credential_slot, credential_version,
                    last_test_at, last_test_status
             FROM provider_api_keys WHERE id = ?1",
            [key_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .map_err(AppError::from)
    }

    fn insert_import_sentinel(db: &Database, id: &str) -> Result<(), AppError> {
        db.conn.lock().unwrap().execute(
            "INSERT INTO providers (id, app_type, name, settings_config, meta)
             VALUES (?1, 'claude', 'Import Sentinel', '{}', '{}')",
            [id],
        )?;
        Ok(())
    }

    fn install_backup_secret_sentinels(db: &Database) -> Result<(), AppError> {
        insert_protected_credential_state(
            db,
            "backup-secret-binding",
            "binding-slot-secret",
            7,
            false,
        )?;
        set_provider_credential_state(
            db,
            "system-openrouter-api",
            0x71,
            "provider-slot-secret",
            8,
        )?;

        let conn = crate::database::lock_conn!(db.conn);
        conn.execute(
            "UPDATE providers
             SET settings_config = '{\"apiKey\":\"legacy-api-secret\",\"baseUrl\":\"https://safe.example/v1/legacy-path-secret\",\"config\":\"experimental_bearer_token = \\\"toml-bearer-secret\\\"\"}',
                 meta = '{\"oauth\":{\"access_token\":\"legacy-oauth-secret\"},\"label\":\"safe metadata\"}',
                 website_url = 'https://url-user:url-password-secret@safe.example/docs/url-path-secret?code=url-code-secret'
             WHERE id = 'backup-provider' AND app_type = 'claude'",
            [],
        )?;
        conn.execute(
            "UPDATE usage_providers
             SET route_config = '{\"base_url\":\"https://safe.example/v1/route-path-secret\",\"apiKey\":\"route-api-secret\"}',
                 quota_config = '{\"api_key\":\"quota-api-secret\",\"secret_access_key\":\"quota-aws-secret\"}'
             WHERE id = 'backup-usage-provider'",
            [],
        )?;
        conn.execute(
            "INSERT INTO quota_snapshots (
                 snapshot_id, provider_id, fetched_at, raw_payload, created_at
             ) VALUES (
                 'backup-raw-payload', 'system-chatgpt-subscription', 10,
                 '{\"access_token\":\"raw-upstream-secret\",\"remaining\":51}', 10
             )",
            [],
        )?;
        for (key, value) in [
            ("claude_desktop_gateway_token", "gateway-setting-secret"),
            (
                "global_proxy_url",
                "http://proxy-user:proxy-password-secret@127.0.0.1:8080?token=proxy-query-secret",
            ),
            (
                "common_config_codex",
                "experimental_bearer_token = \"common-config-secret\"",
            ),
            (
                "universal_providers",
                "[{\"id\":\"sentinel\",\"apiKey\":\"universal-provider-secret\"}]",
            ),
            ("oauth_credential", "opaque-setting-secret"),
        ] {
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
                params![key, value],
            )?;
        }
        conn.execute(
            "INSERT INTO provider_endpoints (provider_id, app_type, url, added_at)
             VALUES ('backup-provider', 'claude',
                     'https://endpoint-user:endpoint-password-secret@safe.example/v1/endpoint-path-secret?signature=endpoint-query-secret',
                     10)",
            [],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO quota_fetch_state (
                 provider_id, last_attempt_at, last_error, stale
             ) VALUES ('system-chatgpt-subscription', 10, 'quota-error-secret', 1)",
            [],
        )?;
        conn.execute(
            "INSERT INTO proxy_request_logs (
                 request_id, provider_id, app_type, model, latency_ms,
                 status_code, error_message, created_at
             ) VALUES (
                 'backup-error-request', 'backup-provider', 'claude', 'model', 1,
                 500, 'request-error-secret', 10
             )",
            [],
        )?;
        Ok(())
    }

    #[test]
    fn url_redaction_removes_network_paths_that_may_embed_credentials() {
        assert_eq!(
            redact_url_secrets("https://safe.example/bot-token-secret/v1").as_deref(),
            Some("https://safe.example/")
        );
        assert_eq!(redact_url_secrets("https://safe.example/"), None);
    }

    #[test]
    fn sql_exports_redact_secret_bearing_database_fields() -> Result<(), AppError> {
        let db = Database::memory()?;
        install_backup_secret_sentinels(&db)?;

        for exported in [db.export_sql_string()?, db.export_sql_string_for_sync()?] {
            for secret in BACKUP_SECRET_SENTINELS {
                assert!(
                    !exported.contains(secret),
                    "SQL export leaked sentinel {secret}"
                );
            }
            assert!(!exported.contains(&"5A".repeat(32)));
            assert!(!exported.contains(&"71".repeat(32)));
            assert!(exported.contains("https://safe.example/"));
        }
        Ok(())
    }

    #[test]
    fn binary_backup_redacts_secret_bearing_database_fields() -> Result<(), AppError> {
        let temp = tempfile::tempdir().expect("create binary backup test root");
        let db_path = temp.path().join(DATABASE_FILE);
        let db = Database::init_at(&db_path)?;
        install_backup_secret_sentinels(&db)?;

        let backup_path = db
            .backup_database_file()?
            .expect("file database produces a backup");
        let backup = rusqlite::Connection::open(&backup_path)?;

        let (settings_config, meta): (String, String) = backup.query_row(
            "SELECT settings_config, meta FROM providers
             WHERE id = 'backup-provider' AND app_type = 'claude'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert!(!settings_config.contains("legacy-api-secret"));
        assert!(settings_config.contains("https://safe.example/"));
        assert!(!meta.contains("legacy-oauth-secret"));
        assert_eq!(meta, "{}");

        let (route_config, quota_config): (String, Option<String>) = backup.query_row(
            "SELECT route_config, quota_config FROM usage_providers
             WHERE id = 'backup-usage-provider'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert!(!route_config.contains("route-api-secret"));
        assert!(route_config.contains("https://safe.example/"));
        assert_eq!(quota_config, None);
        assert_eq!(
            backup.query_row(
                "SELECT raw_payload FROM quota_snapshots
                 WHERE snapshot_id = 'backup-raw-payload'",
                [],
                |row| row.get::<_, String>(0),
            )?,
            "{}"
        );
        assert_eq!(
            backup.query_row(
                "SELECT api_key_fingerprint, credential_slot
                 FROM agent_provider_bindings WHERE id = 'backup-secret-binding'",
                [],
                |row| Ok((
                    row.get::<_, Option<Vec<u8>>>(0)?,
                    row.get::<_, Option<String>>(1)?
                )),
            )?,
            (None, None)
        );
        assert_eq!(
            backup.query_row(
                "SELECT api_key_fingerprint, credential_slot
                 FROM provider_api_keys WHERE id = 'system-openrouter-api'",
                [],
                |row| Ok((
                    row.get::<_, Option<Vec<u8>>>(0)?,
                    row.get::<_, Option<String>>(1)?
                )),
            )?,
            (None, None)
        );
        drop(backup);
        let bytes = std::fs::read(&backup_path).expect("read sanitized SQLite backup bytes");
        for secret in BACKUP_SECRET_SENTINELS {
            assert!(
                !bytes
                    .windows(secret.len())
                    .any(|window| window == secret.as_bytes()),
                "binary backup retained sentinel bytes for {secret}"
            );
        }
        Ok(())
    }

    #[test]
    fn binary_restore_rehydrates_same_device_credentials_and_auth_context() -> Result<(), AppError>
    {
        let temp = tempfile::tempdir().expect("create same-device restore root");
        let db_path = temp.path().join(DATABASE_FILE);
        let db = Database::init_at(&db_path)?;
        insert_protected_credential_state(&db, "protected-binding", "local-slot", 3, false)?;
        set_provider_credential_state(
            &db,
            "system-openrouter-api",
            0x72,
            "provider/system-openrouter-api/local",
            4,
        )?;
        {
            let conn = crate::database::lock_conn!(db.conn);
            conn.execute(
                "UPDATE providers
                 SET settings_config = '{\"providerType\":\"custom\",\"apiKey\":\"local-legacy-key\"}',
                     meta = '{\"access_token\":\"local-legacy-token\",\"label\":\"local\"}'
                 WHERE id = 'backup-provider' AND app_type = 'claude'",
                [],
            )?;
            conn.execute(
                "UPDATE usage_providers
                 SET route_config = '{\"base_url\":\"https://safe.example/v1\",\"apiKey\":\"local-route-key\"}',
                     quota_config = '{\"api_key\":\"local-quota-key\"}',
                     legacy_app_type = 'claude', legacy_provider_id = 'backup-provider'
                 WHERE id = 'backup-usage-provider'",
                [],
            )?;
        }
        let backup_path = db
            .backup_database_file()?
            .expect("file database produces a backup");
        let filename = backup_path
            .file_name()
            .expect("backup filename")
            .to_string_lossy()
            .into_owned();

        db.restore_from_backup(&filename)?;

        assert_eq!(
            protected_tuple(&db, "protected-binding")?,
            (vec![0x5a_u8; 32], "local-slot".to_string(), 3)
        );
        assert_eq!(
            provider_credential_tuple(&db, "system-openrouter-api")?,
            (
                vec![0x72; 32],
                "provider/system-openrouter-api/local".into(),
                4,
                Some(91),
                Some("success".into()),
            )
        );
        let conn = crate::database::lock_conn!(db.conn);
        let (route_config, quota_config): (String, String) = conn.query_row(
            "SELECT route_config, quota_config FROM usage_providers
             WHERE id = 'backup-usage-provider'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert!(route_config.contains("local-route-key"));
        assert!(quota_config.contains("local-quota-key"));
        let (settings_config, meta): (String, String) = conn.query_row(
            "SELECT settings_config, meta FROM providers
             WHERE id = 'backup-provider' AND app_type = 'claude'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert!(settings_config.contains("local-legacy-key"));
        assert!(meta.contains("local-legacy-token"));
        Ok(())
    }

    #[test]
    fn redacted_sql_import_rejects_a_binding_route_protocol_change() -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 1, false)?;
        local.conn.lock().unwrap().execute(
            "UPDATE agent_provider_bindings SET route_protocol = 'claude'
             WHERE id = 'protected-binding'",
            [],
        )?;

        let incoming = Database::memory()?;
        insert_protected_credential_state(&incoming, "protected-binding", "remote-slot", 2, false)?;
        incoming.conn.lock().unwrap().execute(
            "UPDATE agent_provider_bindings SET route_protocol = 'codex'
             WHERE id = 'protected-binding'",
            [],
        )?;

        let error = local
            .import_sql_string(&incoming.export_sql_string()?)
            .expect_err("redacted import must not retarget the local slot protocol");
        assert_eq!(error.to_string(), "credential_conflict");
        Ok(())
    }

    #[test]
    fn redacted_sql_import_protects_provider_level_binding_protocols() -> Result<(), AppError> {
        let local = Database::memory()?;
        set_provider_credential_state(
            &local,
            "system-openrouter-api",
            0x72,
            "provider/system-openrouter-api/local",
            4,
        )?;
        let agent_id = local.conn.lock().unwrap().query_row(
            "SELECT agent_module_id FROM agent_provider_bindings
             WHERE provider_id = 'system-openrouter-api' ORDER BY agent_module_id LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )?;

        let incoming = Database::memory()?;
        insert_import_sentinel(&incoming, "provider-level-protocol-import")?;
        incoming.conn.lock().unwrap().execute(
            "UPDATE agent_provider_bindings SET route_protocol = 'tampered-protocol'
             WHERE provider_id = 'system-openrouter-api' AND agent_module_id = ?1",
            [&agent_id],
        )?;

        let error = local
            .import_sql_string(&incoming.export_sql_string()?)
            .expect_err("Provider-level keys must protect every logical binding protocol");
        assert_eq!(error.to_string(), "credential_conflict");
        Ok(())
    }

    #[test]
    fn ordinary_redacted_import_preserves_local_provider_credential_metadata(
    ) -> Result<(), AppError> {
        let local = Database::memory()?;
        set_provider_credential_state(
            &local,
            "system-openrouter-api",
            0x71,
            "provider/system-openrouter-api/local",
            3,
        )?;
        let incoming = Database::memory()?;
        insert_import_sentinel(&incoming, "provider-credential-ordinary-import")?;

        local.import_sql_string(&incoming.export_sql_string()?)?;
        assert_eq!(
            provider_credential_tuple(&local, "system-openrouter-api")?,
            (
                vec![0x71; 32],
                "provider/system-openrouter-api/local".into(),
                3,
                Some(91),
                Some("success".into()),
            )
        );
        Ok(())
    }

    #[test]
    fn sync_import_preserves_local_provider_credential_and_connection_test_metadata(
    ) -> Result<(), AppError> {
        let local = Database::memory()?;
        set_provider_credential_state(
            &local,
            "system-openrouter-api",
            0x72,
            "provider/system-openrouter-api/local",
            4,
        )?;
        let incoming = Database::memory()?;
        insert_import_sentinel(&incoming, "provider-credential-sync-import")?;
        set_provider_credential_state(
            &incoming,
            "system-openrouter-api",
            0x73,
            "provider/system-openrouter-api/remote",
            8,
        )?;

        local.import_sql_string_for_sync(&incoming.export_sql_string_for_sync()?)?;
        assert_eq!(
            provider_credential_tuple(&local, "system-openrouter-api")?,
            (
                vec![0x72; 32],
                "provider/system-openrouter-api/local".into(),
                4,
                Some(91),
                Some("success".into()),
            )
        );
        Ok(())
    }

    #[test]
    fn import_repairs_canonical_system_metadata_and_preserves_seed_marker() -> Result<(), AppError>
    {
        let local = Database::memory()?;
        let incoming = Database::memory()?;
        insert_import_sentinel(&incoming, "system-canonical-import")?;
        {
            let conn = crate::database::lock_conn!(incoming.conn);
            conn.execute(
                "UPDATE usage_providers
                 SET name = 'Incoming Stale Router',
                     route_config = '{\"base_url\":\"https://attacker.invalid\"}'
                 WHERE id = 'system-openrouter-api'",
                [],
            )?;
            conn.execute(
                "UPDATE settings SET value = 'false'
                 WHERE key = 'system_provider_default_bindings_v1_seeded'",
                [],
            )?;
        }

        local.import_sql_string(&incoming.export_sql_string()?)?;
        let conn = crate::database::lock_conn!(local.conn);
        assert_eq!(
            conn.query_row(
                "SELECT name FROM usage_providers WHERE id = 'system-openrouter-api'",
                [],
                |row| row.get::<_, String>(0),
            )?,
            "OpenRouter"
        );
        assert_eq!(
            conn.query_row(
                "SELECT value FROM settings
                 WHERE key = 'system_provider_default_bindings_v1_seeded'",
                [],
                |row| row.get::<_, String>(0),
            )?,
            "true"
        );
        Ok(())
    }

    #[test]
    fn sync_export_skips_provider_credential_journal_rows() -> Result<(), AppError> {
        let db = Database::memory()?;
        let key_id = db.create_provider_api_key("system-openrouter-api", "Journal key")?;
        db.conn.lock().unwrap().execute(
            "INSERT INTO provider_credential_operations (
                 operation_id, key_id, generation, operation_kind,
                 status, staging_slot, previous_slot, created_at, updated_at
             ) VALUES (
                 'provider-journal-secret-sentinel', ?1, 1,
                 'set', 'pending', 'provider/staging/sentinel', NULL, 1, 1
             )",
            [&key_id],
        )?;

        let exported = db.export_sql_string_for_sync()?;
        assert!(!exported.contains("provider-journal-secret-sentinel"));
        assert!(!exported.contains("provider/staging/sentinel"));
        Ok(())
    }

    fn insert_custom_agent(
        db: &Database,
        agent_id: &str,
        ever_bound: bool,
    ) -> Result<(), AppError> {
        let conn = crate::database::lock_conn!(db.conn);
        conn.execute(
            "INSERT OR IGNORE INTO providers (id, app_type, name, settings_config, meta)
             VALUES ('ever-bound-import-provider', 'claude', 'Import Sentinel', '{}', '{}')",
            [],
        )?;
        conn.execute(
            "INSERT INTO agent_modules (
                 id, name, is_fixed, visible, sort_order, archived_at,
                 ever_bound, created_at, updated_at
             ) VALUES (?1, 'Backup Custom Agent', 0, 1, 90, NULL, ?2, 10, 10)",
            params![agent_id, ever_bound],
        )?;
        Ok(())
    }

    fn agent_ever_bound(db: &Database, agent_id: &str) -> Result<bool, AppError> {
        let conn = crate::database::lock_conn!(db.conn);
        conn.query_row(
            "SELECT ever_bound FROM agent_modules WHERE id = ?1",
            [agent_id],
            |row| row.get(0),
        )
        .map_err(AppError::from)
    }

    fn install_delayed_route_retarget_trigger(db: &Database) -> Result<(), AppError> {
        db.conn.lock().unwrap().execute_batch(
            "CREATE TRIGGER delayed_protected_route_retarget
             AFTER INSERT ON proxy_request_logs
             BEGIN
                 UPDATE usage_providers
                 SET route_config = '{\"base_url\":\"https://attacker.invalid/v1\"}'
                 WHERE id = 'backup-usage-provider';
             END;",
        )?;
        Ok(())
    }

    fn reserved_name_route_retarget_trigger_sql() -> &'static str {
        "CREATE TRIGGER sqlite_delayed_protected_route_retarget
         AFTER INSERT ON proxy_request_logs
         BEGIN
             UPDATE usage_providers
             SET route_config = '{\"base_url\":\"https://attacker.invalid/v1\"}'
             WHERE id = 'backup-usage-provider';
         END"
    }

    fn inject_reserved_name_trigger_into_sql(sql: &str) -> String {
        let trigger_sql = reserved_name_route_retarget_trigger_sql().replace('\'', "''");
        let payload = format!(
            "PRAGMA writable_schema=ON;\n\
             INSERT INTO sqlite_schema(type,name,tbl_name,rootpage,sql) VALUES(\
             'trigger','sqlite_delayed_protected_route_retarget','proxy_request_logs',0,\
             '{trigger_sql}');\n\
             PRAGMA writable_schema=OFF;\n"
        );
        sql.replacen("COMMIT;\n", &format!("{payload}COMMIT;\n"), 1)
    }

    fn install_reserved_name_route_retarget_trigger(db: &Database) -> Result<(), AppError> {
        let conn = db.conn.lock().unwrap();
        conn.execute_batch("PRAGMA writable_schema=ON;")?;
        let insert_result = conn.execute(
            "INSERT INTO sqlite_schema(type,name,tbl_name,rootpage,sql)
             VALUES('trigger', ?1, 'proxy_request_logs', 0, ?2)",
            params![
                "sqlite_delayed_protected_route_retarget",
                reserved_name_route_retarget_trigger_sql()
            ],
        );
        let disable_result = conn.execute_batch("PRAGMA writable_schema=OFF;");
        insert_result?;
        disable_result?;
        Ok(())
    }

    #[test]
    fn sql_import_rejects_removing_a_local_protected_credential() -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 1, false)?;

        let incoming = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(incoming.conn);
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('incoming-provider', 'claude', 'Incoming Provider', '{}', '{}')",
                [],
            )?;
        }
        let incoming_sql = incoming.export_sql_string()?;

        let error = local
            .import_sql_string(&incoming_sql)
            .expect_err("import must not orphan the local credential slot");
        assert_eq!(error.to_string(), "credential_conflict");
        assert_eq!(
            protected_tuple(&local, "protected-binding")?,
            (vec![0x5a_u8; 32], "local-slot".to_string(), 1)
        );
        Ok(())
    }

    #[test]
    fn redacted_sql_import_keeps_the_local_protected_credential_tuple() -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 1, false)?;

        let incoming = Database::memory()?;
        insert_protected_credential_state(
            &incoming,
            "protected-binding",
            "different-slot",
            2,
            false,
        )?;
        let incoming_sql = incoming.export_sql_string()?;

        local.import_sql_string(&incoming_sql)?;
        assert_eq!(
            protected_tuple(&local, "protected-binding")?,
            (vec![0x5a_u8; 32], "local-slot".to_string(), 1)
        );
        Ok(())
    }

    #[test]
    fn sql_import_rejects_retargeting_a_local_protected_credential() -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 1, false)?;

        let incoming = Database::memory()?;
        insert_protected_credential_state(&incoming, "protected-binding", "local-slot", 1, false)?;
        incoming.conn.lock().unwrap().execute(
            "UPDATE usage_providers
             SET route_config = '{\"base_url\":\"https://attacker.invalid/v1\"}'
             WHERE id = 'backup-usage-provider'",
            [],
        )?;

        let error = local
            .import_sql_string(&incoming.export_sql_string()?)
            .expect_err("import must not retarget a locally protected API key");
        assert_eq!(error.to_string(), "credential_conflict");
        Ok(())
    }

    #[test]
    fn sql_import_rejects_a_delayed_route_retarget_trigger() -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 1, false)?;

        let incoming = Database::memory()?;
        insert_protected_credential_state(&incoming, "protected-binding", "local-slot", 1, false)?;
        install_delayed_route_retarget_trigger(&incoming)?;

        let error = local
            .import_sql_string(&incoming.export_sql_string()?)
            .expect_err("import must reject dormant schema code that can retarget a local key");
        assert_eq!(error.to_string(), "credential_conflict");
        Ok(())
    }

    #[test]
    fn sql_import_rejects_a_reserved_name_delayed_retarget_trigger() -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 1, false)?;

        let incoming = Database::memory()?;
        insert_protected_credential_state(&incoming, "protected-binding", "local-slot", 1, false)?;
        let malicious_sql = inject_reserved_name_trigger_into_sql(&incoming.export_sql_string()?);

        let error = local
            .import_sql_string(&malicious_sql)
            .expect_err("import must inspect reserved-name trigger rows too");
        assert_eq!(error.to_string(), "credential_conflict");
        Ok(())
    }

    #[test]
    fn sql_import_rejects_changing_protected_auth_classification() -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 1, false)?;

        let incoming = Database::memory()?;
        insert_protected_credential_state(&incoming, "protected-binding", "local-slot", 1, false)?;
        incoming.conn.lock().unwrap().execute(
            "UPDATE usage_providers
             SET token_sources = '[\"session_log\"]'
             WHERE id = 'backup-usage-provider'",
            [],
        )?;

        let error = local
            .import_sql_string(&incoming.export_sql_string()?)
            .expect_err("import must not change auth classification around a local key");
        assert_eq!(error.to_string(), "credential_conflict");
        Ok(())
    }

    #[test]
    fn sql_import_rejects_changing_protected_legacy_auth_context() -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 1, false)?;
        {
            let conn = local.conn.lock().unwrap();
            conn.execute(
                "UPDATE providers
                 SET settings_config = '{\"providerType\":\"codex_oauth\"}'
                 WHERE id = 'backup-provider' AND app_type = 'claude'",
                [],
            )?;
            conn.execute(
                "UPDATE usage_providers
                 SET legacy_app_type = 'claude', legacy_provider_id = 'backup-provider'
                 WHERE id = 'backup-usage-provider'",
                [],
            )?;
        }

        let incoming = Database::memory()?;
        insert_protected_credential_state(&incoming, "protected-binding", "local-slot", 1, false)?;
        incoming.conn.lock().unwrap().execute(
            "UPDATE usage_providers
             SET legacy_app_type = 'claude', legacy_provider_id = 'backup-provider'
             WHERE id = 'backup-usage-provider'",
            [],
        )?;

        let error = local
            .import_sql_string(&incoming.export_sql_string()?)
            .expect_err("import must not remove managed-auth evidence around a local key");
        assert_eq!(error.to_string(), "credential_conflict");
        Ok(())
    }

    #[test]
    fn sql_import_rejects_reenabling_protected_routing_state() -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 1, false)?;
        {
            let conn = local.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO agent_modules (
                     id, name, sort_order, visible, is_fixed, archived_at,
                     created_at, updated_at
                 ) VALUES ('security-agent', 'Security Agent', 100, 1, 0, NULL, 10, 10)",
                [],
            )?;
            conn.execute(
                "UPDATE agent_provider_bindings
                 SET agent_module_id = 'security-agent', enabled = 0
                 WHERE id = 'protected-binding'",
                [],
            )?;
            conn.execute(
                "UPDATE usage_providers SET enabled = 0
                 WHERE id = 'backup-usage-provider'",
                [],
            )?;
            conn.execute(
                "UPDATE agent_modules SET archived_at = 42 WHERE id = 'security-agent'",
                [],
            )?;
        }

        let incoming = Database::memory()?;
        insert_protected_credential_state(&incoming, "protected-binding", "local-slot", 1, false)?;
        {
            let conn = incoming.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO agent_modules (
                     id, name, sort_order, visible, is_fixed, archived_at,
                     created_at, updated_at
                 ) VALUES ('security-agent', 'Security Agent', 100, 1, 0, NULL, 10, 10)",
                [],
            )?;
            conn.execute(
                "UPDATE agent_provider_bindings SET agent_module_id = 'security-agent'
                 WHERE id = 'protected-binding'",
                [],
            )?;
        }

        let error = local
            .import_sql_string(&incoming.export_sql_string()?)
            .expect_err("import must not re-enable any local protected route");
        assert_eq!(error.to_string(), "credential_conflict");
        Ok(())
    }

    #[test]
    fn sql_import_rejects_removing_a_local_credential_journal_row() -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 1, true)?;

        let incoming = Database::memory()?;
        insert_protected_credential_state(&incoming, "protected-binding", "local-slot", 1, false)?;
        let incoming_sql = incoming.export_sql_string()?;

        let error = local
            .import_sql_string(&incoming_sql)
            .expect_err("import must preserve every local lifecycle journal row");
        assert_eq!(error.to_string(), "credential_conflict");
        let journal_count = {
            let conn = crate::database::lock_conn!(local.conn);
            conn.query_row(
                "SELECT COUNT(*) FROM agent_credential_operations
                 WHERE operation_id = 'backup-cleanup-operation'",
                [],
                |row| row.get::<_, i64>(0),
            )?
        };
        assert_eq!(journal_count, 1);
        Ok(())
    }

    #[test]
    fn redacted_sql_import_drops_remote_credentials_and_journal_rows() -> Result<(), AppError> {
        let local = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(local.conn);
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('local-provider', 'claude', 'Local Provider', '{}', '{}')",
                [],
            )?;
        }

        let incoming = Database::memory()?;
        insert_protected_credential_state(
            &incoming,
            "remote-protected-binding",
            "remote-slot",
            1,
            true,
        )?;
        let incoming_sql = incoming.export_sql_string()?;

        local.import_sql_string(&incoming_sql)?;
        assert_eq!(
            optional_protected_tuple(&local, "remote-protected-binding")?,
            (None, None, 1)
        );
        let journal_count = local.conn.lock().unwrap().query_row(
            "SELECT COUNT(*) FROM agent_credential_operations",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        assert_eq!(journal_count, 0);
        Ok(())
    }

    #[test]
    fn redacted_sql_import_rejects_an_injected_credential_journal_row() -> Result<(), AppError> {
        let local = Database::memory()?;
        let incoming = Database::memory()?;
        insert_protected_credential_state(
            &incoming,
            "remote-protected-binding",
            "remote-slot",
            1,
            false,
        )?;
        let payload = "INSERT INTO agent_credential_operations (
             operation_id, binding_id, generation, operation_kind,
             status, staging_slot, previous_slot, created_at, updated_at
         ) VALUES (
             'injected-remote-operation', 'remote-protected-binding', 1,
             'replace', 'cleanup', 'injected-slot', NULL, 11, 12
         );\n";
        let tampered =
            incoming
                .export_sql_string()?
                .replacen("COMMIT;\n", &format!("{payload}COMMIT;\n"), 1);

        let error = local
            .import_sql_string(&tampered)
            .expect_err("marked backups must reject an injected credential journal");
        assert_eq!(error.to_string(), "credential_conflict");
        Ok(())
    }

    #[test]
    fn redacted_sql_import_rejects_an_injected_credential_slot() -> Result<(), AppError> {
        let local = Database::memory()?;
        let incoming = Database::memory()?;
        insert_protected_credential_state(
            &incoming,
            "remote-protected-binding",
            "remote-slot",
            1,
            false,
        )?;
        let tampered = incoming.export_sql_string()?.replacen(
            "COMMIT;\n",
            "UPDATE agent_provider_bindings
             SET api_key_fingerprint = X'5A5A5A5A5A5A5A5A5A5A5A5A5A5A5A5A5A5A5A5A5A5A5A5A5A5A5A5A5A5A5A5A',
                 credential_slot = 'injected-slot'
             WHERE id = 'remote-protected-binding';
             COMMIT;\n",
            1,
        );

        let error = local
            .import_sql_string(&tampered)
            .expect_err("marked backups must reject an injected Keychain slot");
        assert_eq!(error.to_string(), "credential_conflict");
        Ok(())
    }

    #[test]
    fn redacted_sql_import_rejects_a_local_provider_credential_journal() -> Result<(), AppError> {
        let local = Database::memory()?;
        let key_id = local.create_provider_api_key("system-openrouter-api", "Journal key")?;
        local.conn.lock().unwrap().execute(
            "INSERT INTO provider_credential_operations (
                 operation_id, key_id, generation, operation_kind,
                 status, staging_slot, previous_slot, created_at, updated_at
             ) VALUES (
                 'local-provider-operation', ?1, 1,
                 'set', 'pending', 'provider/staging/local', NULL, 10, 10
             )",
            [&key_id],
        )?;
        let incoming = Database::memory()?;
        insert_import_sentinel(&incoming, "provider-journal-import")?;

        let error = local
            .import_sql_string(&incoming.export_sql_string()?)
            .expect_err("restore must not discard local Provider credential lifecycle work");
        assert_eq!(error.to_string(), "credential_conflict");
        Ok(())
    }

    #[test]
    fn sync_export_omits_device_local_credential_journal_rows() -> Result<(), AppError> {
        let db = Database::memory()?;
        insert_protected_credential_state(&db, "protected-binding", "local-slot", 1, true)?;

        let sync_sql = db.export_sql_string_for_sync()?;

        assert!(!sync_sql.contains("INSERT INTO \"agent_credential_operations\""));
        assert!(!sync_sql.contains("backup-cleanup-operation"));
        Ok(())
    }

    #[test]
    fn ordinary_redacted_import_preserves_ever_bound_tombstone() -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_custom_agent(&local, "formerly-bound-agent", true)?;
        let incoming = Database::memory()?;
        insert_custom_agent(&incoming, "formerly-bound-agent", false)?;

        local.import_sql_string(&incoming.export_sql_string()?)?;
        assert!(agent_ever_bound(&local, "formerly-bound-agent")?);
        Ok(())
    }

    #[test]
    fn sync_import_preserves_local_ever_bound_tombstone() -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_custom_agent(&local, "formerly-bound-agent", true)?;
        let incoming = Database::memory()?;
        insert_custom_agent(&incoming, "formerly-bound-agent", false)?;

        local.import_sql_string_for_sync(&incoming.export_sql_string_for_sync()?)?;

        assert!(agent_ever_bound(&local, "formerly-bound-agent")?);
        Ok(())
    }

    #[test]
    fn sync_import_preserves_local_active_credential_columns() -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 3, false)?;

        let incoming = Database::memory()?;
        insert_protected_credential_state(&incoming, "protected-binding", "remote-slot", 7, false)?;
        incoming.conn.lock().unwrap().execute(
            "UPDATE agent_provider_bindings SET api_key_fingerprint = ?1
             WHERE id = 'protected-binding'",
            [vec![0x6b_u8; 32]],
        )?;

        local.import_sql_string_for_sync(&incoming.export_sql_string_for_sync()?)?;

        assert_eq!(
            protected_tuple(&local, "protected-binding")?,
            (vec![0x5a_u8; 32], "local-slot".to_string(), 3)
        );
        Ok(())
    }

    #[test]
    fn sync_import_preserves_local_active_credential_when_remote_is_cleared() -> Result<(), AppError>
    {
        let local = Database::memory()?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 3, false)?;

        let incoming = Database::memory()?;
        insert_protected_credential_state(&incoming, "protected-binding", "remote-slot", 7, false)?;
        incoming.conn.lock().unwrap().execute(
            "UPDATE agent_provider_bindings
             SET api_key_fingerprint = NULL, credential_slot = NULL,
                 credential_version = 8
             WHERE id = 'protected-binding'",
            [],
        )?;

        local.import_sql_string_for_sync(&incoming.export_sql_string_for_sync()?)?;

        assert_eq!(
            protected_tuple(&local, "protected-binding")?,
            (vec![0x5a_u8; 32], "local-slot".to_string(), 3)
        );
        Ok(())
    }

    #[test]
    fn sync_import_rejects_retargeting_a_local_protected_credential() -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 3, false)?;

        let incoming = Database::memory()?;
        insert_protected_credential_state(&incoming, "protected-binding", "remote-slot", 7, false)?;
        incoming.conn.lock().unwrap().execute(
            "UPDATE usage_providers
             SET route_config = '{\"base_url\":\"https://attacker.invalid/v1\"}'
             WHERE id = 'backup-usage-provider'",
            [],
        )?;

        let error = local
            .import_sql_string_for_sync(&incoming.export_sql_string_for_sync()?)
            .expect_err("sync must not retarget a locally protected API key");
        assert_eq!(error.to_string(), "credential_conflict");
        assert_eq!(
            protected_tuple(&local, "protected-binding")?,
            (vec![0x5a_u8; 32], "local-slot".to_string(), 3)
        );
        Ok(())
    }

    #[test]
    fn sync_import_rejects_a_delayed_route_retarget_trigger() -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 3, false)?;

        let incoming = Database::memory()?;
        insert_protected_credential_state(&incoming, "protected-binding", "remote-slot", 7, false)?;
        install_delayed_route_retarget_trigger(&incoming)?;

        let error = local
            .import_sql_string_for_sync(&incoming.export_sql_string_for_sync()?)
            .expect_err("sync must reject dormant schema code that can retarget a local key");
        assert_eq!(error.to_string(), "credential_conflict");
        Ok(())
    }

    #[test]
    fn sync_import_rejects_a_reserved_name_delayed_retarget_trigger() -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 3, false)?;

        let incoming = Database::memory()?;
        insert_protected_credential_state(&incoming, "protected-binding", "remote-slot", 7, false)?;
        let malicious_sql =
            inject_reserved_name_trigger_into_sql(&incoming.export_sql_string_for_sync()?);

        let error = local
            .import_sql_string_for_sync(&malicious_sql)
            .expect_err("sync must inspect reserved-name trigger rows too");
        assert_eq!(error.to_string(), "credential_conflict");
        Ok(())
    }

    #[test]
    fn sync_import_rejects_remote_deletion_of_a_locally_protected_binding() -> Result<(), AppError>
    {
        let local = Database::memory()?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 1, false)?;

        let incoming = Database::memory()?;
        insert_protected_credential_state(&incoming, "protected-binding", "remote-slot", 1, false)?;
        incoming.conn.lock().unwrap().execute(
            "DELETE FROM agent_provider_bindings WHERE id = 'protected-binding'",
            [],
        )?;

        let error = local
            .import_sql_string_for_sync(&incoming.export_sql_string_for_sync()?)
            .expect_err("sync must not orphan a local protected item");
        assert_eq!(error.to_string(), "credential_conflict");
        assert_eq!(
            protected_tuple(&local, "protected-binding")?,
            (vec![0x5a_u8; 32], "local-slot".to_string(), 1)
        );
        Ok(())
    }

    #[test]
    fn sync_import_rejects_rebinding_a_locally_protected_item_to_another_agent(
    ) -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 1, false)?;

        let incoming = Database::memory()?;
        insert_protected_credential_state(&incoming, "protected-binding", "remote-slot", 1, false)?;
        incoming.conn.lock().unwrap().execute(
            "UPDATE agent_provider_bindings SET agent_module_id = 'codex'
             WHERE id = 'protected-binding'",
            [],
        )?;

        let error = local
            .import_sql_string_for_sync(&incoming.export_sql_string_for_sync()?)
            .expect_err("sync must not rebind a local protected item");
        assert_eq!(error.to_string(), "credential_conflict");
        assert_eq!(
            protected_tuple(&local, "protected-binding")?,
            (vec![0x5a_u8; 32], "local-slot".to_string(), 1)
        );
        Ok(())
    }

    #[test]
    fn ordinary_import_is_allowed_after_the_local_credential_was_cleared() -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 1, false)?;
        local.conn.lock().unwrap().execute(
            "UPDATE agent_provider_bindings
             SET api_key_fingerprint = NULL, credential_slot = NULL,
                 credential_version = 2, enabled = 0
             WHERE id = 'protected-binding'",
            [],
        )?;

        let incoming = Database::memory()?;
        insert_protected_credential_state(
            &incoming,
            "protected-binding",
            "restored-missing-slot",
            1,
            false,
        )?;

        local.import_sql_string(&incoming.export_sql_string()?)?;
        assert_eq!(
            optional_protected_tuple(&local, "protected-binding")?,
            (None, None, 1)
        );
        Ok(())
    }

    #[test]
    fn sql_import_accepts_an_exact_superset_of_local_protected_state() -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 1, false)?;

        let incoming = Database::memory()?;
        insert_protected_credential_state(&incoming, "protected-binding", "local-slot", 1, false)?;
        {
            let conn = crate::database::lock_conn!(incoming.conn);
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('incoming-extra', 'claude', 'Incoming Extra', '{}', '{}')",
                [],
            )?;
            conn.execute(
                "INSERT INTO usage_providers (
                     id, name, billing_kind, product_group_id, token_sources,
                     route_app_type, route_config, enabled, needs_review,
                     created_at, updated_at
                 ) VALUES (
                     'remote-usage-provider', 'Remote Usage Provider', 'metered', 'backup',
                     '[\"proxy\"]', 'codex', '{}', 1, 0, 10, 10
                 )",
                [],
            )?;
            conn.execute(
                "INSERT INTO agent_provider_bindings (
                     id, agent_module_id, provider_id, enabled,
                     api_key_fingerprint, credential_slot, credential_version,
                     created_at, updated_at
                 ) VALUES (
                     'remote-protected-binding', 'codex', 'remote-usage-provider', 1,
                     ?1, 'remote-slot', 1, 10, 10
                 )",
                [vec![0x6b_u8; 32]],
            )?;
        }
        let incoming_sql = incoming.export_sql_string()?;

        local.import_sql_string(&incoming_sql)?;
        assert_eq!(
            protected_tuple(&local, "protected-binding")?,
            (vec![0x5a_u8; 32], "local-slot".to_string(), 1)
        );
        assert_eq!(
            optional_protected_tuple(&local, "remote-protected-binding")?,
            (None, None, 1)
        );
        Ok(())
    }

    #[test]
    fn binary_restore_rejects_a_backup_missing_local_protected_state() -> Result<(), AppError> {
        let temp = tempfile::tempdir().expect("create binary restore test root");
        let local_dir = temp.path().join("local");
        let incoming_dir = temp.path().join("incoming");
        std::fs::create_dir_all(&local_dir).expect("create local database directory");
        std::fs::create_dir_all(&incoming_dir).expect("create incoming database directory");

        let local_path = local_dir.join(DATABASE_FILE);
        let local = Database::init_at(&local_path)?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 1, false)?;

        let incoming_path = incoming_dir.join(DATABASE_FILE);
        let incoming = Database::init_at(&incoming_path)?;
        {
            let conn = crate::database::lock_conn!(incoming.conn);
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('incoming-provider', 'claude', 'Incoming Provider', '{}', '{}')",
                [],
            )?;
        }
        let incoming_backup = incoming
            .backup_database_file()?
            .expect("file database produces a backup");
        let local_backup_dir = local_dir.join("backups");
        std::fs::create_dir_all(&local_backup_dir).expect("create local backup directory");
        std::fs::copy(
            &incoming_backup,
            local_backup_dir.join("missing-protected.db"),
        )
        .expect("copy incoming snapshot into local backup directory");

        let error = local
            .restore_from_backup("missing-protected.db")
            .expect_err("binary restore must not orphan the local credential slot");
        assert_eq!(error.to_string(), "credential_conflict");
        assert_eq!(
            protected_tuple(&local, "protected-binding")?,
            (vec![0x5a_u8; 32], "local-slot".to_string(), 1)
        );
        Ok(())
    }

    #[test]
    fn binary_restore_rejects_retargeting_a_local_protected_credential() -> Result<(), AppError> {
        let temp = tempfile::tempdir().expect("create binary restore test root");
        let local_dir = temp.path().join("local");
        let incoming_dir = temp.path().join("incoming");
        std::fs::create_dir_all(&local_dir).expect("create local database directory");
        std::fs::create_dir_all(&incoming_dir).expect("create incoming database directory");

        let local_path = local_dir.join(DATABASE_FILE);
        let local = Database::init_at(&local_path)?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 1, false)?;

        let incoming_path = incoming_dir.join(DATABASE_FILE);
        let incoming = Database::init_at(&incoming_path)?;
        insert_protected_credential_state(&incoming, "protected-binding", "local-slot", 1, false)?;
        incoming.conn.lock().unwrap().execute(
            "UPDATE usage_providers
             SET route_config = '{\"base_url\":\"https://attacker.invalid/v1\"}'
             WHERE id = 'backup-usage-provider'",
            [],
        )?;
        let incoming_backup = incoming
            .backup_database_file()?
            .expect("file database produces a backup");
        let local_backup_dir = local_dir.join("backups");
        std::fs::create_dir_all(&local_backup_dir).expect("create local backup directory");
        std::fs::copy(
            &incoming_backup,
            local_backup_dir.join("retargeted-protected.db"),
        )
        .expect("copy retargeted snapshot into local backup directory");

        let error = local
            .restore_from_backup("retargeted-protected.db")
            .expect_err("binary restore must not retarget a local protected API key");
        assert_eq!(error.to_string(), "credential_conflict");
        assert_eq!(
            protected_tuple(&local, "protected-binding")?,
            (vec![0x5a_u8; 32], "local-slot".to_string(), 1)
        );
        Ok(())
    }

    #[test]
    fn binary_restore_rejects_a_delayed_route_retarget_trigger() -> Result<(), AppError> {
        let temp = tempfile::tempdir().expect("create binary restore test root");
        let local_dir = temp.path().join("local");
        let incoming_dir = temp.path().join("incoming");
        std::fs::create_dir_all(&local_dir).expect("create local database directory");
        std::fs::create_dir_all(&incoming_dir).expect("create incoming database directory");

        let local_path = local_dir.join(DATABASE_FILE);
        let local = Database::init_at(&local_path)?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 1, false)?;

        let incoming_path = incoming_dir.join(DATABASE_FILE);
        let incoming = Database::init_at(&incoming_path)?;
        insert_protected_credential_state(&incoming, "protected-binding", "local-slot", 1, false)?;
        install_delayed_route_retarget_trigger(&incoming)?;
        let incoming_backup = incoming
            .backup_database_file()?
            .expect("file database produces a backup");
        let local_backup_dir = local_dir.join("backups");
        std::fs::create_dir_all(&local_backup_dir).expect("create local backup directory");
        std::fs::copy(
            &incoming_backup,
            local_backup_dir.join("trigger-protected.db"),
        )
        .expect("copy trigger snapshot into local backup directory");

        let error = local
            .restore_from_backup("trigger-protected.db")
            .expect_err("binary restore must reject dormant schema code around a local key");
        assert_eq!(error.to_string(), "credential_conflict");
        Ok(())
    }

    #[test]
    fn binary_restore_rejects_a_reserved_name_delayed_retarget_trigger() -> Result<(), AppError> {
        let temp = tempfile::tempdir().expect("create binary restore test root");
        let local_dir = temp.path().join("local");
        let incoming_dir = temp.path().join("incoming");
        std::fs::create_dir_all(&local_dir).expect("create local database directory");
        std::fs::create_dir_all(&incoming_dir).expect("create incoming database directory");

        let local_path = local_dir.join(DATABASE_FILE);
        let local = Database::init_at(&local_path)?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 1, false)?;

        let incoming_path = incoming_dir.join(DATABASE_FILE);
        let incoming = Database::init_at(&incoming_path)?;
        insert_protected_credential_state(&incoming, "protected-binding", "local-slot", 1, false)?;
        install_reserved_name_route_retarget_trigger(&incoming)?;
        let incoming_backup = incoming
            .backup_database_file()?
            .expect("file database produces a backup");
        let local_backup_dir = local_dir.join("backups");
        std::fs::create_dir_all(&local_backup_dir).expect("create local backup directory");
        std::fs::copy(
            &incoming_backup,
            local_backup_dir.join("reserved-trigger-protected.db"),
        )
        .expect("copy reserved trigger snapshot into local backup directory");

        let error = local
            .restore_from_backup("reserved-trigger-protected.db")
            .expect_err("binary restore must inspect reserved-name trigger rows too");
        assert_eq!(error.to_string(), "credential_conflict");
        Ok(())
    }

    #[test]
    fn sql_export_uses_llm_usage_bar_header() -> Result<(), AppError> {
        let db = Database::memory()?;
        let sql = db.export_sql_string()?;

        assert!(sql.starts_with(&format!("{LLM_USAGE_BAR_SQL_EXPORT_HEADER}\n")));
        assert!(!sql.starts_with(&format!("{LEGACY_CC_SWITCH_SQL_EXPORT_HEADER}\n")));
        Ok(())
    }

    #[test]
    fn provider_daily_budgets_round_trip_through_production_sql_backup() -> Result<(), AppError> {
        let db = Database::memory()?;
        db.conn.lock().unwrap().execute(
            "INSERT INTO providers (id, app_type, name, settings_config, meta)
             VALUES ('budget-backup-sentinel', 'claude', 'Budget Backup', '{}', '{}')",
            [],
        )?;
        for id in ["backup-metered", "backup-metered-null"] {
            db.save_usage_provider(&UsageProviderInput {
                id: id.to_string(),
                name: format!("Provider {id}"),
                billing_kind: BillingKind::Metered,
                product_group_id: "backup".to_string(),
                token_sources: vec![TokenSource::Proxy],
                session_source_bindings: None,
                quota_source: None,
                quota_interval_seconds: None,
                route_app_type: None,
                route_config: None,
                quota_config: None,
                enabled: true,
            })?;
        }
        db.set_provider_daily_budget("backup-metered", Some("42.7500"))?;

        let archive = db.export_sql_string()?;
        assert!(archive.contains("\"daily_budget_usd\""));
        let restored = Database::memory()?;
        restored.import_sql_string(&archive)?;

        assert_eq!(
            restored
                .get_usage_provider("backup-metered")?
                .unwrap()
                .daily_budget_usd
                .as_deref(),
            Some("42.75"),
        );
        assert_eq!(
            restored
                .get_usage_provider("backup-metered-null")?
                .unwrap()
                .daily_budget_usd,
            None,
        );
        Ok(())
    }

    fn insert_metered_budget_row(
        db: &Database,
        provider_id: &str,
        raw_budget: &str,
    ) -> Result<(), AppError> {
        insert_metered_budget_row_for_conn(&db.conn.lock().unwrap(), provider_id, raw_budget)
    }

    fn insert_metered_budget_row_for_conn(
        conn: &rusqlite::Connection,
        provider_id: &str,
        raw_budget: &str,
    ) -> Result<(), AppError> {
        conn.execute(
            "INSERT INTO usage_providers (
                 id, name, billing_kind, product_group_id, token_sources,
                 enabled, needs_review, created_at, updated_at, daily_budget_usd
             ) VALUES (?1, 'Budget Import Sentinel', 'metered', 'budget-import',
                 '[\"proxy\"]', 1, 0, 10, 10, ?2)",
            params![provider_id, raw_budget],
        )?;
        Ok(())
    }

    #[test]
    fn sql_import_rejects_invalid_daily_budget_without_replacing_live_state() -> Result<(), AppError>
    {
        let local = Database::memory()?;
        insert_import_sentinel(&local, "live-budget-import-sentinel")?;
        let incoming = Database::memory()?;
        insert_import_sentinel(&incoming, "incoming-budget-import-sentinel")?;
        insert_metered_budget_row(&incoming, "invalid-import-budget", "0")?;

        let error = local
            .import_sql_string(&incoming.export_sql_string()?)
            .expect_err("zero daily budget must be rejected before replacing live state");
        assert!(error.to_string().contains("invalid provider daily budget"));
        assert!(local
            .get_provider_by_id("live-budget-import-sentinel", "claude")?
            .is_some());
        assert!(local
            .get_provider_by_id("incoming-budget-import-sentinel", "claude")?
            .is_none());
        Ok(())
    }

    #[test]
    fn sql_import_rejects_a_subscription_daily_budget_without_replacing_live_state(
    ) -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_import_sentinel(&local, "live-subscription-budget-sentinel")?;
        let incoming = Database::memory()?;
        incoming.conn.lock().unwrap().execute(
            "UPDATE usage_providers SET daily_budget_usd = '10'
             WHERE id = 'system-chatgpt-subscription'",
            [],
        )?;

        let error = local
            .import_sql_string(&incoming.export_sql_string()?)
            .expect_err("subscription Providers must never accept a daily spend budget");
        assert!(error.to_string().contains("invalid provider daily budget"));
        assert!(local
            .get_provider_by_id("live-subscription-budget-sentinel", "claude")?
            .is_some());
        Ok(())
    }

    #[test]
    fn sql_import_normalizes_valid_daily_budget_text() -> Result<(), AppError> {
        let local = Database::memory()?;
        let incoming = Database::memory()?;
        insert_import_sentinel(&incoming, "normalized-budget-import-sentinel")?;
        insert_metered_budget_row(&incoming, "normalized-import-budget", "42.7500")?;

        local.import_sql_string(&incoming.export_sql_string()?)?;
        let stored = local.conn.lock().unwrap().query_row(
            "SELECT daily_budget_usd FROM usage_providers
             WHERE id = 'normalized-import-budget'",
            [],
            |row| row.get::<_, String>(0),
        )?;
        assert_eq!(stored, "42.75");
        Ok(())
    }

    #[test]
    fn binary_restore_rejects_invalid_daily_budget_without_replacing_live_state(
    ) -> Result<(), AppError> {
        let temp = tempfile::tempdir().expect("create invalid budget restore root");
        let db_path = temp.path().join(DATABASE_FILE);
        let db = Database::init_at(&db_path)?;
        insert_import_sentinel(&db, "live-binary-budget-sentinel")?;
        let backup_path = db
            .backup_database_file()?
            .expect("file database produces a backup");
        {
            let backup = rusqlite::Connection::open(&backup_path)?;
            insert_metered_budget_row_for_conn(&backup, "invalid-binary-restore-budget", "-1")?;
        }
        let filename = backup_path
            .file_name()
            .expect("backup filename")
            .to_string_lossy()
            .into_owned();

        let error = db
            .restore_from_backup(&filename)
            .expect_err("negative daily budget must be rejected before restore");
        assert!(error.to_string().contains("invalid provider daily budget"));
        assert!(db
            .get_provider_by_id("live-binary-budget-sentinel", "claude")?
            .is_some());
        assert!(db
            .get_usage_provider("invalid-binary-restore-budget")?
            .is_none());
        Ok(())
    }

    #[test]
    fn binary_restore_rejects_a_subscription_daily_budget_without_replacing_live_state(
    ) -> Result<(), AppError> {
        let temp = tempfile::tempdir().expect("create subscription budget restore root");
        let db_path = temp.path().join(DATABASE_FILE);
        let db = Database::init_at(&db_path)?;
        insert_import_sentinel(&db, "live-binary-subscription-budget-sentinel")?;
        let backup_path = db
            .backup_database_file()?
            .expect("file database produces a backup");
        rusqlite::Connection::open(&backup_path)?.execute(
            "UPDATE usage_providers SET daily_budget_usd = '10'
             WHERE id = 'system-chatgpt-subscription'",
            [],
        )?;
        let filename = backup_path
            .file_name()
            .expect("backup filename")
            .to_string_lossy()
            .into_owned();

        let error = db
            .restore_from_backup(&filename)
            .expect_err("binary restore must reject a subscription daily budget");
        assert!(error.to_string().contains("invalid provider daily budget"));
        assert!(db
            .get_provider_by_id("live-binary-subscription-budget-sentinel", "claude")?
            .is_some());
        Ok(())
    }

    #[test]
    fn sql_import_header_accepts_current_and_exact_legacy_bytes() {
        let current = format!("{LLM_USAGE_BAR_SQL_EXPORT_HEADER}\nBEGIN TRANSACTION;");
        let legacy = format!("{LEGACY_CC_SWITCH_SQL_EXPORT_HEADER}\nBEGIN TRANSACTION;");
        let near_miss =
            format!("{LEGACY_CC_SWITCH_SQL_EXPORT_HEADER} with suffix\nBEGIN TRANSACTION;");

        assert!(Database::validate_llm_usage_bar_sql_export(&current).is_ok());
        assert!(Database::validate_llm_usage_bar_sql_export(&legacy).is_ok());
        assert!(Database::validate_llm_usage_bar_sql_export(&near_miss).is_err());
    }

    #[test]
    #[serial]
    fn backup_management_stays_on_instance_path_after_global_directory_switch(
    ) -> Result<(), AppError> {
        let _restore = TestHomeRestore(std::env::var_os("LLM_USAGE_BAR_TEST_HOME"));
        let temp = tempfile::tempdir().expect("create backup path test root");
        let home_a = temp.path().join("home-a");
        let home_b = temp.path().join("home-b");
        let app_dir_a = home_a.join(".llm-usage-bar");
        let app_dir_b = home_b.join(".llm-usage-bar");
        std::fs::create_dir_all(&app_dir_a).expect("create A app directory");
        std::fs::create_dir_all(app_dir_b.join("backups")).expect("create B backup directory");

        std::env::set_var("LLM_USAGE_BAR_TEST_HOME", &home_a);
        let db = Database::init_at(&app_dir_a.join(DATABASE_FILE))?;

        // Simulate the settings hot-switch: global path helpers now resolve B,
        // while the live Database connection remains authoritative for A.
        std::env::set_var("LLM_USAGE_BAR_TEST_HOME", &home_b);
        let created = db
            .backup_database_file()?
            .expect("file-backed database creates a backup");
        assert_eq!(created.parent(), Some(app_dir_a.join("backups").as_path()));
        let created_name = created
            .file_name()
            .expect("backup filename")
            .to_string_lossy()
            .into_owned();

        let b_same_name = app_dir_b.join("backups").join(&created_name);
        let b_only = app_dir_b.join("backups").join("b-only.db");
        std::fs::write(&b_same_name, b"B-same-name-sentinel").expect("seed B same-name sentinel");
        std::fs::write(&b_only, b"B-only-sentinel").expect("seed B-only sentinel");

        let listed = db.list_backups()?;
        assert!(listed.iter().any(|entry| entry.filename == created_name));
        assert!(!listed.iter().any(|entry| entry.filename == "b-only.db"));

        let renamed = db.rename_backup(&created_name, "renamed-in-a")?;
        assert_eq!(renamed, "renamed-in-a.db");
        assert!(!created.exists());
        assert!(app_dir_a.join("backups").join(&renamed).exists());
        assert_eq!(
            std::fs::read(&b_same_name).expect("read B same-name sentinel"),
            b"B-same-name-sentinel"
        );
        assert!(!app_dir_b.join("backups").join(&renamed).exists());

        db.delete_backup(&renamed)?;
        assert!(!app_dir_a.join("backups").join(&renamed).exists());
        assert_eq!(
            std::fs::read(&b_same_name).expect("read B same-name sentinel after delete"),
            b"B-same-name-sentinel"
        );
        assert_eq!(
            std::fs::read(&b_only).expect("read B-only sentinel after delete"),
            b"B-only-sentinel"
        );

        Ok(())
    }

    #[test]
    fn memory_backup_management_is_empty_and_mutations_are_rejected() -> Result<(), AppError> {
        let db = Database::memory()?;
        assert!(db.list_backups()?.is_empty());

        let rename_error = db
            .rename_backup("anything.db", "renamed")
            .expect_err("memory database rename must fail");
        assert!(rename_error.to_string().contains("内存数据库"));

        let delete_error = db
            .delete_backup("anything.db")
            .expect_err("memory database delete must fail");
        assert!(delete_error.to_string().contains("内存数据库"));
        Ok(())
    }

    #[test]
    fn sync_import_preserves_local_only_tables() -> Result<(), AppError> {
        let remote_db = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(remote_db.conn);
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('remote-provider', 'claude', 'Remote Provider', '{}', '{}')",
                [],
            )?;
        }
        let remote_sql = remote_db.export_sql_string_for_sync()?;

        let local_db = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(local_db.conn);
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('local-provider', 'claude', 'Local Provider', '{}', '{}')",
                [],
            )?;
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model,
                    input_tokens, output_tokens, total_cost_usd,
                    latency_ms, status_code, created_at
                ) VALUES ('req-1', 'local-provider', 'claude', 'claude-3', 100, 50, '0.01', 120, 200, 1000)",
                [],
            )?;
            conn.execute(
                "INSERT INTO usage_daily_rollups (
                    date, app_type, provider_id, model, request_count, success_count,
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    total_cost_usd, avg_latency_ms
                ) VALUES ('2026-03-01', 'claude', 'local-provider', 'claude-3', 7, 7, 700, 350, 0, 0, '0.07', 120)",
                [],
            )?;
        }

        local_db.import_sql_string_for_sync(&remote_sql)?;

        let remote_provider_exists: i64 = {
            let conn = crate::database::lock_conn!(local_db.conn);
            conn.query_row(
                "SELECT COUNT(*) FROM providers WHERE id = 'remote-provider' AND app_type = 'claude'",
                [],
                |row| row.get(0),
            )?
        };
        assert_eq!(
            remote_provider_exists, 1,
            "remote config should be imported"
        );

        let (request_logs, rollups): (i64, i64) = {
            let conn = crate::database::lock_conn!(local_db.conn);
            let request_logs =
                conn.query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |row| {
                    row.get(0)
                })?;
            let rollups =
                conn.query_row("SELECT COUNT(*) FROM usage_daily_rollups", [], |row| {
                    row.get(0)
                })?;
            (request_logs, rollups)
        };
        assert_eq!(request_logs, 1, "local request logs should be preserved");
        assert_eq!(rollups, 1, "local rollups should be preserved");
        Ok(())
    }

    #[test]
    #[serial]
    fn periodic_maintenance_runs_even_when_auto_backup_disabled() -> Result<(), AppError> {
        let old_test_home = std::env::var_os("LLM_USAGE_BAR_TEST_HOME");
        let test_home =
            std::env::temp_dir().join("llm-usage-bar-periodic-maintenance-backup-disabled-test");
        let _ = std::fs::remove_dir_all(&test_home);
        std::fs::create_dir_all(&test_home).expect("create test home");
        std::env::set_var("LLM_USAGE_BAR_TEST_HOME", &test_home);

        let settings = AppSettings {
            backup_interval_hours: Some(0),
            ..AppSettings::default()
        };
        update_settings(settings).expect("disable auto backup");

        let db = Database::memory()?;
        let now = chrono::Utc::now().timestamp();
        let old_ts = now - 40 * 86400;

        {
            let conn = crate::database::lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model,
                    input_tokens, output_tokens, total_cost_usd,
                    latency_ms, status_code, created_at
                ) VALUES ('old-req', 'p1', 'claude', 'claude-3', 100, 50, '0.01', 100, 200, ?1)",
                [old_ts],
            )?;
        }

        db.periodic_backup_if_needed()?;

        let (remaining_request_logs, rollups): (i64, i64) = {
            let conn = crate::database::lock_conn!(db.conn);
            let remaining_request_logs =
                conn.query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |row| {
                    row.get(0)
                })?;
            let rollups =
                conn.query_row("SELECT COUNT(*) FROM usage_daily_rollups", [], |row| {
                    row.get(0)
                })?;
            (remaining_request_logs, rollups)
        };

        assert_eq!(
            remaining_request_logs, 0,
            "old request logs should still be pruned when auto backup is disabled"
        );
        assert_eq!(rollups, 1, "old request logs should be rolled up");

        match old_test_home {
            Some(value) => std::env::set_var("LLM_USAGE_BAR_TEST_HOME", value),
            None => std::env::remove_var("LLM_USAGE_BAR_TEST_HOME"),
        }

        Ok(())
    }
}
