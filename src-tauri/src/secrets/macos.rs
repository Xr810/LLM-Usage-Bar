use super::{CredentialStore, CredentialStoreError, KEYCHAIN_SERVICE};
use security_framework::base::Error as SecurityFrameworkError;
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};
use std::time::Instant;

#[cfg(test)]
mod probe;

#[derive(Default)]
#[cfg_attr(debug_assertions, allow(dead_code))]
pub(super) struct MacOsCredentialStore {
    // Only native tests can select a file keychain. Release keeps its original
    // implicit platform target, with no environment-variable override.
    #[cfg(test)]
    target: Option<probe::Target>,
}

fn is_item_not_found(error: SecurityFrameworkError) -> bool {
    error.code() == security_framework_sys::base::errSecItemNotFound
}

impl CredentialStore for MacOsCredentialStore {
    fn put(&self, slot: &str, secret: &[u8]) -> Result<(), CredentialStoreError> {
        let started = Instant::now();
        #[cfg(test)]
        let result = match &self.target {
            Some(target) => target.put(slot, secret),
            None => set_generic_password(KEYCHAIN_SERVICE, slot, secret),
        };
        #[cfg(not(test))]
        let result = set_generic_password(KEYCHAIN_SERVICE, slot, secret);
        result.map_err(|error| {
            log::error!(
                "macOS credential store put failed: os_status={}, elapsed_ms={}",
                error.code(),
                started.elapsed().as_millis()
            );
            CredentialStoreError::OperationFailed
        })
    }

    fn get(&self, slot: &str) -> Result<Option<Vec<u8>>, CredentialStoreError> {
        let started = Instant::now();
        #[cfg(test)]
        let result = match &self.target {
            Some(target) => target.get(slot),
            None => get_generic_password(KEYCHAIN_SERVICE, slot),
        };
        #[cfg(not(test))]
        let result = get_generic_password(KEYCHAIN_SERVICE, slot);
        match result {
            Ok(secret) => Ok(Some(secret)),
            Err(error) if is_item_not_found(error) => Ok(None),
            Err(error) => {
                log::error!(
                    "macOS credential store get failed: os_status={}, elapsed_ms={}",
                    error.code(),
                    started.elapsed().as_millis()
                );
                Err(CredentialStoreError::OperationFailed)
            }
        }
    }

    fn delete(&self, slot: &str) -> Result<(), CredentialStoreError> {
        let started = Instant::now();
        #[cfg(test)]
        let result = match &self.target {
            Some(target) => target.delete(slot),
            None => delete_generic_password(KEYCHAIN_SERVICE, slot),
        };
        #[cfg(not(test))]
        let result = delete_generic_password(KEYCHAIN_SERVICE, slot);
        match result {
            Ok(()) => Ok(()),
            Err(error) if is_item_not_found(error) => Ok(()),
            Err(error) => {
                log::error!(
                    "macOS credential store delete failed: os_status={}, elapsed_ms={}",
                    error.code(),
                    started.elapsed().as_millis()
                );
                Err(CredentialStoreError::OperationFailed)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_not_found_is_the_only_missing_item_status() {
        let missing = security_framework::base::Error::from_code(
            security_framework_sys::base::errSecItemNotFound,
        );
        let denied = security_framework::base::Error::from_code(
            security_framework_sys::base::errSecAuthFailed,
        );

        assert!(is_item_not_found(missing));
        assert!(!is_item_not_found(denied));
    }

    #[test]
    fn keychain_service_is_domain_scoped_and_versioned() {
        assert_eq!(
            KEYCHAIN_SERVICE,
            "com.xr810.llm-usage-bar.agent-provider-binding.v1"
        );
    }
}
