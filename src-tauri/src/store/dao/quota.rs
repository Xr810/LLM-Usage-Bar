use crate::error::AppError;
use crate::model::{QuotaFetchState, QuotaSnapshot};
use crate::store::{lock_conn, to_json_string, Database};
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
        consecutive_failures: row.get::<_, i64>(4)? as u32,
        stale: row.get(5)?,
    })
}

fn fetch_state_on_conn(
    conn: &Connection,
    provider_id: &str,
) -> Result<Option<QuotaFetchState>, AppError> {
    conn.query_row(
        "SELECT provider_id, last_attempt_at, last_success_at, last_error,
                consecutive_failures, stale
         FROM quota_fetch_state WHERE provider_id = ?1",
        [provider_id],
        quota_fetch_state_from_row,
    )
    .optional()
    .map_err(AppError::from)
}

fn latest_snapshot_on_conn(
    conn: &Connection,
    provider_id: &str,
) -> Result<Option<QuotaSnapshot>, AppError> {
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

impl Database {
    pub fn append_quota_success(
        &self,
        snapshot: &QuotaSnapshot,
    ) -> Result<QuotaFetchState, AppError> {
        let raw_payload = to_json_string(&snapshot.raw_payload)?;
        let mut conn = lock_conn!(self.conn);
        let transaction = conn.transaction()?;
        let previous = latest_snapshot_on_conn(&transaction, &snapshot.provider_id)?;
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
                provider_id, last_attempt_at, last_success_at, last_error,
                consecutive_failures, stale
             ) VALUES (?1, ?2, ?2, NULL, 0, 0)
             ON CONFLICT(provider_id) DO UPDATE SET
                last_attempt_at = excluded.last_attempt_at,
                last_success_at = excluded.last_success_at,
                last_error = NULL,
                consecutive_failures = 0,
                stale = 0
             WHERE excluded.last_attempt_at >= COALESCE(
                quota_fetch_state.last_attempt_at, -9223372036854775808
             )",
            params![snapshot.provider_id, snapshot.fetched_at],
        )?;
        let state = fetch_state_on_conn(&transaction, &snapshot.provider_id)?
            .ok_or_else(|| AppError::Database("quota fetch state was not saved".to_string()))?;
        if let Some(previous) = previous {
            if let Err(error) = crate::usage::usage_light_prediction::resolve_snapshot_rollovers(
                &transaction,
                &previous,
                snapshot,
            ) {
                log::warn!(
                    "Failed to resolve usage light predictions after quota rollover for {}: {error}",
                    snapshot.provider_id
                );
            }
        }
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
                provider_id, last_attempt_at, last_success_at, last_error,
                consecutive_failures, stale
             ) VALUES (?1, ?2, NULL, ?3, 1, 0)
             ON CONFLICT(provider_id) DO UPDATE SET
                last_attempt_at = excluded.last_attempt_at,
                last_error = excluded.last_error,
                consecutive_failures = quota_fetch_state.consecutive_failures + 1,
                stale = CASE
                    WHEN quota_fetch_state.consecutive_failures + 1 >= 5 THEN 1
                    ELSE 0
                END
             WHERE excluded.last_attempt_at > COALESCE(
                quota_fetch_state.last_attempt_at, -9223372036854775808
             )",
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
        latest_snapshot_on_conn(&conn, provider_id)
    }

    pub fn quota_snapshots_since(
        &self,
        provider_id: &str,
        since: i64,
    ) -> Result<Vec<QuotaSnapshot>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut statement = conn.prepare(
            "SELECT snapshot_id, provider_id, fetched_at,
                    five_hour_utilization_percent, five_hour_resets_at,
                    seven_day_utilization_percent, seven_day_resets_at,
                    manual_resets_remaining, raw_payload, created_at
             FROM quota_snapshots
             WHERE provider_id = ?1 AND fetched_at >= ?2
             ORDER BY fetched_at ASC, snapshot_id ASC",
        )?;
        let rows = statement.query_map(params![provider_id, since], quota_snapshot_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    pub fn get_quota_fetch_state(
        &self,
        provider_id: &str,
    ) -> Result<Option<QuotaFetchState>, AppError> {
        let conn = lock_conn!(self.conn);
        fetch_state_on_conn(&conn, provider_id)
    }

    pub fn latest_quota_status(
        &self,
        provider_id: &str,
    ) -> Result<(Option<QuotaSnapshot>, Option<QuotaFetchState>), AppError> {
        let conn = lock_conn!(self.conn);
        Ok((
            latest_snapshot_on_conn(&conn, provider_id)?,
            fetch_state_on_conn(&conn, provider_id)?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{BillingKind, QuotaSnapshot, TokenSource, UsageProviderInput};
    use crate::store::Database;
    use serde_json::json;

    fn save_provider(db: &Database, id: &str) {
        db.save_usage_provider(&UsageProviderInput {
            id: id.to_string(),
            name: id.to_string(),
            billing_kind: BillingKind::Subscription,
            product_group_id: "claude".to_string(),
            token_sources: vec![TokenSource::SessionLog],
            session_source_bindings: None,
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
        assert_eq!(state.consecutive_failures, 0);
        assert!(!state.stale);
        assert_eq!(db.latest_quota_snapshot("sub").unwrap(), Some(first));
        assert_eq!(db.get_quota_fetch_state("sub").unwrap(), Some(state));
    }

    #[test]
    fn quota_failure_preserves_snapshot_and_only_marks_stale_after_ten_minute_retry() {
        let db = Database::memory().unwrap();
        save_provider(&db, "sub");
        let first_success_at = 100;
        let first = snapshot("first", first_success_at);
        db.append_quota_success(&first).unwrap();

        let mut state = db.record_quota_failure("sub", 200, "timeout").unwrap();
        assert_eq!(state.last_attempt_at, Some(200));
        assert_eq!(state.last_success_at, Some(first_success_at));
        assert_eq!(state.last_error.as_deref(), Some("timeout"));
        assert_eq!(state.consecutive_failures, 1);
        assert!(!state.stale);
        for attempt in 2..=5 {
            state = db
                .record_quota_failure("sub", 200 + i64::from(attempt), "timeout")
                .unwrap();
            assert_eq!(state.consecutive_failures, attempt);
            assert_eq!(state.stale, attempt >= 5);
        }
        assert_eq!(
            db.latest_quota_snapshot("sub").unwrap().unwrap().fetched_at,
            first_success_at
        );
        assert_eq!(db.get_quota_fetch_state("sub").unwrap(), Some(state));
    }

    #[test]
    fn older_failure_cannot_overwrite_a_newer_success() {
        let db = Database::memory().unwrap();
        save_provider(&db, "sub");
        db.append_quota_success(&snapshot("new", 200)).unwrap();

        let state = db.record_quota_failure("sub", 100, "late timeout").unwrap();
        assert_eq!(state.last_attempt_at, Some(200));
        assert_eq!(state.last_success_at, Some(200));
        assert_eq!(state.last_error, None);
        assert_eq!(state.consecutive_failures, 0);
        assert!(!state.stale);
    }

    #[test]
    fn older_success_cannot_roll_back_newer_fetch_state_or_snapshot() {
        let db = Database::memory().unwrap();
        save_provider(&db, "sub");
        let newer = snapshot("new", 200);
        db.append_quota_success(&newer).unwrap();

        let state = db.append_quota_success(&snapshot("old-late", 100)).unwrap();
        assert_eq!(state.last_attempt_at, Some(200));
        assert_eq!(state.last_success_at, Some(200));
        assert_eq!(db.latest_quota_snapshot("sub").unwrap(), Some(newer));
    }

    #[test]
    fn latest_quota_status_reads_snapshot_and_state_together() {
        let db = Database::memory().unwrap();
        save_provider(&db, "sub");
        let current = snapshot("current", 200);
        let state = db.append_quota_success(&current).unwrap();

        assert_eq!(
            db.latest_quota_status("sub").unwrap(),
            (Some(current), Some(state))
        );
    }

    #[test]
    fn quota_snapshots_since_filters_and_orders_history_stably() {
        let db = Database::memory().unwrap();
        save_provider(&db, "sub");
        for (id, fetched_at) in [
            ("later", 300),
            ("same-b", 200),
            ("old", 100),
            ("same-a", 200),
        ] {
            db.append_quota_success(&snapshot(id, fetched_at)).unwrap();
        }

        let snapshots = db.quota_snapshots_since("sub", 200).unwrap();
        assert_eq!(
            snapshots
                .iter()
                .map(|snapshot| (snapshot.fetched_at, snapshot.snapshot_id.as_str()))
                .collect::<Vec<_>>(),
            vec![(200, "same-a"), (200, "same-b"), (300, "later")],
        );
    }
}
