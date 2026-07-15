use super::binding_credentials::CredentialMutationKind;
use crate::database::{lock_conn, Database};
use crate::error::AppError;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
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

fn provider_staging_slot(provider_id: &str, generation: u64) -> String {
    format!(
        "provider/{provider_id}/{generation}/{}",
        uuid::Uuid::new_v4()
    )
}

pub(crate) struct ProviderCredentialSnapshot {
    pub(crate) fingerprint: Option<Vec<u8>>,
    pub(crate) credential_slot: Option<String>,
    pub(crate) credential_version: u64,
}

pub(crate) struct ProviderCredentialOperationReservation {
    pub(crate) operation_id: String,
    pub(crate) provider_id: String,
    pub(crate) kind: CredentialMutationKind,
    pub(crate) expected_version: u64,
    pub(crate) generation: u64,
    pub(crate) staging_slot: Option<String>,
    pub(crate) previous_slot: Option<String>,
    pub(crate) previous_fingerprint: Option<Vec<u8>>,
}

pub(crate) struct ProviderCredentialJournalEntry {
    pub(crate) operation_id: String,
    pub(crate) provider_id: String,
    pub(crate) kind: CredentialMutationKind,
    pub(crate) status: String,
    pub(crate) generation: u64,
    pub(crate) staging_slot: Option<String>,
    pub(crate) previous_slot: Option<String>,
}

pub(crate) struct ProviderCredentialReconcileState {
    pub(crate) credential_version: u64,
    pub(crate) credential_slot: Option<String>,
    pub(crate) has_fingerprint: bool,
}

fn parse_mutation_kind(raw: &str) -> Result<CredentialMutationKind, AppError> {
    match raw {
        "set" => Ok(CredentialMutationKind::Set),
        "replace" => Ok(CredentialMutationKind::Replace),
        "clear" => Ok(CredentialMutationKind::Clear),
        _ => Err(public_error("credential_unavailable")),
    }
}

impl Database {
    pub(crate) fn provider_credential_snapshot(
        &self,
        provider_id: &str,
    ) -> Result<Option<ProviderCredentialSnapshot>, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT api_key_fingerprint, credential_slot,
                    credential_version
             FROM provider_api_credentials WHERE provider_id = ?1",
            [provider_id],
            |row| {
                let raw_version = row.get::<_, i64>(2)?;
                Ok(ProviderCredentialSnapshot {
                    fingerprint: row.get(0)?,
                    credential_slot: row.get(1)?,
                    credential_version: u64::try_from(raw_version)
                        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(2, raw_version))?,
                })
            },
        )
        .optional()
        .map_err(AppError::from)
    }

    pub(crate) fn record_provider_connection_test(
        &self,
        provider_id: &str,
        expected_version: u64,
        status: &str,
        error_code: Option<&str>,
    ) -> Result<i64, AppError> {
        if !matches!(status, "success" | "failed") {
            return Err(public_error("invalid_connection_status"));
        }
        let expected_version =
            i64::try_from(expected_version).map_err(|_| public_error("credential_conflict"))?;
        let tested_at = now_timestamp()?;
        let conn = lock_conn!(self.conn);
        let changed = conn.execute(
            "UPDATE provider_api_credentials
             SET last_test_at = ?3, last_test_status = ?4,
                 last_test_error_code = ?5, updated_at = ?3
             WHERE provider_id = ?1 AND credential_version = ?2",
            params![provider_id, expected_version, tested_at, status, error_code],
        )?;
        if changed != 1 {
            return Err(public_error("credential_conflict"));
        }
        Ok(tested_at)
    }

    pub(crate) fn reserve_provider_credential_operation(
        &self,
        provider_id: &str,
        expected_version: u64,
        kind: CredentialMutationKind,
        new_fingerprint: Option<&[u8; 32]>,
    ) -> Result<ProviderCredentialOperationReservation, AppError> {
        if kind == CredentialMutationKind::Delete {
            return Err(public_error("unsupported_auth"));
        }
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
        .then(|| provider_staging_slot(provider_id, generation));

        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = transaction
            .query_row(
                "SELECT credential.credential_version,
                        credential.api_key_fingerprint, credential.credential_slot
                 FROM provider_api_credentials credential
                 JOIN usage_providers provider ON provider.id = credential.provider_id
                 WHERE credential.provider_id = ?1
                   AND provider.system_preset_key IN (
                       'openai-api', 'anthropic-api', 'openrouter-api'
                   )",
                [provider_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<Vec<u8>>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| public_error("unsupported_auth"))?;
        let current_version =
            u64::try_from(state.0).map_err(|_| public_error("credential_unavailable"))?;
        if current_version != expected_version {
            return Err(public_error("credential_conflict"));
        }
        let current_fingerprint = state.1;
        let current_slot = state.2;
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
            CredentialMutationKind::Delete => unreachable!(),
        }

        let insert = transaction.execute(
            "INSERT INTO provider_credential_operations (
                 operation_id, provider_id, generation, operation_kind, status,
                 staging_slot, previous_slot, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?6, ?7, ?7)",
            params![
                operation_id,
                provider_id,
                generation_i64,
                kind.as_str(),
                staging_slot,
                current_slot,
                now,
            ],
        );
        if let Err(error) = insert {
            return if is_constraint_error(&error) {
                Err(public_error("credential_conflict"))
            } else {
                Err(error.into())
            };
        }
        transaction.commit()?;
        Ok(ProviderCredentialOperationReservation {
            operation_id,
            provider_id: provider_id.to_string(),
            kind,
            expected_version,
            generation,
            staging_slot,
            previous_slot: current_slot,
            previous_fingerprint: current_fingerprint,
        })
    }

    pub(crate) fn publish_provider_credential_operation(
        &self,
        reservation: &ProviderCredentialOperationReservation,
        new_fingerprint: Option<&[u8; 32]>,
    ) -> Result<(), AppError> {
        let expected = i64::try_from(reservation.expected_version)
            .map_err(|_| public_error("credential_conflict"))?;
        let generation = i64::try_from(reservation.generation)
            .map_err(|_| public_error("credential_conflict"))?;
        let now = now_timestamp()?;
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let updated = match reservation.kind {
            CredentialMutationKind::Set | CredentialMutationKind::Replace => {
                let fingerprint =
                    new_fingerprint.ok_or_else(|| public_error("credential_required"))?;
                let staging_slot = reservation
                    .staging_slot
                    .as_deref()
                    .ok_or_else(|| public_error("credential_unavailable"))?;
                transaction.execute(
                    "UPDATE provider_api_credentials
                     SET api_key_fingerprint = ?2, credential_slot = ?3,
                         credential_version = ?4, updated_at = ?5
                     WHERE provider_id = ?1 AND credential_version = ?6
                       AND credential_slot IS ?7 AND api_key_fingerprint IS ?8
                       AND EXISTS (
                           SELECT 1 FROM provider_credential_operations
                           WHERE operation_id = ?9 AND provider_id = ?1
                             AND generation = ?4 AND status = 'pending'
                             AND staging_slot = ?3
                       )",
                    params![
                        reservation.provider_id,
                        fingerprint.as_slice(),
                        staging_slot,
                        generation,
                        now,
                        expected,
                        reservation.previous_slot,
                        reservation.previous_fingerprint,
                        reservation.operation_id,
                    ],
                )
            }
            CredentialMutationKind::Clear => transaction.execute(
                "UPDATE provider_api_credentials
                 SET api_key_fingerprint = NULL, credential_slot = NULL,
                     credential_version = ?2, updated_at = ?3
                 WHERE provider_id = ?1 AND credential_version = ?4
                   AND credential_slot IS ?5 AND api_key_fingerprint IS ?6
                   AND EXISTS (
                       SELECT 1 FROM provider_credential_operations
                       WHERE operation_id = ?7 AND provider_id = ?1
                         AND generation = ?2 AND status = 'pending'
                   )",
                params![
                    reservation.provider_id,
                    generation,
                    now,
                    expected,
                    reservation.previous_slot,
                    reservation.previous_fingerprint,
                    reservation.operation_id,
                ],
            ),
            CredentialMutationKind::Delete => return Err(public_error("unsupported_auth")),
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
            "UPDATE provider_credential_operations
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

    pub(crate) fn provider_credential_slot_is_active(&self, slot: &str) -> Result<bool, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM provider_api_credentials WHERE credential_slot = ?1
                 UNION ALL
                 SELECT 1 FROM agent_provider_bindings WHERE credential_slot = ?1
             )",
            [slot],
            |row| row.get(0),
        )
        .map_err(AppError::from)
    }

    pub(crate) fn provider_credential_reconcile_state(
        &self,
        provider_id: &str,
    ) -> Result<Option<ProviderCredentialReconcileState>, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT credential_version, credential_slot,
                    api_key_fingerprint IS NOT NULL
             FROM provider_api_credentials WHERE provider_id = ?1",
            [provider_id],
            |row| {
                let raw_version = row.get::<_, i64>(0)?;
                Ok(ProviderCredentialReconcileState {
                    credential_version: u64::try_from(raw_version)
                        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, raw_version))?,
                    credential_slot: row.get(1)?,
                    has_fingerprint: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(AppError::from)
    }

    pub(crate) fn claim_pending_provider_staging_cleanup(
        &self,
        reservation: &ProviderCredentialOperationReservation,
    ) -> Result<bool, AppError> {
        let Some(staging_slot) = reservation.staging_slot.as_deref() else {
            return Ok(false);
        };
        let generation = i64::try_from(reservation.generation)
            .map_err(|_| public_error("credential_conflict"))?;
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let claimed = transaction.execute(
            "UPDATE provider_credential_operations
             SET status = 'cleanup', previous_slot = staging_slot, updated_at = ?4
             WHERE operation_id = ?1 AND provider_id = ?2
               AND generation = ?3 AND status = 'pending'
               AND NOT EXISTS (
                   SELECT 1 FROM provider_api_credentials WHERE credential_slot = ?5
               )
               AND NOT EXISTS (
                   SELECT 1 FROM agent_provider_bindings WHERE credential_slot = ?5
               )",
            params![
                reservation.operation_id,
                reservation.provider_id,
                generation,
                now_timestamp()?,
                staging_slot,
            ],
        )?;
        transaction.commit()?;
        Ok(claimed == 1)
    }

    pub(crate) fn finish_claimed_provider_staging_cleanup(
        &self,
        reservation: &ProviderCredentialOperationReservation,
    ) -> Result<(), AppError> {
        let Some(staging_slot) = reservation.staging_slot.as_deref() else {
            return Err(public_error("credential_conflict"));
        };
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if transaction.execute(
            "DELETE FROM provider_credential_operations
             WHERE operation_id = ?1 AND status = 'cleanup'
               AND previous_slot = ?2
               AND NOT EXISTS (
                   SELECT 1 FROM provider_api_credentials WHERE credential_slot = ?2
               )
               AND NOT EXISTS (
                   SELECT 1 FROM agent_provider_bindings WHERE credential_slot = ?2
               )",
            params![reservation.operation_id, staging_slot],
        )? != 1
        {
            return Err(public_error("credential_conflict"));
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn finish_provider_credential_operation(
        &self,
        reservation: &ProviderCredentialOperationReservation,
    ) -> Result<(), AppError> {
        if let Some(previous_slot) = reservation.previous_slot.as_deref() {
            if self.provider_credential_slot_is_active(previous_slot)? {
                return Err(public_error("credential_conflict"));
            }
        }
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if transaction.execute(
            "DELETE FROM provider_credential_operations
             WHERE operation_id = ?1 AND status IN ('committed', 'cleanup')",
            [&reservation.operation_id],
        )? != 1
        {
            return Err(public_error("credential_conflict"));
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn provider_credential_journal_entries(
        &self,
    ) -> Result<Vec<ProviderCredentialJournalEntry>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut statement = conn.prepare(
            "SELECT operation_id, provider_id, operation_kind, status, generation,
                    staging_slot, previous_slot
             FROM provider_credential_operations
             ORDER BY provider_id, generation, operation_id",
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
                    provider_id,
                    kind,
                    status,
                    generation,
                    staging_slot,
                    previous_slot,
                ) = row?;
                Ok(ProviderCredentialJournalEntry {
                    operation_id,
                    provider_id,
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

    pub(crate) fn promote_pending_provider_credential_operation(
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
            "UPDATE provider_credential_operations SET status = ?2, updated_at = ?3
             WHERE operation_id = ?1 AND status = 'pending'",
            params![operation_id, status, now_timestamp()?],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn delete_pending_provider_journal_entry(
        &self,
        operation_id: &str,
        provider_id: &str,
        generation: u64,
    ) -> Result<(), AppError> {
        let generation =
            i64::try_from(generation).map_err(|_| public_error("credential_conflict"))?;
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "DELETE FROM provider_credential_operations
             WHERE operation_id = ?1 AND provider_id = ?2
               AND generation = ?3 AND status = 'pending'
               AND NOT EXISTS (
                   SELECT 1 FROM provider_api_credentials
                   WHERE provider_id = ?2 AND credential_version = ?3
                     AND credential_slot IS NULL AND api_key_fingerprint IS NULL
               )",
            params![operation_id, provider_id, generation],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn finish_provider_journal_entry(
        &self,
        entry: &ProviderCredentialJournalEntry,
    ) -> Result<(), AppError> {
        self.finish_provider_credential_operation(&ProviderCredentialOperationReservation {
            operation_id: entry.operation_id.clone(),
            provider_id: entry.provider_id.clone(),
            kind: entry.kind,
            expected_version: entry.generation.saturating_sub(1),
            generation: entry.generation,
            staging_slot: entry.staging_slot.clone(),
            previous_slot: entry.previous_slot.clone(),
            previous_fingerprint: None,
        })
    }
}
