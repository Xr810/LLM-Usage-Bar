//! OpenRouter 账户余额的业务 api 层(D1):与传输无关,不 import 任何 tauri 类型。
//!
//! 「设置管理 key / 清空 / 读余额视图」的契约收在这里,
//! `commands/openrouter_balance.rs` 退化成薄壳;将来的 socket 面板直接调这里。
//! 安全面:入参里的 key 转成 `SecretString`(drop 时零化),日志与错误串
//! 绝不包含 key 明文;视图里只有金额与时间,没有凭据。

use std::sync::Arc;

use crate::error::AppError;
use crate::secrets::{CredentialStore, SecretString};
use crate::services::balance::{
    clear_snapshot, has_openrouter_management_key, load_snapshot, OpenRouterAccountBalanceView,
    OpenRouterBalanceSnapshot, OPENROUTER_MANAGEMENT_KEY_SLOT,
};
use crate::store::Database;

/// OpenRouter 账户余额的编排层。这里只做读写编排,
/// 网络查询在 `services/balance.rs` 的调度器里。
pub struct OpenRouterBalanceApi {
    db: Arc<Database>,
    credentials: Arc<dyn CredentialStore>,
}

impl OpenRouterBalanceApi {
    pub fn new(db: Arc<Database>, credentials: Arc<dyn CredentialStore>) -> Self {
        Self { db, credentials }
    }

    /// 存管理 key(trim 后)。空串拒绝;验证(非空)通过才写入钥匙串,
    /// 失败路径绝不先清旧值——已有值保持原样(验证后才替换)。
    pub fn set_management_key(&self, key: &str) -> Result<(), AppError> {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            return Err(AppError::InvalidInput(
                "OpenRouter 管理 key 不能为空".to_string(),
            ));
        }
        let secret = SecretString::new(trimmed.to_string());
        self.credentials
            .put(OPENROUTER_MANAGEMENT_KEY_SLOT, secret.expose_bytes())
            .map_err(|error| AppError::Message(format!("保存 OpenRouter 管理 key 失败: {error}")))
    }

    /// 删钥匙串条目 + 清余额快照。
    pub fn clear_management_key(&self) -> Result<(), AppError> {
        self.credentials
            .delete(OPENROUTER_MANAGEMENT_KEY_SLOT)
            .map_err(|error| {
                AppError::Message(format!("删除 OpenRouter 管理 key 失败: {error}"))
            })?;
        clear_snapshot(&self.db)
    }

    /// 只读余额视图:settings 里的快照 + 钥匙串里有没有 key,不发请求。
    pub fn get_account_balance(&self) -> Result<OpenRouterAccountBalanceView, AppError> {
        let has_management_key = has_openrouter_management_key(&*self.credentials);
        let snapshot: Option<OpenRouterBalanceSnapshot> = load_snapshot(&self.db)?;
        Ok(OpenRouterAccountBalanceView {
            total_credits_usd: snapshot
                .as_ref()
                .map(|value| value.total_credits_usd.clone()),
            total_usage_usd: snapshot.as_ref().map(|value| value.total_usage_usd.clone()),
            balance_usd: snapshot.as_ref().map(|value| value.balance_usd.clone()),
            fetched_at: snapshot.as_ref().map(|value| value.fetched_at),
            has_management_key,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::CredentialStoreError;
    use crate::services::balance::{
        save_snapshot, OpenRouterBalanceSnapshot, OPENROUTER_ACCOUNT_BALANCE_SETTING_KEY,
    };
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemoryCredentialStore {
        items: Mutex<HashMap<String, Vec<u8>>>,
        fail_next_put: AtomicBool,
    }

    impl CredentialStore for MemoryCredentialStore {
        fn put(&self, slot: &str, secret: &[u8]) -> Result<(), CredentialStoreError> {
            if self.fail_next_put.swap(false, Ordering::SeqCst) {
                return Err(CredentialStoreError::OperationFailed);
            }
            self.items
                .lock()
                .unwrap()
                .insert(slot.to_string(), secret.to_vec());
            Ok(())
        }

        fn get(&self, slot: &str) -> Result<Option<Vec<u8>>, CredentialStoreError> {
            Ok(self.items.lock().unwrap().get(slot).cloned())
        }

        fn delete(&self, slot: &str) -> Result<(), CredentialStoreError> {
            self.items.lock().unwrap().remove(slot);
            Ok(())
        }
    }

    fn snapshot_fixture(fetched_at: i64) -> OpenRouterBalanceSnapshot {
        OpenRouterBalanceSnapshot {
            total_credits_usd: "100.5".to_string(),
            total_usage_usd: "25.75".to_string(),
            balance_usd: "74.75".to_string(),
            fetched_at,
        }
    }

    // ── 7. set/clear 命令语义与不泄 key 明文 ─────────────────

    #[test]
    fn set_management_key_trims_and_stores_into_the_credential_store() {
        let db = Arc::new(Database::memory().unwrap());
        let store = Arc::new(MemoryCredentialStore::default());
        let api = OpenRouterBalanceApi::new(db, store.clone());

        api.set_management_key("  sk-or-mgmt-test-123  ").unwrap();
        assert_eq!(
            store
                .get(OPENROUTER_MANAGEMENT_KEY_SLOT)
                .unwrap()
                .as_deref(),
            Some(&b"sk-or-mgmt-test-123"[..])
        );
    }

    #[test]
    fn set_management_key_rejects_empty_input_without_touching_the_store() {
        let db = Arc::new(Database::memory().unwrap());
        let store = Arc::new(MemoryCredentialStore::default());
        let api = OpenRouterBalanceApi::new(db, store.clone());

        assert!(api.set_management_key("").is_err());
        assert!(api.set_management_key("   ").is_err());
        assert_eq!(store.get(OPENROUTER_MANAGEMENT_KEY_SLOT).unwrap(), None);
    }

    #[test]
    fn set_management_key_failure_keeps_the_existing_value() {
        let db = Arc::new(Database::memory().unwrap());
        let store = Arc::new(MemoryCredentialStore::default());
        let api = OpenRouterBalanceApi::new(db, store.clone());
        store
            .put(OPENROUTER_MANAGEMENT_KEY_SLOT, b"sk-or-old")
            .unwrap();
        store.fail_next_put.store(true, Ordering::SeqCst);

        assert!(api.set_management_key("sk-or-new").is_err());
        assert_eq!(
            store
                .get(OPENROUTER_MANAGEMENT_KEY_SLOT)
                .unwrap()
                .as_deref(),
            Some(&b"sk-or-old"[..])
        );
    }

    #[test]
    fn clear_management_key_removes_the_key_and_the_snapshot() {
        let db = Arc::new(Database::memory().unwrap());
        let store = Arc::new(MemoryCredentialStore::default());
        let api = OpenRouterBalanceApi::new(db.clone(), store.clone());
        store
            .put(OPENROUTER_MANAGEMENT_KEY_SLOT, b"sk-or-mgmt-test")
            .unwrap();
        save_snapshot(&db, &snapshot_fixture(123)).unwrap();

        api.clear_management_key().unwrap();
        assert_eq!(store.get(OPENROUTER_MANAGEMENT_KEY_SLOT).unwrap(), None);
        assert_eq!(
            db.get_setting(OPENROUTER_ACCOUNT_BALANCE_SETTING_KEY)
                .unwrap(),
            None
        );

        let view = api.get_account_balance().unwrap();
        assert!(!view.has_management_key);
        assert_eq!(view.total_credits_usd, None);
        assert_eq!(view.balance_usd, None);
    }

    // ── 6. 无管理 key 时视图全 None ──────────────────────────

    #[test]
    fn get_account_balance_returns_none_fields_when_no_key_or_snapshot() {
        let db = Arc::new(Database::memory().unwrap());
        let store = Arc::new(MemoryCredentialStore::default());
        let api = OpenRouterBalanceApi::new(db, store);

        let view = api.get_account_balance().unwrap();
        assert!(!view.has_management_key);
        assert_eq!(view.total_credits_usd, None);
        assert_eq!(view.total_usage_usd, None);
        assert_eq!(view.balance_usd, None);
        assert_eq!(view.fetched_at, None);
    }

    #[test]
    fn get_account_balance_exposes_the_snapshot_without_any_secret() {
        let db = Arc::new(Database::memory().unwrap());
        let store = Arc::new(MemoryCredentialStore::default());
        store
            .put(OPENROUTER_MANAGEMENT_KEY_SLOT, b"sk-or-secret-value")
            .unwrap();
        save_snapshot(&db, &snapshot_fixture(456)).unwrap();
        let api = OpenRouterBalanceApi::new(db, store);

        let view = api.get_account_balance().unwrap();
        assert!(view.has_management_key);
        assert_eq!(view.total_credits_usd.as_deref(), Some("100.5"));
        assert_eq!(view.total_usage_usd.as_deref(), Some("25.75"));
        assert_eq!(view.balance_usd.as_deref(), Some("74.75"));
        assert_eq!(view.fetched_at, Some(456));
    }

    #[test]
    fn error_paths_never_leak_the_key_plaintext() {
        let db = Arc::new(Database::memory().unwrap());
        let store = Arc::new(MemoryCredentialStore::default());
        let api = OpenRouterBalanceApi::new(db, store.clone());
        store.fail_next_put.store(true, Ordering::SeqCst);
        const SECRET: &str = "sk-or-mgmt-super-secret-abc123";

        let error = api.set_management_key(SECRET).unwrap_err();
        // 返回体(错误串)不含 key 明文;这条错误串也是日志里会出现的文本
        assert!(!format!("{error}").contains(SECRET));
        assert!(!format!("{error:?}").contains(SECRET));

        let view = api.get_account_balance().unwrap();
        // 视图序列化后也不含 key 明文(视图根本没有凭据字段)
        let serialized = serde_json::to_string(&view).unwrap();
        assert!(!serialized.contains(SECRET));
    }
}
