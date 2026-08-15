//! `UpstreamAuth` 的真实实现(T9):按 provider 行上记的 auth_kind 去取凭据。
//!
//! 任务书:docs/tasks/T9-upstream-auth.md。这里不持有任何凭据,每次
//! `headers_for` 都现取现用、用完不留;取不到一律 `Err`,绝不把客户端来的
//! 头当作凭据回落(T6 的转发层本来就会丢弃客户端凭据头,这里也不补)。

use std::sync::Arc;

use futures::future::BoxFuture;

use crate::error::AppError;
use crate::router::server::UpstreamAuth;
use crate::secrets::BindingCredentialService;
use crate::store::{Database, RouterAuthKind};

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
    fn headers_for<'a>(
        &'a self,
        provider_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, String)>, AppError>> {
        Box::pin(async move { self.resolve_headers(provider_id).await })
    }
}

impl RouterUpstreamAuth {
    /// trait 方法只负责装箱,判定逻辑放这里——嵌在 `Box::pin` 里的一大段
    /// async 块很难读,而这一段是安全面,必须好读。
    async fn resolve_headers(&self, provider_id: &str) -> Result<Vec<(String, String)>, AppError> {
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
                let key_id = provider.credential_key_id.ok_or_else(|| {
                    AppError::Message("bearer_key 认证缺少 credential_key_id".to_string())
                })?;
                // 版本从当前快照取:router 要的是「此刻这把 key」,不是界面上
                // 某个时刻看到的那把,所以这里没有一个外部传进来的期望版本。
                // 这不会削弱校验——resolve_provider_api_key 取回凭据之后还会
                // 再比对一次版本、槽位与指纹,轮换发生在这两步之间仍然会被拒。
                let version = self
                    .db
                    .provider_credential_snapshot(&key_id)?
                    .ok_or_else(|| AppError::Message(format!("凭据不存在: {key_id}")))?
                    .credential_version;
                let credential = self
                    .credentials
                    .resolve_provider_api_key(&key_id, version)
                    .await?;
                let key = std::str::from_utf8(credential.expose_secret())
                    .map_err(|_| AppError::Message("凭据不是合法 UTF-8".to_string()))?;
                // 拼完立刻 drop 掉带 Zeroizing 的那份。头本身是普通 String——
                // trait 的返回类型如此(T6 定的),这一段无法零化,所以调用方
                // 用完就丢、不要存(server.rs 的转发循环就是这么用的)。
                let header = format!("Bearer {key}");
                drop(credential);
                Ok(vec![("Authorization".to_string(), header)])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{CredentialStore, CredentialStoreError, SecretString};
    use crate::store::{RouterProvider, WireApi};
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

    /// 造一把真的 provider key:建行 + 经凭据体系存进假 store。
    /// 走的是生产入口,所以指纹、版本、槽位都是真的——测试里手搓这三样
    /// 等于把要验的东西自己伪造一遍。
    async fn seed_real_key(
        db: &Database,
        credentials: &BindingCredentialService,
        plaintext: &str,
    ) -> String {
        let key_id = db
            .create_provider_api_key("system-openrouter-api", "router test key")
            .unwrap();
        credentials
            .set_provider_api_key(&key_id, 0, SecretString::new(plaintext.to_string()))
            .await
            .unwrap();
        key_id
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
    #[tokio::test]
    async fn auth_none_returns_empty_headers() {
        let db = memory_db();
        upsert_provider(&db, "no-auth", RouterAuthKind::None, None);

        let headers = auth(db).headers_for("no-auth").await.unwrap();

        assert_eq!(headers, Vec::<(String, String)>::new());
    }

    // 2. bearer_key 且 key 存在 → 恰好一个头,值是 Bearer <明文>。
    #[tokio::test]
    async fn bearer_key_with_existing_key_returns_the_bearer_header() {
        const PLAINTEXT: &str = "sk-lub-router-bearer-1a2b3c4d";
        let db = memory_db();
        let credentials = Arc::new(BindingCredentialService::new(
            db.clone(),
            Arc::new(MemoryCredentialStore::default()),
        ));
        let key_id = seed_real_key(&db, &credentials, PLAINTEXT).await;
        upsert_provider(&db, "keyed", RouterAuthKind::BearerKey, Some(&key_id));

        let headers = RouterUpstreamAuth::new(db, credentials)
            .headers_for("keyed")
            .await
            .unwrap();

        assert_eq!(
            headers,
            vec![("Authorization".to_string(), format!("Bearer {PLAINTEXT}"))]
        );
    }

    // 3. bearer_key 但 credential_key_id 是 None → Err。
    #[tokio::test]
    async fn bearer_key_without_credential_key_id_is_an_error() {
        let db = memory_db();
        upsert_provider(&db, "keyless", RouterAuthKind::BearerKey, None);

        let error = auth(db).headers_for("keyless").await.unwrap_err();

        assert!(error.to_string().contains("credential_key_id"));
    }

    // 4. bearer_key 但那把 key 已被删(表里不存在)→ Err,不 panic。
    #[tokio::test]
    async fn bearer_key_pointing_at_deleted_key_is_an_error_without_panic() {
        let db = memory_db();
        upsert_provider(&db, "ghost", RouterAuthKind::BearerKey, Some("deleted-key"));

        let error = auth(db).headers_for("ghost").await.unwrap_err();

        assert!(error.to_string().contains("凭据不存在"));
    }

    // 5. provider_id 在表里不存在 → Err,绝不是空头。
    #[tokio::test]
    async fn unknown_provider_id_is_an_error_not_empty_headers() {
        let db = memory_db();

        let error = auth(db).headers_for("nobody").await.unwrap_err();

        assert!(error.to_string().contains("不存在"));
    }

    // 6. 库里 auth_kind 是非法字符串 → 读取返回 Err,不 panic。
    #[tokio::test]
    async fn invalid_auth_kind_in_db_is_an_error_without_panic() {
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

        let error = auth(db).headers_for("broken").await.unwrap_err();

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
    // 走的是**真实的 resolve 失败路径**:先按生产入口存一把已知明文的 key
    // (于是指纹、槽位、版本都是真的),再把 store 里那条抹掉,让
    // resolve_provider_api_key 在「行还在、秘密没了」这个状态上失败。
    // 这是最容易把明文带进错误信息的一条分支,所以盯它。
    #[tokio::test]
    async fn error_message_never_contains_the_credential_plaintext() {
        const PLAINTEXT: &str = "sk-lub-security-probe-9f3a7c1e2b4d5f6a";
        let db = memory_db();
        let store = Arc::new(MemoryCredentialStore::default());
        let service = Arc::new(BindingCredentialService::new(db.clone(), store.clone()));
        let key_id = seed_real_key(&db, &service, PLAINTEXT).await;
        upsert_provider(&db, "probe", RouterAuthKind::BearerKey, Some(&key_id));
        // 抹掉秘密但留下 key 行:resolve 会走到「取不到/对不上」而不是「不存在」。
        store.items.lock().unwrap().clear();

        let error = RouterUpstreamAuth::new(db, service)
            .headers_for("probe")
            .await
            .unwrap_err();
        let rendered = error.to_string();

        assert!(
            !rendered.contains(PLAINTEXT),
            "错误信息绝不能携带凭据明文: {rendered}"
        );
    }
}
