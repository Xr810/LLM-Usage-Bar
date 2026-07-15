use crate::credentials::{BindingCredentialService, ResolvedProviderCredential};
use crate::database::Database;
use crate::error::AppError;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use zeroize::Zeroizing;

pub(crate) struct SystemProviderConnectionRequest {
    url: String,
    headers: Vec<(String, Zeroizing<String>)>,
}

impl std::fmt::Debug for SystemProviderConnectionRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SystemProviderConnectionRequest([REDACTED])")
    }
}

impl SystemProviderConnectionRequest {
    #[cfg(test)]
    fn headers_for_test(&self) -> Vec<(String, String)> {
        self.headers
            .iter()
            .map(|(name, value)| (name.clone(), value.to_string()))
            .collect()
    }
}

pub(crate) type SystemProviderConnectionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<u16, AppError>> + Send + 'a>>;

pub(crate) trait SystemProviderConnectionClient: Send + Sync {
    fn get(&self, request: SystemProviderConnectionRequest) -> SystemProviderConnectionFuture<'_>;
}

#[derive(Default)]
struct ReqwestSystemProviderConnectionClient;

impl SystemProviderConnectionClient for ReqwestSystemProviderConnectionClient {
    fn get(&self, request: SystemProviderConnectionRequest) -> SystemProviderConnectionFuture<'_> {
        Box::pin(async move {
            let client = crate::proxy::http_client::get();
            let mut builder = client.get(&request.url).timeout(Duration::from_secs(8));
            for (name, value) in &request.headers {
                let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                    .map_err(|_| AppError::Message("connection_failed".to_string()))?;
                let value = reqwest::header::HeaderValue::from_str(value)
                    .map_err(|_| AppError::Message("connection_failed".to_string()))?;
                builder = builder.header(name, value);
            }
            builder
                .send()
                .await
                .map(|response| response.status().as_u16())
                .map_err(|_| AppError::Message("connection_failed".to_string()))
        })
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemProviderConnectionTestResult {
    pub provider_id: String,
    pub success: bool,
    pub status: String,
    pub tested_at: i64,
    pub error_code: Option<String>,
}

pub struct SystemProviderConnectionService {
    db: Arc<Database>,
    credentials: Arc<BindingCredentialService>,
    client: Arc<dyn SystemProviderConnectionClient>,
}

impl SystemProviderConnectionService {
    pub fn production(db: Arc<Database>, credentials: Arc<BindingCredentialService>) -> Self {
        Self::new(
            db,
            credentials,
            Arc::new(ReqwestSystemProviderConnectionClient),
        )
    }

    pub(crate) fn new(
        db: Arc<Database>,
        credentials: Arc<BindingCredentialService>,
        client: Arc<dyn SystemProviderConnectionClient>,
    ) -> Self {
        Self {
            db,
            credentials,
            client,
        }
    }

    fn build_request(
        credential: &ResolvedProviderCredential,
    ) -> Result<SystemProviderConnectionRequest, AppError> {
        let base = credential.canonical_endpoint().trim_end_matches('/');
        let (url, headers) = match credential.system_preset_key() {
            "openai-api" | "openrouter-api" => (
                format!("{base}/models"),
                vec![(
                    "authorization".to_string(),
                    Zeroizing::new(format!(
                        "Bearer {}",
                        std::str::from_utf8(credential.expose_secret())
                            .map_err(|_| AppError::Message("credential_unavailable".to_string()))?
                    )),
                )],
            ),
            "anthropic-api" => (
                format!("{base}/v1/models"),
                vec![
                    (
                        "x-api-key".to_string(),
                        Zeroizing::new(
                            std::str::from_utf8(credential.expose_secret())
                                .map_err(|_| {
                                    AppError::Message("credential_unavailable".to_string())
                                })?
                                .to_string(),
                        ),
                    ),
                    (
                        "anthropic-version".to_string(),
                        Zeroizing::new("2023-06-01".to_string()),
                    ),
                ],
            ),
            _ => return Err(AppError::Message("unsupported_auth".to_string())),
        };
        Ok(SystemProviderConnectionRequest { url, headers })
    }

    pub async fn test(
        &self,
        provider_id: &str,
        expected_version: u64,
    ) -> Result<SystemProviderConnectionTestResult, AppError> {
        let credential = self
            .credentials
            .resolve_provider_api_key(provider_id, expected_version)
            .await?;
        let frozen_provider_id = credential.provider_id().to_string();
        let frozen_version = credential.credential_version();
        let request = Self::build_request(&credential)?;
        drop(credential);
        let (success, error_code) = match self.client.get(request).await {
            Ok(200..=299) => (true, None),
            Ok(401 | 403) => (false, Some("authentication_failed".to_string())),
            Ok(_) => (false, Some("upstream_rejected".to_string())),
            Err(_) => (false, Some("connection_failed".to_string())),
        };
        let status = if success { "success" } else { "failed" };
        let tested_at = self.db.record_provider_connection_test(
            &frozen_provider_id,
            frozen_version,
            status,
            error_code.as_deref(),
        )?;
        Ok(SystemProviderConnectionTestResult {
            provider_id: frozen_provider_id,
            success,
            status: status.to_string(),
            tested_at,
            error_code,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::{CredentialStore, CredentialStoreError, SecretString};
    use std::collections::{HashMap, VecDeque};
    use std::sync::{Arc, Mutex};
    use zeroize::Zeroizing;

    #[derive(Default)]
    struct MemoryStore(Mutex<HashMap<String, Zeroizing<Vec<u8>>>>);

    impl CredentialStore for MemoryStore {
        fn put(&self, slot: &str, secret: &[u8]) -> Result<(), CredentialStoreError> {
            self.0
                .lock()
                .unwrap()
                .insert(slot.to_string(), Zeroizing::new(secret.to_vec()));
            Ok(())
        }

        fn get(&self, slot: &str) -> Result<Option<Vec<u8>>, CredentialStoreError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(slot)
                .map(|secret| secret.to_vec()))
        }

        fn delete(&self, slot: &str) -> Result<(), CredentialStoreError> {
            self.0.lock().unwrap().remove(slot);
            Ok(())
        }
    }

    struct RecordingClient {
        requests: Mutex<Vec<SystemProviderConnectionRequest>>,
        results: Mutex<VecDeque<Result<u16, AppError>>>,
    }

    impl SystemProviderConnectionClient for RecordingClient {
        fn get(
            &self,
            request: SystemProviderConnectionRequest,
        ) -> SystemProviderConnectionFuture<'_> {
            self.requests.lock().unwrap().push(request);
            let result = self.results.lock().unwrap().pop_front().unwrap();
            Box::pin(async move { result })
        }
    }

    #[tokio::test]
    async fn probes_only_canonical_model_endpoints_with_fixed_auth_headers() {
        for (provider_id, key, expected_url, expected_headers) in [
            (
                "system-openai-api",
                "openai-connection-secret",
                "https://api.openai.com/v1/models",
                vec![("authorization", "Bearer openai-connection-secret")],
            ),
            (
                "system-openrouter-api",
                "openrouter-connection-secret",
                "https://openrouter.ai/api/v1/models",
                vec![("authorization", "Bearer openrouter-connection-secret")],
            ),
            (
                "system-anthropic-api",
                "anthropic-connection-secret",
                "https://api.anthropic.com/v1/models",
                vec![
                    ("x-api-key", "anthropic-connection-secret"),
                    ("anthropic-version", "2023-06-01"),
                ],
            ),
        ] {
            let db = Arc::new(Database::memory().unwrap());
            let credentials = Arc::new(BindingCredentialService::new(
                db.clone(),
                Arc::new(MemoryStore::default()),
            ));
            credentials
                .set_provider_api_key(provider_id, 0, SecretString::new(key.to_string()))
                .await
                .unwrap();
            let client = Arc::new(RecordingClient {
                requests: Mutex::new(Vec::new()),
                results: Mutex::new(vec![Ok(200)].into()),
            });
            let service =
                SystemProviderConnectionService::new(db.clone(), credentials, client.clone());
            let result = service.test(provider_id, 1).await.unwrap();
            assert!(result.success);
            assert_eq!(result.status, "success");
            assert_eq!(result.error_code, None);
            assert!(!serde_json::to_string(&result).unwrap().contains(key));

            let requests = client.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].url, expected_url);
            assert_eq!(
                requests[0].headers_for_test(),
                expected_headers
                    .into_iter()
                    .map(|(name, value)| (name.to_string(), value.to_string()))
                    .collect::<Vec<_>>()
            );
            let provider = db
                .list_usage_providers()
                .unwrap()
                .into_iter()
                .find(|provider| provider.id == provider_id)
                .unwrap();
            assert_eq!(
                provider.last_connection_test_status.as_deref(),
                Some("success")
            );
        }
    }

    #[tokio::test]
    async fn probe_collapses_http_and_transport_details_to_public_codes() {
        for (transport_result, expected_code) in [
            (Ok(401), "authentication_failed"),
            (Ok(503), "upstream_rejected"),
            (
                Err(AppError::Message(
                    "secret upstream body and host diagnostics".to_string(),
                )),
                "connection_failed",
            ),
        ] {
            let db = Arc::new(Database::memory().unwrap());
            let credentials = Arc::new(BindingCredentialService::new(
                db.clone(),
                Arc::new(MemoryStore::default()),
            ));
            credentials
                .set_provider_api_key(
                    "system-openrouter-api",
                    0,
                    SecretString::new("redacted-connection-key".to_string()),
                )
                .await
                .unwrap();
            let service = SystemProviderConnectionService::new(
                db,
                credentials,
                Arc::new(RecordingClient {
                    requests: Mutex::new(Vec::new()),
                    results: Mutex::new(vec![transport_result].into()),
                }),
            );
            let result = service.test("system-openrouter-api", 1).await.unwrap();
            assert!(!result.success);
            assert_eq!(result.error_code.as_deref(), Some(expected_code));
            let json = serde_json::to_string(&result).unwrap();
            assert!(!json.contains("secret upstream"));
            assert!(!json.contains("redacted-connection-key"));
        }
    }
}
