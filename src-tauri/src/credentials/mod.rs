pub mod codex_oauth_auth;
mod lifecycle_lock;
mod service;

#[cfg(target_os = "macos")]
mod macos;
mod unavailable;

use serde::{Deserialize, Deserializer};
use std::fmt;
use std::sync::Arc;
use zeroize::Zeroizing;

#[cfg(test)]
pub(crate) use service::CredentialExposureGuard;
pub(crate) use service::ResolvedProviderCredential;
pub use service::{BindingCredentialService, ResolvedBindingCredential};
pub(crate) use service::{CredentialExposureGuardSet, CredentialSemanticStreamScannerSet};

pub(super) const KEYCHAIN_SERVICE: &str = "com.xr810.llm-usage-bar.agent-provider-binding.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CredentialStoreError {
    #[error("credential store unavailable")]
    Unavailable,
    #[error("credential store operation failed")]
    OperationFailed,
}

pub trait CredentialStore: Send + Sync {
    fn put(&self, slot: &str, secret: &[u8]) -> Result<(), CredentialStoreError>;
    fn get(&self, slot: &str) -> Result<Option<Vec<u8>>, CredentialStoreError>;
    fn delete(&self, slot: &str) -> Result<(), CredentialStoreError>;
}

/// A non-serializable credential input. Its allocation is zeroized on drop.
pub struct SecretString(Zeroizing<String>);

impl SecretString {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    pub(crate) fn expose_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).map(Self::new)
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretString([REDACTED])")
    }
}

pub(crate) fn unavailable_credential_store() -> Arc<dyn CredentialStore> {
    Arc::new(unavailable::UnavailableCredentialStore)
}

pub(crate) fn production_credential_store() -> Arc<dyn CredentialStore> {
    #[cfg(all(target_os = "macos", not(debug_assertions)))]
    {
        Arc::new(macos::MacOsCredentialStore)
    }
    #[cfg(not(all(target_os = "macos", not(debug_assertions))))]
    {
        unavailable_credential_store()
    }
}

#[cfg(test)]
mod tests;
