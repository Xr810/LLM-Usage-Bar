use crate::database::{lock_conn, to_json_string, Database};
use crate::error::AppError;
use crate::usage::domain::{QuotaFetchState, QuotaSnapshot};
use rusqlite::{params, types::Type, Connection, OptionalExtension, Row};

fn quota_snapshot_from_row(row: &Row<'_>) -> rusqlite::Result<QuotaSnapshot> {
    let raw_payload = serde_json::from_str(&row.get::<_, String>(8)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(8, Type::Text, Box::new(error))
    })?;
    Ok(QuotaSnapshot {
        snapshot_id: row.get(0)?,
        provider_id: row.get(1)?,
        fetched_at: row.get(2)?,
        five_hour_utilization_percent: row.get(3)?,
        five_hour_resets_at: row.get(4)?,
        seven_day_utilization_percent: row.get(5)?,
        seven_day_resets_at: row.get(6)?,
        manual_resets_remaining: row.get(7)?,
        raw_payload,
        created_at: row.get(9)?,
    })
}

fn quota_fetch_state_from_row(row: &Row<'_>) -> rusqlite::Result<QuotaFetchState> {
    Ok(QuotaFetchState {
        provider_id: row.get(0)?,
        last_attempt_at: row.get(1)?,
        last_success_at: row.get(2)?,
        last_error: row.get(3)?,
        stale: row.get(4)?,
    })
}

fn fetch_state_on_conn(
    conn: &Connection,
    provider_id: &str,
) -> Result<Option<QuotaFetchState>, AppError> {
    conn.query_row(
        "SELECT provider_id, last_attempt_at, last_success_at, last_error, stale
         FROM quota_fetch_state WHERE provider_id = ?1",
        [provider_id],
        quota_fetch_state_from_row,
    )
    .optional()
    .map_err(AppError::from)
}

impl Database {
    pub fn append_quota_success(
        &self,
        snapshot: &QuotaSnapshot,
    ) -> Result<QuotaFetchState, AppError> {
        let raw_payload = to_json_string(&snapshot.raw_payload)?;
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction()?;
        transaction.execute(
            "INSERT INTO quota_snapshots (
                snapshot_id, provider_id, fetched_at, five_hour_utilization_percent,
                five_hour_resets_at, seven_day_utilization_percent, seven_day_resets_at,
                manual_resets_remaining, raw_payload, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                snapshot.snapshot_id,
                snapshot.provider_id,
                snapshot.fetched_at,
                snapshot.five_hour_utilization_percent,
                snapshot.five_hour_resets_at,
                snapshot.seven_day_utilization_percent,
                snapshot.seven_day_resets_at,
                snapshot.manual_resets_remaining,
                raw_payload,
                snapshot.created_at,
            ],
        )?;
        transaction.execute(
            "INSERT INTO quota_fetch_state (
                provider_id, last_attempt_at, last_success_at, last_error, stale
             ) VALUES (?1, ?2, ?2, NULL, 0)
             ON CONFLICT(provider_id) DO UPDATE SET
                last_attempt_at = excluded.last_attempt_at,
                last_success_at = excluded.last_success_at,
                last_error = NULL,
                stale = 0",
            params![snapshot.provider_id, snapshot.fetched_at],
        )?;
        let state = fetch_state_on_conn(&transaction, &snapshot.provider_id)?
            .ok_or_else(|| AppError::Database("quota fetch state was not saved".to_string()))?;
        transaction.commit()?;
        Ok(state)
    }

    pub fn record_quota_failure(
        &self,
        provider_id: &str,
        attempted_at: i64,
        error: &str,
    ) -> Result<QuotaFetchState, AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "INSERT INTO quota_fetch_state (
                provider_id, last_attempt_at, last_success_at, last_error, stale
             ) VALUES (?1, ?2, NULL, ?3, 1)
             ON CONFLICT(provider_id) DO UPDATE SET
                last_attempt_at = excluded.last_attempt_at,
                last_error = excluded.last_error,
                stale = 1",
            params![provider_id, attempted_at, error],
        )?;
        fetch_state_on_conn(&conn, provider_id)?
            .ok_or_else(|| AppError::Database("quota fetch state was not saved".to_string()))
    }

    pub fn latest_quota_snapshot(
        &self,
        provider_id: &str,
    ) -> Result<Option<QuotaSnapshot>, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT snapshot_id, provider_id, fetched_at,
                    five_hour_utilization_percent, five_hour_resets_at,
                    seven_day_utilization_percent, seven_day_resets_at,
                    manual_resets_remaining, raw_payload, created_at
             FROM quota_snapshots WHERE provider_id = ?1
             ORDER BY fetched_at DESC, snapshot_id DESC LIMIT 1",
            [provider_id],
            quota_snapshot_from_row,
        )
        .optional()
        .map_err(AppError::from)
    }

    pub fn get_quota_fetch_state(
        &self,
        provider_id: &str,
    ) -> Result<Option<QuotaFetchState>, AppError> {
        let conn = lock_conn!(self.conn);
        fetch_state_on_conn(&conn, provider_id)
    }
}

#[cfg(test)]
mod tests {
    use crate::database::Database;
    use crate::usage::domain::{BillingKind, QuotaSnapshot, TokenSource, UsageProviderInput};
    use serde_json::json;

    fn save_provider(db: &Database, id: &str) {
        db.save_usage_provider(&UsageProviderInput {
            id: id.to_string(),
            name: id.to_string(),
            billing_kind: BillingKind::Subscription,
            product_group_id: "claude".to_string(),
            token_sources: vec![TokenSource::SessionLog],
            quota_source: Some("official".to_string()),
            quota_interval_seconds: Some(300),
            route_app_type: None,
            route_config: None,
            quota_config: None,
            enabled: true,
        })
        .unwrap();
    }

    fn snapshot(id: &str, fetched_at: i64) -> QuotaSnapshot {
        QuotaSnapshot {
            snapshot_id: id.to_string(),
            provider_id: "sub".to_string(),
            fetched_at,
            five_hour_utilization_percent: Some("25.5".to_string()),
            five_hour_resets_at: Some("2026-01-01T05:00:00Z".to_string()),
            seven_day_utilization_percent: Some("50".to_string()),
            seven_day_resets_at: Some("2026-01-07T00:00:00Z".to_string()),
            manual_resets_remaining: Some(2),
            raw_payload: json!({"remaining": 75}),
            created_at: fetched_at + 1,
        }
    }

    #[test]
    fn quota_success_appends_snapshot_and_records_fresh_fetch_state() {
        let db = Database::memory().unwrap();
        save_provider(&db, "sub");
        let first = snapshot("first", 100);

        let state = db.append_quota_success(&first).unwrap();
        assert_eq!(state.provider_id, "sub");
        assert_eq!(state.last_attempt_at, Some(100));
        assert_eq!(state.last_success_at, Some(100));
        assert_eq!(state.last_error, None);
        assert!(!state.stale);
        assert_eq!(db.latest_quota_snapshot("sub").unwrap(), Some(first));
        assert_eq!(db.get_quota_fetch_state("sub").unwrap(), Some(state));
    }

    #[test]
    fn quota_failure_preserves_last_successful_snapshot_and_marks_state_stale() {
        let db = Database::memory().unwrap();
        save_provider(&db, "sub");
        let first_success_at = 100;
        let first = snapshot("first", first_success_at);
        db.append_quota_success(&first).unwrap();

        let state = db.record_quota_failure("sub", 200, "timeout").unwrap();
        assert_eq!(state.last_attempt_at, Some(200));
        assert_eq!(state.last_success_at, Some(first_success_at));
        assert_eq!(state.last_error.as_deref(), Some("timeout"));
        assert!(state.stale);
        assert_eq!(
            db.latest_quota_snapshot("sub").unwrap().unwrap().fetched_at,
            first_success_at
        );
        assert_eq!(db.get_quota_fetch_state("sub").unwrap(), Some(state));
    }
}
