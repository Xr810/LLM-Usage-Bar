use super::{BindingCredentialService, CredentialStore, CredentialStoreError, SecretString};
use crate::database::{CredentialMutationKind, Database};
use crate::error::AppError;
use crate::provider::{Provider, ProviderMeta};
use crate::proxy::provider_router::BindingPricingOverride;
use crate::usage::domain::{
    AgentModuleInput, AgentProviderBindingInput, AgentProviderBindingView, BindingCredentialStatus,
};
use serde_json::json;
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

#[test]
fn exposure_guard_detects_raw_and_url_encoded_credentials() {
    let guard = super::CredentialExposureGuard::from_secret(b"sk/test key+tail");

    assert!(guard.contains("prefix-sk/test key+tail-suffix"));
    assert!(guard.contains("trace=sk%2Ftest%20key%2Btail"));
    assert!(guard.contains("trace=sk%2Ftest+key%2Btail"));
    assert!(guard.contains("trace=sk%2525252Ftest%25252520key%2525252Btail"));
    assert!(guard.contains(r#"trace=sk\u002ftest\u0020key\u002btail"#));
    assert!(guard.contains_bytes(b"prefix-sk/test key+tail-suffix"));
    assert!(guard.contains_bytes(b"\x80trace=sk%2Ftest%20key%2Btail"));
    let mut suspicious_depth = "unrelated%2Fvalue".to_string();
    for _ in 0..17 {
        suspicious_depth = suspicious_depth.replace('%', "%25");
    }
    assert!(guard.contains(&suspicious_depth));
    assert!(!guard.contains("trace=unrelated"));
}

#[test]
fn exposure_stream_scanner_detects_chunk_split_raw_and_encoded_credentials() {
    let guard = super::CredentialExposureGuard::from_secret(b"stream/key+sentinel");

    let mut raw = guard.stream_scanner();
    assert!(!raw.push(b"prefix-stream/key"));
    assert!(raw.push(b"+sentinel-suffix"));

    let mut encoded = guard.stream_scanner();
    assert!(!encoded.push(b"trace=stream%252Fkey%252"));
    assert!(encoded.push(b"Bsentinel"));

    let mut json_escaped = guard.stream_scanner();
    assert!(!json_escaped.push(br#"data: {"id":"stream\u002fkey\u002bse"#));
    assert!(json_escaped.push(br#"ntinel"}"#));

    let mut safe = guard.stream_scanner();
    assert!(!safe.push(b"trace=stream%252Fother"));

    let json_escaped = b"stream/key+sentinel"
        .iter()
        .map(|byte| format!(r"\u{:04x}", byte))
        .collect::<String>();
    let percent_encoded_json = json_escaped
        .bytes()
        .map(|byte| format!("%{byte:02X}"))
        .collect::<String>();
    let mut composed = guard.stream_scanner();
    assert!(composed.push(percent_encoded_json.as_bytes()));

    // Exercise an alternating normalization chain rather than only a one-way
    // percent -> JSON composition: percent(JSON(percent(secret))).
    let percent_encoded_secret = b"stream/key+sentinel"
        .iter()
        .map(|byte| format!("%{byte:02X}"))
        .collect::<String>();
    let json_escaped_percent = percent_encoded_secret
        .bytes()
        .map(|byte| format!(r"\u{byte:04x}"))
        .collect::<String>();
    let alternating = json_escaped_percent
        .bytes()
        .map(|byte| format!("%{byte:02X}"))
        .collect::<String>();
    assert!(guard.contains_bytes(alternating.as_bytes()));

    let encode_all = |value: &str| {
        value
            .bytes()
            .map(|byte| format!("%{byte:02X}"))
            .collect::<String>()
    };
    let mut sixteen_layers = encode_all("unrelated-depth-value");
    for _ in 1..16 {
        sixteen_layers = sixteen_layers.replace('%', "%25");
    }
    let mut bounded = guard.stream_scanner();
    assert!(!bounded.push(sixteen_layers.as_bytes()));

    let seventeen_layers = sixteen_layers.replace('%', "%25");
    let mut excessive = guard.stream_scanner();
    assert!(excessive.push(seventeen_layers.as_bytes()));
}

#[test]
fn exposure_semantic_scanner_joins_only_matching_json_fields() {
    let guard = super::CredentialExposureGuard::from_secret(b"semantic/stream-key");
    let mut scanner = guard.semantic_stream_scanner();

    assert!(!scanner.push_json_value(&json!({
        "type": "content_block_delta",
        "delta": { "type": "text_delta", "text": "semantic/" }
    })));
    assert!(scanner.push_json_value(&json!({
        "type": "content_block_delta",
        "delta": { "type": "text_delta", "text": "stream-key" }
    })));

    let split_blocks = json!({
        "content": [
            { "type": "text", "text": "semantic/" },
            { "type": "text", "text": "stream-key" }
        ]
    });
    assert!(guard.contains_json_value(&split_blocks));

    let unrelated_fields = json!({
        "text": "semantic/",
        "model": "stream-key"
    });
    assert!(!guard.contains_json_value(&unrelated_fields));

    let mut partial = guard.semantic_stream_scanner();
    assert!(!partial.push_json_value(&json!({ "delta": "semantic/" })));
    assert!(partial.has_partial_match());
    assert!(!partial.push_json_value(&json!({ "delta": "definitely-safe" })));
    assert!(!partial.has_partial_match());

    let mut interleaved = guard.semantic_stream_scanner();
    assert!(!interleaved.push_json_value(&json!({
        "index": 0,
        "delta": { "text": "semantic/" }
    })));
    assert!(!interleaved.push_json_value(&json!({
        "index": 1,
        "delta": { "text": "safe" }
    })));
    assert!(interleaved.push_json_value(&json!({
        "index": 0,
        "delta": { "text": "stream-key" }
    })));

    let mut thinking = guard.semantic_stream_scanner();
    assert!(!thinking.push_json_value(&json!({ "delta": { "thinking": "semantic/" } })));
    assert!(thinking.push_json_value(&json!({ "delta": { "thinking": "stream-key" } })));

    let mut reasoning = guard.semantic_stream_scanner();
    assert!(!reasoning.push_json_value(&json!({
        "choices": [{ "index": 0, "delta": {
            "role": "assistant", "reasoning": "semantic/"
        }}]
    })));
    assert!(reasoning.push_json_value(&json!({
        "choices": [{ "index": 0, "delta": {
            "role": "assistant", "reasoning": "stream-key"
        }}]
    })));

    let percent_encoded = b"semantic/stream-key"
        .iter()
        .map(|byte| format!("%{byte:02X}"))
        .collect::<String>();
    assert!(guard.contains_json_value(&json!({
        "content": [
            { "text": "%" },
            { "text": &percent_encoded[1..] }
        ]
    })));

    let json_escaped = b"semantic/stream-key"
        .iter()
        .map(|byte| format!(r"\u{byte:04x}"))
        .collect::<String>();
    assert!(guard.contains_json_value(&json!({
        "content": [
            { "text": "\\" },
            { "text": &json_escaped[1..] }
        ]
    })));

    assert!(!guard.contains_json_value(&json!({
        "content": [{ "text": "x".repeat(300 * 1024) }]
    })));
    assert!(!guard.contains_json_value(&json!({
        "content": [{ "text": format!("{}s", "x".repeat(300 * 1024)) }]
    })));
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

fn requested_enabled(db: &Database, binding: &AgentProviderBindingView) {
    db.save_agent_provider_binding(&AgentProviderBindingInput {
        id: Some(binding.id.clone()),
        agent_module_id: binding.agent_module_id.clone(),
        provider_id: binding.provider_id.clone(),
        enabled: true,
    })
    .unwrap();
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
async fn replace_uses_a_new_slot_and_only_the_new_key_resolves() {
    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, "replace-direct");
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store.clone());
    let first = service
        .set_binding_api_key(&binding.id, 0, secret("first-rotation-key"))
        .await
        .unwrap();
    let old_slot = private_binding_state(&db, &binding.id).1.unwrap();
    requested_enabled(&db, &first);

    let second = service
        .replace_binding_api_key(&binding.id, 1, secret("second-rotation-key"))
        .await
        .unwrap();
    let new_slot = private_binding_state(&db, &binding.id).1.unwrap();
    assert_ne!(old_slot, new_slot);
    assert_eq!(second.credential_version, 2);
    assert_eq!(store.item_count(), 1);
    assert_eq!(journal_count(&db, &binding.id), 0);

    assert_eq!(
        service
            .resolve_binding_api_key(secret("first-rotation-key"))
            .await
            .unwrap_err()
            .to_string(),
        "binding_not_found"
    );
    let resolved = service
        .resolve_binding_api_key(secret("second-rotation-key"))
        .await
        .unwrap();
    assert_eq!(resolved.binding_id(), binding.id);
    assert_eq!(resolved.agent_module_id(), "codex");
    assert_eq!(resolved.provider_id(), "replace-direct");
    assert_eq!(resolved.expose_secret(), b"second-rotation-key");
    assert_eq!(
        format!("{resolved:?}"),
        "ResolvedBindingCredential([REDACTED])"
    );
}

#[tokio::test]
async fn unlinked_v13_provider_does_not_inherit_pricing_from_a_same_id_v12_provider() {
    const PROVIDER_ID: &str = "same-id-provider";
    const BINDING_KEY: &str = "same-id-binding-key";

    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, PROVIDER_ID);
    let mut legacy = Provider::with_id(
        PROVIDER_ID.to_string(),
        "Unlinked v12 provider".to_string(),
        serde_json::json!({"baseUrl": "https://legacy.example"}),
        None,
    );
    legacy.meta = Some(ProviderMeta {
        cost_multiplier: Some("9.25".to_string()),
        pricing_model_source: Some("request".to_string()),
        ..ProviderMeta::default()
    });
    db.save_provider("claude", &legacy).unwrap();

    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store);
    let configured = service
        .set_binding_api_key(&binding.id, 0, secret(BINDING_KEY))
        .await
        .unwrap();
    requested_enabled(&db, &configured);

    let resolved = service
        .resolve_binding_api_key(secret(BINDING_KEY))
        .await
        .unwrap();

    assert_eq!(resolved.provider_id(), PROVIDER_ID);
    assert_eq!(resolved.product_group_id(), "test");
    assert_eq!(resolved.legacy_pricing_provider_id(), None);
    assert_eq!(
        resolved.pricing_override(),
        &BindingPricingOverride::default()
    );
}

#[tokio::test]
async fn orphaned_legacy_link_keeps_pricing_snapshot_and_legacy_log_identity() {
    const PROVIDER_ID: &str = "migrated-v13-provider";
    const MISSING_LEGACY_ID: &str = "deleted-v12-provider";
    const BINDING_KEY: &str = "orphaned-link-binding-key";

    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, PROVIDER_ID);
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE usage_providers
             SET quota_config = ?1,
                 legacy_app_type = 'claude',
                 legacy_provider_id = ?2
             WHERE id = ?3",
            rusqlite::params![
                serde_json::json!({
                    "costMultiplier": "2.75",
                    "pricingModelSource": "request"
                })
                .to_string(),
                MISSING_LEGACY_ID,
                PROVIDER_ID,
            ],
        )
        .unwrap();
    assert!(db
        .get_provider_by_id(MISSING_LEGACY_ID, "claude")
        .unwrap()
        .is_none());

    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store);
    let configured = service
        .set_binding_api_key(&binding.id, 0, secret(BINDING_KEY))
        .await
        .unwrap();
    requested_enabled(&db, &configured);

    let resolved = service
        .resolve_binding_api_key(secret(BINDING_KEY))
        .await
        .unwrap();

    assert_eq!(
        resolved.legacy_pricing_provider_id(),
        Some(MISSING_LEGACY_ID)
    );
    assert_eq!(
        resolved.pricing_override(),
        &BindingPricingOverride {
            cost_multiplier: Some("2.75".to_string()),
            pricing_model_source: Some("request".to_string()),
        }
    );
}

#[tokio::test]
async fn runtime_route_cannot_repeat_the_binding_key_in_non_auth_metadata() {
    const BINDING_KEY: &str = "runtime-route-recontamination-key";

    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, "route-recontamination-direct");
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store);
    let configured = service
        .set_binding_api_key(&binding.id, 0, secret(BINDING_KEY))
        .await
        .unwrap();
    requested_enabled(&db, &configured);
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE usage_providers
             SET route_config = ?1
             WHERE id = 'route-recontamination-direct'",
            [serde_json::json!({
                "baseUrl": "https://api.example",
                "model": BINDING_KEY
            })
            .to_string()],
        )
        .unwrap();

    assert_eq!(
        service
            .resolve_binding_api_key(secret(BINDING_KEY))
            .await
            .unwrap_err()
            .to_string(),
        "invalid_binding"
    );
}

#[tokio::test]
async fn frozen_ownership_cannot_repeat_the_binding_key() {
    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, "ownership-recontamination-direct");
    let binding_key = binding.id.clone();
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store);
    let configured = service
        .set_binding_api_key(&binding.id, 0, secret(&binding_key))
        .await
        .unwrap();
    requested_enabled(&db, &configured);

    assert_eq!(
        service
            .resolve_binding_api_key(secret(&binding_key))
            .await
            .unwrap_err()
            .to_string(),
        "invalid_binding"
    );
}

#[tokio::test]
async fn requested_disabled_binding_is_rejected_without_reading_the_protected_item() {
    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, "disabled-direct");
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db, store.clone());
    service
        .set_binding_api_key(&binding.id, 0, secret("disabled-binding-key"))
        .await
        .unwrap();
    let reads_before_resolve = store.get_call_count();

    let error = service
        .resolve_binding_api_key(secret("disabled-binding-key"))
        .await
        .unwrap_err();

    assert_eq!(error.to_string(), "binding_not_found");
    assert_eq!(store.get_call_count(), reads_before_resolve);
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
async fn clear_commits_fail_closed_before_retrying_orphan_cleanup() {
    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, "clear-direct");
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store.clone());
    let configured = service
        .set_binding_api_key(&binding.id, 0, secret("clear-key-sentinel"))
        .await
        .unwrap();
    requested_enabled(&db, &configured);
    let old_slot = private_binding_state(&db, &binding.id).1.unwrap();
    store.fail_next_delete();

    let cleared = service.clear_binding_api_key(&binding.id, 1).await.unwrap();
    assert_eq!(cleared.credential_version, 2);
    assert_eq!(cleared.credential_status, BindingCredentialStatus::Missing);
    assert!(!cleared.can_clear_credential);
    assert!(!cleared.enabled);
    assert_eq!(
        private_binding_state(&db, &binding.id),
        (None, None, 2, false)
    );
    assert_eq!(journal_count(&db, &binding.id), 1);
    assert_eq!(store.item_count(), 1);
    assert_eq!(
        service
            .resolve_binding_api_key(secret("clear-key-sentinel"))
            .await
            .unwrap_err()
            .to_string(),
        "binding_not_found"
    );

    service.reconcile_startup().await.unwrap();
    assert_eq!(journal_count(&db, &binding.id), 0);
    assert_eq!(store.item_count(), 0);
    assert!(store.get(&old_slot).unwrap().is_none());
}

#[tokio::test]
async fn missing_or_mismatched_protected_item_is_unavailable_and_never_effective() {
    let db = Arc::new(Database::memory().unwrap());
    let binding = direct_binding(&db, "missing-direct");
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store.clone());
    let configured = service
        .set_binding_api_key(&binding.id, 0, secret("verified-key-sentinel"))
        .await
        .unwrap();
    requested_enabled(&db, &configured);
    let slot = private_binding_state(&db, &binding.id).1.unwrap();
    let listed = service
        .list_agent_provider_bindings(Some("codex"))
        .await
        .unwrap();
    let configured = listed.iter().find(|view| view.id == binding.id).unwrap();
    assert_eq!(
        configured.credential_status,
        BindingCredentialStatus::Configured
    );
    assert!(configured.effective_enabled);

    store.remove(&slot);
    let listed = service
        .list_agent_provider_bindings(Some("codex"))
        .await
        .unwrap();
    let missing = listed.iter().find(|view| view.id == binding.id).unwrap();
    assert_eq!(
        missing.credential_status,
        BindingCredentialStatus::Unavailable
    );
    assert!(missing.can_clear_credential);
    assert!(!missing.effective_enabled);
    assert_eq!(
        service
            .resolve_binding_api_key(secret("verified-key-sentinel"))
            .await
            .unwrap_err()
            .to_string(),
        "credential_unavailable"
    );

    store.replace_for_test(&slot, b"different-protected-value");
    let listed = service
        .list_agent_provider_bindings(Some("codex"))
        .await
        .unwrap();
    let mismatched = listed.iter().find(|view| view.id == binding.id).unwrap();
    assert_eq!(
        mismatched.credential_status,
        BindingCredentialStatus::Unavailable
    );
    assert!(mismatched.can_clear_credential);
    assert!(!mismatched.effective_enabled);
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
async fn reconciliation_never_deletes_a_staging_slot_that_is_active_for_another_binding() {
    let db = Arc::new(Database::memory().unwrap());
    let pending = direct_binding(&db, "pending-direct");
    let active = direct_binding(&db, "active-direct");
    let store = Arc::new(MemoryCredentialStore::default());
    let service = BindingCredentialService::new(db.clone(), store.clone());
    let configured = service
        .set_binding_api_key(&active.id, 0, secret("active-key-must-survive"))
        .await
        .unwrap();
    requested_enabled(&db, &configured);
    let active_slot = private_binding_state(&db, &active.id).1.unwrap();
    let reservation = db
        .reserve_credential_operation(
            &pending.id,
            0,
            CredentialMutationKind::Set,
            Some(&[31_u8; 32]),
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
    assert_eq!(store.item_count(), 1);
    assert_eq!(journal_count(&db, &pending.id), 1);
    assert_eq!(
        service
            .resolve_binding_api_key(secret("active-key-must-survive"))
            .await
            .unwrap()
            .binding_id(),
        active.id
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

#[test]
fn app_state_and_proxy_share_the_injected_credential_service() {
    let db = Arc::new(Database::memory().unwrap());
    let store = Arc::new(MemoryCredentialStore::default());
    let state = crate::store::AppState::new_with_credential_store(db, store.clone());
    let proxy_service = state.proxy_service.binding_credential_service();
    assert!(Arc::ptr_eq(
        &state.binding_credential_service,
        &proxy_service
    ));
    let injected: Arc<dyn CredentialStore> = store;
    assert!(Arc::ptr_eq(&state.credential_store, &injected));
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
