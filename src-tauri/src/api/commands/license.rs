//! BE-1 License 外壳（契约 C2、C20）。
//!
//! 社区构建恒为 `tier=free`、`build=community`。激活和停用返回
//! `COMMUNITY_BUILD`，不读也不写设置、数据库或钥匙串。

use crate::error::AppError;
use crate::policy::LicenseStatus;

#[tauri::command]
pub fn get_license_status(state: tauri::State<'_, crate::AppState>) -> LicenseStatus {
    state.license.status()
}

#[tauri::command]
pub fn activate_license(
    state: tauri::State<'_, crate::AppState>,
    token: String,
) -> Result<LicenseStatus, AppError> {
    state.license.activate(&token)
}

#[tauri::command]
pub fn deactivate_license(state: tauri::State<'_, crate::AppState>) -> Result<(), AppError> {
    state.license.deactivate()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{
        activate_community_license, deactivate_community_license, LicenseBuild, LicenseProvider,
        LicenseTier,
    };
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use tauri::Manager;

    // Test-only official provider. Production activation stays out of BE-30.
    struct TestLicense(AtomicBool);

    impl LicenseProvider for TestLicense {
        fn status(&self) -> LicenseStatus {
            let mut status = LicenseStatus::community_free();
            status.build = LicenseBuild::Official;
            status.tier = if self.0.load(Ordering::SeqCst) {
                LicenseTier::Pro
            } else {
                LicenseTier::Free
            };
            status
        }

        fn activate(&self, token: &str) -> Result<LicenseStatus, AppError> {
            activate_community_license(token)
        }

        fn deactivate(&self) -> Result<(), AppError> {
            deactivate_community_license()
        }
    }

    impl crate::RetentionPolicyProvider for TestLicense {
        fn retention_policy(&self, detail_days: i64) -> crate::RetentionPolicy {
            crate::RetentionPolicy {
                detail_days,
                hourly_days: if self.0.load(Ordering::SeqCst) {
                    730
                } else {
                    400
                },
            }
        }
    }

    #[tokio::test]
    async fn injected_license_controls_commands_tray_and_downgrade_visibility_not_pruning() {
        let provider = Arc::new(TestLicense(AtomicBool::new(true)));
        let license = provider.clone();
        let retention = provider.clone();
        let injections = crate::Injections {
            license: Box::new(move |_| Ok(license)),
            retention: Box::new(move |_| Ok(retention)),
            ..crate::Injections::community()
        };
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(
            crate::Database::open_before_retention(&dir.path().join("retention.db")).unwrap(),
        );
        let now = chrono::Utc::now();
        let old_hour = (now - chrono::Duration::days(500))
            .timestamp()
            .div_euclid(3600)
            * 3600;
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO usage_hourly_rollups
             (hour_utc, app_type, provider_id, model, billing_identity, data_source, request_count)
             VALUES (?1, 'claude', 'test-provider', 'test-model', '', 'proxy', 7)",
                [old_hour],
            )
            .unwrap();
        let state = crate::AppState::new_with_injections(
            db.clone(),
            crate::secrets::unavailable_credential_store(),
            Arc::new(crate::quota::QuotaService::new(db.clone())),
            injections,
        );
        assert_eq!(
            state.tray_usage_service.cached_snapshot().await.build_type,
            LicenseBuild::Official
        );
        let snapshot = state
            .tray_usage_service
            .rebuild_from_persisted(|_| {})
            .await;
        assert_eq!(snapshot.build_type, LicenseBuild::Official);
        assert!(snapshot.refresh_error.is_none());
        let app = tauri::test::mock_app();
        app.manage(state);
        assert_eq!(get_license_status(app.state()).tier, LicenseTier::Pro);
        let from = (now - chrono::Duration::days(600)).date_naive().to_string();
        let to = now.date_naive().to_string();
        let query = |group: Option<&str>| {
            crate::api::commands::query_usage_trend(
                app.state(),
                "UTC".into(),
                from.clone(),
                to.clone(),
                "day".into(),
                group.map(str::to_string),
                None,
            )
        };
        let pro = query(Some("provider")).unwrap();
        assert_eq!(pro.clamped_from, None);
        assert_eq!(
            pro.series
                .iter()
                .flat_map(|s| &s.points)
                .map(|p| p.requests)
                .sum::<i64>(),
            7
        );

        // Required test (d): downgrade, prune with the lower current policy,
        // retain the 500-day row, but hide it from current Free queries.
        provider.0.store(false, Ordering::SeqCst);
        db.rollup_and_prune_configured().unwrap();
        assert_eq!(
            db.get_setting("retention_high_water_days")
                .unwrap()
                .as_deref(),
            Some("730")
        );
        let retained: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT request_count FROM usage_hourly_rollups WHERE hour_utc = ?1",
                [old_hour],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(retained, 7);
        assert_eq!(get_license_status(app.state()).tier, LicenseTier::Free);
        assert_eq!(
            query(Some("provider")).unwrap_err().code(),
            Some(crate::error::CODE_PRO_REQUIRED)
        );
        let free = query(None).unwrap();
        assert_eq!(
            free.clamped_from,
            Some((now.date_naive() - chrono::Duration::days(29)).to_string())
        );
        assert_eq!(free.series[0].points.len(), 30);
        assert_eq!(
            free.series
                .iter()
                .flat_map(|s| &s.points)
                .map(|p| p.requests)
                .sum::<i64>(),
            0
        );
    }

    /// QA-20: the retention high-water mark only rises, and renewing after a
    /// downgrade makes the retained (hidden) history visible again.
    #[tokio::test]
    async fn renewal_after_downgrade_restores_hidden_history_and_high_water_never_lowers() {
        let provider = Arc::new(TestLicense(AtomicBool::new(true)));
        let license = provider.clone();
        let retention = provider.clone();
        let injections = crate::Injections {
            license: Box::new(move |_| Ok(license)),
            retention: Box::new(move |_| Ok(retention)),
            ..crate::Injections::community()
        };
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(
            crate::Database::open_before_retention(&dir.path().join("renewal.db")).unwrap(),
        );
        let now = chrono::Utc::now();
        let old_hour = (now - chrono::Duration::days(500))
            .timestamp()
            .div_euclid(3600)
            * 3600;
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO usage_hourly_rollups
             (hour_utc, app_type, provider_id, model, billing_identity, data_source, request_count)
             VALUES (?1, 'claude', 'test-provider', 'test-model', '', 'proxy', 7)",
                [old_hour],
            )
            .unwrap();
        let state = crate::AppState::new_with_injections(
            db.clone(),
            crate::secrets::unavailable_credential_store(),
            Arc::new(crate::quota::QuotaService::new(db.clone())),
            injections,
        );
        let app = tauri::test::mock_app();
        app.manage(state);
        let from = (now - chrono::Duration::days(600)).date_naive().to_string();
        let to = now.date_naive().to_string();
        let query = |group: Option<&str>| {
            crate::api::commands::query_usage_trend(
                app.state(),
                "UTC".into(),
                from.clone(),
                to.clone(),
                "day".into(),
                group.map(str::to_string),
                None,
            )
        };
        let total = |r: &crate::usage::trend::UsageTrendResponse| {
            r.series
                .iter()
                .flat_map(|s| &s.points)
                .map(|p| p.requests)
                .sum::<i64>()
        };

        // Pro prune raises the high-water mark to 730.
        db.rollup_and_prune_configured().unwrap();
        // Downgrade: prune again under Free; history is kept but hidden.
        provider.0.store(false, Ordering::SeqCst);
        db.rollup_and_prune_configured().unwrap();
        assert_eq!(get_license_status(app.state()).tier, LicenseTier::Free);
        assert!(query(None).unwrap().clamped_from.is_some());
        assert_eq!(total(&query(None).unwrap()), 0);

        // High-water mark only rises.
        assert_eq!(db.raise_retention_high_water_days(400).unwrap(), 730);
        assert_eq!(db.raise_retention_high_water_days(0).unwrap(), 730);
        assert!(db.raise_retention_high_water_days(-1).is_err());
        db.rollup_and_prune_configured().unwrap();
        assert_eq!(
            db.get_setting("retention_high_water_days")
                .unwrap()
                .as_deref(),
            Some("730")
        );

        // Renewal: grouped and ungrouped queries see all 7 requests, unclamped.
        provider.0.store(true, Ordering::SeqCst);
        assert_eq!(get_license_status(app.state()).tier, LicenseTier::Pro);
        let grouped = query(Some("provider")).unwrap();
        assert_eq!(grouped.clamped_from, None);
        assert_eq!(total(&grouped), 7);
        let ungrouped = query(None).unwrap();
        assert_eq!(ungrouped.clamped_from, None);
        assert_eq!(total(&ungrouped), 7);
    }

    #[test]
    fn command_shell_matches_the_community_provider() {
        use tauri::Manager;
        let app = tauri::test::mock_app();
        app.manage(crate::AppState::new(std::sync::Arc::new(
            crate::Database::memory().unwrap(),
        )));
        let before = get_license_status(app.state());
        assert_eq!(before.tier, crate::policy::LicenseTier::Free);
        assert_eq!(before.build, crate::policy::LicenseBuild::Community);

        let token = "shell-token";
        let err = activate_license(app.state(), token.to_string()).expect_err("community");
        assert_eq!(err.code(), Some(crate::error::CODE_COMMUNITY_BUILD));
        assert!(!serde_json::to_string(&err).expect("json").contains(token));
        assert!(deactivate_license(app.state()).is_err());
        assert_eq!(get_license_status(app.state()), before);
    }
}
