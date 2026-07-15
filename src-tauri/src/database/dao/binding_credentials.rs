use super::agent_provider_bindings::{
    binding_auth_mode, binding_record_from_row, BindingAuthMode, BindingRecord,
    BINDING_RECORD_QUERY,
};
use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::provider::Provider;
use crate::usage::domain::{AgentProviderBindingView, BindingCredentialStatus};
use crate::usage::system_providers::is_fixed_api_preset;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;

fn public_error(code: &'static str) -> AppError {
    AppError::Message(code.to_string())
}

fn now_timestamp() -> Result<i64, AppError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|_| public_error("credential_unavailable"))
}

fn is_constraint_error(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(sqlite, _)
            if sqlite.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CredentialMutationKind {
    Set,
    Replace,
    Clear,
    Delete,
}

impl CredentialMutationKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Set => "set",
            Self::Replace => "replace",
            Self::Clear => "clear",
            Self::Delete => "delete",
        }
    }
}

pub(crate) struct CredentialBindingSnapshot {
    pub(crate) id: String,
    pub(crate) agent_module_id: String,
    pub(crate) provider_id: String,
    pub(crate) enabled: bool,
    pub(crate) fingerprint: Option<Vec<u8>>,
    pub(crate) credential_slot: Option<String>,
    pub(crate) credential_version: u64,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) provider_enabled: bool,
    pub(crate) route_app_type: Option<String>,
    pub(crate) route_config: Option<Value>,
    pub(crate) quota_config: Option<Value>,
    pub(crate) agent_archived_at: Option<i64>,
    pub(crate) auth_mode: BindingAuthMode,
    pub(crate) provider_name: String,
    pub(crate) product_group_id: String,
    pub(crate) legacy_migration_linked: bool,
    pub(crate) legacy_provider_id: Option<String>,
    pub(crate) legacy_provider: Option<Provider>,
    pub(crate) route_protocol: Option<String>,
    pub(crate) system_preset_key: Option<String>,
    pub(crate) provider_fingerprint: Option<Vec<u8>>,
    pub(crate) provider_credential_slot: Option<String>,
    pub(crate) provider_credential_version: u64,
}

impl CredentialBindingSnapshot {
    pub(crate) fn is_fixed_system_api(&self) -> bool {
        is_fixed_api_preset(self.system_preset_key.as_deref())
    }

    pub(crate) fn into_view(
        self,
        credential_status: BindingCredentialStatus,
        provider_credential_status: BindingCredentialStatus,
    ) -> AgentProviderBindingView {
        let can_clear_credential = self.fingerprint.is_some() && self.credential_slot.is_some();
        let is_fixed_api = self.is_fixed_system_api();
        let effective_enabled = if is_fixed_api {
            self.enabled
                && self.provider_enabled
                && self.agent_archived_at.is_none()
                && self.auth_mode == BindingAuthMode::DirectApiKey
                && credential_status == BindingCredentialStatus::Configured
                && provider_credential_status == BindingCredentialStatus::Configured
        } else {
            self.enabled
                && self.provider_enabled
                && self.agent_archived_at.is_none()
                && self.auth_mode == BindingAuthMode::DirectApiKey
                && credential_status == BindingCredentialStatus::Configured
        };
        AgentProviderBindingView {
            id: self.id,
            agent_module_id: self.agent_module_id,
            provider_id: self.provider_id,
            enabled: self.enabled,
            effective_enabled,
            credential_status,
            can_clear_credential,
            credential_version: self.credential_version,
            route_protocol: self.route_protocol,
            local_credential_status: if is_fixed_api {
                credential_status
            } else {
                BindingCredentialStatus::NotRequired
            },
            provider_credential_status: if is_fixed_api {
                provider_credential_status
            } else {
                BindingCredentialStatus::NotRequired
            },
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

pub(crate) struct CredentialOperationReservation {
    pub(crate) operation_id: String,
    pub(crate) binding_id: String,
    pub(crate) kind: CredentialMutationKind,
    pub(crate) expected_version: u64,
    pub(crate) generation: u64,
    pub(crate) staging_slot: Option<String>,
    pub(crate) previous_slot: Option<String>,
    pub(crate) previous_fingerprint: Option<Vec<u8>>,
}

pub(crate) struct CredentialJournalEntry {
    pub(crate) operation_id: String,
    pub(crate) binding_id: String,
    pub(crate) kind: CredentialMutationKind,
    pub(crate) status: String,
    pub(crate) generation: u64,
    pub(crate) staging_slot: Option<String>,
    pub(crate) previous_slot: Option<String>,
}

pub(crate) struct CredentialReconcileState {
    pub(crate) credential_version: u64,
    pub(crate) credential_slot: Option<String>,
    pub(crate) has_fingerprint: bool,
    pub(crate) enabled: bool,
}

fn parse_mutation_kind(raw: &str) -> Result<CredentialMutationKind, AppError> {
    match raw {
        "set" => Ok(CredentialMutationKind::Set),
        "replace" => Ok(CredentialMutationKind::Replace),
        "clear" => Ok(CredentialMutationKind::Clear),
        "delete" => Ok(CredentialMutationKind::Delete),
        _ => Err(public_error("credential_unavailable")),
    }
}

impl Database {
    fn credential_binding_snapshot_from_record(
        record: BindingRecord,
    ) -> Result<CredentialBindingSnapshot, AppError> {
        let credential_version = u64::try_from(record.credential_version)
            .map_err(|_| public_error("credential_unavailable"))?;
        let provider_credential_version = u64::try_from(record.provider_credential_version)
            .map_err(|_| public_error("credential_unavailable"))?;
        let auth_mode = binding_auth_mode(&record);
        Ok(CredentialBindingSnapshot {
            id: record.id,
            agent_module_id: record.agent_module_id,
            provider_id: record.provider_id,
            enabled: record.enabled,
            fingerprint: record.fingerprint,
            credential_slot: record.credential_slot,
            credential_version,
            created_at: record.created_at,
            updated_at: record.updated_at,
            provider_enabled: record.provider_enabled,
            route_app_type: record.route_app_type,
            route_config: record.route_config,
            quota_config: record.quota_config,
            agent_archived_at: record.agent_archived_at,
            auth_mode,
            provider_name: record.provider_name,
            product_group_id: record.product_group_id,
            legacy_migration_linked: record.legacy_migration_linked,
            legacy_provider_id: record.legacy_provider_id,
            legacy_provider: record.legacy_provider,
            route_protocol: record.route_protocol,
            system_preset_key: record.system_preset_key,
            provider_fingerprint: record.provider_fingerprint,
            provider_credential_slot: record.provider_credential_slot,
            provider_credential_version,
        })
    }

    fn credential_binding_snapshots_on_conn(
        conn: &rusqlite::Connection,
        predicate: &str,
        parameter: Option<&str>,
    ) -> Result<Vec<CredentialBindingSnapshot>, AppError> {
        let sql = format!(
            "{BINDING_RECORD_QUERY} {predicate}
             ORDER BY binding.agent_module_id, binding.provider_id, binding.id"
        );
        let mut statement = conn.prepare(&sql)?;
        let rows = match parameter {
            Some(parameter) => statement
                .query_map([parameter], binding_record_from_row)?
                .collect::<Result<Vec<_>, _>>()?,
            None => statement
                .query_map([], binding_record_from_row)?
                .collect::<Result<Vec<_>, _>>()?,
        };
        rows.into_iter()
            .map(Self::credential_binding_snapshot_from_record)
            .collect()
    }

    pub(crate) fn credential_binding_snapshot(
        &self,
        binding_id: &str,
    ) -> Result<Option<CredentialBindingSnapshot>, AppError> {
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let snapshot = Self::credential_binding_snapshots_on_conn(
            &transaction,
            "WHERE binding.id = ?1",
            Some(binding_id),
        )?
        .into_iter()
        .next();
        transaction.commit()?;
        Ok(snapshot)
    }

    pub(crate) fn credential_binding_snapshots(
        &self,
        agent_module_id: Option<&str>,
    ) -> Result<Vec<CredentialBindingSnapshot>, AppError> {
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let snapshots = match agent_module_id {
            Some(agent_module_id) => Self::credential_binding_snapshots_on_conn(
                &transaction,
                "WHERE binding.agent_module_id = ?1",
                Some(agent_module_id),
            ),
            None => Self::credential_binding_snapshots_on_conn(&transaction, "", None),
        }?;
        transaction.commit()?;
        Ok(snapshots)
    }

    pub(crate) fn credential_binding_fail_closed_view(
        &self,
        binding_id: &str,
    ) -> Result<AgentProviderBindingView, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT binding.id, binding.agent_module_id, binding.provider_id,
                    binding.enabled, binding.api_key_fingerprint,
                    binding.credential_slot, binding.credential_version,
                    binding.created_at, binding.updated_at,
                    binding.route_protocol, provider.system_preset_key
             FROM agent_provider_bindings binding
             JOIN usage_providers provider ON provider.id = binding.provider_id
             WHERE binding.id = ?1",
            [binding_id],
            |row| {
                let fingerprint = row.get::<_, Option<Vec<u8>>>(4)?;
                let credential_slot = row.get::<_, Option<String>>(5)?;
                let credential_version = row.get::<_, i64>(6)?;
                Ok(AgentProviderBindingView {
                    id: row.get(0)?,
                    agent_module_id: row.get(1)?,
                    provider_id: row.get(2)?,
                    enabled: row.get(3)?,
                    effective_enabled: false,
                    credential_status: BindingCredentialStatus::Unavailable,
                    can_clear_credential: fingerprint.is_some() && credential_slot.is_some(),
                    credential_version: u64::try_from(credential_version).map_err(|_| {
                        rusqlite::Error::IntegralValueOutOfRange(6, credential_version)
                    })?,
                    route_protocol: row.get(9)?,
                    local_credential_status: if is_fixed_api_preset(
                        row.get::<_, Option<String>>(10)?.as_deref(),
                    ) {
                        BindingCredentialStatus::Unavailable
                    } else {
                        BindingCredentialStatus::NotRequired
                    },
                    provider_credential_status: if is_fixed_api_preset(
                        row.get::<_, Option<String>>(10)?.as_deref(),
                    ) {
                        BindingCredentialStatus::Unavailable
                    } else {
                        BindingCredentialStatus::NotRequired
                    },
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            },
        )
        .optional()
        .map_err(AppError::from)?
        .ok_or_else(|| public_error("binding_not_found"))
    }

    pub(crate) fn credential_binding_by_fingerprint(
        &self,
        fingerprint: &[u8; 32],
    ) -> Result<Option<CredentialBindingSnapshot>, AppError> {
        let conn = lock_conn!(self.conn);
        let sql = format!(
            "{BINDING_RECORD_QUERY}
             WHERE binding.api_key_fingerprint = ?1
               AND binding.enabled = 1
               AND provider.enabled = 1
               AND agent.archived_at IS NULL"
        );
        let record = conn
            .query_row(&sql, [fingerprint.as_slice()], binding_record_from_row)
            .optional()?;
        record
            .map(Self::credential_binding_snapshot_from_record)
            .transpose()
    }

    pub(crate) fn credential_slot_is_active(&self, slot: &str) -> Result<bool, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM agent_provider_bindings WHERE credential_slot = ?1
                 UNION ALL
                 SELECT 1 FROM provider_api_credentials WHERE credential_slot = ?1
             )",
            [slot],
            |row| row.get(0),
        )
        .map_err(Into::into)
    }

    pub(crate) fn credential_reconcile_state(
        &self,
        binding_id: &str,
    ) -> Result<Option<CredentialReconcileState>, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT credential_version, credential_slot,
                    api_key_fingerprint IS NOT NULL, enabled
             FROM agent_provider_bindings WHERE id = ?1",
            [binding_id],
            |row| {
                let credential_version = row.get::<_, i64>(0)?;
                Ok(CredentialReconcileState {
                    credential_version: u64::try_from(credential_version).map_err(|_| {
                        rusqlite::Error::IntegralValueOutOfRange(0, credential_version)
                    })?,
                    credential_slot: row.get(1)?,
                    has_fingerprint: row.get(2)?,
                    enabled: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(AppError::from)
    }

    pub(crate) fn reserve_credential_operation(
        &self,
        binding_id: &str,
        expected_version: u64,
        kind: CredentialMutationKind,
        new_fingerprint: Option<&[u8; 32]>,
    ) -> Result<CredentialOperationReservation, AppError> {
        i64::try_from(expected_version).map_err(|_| public_error("credential_conflict"))?;
        let generation = expected_version
            .checked_add(1)
            .ok_or_else(|| public_error("credential_conflict"))?;
        let generation_i64 =
            i64::try_from(generation).map_err(|_| public_error("credential_conflict"))?;
        let now = now_timestamp()?;
        let operation_id = uuid::Uuid::new_v4().to_string();
        let staging_slot = matches!(
            kind,
            CredentialMutationKind::Set | CredentialMutationKind::Replace
        )
        .then(|| uuid::Uuid::new_v4().to_string());

        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let needs_direct_auth = matches!(
            kind,
            CredentialMutationKind::Set | CredentialMutationKind::Replace
        );
        let (current_version, current_fingerprint, current_slot) = if needs_direct_auth {
            let snapshot = Self::credential_binding_snapshots_on_conn(
                &transaction,
                "WHERE binding.id = ?1",
                Some(binding_id),
            )?
            .into_iter()
            .next()
            .ok_or_else(|| public_error("binding_not_found"))?;
            if snapshot.agent_archived_at.is_some()
                || (!snapshot.provider_enabled && !snapshot.is_fixed_system_api())
            {
                return Err(public_error("invalid_binding"));
            }
            if snapshot.auth_mode != BindingAuthMode::DirectApiKey {
                return Err(public_error("unsupported_auth"));
            }
            (
                snapshot.credential_version,
                snapshot.fingerprint,
                snapshot.credential_slot,
            )
        } else {
            let state = transaction
                .query_row(
                    "SELECT credential_version, api_key_fingerprint, credential_slot
                     FROM agent_provider_bindings WHERE id = ?1",
                    [binding_id],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, Option<Vec<u8>>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ))
                    },
                )
                .optional()?
                .ok_or_else(|| public_error("binding_not_found"))?;
            (
                u64::try_from(state.0).map_err(|_| public_error("credential_unavailable"))?,
                state.1,
                state.2,
            )
        };
        if current_version != expected_version {
            return Err(public_error("credential_conflict"));
        }
        let unfinished_delete: bool = transaction.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM agent_credential_operations
                 WHERE binding_id = ?1 AND operation_kind = 'delete'
             )",
            [binding_id],
            |row| row.get(0),
        )?;
        if unfinished_delete {
            return Err(public_error("credential_conflict"));
        }
        match kind {
            CredentialMutationKind::Set => {
                if current_fingerprint.is_some() || current_slot.is_some() {
                    return Err(public_error("credential_conflict"));
                }
            }
            CredentialMutationKind::Replace => {
                let Some(current) = current_fingerprint.as_deref() else {
                    return Err(public_error("credential_required"));
                };
                if current_slot.is_none() {
                    return Err(public_error("credential_required"));
                }
                let Some(new_fingerprint) = new_fingerprint else {
                    return Err(public_error("credential_required"));
                };
                if current.len() == new_fingerprint.len()
                    && bool::from(current.ct_eq(new_fingerprint.as_slice()))
                {
                    return Err(public_error("credential_conflict"));
                }
            }
            CredentialMutationKind::Clear => {
                if current_fingerprint.is_none() || current_slot.is_none() {
                    return Err(public_error("credential_required"));
                }
            }
            CredentialMutationKind::Delete => {}
        }

        let insert_result = transaction.execute(
            "INSERT INTO agent_credential_operations (
                 operation_id, binding_id, generation, operation_kind, status,
                 staging_slot, previous_slot, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?6, ?7, ?7)",
            params![
                operation_id,
                binding_id,
                generation_i64,
                kind.as_str(),
                staging_slot,
                current_slot,
                now
            ],
        );
        if let Err(error) = insert_result {
            return if is_constraint_error(&error) {
                Err(public_error("credential_conflict"))
            } else {
                Err(error.into())
            };
        }
        transaction.commit()?;
        Ok(CredentialOperationReservation {
            operation_id,
            binding_id: binding_id.to_string(),
            kind,
            expected_version,
            generation,
            staging_slot,
            previous_slot: current_slot,
            previous_fingerprint: current_fingerprint,
        })
    }

    pub(crate) fn publish_credential_operation(
        &self,
        reservation: &CredentialOperationReservation,
        new_fingerprint: Option<&[u8; 32]>,
    ) -> Result<(), AppError> {
        let expected = i64::try_from(reservation.expected_version)
            .map_err(|_| public_error("credential_conflict"))?;
        let generation = i64::try_from(reservation.generation)
            .map_err(|_| public_error("credential_conflict"))?;
        let now = now_timestamp()?;
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if matches!(
            reservation.kind,
            CredentialMutationKind::Set | CredentialMutationKind::Replace
        ) {
            let snapshot = Self::credential_binding_snapshots_on_conn(
                &transaction,
                "WHERE binding.id = ?1",
                Some(&reservation.binding_id),
            )?
            .into_iter()
            .next()
            .ok_or_else(|| public_error("credential_conflict"))?;
            if snapshot.agent_archived_at.is_some()
                || (!snapshot.provider_enabled && !snapshot.is_fixed_system_api())
                || snapshot.auth_mode != BindingAuthMode::DirectApiKey
            {
                return Err(public_error("credential_conflict"));
            }
        }
        let updated = match reservation.kind {
            CredentialMutationKind::Set | CredentialMutationKind::Replace => {
                let fingerprint =
                    new_fingerprint.ok_or_else(|| public_error("credential_required"))?;
                let staging_slot = reservation
                    .staging_slot
                    .as_deref()
                    .ok_or_else(|| public_error("credential_unavailable"))?;
                transaction.execute(
                    "UPDATE agent_provider_bindings
                     SET api_key_fingerprint = ?2, credential_slot = ?3,
                         credential_version = ?4, updated_at = ?5
                     WHERE id = ?1 AND credential_version = ?6
                       AND credential_slot IS ?7 AND api_key_fingerprint IS ?8
                       AND EXISTS (
                           SELECT 1 FROM agent_credential_operations
                           WHERE operation_id = ?9 AND binding_id = ?1
                             AND generation = ?4 AND status = 'pending'
                             AND staging_slot = ?3
                       )",
                    params![
                        reservation.binding_id,
                        fingerprint.as_slice(),
                        staging_slot,
                        generation,
                        now,
                        expected,
                        reservation.previous_slot,
                        reservation.previous_fingerprint,
                        reservation.operation_id
                    ],
                )
            }
            CredentialMutationKind::Clear | CredentialMutationKind::Delete => transaction.execute(
                "UPDATE agent_provider_bindings
                 SET api_key_fingerprint = NULL, credential_slot = NULL,
                     credential_version = ?2, enabled = 0, updated_at = ?3
                 WHERE id = ?1 AND credential_version = ?4
                   AND credential_slot IS ?5 AND api_key_fingerprint IS ?6
                   AND EXISTS (
                       SELECT 1 FROM agent_credential_operations
                       WHERE operation_id = ?7 AND binding_id = ?1
                         AND generation = ?2 AND status = 'pending'
                   )",
                params![
                    reservation.binding_id,
                    generation,
                    now,
                    expected,
                    reservation.previous_slot,
                    reservation.previous_fingerprint,
                    reservation.operation_id
                ],
            ),
        };
        let updated = match updated {
            Ok(updated) => updated,
            Err(error) if is_constraint_error(&error) => {
                return Err(public_error("credential_conflict"));
            }
            Err(error) => return Err(error.into()),
        };
        if updated != 1 {
            return Err(public_error("credential_conflict"));
        }
        let status = if reservation.previous_slot.is_some() {
            "cleanup"
        } else {
            "committed"
        };
        if transaction.execute(
            "UPDATE agent_credential_operations
             SET status = ?2, updated_at = ?3
             WHERE operation_id = ?1 AND status = 'pending'",
            params![reservation.operation_id, status, now],
        )? != 1
        {
            return Err(public_error("credential_conflict"));
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn claim_pending_staging_cleanup(
        &self,
        reservation: &CredentialOperationReservation,
    ) -> Result<bool, AppError> {
        let Some(staging_slot) = reservation.staging_slot.as_deref() else {
            return Ok(false);
        };
        let generation = i64::try_from(reservation.generation)
            .map_err(|_| public_error("credential_conflict"))?;
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let claimed = transaction.execute(
            "UPDATE agent_credential_operations
             SET status = 'cleanup', previous_slot = staging_slot, updated_at = ?4
             WHERE operation_id = ?1 AND binding_id = ?2
               AND generation = ?3 AND status = 'pending'
               AND NOT EXISTS (
                   SELECT 1 FROM agent_provider_bindings
                   WHERE credential_slot = ?5
               )
               AND NOT EXISTS (
                   SELECT 1 FROM provider_api_credentials
                   WHERE credential_slot = ?5
               )",
            params![
                reservation.operation_id,
                reservation.binding_id,
                generation,
                now_timestamp()?,
                staging_slot
            ],
        )?;
        transaction.commit()?;
        Ok(claimed == 1)
    }

    pub(crate) fn finish_claimed_staging_cleanup(
        &self,
        reservation: &CredentialOperationReservation,
    ) -> Result<(), AppError> {
        let Some(staging_slot) = reservation.staging_slot.as_deref() else {
            return Err(public_error("credential_conflict"));
        };
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if transaction.execute(
            "DELETE FROM agent_credential_operations
             WHERE operation_id = ?1 AND status = 'cleanup'
               AND previous_slot = ?2
               AND NOT EXISTS (
                   SELECT 1 FROM agent_provider_bindings
                   WHERE credential_slot = ?2
               )
               AND NOT EXISTS (
                   SELECT 1 FROM provider_api_credentials
                   WHERE credential_slot = ?2
               )",
            params![reservation.operation_id, staging_slot],
        )? != 1
        {
            return Err(public_error("credential_conflict"));
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn finish_credential_operation(
        &self,
        reservation: &CredentialOperationReservation,
    ) -> Result<(), AppError> {
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if reservation.kind == CredentialMutationKind::Delete {
            let generation = i64::try_from(reservation.generation)
                .map_err(|_| public_error("credential_conflict"))?;
            let ready: bool = transaction.query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM agent_provider_bindings
                     WHERE id = ?1 AND credential_version = ?2
                       AND credential_slot IS NULL
                       AND api_key_fingerprint IS NULL AND enabled = 0
                 )",
                params![reservation.binding_id, generation],
                |row| row.get(0),
            )?;
            if !ready {
                return Err(public_error("credential_conflict"));
            }
        }
        let previous_is_active: bool = match reservation.previous_slot.as_deref() {
            Some(previous_slot) => transaction.query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM agent_provider_bindings
                     WHERE credential_slot = ?1
                     UNION ALL
                     SELECT 1 FROM provider_api_credentials
                     WHERE credential_slot = ?1
                 )",
                [previous_slot],
                |row| row.get(0),
            )?,
            None => false,
        };
        if previous_is_active {
            return Err(public_error("credential_conflict"));
        }
        let deleted = transaction.execute(
            "DELETE FROM agent_credential_operations
             WHERE operation_id = ?1 AND status IN ('committed', 'cleanup')",
            [&reservation.operation_id],
        )?;
        if deleted != 1 {
            return Err(public_error("credential_conflict"));
        }
        if reservation.kind == CredentialMutationKind::Delete
            && transaction.execute(
                "DELETE FROM agent_provider_bindings WHERE id = ?1",
                [&reservation.binding_id],
            )? != 1
        {
            return Err(public_error("credential_conflict"));
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn credential_journal_entries(
        &self,
    ) -> Result<Vec<CredentialJournalEntry>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut statement = conn.prepare(
            "SELECT operation_id, binding_id, operation_kind, status, generation,
                    staging_slot, previous_slot
             FROM agent_credential_operations
             ORDER BY binding_id, generation, operation_id",
        )?;
        let entries = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })?
            .map(|row| {
                let (
                    operation_id,
                    binding_id,
                    kind,
                    status,
                    generation,
                    staging_slot,
                    previous_slot,
                ) = row?;
                Ok(CredentialJournalEntry {
                    operation_id,
                    binding_id,
                    kind: parse_mutation_kind(&kind)?,
                    status,
                    generation: u64::try_from(generation)
                        .map_err(|_| public_error("credential_unavailable"))?,
                    staging_slot,
                    previous_slot,
                })
            })
            .collect();
        entries
    }

    pub(crate) fn promote_pending_credential_operation(
        &self,
        operation_id: &str,
        status: &str,
    ) -> Result<(), AppError> {
        if !matches!(status, "committed" | "cleanup") {
            return Err(public_error("credential_unavailable"));
        }
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "UPDATE agent_credential_operations SET status = ?2, updated_at = ?3
             WHERE operation_id = ?1 AND status = 'pending'",
            params![operation_id, status, now_timestamp()?],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn delete_pending_journal_entry(
        &self,
        operation_id: &str,
        binding_id: &str,
        generation: u64,
    ) -> Result<(), AppError> {
        let generation =
            i64::try_from(generation).map_err(|_| public_error("credential_conflict"))?;
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "DELETE FROM agent_credential_operations
             WHERE operation_id = ?1 AND binding_id = ?2
               AND generation = ?3 AND status = 'pending'
               AND NOT EXISTS (
                   SELECT 1 FROM agent_provider_bindings
                   WHERE id = ?2 AND credential_version = ?3
                     AND credential_slot IS NULL
                     AND api_key_fingerprint IS NULL AND enabled = 0
               )",
            params![operation_id, binding_id, generation],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn finish_journal_entry(
        &self,
        entry: &CredentialJournalEntry,
    ) -> Result<(), AppError> {
        let reservation = CredentialOperationReservation {
            operation_id: entry.operation_id.clone(),
            binding_id: entry.binding_id.clone(),
            kind: entry.kind,
            expected_version: entry.generation.saturating_sub(1),
            generation: entry.generation,
            staging_slot: entry.staging_slot.clone(),
            previous_slot: entry.previous_slot.clone(),
            previous_fingerprint: None,
        };
        self.finish_credential_operation(&reservation)
    }
}

#[cfg(test)]
mod tests {
    use super::{CredentialMutationKind, Database};
    use crate::provider::Provider;
    use crate::usage::domain::AgentProviderBindingInput;
    use rusqlite::params;
    use serde_json::json;

    fn insert_direct_provider(db: &Database, id: &str) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO usage_providers (
                 id, name, billing_kind, product_group_id, token_sources,
                 route_app_type, route_config, quota_config, enabled, needs_review,
                 created_at, updated_at
             ) VALUES (?1, ?1, 'metered', 'test', '[\"proxy\"]',
                       'claude', '{}', NULL, 1, 0, 10, 10)",
            [id],
        )
        .unwrap();
    }

    fn insert_custom_agent(db: &Database, id: &str) {
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO agent_modules (
                     id, name, sort_order, visible, is_fixed, archived_at,
                     created_at, updated_at
                 ) VALUES (?1, ?1, 100, 1, 0, NULL, 10, 10)",
                [id],
            )
            .unwrap();
    }

    fn direct_binding_for_agent(db: &Database, agent_module_id: &str, provider_id: &str) -> String {
        db.save_agent_provider_binding(&AgentProviderBindingInput {
            id: None,
            agent_module_id: agent_module_id.to_string(),
            provider_id: provider_id.to_string(),
            enabled: false,
        })
        .unwrap()
        .id
    }

    fn install_credential_metadata(
        db: &Database,
        binding_id: &str,
        fingerprint: &[u8; 32],
        slot: &str,
        version: u64,
        enabled: bool,
    ) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "UPDATE agent_provider_bindings
             SET api_key_fingerprint = ?2, credential_slot = ?3,
                 credential_version = ?4, enabled = ?5
             WHERE id = ?1",
            params![binding_id, fingerprint.as_slice(), slot, version, enabled],
        )
        .unwrap();
    }

    #[test]
    fn fingerprint_lookup_atomically_filters_disabled_provider_and_archived_agent() {
        let db = Database::memory().unwrap();
        insert_direct_provider(&db, "direct");
        insert_custom_agent(&db, "custom-agent");
        let binding_id = direct_binding_for_agent(&db, "custom-agent", "direct");
        let fingerprint = [7_u8; 32];
        install_credential_metadata(&db, &binding_id, &fingerprint, "slot", 1, false);

        assert!(db
            .credential_binding_by_fingerprint(&fingerprint)
            .unwrap()
            .is_none());

        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE agent_provider_bindings SET enabled = 1 WHERE id = ?1",
                [&binding_id],
            )
            .unwrap();
        }
        assert!(db
            .credential_binding_by_fingerprint(&fingerprint)
            .unwrap()
            .is_some());

        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE usage_providers SET enabled = 0 WHERE id = 'direct'",
                [],
            )
            .unwrap();
        }
        assert!(db
            .credential_binding_by_fingerprint(&fingerprint)
            .unwrap()
            .is_none());

        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE usage_providers SET enabled = 1 WHERE id = 'direct'",
                [],
            )
            .unwrap();
            conn.execute(
                "UPDATE agent_modules
                 SET archived_at = 20, visible = 0 WHERE id = 'custom-agent'",
                [],
            )
            .unwrap();
        }
        assert!(db
            .credential_binding_by_fingerprint(&fingerprint)
            .unwrap()
            .is_none());
    }

    #[test]
    fn fail_closed_view_preserves_cleanup_capability_until_metadata_is_cleared() {
        let db = Database::memory().unwrap();
        insert_direct_provider(&db, "direct");
        let binding_id = direct_binding_for_agent(&db, "codex", "direct");
        let fingerprint = [19_u8; 32];
        install_credential_metadata(&db, &binding_id, &fingerprint, "slot", 1, true);

        let unavailable = db.credential_binding_fail_closed_view(&binding_id).unwrap();
        assert!(unavailable.can_clear_credential);

        let reservation = db
            .reserve_credential_operation(&binding_id, 1, CredentialMutationKind::Clear, None)
            .unwrap();
        db.publish_credential_operation(&reservation, None).unwrap();

        let cleared = db.credential_binding_fail_closed_view(&binding_id).unwrap();
        assert!(!cleared.can_clear_credential);
    }

    #[test]
    fn fingerprint_lookup_freezes_provider_name_and_legacy_runtime_envelope() {
        let db = Database::memory().unwrap();
        let mut legacy = Provider::with_id(
            "legacy-route".to_string(),
            "Legacy route".to_string(),
            json!({"env": {"ANTHROPIC_AUTH_TOKEN": "old-secret"}}),
            Some("https://provider.example".to_string()),
        );
        legacy.category = Some("relay".to_string());
        legacy.icon = Some("anthropic".to_string());
        legacy.meta = Some(
            serde_json::from_value(json!({
                "apiFormat": "openai_chat",
                "costMultiplier": "1.5"
            }))
            .unwrap(),
        );
        db.save_provider("claude", &legacy).unwrap();
        insert_direct_provider(&db, "direct");
        insert_custom_agent(&db, "custom-agent");
        let binding_id = direct_binding_for_agent(&db, "custom-agent", "direct");
        let fingerprint = [17_u8; 32];
        install_credential_metadata(&db, &binding_id, &fingerprint, "slot", 1, true);
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE usage_providers
                 SET name = 'Frozen usage name',
                     product_group_id = 'frozen-group',
                     route_config = '{\"baseUrl\":\"https://upstream.example\"}',
                     quota_config = '{\"apiFormat\":\"openai_chat\"}',
                     legacy_app_type = 'claude', legacy_provider_id = 'legacy-route'
                 WHERE id = 'direct'",
                [],
            )
            .unwrap();

        let snapshot = db
            .credential_binding_by_fingerprint(&fingerprint)
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.provider_name, "Frozen usage name");
        assert_eq!(snapshot.product_group_id, "frozen-group");
        assert!(snapshot.legacy_migration_linked);
        assert_eq!(
            snapshot
                .quota_config
                .as_ref()
                .and_then(|value| value.get("apiFormat"))
                .and_then(|value| value.as_str()),
            Some("openai_chat")
        );
        let frozen_legacy = snapshot.legacy_provider.unwrap();
        assert_eq!(frozen_legacy.id, "legacy-route");
        assert_eq!(frozen_legacy.name, "Legacy route");
        assert_eq!(frozen_legacy.category.as_deref(), Some("relay"));
        assert_eq!(frozen_legacy.icon.as_deref(), Some("anthropic"));
        assert_eq!(
            frozen_legacy
                .meta
                .as_ref()
                .and_then(|meta| meta.api_format.as_deref()),
            Some("openai_chat")
        );

        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE usage_providers
                 SET name = 'Mutated later', product_group_id = 'mutated-group',
                     route_app_type = 'codex'
                 WHERE id = 'direct'",
                [],
            )
            .unwrap();
        assert_eq!(snapshot.provider_name, "Frozen usage name");
        assert_eq!(snapshot.product_group_id, "frozen-group");
        assert_eq!(snapshot.route_app_type.as_deref(), Some("claude"));
    }

    #[test]
    fn publish_set_revalidates_provider_and_auth_after_reservation() {
        for mutation in [
            "UPDATE usage_providers SET enabled = 0 WHERE id = 'direct'",
            "UPDATE usage_providers
             SET billing_kind = 'subscription', token_sources = '[\"session_log\"]',
                 route_app_type = NULL
             WHERE id = 'direct'",
            "UPDATE agent_modules
             SET archived_at = 20, visible = 0 WHERE id = 'custom-agent'",
        ] {
            let db = Database::memory().unwrap();
            insert_direct_provider(&db, "direct");
            insert_custom_agent(&db, "custom-agent");
            let binding_id = direct_binding_for_agent(&db, "custom-agent", "direct");
            let fingerprint = [9_u8; 32];
            let reservation = db
                .reserve_credential_operation(
                    &binding_id,
                    0,
                    CredentialMutationKind::Set,
                    Some(&fingerprint),
                )
                .unwrap();
            db.conn.lock().unwrap().execute(mutation, []).unwrap();

            assert_eq!(
                db.publish_credential_operation(&reservation, Some(&fingerprint))
                    .unwrap_err()
                    .to_string(),
                "credential_conflict"
            );
            let snapshot = db
                .credential_binding_snapshot(&binding_id)
                .unwrap()
                .unwrap();
            assert_eq!(snapshot.credential_version, 0);
            assert!(snapshot.fingerprint.is_none());
            assert!(snapshot.credential_slot.is_none());
        }
    }

    #[test]
    fn clear_remains_allowed_after_provider_auth_changes() {
        let db = Database::memory().unwrap();
        insert_direct_provider(&db, "direct");
        let binding_id = direct_binding_for_agent(&db, "codex", "direct");
        let fingerprint = [11_u8; 32];
        install_credential_metadata(&db, &binding_id, &fingerprint, "old-slot", 1, true);
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE usage_providers
                 SET billing_kind = 'subscription', token_sources = '[\"session_log\"]',
                     route_app_type = NULL
                 WHERE id = 'direct'",
                [],
            )
            .unwrap();

        let reservation = db
            .reserve_credential_operation(&binding_id, 1, CredentialMutationKind::Clear, None)
            .unwrap();
        db.publish_credential_operation(&reservation, None).unwrap();

        let snapshot = db
            .credential_binding_snapshot(&binding_id)
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.credential_version, 2);
        assert!(!snapshot.enabled);
        assert!(snapshot.fingerprint.is_none());
        assert!(snapshot.credential_slot.is_none());
    }

    #[test]
    fn pending_staging_cleanup_never_claims_a_globally_active_slot() {
        let db = Database::memory().unwrap();
        insert_direct_provider(&db, "one");
        insert_direct_provider(&db, "two");
        let one = direct_binding_for_agent(&db, "codex", "one");
        let two = direct_binding_for_agent(&db, "codex", "two");
        let pending_fingerprint = [13_u8; 32];
        let reservation = db
            .reserve_credential_operation(
                &one,
                0,
                CredentialMutationKind::Set,
                Some(&pending_fingerprint),
            )
            .unwrap();
        let staging_slot = reservation.staging_slot.as_deref().unwrap();
        install_credential_metadata(&db, &two, &[14_u8; 32], staging_slot, 1, false);

        assert!(!db.claim_pending_staging_cleanup(&reservation).unwrap());
        assert_eq!(
            db.credential_journal_entries().unwrap()[0].status,
            "pending"
        );
    }

    #[test]
    fn journal_entries_are_ordered_by_binding_generation_not_wall_clock() {
        let db = Database::memory().unwrap();
        insert_direct_provider(&db, "direct");
        let binding_id = direct_binding_for_agent(&db, "codex", "direct");
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO agent_credential_operations (
                 operation_id, binding_id, generation, operation_kind, status,
                 staging_slot, previous_slot, created_at, updated_at
             ) VALUES ('generation-two', ?1, 2, 'clear', 'cleanup',
                       NULL, 'slot-one', 10, 10)",
            [&binding_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO agent_credential_operations (
                 operation_id, binding_id, generation, operation_kind, status,
                 staging_slot, previous_slot, created_at, updated_at
             ) VALUES ('generation-one', ?1, 1, 'replace', 'cleanup',
                       'slot-one', 'slot-zero', 20, 20)",
            [&binding_id],
        )
        .unwrap();
        drop(conn);

        let generations = db
            .credential_journal_entries()
            .unwrap()
            .into_iter()
            .map(|entry| entry.generation)
            .collect::<Vec<_>>();
        assert_eq!(generations, vec![1, 2]);
    }
}
