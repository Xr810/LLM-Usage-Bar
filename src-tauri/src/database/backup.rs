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

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct ProtectedBindingState {
    binding_id: String,
    agent_module_id: String,
    provider_id: String,
    binding_enabled: bool,
    fingerprint: Option<Vec<u8>>,
    credential_slot: Option<String>,
    credential_version: i64,
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
    provider_id: String,
    fingerprint: Option<Vec<u8>>,
    credential_slot: Option<String>,
    credential_version: i64,
    last_test_at: Option<i64>,
    last_test_status: Option<String>,
    last_test_error_code: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl ProtectedProviderCredentialState {
    fn is_pristine_placeholder(&self) -> bool {
        self.fingerprint.is_none()
            && self.credential_slot.is_none()
            && self.credential_version == 0
            && self.last_test_at.is_none()
            && self.last_test_status.is_none()
            && self.last_test_error_code.is_none()
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct ProviderCredentialJournalState {
    operation_id: String,
    provider_id: String,
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
    "stream_check_logs",
    "provider_health",
    "proxy_live_backup",
    "usage_daily_rollups",
    "agent_credential_operations",
    "provider_credential_operations",
];

/// Tables whose local data is preserved (restored from local snapshot) during WebDAV import.
/// Excludes ephemeral tables like provider_health that can safely rebuild at runtime.
const SYNC_PRESERVE_TABLES: &[&str] = &[
    "proxy_request_logs",
    "stream_check_logs",
    "proxy_live_backup",
    "usage_daily_rollups",
];

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
                provider_enabled: row.get(7)?,
                billing_kind: row.get(8)?,
                token_sources: row.get(9)?,
                route_app_type: row.get(10)?,
                route_config: row.get(11)?,
                quota_config: row.get(12)?,
                legacy_app_type: row.get(13)?,
                legacy_provider_id: row.get(14)?,
                legacy_settings_config: row.get(15)?,
                legacy_meta: row.get(16)?,
                agent_archived_at: row.get(17)?,
            })
        })?;

        rows.collect::<Result<BTreeSet<_>, _>>()
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
        if !Self::table_exists(conn, "provider_api_credentials")? {
            return Ok(BTreeSet::new());
        }
        let mut statement = conn.prepare(
            "SELECT provider_id, api_key_fingerprint, credential_slot,
                    credential_version, last_test_at, last_test_status,
                    last_test_error_code, created_at, updated_at
             FROM provider_api_credentials ORDER BY provider_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(ProtectedProviderCredentialState {
                provider_id: row.get(0)?,
                fingerprint: row.get(1)?,
                credential_slot: row.get(2)?,
                credential_version: row.get(3)?,
                last_test_at: row.get(4)?,
                last_test_status: row.get(5)?,
                last_test_error_code: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
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
            "SELECT operation_id, provider_id, generation, operation_kind, status,
                    staging_slot, previous_slot, created_at, updated_at
             FROM provider_credential_operations ORDER BY operation_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(ProviderCredentialJournalState {
                operation_id: row.get(0)?,
                provider_id: row.get(1)?,
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
        if current_provider_credentials
            .iter()
            .filter(|state| !state.is_pristine_placeholder())
            .any(|state| !incoming_provider_credentials.contains(state))
        {
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
                "UPDATE provider_api_credentials
                 SET api_key_fingerprint = ?2, credential_slot = ?3,
                     credential_version = ?4, last_test_at = ?5,
                     last_test_status = ?6, last_test_error_code = ?7,
                     created_at = ?8, updated_at = ?9
                 WHERE provider_id = ?1",
                rusqlite::params![
                    state.provider_id,
                    state.fingerprint,
                    state.credential_slot,
                    state.credential_version,
                    state.last_test_at,
                    state.last_test_status,
                    state.last_test_error_code,
                    state.created_at,
                    state.updated_at,
                ],
            ) {
                Ok(1) => {}
                Ok(_) | Err(_) => return Err(Self::credential_conflict()),
            }
        }
        Self::preserve_system_provider_seed_marker(local, incoming)?;
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

    /// 导出为 SQLite 兼容的 SQL 文本（内存字符串，完整导出）
    pub fn export_sql_string(&self) -> Result<String, AppError> {
        let snapshot = self.snapshot_to_memory()?;
        Self::dump_sql(&snapshot, &[])
    }

    /// Export SQL for sync (WebDAV), skipping local-only tables' data
    pub fn export_sql_string_for_sync(&self) -> Result<String, AppError> {
        let snapshot = self.snapshot_to_memory()?;
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

        // 补齐缺失表/索引并进行基础校验
        Self::create_tables_on_conn(&temp_conn)?;
        if Self::get_user_version(&temp_conn)? == crate::database::SCHEMA_VERSION {
            crate::usage::system_provider_migration::reconcile_system_provider_catalog(&temp_conn)?;
        }
        Self::apply_schema_migrations_on_conn(&temp_conn)?;
        crate::usage::system_provider_migration::reconcile_system_provider_catalog(&temp_conn)?;
        crate::usage::system_provider_migration::validate_schema_v17_complete(&temp_conn)?;
        Self::validate_basic_state(&temp_conn)?;
        Self::validate_import_schema_allowlist(&temp_conn)?;
        if let Some(local_snapshot) = local_snapshot.as_ref() {
            Self::restore_tables(local_snapshot, &temp_conn, preserve_tables)?;
            Self::preserve_local_credentials_for_sync(local_snapshot, &temp_conn)?;
        }

        // 使用 Backup 将临时库原子写回主库
        {
            let mut main_conn = lock_conn!(self.conn);
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
        match self.cleanup_old_stream_check_logs(7) {
            Ok(deleted) => {
                reclaimed_rows += deleted;
            }
            Err(e) => {
                log::warn!("Periodic stream_check_logs cleanup failed: {e}");
            }
        }
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
            let conn = lock_conn!(self.conn);
            let mut dest_conn =
                Connection::open(&backup_path).map_err(|e| AppError::Database(e.to_string()))?;
            let backup = Backup::new(&conn, &mut dest_conn)
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
        let mcp_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM mcp_servers", [], |row| row.get(0))
            .map_err(|e| AppError::Database(e.to_string()))?;

        if provider_count == 0 && mcp_count == 0 {
            return Err(AppError::Config(
                "导入的 SQL 未包含有效的供应商或 MCP 数据".to_string(),
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
            "{LLM_USAGE_BAR_SQL_EXPORT_HEADER}\n-- 生成时间: {timestamp}\n-- user_version: {user_version}\n"
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
        Self::create_tables_on_conn(&staged_conn)?;
        if Self::get_user_version(&staged_conn)? == crate::database::SCHEMA_VERSION {
            crate::usage::system_provider_migration::reconcile_system_provider_catalog(
                &staged_conn,
            )?;
        }
        Self::apply_schema_migrations_on_conn(&staged_conn)?;
        crate::usage::system_provider_migration::reconcile_system_provider_catalog(&staged_conn)?;
        crate::usage::system_provider_migration::validate_schema_v17_complete(&staged_conn)?;
        Self::validate_import_schema_allowlist(&staged_conn)?;

        // Step 3: Preserve device-local credential lifecycle state, then restore.
        {
            let mut main_conn = lock_conn!(self.conn);
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
    use super::{Database, LEGACY_CC_SWITCH_SQL_EXPORT_HEADER, LLM_USAGE_BAR_SQL_EXPORT_HEADER};
    use crate::error::AppError;
    use crate::product_identity::DATABASE_FILE;
    use crate::settings::{update_settings, AppSettings};
    use rusqlite::params;
    use serial_test::serial;
    use std::ffi::OsString;

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

    fn set_provider_credential_state(
        db: &Database,
        provider_id: &str,
        fingerprint_byte: u8,
        slot: &str,
        version: i64,
    ) -> Result<(), AppError> {
        let conn = crate::database::lock_conn!(db.conn);
        conn.execute(
            "UPDATE provider_api_credentials
             SET api_key_fingerprint = ?2, credential_slot = ?3,
                 credential_version = ?4, last_test_at = 91,
                 last_test_status = 'success', last_test_error_code = NULL,
                 updated_at = 92
             WHERE provider_id = ?1",
            params![provider_id, vec![fingerprint_byte; 32], slot, version],
        )?;
        Ok(())
    }

    fn provider_credential_tuple(
        db: &Database,
        provider_id: &str,
    ) -> Result<(Vec<u8>, String, i64, Option<i64>, Option<String>), AppError> {
        let conn = crate::database::lock_conn!(db.conn);
        conn.query_row(
            "SELECT api_key_fingerprint, credential_slot, credential_version,
                    last_test_at, last_test_status
             FROM provider_api_credentials WHERE provider_id = ?1",
            [provider_id],
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

    #[test]
    fn ordinary_import_rejects_removing_local_provider_credential_metadata() -> Result<(), AppError>
    {
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

        let error = local
            .import_sql_string(&incoming.export_sql_string()?)
            .expect_err("ordinary import must not orphan a local Provider credential slot");
        assert_eq!(error.to_string(), "credential_conflict");
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
        db.conn.lock().unwrap().execute(
            "INSERT INTO provider_credential_operations (
                 operation_id, provider_id, generation, operation_kind,
                 status, staging_slot, previous_slot, created_at, updated_at
             ) VALUES (
                 'provider-journal-secret-sentinel', 'system-openrouter-api', 1,
                 'set', 'pending', 'provider/staging/sentinel', NULL, 1, 1
             )",
            [],
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
    fn sql_import_rejects_changing_a_local_protected_credential_tuple() -> Result<(), AppError> {
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

        let error = local
            .import_sql_string(&incoming_sql)
            .expect_err("import must preserve the complete local credential tuple");
        assert_eq!(error.to_string(), "credential_conflict");
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
    fn sql_import_rejects_a_remote_only_credential_journal_row() -> Result<(), AppError> {
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

        let error = local
            .import_sql_string(&incoming_sql)
            .expect_err("a remote deletion intent must never execute on this device");
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
    fn ordinary_import_rejects_downgrading_ever_bound_tombstone() -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_custom_agent(&local, "formerly-bound-agent", true)?;
        let incoming = Database::memory()?;
        insert_custom_agent(&incoming, "formerly-bound-agent", false)?;

        let error = local
            .import_sql_string(&incoming.export_sql_string()?)
            .expect_err("ordinary import must not make a formerly bound Agent hard-deletable");

        assert_eq!(error.to_string(), "credential_conflict");
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
            protected_tuple(&local, "protected-binding")?,
            (vec![0x5a_u8; 32], "restored-missing-slot".to_string(), 1)
        );
        Ok(())
    }

    #[test]
    fn sql_import_accepts_an_exact_superset_of_local_protected_state() -> Result<(), AppError> {
        let local = Database::memory()?;
        insert_protected_credential_state(&local, "protected-binding", "local-slot", 1, true)?;

        let incoming = Database::memory()?;
        insert_protected_credential_state(&incoming, "protected-binding", "local-slot", 1, true)?;
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
            protected_tuple(&local, "remote-protected-binding")?,
            (vec![0x6b_u8; 32], "remote-slot".to_string(), 1)
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
            conn.execute(
                "INSERT INTO stream_check_logs (
                    provider_id, provider_name, app_type, status, success, message,
                    response_time_ms, http_status, model_used, retry_count, tested_at
                ) VALUES ('local-provider', 'Local Provider', 'claude', 'operational', 1, 'ok', 42, 200, 'claude-3', 0, 1000)",
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

        let (request_logs, rollups, stream_logs): (i64, i64, i64) = {
            let conn = crate::database::lock_conn!(local_db.conn);
            let request_logs =
                conn.query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |row| {
                    row.get(0)
                })?;
            let rollups =
                conn.query_row("SELECT COUNT(*) FROM usage_daily_rollups", [], |row| {
                    row.get(0)
                })?;
            let stream_logs =
                conn.query_row("SELECT COUNT(*) FROM stream_check_logs", [], |row| {
                    row.get(0)
                })?;
            (request_logs, rollups, stream_logs)
        };
        assert_eq!(request_logs, 1, "local request logs should be preserved");
        assert_eq!(rollups, 1, "local rollups should be preserved");
        assert_eq!(
            stream_logs, 1,
            "local stream check logs should be preserved"
        );

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
        let old_stream_ts = now - 8 * 86400;

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
            conn.execute(
                "INSERT INTO stream_check_logs (
                    provider_id, provider_name, app_type, status, success, message,
                    response_time_ms, http_status, model_used, retry_count, tested_at
                ) VALUES ('p1', 'Provider 1', 'claude', 'operational', 1, 'ok', 42, 200, 'claude-3', 0, ?1)",
                [old_stream_ts],
            )?;
        }

        db.periodic_backup_if_needed()?;

        let (remaining_request_logs, stream_logs, rollups): (i64, i64, i64) = {
            let conn = crate::database::lock_conn!(db.conn);
            let remaining_request_logs =
                conn.query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |row| {
                    row.get(0)
                })?;
            let stream_logs =
                conn.query_row("SELECT COUNT(*) FROM stream_check_logs", [], |row| {
                    row.get(0)
                })?;
            let rollups =
                conn.query_row("SELECT COUNT(*) FROM usage_daily_rollups", [], |row| {
                    row.get(0)
                })?;
            (remaining_request_logs, stream_logs, rollups)
        };

        assert_eq!(
            remaining_request_logs, 0,
            "old request logs should still be pruned when auto backup is disabled"
        );
        assert_eq!(
            stream_logs, 0,
            "old stream check logs should still be pruned when auto backup is disabled"
        );
        assert_eq!(rollups, 1, "old request logs should be rolled up");

        match old_test_home {
            Some(value) => std::env::set_var("LLM_USAGE_BAR_TEST_HOME", value),
            None => std::env::remove_var("LLM_USAGE_BAR_TEST_HOME"),
        }

        Ok(())
    }
}
