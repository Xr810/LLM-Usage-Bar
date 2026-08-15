use super::lifecycle_lock::{CredentialLifecycleLock, SharedLifecycleGuard};
use super::{CredentialStore, SecretString, KEYCHAIN_SERVICE};
use crate::error::AppError;
use crate::model::{
    AgentProviderBindingInput, AgentProviderBindingView, BindingCredentialStatus,
    LocalBindingKeyReveal, ProviderApiKeyView, SystemProviderAuthKind, UsageProviderView,
};
use crate::store::{
    BindingAuthMode, CredentialBindingSnapshot, CredentialJournalEntry, CredentialMutationKind,
    CredentialOperationReservation, Database, ProviderCredentialJournalEntry,
    ProviderCredentialOperationReservation, ProviderCredentialSnapshot,
};
use crate::usage::system_providers::{is_fixed_api_preset, system_binding_route_protocol};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

const FINGERPRINT_SEPARATOR: &[u8] = b"\0";
const PROVIDER_FINGERPRINT_DOMAIN: &[u8] = b"com.xr810.llm-usage-bar.provider-upstream.v1\0";
const MIN_BINDING_CREDENTIAL_BYTES: usize = 16;
const MAX_BINDING_CREDENTIAL_BYTES: usize = 4096;
const MIN_BINDING_CREDENTIAL_DISTINCT_BYTES: usize = 4;
fn public_error(code: &'static str) -> AppError {
    AppError::Message(code.to_string())
}

fn normalize_db_error(error: AppError) -> AppError {
    if let AppError::Message(code) = error {
        if matches!(
            code.as_str(),
            "binding_not_found"
                | "invalid_binding"
                | "credential_required"
                | "credential_conflict"
                | "credential_unavailable"
                | "unsupported_auth"
        ) {
            return AppError::Message(code);
        }
    }
    log::error!("credential database operation failed");
    public_error("credential_unavailable")
}

fn credential_fingerprint(secret: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(KEYCHAIN_SERVICE.as_bytes());
    hasher.update(FINGERPRINT_SEPARATOR);
    hasher.update(secret);
    hasher.finalize().into()
}

fn provider_credential_fingerprint(secret: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(PROVIDER_FINGERPRINT_DOMAIN);
    hasher.update(secret);
    hasher.finalize().into()
}

fn provider_credential_is_acceptable(secret: &[u8]) -> bool {
    (1..=MAX_BINDING_CREDENTIAL_BYTES).contains(&secret.len())
        && secret.iter().all(u8::is_ascii_graphic)
}

fn generate_local_binding_key() -> SecretString {
    SecretString::new(format!(
        "lub_{}_{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple(),
    ))
}

/// Binding credentials are forwarded through HTTP authentication headers and
/// also select a local route. Reject trivially enumerable or ambiguous values
/// whose ordinary runtime rendering (for example `200`) could collide with
/// status, latency, or token diagnostics despite credential-aware sink guards.
fn binding_credential_is_acceptable(secret: &[u8]) -> bool {
    if !(MIN_BINDING_CREDENTIAL_BYTES..=MAX_BINDING_CREDENTIAL_BYTES).contains(&secret.len())
        || !secret.iter().all(u8::is_ascii_graphic)
    {
        return false;
    }
    let mut seen = [false; 256];
    let mut distinct = 0;
    for byte in secret {
        let slot = &mut seen[usize::from(*byte)];
        if !*slot {
            *slot = true;
            distinct += 1;
            if distinct >= MIN_BINDING_CREDENTIAL_DISTINCT_BYTES {
                return true;
            }
        }
    }
    false
}

/// Verified, frozen Provider credential used only for a fixed-endpoint
/// connection probe. It is intentionally non-Clone and non-serializable.
pub(crate) struct ResolvedProviderCredential {
    key_id: String,
    provider_id: String,
    system_preset_key: String,
    canonical_endpoint: String,
    credential_version: u64,
    secret: Zeroizing<Vec<u8>>,
}

impl ResolvedProviderCredential {
    pub(crate) fn key_id(&self) -> &str {
        &self.key_id
    }

    pub(crate) fn provider_id(&self) -> &str {
        &self.provider_id
    }

    pub(crate) fn system_preset_key(&self) -> &str {
        &self.system_preset_key
    }

    pub(crate) fn canonical_endpoint(&self) -> &str {
        &self.canonical_endpoint
    }

    pub(crate) fn credential_version(&self) -> u64 {
        self.credential_version
    }

    pub(crate) fn expose_secret(&self) -> &[u8] {
        self.secret.as_slice()
    }
}

impl fmt::Debug for ResolvedProviderCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ResolvedProviderCredential([REDACTED])")
    }
}

pub struct BindingCredentialService {
    db: Arc<Database>,
    store: Arc<dyn CredentialStore>,
    lifecycle_lock: CredentialLifecycleLock,
}

impl BindingCredentialService {
    pub fn new(db: Arc<Database>, store: Arc<dyn CredentialStore>) -> Self {
        let lifecycle_lock = CredentialLifecycleLock::new(&db);
        Self {
            db,
            store,
            lifecycle_lock,
        }
    }

    async fn store_put(
        &self,
        slot: String,
        secret: Zeroizing<Vec<u8>>,
        lifecycle_guard: Arc<SharedLifecycleGuard>,
    ) -> Result<(), ()> {
        let store = self.store.clone();
        match tokio::task::spawn_blocking(move || {
            let _lifecycle_guard = lifecycle_guard;
            store.put(&slot, secret.as_slice())
        })
        .await
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) | Err(_) => {
                log::error!("credential store put failed");
                Err(())
            }
        }
    }

    async fn store_get(&self, slot: String) -> Result<Option<Zeroizing<Vec<u8>>>, ()> {
        let store = self.store.clone();
        match tokio::task::spawn_blocking(move || {
            store.get(&slot).map(|secret| secret.map(Zeroizing::new))
        })
        .await
        {
            Ok(Ok(secret)) => Ok(secret),
            Ok(Err(_)) | Err(_) => {
                log::error!("credential store get failed");
                Err(())
            }
        }
    }

    async fn store_delete(&self, slot: String) -> Result<(), ()> {
        let store = self.store.clone();
        match tokio::task::spawn_blocking(move || store.delete(&slot)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) | Err(_) => {
                log::error!("credential store delete failed");
                Err(())
            }
        }
    }

    async fn cleanup_staging_after_failure(&self, reservation: &CredentialOperationReservation) {
        let Some(staging_slot) = reservation.staging_slot.as_deref() else {
            if self
                .db
                .delete_pending_journal_entry(
                    &reservation.operation_id,
                    &reservation.binding_id,
                    reservation.generation,
                )
                .is_err()
            {
                log::error!("credential pending journal cleanup failed");
            }
            return;
        };
        match self.db.claim_pending_staging_cleanup(reservation) {
            Ok(false) => return,
            Err(_) => {
                log::error!("credential staging cleanup claim failed");
                return;
            }
            Ok(true) => {}
        }
        match self.db.credential_slot_is_active(staging_slot) {
            Ok(false) => {}
            Ok(true) | Err(_) => {
                log::error!("credential staging cleanup safety check failed");
                return;
            }
        }
        if self.store_delete(staging_slot.to_string()).await.is_ok()
            && self.db.finish_claimed_staging_cleanup(reservation).is_err()
        {
            log::error!("credential pending journal cleanup failed");
        }
    }

    async fn finish_published_operation(
        &self,
        reservation: &CredentialOperationReservation,
    ) -> Result<(), ()> {
        if let Some(previous_slot) = reservation.previous_slot.as_deref() {
            match self.db.credential_slot_is_active(previous_slot) {
                Ok(true) => return Err(()),
                Err(_) => {
                    log::error!("credential cleanup safety check failed");
                    return Err(());
                }
                Ok(false) => {}
            }
            self.store_delete(previous_slot.to_string()).await?;
        }
        self.db
            .finish_credential_operation(reservation)
            .map_err(|_| {
                log::error!("credential journal finalization failed");
            })
    }

    async fn cleanup_provider_staging_after_failure(
        &self,
        reservation: &ProviderCredentialOperationReservation,
    ) {
        let Some(staging_slot) = reservation.staging_slot.as_deref() else {
            if self
                .db
                .delete_pending_provider_journal_entry(
                    &reservation.operation_id,
                    &reservation.key_id,
                    reservation.generation,
                )
                .is_err()
            {
                log::error!("provider credential pending journal cleanup failed");
            }
            return;
        };
        match self.db.claim_pending_provider_staging_cleanup(reservation) {
            Ok(false) => return,
            Err(_) => {
                log::error!("provider credential staging cleanup claim failed");
                return;
            }
            Ok(true) => {}
        }
        match self.db.provider_credential_slot_is_active(staging_slot) {
            Ok(false) => {}
            Ok(true) | Err(_) => {
                log::error!("provider credential staging cleanup safety check failed");
                return;
            }
        }
        if self.store_delete(staging_slot.to_string()).await.is_ok()
            && self
                .db
                .finish_claimed_provider_staging_cleanup(reservation)
                .is_err()
        {
            log::error!("provider credential pending journal cleanup failed");
        }
    }

    async fn finish_published_provider_operation(
        &self,
        reservation: &ProviderCredentialOperationReservation,
    ) -> Result<(), ()> {
        if let Some(previous_slot) = reservation.previous_slot.as_deref() {
            match self.db.provider_credential_slot_is_active(previous_slot) {
                Ok(true) => return Err(()),
                Err(_) => {
                    log::error!("provider credential cleanup safety check failed");
                    return Err(());
                }
                Ok(false) => {}
            }
            self.store_delete(previous_slot.to_string()).await?;
        }
        self.db
            .finish_provider_credential_operation(reservation)
            .map_err(|_| {
                log::error!("provider credential journal finalization failed");
            })
    }

    async fn mutate_provider_api_key(
        &self,
        key_id: &str,
        expected_version: u64,
        api_key: SecretString,
        kind: CredentialMutationKind,
    ) -> Result<ProviderApiKeyView, AppError> {
        if !provider_credential_is_acceptable(api_key.expose_bytes()) {
            return Err(public_error("credential_required"));
        }
        let lifecycle_guard = Arc::new(self.lifecycle_lock.shared().await.map_err(|_| {
            log::error!("credential lifecycle lock failed");
            public_error("credential_unavailable")
        })?);
        let fingerprint = provider_credential_fingerprint(api_key.expose_bytes());
        let reservation = self
            .db
            .reserve_provider_credential_operation(
                key_id,
                expected_version,
                kind,
                Some(&fingerprint),
            )
            .map_err(normalize_db_error)?;
        let staging_slot = reservation
            .staging_slot
            .as_deref()
            .ok_or_else(|| public_error("credential_unavailable"))?
            .to_string();
        let protected_secret = Zeroizing::new(api_key.expose_bytes().to_vec());
        if self
            .store_put(staging_slot, protected_secret, lifecycle_guard.clone())
            .await
            .is_err()
        {
            self.cleanup_provider_staging_after_failure(&reservation)
                .await;
            return Err(public_error("credential_unavailable"));
        }
        if let Err(error) = self
            .db
            .publish_provider_credential_operation(&reservation, Some(&fingerprint))
        {
            self.cleanup_provider_staging_after_failure(&reservation)
                .await;
            return Err(normalize_db_error(error));
        }
        let _ = self.finish_published_provider_operation(&reservation).await;
        self.provider_key_view(key_id).await
    }

    pub async fn set_provider_api_key(
        &self,
        key_id: &str,
        expected_version: u64,
        api_key: SecretString,
    ) -> Result<ProviderApiKeyView, AppError> {
        self.mutate_provider_api_key(
            key_id,
            expected_version,
            api_key,
            CredentialMutationKind::Set,
        )
        .await
    }

    pub async fn replace_provider_api_key(
        &self,
        key_id: &str,
        expected_version: u64,
        api_key: SecretString,
    ) -> Result<ProviderApiKeyView, AppError> {
        self.mutate_provider_api_key(
            key_id,
            expected_version,
            api_key,
            CredentialMutationKind::Replace,
        )
        .await
    }

    pub async fn clear_provider_api_key(
        &self,
        key_id: &str,
        expected_version: u64,
    ) -> Result<ProviderApiKeyView, AppError> {
        let _lifecycle_guard = self.lifecycle_lock.shared().await.map_err(|_| {
            log::error!("credential lifecycle lock failed");
            public_error("credential_unavailable")
        })?;
        let reservation = self
            .db
            .reserve_provider_credential_operation(
                key_id,
                expected_version,
                CredentialMutationKind::Clear,
                None,
            )
            .map_err(normalize_db_error)?;
        if let Err(error) = self
            .db
            .publish_provider_credential_operation(&reservation, None)
        {
            self.cleanup_provider_staging_after_failure(&reservation)
                .await;
            return Err(normalize_db_error(error));
        }
        let _ = self.finish_published_provider_operation(&reservation).await;
        self.provider_key_view(key_id).await
    }

    pub(crate) async fn resolve_provider_api_key(
        &self,
        key_id: &str,
        expected_version: u64,
    ) -> Result<ResolvedProviderCredential, AppError> {
        let initial = self
            .db
            .provider_credential_snapshot(key_id)
            .map_err(normalize_db_error)?
            .ok_or_else(|| public_error("unsupported_auth"))?;
        if initial.credential_version != expected_version {
            return Err(public_error("credential_conflict"));
        }
        let fingerprint = initial
            .fingerprint
            .as_deref()
            .filter(|fingerprint| fingerprint.len() == 32)
            .ok_or_else(|| public_error("credential_unavailable"))?;
        let slot = initial
            .credential_slot
            .as_deref()
            .ok_or_else(|| public_error("credential_unavailable"))?
            .to_string();
        let secret = self
            .store_get(slot.clone())
            .await
            .map_err(|_| public_error("credential_unavailable"))?
            .ok_or_else(|| public_error("credential_unavailable"))?;
        if !provider_credential_is_acceptable(secret.as_slice()) {
            return Err(public_error("credential_unavailable"));
        }
        let actual = provider_credential_fingerprint(secret.as_slice());
        if !bool::from(actual.as_slice().ct_eq(fingerprint)) {
            return Err(public_error("credential_unavailable"));
        }
        let current = self
            .db
            .provider_credential_snapshot(key_id)
            .map_err(normalize_db_error)?
            .ok_or_else(|| public_error("unsupported_auth"))?;
        if current.credential_version != initial.credential_version
            || current.credential_slot.as_deref() != Some(slot.as_str())
            || current.fingerprint != initial.fingerprint
        {
            return Err(public_error("credential_conflict"));
        }
        // The key row is the only thing that knows which Provider this secret
        // belongs to, and it is read after the credential re-check so a key
        // moved or removed mid-flight cannot resolve against a stale Provider.
        let owning_provider_id = self
            .db
            .provider_api_key(key_id)
            .map_err(normalize_db_error)?
            .map(|key| key.provider_id)
            .ok_or_else(|| public_error("unsupported_auth"))?;
        let provider = self
            .db
            .list_usage_providers()
            .map_err(normalize_db_error)?
            .into_iter()
            .find(|provider| provider.id == owning_provider_id)
            .filter(|provider| {
                provider.enabled
                    && provider.system_auth_kind == Some(SystemProviderAuthKind::ProviderApiKey)
            })
            .ok_or_else(|| public_error("unsupported_auth"))?;
        Ok(ResolvedProviderCredential {
            key_id: key_id.to_string(),
            provider_id: provider.id,
            system_preset_key: provider
                .system_preset_key
                .ok_or_else(|| public_error("unsupported_auth"))?,
            canonical_endpoint: provider
                .canonical_endpoint
                .ok_or_else(|| public_error("unsupported_auth"))?,
            credential_version: current.credential_version,
            secret,
        })
    }

    async fn provider_credential_status(
        &self,
        snapshot: &ProviderCredentialSnapshot,
    ) -> BindingCredentialStatus {
        let (Some(fingerprint), Some(slot)) = (
            snapshot.fingerprint.as_deref(),
            snapshot.credential_slot.as_deref(),
        ) else {
            return if snapshot.fingerprint.is_none() && snapshot.credential_slot.is_none() {
                BindingCredentialStatus::Missing
            } else {
                BindingCredentialStatus::Unavailable
            };
        };
        if fingerprint.len() != 32 || snapshot.credential_version == 0 {
            return BindingCredentialStatus::Unavailable;
        }
        let Ok(Some(secret)) = self.store_get(slot.to_string()).await else {
            return BindingCredentialStatus::Unavailable;
        };
        let actual = provider_credential_fingerprint(secret.as_slice());
        if bool::from(actual.as_slice().ct_eq(fingerprint)) {
            BindingCredentialStatus::Configured
        } else {
            BindingCredentialStatus::Unavailable
        }
    }

    /// The view of one key after a mutation. Status is probed against the real
    /// keychain the same way the Provider-level view does, so a write that
    /// landed in the database but not in the keychain reports `Unavailable`
    /// rather than a confident `Configured`.
    pub(crate) async fn provider_key_view(
        &self,
        key_id: &str,
    ) -> Result<ProviderApiKeyView, AppError> {
        let key = self
            .db
            .provider_api_key(key_id)
            .map_err(normalize_db_error)?
            .ok_or_else(|| public_error("unsupported_auth"))?;
        let snapshot = ProviderCredentialSnapshot {
            fingerprint: key.fingerprint.clone(),
            credential_slot: key.credential_slot.clone(),
            credential_version: key.credential_version,
        };
        let can_clear_credential =
            snapshot.fingerprint.is_some() && snapshot.credential_slot.is_some();
        let credential_status = self.provider_credential_status(&snapshot).await;
        let key_usage = self
            .db
            .provider_key_usage_view(key_id, key.credential_version)
            .map_err(normalize_db_error)?;
        Ok(ProviderApiKeyView {
            id: key.id,
            provider_id: key.provider_id,
            label: key.label,
            can_clear_credential,
            credential_status,
            credential_version: key.credential_version,
            last_connection_test_at: key.last_test_at,
            last_connection_test_status: key.last_test_status,
            last_connection_test_error_code: key.last_test_error_code,
            sort_order: key.sort_order,
            key_usage,
        })
    }

    pub async fn list_usage_providers(&self) -> Result<Vec<UsageProviderView>, AppError> {
        let mut providers = self.db.list_usage_providers().map_err(normalize_db_error)?;
        for provider in &mut providers {
            if provider.system_auth_kind != Some(SystemProviderAuthKind::ProviderApiKey) {
                continue;
            }
            for key in &mut provider.api_keys {
                let snapshot = self
                    .db
                    .provider_credential_snapshot(&key.id)
                    .map_err(normalize_db_error)?
                    .ok_or_else(|| public_error("credential_unavailable"))?;
                key.credential_status = self.provider_credential_status(&snapshot).await;
                key.can_clear_credential =
                    snapshot.fingerprint.is_some() && snapshot.credential_slot.is_some();
            }
        }
        let verified_bindings = self.list_agent_provider_bindings(None).await?;
        for provider in &mut providers {
            provider.bindings = verified_bindings
                .iter()
                .filter(|binding| binding.provider_id == provider.id)
                .cloned()
                .collect();
        }
        Ok(providers)
    }

    async fn publish_api_key_mutation(
        &self,
        binding_id: &str,
        expected_version: u64,
        api_key: SecretString,
        kind: CredentialMutationKind,
    ) -> Result<(), AppError> {
        if !binding_credential_is_acceptable(api_key.expose_bytes()) {
            return Err(public_error("credential_required"));
        }
        let lifecycle_guard = Arc::new(self.lifecycle_lock.shared().await.map_err(|_| {
            log::error!("credential lifecycle lock failed");
            public_error("credential_unavailable")
        })?);
        let fingerprint = credential_fingerprint(api_key.expose_bytes());
        let reservation = self
            .db
            .reserve_credential_operation(binding_id, expected_version, kind, Some(&fingerprint))
            .map_err(normalize_db_error)?;
        let staging_slot = reservation
            .staging_slot
            .as_deref()
            .ok_or_else(|| public_error("credential_unavailable"))?
            .to_string();
        let protected_secret = Zeroizing::new(api_key.expose_bytes().to_vec());
        if self
            .store_put(staging_slot, protected_secret, lifecycle_guard.clone())
            .await
            .is_err()
        {
            self.cleanup_staging_after_failure(&reservation).await;
            return Err(public_error("credential_unavailable"));
        }
        if let Err(error) = self
            .db
            .publish_credential_operation(&reservation, Some(&fingerprint))
        {
            self.cleanup_staging_after_failure(&reservation).await;
            return Err(normalize_db_error(error));
        }
        // The new generation is already authoritative. Old-slot cleanup is
        // retryable and must not make callers retry the mutation with a stale
        // expected version.
        let _ = self.finish_published_operation(&reservation).await;
        Ok(())
    }

    async fn mutate_api_key(
        &self,
        binding_id: &str,
        expected_version: u64,
        api_key: SecretString,
        kind: CredentialMutationKind,
    ) -> Result<AgentProviderBindingView, AppError> {
        self.publish_api_key_mutation(binding_id, expected_version, api_key, kind)
            .await?;
        self.binding_view(binding_id).await
    }

    pub async fn set_binding_api_key(
        &self,
        binding_id: &str,
        expected_version: u64,
        api_key: SecretString,
    ) -> Result<AgentProviderBindingView, AppError> {
        self.mutate_api_key(
            binding_id,
            expected_version,
            api_key,
            CredentialMutationKind::Set,
        )
        .await
    }

    pub async fn replace_binding_api_key(
        &self,
        binding_id: &str,
        expected_version: u64,
        api_key: SecretString,
    ) -> Result<AgentProviderBindingView, AppError> {
        self.mutate_api_key(
            binding_id,
            expected_version,
            api_key,
            CredentialMutationKind::Replace,
        )
        .await
    }

    pub async fn clear_binding_api_key(
        &self,
        binding_id: &str,
        expected_version: u64,
    ) -> Result<AgentProviderBindingView, AppError> {
        let _lifecycle_guard = self.lifecycle_lock.shared().await.map_err(|_| {
            log::error!("credential lifecycle lock failed");
            public_error("credential_unavailable")
        })?;
        let reservation = self
            .db
            .reserve_credential_operation(
                binding_id,
                expected_version,
                CredentialMutationKind::Clear,
                None,
            )
            .map_err(normalize_db_error)?;
        if let Err(error) = self.db.publish_credential_operation(&reservation, None) {
            self.cleanup_staging_after_failure(&reservation).await;
            return Err(normalize_db_error(error));
        }
        let _ = self.finish_published_operation(&reservation).await;
        match self.binding_view(binding_id).await {
            Ok(view) => Ok(view),
            Err(_) => {
                log::error!("credential binding view failed closed");
                self.db
                    .credential_binding_fail_closed_view(binding_id)
                    .map_err(normalize_db_error)
            }
        }
    }

    pub async fn delete_binding(
        &self,
        binding_id: &str,
        expected_version: u64,
    ) -> Result<(), AppError> {
        let _lifecycle_guard = self.lifecycle_lock.shared().await.map_err(|_| {
            log::error!("credential lifecycle lock failed");
            public_error("credential_unavailable")
        })?;
        let reservation = self
            .db
            .reserve_credential_operation(
                binding_id,
                expected_version,
                CredentialMutationKind::Delete,
                None,
            )
            .map_err(normalize_db_error)?;
        if let Err(error) = self.db.publish_credential_operation(&reservation, None) {
            self.cleanup_staging_after_failure(&reservation).await;
            return Err(normalize_db_error(error));
        }
        self.finish_published_operation(&reservation)
            .await
            .map_err(|_| public_error("credential_unavailable"))
    }

    async fn credential_status(
        &self,
        snapshot: &CredentialBindingSnapshot,
    ) -> BindingCredentialStatus {
        match snapshot.auth_mode {
            BindingAuthMode::SessionOnly | BindingAuthMode::ManagedAuth => {
                if snapshot.fingerprint.is_none() && snapshot.credential_slot.is_none() {
                    BindingCredentialStatus::NotRequired
                } else {
                    // A provider may change auth modes after a direct key was
                    // configured. Keep the leftover visible as unavailable so
                    // callers can explicitly clear it.
                    BindingCredentialStatus::Unavailable
                }
            }
            BindingAuthMode::Unsupported => BindingCredentialStatus::Unavailable,
            BindingAuthMode::DirectApiKey => {
                let (Some(fingerprint), Some(slot)) = (
                    snapshot.fingerprint.as_deref(),
                    snapshot.credential_slot.as_deref(),
                ) else {
                    return if snapshot.fingerprint.is_none() && snapshot.credential_slot.is_none() {
                        BindingCredentialStatus::Missing
                    } else {
                        BindingCredentialStatus::Unavailable
                    };
                };
                if fingerprint.len() != 32 {
                    return BindingCredentialStatus::Unavailable;
                }
                let Ok(Some(secret)) = self.store_get(slot.to_string()).await else {
                    return BindingCredentialStatus::Unavailable;
                };
                let actual = credential_fingerprint(secret.as_slice());
                if bool::from(actual.as_slice().ct_eq(fingerprint)) {
                    BindingCredentialStatus::Configured
                } else {
                    BindingCredentialStatus::Unavailable
                }
            }
        }
    }

    async fn binding_provider_credential_status(
        &self,
        snapshot: &CredentialBindingSnapshot,
    ) -> BindingCredentialStatus {
        if !snapshot.is_fixed_system_api() {
            return BindingCredentialStatus::NotRequired;
        }
        let (Some(fingerprint), Some(slot)) = (
            snapshot.provider_fingerprint.as_deref(),
            snapshot.provider_credential_slot.as_deref(),
        ) else {
            return if snapshot.provider_fingerprint.is_none()
                && snapshot.provider_credential_slot.is_none()
            {
                BindingCredentialStatus::Missing
            } else {
                BindingCredentialStatus::Unavailable
            };
        };
        if fingerprint.len() != 32 || snapshot.provider_credential_version == 0 {
            return BindingCredentialStatus::Unavailable;
        }
        let Ok(Some(secret)) = self.store_get(slot.to_string()).await else {
            return BindingCredentialStatus::Unavailable;
        };
        let actual = provider_credential_fingerprint(secret.as_slice());
        if bool::from(actual.as_slice().ct_eq(fingerprint)) {
            BindingCredentialStatus::Configured
        } else {
            BindingCredentialStatus::Unavailable
        }
    }

    async fn binding_view(&self, binding_id: &str) -> Result<AgentProviderBindingView, AppError> {
        let snapshot = self
            .db
            .credential_binding_snapshot(binding_id)
            .map_err(normalize_db_error)?
            .ok_or_else(|| public_error("binding_not_found"))?;
        let status = self.credential_status(&snapshot).await;
        let provider_status = self.binding_provider_credential_status(&snapshot).await;
        Ok(snapshot.into_view(status, provider_status))
    }

    pub async fn list_agent_provider_bindings(
        &self,
        agent_module_id: Option<&str>,
    ) -> Result<Vec<AgentProviderBindingView>, AppError> {
        let snapshots = self
            .db
            .credential_binding_snapshots(agent_module_id)
            .map_err(normalize_db_error)?;
        let mut views = Vec::with_capacity(snapshots.len());
        for snapshot in snapshots {
            let status = self.credential_status(&snapshot).await;
            let provider_status = self.binding_provider_credential_status(&snapshot).await;
            views.push(snapshot.into_view(status, provider_status));
        }
        Ok(views)
    }

    pub async fn create_system_api_binding(
        &self,
        input: AgentProviderBindingInput,
    ) -> Result<AgentProviderBindingView, AppError> {
        if input.id.is_some() {
            return Err(public_error("invalid_binding"));
        }
        let provider = self
            .db
            .get_usage_provider(&input.provider_id)
            .map_err(normalize_db_error)?
            .ok_or_else(|| public_error("invalid_binding"))?;
        let preset_key = provider.system_preset_key.as_deref();
        if !is_fixed_api_preset(preset_key)
            || preset_key
                .and_then(|preset| system_binding_route_protocol(preset, &input.agent_module_id))
                .is_none()
        {
            return Err(public_error("invalid_binding"));
        }

        let requested_enabled = input.enabled;
        let reserved = self
            .db
            .save_agent_provider_binding(&AgentProviderBindingInput {
                id: None,
                agent_module_id: input.agent_module_id.clone(),
                provider_id: input.provider_id.clone(),
                enabled: false,
            })
            .map_err(normalize_db_error)?;
        if let Err(error) = self
            .set_binding_api_key(
                &reserved.id,
                reserved.credential_version,
                generate_local_binding_key(),
            )
            .await
        {
            let _ = self
                .db
                .delete_agent_provider_binding_metadata(&reserved.id, reserved.credential_version);
            return Err(error);
        }
        if requested_enabled {
            if let Err(error) = self
                .db
                .save_agent_provider_binding(&AgentProviderBindingInput {
                    id: Some(reserved.id.clone()),
                    agent_module_id: input.agent_module_id,
                    provider_id: input.provider_id,
                    enabled: true,
                })
            {
                let _ = self.delete_binding(&reserved.id, 1).await;
                return Err(normalize_db_error(error));
            }
        }
        self.binding_view(&reserved.id).await
    }

    pub async fn reveal_local_binding_key(
        &self,
        binding_id: &str,
        expected_version: u64,
    ) -> Result<LocalBindingKeyReveal, AppError> {
        let initial = self
            .db
            .credential_binding_snapshot(binding_id)
            .map_err(normalize_db_error)?
            .ok_or_else(|| public_error("binding_not_found"))?;
        if !initial.is_fixed_system_api() {
            return Err(public_error("unsupported_auth"));
        }
        if initial.credential_version != expected_version {
            return Err(public_error("credential_conflict"));
        }
        let fingerprint = initial
            .fingerprint
            .as_deref()
            .filter(|fingerprint| fingerprint.len() == 32)
            .ok_or_else(|| public_error("credential_required"))?;
        let slot = initial
            .credential_slot
            .as_deref()
            .ok_or_else(|| public_error("credential_required"))?
            .to_string();
        let secret = self
            .store_get(slot.clone())
            .await
            .map_err(|_| public_error("credential_unavailable"))?
            .ok_or_else(|| public_error("credential_unavailable"))?;
        if !binding_credential_is_acceptable(secret.as_slice())
            || !bool::from(
                credential_fingerprint(secret.as_slice())
                    .as_slice()
                    .ct_eq(fingerprint),
            )
        {
            return Err(public_error("credential_unavailable"));
        }
        let authoritative = self
            .db
            .credential_binding_snapshot(binding_id)
            .map_err(normalize_db_error)?
            .ok_or_else(|| public_error("binding_not_found"))?;
        if !authoritative.is_fixed_system_api()
            || authoritative.credential_version != expected_version
            || authoritative.credential_slot.as_deref() != Some(slot.as_str())
            || authoritative.fingerprint.as_deref() != Some(fingerprint)
        {
            return Err(public_error("credential_conflict"));
        }
        let local_key = String::from_utf8(secret.to_vec())
            .map_err(|_| public_error("credential_unavailable"))?;
        Ok(LocalBindingKeyReveal {
            binding_id: binding_id.to_string(),
            credential_version: expected_version,
            local_key,
        })
    }

    pub async fn rotate_local_binding_key(
        &self,
        binding_id: &str,
        expected_version: u64,
    ) -> Result<LocalBindingKeyReveal, AppError> {
        let snapshot = self
            .db
            .credential_binding_snapshot(binding_id)
            .map_err(normalize_db_error)?
            .ok_or_else(|| public_error("binding_not_found"))?;
        if !snapshot.is_fixed_system_api() {
            return Err(public_error("unsupported_auth"));
        }
        if snapshot.credential_version != expected_version {
            return Err(public_error("credential_conflict"));
        }
        let rotated = self
            .replace_binding_api_key(binding_id, expected_version, generate_local_binding_key())
            .await?;
        self.reveal_local_binding_key(binding_id, rotated.credential_version)
            .await
    }

    async fn initialize_missing_fixed_api_binding_local_keys(
        &self,
    ) -> Result<HashSet<String>, AppError> {
        let snapshots = self
            .db
            .credential_binding_snapshots(None)
            .map_err(normalize_db_error)?;
        let mut unavailable_bindings = HashSet::new();
        for snapshot in snapshots {
            if !snapshot.is_fixed_system_api()
                || snapshot.agent_archived_at.is_some()
                || snapshot.fingerprint.is_some()
                || snapshot.credential_slot.is_some()
            {
                continue;
            }
            if let Err(error) = self
                .publish_api_key_mutation(
                    &snapshot.id,
                    snapshot.credential_version,
                    generate_local_binding_key(),
                    CredentialMutationKind::Set,
                )
                .await
            {
                unavailable_bindings.insert(snapshot.id.clone());
                log::error!(
                    "fixed API binding local credential generation failed: {}",
                    error
                );
            }
        }
        Ok(unavailable_bindings)
    }

    /// Initialize only missing local keys without reading any existing
    /// protected item. Application startup uses this path so an ad-hoc or newly
    /// updated build does not fan out macOS Keychain authorization dialogs.
    pub(crate) async fn initialize_startup_binding_keys(&self) -> Result<(), AppError> {
        self.initialize_missing_fixed_api_binding_local_keys()
            .await
            .map(|_| ())
    }

    pub async fn ensure_fixed_api_binding_local_keys(
        &self,
    ) -> Result<Vec<AgentProviderBindingView>, AppError> {
        let unavailable_bindings = self
            .initialize_missing_fixed_api_binding_local_keys()
            .await?;
        let mut views = self.list_agent_provider_bindings(None).await?;
        for view in &mut views {
            if unavailable_bindings.contains(&view.id) {
                view.local_credential_status = BindingCredentialStatus::Unavailable;
                view.credential_status = BindingCredentialStatus::Unavailable;
                view.effective_enabled = false;
            }
        }
        Ok(views)
    }

    async fn reconcile_entry(&self, mut entry: CredentialJournalEntry) -> Result<(), ()> {
        if entry.status == "pending" {
            let snapshot = self
                .db
                .credential_reconcile_state(&entry.binding_id)
                .map_err(|_| ())?
                .ok_or(())?;
            let published = match entry.kind {
                CredentialMutationKind::Set | CredentialMutationKind::Replace => {
                    snapshot.credential_version == entry.generation
                        && snapshot.credential_slot.as_deref() == entry.staging_slot.as_deref()
                        && snapshot.has_fingerprint
                }
                CredentialMutationKind::Clear | CredentialMutationKind::Delete => {
                    snapshot.credential_version == entry.generation
                        && snapshot.credential_slot.is_none()
                        && !snapshot.has_fingerprint
                        && !snapshot.enabled
                }
            };
            if published {
                let status = if entry.previous_slot.is_some() {
                    "cleanup"
                } else {
                    "committed"
                };
                self.db
                    .promote_pending_credential_operation(&entry.operation_id, status)
                    .map_err(|_| ())?;
                entry.status = status.to_string();
            } else {
                if let Some(staging_slot) = entry.staging_slot.as_deref() {
                    if self
                        .db
                        .credential_slot_is_active(staging_slot)
                        .map_err(|_| ())?
                    {
                        return Err(());
                    }
                    let reservation = CredentialOperationReservation {
                        operation_id: entry.operation_id.clone(),
                        binding_id: entry.binding_id.clone(),
                        kind: entry.kind,
                        expected_version: entry.generation.saturating_sub(1),
                        generation: entry.generation,
                        staging_slot: entry.staging_slot.clone(),
                        previous_slot: entry.previous_slot.clone(),
                        previous_fingerprint: None,
                    };
                    if !self
                        .db
                        .claim_pending_staging_cleanup(&reservation)
                        .map_err(|_| ())?
                    {
                        return Err(());
                    }
                    if self
                        .db
                        .credential_slot_is_active(staging_slot)
                        .map_err(|_| ())?
                    {
                        return Err(());
                    }
                    self.store_delete(staging_slot.to_string()).await?;
                    self.db
                        .finish_claimed_staging_cleanup(&reservation)
                        .map_err(|_| ())?;
                } else {
                    self.db
                        .delete_pending_journal_entry(
                            &entry.operation_id,
                            &entry.binding_id,
                            entry.generation,
                        )
                        .map_err(|_| ())?;
                }
                return Ok(());
            }
        }
        if entry.status == "cleanup" {
            if let Some(previous_slot) = entry.previous_slot.as_deref() {
                if self
                    .db
                    .credential_slot_is_active(previous_slot)
                    .map_err(|_| ())?
                {
                    return Err(());
                }
                self.store_delete(previous_slot.to_string()).await?;
            }
        }
        self.db.finish_journal_entry(&entry).map_err(|_| ())
    }

    async fn reconcile_provider_entry(
        &self,
        mut entry: ProviderCredentialJournalEntry,
    ) -> Result<(), ()> {
        if entry.status == "pending" {
            let snapshot = self
                .db
                .provider_credential_reconcile_state(&entry.key_id)
                .map_err(|_| ())?
                .ok_or(())?;
            let published = match entry.kind {
                CredentialMutationKind::Set | CredentialMutationKind::Replace => {
                    snapshot.credential_version == entry.generation
                        && snapshot.credential_slot.as_deref() == entry.staging_slot.as_deref()
                        && snapshot.has_fingerprint
                }
                CredentialMutationKind::Clear => {
                    snapshot.credential_version == entry.generation
                        && snapshot.credential_slot.is_none()
                        && !snapshot.has_fingerprint
                }
                CredentialMutationKind::Delete => false,
            };
            if published {
                let status = if entry.previous_slot.is_some() {
                    "cleanup"
                } else {
                    "committed"
                };
                self.db
                    .promote_pending_provider_credential_operation(&entry.operation_id, status)
                    .map_err(|_| ())?;
                entry.status = status.to_string();
            } else {
                if let Some(staging_slot) = entry.staging_slot.as_deref() {
                    if self
                        .db
                        .provider_credential_slot_is_active(staging_slot)
                        .map_err(|_| ())?
                    {
                        return Err(());
                    }
                    let reservation = ProviderCredentialOperationReservation {
                        operation_id: entry.operation_id.clone(),
                        key_id: entry.key_id.clone(),
                        kind: entry.kind,
                        expected_version: entry.generation.saturating_sub(1),
                        generation: entry.generation,
                        staging_slot: entry.staging_slot.clone(),
                        previous_slot: entry.previous_slot.clone(),
                        previous_fingerprint: None,
                    };
                    if !self
                        .db
                        .claim_pending_provider_staging_cleanup(&reservation)
                        .map_err(|_| ())?
                    {
                        return Err(());
                    }
                    if self
                        .db
                        .provider_credential_slot_is_active(staging_slot)
                        .map_err(|_| ())?
                    {
                        return Err(());
                    }
                    self.store_delete(staging_slot.to_string()).await?;
                    self.db
                        .finish_claimed_provider_staging_cleanup(&reservation)
                        .map_err(|_| ())?;
                } else {
                    self.db
                        .delete_pending_provider_journal_entry(
                            &entry.operation_id,
                            &entry.key_id,
                            entry.generation,
                        )
                        .map_err(|_| ())?;
                }
                return Ok(());
            }
        }
        if entry.status == "cleanup" {
            if let Some(previous_slot) = entry.previous_slot.as_deref() {
                if self
                    .db
                    .provider_credential_slot_is_active(previous_slot)
                    .map_err(|_| ())?
                {
                    return Err(());
                }
                self.store_delete(previous_slot.to_string()).await?;
            }
        }
        self.db
            .finish_provider_journal_entry(&entry)
            .map_err(|_| ())
    }

    async fn reconcile_provider_entries_locked(&self) -> Result<(), AppError> {
        let initial_entries = self
            .db
            .provider_credential_journal_entries()
            .map_err(normalize_db_error)?;
        let mut previous_count = initial_entries.len();
        let max_passes = previous_count.saturating_add(1).max(1);
        let mut next_entries = Some(initial_entries);
        for _ in 0..max_passes {
            let entries = match next_entries.take() {
                Some(entries) => entries,
                None => self
                    .db
                    .provider_credential_journal_entries()
                    .map_err(normalize_db_error)?,
            };
            if entries.is_empty() {
                break;
            }
            for entry in entries {
                if self.reconcile_provider_entry(entry).await.is_err() {
                    log::error!("provider credential startup reconciliation step failed");
                }
            }
            let remaining = self
                .db
                .provider_credential_journal_entries()
                .map_err(normalize_db_error)?
                .len();
            if remaining == 0 {
                break;
            }
            if remaining >= previous_count {
                break;
            }
            previous_count = remaining;
        }
        if self
            .db
            .provider_credential_journal_entries()
            .map_err(normalize_db_error)?
            .is_empty()
        {
            Ok(())
        } else {
            Err(public_error("credential_unavailable"))
        }
    }

    async fn reconcile_locked(&self, audit_active_pointers: bool) -> Result<(), AppError> {
        self.reconcile_provider_entries_locked().await?;
        let initial_entries = self
            .db
            .credential_journal_entries()
            .map_err(normalize_db_error)?;
        let mut previous_count = initial_entries.len();
        let max_passes = previous_count.saturating_add(1).max(1);
        let mut next_entries = Some(initial_entries);
        for _ in 0..max_passes {
            let entries = match next_entries.take() {
                Some(entries) => entries,
                None => self
                    .db
                    .credential_journal_entries()
                    .map_err(normalize_db_error)?,
            };
            if entries.is_empty() {
                break;
            }
            for entry in entries {
                if self.reconcile_entry(entry).await.is_err() {
                    log::error!("credential startup reconciliation step failed");
                }
            }
            let remaining = self
                .db
                .credential_journal_entries()
                .map_err(normalize_db_error)?
                .len();
            if remaining == 0 {
                break;
            }
            if remaining >= previous_count {
                break;
            }
            previous_count = remaining;
        }
        let failed = !self
            .db
            .credential_journal_entries()
            .map_err(normalize_db_error)?
            .is_empty();
        if audit_active_pointers {
            // Explicit maintenance operations audit every active pointer after
            // journal cleanup. Normal application startup deliberately skips
            // these reads: macOS may require one authorization dialog per
            // Keychain item, and status is verified lazily before publication
            // or credential use anyway.
            if self.list_usage_providers().await.is_err() {
                log::error!("protected credential active-pointer audit failed");
            }
        }
        if failed {
            Err(public_error("credential_unavailable"))
        } else {
            Ok(())
        }
    }

    pub async fn reconcile_startup(&self) -> Result<(), AppError> {
        let _lifecycle_guard = self.lifecycle_lock.exclusive().await.map_err(|_| {
            log::error!("credential lifecycle lock failed");
            public_error("credential_unavailable")
        })?;
        self.reconcile_locked(true).await
    }

    /// Recover interrupted credential mutations without opening any active
    /// Keychain item. This is the startup path; active values remain fail-closed
    /// and are verified lazily when a UI view or proxy route actually needs
    /// them.
    pub(crate) async fn reconcile_startup_journals(&self) -> Result<(), AppError> {
        let _lifecycle_guard = self.lifecycle_lock.exclusive().await.map_err(|_| {
            log::error!("credential lifecycle lock failed");
            public_error("credential_unavailable")
        })?;
        self.reconcile_locked(false).await
    }

    pub(crate) async fn run_exclusive_database_change<T, Fut>(
        &self,
        operation: Fut,
    ) -> Result<T, AppError>
    where
        Fut: std::future::Future<Output = Result<T, AppError>>,
    {
        let _lifecycle_guard = self.lifecycle_lock.exclusive().await.map_err(|_| {
            log::error!("credential lifecycle lock failed");
            public_error("credential_unavailable")
        })?;
        self.reconcile_locked(true).await?;
        let result = operation.await;
        self.reconcile_locked(true).await?;
        result
    }

    pub(crate) async fn run_exclusive_blocking_database_change<T, Operation>(
        &self,
        operation: Operation,
    ) -> Result<T, AppError>
    where
        T: Send + 'static,
        Operation: FnOnce() -> Result<T, AppError> + Send + 'static,
    {
        let lifecycle_guard = Arc::new(self.lifecycle_lock.exclusive().await.map_err(|_| {
            log::error!("credential lifecycle lock failed");
            public_error("credential_unavailable")
        })?);
        self.reconcile_locked(true).await?;
        let detached_guard = lifecycle_guard.clone();
        let result = match tokio::task::spawn_blocking(move || {
            let _lifecycle_guard = detached_guard;
            operation()
        })
        .await
        {
            Ok(result) => result,
            Err(_) => {
                log::error!("protected database change task failed");
                Err(public_error("credential_unavailable"))
            }
        };
        self.reconcile_locked(true).await?;
        result
    }
}
