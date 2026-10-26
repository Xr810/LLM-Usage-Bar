use super::*;
use crate::policy::{CommunityLicenseProvider, RetentionStore, LICENSE_CHANGED_EVENT};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, Listener, Manager};

// No activation semantics or verification: a test-only provider exercises the
// public prerequisite in precisely the order required of build providers.
struct TestProvider {
    db: Arc<Database>,
    current: Mutex<LicenseStatus>,
}

impl TestProvider {
    fn new(db: Arc<Database>) -> Self {
        let mut status = LicenseStatus::community_free();
        status.build = LicenseBuild::Official;
        Self {
            db,
            current: Mutex::new(status),
        }
    }
}

impl LicenseProvider for TestProvider {
    fn status(&self) -> LicenseStatus {
        self.current.lock().unwrap().clone()
    }
    fn activate(&self, _: &str) -> Result<LicenseStatus, AppError> {
        RetentionStore::new(self.db.clone()).set_pending_target(730)?;
        assert_eq!(
            self.db.get_setting("retention_high_water_days")?.as_deref(),
            Some("730")
        );
        let mut current = self.current.lock().unwrap();
        current.tier = LicenseTier::Pro;
        current.state = LicenseState::Active;
        current.expires_at = Some("2090-01-01T00:00:00Z".into());
        Ok(current.clone())
    }
    fn deactivate(&self) -> Result<(), AppError> {
        let mut status = LicenseStatus::community_free();
        status.build = LicenseBuild::Official;
        *self.current.lock().unwrap() = status;
        Ok(())
    }
}

fn at(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

// A regression must fail promptly even if a callback blocks on a mutex. The
// timeout is a test failure, not a production escape hatch or skipped assertion.
fn bounded(work: impl FnOnce() + Send + 'static) {
    let (done, completed) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        work();
        done.send(()).unwrap();
    });
    completed
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("license operation deadlocked or panicked");
    worker.join().unwrap();
}

#[test]
fn publisher_can_read_database_and_reenter_status_activate_and_deactivate() {
    bounded(|| {
        let db = Arc::new(Database::memory().unwrap());
        let provider = Arc::new(TestProvider::new(db.clone()));
        let lifecycle = Arc::new(LicenseLifecycle::new(provider.clone(), db.clone()));
        let competing = Arc::new(LicenseLifecycle::new(provider, db.clone()));
        let current = Arc::downgrade(&lifecycle);
        let other = Arc::downgrade(&competing);
        let events = Arc::new(Mutex::new(Vec::new()));
        let recorded = events.clone();
        let publish: LicensePublisher = Arc::new(move |status| {
            assert_eq!(
                db.get_setting("retention_high_water_days")?.as_deref(),
                Some("730")
            );
            let lifecycle = current.upgrade().unwrap();
            let competing = other.upgrade().unwrap();
            assert_eq!(lifecycle.status().tier, status.tier);
            assert_eq!(competing.status().tier, status.tier);
            let count = {
                let mut recorded = recorded.lock().unwrap();
                recorded.push(status.state);
                recorded.len()
            };
            match count {
                1 => competing.deactivate()?,
                2 => {
                    lifecycle.activate("test")?;
                }
                _ => {}
            }
            Ok(())
        });
        lifecycle.set_publisher(publish.clone());
        competing.set_publisher(publish);
        lifecycle.activate("test").unwrap();
        lifecycle.reconcile(Utc::now()).unwrap();
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                LicenseState::Active,
                LicenseState::Free,
                LicenseState::Active
            ]
        );
        assert_eq!(lifecycle.status().tier, LicenseTier::Pro);
    });
}

#[test]
fn callback_cannot_overwrite_an_acknowledgement_changed_during_delivery() {
    bounded(|| {
        for erase in [false, true] {
            let db = Arc::new(Database::memory().unwrap());
            let provider = Arc::new(TestProvider::new(db.clone()));
            let lifecycle = LicenseLifecycle::new(provider, db.clone());
            let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let calls = attempts.clone();
            let callback_db = db.clone();
            if erase {
                db.set_setting("license_last_emitted_state", "previous")
                    .unwrap();
            }
            lifecycle.set_publisher(Arc::new(move |_| {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    if erase {
                        callback_db.conn.lock()?.execute(
                            "DELETE FROM settings WHERE key='license_last_emitted_state'",
                            [],
                        )?;
                    } else {
                        callback_db
                            .set_setting("license_last_emitted_state", "changed-during-emit")?;
                    }
                }
                Ok(())
            }));
            lifecycle.activate("test").unwrap();
            assert_eq!(
                db.get_setting("license_last_emitted_state")
                    .unwrap()
                    .as_deref(),
                (!erase).then_some("changed-during-emit")
            );
            lifecycle.reconcile(Utc::now()).unwrap();
            lifecycle.reconcile(Utc::now()).unwrap();
            assert_eq!(attempts.load(Ordering::SeqCst), 2);
            assert_eq!(lifecycle.status().tier, LicenseTier::Pro);
        }
    });
}

#[test]
fn commands_emit_full_tauri_payload_once_per_transition_and_community_never_emits() {
    for community in [false, true] {
        let db = Arc::new(Database::memory().unwrap());
        let provider: Arc<dyn LicenseProvider> = if community {
            Arc::new(CommunityLicenseProvider)
        } else {
            Arc::new(TestProvider::new(db.clone()))
        };
        let injections = crate::Injections {
            license: provider,
            ..crate::Injections::community()
        };
        let state = crate::AppState::new_with_injections(
            db.clone(),
            crate::secrets::unavailable_credential_store(),
            Arc::new(crate::quota::QuotaService::new(db)),
            &injections,
        );
        let app = tauri::test::mock_app();
        let events = Arc::new(Mutex::new(Vec::<LicenseStatus>::new()));
        let captured = events.clone();
        app.listen(LICENSE_CHANGED_EVENT, move |event| {
            captured
                .lock()
                .unwrap()
                .push(serde_json::from_str(event.payload()).unwrap());
        });
        let handle = app.handle().clone();
        state
            .license_lifecycle
            .set_publisher(Arc::new(move |status| {
                handle
                    .emit(LICENSE_CHANGED_EVENT, status)
                    .map_err(|_| AppError::internal("Test emit failed."))
            }));
        app.manage(state);
        for _ in 0..2 {
            let activated =
                crate::api::commands::activate_license(app.state(), "test-token".into());
            if community {
                assert_eq!(activated.unwrap_err().code(), Some("COMMUNITY_BUILD"));
            } else {
                assert_eq!(activated.unwrap().tier, LicenseTier::Pro);
            }
        }
        for _ in 0..2 {
            let result = crate::api::commands::deactivate_license(app.state());
            if community {
                assert_eq!(result.unwrap_err().code(), Some("COMMUNITY_BUILD"));
            } else {
                result.unwrap();
            }
        }
        app.state::<crate::AppState>()
            .license_lifecycle
            .reconcile(Utc::now())
            .unwrap();
        let events = events.lock().unwrap();
        if community {
            assert!(events.is_empty());
        } else {
            assert_eq!(events.len(), 2);
            assert_eq!(events[0].state, LicenseState::Active);
            assert_eq!(
                events[0].grace_until.as_deref(),
                Some("2090-01-15T00:00:00Z")
            );
            assert_eq!(events[1].state, LicenseState::Free);
        }
    }
}

#[test]
fn grace_boundaries_concurrent_ticks_key_changes_and_restart_deduplicate() {
    bounded(|| {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("license.db");
        let db = Arc::new(Database::init_at(&path).unwrap());
        let provider = Arc::new(TestProvider::new(db.clone()));
        provider.activate("test").unwrap();
        let lifecycle = LicenseLifecycle::new(provider.clone(), db.clone());
        let events = Arc::new(Mutex::new(Vec::new()));
        let recorded = events.clone();
        let publish: LicensePublisher = Arc::new(move |s| {
            recorded.lock().unwrap().push(s.clone());
            Ok(())
        });
        lifecycle.set_publisher(publish.clone());
        lifecycle.reconcile(at("2089-12-31T23:59:59Z")).unwrap();
        let competing = LicenseLifecycle::new(provider.clone(), db.clone());
        competing.set_publisher(publish.clone());
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| lifecycle.reconcile(at("2090-01-01T00:00:00Z")).unwrap());
                scope.spawn(|| competing.reconcile(at("2090-01-01T00:00:00Z")).unwrap());
            }
        });
        lifecycle.reconcile(at("2090-01-14T23:59:59Z")).unwrap();
        lifecycle.reconcile(at("2090-01-15T00:00:00Z")).unwrap();
        lifecycle.reconcile(at("2090-01-15T00:00:01Z")).unwrap();
        {
            let events = events.lock().unwrap();
            assert_eq!(
                events.iter().map(|s| s.state).collect::<Vec<_>>(),
                vec![
                    LicenseState::Active,
                    LicenseState::Grace,
                    LicenseState::Free
                ]
            );
            assert_eq!(events[1].tier, LicenseTier::Pro);
            assert_eq!(events[2].tier, LicenseTier::Free);
        }
        drop(competing);
        drop(lifecycle);
        drop(provider);
        drop(db);
        let db = Arc::new(Database::init_at(&path).unwrap());
        let provider = Arc::new(TestProvider::new(db.clone()));
        provider.activate("test").unwrap();
        let restarted = LicenseLifecycle::new(provider.clone(), db.clone());
        restarted.set_publisher(publish);
        restarted.reconcile(at("2090-01-15T00:00:01Z")).unwrap();
        assert_eq!(events.lock().unwrap().len(), 3);
        // Same lifecycle state but a renewed expiry is a new event.
        provider.current.lock().unwrap().expires_at = Some("2090-02-01T00:00:00Z".into());
        restarted.reconcile(at("2090-03-01T00:00:00Z")).unwrap();
        assert_eq!(events.lock().unwrap().len(), 4);
        assert_eq!(
            db.get_setting("retention_high_water_days")
                .unwrap()
                .as_deref(),
            Some("730")
        );
    });
}

#[test]
fn failed_emit_preserves_tier_and_retries_without_acknowledging() {
    let db = Arc::new(Database::memory().unwrap());
    let provider = Arc::new(TestProvider::new(db.clone()));
    let lifecycle = LicenseLifecycle::new(provider.clone(), db.clone());
    let fail = Arc::new(AtomicBool::new(true));
    let failure = fail.clone();
    let events = Arc::new(Mutex::new(Vec::new()));
    let recorded = events.clone();
    lifecycle.set_publisher(Arc::new(move |s| {
        if failure.load(Ordering::SeqCst) {
            return Err(AppError::internal("Test emit failure."));
        }
        recorded.lock().unwrap().push(s.clone());
        Ok(())
    }));
    assert_eq!(lifecycle.activate("test").unwrap().tier, LicenseTier::Pro);
    assert_eq!(provider.status().tier, LicenseTier::Pro);
    assert!(db
        .get_setting("license_last_emitted_state")
        .unwrap()
        .is_none());
    fail.store(false, Ordering::SeqCst);
    lifecycle.reconcile(Utc::now()).unwrap();
    lifecycle.reconcile(Utc::now()).unwrap();
    assert_eq!(events.lock().unwrap().len(), 1);
}

#[test]
fn retention_failures_keep_tier_and_startup_retries_even_in_community() {
    for stage in ["pending_record", "retention_high_water"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("retry.db");
        let db = Arc::new(Database::init_at(&path).unwrap());
        let key = if stage == "pending_record" {
            "retention_pending_target"
        } else {
            "retention_high_water_days"
        };
        db.conn.lock().unwrap().execute_batch(&format!(
            "CREATE TRIGGER fail_retention BEFORE INSERT ON settings WHEN NEW.key='{key}' BEGIN SELECT RAISE(ABORT, 'test disk failure'); END;"
        )).unwrap();
        let provider = TestProvider::new(db.clone());
        let before = provider.status();
        let error = provider.activate("test").unwrap_err();
        assert_eq!(
            serde_json::to_value(error).unwrap(),
            serde_json::json!({
                "code": "LICENSE_ACTIVATION_RETRYABLE",
                "message": "License activation could not persist retention. Please retry.",
                "details": { "stage": stage, "retryable": true }
            })
        );
        assert_eq!(provider.status(), before);
        assert_eq!(
            RetentionStore::new(db.clone()).pending_target(),
            (stage == "retention_high_water").then_some(730)
        );
        if stage == "retention_high_water" {
            // Startup must remain available but skip destructive cleanup while
            // the high-water write is still failing. Protect a 500-day row.
            let old_hour = (Utc::now() - Duration::days(500))
                .timestamp()
                .div_euclid(3600)
                * 3600;
            db.conn
                .lock()
                .unwrap()
                .execute(
                    "INSERT INTO usage_hourly_rollups
                 (hour_utc,app_type,provider_id,model,billing_identity,data_source,request_count)
                 VALUES (?1,'claude','test','model','','proxy',9)",
                    [old_hour],
                )
                .unwrap();
            assert!(db.rollup_and_prune_configured().is_err());
            let reopened = Arc::new(Database::init_at(&path).unwrap());
            let state = crate::AppState::new(reopened.clone());
            assert_eq!(state.license.status(), LicenseStatus::community_free());
            assert_eq!(
                RetentionStore::new(reopened.clone()).pending_target(),
                Some(730)
            );
            let count: i64 = reopened
                .conn
                .lock()
                .unwrap()
                .query_row(
                    "SELECT request_count FROM usage_hourly_rollups WHERE hour_utc=?1",
                    [old_hour],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 9);
        }
        db.conn
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_retention")
            .unwrap();
        drop(provider);
        drop(db);
        let db = Arc::new(Database::init_at(&path).unwrap());
        let state = crate::AppState::new(db.clone());
        assert_eq!(state.license.status(), LicenseStatus::community_free());
        assert_eq!(RetentionStore::new(db.clone()).pending_target(), None);
        if stage == "retention_high_water" {
            let count: i64 = db
                .conn
                .lock()
                .unwrap()
                .query_row(
                    "SELECT SUM(request_count) FROM usage_hourly_rollups WHERE provider_id='test'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 9);
            assert_eq!(
                db.get_setting("retention_high_water_days")
                    .unwrap()
                    .as_deref(),
                Some("730")
            );
            RetentionStore::new(db.clone())
                .set_pending_target(400)
                .unwrap();
            assert_eq!(
                db.get_setting("retention_high_water_days")
                    .unwrap()
                    .as_deref(),
                Some("730")
            );
        } else {
            let provider = TestProvider::new(db);
            assert_eq!(provider.activate("test").unwrap().tier, LicenseTier::Pro);
        }
    }
}
