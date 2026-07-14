use super::lifecycle_lock::{CredentialLifecycleLock, SharedLifecycleGuard};
use super::{CredentialStore, SecretString, KEYCHAIN_SERVICE};
use crate::database::{
    BindingAuthMode, CredentialBindingSnapshot, CredentialJournalEntry, CredentialMutationKind,
    CredentialOperationReservation, Database,
};
use crate::error::AppError;
use crate::usage::domain::{AgentProviderBindingView, BindingCredentialStatus};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

const FINGERPRINT_SEPARATOR: &[u8] = b"\0";

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

/// A verified, frozen binding projection and its protected upstream credential.
/// It is intentionally non-Clone and non-serializable.
#[allow(dead_code)] // Its route/key accessors are consumed by Task 4 proxy routing.
pub struct ResolvedBindingCredential {
    binding_id: String,
    agent_module_id: String,
    provider_id: String,
    route_app_type: String,
    route_config: Option<Value>,
    secret: Zeroizing<Vec<u8>>,
}

#[allow(dead_code)] // Its route/key accessors are consumed by Task 4 proxy routing.
impl ResolvedBindingCredential {
    pub fn binding_id(&self) -> &str {
        &self.binding_id
    }

    pub fn agent_module_id(&self) -> &str {
        &self.agent_module_id
    }

    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    pub(crate) fn route_app_type(&self) -> &str {
        &self.route_app_type
    }

    pub(crate) fn route_config(&self) -> Option<&Value> {
        self.route_config.as_ref()
    }

    pub(crate) fn expose_secret(&self) -> &[u8] {
        self.secret.as_slice()
    }
}

impl fmt::Debug for ResolvedBindingCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ResolvedBindingCredential([REDACTED])")
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

    async fn mutate_api_key(
        &self,
        binding_id: &str,
        expected_version: u64,
        api_key: SecretString,
        kind: CredentialMutationKind,
    ) -> Result<AgentProviderBindingView, AppError> {
        if api_key.expose_bytes().is_empty()
            || api_key
                .expose_bytes()
                .iter()
                .all(|byte| byte.is_ascii_whitespace())
        {
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

    async fn binding_view(&self, binding_id: &str) -> Result<AgentProviderBindingView, AppError> {
        let snapshot = self
            .db
            .credential_binding_snapshot(binding_id)
            .map_err(normalize_db_error)?
            .ok_or_else(|| public_error("binding_not_found"))?;
        let status = self.credential_status(&snapshot).await;
        Ok(snapshot.into_view(status))
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
            views.push(snapshot.into_view(status));
        }
        Ok(views)
    }

    pub async fn resolve_binding_api_key(
        &self,
        api_key: SecretString,
    ) -> Result<ResolvedBindingCredential, AppError> {
        if api_key.expose_bytes().is_empty() {
            return Err(public_error("credential_required"));
        }
        let inbound_fingerprint = credential_fingerprint(api_key.expose_bytes());
        let snapshot = self
            .db
            .credential_binding_by_fingerprint(&inbound_fingerprint)
            .map_err(normalize_db_error)?
            .ok_or_else(|| public_error("binding_not_found"))?;
        if !snapshot.enabled
            || !snapshot.provider_enabled
            || snapshot.agent_archived_at.is_some()
            || snapshot.auth_mode != BindingAuthMode::DirectApiKey
        {
            return Err(public_error("invalid_binding"));
        }
        let db_fingerprint = snapshot
            .fingerprint
            .as_deref()
            .filter(|fingerprint| fingerprint.len() == 32)
            .ok_or_else(|| public_error("credential_unavailable"))?;
        let slot = snapshot
            .credential_slot
            .as_deref()
            .ok_or_else(|| public_error("credential_unavailable"))?;
        if !bool::from(inbound_fingerprint.as_slice().ct_eq(db_fingerprint)) {
            return Err(public_error("credential_unavailable"));
        }
        let secret = self
            .store_get(slot.to_string())
            .await
            .map_err(|_| public_error("credential_unavailable"))?
            .ok_or_else(|| public_error("credential_unavailable"))?;
        let stored_fingerprint = credential_fingerprint(secret.as_slice());
        if !bool::from(stored_fingerprint.as_slice().ct_eq(db_fingerprint))
            || !bool::from(stored_fingerprint.ct_eq(&inbound_fingerprint))
        {
            return Err(public_error("credential_unavailable"));
        }
        let route_app_type = snapshot
            .route_app_type
            .ok_or_else(|| public_error("invalid_binding"))?;
        Ok(ResolvedBindingCredential {
            binding_id: snapshot.id,
            agent_module_id: snapshot.agent_module_id,
            provider_id: snapshot.provider_id,
            route_app_type,
            route_config: snapshot.route_config,
            secret,
        })
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

    async fn reconcile_locked(&self) -> Result<(), AppError> {
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
        // Audit every active pointer after journal cleanup. Status remains a
        // derived fail-closed view; reconciliation never guesses or rewrites a
        // missing protected value.
        if self.list_agent_provider_bindings(None).await.is_err() {
            log::error!("credential active-pointer audit failed");
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
        self.reconcile_locked().await
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
        self.reconcile_locked().await?;
        let result = operation.await;
        self.reconcile_locked().await?;
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
        self.reconcile_locked().await?;
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
        self.reconcile_locked().await?;
        result
    }
}
