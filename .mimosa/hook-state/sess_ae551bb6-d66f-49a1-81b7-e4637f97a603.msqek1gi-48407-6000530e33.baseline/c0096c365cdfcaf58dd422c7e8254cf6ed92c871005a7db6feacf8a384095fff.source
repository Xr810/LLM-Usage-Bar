use super::{CredentialStore, CredentialStoreError};

#[derive(Debug, Default)]
pub(super) struct UnavailableCredentialStore;

impl CredentialStore for UnavailableCredentialStore {
    fn put(&self, _slot: &str, _secret: &[u8]) -> Result<(), CredentialStoreError> {
        Err(CredentialStoreError::Unavailable)
    }

    fn get(&self, _slot: &str) -> Result<Option<Vec<u8>>, CredentialStoreError> {
        Err(CredentialStoreError::Unavailable)
    }

    fn delete(&self, _slot: &str) -> Result<(), CredentialStoreError> {
        Err(CredentialStoreError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_operation_fails_closed_as_unavailable() {
        let store = UnavailableCredentialStore;

        assert_eq!(
            store.put("slot", b"secret"),
            Err(CredentialStoreError::Unavailable)
        );
        assert_eq!(store.get("slot"), Err(CredentialStoreError::Unavailable));
        assert_eq!(store.delete("slot"), Err(CredentialStoreError::Unavailable));
    }
}
