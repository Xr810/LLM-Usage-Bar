use super::{CredentialStore, CredentialStoreError, KEYCHAIN_SERVICE};
use security_framework::base::Error as SecurityFrameworkError;
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

#[derive(Debug, Default)]
#[cfg_attr(debug_assertions, allow(dead_code))]
pub(super) struct MacOsCredentialStore;

fn is_item_not_found(error: SecurityFrameworkError) -> bool {
    error.code() == security_framework_sys::base::errSecItemNotFound
}

impl CredentialStore for MacOsCredentialStore {
    fn put(&self, slot: &str, secret: &[u8]) -> Result<(), CredentialStoreError> {
        set_generic_password(KEYCHAIN_SERVICE, slot, secret)
            .map_err(|_| CredentialStoreError::OperationFailed)
    }

    fn get(&self, slot: &str) -> Result<Option<Vec<u8>>, CredentialStoreError> {
        match get_generic_password(KEYCHAIN_SERVICE, slot) {
            Ok(secret) => Ok(Some(secret)),
            Err(error) if is_item_not_found(error) => Ok(None),
            Err(_) => Err(CredentialStoreError::OperationFailed),
        }
    }

    fn delete(&self, slot: &str) -> Result<(), CredentialStoreError> {
        match delete_generic_password(KEYCHAIN_SERVICE, slot) {
            Ok(()) => Ok(()),
            Err(error) if is_item_not_found(error) => Ok(()),
            Err(_) => Err(CredentialStoreError::OperationFailed),
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
