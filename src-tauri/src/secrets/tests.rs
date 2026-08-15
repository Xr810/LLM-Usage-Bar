use super::{BindingCredentialService, CredentialStore, CredentialStoreError, SecretString};
use crate::error::AppError;
use crate::store::{CredentialMutationKind, Database};
use crate::usage::domain::{
    AgentModuleInput, AgentProviderBindingInput, AgentProviderBindingView, BindingCredentialStatus,
    SystemProviderAuthKind,
};
use crate::usage::system_providers::system_provider_definitions;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, Once, Weak};
use std::time::Duration;
use zeroize::Zeroizing;

#[derive(Default)]
struct MemoryCredentialStore {
    items: Mutex<HashMap<String, Zeroizing<Vec<u8>>>>,
    fail_next_delete: AtomicBool,
    fail_put_after_write: AtomicBool,
    block_next_put: AtomicBool,
    put_waiting: AtomicBool,
    put_release: (Mutex<bool>, Condvar),
    block_next_get: AtomicBool,
    get_waiting: AtomicBool,
    get_release: (Mutex<bool>, Condvar),
    get_calls: AtomicUsize,
    lock_probe: Mutex<Option<Weak<Database>>>,
}

impl CredentialStore for MemoryCredentialStore {
    fn put(&self, slot: &str, secret: &[u8]) -> Result<(), CredentialStoreError> {
        self.assert_database_unlocked();
        if self.block_next_put.swap(false, Ordering::SeqCst) {
            self.put_waiting.store(true, Ordering::SeqCst);
            let (released, condition) = &self.put_release;
            let mut released = released.lock().unwrap();
            while !*released {
                released = condition.wait(released).unwrap();
            }
        }
        self.items
            .lock()
            .unwrap()
            .insert(slot.to_string(), Zeroizing::new(secret.to_vec()));
        if self.fail_put_after_write.swap(false, Ordering::SeqCst) {
            return Err(CredentialStoreError::OperationFailed);
        }
        Ok(())
    }

    fn get(&self, slot: &str) -> Result<Option<Vec<u8>>, CredentialStoreError> {
        self.assert_database_unlocked();
        self.get_calls.fetch_add(1, Ordering::SeqCst);
        if self.block_next_get.swap(false, Ordering::SeqCst) {
            self.get_waiting.store(true, Ordering::SeqCst);
            let (released, condition) = &self.get_release;
            let mut released = released.lock().unwrap();
            while !*released {
                released = condition.wait(released).unwrap();
            }
        }
        Ok(self
            .items
            .lock()
            .unwrap()
            .get(slot)
            .map(|value| value.to_vec()))
    }

    fn delete(&self, slot: &str) -> Result<(), CredentialStoreError> {
        self.assert_database_unlocked();
        if self.fail_next_delete.swap(false, Ordering::SeqCst) {
            return Err(CredentialStoreError::OperationFailed);
        }
        self.items.lock().unwrap().remove(slot);
        Ok(())
    }
}

impl MemoryCredentialStore {
    fn probe_database_lock(&self, db: &Arc<Database>) {
        *self.lock_probe.lock().unwrap() = Some(Arc::downgrade(db));
    }

    fn assert_database_unlocked(&self) {
        let database = self
            .lock_probe
            .lock()
            .unwrap()
            .as_ref()
            .and_then(Weak::upgrade);
        if let Some(database) = database {
            assert!(
                database.conn.try_lock().is_ok(),
                "credential store I/O ran while the SQLite mutex was held"
            );
        }
    }

    fn fail_next_delete(&self) {
        self.fail_next_delete.store(true, Ordering::SeqCst);
    }

    fn fail_put_after_write(&self) {
        self.fail_put_after_write.store(true, Ordering::SeqCst);
    }

    fn block_next_put(&self) {
        *self.put_release.0.lock().unwrap() = false;
        self.put_waiting.store(false, Ordering::SeqCst);
        self.block_next_put.store(true, Ordering::SeqCst);
    }

    fn put_is_waiting(&self) -> bool {
        self.put_waiting.load(Ordering::SeqCst)
    }

    fn release_put(&self) {
        *self.put_release.0.lock().unwrap() = true;
        self.put_release.1.notify_all();
    }

    fn get_call_count(&self) -> usize {
        self.get_calls.load(Ordering::SeqCst)
    }

    fn remove(&self, slot: &str) {
        self.items.lock().unwrap().remove(slot);
    }

    fn replace_for_test(&self, slot: &str, value: &[u8]) {
        self.items
            .lock()
            .unwrap()
            .insert(slot.to_string(), Zeroizing::new(value.to_vec()));
    }

    fn item_count(&self) -> usize {
        self.items.lock().unwrap().len()
    }
}

fn secret(value: &str) -> SecretString {
    let value = if value.len() < 16 {
        format!("{value}-test-credential")
    } else {
        value.to_string()
    };
    SecretString::new(value)
}

fn direct_binding(db: &Database, provider_id: &str) -> AgentProviderBindingView {
    direct_binding_for_agent(db, provider_id, "codex")
}

fn direct_binding_for_agent(
    db: &Database,
    provider_id: &str,
    agent_module_id: &str,
) -> AgentProviderBindingView {
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO usage_providers (
                 id, name, billing_kind, product_group_id, token_sources,
                 route_app_type, route_config, enabled, needs_review,
                 created_at, updated_at
             ) VALUES (?1, ?1, 'metered', 'test', '[\"proxy\"]',
                       'claude', '{\"baseUrl\":\"https://api.example\"}',
                       1, 0, 10, 10)",
            [provider_id],
        )
        .unwrap();
    }
    db.save_agent_provider_binding(&AgentProviderBindingInput {
        id: None,
        agent_module_id: agent_module_id.to_string(),
        provider_id: provider_id.to_string(),
        enabled: false,
    })
    .unwrap()
}

fn private_binding_state(
    db: &Database,
    binding_id: &str,
) -> (Option<Vec<u8>>, Option<String>, i64, bool) {
    let conn = db.conn.lock().unwrap();
    conn.query_row(
        "SELECT api_key_fingerprint, credential_slot, credential_version, enabled
         FROM agent_provider_bindings WHERE id = ?1",
        [binding_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )
    .unwrap()
}

fn journal_count(db: &Database, binding_id: &str) -> i64 {
    let conn = db.conn.lock().unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM agent_credential_operations WHERE binding_id = ?1",
        [binding_id],
        |row| row.get(0),
    )
    .unwrap()
}

struct CapturingLogger;

static CAPTURED_LOGS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static CAPTURING_LOGGER: CapturingLogger = CapturingLogger;
static LOGGER_INIT: Once = Once::new();
static LOG_CAPTURE_ENABLED: AtomicBool = AtomicBool::new(false);

impl log::Log for CapturingLogger {
    fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        if LOG_CAPTURE_ENABLED.load(Ordering::SeqCst) {
            CAPTURED_LOGS
                .lock()
                .unwrap()
                .push(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

fn start_log_capture() {
    LOGGER_INIT.call_once(|| {
        log::set_logger(&CAPTURING_LOGGER).expect("test logger should initialize once");
        log::set_max_level(log::LevelFilter::Trace);
    });
    CAPTURED_LOGS.lock().unwrap().clear();
    LOG_CAPTURE_ENABLED.store(true, Ordering::SeqCst);
}

fn finish_log_capture() -> String {
    LOG_CAPTURE_ENABLED.store(false, Ordering::SeqCst);
    CAPTURED_LOGS.lock().unwrap().join("\n")
}

#[test]
fn secret_input_debug_is_redacted_and_has_no_serialize_surface() {
    let input: SecretString = serde_json::from_str("\"super-secret-sentinel\"").unwrap();
    assert_eq!(format!("{input:?}"), "SecretString([REDACTED])");
}

#[tokio::test]
async fn set_is_not_replace_and_a_second_set_conflicts() {
    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, "direct");
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db, store);

    let configured = service
        .set_binding_api_key(&binding.id, 0, secret("first-key-sentinel"))
        .await
        .unwrap();
    assert_eq!(configured.credential_version, 1);
    assert_eq!(
        configured.credential_status,
        BindingCredentialStatus::Configured
    );

    let error = service
        .set_binding_api_key(&binding.id, 1, secret("second-key-sentinel"))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "credential_conflict");
}

#[tokio::test]
async fn short_or_low_diversity_binding_credentials_are_rejected() {
    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, "credential-strength-direct");
    let service = BindingCredentialService::new(db, Arc::new(MemoryCredentialStore::default()));

    for rejected in ["200", "aaaaaaaaaaaaaaaa", "contains whitespace"] {
        let error = service
            .set_binding_api_key(&binding.id, 0, SecretString::new(rejected.to_string()))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "credential_required");
    }
}

#[tokio::test]
async fn protected_store_io_never_runs_under_the_sqlite_mutex() {
    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, "lock-probe-direct");
    let store = Arc::new(MemoryCredentialStore::default());
    store.probe_database_lock(&db);
    let service = BindingCredentialService::new(db, store);
    service
        .set_binding_api_key(&binding.id, 0, secret("lock-probe-one"))
        .await
        .unwrap();
    service
        .list_agent_provider_bindings(Some("codex"))
        .await
        .unwrap();
    service
        .replace_binding_api_key(&binding.id, 1, secret("lock-probe-two"))
        .await
        .unwrap();
    service.clear_binding_api_key(&binding.id, 2).await.unwrap();
}

#[tokio::test]
async fn duplicate_key_publish_rolls_back_only_its_staging_generation() {
    let db = Arc::new(Database::memory().unwrap());
    let first = direct_binding(&db, "duplicate-one");
    let second = direct_binding(&db, "duplicate-two");
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store.clone());

    service
        .set_binding_api_key(&first.id, 0, secret("duplicate-key-sentinel"))
        .await
        .unwrap();
    let error = service
        .set_binding_api_key(&second.id, 0, secret("duplicate-key-sentinel"))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "credential_conflict");
    assert_eq!(
        private_binding_state(&db, &second.id),
        (None, None, 0, false)
    );
    assert_eq!(journal_count(&db, &second.id), 0);
    assert_eq!(store.item_count(), 1);
}

#[tokio::test]
async fn concurrent_rotation_has_one_winner_and_monotonic_generation() {
    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, "concurrent-direct");
    let store = Arc::new(MemoryCredentialStore::default());
    let service = Arc::new(BindingCredentialService::new(db.clone(), store.clone()));
    service
        .set_binding_api_key(&binding.id, 0, secret("concurrent-original"))
        .await
        .unwrap();

    let left_service = service.clone();
    let left_id = binding.id.clone();
    let right_service = service.clone();
    let right_id = binding.id.clone();
    let (left, right) = tokio::join!(
        async move {
            left_service
                .replace_binding_api_key(&left_id, 1, secret("concurrent-left"))
                .await
        },
        async move {
            right_service
                .replace_binding_api_key(&right_id, 1, secret("concurrent-right"))
                .await
        }
    );
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    let loser = if let Err(error) = left {
        error
    } else {
        right.unwrap_err()
    };
    assert_eq!(loser.to_string(), "credential_conflict");
    assert_eq!(private_binding_state(&db, &binding.id).2, 2);
    assert_eq!(journal_count(&db, &binding.id), 0);
    assert_eq!(store.item_count(), 1);
}

#[tokio::test]
async fn a_cleared_binding_can_be_set_again_without_reusing_a_generation() {
    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, "reset-direct");
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db, store);
    service
        .set_binding_api_key(&binding.id, 0, secret("generation-one"))
        .await
        .unwrap();
    let cleared = service.clear_binding_api_key(&binding.id, 1).await.unwrap();
    assert_eq!(cleared.credential_version, 2);
    let reset = service
        .set_binding_api_key(&binding.id, 2, secret("generation-three"))
        .await
        .unwrap();
    assert_eq!(reset.credential_version, 3);
}

#[tokio::test]
async fn startup_reconciliation_removes_a_reserved_but_unpublished_staging_item() {
    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, "reconcile-direct");
    let store = Arc::new(MemoryCredentialStore::default());
    let reservation = db
        .reserve_credential_operation(
            &binding.id,
            0,
            CredentialMutationKind::Set,
            Some(&[7_u8; 32]),
        )
        .unwrap();
    let slot = reservation.staging_slot.as_deref().unwrap();
    store.put(slot, b"unpublished-key").unwrap();
    assert_eq!(journal_count(&db, &binding.id), 1);

    let service = BindingCredentialService::new(db.clone(), store.clone());
    service.reconcile_startup().await.unwrap();
    assert_eq!(journal_count(&db, &binding.id), 0);
    assert_eq!(store.item_count(), 0);
    assert_eq!(
        private_binding_state(&db, &binding.id),
        (None, None, 0, false)
    );
}

#[tokio::test]
async fn ambiguous_put_failure_cleans_the_written_staging_item_and_journal() {
    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, "put-failure-direct");
    let store = Arc::new(MemoryCredentialStore::default());
    store.fail_put_after_write();
    let service = BindingCredentialService::new(db.clone(), store.clone());

    let error = service
        .set_binding_api_key(&binding.id, 0, secret("put-failure-key"))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "credential_unavailable");
    assert_eq!(
        private_binding_state(&db, &binding.id),
        (None, None, 0, false)
    );
    assert_eq!(journal_count(&db, &binding.id), 0);
    assert_eq!(store.item_count(), 0);
}

#[tokio::test]
async fn failed_staging_delete_stays_journaled_until_reconciliation() {
    let db = Arc::new(Database::memory().unwrap());
    let first = direct_binding(&db, "staging-cleanup-one");
    let second = direct_binding(&db, "staging-cleanup-two");
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store.clone());
    service
        .set_binding_api_key(&first.id, 0, secret("staging-duplicate-key"))
        .await
        .unwrap();
    store.fail_next_delete();

    assert_eq!(
        service
            .set_binding_api_key(&second.id, 0, secret("staging-duplicate-key"))
            .await
            .unwrap_err()
            .to_string(),
        "credential_conflict"
    );
    assert_eq!(journal_count(&db, &second.id), 1);
    assert_eq!(store.item_count(), 2);

    service.reconcile_startup().await.unwrap();
    assert_eq!(journal_count(&db, &second.id), 0);
    assert_eq!(store.item_count(), 1);
    assert_eq!(
        private_binding_state(&db, &second.id),
        (None, None, 0, false)
    );
}

#[tokio::test]
async fn failed_delete_cannot_be_revived_and_reconciliation_finishes_it() {
    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, "delete-direct");
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store.clone());
    service
        .set_binding_api_key(&binding.id, 0, secret("delete-key-sentinel"))
        .await
        .unwrap();
    store.fail_next_delete();

    assert_eq!(
        service
            .delete_binding(&binding.id, 1)
            .await
            .unwrap_err()
            .to_string(),
        "credential_unavailable"
    );
    assert_eq!(
        private_binding_state(&db, &binding.id),
        (None, None, 2, false)
    );
    assert_eq!(journal_count(&db, &binding.id), 1);
    assert_eq!(
        service
            .set_binding_api_key(&binding.id, 2, secret("must-not-revive"))
            .await
            .unwrap_err()
            .to_string(),
        "credential_conflict"
    );

    service.reconcile_startup().await.unwrap();
    assert!(db
        .credential_binding_snapshot(&binding.id)
        .unwrap()
        .is_none());
    assert_eq!(store.item_count(), 0);
}

#[tokio::test]
async fn reconciliation_finishes_stacked_cleanup_generations_in_one_run() {
    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, "stacked-cleanup-direct");
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store.clone());
    service
        .set_binding_api_key(&binding.id, 0, secret("stacked-generation-one"))
        .await
        .unwrap();

    store.fail_next_delete();
    service
        .replace_binding_api_key(&binding.id, 1, secret("stacked-generation-two"))
        .await
        .unwrap();
    assert_eq!(journal_count(&db, &binding.id), 1);
    assert_eq!(store.item_count(), 2);

    store.fail_next_delete();
    assert_eq!(
        service
            .delete_binding(&binding.id, 2)
            .await
            .unwrap_err()
            .to_string(),
        "credential_unavailable"
    );
    assert_eq!(journal_count(&db, &binding.id), 2);

    service.reconcile_startup().await.unwrap();
    assert_eq!(store.item_count(), 0);
    assert!(db
        .credential_binding_snapshot(&binding.id)
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn archived_agent_allows_cleanup_but_rejects_new_secret_material() {
    let db = Arc::new(Database::memory().unwrap());
    let agent = db
        .save_agent_module(&AgentModuleInput {
            id: None,
            name: "Disposable Agent".to_string(),
            sort_order: 50,
            visible: true,
        })
        .unwrap();
    let binding = direct_binding_for_agent(&db, "archived-direct", &agent.id);
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store);
    service
        .set_binding_api_key(&binding.id, 0, secret("archived-key"))
        .await
        .unwrap();
    db.delete_agent_module(&agent.id).unwrap();

    assert_eq!(
        service
            .replace_binding_api_key(&binding.id, 1, secret("archive-replace"))
            .await
            .unwrap_err()
            .to_string(),
        "invalid_binding"
    );
    let cleared = service.clear_binding_api_key(&binding.id, 1).await.unwrap();
    assert_eq!(cleared.credential_version, 2);
    assert_eq!(cleared.credential_status, BindingCredentialStatus::Missing);
}

#[tokio::test]
async fn session_and_managed_auth_bindings_reject_key_mutation() {
    let db = Arc::new(Database::memory().unwrap());
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO usage_providers (
                 id, name, billing_kind, product_group_id, token_sources,
                 enabled, needs_review, created_at, updated_at
             ) VALUES ('session-only', 'Session', 'subscription', 'test',
                       '[\"session_log\"]', 1, 0, 10, 10)",
            [],
        )
        .unwrap();
    }
    let binding = db
        .save_agent_provider_binding(&AgentProviderBindingInput {
            id: None,
            agent_module_id: "claude-code".to_string(),
            provider_id: "session-only".to_string(),
            enabled: true,
        })
        .unwrap();
    let service = BindingCredentialService::new(db, Arc::new(MemoryCredentialStore::default()));
    assert_eq!(
        service
            .set_binding_api_key(&binding.id, 0, secret("unsupported-key"))
            .await
            .unwrap_err()
            .to_string(),
        "unsupported_auth"
    );
}

#[tokio::test]
async fn credential_can_be_cleared_after_provider_switches_to_session_auth() {
    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, "auth-change-direct");
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store.clone());
    service
        .set_binding_api_key(&binding.id, 0, secret("auth-change-key"))
        .await
        .unwrap();
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE usage_providers
             SET billing_kind = 'subscription', token_sources = '[\"session_log\"]',
                 route_app_type = NULL
             WHERE id = 'auth-change-direct'",
            [],
        )
        .unwrap();
    let before_clear = service
        .list_agent_provider_bindings(Some("codex"))
        .await
        .unwrap();
    let before_clear = before_clear
        .iter()
        .find(|view| view.id == binding.id)
        .unwrap();
    assert_eq!(
        before_clear.credential_status,
        BindingCredentialStatus::Unavailable
    );
    assert!(before_clear.can_clear_credential);

    let cleared = service.clear_binding_api_key(&binding.id, 1).await.unwrap();
    assert_eq!(cleared.credential_version, 2);
    assert!(!cleared.can_clear_credential);
    assert!(!cleared.enabled);
    assert_eq!(store.item_count(), 0);
    assert_eq!(journal_count(&db, &binding.id), 0);
}

#[tokio::test]
async fn malformed_provider_metadata_cannot_block_credential_cleanup() {
    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, "malformed-cleanup-direct");
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store.clone());
    service
        .set_binding_api_key(&binding.id, 0, secret("malformed-cleanup-key"))
        .await
        .unwrap();
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE usage_providers SET route_config = '{malformed'
             WHERE id = 'malformed-cleanup-direct'",
            [],
        )
        .unwrap();
    store.fail_next_delete();

    let cleared = service.clear_binding_api_key(&binding.id, 1).await.unwrap();
    assert_eq!(cleared.credential_version, 2);
    assert!(!cleared.effective_enabled);
    assert_eq!(
        cleared.credential_status,
        BindingCredentialStatus::Unavailable
    );
    assert!(!cleared.can_clear_credential);
    assert_eq!(store.item_count(), 1);
    assert_eq!(journal_count(&db, &binding.id), 1);
    service.reconcile_startup().await.unwrap();
    assert_eq!(store.item_count(), 0);
    assert_eq!(journal_count(&db, &binding.id), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn independent_database_connections_serialize_one_binding_generation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shared.db");
    let first_db = Arc::new(Database::init_at(&path).unwrap());
    let binding = direct_binding(&first_db, "cross-process-direct");
    let second_db = Arc::new(Database::init_at(&path).unwrap());
    let store = Arc::new(MemoryCredentialStore::default());
    let first_service = Arc::new(BindingCredentialService::new(
        first_db.clone(),
        store.clone(),
    ));
    let second_service = Arc::new(BindingCredentialService::new(second_db, store.clone()));
    first_service
        .set_binding_api_key(&binding.id, 0, secret("cross-process-original"))
        .await
        .unwrap();

    let left_id = binding.id.clone();
    let left = tokio::spawn(async move {
        first_service
            .replace_binding_api_key(&left_id, 1, secret("cross-process-left"))
            .await
    });
    let right_id = binding.id.clone();
    let right = tokio::spawn(async move {
        second_service
            .replace_binding_api_key(&right_id, 1, secret("cross-process-right"))
            .await
    });
    let left = left.await.unwrap();
    let right = right.await.unwrap();
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    assert_eq!(private_binding_state(&first_db, &binding.id).2, 2);
    assert_eq!(journal_count(&first_db, &binding.id), 0);
    assert_eq!(store.item_count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exclusive_reconciliation_waits_for_an_inflight_protected_store_write() {
    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, "lifecycle-barrier-direct");
    let store = Arc::new(MemoryCredentialStore::default());
    store.block_next_put();
    let service = Arc::new(BindingCredentialService::new(db.clone(), store.clone()));

    let mutation_service = service.clone();
    let binding_id = binding.id.clone();
    let mutation = tokio::spawn(async move {
        mutation_service
            .set_binding_api_key(&binding_id, 0, secret("barrier-key"))
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while !store.put_is_waiting() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("protected-store write should reach the test barrier");

    let reconciliation_service = service.clone();
    let reconciliation =
        tokio::spawn(async move { reconciliation_service.reconcile_startup().await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !reconciliation.is_finished(),
        "exclusive reconciliation must wait for the shared mutation lifecycle"
    );

    store.release_put();
    mutation.await.unwrap().unwrap();
    reconciliation.await.unwrap().unwrap();
    assert_eq!(journal_count(&db, &binding.id), 0);
    assert_eq!(store.item_count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_mutation_keeps_the_lifecycle_guard_until_store_put_finishes() {
    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, "cancelled-lifecycle-direct");
    let store = Arc::new(MemoryCredentialStore::default());
    store.block_next_put();
    let service = Arc::new(BindingCredentialService::new(db.clone(), store.clone()));

    let mutation_service = service.clone();
    let binding_id = binding.id.clone();
    let mutation = tokio::spawn(async move {
        mutation_service
            .set_binding_api_key(&binding_id, 0, secret("cancelled-barrier-key"))
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while !store.put_is_waiting() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("protected-store write should reach the cancellation barrier");

    mutation.abort();
    assert!(mutation.await.unwrap_err().is_cancelled());
    let reconciliation_service = service.clone();
    let reconciliation =
        tokio::spawn(async move { reconciliation_service.reconcile_startup().await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !reconciliation.is_finished(),
        "detached store I/O must retain the shared lifecycle guard"
    );

    store.release_put();
    reconciliation.await.unwrap().unwrap();
    assert_eq!(journal_count(&db, &binding.id), 0);
    assert_eq!(store.item_count(), 0);
    assert_eq!(
        private_binding_state(&db, &binding.id),
        (None, None, 0, false)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_blocking_database_change_retains_its_exclusive_guard() {
    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, "cancelled-database-change-direct");
    let store = Arc::new(MemoryCredentialStore::default());
    store.block_next_put();
    let service = Arc::new(BindingCredentialService::new(db.clone(), store.clone()));

    let operation_service = service.clone();
    let operation_store = store.clone();
    let operation = tokio::spawn(async move {
        operation_service
            .run_exclusive_blocking_database_change(move || {
                operation_store
                    .put("blocking-database-change", b"test-barrier")
                    .map_err(|_| AppError::Message("test_failure".to_string()))?;
                Ok(())
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while !store.put_is_waiting() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("blocking database operation should reach the test barrier");
    operation.abort();
    assert!(operation.await.unwrap_err().is_cancelled());

    let mutation_service = service.clone();
    let binding_id = binding.id.clone();
    let mutation = tokio::spawn(async move {
        mutation_service
            .set_binding_api_key(&binding_id, 0, secret("after-database-change"))
            .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !mutation.is_finished(),
        "shared credential mutation must wait for detached database replacement"
    );

    store.release_put();
    mutation.await.unwrap().unwrap();
    store.remove("blocking-database-change");
    assert_eq!(journal_count(&db, &binding.id), 0);
    assert_eq!(store.item_count(), 1);
}

#[tokio::test]
async fn sql_sync_and_binary_backups_never_contain_a_new_binding_key() {
    let directory = tempfile::tempdir().unwrap();
    let database_path = directory.path().join("credentials.db");
    let db = Arc::new(Database::init_at(&database_path).unwrap());
    let binding = direct_binding(&db, "backup-direct");
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store);
    let raw_key = "backup-raw-key-sentinel-9d9d";
    let view = service
        .set_binding_api_key(&binding.id, 0, secret(raw_key))
        .await
        .unwrap();

    let serialized = serde_json::to_string(&view).unwrap();
    let mut hasher = Sha256::new();
    hasher.update(super::KEYCHAIN_SERVICE.as_bytes());
    hasher.update(b"\0");
    hasher.update(raw_key.as_bytes());
    let fingerprint_hex = hex::encode(hasher.finalize());
    let full_sql = db.export_sql_string().unwrap();
    let sync_sql = db.export_sql_string_for_sync().unwrap();
    for surface in [&serialized, &full_sql, &sync_sql] {
        assert!(!surface.contains(raw_key));
    }
    for forbidden in [
        raw_key,
        "backup-raw-key",
        "sentinel-9d9d",
        fingerprint_hex.as_str(),
        "credentialSlot",
        "apiKeyFingerprint",
    ] {
        assert!(!serialized.contains(forbidden));
    }
    let backup = db.backup_database_file().unwrap().unwrap();
    let bytes = std::fs::read(backup).unwrap();
    assert!(!bytes
        .windows(raw_key.len())
        .any(|window| window == raw_key.as_bytes()));
}

#[tokio::test]
#[serial_test::serial]
async fn credential_failures_log_only_generic_messages() {
    start_log_capture();
    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, "log-redaction-direct");
    let store = Arc::new(MemoryCredentialStore::default());
    store.fail_put_after_write();
    let service = BindingCredentialService::new(db, store);
    let raw_key = "log-secret-prefix-4f71-log-secret-suffix";

    let error = service
        .set_binding_api_key(&binding.id, 0, secret(raw_key))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "credential_unavailable");

    let mut hasher = Sha256::new();
    hasher.update(super::KEYCHAIN_SERVICE.as_bytes());
    hasher.update(b"\0");
    hasher.update(raw_key.as_bytes());
    let fingerprint = hex::encode(hasher.finalize());
    let logs = finish_log_capture();
    for forbidden in [
        raw_key,
        "log-secret-prefix",
        "log-secret-suffix",
        fingerprint.as_str(),
        &fingerprint[..12],
        &fingerprint[fingerprint.len() - 12..],
    ] {
        assert!(
            !logs.contains(forbidden),
            "logs exposed credential material"
        );
    }
    assert!(logs.contains("credential store put failed"));
}

fn private_provider_credential_state(
    db: &Database,
    key_id: &str,
) -> (Option<Vec<u8>>, Option<String>, i64) {
    db.conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT api_key_fingerprint, credential_slot, credential_version
             FROM provider_api_keys WHERE id = ?1",
            [key_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
}

fn provider_credential_journal_count(db: &Database, key_id: &str) -> i64 {
    db.conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM provider_credential_operations WHERE key_id = ?1",
            [key_id],
            |row| row.get(0),
        )
        .unwrap()
}

fn create_provider_key(db: &Database, provider_id: &str) -> String {
    db.create_provider_api_key(provider_id, "Test key").unwrap()
}

fn all_binding_credential_state(db: &Database) -> Vec<(String, String, String, bool, i64)> {
    let conn = db.conn.lock().unwrap();
    let mut statement = conn
        .prepare(
            "SELECT id, agent_module_id, provider_id, enabled, credential_version
             FROM agent_provider_bindings ORDER BY id",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[tokio::test]
async fn provider_credential_set_replace_clear_is_versioned_and_redacted() {
    let db = Arc::new(Database::memory().unwrap());
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store.clone());
    let key_id = create_provider_key(&db, "system-openrouter-api");
    let first_key = "provider-first-secret-sentinel";
    let second_key = "provider-second-secret-sentinel";

    let configured = service
        .set_provider_api_key(&key_id, 0, SecretString::new(first_key.to_string()))
        .await
        .unwrap();
    assert_eq!(configured.credential_version, 1);
    assert_eq!(
        configured.credential_status,
        BindingCredentialStatus::Configured
    );
    assert!(configured.can_clear_credential);
    let serialized = serde_json::to_string(&configured).unwrap();
    assert!(!serialized.contains(first_key));
    let first_slot = private_provider_credential_state(&db, &key_id).1.unwrap();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO provider_key_usage_snapshots (
                key_id, credential_version, usage_total_usd,
                fetched_at, created_at, updated_at
             ) VALUES (?1, 1, '1.5', 10, 10, 10)",
            [&key_id],
        )
        .unwrap();
    }

    let replaced = service
        .replace_provider_api_key(&key_id, 1, SecretString::new(second_key.to_string()))
        .await
        .unwrap();
    assert_eq!(replaced.credential_version, 2);
    assert_eq!(replaced.key_usage, None);
    assert_eq!(
        db.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM provider_key_usage_snapshots
                 WHERE key_id = ?1",
                [&key_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    let second_slot = private_provider_credential_state(&db, &key_id).1.unwrap();
    assert_ne!(first_slot, second_slot);
    assert_eq!(store.item_count(), 1);
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO provider_key_usage_snapshots (
                key_id, credential_version, usage_total_usd,
                fetched_at, created_at, updated_at
             ) VALUES (?1, 2, '2.5', 20, 20, 20)",
            [&key_id],
        )
        .unwrap();
    }

    let cleared = service.clear_provider_api_key(&key_id, 2).await.unwrap();
    assert_eq!(cleared.credential_version, 3);
    assert_eq!(cleared.key_usage, None);
    assert_eq!(cleared.credential_status, BindingCredentialStatus::Missing);
    assert!(!cleared.can_clear_credential);
    assert_eq!(
        private_provider_credential_state(&db, &key_id),
        (None, None, 3)
    );
    assert_eq!(provider_credential_journal_count(&db, &key_id), 0);
    assert_eq!(store.item_count(), 0);
    assert_eq!(
        db.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM provider_key_usage_snapshots
                 WHERE key_id = ?1",
                [&key_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn provider_credential_accepts_the_built_in_api_catalog_only() {
    let db = Arc::new(Database::memory().unwrap());
    let service =
        BindingCredentialService::new(db.clone(), Arc::new(MemoryCredentialStore::default()));

    let api_definitions = system_provider_definitions()
        .into_iter()
        .filter(|definition| definition.auth_kind == SystemProviderAuthKind::ProviderApiKey)
        .collect::<Vec<_>>();
    assert_eq!(api_definitions.len(), 18);
    for definition in api_definitions {
        let key_id = create_provider_key(&db, definition.id);
        let configured = service
            .set_provider_api_key(
                &key_id,
                0,
                secret(&format!("{}-provider-key", definition.preset_key)),
            )
            .await
            .unwrap();
        assert_eq!(configured.credential_version, 1);
    }

    for rejected in [
        "system-chatgpt-subscription",
        "system-claude-subscription",
        "missing-custom-provider",
    ] {
        assert_eq!(
            service
                .set_provider_api_key(rejected, 0, secret("provider-rejected-key"))
                .await
                .unwrap_err()
                .to_string(),
            "unsupported_auth"
        );
    }
}

#[tokio::test]
async fn provider_credential_rotation_never_changes_binding_generations_or_selection() {
    let db = Arc::new(Database::memory().unwrap());
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store);
    let bindings_before = all_binding_credential_state(&db);
    let key_id = create_provider_key(&db, "system-openrouter-api");

    service
        .set_provider_api_key(&key_id, 0, secret("provider-rotation-one"))
        .await
        .unwrap();
    service
        .replace_provider_api_key(&key_id, 1, secret("provider-rotation-two"))
        .await
        .unwrap();
    service.clear_provider_api_key(&key_id, 2).await.unwrap();

    assert_eq!(all_binding_credential_state(&db), bindings_before);
}

#[tokio::test]
async fn provider_credential_store_failure_and_concurrent_mutations_fail_closed() {
    let db = Arc::new(Database::memory().unwrap());
    let store = Arc::new(MemoryCredentialStore::default());
    store.fail_put_after_write();
    let service = Arc::new(BindingCredentialService::new(db.clone(), store.clone()));
    let key_id = create_provider_key(&db, "system-openrouter-api");

    assert_eq!(
        service
            .set_provider_api_key(&key_id, 0, secret("provider-store-failure"),)
            .await
            .unwrap_err()
            .to_string(),
        "credential_unavailable"
    );
    assert_eq!(
        private_provider_credential_state(&db, &key_id),
        (None, None, 0)
    );
    assert_eq!(provider_credential_journal_count(&db, &key_id), 0);
    assert_eq!(store.item_count(), 0);

    service
        .set_provider_api_key(&key_id, 0, secret("provider-concurrent-original"))
        .await
        .unwrap();
    let replacing = service.clone();
    let clearing = service.clone();
    let replacing_key_id = key_id.clone();
    let clearing_key_id = key_id.clone();
    let (replace_result, clear_result) = tokio::join!(
        async move {
            replacing
                .replace_provider_api_key(
                    &replacing_key_id,
                    1,
                    secret("provider-concurrent-replacement"),
                )
                .await
        },
        async move { clearing.clear_provider_api_key(&clearing_key_id, 1).await }
    );
    assert_eq!(
        usize::from(replace_result.is_ok()) + usize::from(clear_result.is_ok()),
        1
    );
    let loser = replace_result.err().or_else(|| clear_result.err()).unwrap();
    assert_eq!(loser.to_string(), "credential_conflict");
    assert_eq!(private_provider_credential_state(&db, &key_id).2, 2);
    assert_eq!(provider_credential_journal_count(&db, &key_id), 0);
}

#[tokio::test]
async fn provider_credential_startup_reconciliation_removes_unpublished_staging_items() {
    let db = Arc::new(Database::memory().unwrap());
    let store = Arc::new(MemoryCredentialStore::default());
    let key_id = create_provider_key(&db, "system-openrouter-api");
    let reservation = db
        .reserve_provider_credential_operation(
            &key_id,
            0,
            CredentialMutationKind::Set,
            Some(&[0x81_u8; 32]),
        )
        .unwrap();
    let staging_slot = reservation.staging_slot.as_deref().unwrap();
    store
        .put(staging_slot, b"provider-unpublished-secret")
        .unwrap();

    let service = BindingCredentialService::new(db.clone(), store.clone());
    service.reconcile_startup().await.unwrap();
    assert_eq!(provider_credential_journal_count(&db, &key_id), 0);
    assert_eq!(store.item_count(), 0);
    assert_eq!(
        private_provider_credential_state(&db, &key_id),
        (None, None, 0)
    );
}

#[tokio::test]
async fn provider_credential_missing_or_mismatched_protected_items_are_unavailable() {
    let db = Arc::new(Database::memory().unwrap());
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store.clone());
    let key_id = create_provider_key(&db, "system-openrouter-api");
    service
        .set_provider_api_key(&key_id, 0, secret("provider-protected-status"))
        .await
        .unwrap();
    let slot = private_provider_credential_state(&db, &key_id).1.unwrap();

    store.remove(&slot);
    let missing = service
        .list_usage_providers()
        .await
        .unwrap()
        .into_iter()
        .find(|provider| provider.id == "system-openrouter-api")
        .unwrap();
    let missing_key = missing
        .api_keys
        .iter()
        .find(|key| key.id == key_id)
        .unwrap();
    assert_eq!(
        missing_key.credential_status,
        BindingCredentialStatus::Unavailable
    );
    assert!(missing_key.can_clear_credential);

    store.replace_for_test(&slot, b"different-provider-protected-value");
    let mismatched = service
        .list_usage_providers()
        .await
        .unwrap()
        .into_iter()
        .find(|provider| provider.id == "system-openrouter-api")
        .unwrap();
    assert_eq!(
        mismatched
            .api_keys
            .iter()
            .find(|key| key.id == key_id)
            .unwrap()
            .credential_status,
        BindingCredentialStatus::Unavailable
    );
}

#[tokio::test]
async fn provider_credential_duplicate_publish_cleans_only_its_staging_generation() {
    let db = Arc::new(Database::memory().unwrap());
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store.clone());
    let duplicate = "provider-duplicate-secret-sentinel";
    let openrouter_key_id = create_provider_key(&db, "system-openrouter-api");
    let openai_key_id = create_provider_key(&db, "system-openai-api");
    service
        .set_provider_api_key(
            &openrouter_key_id,
            0,
            SecretString::new(duplicate.to_string()),
        )
        .await
        .unwrap();

    assert_eq!(
        service
            .set_provider_api_key(&openai_key_id, 0, SecretString::new(duplicate.to_string()),)
            .await
            .unwrap_err()
            .to_string(),
        "credential_conflict"
    );
    assert_eq!(
        private_provider_credential_state(&db, &openai_key_id),
        (None, None, 0)
    );
    assert_eq!(provider_credential_journal_count(&db, &openai_key_id), 0);
    assert_eq!(store.item_count(), 1);
}

#[tokio::test]
async fn provider_credential_startup_reconciliation_finishes_published_operations() {
    let db = Arc::new(Database::memory().unwrap());
    let store = Arc::new(MemoryCredentialStore::default());
    let raw_key = b"provider-published-before-crash";
    let key_id = create_provider_key(&db, "system-openrouter-api");
    let mut hasher = Sha256::new();
    hasher.update(b"com.xr810.llm-usage-bar.provider-upstream.v1\0");
    hasher.update(raw_key);
    let fingerprint: [u8; 32] = hasher.finalize().into();
    let reservation = db
        .reserve_provider_credential_operation(
            &key_id,
            0,
            CredentialMutationKind::Set,
            Some(&fingerprint),
        )
        .unwrap();
    let slot = reservation.staging_slot.as_deref().unwrap();
    store.put(slot, raw_key).unwrap();
    db.publish_provider_credential_operation(&reservation, Some(&fingerprint))
        .unwrap();
    assert_eq!(provider_credential_journal_count(&db, &key_id), 1);

    let service = BindingCredentialService::new(db.clone(), store.clone());
    service.reconcile_startup().await.unwrap();
    assert_eq!(provider_credential_journal_count(&db, &key_id), 0);
    assert_eq!(store.item_count(), 1);
    assert_eq!(
        service
            .list_usage_providers()
            .await
            .unwrap()
            .into_iter()
            .find(|provider| provider.id == "system-openrouter-api")
            .unwrap()
            .api_keys
            .into_iter()
            .find(|key| key.id == key_id)
            .unwrap()
            .credential_status,
        BindingCredentialStatus::Configured
    );
}

#[tokio::test]
async fn provider_credential_exports_and_backups_never_contain_raw_keys() {
    let directory = tempfile::tempdir().unwrap();
    let database_path = directory.path().join("provider-credentials.db");
    let db = Arc::new(Database::init_at(&database_path).unwrap());
    let service =
        BindingCredentialService::new(db.clone(), Arc::new(MemoryCredentialStore::default()));
    let raw_key = "provider-backup-raw-sentinel-36f8";
    let key_id = create_provider_key(&db, "system-openrouter-api");
    let view = service
        .set_provider_api_key(&key_id, 0, SecretString::new(raw_key.to_string()))
        .await
        .unwrap();

    let serialized = serde_json::to_string(&view).unwrap();
    let full_sql = db.export_sql_string().unwrap();
    let sync_sql = db.export_sql_string_for_sync().unwrap();
    for surface in [&serialized, &full_sql, &sync_sql] {
        assert!(!surface.contains(raw_key));
    }
    let backup = db.backup_database_file().unwrap().unwrap();
    let bytes = std::fs::read(backup).unwrap();
    assert!(!bytes
        .windows(raw_key.len())
        .any(|window| window == raw_key.as_bytes()));
}

#[tokio::test]
async fn provider_credential_active_slot_is_never_deleted_by_binding_reconciliation() {
    let db = Arc::new(Database::memory().unwrap());
    let pending = direct_binding(&db, "binding-journal-active-provider");
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store.clone());
    let key_id = create_provider_key(&db, "system-openrouter-api");
    service
        .set_provider_api_key(&key_id, 0, secret("provider-slot-must-survive"))
        .await
        .unwrap();
    let active_slot = private_provider_credential_state(&db, &key_id).1.unwrap();
    let reservation = db
        .reserve_credential_operation(
            &pending.id,
            0,
            CredentialMutationKind::Set,
            Some(&[0x92_u8; 32]),
        )
        .unwrap();
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE agent_credential_operations
             SET staging_slot = ?2 WHERE operation_id = ?1",
            rusqlite::params![reservation.operation_id, active_slot],
        )
        .unwrap();

    assert_eq!(
        service.reconcile_startup().await.unwrap_err().to_string(),
        "credential_unavailable"
    );
    assert_eq!(journal_count(&db, &pending.id), 1);
    assert_eq!(store.item_count(), 1);
    assert_eq!(
        service
            .list_usage_providers()
            .await
            .unwrap()
            .into_iter()
            .find(|provider| provider.id == "system-openrouter-api")
            .unwrap()
            .api_keys
            .into_iter()
            .find(|key| key.id == key_id)
            .unwrap()
            .credential_status,
        BindingCredentialStatus::Configured
    );
}

fn fixed_binding(
    db: &Database,
    agent_module_id: &str,
    provider_id: &str,
) -> AgentProviderBindingView {
    db.list_agent_provider_bindings(Some(agent_module_id))
        .unwrap()
        .into_iter()
        .find(|binding| binding.provider_id == provider_id)
        .unwrap()
}

#[tokio::test]
async fn local_binding_keys_are_generated_per_agent_and_require_provider_key_to_be_effective() {
    let db = Arc::new(Database::memory().unwrap());
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store.clone());
    let key_id = create_provider_key(&db, "system-openrouter-api");

    service.ensure_fixed_api_binding_local_keys().await.unwrap();
    let bindings = service.list_agent_provider_bindings(None).await.unwrap();
    let openrouter = bindings
        .iter()
        .filter(|binding| binding.provider_id == "system-openrouter-api")
        .collect::<Vec<_>>();
    assert_eq!(openrouter.len(), 3);
    assert!(openrouter.iter().all(|binding| {
        binding.local_credential_status == BindingCredentialStatus::Configured
            && binding.provider_credential_status == BindingCredentialStatus::Missing
            && !binding.effective_enabled
            && binding.credential_version == 1
    }));

    let mut revealed = Vec::new();
    for binding in &openrouter {
        let secret = service
            .reveal_local_binding_key(&binding.id, binding.credential_version)
            .await
            .unwrap();
        assert!(secret.local_key.starts_with("lub_"));
        assert_eq!(secret.local_key.len(), 69);
        assert_eq!(secret.binding_id, binding.id);
        assert_eq!(secret.credential_version, 1);
        assert_eq!(format!("{secret:?}"), "LocalBindingKeyReveal([REDACTED])");
        revealed.push(secret.local_key.clone());
    }
    revealed.sort();
    revealed.dedup();
    assert_eq!(revealed.len(), 3);

    service
        .set_provider_api_key(&key_id, 0, secret("openrouter-shared-upstream-key"))
        .await
        .unwrap();
    let bindings = service.list_agent_provider_bindings(None).await.unwrap();
    assert!(bindings
        .iter()
        .filter(|binding| binding.provider_id == "system-openrouter-api")
        .all(|binding| {
            binding.provider_credential_status == BindingCredentialStatus::Configured
                && binding.effective_enabled
        }));
    assert_eq!(store.item_count(), 4);

    service.clear_provider_api_key(&key_id, 1).await.unwrap();
    let bindings = service.list_agent_provider_bindings(None).await.unwrap();
    assert!(bindings
        .iter()
        .filter(|binding| binding.provider_id == "system-openrouter-api")
        .all(|binding| {
            binding.local_credential_status == BindingCredentialStatus::Configured
                && binding.provider_credential_status == BindingCredentialStatus::Missing
                && binding.credential_version == 1
                && !binding.effective_enabled
        }));
    assert_eq!(store.item_count(), 3);
}

#[tokio::test]
async fn normal_startup_initializes_missing_items_without_reading_them() {
    let db = Arc::new(Database::memory().unwrap());
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store.clone());
    let missing_bindings = db
        .credential_binding_snapshots(None)
        .unwrap()
        .into_iter()
        .filter(|snapshot| snapshot.is_fixed_system_api())
        .collect::<Vec<_>>();

    assert!(!missing_bindings.is_empty());
    assert_eq!(store.get_call_count(), 0);

    service.reconcile_startup_journals().await.unwrap();
    service.initialize_startup_binding_keys().await.unwrap();

    assert_eq!(store.get_call_count(), 0);
    assert_eq!(store.items.lock().unwrap().len(), missing_bindings.len());
    for snapshot in db.credential_binding_snapshots(None).unwrap() {
        if snapshot.is_fixed_system_api() {
            assert!(snapshot.fingerprint.is_some());
            assert!(snapshot.credential_slot.is_some());
            assert_eq!(journal_count(&db, &snapshot.id), 0);
        }
    }

    service.reconcile_startup_journals().await.unwrap();
    service.initialize_startup_binding_keys().await.unwrap();

    assert_eq!(store.get_call_count(), 0);
    assert_eq!(store.items.lock().unwrap().len(), missing_bindings.len());
}

#[tokio::test]
async fn normal_startup_does_not_read_existing_protected_items() {
    let db = Arc::new(Database::memory().unwrap());
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db, store.clone());

    // Populate the fixed bindings once. Status-producing APIs may verify them
    // because they represent an explicit UI or maintenance read.
    service.ensure_fixed_api_binding_local_keys().await.unwrap();
    let reads_before_startup = store.get_call_count();
    assert!(reads_before_startup > 0);

    service.reconcile_startup_journals().await.unwrap();
    service.initialize_startup_binding_keys().await.unwrap();

    assert_eq!(store.get_call_count(), reads_before_startup);
}

#[tokio::test]
async fn local_binding_lists_and_setup_views_never_reveal_raw_keys() {
    let db = Arc::new(Database::memory().unwrap());
    let service =
        BindingCredentialService::new(db.clone(), Arc::new(MemoryCredentialStore::default()));
    let created = service
        .create_system_api_binding(AgentProviderBindingInput {
            id: None,
            agent_module_id: "codex".to_string(),
            provider_id: "system-openai-api".to_string(),
            enabled: true,
        })
        .await
        .unwrap();
    let reveal = service
        .reveal_local_binding_key(&created.id, 1)
        .await
        .unwrap();
    let raw = reveal.local_key.clone();

    let listed = service.list_agent_provider_bindings(None).await.unwrap();
    let serialized = serde_json::to_string(&listed).unwrap();
    assert!(!serialized.contains(&raw));
    assert!(!serialized.contains("lub_"));
    assert!(serialized.contains("localCredentialStatus"));
    assert!(serialized.contains("providerCredentialStatus"));
    let provider = service
        .list_usage_providers()
        .await
        .unwrap()
        .into_iter()
        .find(|provider| provider.id == "system-openai-api")
        .unwrap();
    assert_eq!(provider.bindings.len(), 1);
    assert_eq!(
        provider.bindings[0].local_credential_status,
        BindingCredentialStatus::Configured
    );
    assert!(!serde_json::to_string(&provider).unwrap().contains(&raw));
}

#[tokio::test]
async fn local_binding_rejects_unsupported_pairs_and_subscription_key_reveal() {
    let db = Arc::new(Database::memory().unwrap());
    let service =
        BindingCredentialService::new(db.clone(), Arc::new(MemoryCredentialStore::default()));
    assert_eq!(
        service
            .create_system_api_binding(AgentProviderBindingInput {
                id: None,
                agent_module_id: "claude-code".to_string(),
                provider_id: "system-openai-api".to_string(),
                enabled: true,
            })
            .await
            .unwrap_err()
            .to_string(),
        "invalid_binding"
    );

    let claude = fixed_binding(&db, "claude-code", "system-claude-subscription");
    assert_eq!(claude.route_protocol, None);
    assert_eq!(
        service
            .reveal_local_binding_key(&claude.id, claude.credential_version)
            .await
            .unwrap_err()
            .to_string(),
        "unsupported_auth"
    );
}

#[tokio::test]
async fn local_binding_create_store_failure_leaves_no_enabled_or_orphaned_binding() {
    let db = Arc::new(Database::memory().unwrap());
    let store = Arc::new(MemoryCredentialStore::default());
    store.fail_put_after_write();
    let service = BindingCredentialService::new(db.clone(), store.clone());

    assert_eq!(
        service
            .create_system_api_binding(AgentProviderBindingInput {
                id: None,
                agent_module_id: "codex".to_string(),
                provider_id: "system-openai-api".to_string(),
                enabled: true,
            })
            .await
            .unwrap_err()
            .to_string(),
        "credential_unavailable"
    );
    assert!(db
        .list_agent_provider_bindings(Some("codex"))
        .unwrap()
        .into_iter()
        .all(|binding| binding.provider_id != "system-openai-api"));
    assert_eq!(store.item_count(), 0);
}

#[tokio::test]
async fn local_binding_startup_generation_failure_preserves_selection_and_reports_unavailable() {
    let db = Arc::new(Database::memory().unwrap());
    let service = BindingCredentialService::new(db.clone(), super::unavailable_credential_store());

    let views = service.ensure_fixed_api_binding_local_keys().await.unwrap();
    let openrouter = views
        .iter()
        .filter(|binding| binding.provider_id == "system-openrouter-api")
        .collect::<Vec<_>>();
    assert_eq!(openrouter.len(), 3);
    assert!(openrouter.iter().all(|binding| {
        binding.enabled
            && !binding.effective_enabled
            && binding.local_credential_status == BindingCredentialStatus::Unavailable
    }));
    assert!(all_binding_credential_state(&db)
        .into_iter()
        .filter(|(_, _, provider_id, _, _)| provider_id == "system-openrouter-api")
        .all(|(_, _, _, enabled, version)| enabled && version == 0));
}
