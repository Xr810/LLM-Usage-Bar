use crate::database::{lock_conn, Database};
use crate::error::AppError;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use std::time::{SystemTime, UNIX_EPOCH};

fn public_error(code: &'static str) -> AppError {
    AppError::Message(code.to_string())
}

fn now_timestamp() -> Result<i64, AppError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|_| public_error("credential_unavailable"))
}

const MAX_LABEL_BYTES: usize = 120;
/// A Provider with dozens of keys is a mistake, not a workflow, and every key
/// costs a scheduled upstream request every refresh interval.
const MAX_KEYS_PER_PROVIDER: i64 = 20;

fn normalize_label(label: &str) -> Result<String, AppError> {
    let trimmed = label.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_LABEL_BYTES {
        return Err(public_error("invalid_label"));
    }
    // Control characters would let a label forge line breaks in any surface that
    // renders it as text.
    if trimmed.chars().any(char::is_control) {
        return Err(public_error("invalid_label"));
    }
    Ok(trimmed.to_string())
}

pub(crate) struct ProviderApiKeyRow {
    pub(crate) id: String,
    pub(crate) provider_id: String,
    pub(crate) label: String,
    pub(crate) fingerprint: Option<Vec<u8>>,
    pub(crate) credential_slot: Option<String>,
    pub(crate) credential_version: u64,
    pub(crate) last_test_at: Option<i64>,
    pub(crate) last_test_status: Option<String>,
    pub(crate) last_test_error_code: Option<String>,
    pub(crate) sort_order: i64,
}

fn row_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProviderApiKeyRow> {
    let raw_version = row.get::<_, i64>(5)?;
    Ok(ProviderApiKeyRow {
        id: row.get(0)?,
        provider_id: row.get(1)?,
        label: row.get(2)?,
        fingerprint: row.get(3)?,
        credential_slot: row.get(4)?,
        credential_version: u64::try_from(raw_version)
            .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(5, raw_version))?,
        last_test_at: row.get(6)?,
        last_test_status: row.get(7)?,
        last_test_error_code: row.get(8)?,
        sort_order: row.get(9)?,
    })
}

const SELECT_COLUMNS: &str = "id, provider_id, label, api_key_fingerprint, credential_slot,
     credential_version, last_test_at, last_test_status, last_test_error_code, sort_order";

pub(super) fn list_provider_api_keys_on_conn(
    conn: &rusqlite::Connection,
    provider_id: &str,
) -> Result<Vec<ProviderApiKeyRow>, AppError> {
    let mut statement = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM provider_api_keys
         WHERE provider_id = ?1
         ORDER BY sort_order, created_at, id"
    ))?;
    let keys = statement
        .query_map([provider_id], row_from)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(AppError::from)?;
    Ok(keys)
}

impl Database {
    pub(crate) fn list_provider_api_keys(
        &self,
        provider_id: &str,
    ) -> Result<Vec<ProviderApiKeyRow>, AppError> {
        let conn = lock_conn!(self.conn);
        list_provider_api_keys_on_conn(&conn, provider_id)
    }

    pub(crate) fn provider_api_key(
        &self,
        key_id: &str,
    ) -> Result<Option<ProviderApiKeyRow>, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            &format!("SELECT {SELECT_COLUMNS} FROM provider_api_keys WHERE id = ?1"),
            [key_id],
            row_from,
        )
        .optional()
        .map_err(AppError::from)
    }

    /// Adds an empty named key. The credential is written separately through the
    /// journalled mutation path, so a half-finished add leaves a labelled row
    /// with no secret rather than a secret with no owner.
    pub(crate) fn create_provider_api_key(
        &self,
        provider_id: &str,
        label: &str,
    ) -> Result<String, AppError> {
        let label = normalize_label(label)?;
        let key_id = uuid::Uuid::new_v4().to_string();
        let now = now_timestamp()?;
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let provider_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM usage_providers WHERE id = ?1)",
            [provider_id],
            |row| row.get(0),
        )?;
        if !provider_exists {
            return Err(public_error("unsupported_auth"));
        }

        let (count, next_order) = transaction.query_row(
            "SELECT COUNT(*), COALESCE(MAX(sort_order) + 1, 0)
             FROM provider_api_keys WHERE provider_id = ?1",
            [provider_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )?;
        if count >= MAX_KEYS_PER_PROVIDER {
            return Err(public_error("too_many_keys"));
        }

        let inserted = transaction.execute(
            "INSERT INTO provider_api_keys (
                 id, provider_id, label, credential_version, sort_order,
                 created_at, updated_at
             ) VALUES (?1, ?2, ?3, 0, ?4, ?5, ?5)",
            params![key_id, provider_id, label, next_order, now],
        );
        match inserted {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(sqlite, _))
                if sqlite.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                return Err(public_error("duplicate_label"));
            }
            Err(error) => return Err(error.into()),
        }
        transaction.commit()?;
        Ok(key_id)
    }

    pub(crate) fn rename_provider_api_key(
        &self,
        key_id: &str,
        label: &str,
    ) -> Result<(), AppError> {
        let label = normalize_label(label)?;
        let now = now_timestamp()?;
        let conn = lock_conn!(self.conn);
        let renamed = conn.execute(
            "UPDATE provider_api_keys SET label = ?2, updated_at = ?3 WHERE id = ?1",
            params![key_id, label, now],
        );
        match renamed {
            Ok(1) => Ok(()),
            Ok(_) => Err(public_error("credential_conflict")),
            Err(rusqlite::Error::SqliteFailure(sqlite, _))
                if sqlite.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(public_error("duplicate_label"))
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Removes a key row that no longer owns a secret. The caller must have
    /// cleared the credential first; refusing otherwise is what keeps a deleted
    /// key from stranding its secret in the OS keychain forever.
    pub(crate) fn delete_provider_api_key(
        &self,
        key_id: &str,
        expected_version: u64,
    ) -> Result<(), AppError> {
        let expected_version =
            i64::try_from(expected_version).map_err(|_| public_error("credential_conflict"))?;
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let pending: bool = transaction.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM provider_credential_operations WHERE key_id = ?1
             )",
            [key_id],
            |row| row.get(0),
        )?;
        if pending {
            return Err(public_error("credential_conflict"));
        }

        let deleted = transaction.execute(
            "DELETE FROM provider_api_keys
             WHERE id = ?1 AND credential_version = ?2
               AND api_key_fingerprint IS NULL AND credential_slot IS NULL",
            params![key_id, expected_version],
        )?;
        if deleted != 1 {
            return Err(public_error("credential_conflict"));
        }
        transaction.commit()?;
        Ok(())
    }
}
