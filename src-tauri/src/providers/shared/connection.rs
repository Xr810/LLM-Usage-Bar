use crate::error::AppError;
use crate::model::SystemProviderKeyUsageView;
use crate::secrets::{BindingCredentialService, ResolvedProviderCredential};
use crate::store::{Database, ProviderKeyUsageSnapshotInput};
use crate::usage::system_providers::system_provider_definition;
use rust_decimal::Decimal;
use serde_json::Value;
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelListClientError {
    ConnectionFailed,
    InvalidResponse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelListClientResponse {
    status: u16,
    model_ids: Vec<String>,
}

pub(crate) type SystemProviderModelListFuture<'a> = Pin<
    Box<dyn Future<Output = Result<ModelListClientResponse, ModelListClientError>> + Send + 'a>,
>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SystemProviderKeyUsageClientError {
    ConnectionFailed,
    InvalidResponse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SystemProviderKeyUsageClientResponse {
    pub(crate) status: u16,
    pub(crate) usage_total_usd: Option<Decimal>,
    pub(crate) usage_daily_usd: Option<Decimal>,
    pub(crate) usage_weekly_usd: Option<Decimal>,
    pub(crate) usage_monthly_usd: Option<Decimal>,
    pub(crate) limit_usd: Option<Decimal>,
    pub(crate) limit_remaining_usd: Option<Decimal>,
    pub(crate) is_free_tier: Option<bool>,
}

impl SystemProviderKeyUsageClientResponse {
    pub(crate) fn empty(status: u16) -> Self {
        Self {
            status,
            usage_total_usd: None,
            usage_daily_usd: None,
            usage_weekly_usd: None,
            usage_monthly_usd: None,
            limit_usd: None,
            limit_remaining_usd: None,
            is_free_tier: None,
        }
    }
}

pub(crate) type SystemProviderKeyUsageFuture<'a> = Pin<
    Box<
        dyn Future<
                Output = Result<
                    SystemProviderKeyUsageClientResponse,
                    SystemProviderKeyUsageClientError,
                >,
            > + Send
            + 'a,
    >,
>;

pub(crate) trait SystemProviderConnectionClient: Send + Sync {
    fn get(&self, request: SystemProviderConnectionRequest) -> SystemProviderConnectionFuture<'_>;

    fn list_model_ids(
        &self,
        request: SystemProviderConnectionRequest,
    ) -> SystemProviderModelListFuture<'_>;

    fn fetch_key_usage(
        &self,
        request: SystemProviderConnectionRequest,
    ) -> SystemProviderKeyUsageFuture<'_>;
}

#[derive(serde::Deserialize)]
struct ModelListItem {
    id: String,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum ModelListPayload {
    Data { data: Vec<ModelListItem> },
    Models { models: Vec<ModelListItem> },
    Bare(Vec<ModelListItem>),
}

fn parse_model_ids(body: &[u8]) -> Result<Vec<String>, ModelListClientError> {
    let payload: ModelListPayload =
        serde_json::from_slice(body).map_err(|_| ModelListClientError::InvalidResponse)?;
    let items = match payload {
        ModelListPayload::Data { data } => data,
        ModelListPayload::Models { models } => models,
        ModelListPayload::Bare(items) => items,
    };
    let mut seen = HashSet::new();
    let mut model_ids = Vec::new();
    for item in items {
        let id = item.id.trim().to_string();
        if !id.is_empty() && seen.insert(id.clone()) {
            model_ids.push(id);
        }
    }
    Ok(model_ids)
}

fn optional_key_usage_decimal(
    data: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<Decimal>, SystemProviderKeyUsageClientError> {
    let Some(value) = data.get(field) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let raw = match value {
        Value::Number(number) => number.to_string(),
        Value::String(value) => value.clone(),
        _ => return Err(SystemProviderKeyUsageClientError::InvalidResponse),
    };
    Decimal::from_str(raw.trim())
        .map(Some)
        .map_err(|_| SystemProviderKeyUsageClientError::InvalidResponse)
}

fn optional_key_usage_bool(
    data: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<bool>, SystemProviderKeyUsageClientError> {
    match data.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(SystemProviderKeyUsageClientError::InvalidResponse),
    }
}

fn parse_key_usage_response(
    status: u16,
    body: &[u8],
) -> Result<SystemProviderKeyUsageClientResponse, SystemProviderKeyUsageClientError> {
    let payload: Value = serde_json::from_slice(body)
        .map_err(|_| SystemProviderKeyUsageClientError::InvalidResponse)?;
    let data = payload
        .get("data")
        .and_then(Value::as_object)
        .ok_or(SystemProviderKeyUsageClientError::InvalidResponse)?;
    Ok(SystemProviderKeyUsageClientResponse {
        status,
        usage_total_usd: optional_key_usage_decimal(data, "usage")?,
        usage_daily_usd: optional_key_usage_decimal(data, "usage_daily")?,
        usage_weekly_usd: optional_key_usage_decimal(data, "usage_weekly")?,
        usage_monthly_usd: optional_key_usage_decimal(data, "usage_monthly")?,
        limit_usd: optional_key_usage_decimal(data, "limit")?,
        limit_remaining_usd: optional_key_usage_decimal(data, "limit_remaining")?,
        is_free_tier: optional_key_usage_bool(data, "is_free_tier")?,
    })
}

fn decimal_string(value: Option<Decimal>) -> Option<String> {
    value.map(|value| value.normalize().to_string())
}

#[derive(Default)]
struct ReqwestSystemProviderConnectionClient;

impl SystemProviderConnectionClient for ReqwestSystemProviderConnectionClient {
    fn get(&self, request: SystemProviderConnectionRequest) -> SystemProviderConnectionFuture<'_> {
        Box::pin(async move {
            let client = crate::http_client::get();
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

    fn list_model_ids(
        &self,
        request: SystemProviderConnectionRequest,
    ) -> SystemProviderModelListFuture<'_> {
        Box::pin(async move {
            let client = crate::http_client::get();
            let mut builder = client.get(&request.url).timeout(Duration::from_secs(8));
            for (name, value) in &request.headers {
                let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                    .map_err(|_| ModelListClientError::ConnectionFailed)?;
                let value = reqwest::header::HeaderValue::from_str(value)
                    .map_err(|_| ModelListClientError::ConnectionFailed)?;
                builder = builder.header(name, value);
            }
            let response = builder
                .send()
                .await
                .map_err(|_| ModelListClientError::ConnectionFailed)?;
            let status = response.status().as_u16();
            if !(200..=299).contains(&status) {
                // Do not read an upstream error payload. Only the status is
                // needed to produce the public, fixed error code below.
                return Ok(ModelListClientResponse {
                    status,
                    model_ids: Vec::new(),
                });
            }
            let body = response
                .bytes()
                .await
                .map_err(|_| ModelListClientError::ConnectionFailed)?;
            let model_ids = parse_model_ids(&body)?;
            Ok(ModelListClientResponse { status, model_ids })
        })
    }

    fn fetch_key_usage(
        &self,
        request: SystemProviderConnectionRequest,
    ) -> SystemProviderKeyUsageFuture<'_> {
        Box::pin(async move {
            let client = crate::http_client::get();
            let mut builder = client.get(&request.url).timeout(Duration::from_secs(8));
            for (name, value) in &request.headers {
                let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                    .map_err(|_| SystemProviderKeyUsageClientError::ConnectionFailed)?;
                let value = reqwest::header::HeaderValue::from_str(value)
                    .map_err(|_| SystemProviderKeyUsageClientError::ConnectionFailed)?;
                builder = builder.header(name, value);
            }
            let response = builder
                .send()
                .await
                .map_err(|_| SystemProviderKeyUsageClientError::ConnectionFailed)?;
            let status = response.status().as_u16();
            if !(200..=299).contains(&status) {
                // Never read or expose an upstream error payload. The service
                // maps this status to a fixed public error code.
                return Ok(SystemProviderKeyUsageClientResponse::empty(status));
            }
            let body = response
                .bytes()
                .await
                .map_err(|_| SystemProviderKeyUsageClientError::ConnectionFailed)?;
            parse_key_usage_response(status, &body)
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

#[derive(Debug, Clone, Copy)]
enum SystemProviderEndpoint {
    ConnectionTest,
    KeyUsage,
    ModelList,
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
        endpoint: SystemProviderEndpoint,
    ) -> Result<SystemProviderConnectionRequest, AppError> {
        let base = credential.canonical_endpoint().trim_end_matches('/');
        let definition = system_provider_definition(credential.system_preset_key())
            .ok_or_else(|| AppError::Message("unsupported_auth".to_string()))?;
        let route_config = definition
            .route_config
            .as_ref()
            .ok_or_else(|| AppError::Message("unsupported_auth".to_string()))?;
        let path = match endpoint {
            SystemProviderEndpoint::ConnectionTest => definition.connection_test_path,
            SystemProviderEndpoint::KeyUsage => definition.key_usage_path,
            SystemProviderEndpoint::ModelList => definition.model_list_path,
        }
        .filter(|path| path.starts_with('/') && !path.starts_with("//"))
        .ok_or_else(|| AppError::Message("unsupported_auth".to_string()))?;
        let secret = std::str::from_utf8(credential.expose_secret())
            .map_err(|_| AppError::Message("credential_unavailable".to_string()))?;
        let url = format!("{base}{path}");
        let headers = match route_config
            .get("authMode")
            .and_then(serde_json::Value::as_str)
        {
            Some("bearer") => {
                vec![(
                    "authorization".to_string(),
                    Zeroizing::new(format!("Bearer {secret}")),
                )]
            }
            Some("x_api_key") => {
                vec![
                    ("x-api-key".to_string(), Zeroizing::new(secret.to_string())),
                    (
                        "anthropic-version".to_string(),
                        Zeroizing::new("2023-06-01".to_string()),
                    ),
                ]
            }
            _ => return Err(AppError::Message("unsupported_auth".to_string())),
        };
        Ok(SystemProviderConnectionRequest { url, headers })
    }

    pub async fn test(
        &self,
        key_id: &str,
        expected_version: u64,
    ) -> Result<SystemProviderConnectionTestResult, AppError> {
        let credential = self
            .credentials
            .resolve_provider_api_key(key_id, expected_version)
            .await?;
        let frozen_key_id = credential.key_id().to_string();
        let frozen_provider_id = credential.provider_id().to_string();
        let frozen_version = credential.credential_version();
        let request = Self::build_request(&credential, SystemProviderEndpoint::ConnectionTest)?;
        drop(credential);
        let (success, error_code) = match self.client.get(request).await {
            Ok(200..=299) => (true, None),
            Ok(401 | 403) => (false, Some("authentication_failed".to_string())),
            Ok(_) => (false, Some("upstream_rejected".to_string())),
            Err(_) => (false, Some("connection_failed".to_string())),
        };
        let status = if success { "success" } else { "failed" };
        let tested_at = self.db.record_provider_connection_test(
            &frozen_key_id,
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

    /// Fetch one built-in Provider's model catalogue while exposing only model
    /// ID strings. Raw success bodies and upstream error payloads never cross
    /// the client boundary.
    pub async fn list_models(
        &self,
        key_id: &str,
        expected_version: u64,
    ) -> Result<Vec<String>, AppError> {
        let credential = self
            .credentials
            .resolve_provider_api_key(key_id, expected_version)
            .await?;
        let request = Self::build_request(&credential, SystemProviderEndpoint::ModelList)?;
        drop(credential);

        match self.client.list_model_ids(request).await {
            Ok(ModelListClientResponse {
                status: 200..=299,
                model_ids,
            }) => Ok(model_ids),
            Ok(ModelListClientResponse {
                status: 401 | 403, ..
            }) => Err(AppError::Message("authentication_failed".to_string())),
            Ok(_) => Err(AppError::Message("upstream_rejected".to_string())),
            Err(ModelListClientError::InvalidResponse) => {
                Err(AppError::Message("invalid_response".to_string()))
            }
            Err(ModelListClientError::ConnectionFailed) => {
                Err(AppError::Message("connection_failed".to_string()))
            }
        }
    }

    /// Refresh a built-in Provider's key-scoped spend without allowing the raw
    /// upstream response to cross the connection-client boundary.
    pub async fn refresh_key_usage(
        &self,
        key_id: &str,
        expected_version: u64,
    ) -> Result<SystemProviderKeyUsageView, AppError> {
        let credential = self
            .credentials
            .resolve_provider_api_key(key_id, expected_version)
            .await?;
        let frozen_key_id = credential.key_id().to_string();
        let frozen_version = credential.credential_version();
        let request = Self::build_request(&credential, SystemProviderEndpoint::KeyUsage)?;
        drop(credential);

        let response = match self.client.fetch_key_usage(request).await {
            Ok(response) if (200..=299).contains(&response.status) => response,
            Ok(SystemProviderKeyUsageClientResponse {
                status: 401 | 403, ..
            }) => return Err(AppError::Message("authentication_failed".to_string())),
            Ok(_) => return Err(AppError::Message("upstream_rejected".to_string())),
            Err(SystemProviderKeyUsageClientError::InvalidResponse) => {
                return Err(AppError::Message("invalid_response".to_string()))
            }
            Err(SystemProviderKeyUsageClientError::ConnectionFailed) => {
                return Err(AppError::Message("connection_failed".to_string()))
            }
        };

        self.db
            .upsert_provider_key_usage_snapshot(&ProviderKeyUsageSnapshotInput {
                key_id: frozen_key_id,
                credential_version: frozen_version,
                usage_total_usd: decimal_string(response.usage_total_usd),
                usage_daily_usd: decimal_string(response.usage_daily_usd),
                usage_weekly_usd: decimal_string(response.usage_weekly_usd),
                usage_monthly_usd: decimal_string(response.usage_monthly_usd),
                limit_usd: decimal_string(response.limit_usd),
                limit_remaining_usd: decimal_string(response.limit_remaining_usd),
                is_free_tier: response.is_free_tier,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{CredentialStore, CredentialStoreError, SecretString};
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

    fn create_provider_key(db: &Database, provider_id: &str) -> String {
        db.create_provider_api_key(provider_id, "Test key").unwrap()
    }

    struct RecordingClient {
        requests: Mutex<Vec<SystemProviderConnectionRequest>>,
        results: Mutex<VecDeque<Result<u16, AppError>>>,
        model_results: Mutex<VecDeque<Result<ModelListClientResponse, ModelListClientError>>>,
        key_usage_results: Mutex<
            VecDeque<
                Result<SystemProviderKeyUsageClientResponse, SystemProviderKeyUsageClientError>,
            >,
        >,
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

        fn list_model_ids(
            &self,
            request: SystemProviderConnectionRequest,
        ) -> SystemProviderModelListFuture<'_> {
            self.requests.lock().unwrap().push(request);
            let result = self.model_results.lock().unwrap().pop_front().unwrap();
            Box::pin(async move { result })
        }

        fn fetch_key_usage(
            &self,
            request: SystemProviderConnectionRequest,
        ) -> SystemProviderKeyUsageFuture<'_> {
            self.requests.lock().unwrap().push(request);
            let result = self.key_usage_results.lock().unwrap().pop_front().unwrap();
            Box::pin(async move { result })
        }
    }

    #[tokio::test]
    async fn probes_only_canonical_read_only_endpoints_with_fixed_auth_headers() {
        for (provider_id, expected_url, auth_mode) in [
            (
                "system-openai-api",
                "https://api.openai.com/v1/models",
                "bearer",
            ),
            (
                "system-openrouter-api",
                "https://openrouter.ai/api/v1/key",
                "bearer",
            ),
            (
                "system-anthropic-api",
                "https://api.anthropic.com/v1/models",
                "x_api_key",
            ),
            (
                "system-gemini-api",
                "https://generativelanguage.googleapis.com/v1beta/openai/models",
                "bearer",
            ),
            ("system-xai-api", "https://api.x.ai/v1/models", "bearer"),
            (
                "system-deepseek-api",
                "https://api.deepseek.com/models",
                "bearer",
            ),
            (
                "system-kimi-api",
                "https://api.moonshot.cn/v1/models",
                "bearer",
            ),
            (
                "system-glm-api",
                "https://api.z.ai/api/paas/v4/models",
                "bearer",
            ),
            (
                "system-qwen-api",
                "https://dashscope.aliyuncs.com/compatible-mode/v1/models",
                "bearer",
            ),
            (
                "system-minimax-api",
                "https://api.minimaxi.com/v1/models",
                "bearer",
            ),
            (
                "system-mistral-api",
                "https://api.mistral.ai/v1/models",
                "bearer",
            ),
            (
                "system-groq-api",
                "https://api.groq.com/openai/v1/models",
                "bearer",
            ),
            (
                "system-together-api",
                "https://api.together.xyz/v1/models",
                "bearer",
            ),
            (
                "system-fireworks-api",
                "https://api.fireworks.ai/inference/v1/models",
                "bearer",
            ),
            (
                "system-perplexity-api",
                "https://api.perplexity.ai/v1/async/sonar",
                "bearer",
            ),
            (
                "system-siliconflow-api",
                "https://api.siliconflow.cn/v1/models",
                "bearer",
            ),
            (
                "system-nvidia-nim-api",
                "https://integrate.api.nvidia.com/v1/models",
                "bearer",
            ),
            (
                "system-cerebras-api",
                "https://api.cerebras.ai/v1/models",
                "bearer",
            ),
        ] {
            let key = format!("{provider_id}-connection-secret");
            let expected_model_url = match provider_id {
                "system-openrouter-api" => "https://openrouter.ai/api/v1/models",
                "system-perplexity-api" => "https://api.perplexity.ai/v1/models",
                _ => expected_url,
            };
            let db = Arc::new(Database::memory().unwrap());
            db.set_usage_provider_enabled(provider_id, true).unwrap();
            let credentials = Arc::new(BindingCredentialService::new(
                db.clone(),
                Arc::new(MemoryStore::default()),
            ));
            let key_id = create_provider_key(&db, provider_id);
            credentials
                .set_provider_api_key(&key_id, 0, SecretString::new(key.clone()))
                .await
                .unwrap();
            let client = Arc::new(RecordingClient {
                requests: Mutex::new(Vec::new()),
                results: Mutex::new(vec![Ok(200)].into()),
                model_results: Mutex::new(
                    vec![Ok(ModelListClientResponse {
                        status: 200,
                        model_ids: vec!["listed-model".to_string()],
                    })]
                    .into(),
                ),
                key_usage_results: Mutex::new(VecDeque::new()),
            });
            let service =
                SystemProviderConnectionService::new(db.clone(), credentials, client.clone());
            let result = service.test(&key_id, 1).await.unwrap();
            assert!(result.success);
            assert_eq!(result.status, "success");
            assert_eq!(result.error_code, None);
            assert!(!serde_json::to_string(&result).unwrap().contains(&key));
            let model_ids = service.list_models(&key_id, 1).await.unwrap();
            assert_eq!(model_ids, vec!["listed-model"]);
            assert!(!serde_json::to_string(&model_ids).unwrap().contains(&key));

            let requests = client.requests.lock().unwrap();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[0].url, expected_url);
            assert_eq!(requests[1].url, expected_model_url);
            let expected_headers = if auth_mode == "x_api_key" {
                vec![
                    ("x-api-key".to_string(), key.clone()),
                    ("anthropic-version".to_string(), "2023-06-01".to_string()),
                ]
            } else {
                vec![("authorization".to_string(), format!("Bearer {key}"))]
            };
            assert_eq!(requests[0].headers_for_test(), expected_headers);
            assert_eq!(requests[1].headers_for_test(), expected_headers);
            let provider = db
                .list_usage_providers()
                .unwrap()
                .into_iter()
                .find(|provider| provider.id == provider_id)
                .unwrap();
            assert_eq!(
                provider
                    .api_keys
                    .iter()
                    .find(|provider_key| provider_key.id == key_id)
                    .unwrap()
                    .last_connection_test_status
                    .as_deref(),
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
            let key_id = create_provider_key(&db, "system-openrouter-api");
            credentials
                .set_provider_api_key(
                    &key_id,
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
                    model_results: Mutex::new(VecDeque::new()),
                    key_usage_results: Mutex::new(VecDeque::new()),
                }),
            );
            let result = service.test(&key_id, 1).await.unwrap();
            assert!(!result.success);
            assert_eq!(result.error_code.as_deref(), Some(expected_code));
            let json = serde_json::to_string(&result).unwrap();
            assert!(!json.contains("secret upstream"));
            assert!(!json.contains("redacted-connection-key"));
        }
    }

    #[test]
    fn model_list_parser_accepts_configured_response_shapes() {
        for (body, expected) in [
            (
                br#"{"data":[{"id":"openai-model"}]}"#.as_slice(),
                vec!["openai-model"],
            ),
            (
                br#"[{"id":"together-model"}]"#.as_slice(),
                vec!["together-model"],
            ),
            (
                br#"{"models":[{"id":"models-key-model"}]}"#.as_slice(),
                vec!["models-key-model"],
            ),
        ] {
            assert_eq!(parse_model_ids(body).unwrap(), expected);
        }
    }

    #[test]
    fn key_usage_parser_accepts_representative_openrouter_payload() {
        let parsed = parse_key_usage_response(
            200,
            br#"{"data":{"usage":12.345,"usage_daily":0.5,"usage_weekly":2.75,"usage_monthly":8.25,"limit":100,"limit_remaining":87.655,"is_free_tier":false,"label":"ignored"}}"#,
        )
        .unwrap();

        assert_eq!(parsed.status, 200);
        assert_eq!(
            parsed.usage_total_usd,
            Some(Decimal::from_str("12.345").unwrap())
        );
        assert_eq!(
            parsed.usage_daily_usd,
            Some(Decimal::from_str("0.5").unwrap())
        );
        assert_eq!(
            parsed.usage_weekly_usd,
            Some(Decimal::from_str("2.75").unwrap())
        );
        assert_eq!(
            parsed.usage_monthly_usd,
            Some(Decimal::from_str("8.25").unwrap())
        );
        assert_eq!(parsed.limit_usd, Some(Decimal::from_str("100").unwrap()));
        assert_eq!(
            parsed.limit_remaining_usd,
            Some(Decimal::from_str("87.655").unwrap())
        );
        assert_eq!(parsed.is_free_tier, Some(false));
    }

    #[test]
    fn key_usage_parser_preserves_null_unlimited_fields() {
        let parsed = parse_key_usage_response(
            200,
            br#"{"data":{"usage":3.5,"limit":null,"limit_remaining":null,"is_free_tier":true}}"#,
        )
        .unwrap();

        assert_eq!(
            parsed.usage_total_usd,
            Some(Decimal::from_str("3.5").unwrap())
        );
        assert_eq!(parsed.limit_usd, None);
        assert_eq!(parsed.limit_remaining_usd, None);
        assert_eq!(parsed.is_free_tier, Some(true));
    }

    #[test]
    fn key_usage_parser_preserves_missing_optional_fields() {
        let parsed = parse_key_usage_response(200, br#"{"data":{"usage":"1.25"}}"#).unwrap();

        assert_eq!(
            parsed.usage_total_usd,
            Some(Decimal::from_str("1.25").unwrap())
        );
        assert_eq!(parsed.usage_daily_usd, None);
        assert_eq!(parsed.usage_weekly_usd, None);
        assert_eq!(parsed.usage_monthly_usd, None);
        assert_eq!(parsed.limit_usd, None);
        assert_eq!(parsed.limit_remaining_usd, None);
        assert_eq!(parsed.is_free_tier, None);
    }

    #[test]
    fn key_usage_parser_rejects_malformed_bodies() {
        for body in [
            br#"not-json"#.as_slice(),
            br#"{"data":{"usage":{"raw":"secret"}}}"#.as_slice(),
            br#"{"unexpected":{}}"#.as_slice(),
        ] {
            assert_eq!(
                parse_key_usage_response(200, body),
                Err(SystemProviderKeyUsageClientError::InvalidResponse)
            );
        }
    }

    struct RawBodyClient {
        requests: Mutex<Vec<SystemProviderConnectionRequest>>,
        status: u16,
        body: Vec<u8>,
    }

    impl SystemProviderConnectionClient for RawBodyClient {
        fn get(
            &self,
            request: SystemProviderConnectionRequest,
        ) -> SystemProviderConnectionFuture<'_> {
            self.requests.lock().unwrap().push(request);
            let status = self.status;
            Box::pin(async move { Ok(status) })
        }

        fn list_model_ids(
            &self,
            request: SystemProviderConnectionRequest,
        ) -> SystemProviderModelListFuture<'_> {
            self.requests.lock().unwrap().push(request);
            let status = self.status;
            let model_ids = parse_model_ids(&self.body);
            Box::pin(async move {
                model_ids.map(|model_ids| ModelListClientResponse { status, model_ids })
            })
        }

        fn fetch_key_usage(
            &self,
            request: SystemProviderConnectionRequest,
        ) -> SystemProviderKeyUsageFuture<'_> {
            self.requests.lock().unwrap().push(request);
            let status = self.status;
            if !(200..=299).contains(&status) {
                return Box::pin(
                    async move { Ok(SystemProviderKeyUsageClientResponse::empty(status)) },
                );
            }
            let key_usage = parse_key_usage_response(status, &self.body);
            Box::pin(async move { key_usage })
        }
    }

    #[tokio::test]
    async fn model_list_command_output_contains_only_ids_and_no_secrets() {
        let api_key = "sk-provider-key-must-not-leak";
        let body_secret = "sk-upstream-body-secret-must-not-leak";
        let db = Arc::new(Database::memory().unwrap());
        let credentials = Arc::new(BindingCredentialService::new(
            db.clone(),
            Arc::new(MemoryStore::default()),
        ));
        let key_id = create_provider_key(&db, "system-openai-api");
        credentials
            .set_provider_api_key(&key_id, 0, SecretString::new(api_key.to_string()))
            .await
            .unwrap();
        let raw_body = format!(
            r#"{{"data":[{{"id":"gpt-safe","owner":"{body_secret}"}},{{"id":"claude-safe"}}],"debug":"{body_secret}"}}"#
        );
        let client = Arc::new(RawBodyClient {
            requests: Mutex::new(Vec::new()),
            status: 200,
            body: raw_body.into_bytes(),
        });
        let service = SystemProviderConnectionService::new(db, credentials, client.clone());

        let command_output = service.list_models(&key_id, 1).await.unwrap();
        assert_eq!(command_output, vec!["gpt-safe", "claude-safe"]);
        let serialized = serde_json::to_string(&command_output).unwrap();
        assert!(!serialized.contains(api_key));
        assert!(!serialized.contains(body_secret));
        assert!(!serialized.contains("owner"));
        assert!(!serialized.contains("debug"));

        let requests = client.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, "https://api.openai.com/v1/models");
    }

    #[tokio::test]
    async fn key_usage_maps_status_and_client_failures_to_fixed_public_codes() {
        for (client_result, expected_code) in [
            (
                Ok(SystemProviderKeyUsageClientResponse::empty(401)),
                "authentication_failed",
            ),
            (
                Ok(SystemProviderKeyUsageClientResponse::empty(403)),
                "authentication_failed",
            ),
            (
                Ok(SystemProviderKeyUsageClientResponse::empty(500)),
                "upstream_rejected",
            ),
            (
                Err(SystemProviderKeyUsageClientError::ConnectionFailed),
                "connection_failed",
            ),
            (
                Err(SystemProviderKeyUsageClientError::InvalidResponse),
                "invalid_response",
            ),
        ] {
            let db = Arc::new(Database::memory().unwrap());
            let credentials = Arc::new(BindingCredentialService::new(
                db.clone(),
                Arc::new(MemoryStore::default()),
            ));
            let key_id = create_provider_key(&db, "system-openrouter-api");
            credentials
                .set_provider_api_key(
                    &key_id,
                    0,
                    SecretString::new("key-usage-status-secret".to_string()),
                )
                .await
                .unwrap();
            let service = SystemProviderConnectionService::new(
                db,
                credentials,
                Arc::new(RecordingClient {
                    requests: Mutex::new(Vec::new()),
                    results: Mutex::new(VecDeque::new()),
                    model_results: Mutex::new(VecDeque::new()),
                    key_usage_results: Mutex::new(vec![client_result].into()),
                }),
            );

            assert_eq!(
                service
                    .refresh_key_usage(&key_id, 1)
                    .await
                    .unwrap_err()
                    .to_string(),
                expected_code
            );
        }
    }

    #[tokio::test]
    async fn key_usage_output_contains_only_parsed_fields_and_uses_dedicated_path() {
        let api_key = "sk-key-usage-provider-secret";
        let body_secret = "sk-key-usage-body-secret";
        let db = Arc::new(Database::memory().unwrap());
        let credentials = Arc::new(BindingCredentialService::new(
            db.clone(),
            Arc::new(MemoryStore::default()),
        ));
        let key_id = create_provider_key(&db, "system-openrouter-api");
        credentials
            .set_provider_api_key(&key_id, 0, SecretString::new(api_key.to_string()))
            .await
            .unwrap();
        let raw_body = format!(
            r#"{{"data":{{"usage":4.25,"limit":null,"limit_remaining":null,"is_free_tier":false,"debug":"{body_secret}"}},"raw":"{body_secret}"}}"#
        );
        let client = Arc::new(RawBodyClient {
            requests: Mutex::new(Vec::new()),
            status: 200,
            body: raw_body.into_bytes(),
        });
        let service = SystemProviderConnectionService::new(db.clone(), credentials, client.clone());

        let view = service.refresh_key_usage(&key_id, 1).await.unwrap();
        assert_eq!(view.usage_total_usd.as_deref(), Some("4.25"));
        assert_eq!(view.limit_usd, None);
        assert_eq!(view.limit_remaining_usd, None);
        assert!(!view.stale);
        let serialized = serde_json::to_string(&view).unwrap();
        assert!(!serialized.contains(api_key));
        assert!(!serialized.contains(body_secret));
        assert!(!serialized.contains("debug"));
        assert!(!serialized.contains("raw"));

        let provider = db
            .list_usage_providers()
            .unwrap()
            .into_iter()
            .find(|provider| provider.id == "system-openrouter-api")
            .unwrap();
        assert_eq!(
            provider
                .api_keys
                .into_iter()
                .find(|key| key.id == key_id)
                .unwrap()
                .key_usage,
            Some(view)
        );
        let requests = client.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, "https://openrouter.ai/api/v1/key");
        assert_eq!(
            requests[0].headers_for_test(),
            vec![("authorization".to_string(), format!("Bearer {api_key}"))]
        );
    }
}
