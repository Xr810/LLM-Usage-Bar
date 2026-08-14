//! `UpstreamAuth` 的真实实现(T9):按 provider 行上记的 auth_kind 去取凭据。
//!
//! 任务书:docs/tasks/T9-upstream-auth.md。这里不持有任何凭据,每次
//! `headers_for` 都现取现用、用完不留;取不到一律 `Err`,绝不把客户端来的
//! 头当作凭据回落(T6 的转发层本来就会丢弃客户端凭据头,这里也不补)。

use std::sync::Arc;

use crate::credentials::BindingCredentialService;
use crate::database::{Database, RouterAuthKind};
use crate::error::AppError;
use crate::router::server::UpstreamAuth;

/// `UpstreamAuth` 的真实实现:按 provider 行上记的 auth_kind 去取凭据。
///
/// **本结构体不缓存任何凭据**——每次 `headers_for` 都重新取。凭据的生命周期
/// 越短越好,缓存换来的那点性能不值得多一个泄露面。
pub struct RouterUpstreamAuth {
    db: Arc<Database>,
    credentials: Arc<BindingCredentialService>,
}

impl RouterUpstreamAuth {
    pub fn new(db: Arc<Database>, credentials: Arc<BindingCredentialService>) -> Self {
        Self { db, credentials }
    }
}

impl UpstreamAuth for RouterUpstreamAuth {
    fn headers_for(&self, provider_id: &str) -> Result<Vec<(String, String)>, AppError> {
        let provider = self
            .db
            .list_router_providers()?
            .into_iter()
            .find(|provider| provider.id == provider_id)
            .ok_or_else(|| {
                // provider 查不到必须报错:空头会以「未认证」的形态打到上游,
                // 拿回 401 后被 T5 判成 RequestRejected,错误指向完全相反的方向。
                AppError::Message(format!("router provider 不存在: {provider_id}"))
            })?;
        match provider.auth_kind {
            RouterAuthKind::None => Ok(Vec::new()),
            RouterAuthKind::ChatgptOauth => {
                // 只走既有入口:它自己知道读钥匙串还是回落读文件,不在这里
                // 直接碰 ~/.codex/auth.json。message 字段不带进错误信息。
                let (token, _, _, _) =
                    crate::services::subscription::codex::read_codex_credentials();
                match token {
                    Some(token) => Ok(vec![(
                        "Authorization".to_string(),
                        format!("Bearer {token}"),
                    )]),
                    None => Err(AppError::Message("chatgpt_oauth 凭据不可用".to_string())),
                }
            }
            RouterAuthKind::BearerKey => {
                let _key_id = provider.credential_key_id.ok_or_else(|| {
                    AppError::Message("bearer_key 认证缺少 credential_key_id".to_string())
                })?;
                // 取 key 的同步入口尚不存在:resolve_provider_api_key 是 async,
                // 而 headers_for 是同步的(T6 定的签名),router 又正跑在 tokio
                // worker 上不能 block_on。在凭据体系补出同步入口之前这里直接
                // 拒绝,而不是绕过版本校验与指纹比对自己去读 CredentialStore。
                // 待接入口:BindingCredentialService 的同步取 key 方法。下面这行
                // 只是占住 credentials 字段的读取,同步入口落地后删掉。
                let _ = &self.credentials;
                Err(AppError::Message("待接同步凭据入口".to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::{CredentialStore, CredentialStoreError};
    use crate::database::{RouterProvider, WireApi};
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// 内存版假凭据库,照抄 credentials/tests.rs 的做法,只是裁剪到
    /// 本模块测试需要的三个方法。
    #[derive(Default)]
    struct MemoryCredentialStore {
        items: Mutex<HashMap<String, Vec<u8>>>,
    }

    impl CredentialStore for MemoryCredentialStore {
        fn put(&self, slot: &str, secret: &[u8]) -> Result<(), CredentialStoreError> {
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

    fn memory_db() -> Arc<Database> {
        Arc::new(Database::memory().unwrap())
    }

    fn auth(db: Arc<Database>) -> RouterUpstreamAuth {
        RouterUpstreamAuth::new(
            db.clone(),
            Arc::new(BindingCredentialService::new(
                db,
                Arc::new(MemoryCredentialStore::default()),
            )),
        )
    }

    fn upsert_provider(
        db: &Database,
        id: &str,
        auth_kind: RouterAuthKind,
        credential_key_id: Option<&str>,
    ) {
        db.upsert_router_provider(&RouterProvider {
            id: id.to_string(),
            display_name: id.to_string(),
            base_url: format!("https://{id}.example/v1"),
            wire_api: WireApi::Responses,
            priority: 1,
            enabled: true,
            auth_kind,
            credential_key_id: credential_key_id.map(str::to_string),
        })
        .unwrap();
    }

    // 1. auth_kind = none → 空头。
    #[test]
    fn auth_none_returns_empty_headers() {
        let db = memory_db();
        upsert_provider(&db, "no-auth", RouterAuthKind::None, None);

        let headers = auth(db).headers_for("no-auth").unwrap();

        assert_eq!(headers, Vec::<(String, String)>::new());
    }

    // 2. bearer_key 且 key 存在。
    //
    // 任务书期望「恰好一个头,值是 Bearer <明文>」,但取 key 的同步入口尚不存在
    // (resolve_provider_api_key 是 async,headers_for 是同步且不能 block_on),
    // 当前契约是返回占位错误。凭据体系补出同步入口后,此测试要改成断言
    // `[("Authorization", "Bearer <明文>")]`,并把「不走同步入口就取不到 key」
    // 的断言删掉。
    #[test]
    fn bearer_key_with_existing_key_waits_for_sync_entry() {
        let db = memory_db();
        upsert_provider(&db, "keyed", RouterAuthKind::BearerKey, Some("key-1"));

        let error = auth(db).headers_for("keyed").unwrap_err();

        assert_eq!(error.to_string(), "待接同步凭据入口");
    }

    // 3. bearer_key 但 credential_key_id 是 None → Err。
    #[test]
    fn bearer_key_without_credential_key_id_is_an_error() {
        let db = memory_db();
        upsert_provider(&db, "keyless", RouterAuthKind::BearerKey, None);

        let error = auth(db).headers_for("keyless").unwrap_err();

        assert!(error.to_string().contains("credential_key_id"));
    }

    // 4. bearer_key 但那把 key 已被删(表里不存在)→ Err,不 panic。
    #[test]
    fn bearer_key_pointing_at_deleted_key_is_an_error_without_panic() {
        let db = memory_db();
        upsert_provider(&db, "ghost", RouterAuthKind::BearerKey, Some("deleted-key"));

        let error = auth(db).headers_for("ghost").unwrap_err();

        assert_eq!(error.to_string(), "待接同步凭据入口");
    }

    // 5. provider_id 在表里不存在 → Err,绝不是空头。
    #[test]
    fn unknown_provider_id_is_an_error_not_empty_headers() {
        let db = memory_db();

        let error = auth(db).headers_for("nobody").unwrap_err();

        assert!(error.to_string().contains("不存在"));
    }

    // 6. 库里 auth_kind 是非法字符串 → 读取返回 Err,不 panic。
    #[test]
    fn invalid_auth_kind_in_db_is_an_error_without_panic() {
        let db = memory_db();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO router_providers (
                     id, display_name, base_url, wire_api, priority, enabled,
                     auth_kind, credential_key_id, created_at, updated_at
                 ) VALUES (
                     'broken', 'Broken', 'https://broken.example', 'responses',
                     1, 1, 'bogus', NULL, 0, 0
                 )",
                [],
            )
            .unwrap();
        }

        let error = auth(db).headers_for("broken").unwrap_err();

        assert!(error.to_string().contains("auth_kind"));
    }

    // 7. 往返:三个枚举值存进去读出来是同一个。
    #[test]
    fn auth_kind_round_trips_through_storage_for_all_variants() {
        let db = memory_db();
        for kind in [
            RouterAuthKind::ChatgptOauth,
            RouterAuthKind::BearerKey,
            RouterAuthKind::None,
        ] {
            upsert_provider(&db, "roundtrip", kind, None);
            let listed = db.list_router_providers().unwrap();
            let stored = listed.iter().find(|p| p.id == "roundtrip").unwrap();
            assert_eq!(stored.auth_kind, kind);
        }
    }

    // 8. 安全测试:错误信息里不含凭据明文。
    //
    // 构造一把已知内容的 key,让它按第 4 条那样失败,断言 err.to_string()
    // 里搜不到明文。占位错误本身是固定串,这测试对未来的同步实现是护栏。
    #[test]
    fn error_message_never_contains_the_credential_plaintext() {
        const PLAINTEXT: &str = "sk-lub-security-probe-9f3a7c1e2b4d5f6a";
        let db = memory_db();
        let store = Arc::new(MemoryCredentialStore::default());
        store
            .put("probe-slot", PLAINTEXT.as_bytes())
            .expect("seed fake credential store");
        let service = Arc::new(BindingCredentialService::new(db.clone(), store));
        upsert_provider(&db, "probe", RouterAuthKind::BearerKey, Some("probe-key"));

        let error = RouterUpstreamAuth::new(db, service)
            .headers_for("probe")
            .unwrap_err();
        let rendered = error.to_string();

        assert!(
            !rendered.contains(PLAINTEXT),
            "错误信息绝不能携带凭据明文: {rendered}"
        );
    }
}
