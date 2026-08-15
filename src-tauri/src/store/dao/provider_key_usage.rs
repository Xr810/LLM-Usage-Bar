use crate::error::AppError;
use crate::store::{lock_conn, Database};
use crate::usage::domain::SystemProviderKeyUsageView;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) struct ProviderKeyUsageSnapshotInput {
    pub(crate) key_id: String,
    pub(crate) credential_version: u64,
    pub(crate) usage_total_usd: Option<String>,
    pub(crate) usage_daily_usd: Option<String>,
    pub(crate) usage_weekly_usd: Option<String>,
    pub(crate) usage_monthly_usd: Option<String>,
    pub(crate) limit_usd: Option<String>,
    pub(crate) limit_remaining_usd: Option<String>,
    pub(crate) is_free_tier: Option<bool>,
}

fn public_error(code: &'static str) -> AppError {
    AppError::Message(code.to_string())
}

fn now_timestamp() -> Result<i64, AppError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|_| public_error("connection_failed"))
}

pub(super) fn provider_key_usage_view_on_conn(
    conn: &rusqlite::Connection,
    key_id: &str,
    current_credential_version: u64,
) -> Result<Option<SystemProviderKeyUsageView>, AppError> {
    conn.query_row(
        "SELECT credential_version, usage_total_usd, usage_daily_usd,
                usage_weekly_usd, usage_monthly_usd, limit_usd,
                limit_remaining_usd, is_free_tier, fetched_at
         FROM provider_key_usage_snapshots WHERE key_id = ?1",
        [key_id],
        |row| {
            let raw_credential_version = row.get::<_, i64>(0)?;
            let credential_version = u64::try_from(raw_credential_version)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, raw_credential_version))?;
            Ok(SystemProviderKeyUsageView {
                usage_total_usd: row.get(1)?,
                usage_daily_usd: row.get(2)?,
                usage_weekly_usd: row.get(3)?,
                usage_monthly_usd: row.get(4)?,
                limit_usd: row.get(5)?,
                limit_remaining_usd: row.get(6)?,
                is_free_tier: row.get(7)?,
                fetched_at: row.get(8)?,
                credential_version,
                stale: credential_version != current_credential_version,
            })
        },
    )
    .optional()
    .map_err(AppError::from)
}

impl Database {
    pub(crate) fn provider_key_usage_view(
        &self,
        key_id: &str,
        current_credential_version: u64,
    ) -> Result<Option<SystemProviderKeyUsageView>, AppError> {
        let conn = lock_conn!(self.conn);
        provider_key_usage_view_on_conn(&conn, key_id, current_credential_version)
    }

    pub(crate) fn upsert_provider_key_usage_snapshot(
        &self,
        input: &ProviderKeyUsageSnapshotInput,
    ) -> Result<SystemProviderKeyUsageView, AppError> {
        if input.credential_version == 0 {
            return Err(public_error("credential_conflict"));
        }
        let credential_version = i64::try_from(input.credential_version)
            .map_err(|_| public_error("credential_conflict"))?;
        let fetched_at = now_timestamp()?;
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current_version = transaction
            .query_row(
                "SELECT credential_version FROM provider_api_keys
                 WHERE id = ?1",
                [&input.key_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if current_version != Some(credential_version) {
            return Err(public_error("credential_conflict"));
        }

        transaction.execute(
            "INSERT INTO provider_key_usage_snapshots (
                key_id, credential_version, usage_total_usd,
                usage_daily_usd, usage_weekly_usd, usage_monthly_usd,
                limit_usd, limit_remaining_usd, is_free_tier,
                fetched_at, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, ?10)
             ON CONFLICT(key_id) DO UPDATE SET
                credential_version = excluded.credential_version,
                usage_total_usd = excluded.usage_total_usd,
                usage_daily_usd = excluded.usage_daily_usd,
                usage_weekly_usd = excluded.usage_weekly_usd,
                usage_monthly_usd = excluded.usage_monthly_usd,
                limit_usd = excluded.limit_usd,
                limit_remaining_usd = excluded.limit_remaining_usd,
                is_free_tier = excluded.is_free_tier,
                fetched_at = excluded.fetched_at,
                updated_at = excluded.updated_at",
            params![
                input.key_id,
                credential_version,
                input.usage_total_usd,
                input.usage_daily_usd,
                input.usage_weekly_usd,
                input.usage_monthly_usd,
                input.limit_usd,
                input.limit_remaining_usd,
                input.is_free_tier,
                fetched_at,
            ],
        )?;
        let view =
            provider_key_usage_view_on_conn(&transaction, &input.key_id, input.credential_version)?
                .ok_or_else(|| {
                    AppError::Database("provider key usage write was not visible".to_string())
                })?;
        transaction.commit()?;
        Ok(view)
    }
}
