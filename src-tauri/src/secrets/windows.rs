//! Per-user Windows Credential Manager storage; secrets never fall back to disk.
use super::{CredentialStore, CredentialStoreError, KEYCHAIN_SERVICE};
use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND};
use windows_sys::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_MAX_CREDENTIAL_BLOB_SIZE,
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
};

#[derive(Debug, Default)]
pub(super) struct WindowsCredentialStore;

fn target_name(slot: &str) -> Result<Vec<u16>, CredentialStoreError> {
    if slot.is_empty() || slot.contains('\0') {
        return Err(CredentialStoreError::OperationFailed);
    }
    Ok(format!("{KEYCHAIN_SERVICE}/{slot}")
        .encode_utf16()
        .chain(Some(0))
        .collect())
}

struct OwnedCredential(*mut CREDENTIALW);

impl Drop for OwnedCredential {
    fn drop(&mut self) {
        // SAFETY: this pointer was returned by a successful CredReadW call.
        unsafe { CredFree(self.0.cast()) };
    }
}

impl CredentialStore for WindowsCredentialStore {
    fn put(&self, slot: &str, secret: &[u8]) -> Result<(), CredentialStoreError> {
        let mut target = target_name(slot)?;
        if secret.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize {
            return Err(CredentialStoreError::OperationFailed);
        }
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            CredentialBlobSize: secret.len() as u32,
            CredentialBlob: secret.as_ptr() as *mut u8,
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        // SAFETY: all buffers remain valid until this synchronous call returns.
        // CredWriteW copies the blob and does not modify its input buffers.
        if unsafe { CredWriteW(&credential, 0) } == 0 {
            return Err(CredentialStoreError::OperationFailed);
        }
        Ok(())
    }

    fn get(&self, slot: &str) -> Result<Option<Vec<u8>>, CredentialStoreError> {
        let target = target_name(slot)?;
        let mut pointer = std::ptr::null_mut();
        // SAFETY: target is NUL terminated; pointer is a valid out parameter.
        if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut pointer) } == 0 {
            return if unsafe { GetLastError() } == ERROR_NOT_FOUND {
                Ok(None)
            } else {
                Err(CredentialStoreError::OperationFailed)
            };
        }
        if pointer.is_null() {
            return Err(CredentialStoreError::OperationFailed);
        }
        let owned = OwnedCredential(pointer);
        // SAFETY: owned keeps the successful CredReadW allocation alive.
        let credential = unsafe { &*owned.0 };
        if credential.CredentialBlobSize == 0 {
            return Ok(Some(Vec::new()));
        }
        if credential.CredentialBlob.is_null() {
            return Err(CredentialStoreError::OperationFailed);
        }
        // SAFETY: Windows supplies the length of this blob within its allocation.
        let secret = unsafe {
            std::slice::from_raw_parts(
                credential.CredentialBlob,
                credential.CredentialBlobSize as usize,
            )
        };
        Ok(Some(secret.to_vec()))
    }

    fn delete(&self, slot: &str) -> Result<(), CredentialStoreError> {
        let target = target_name(slot)?;
        // SAFETY: target remains a valid NUL-terminated UTF-16 string.
        if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0
            && unsafe { GetLastError() } != ERROR_NOT_FOUND
        {
            return Err(CredentialStoreError::OperationFailed);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_names_are_scoped_and_cannot_be_truncated() {
        assert!(target_name("").is_err());
        assert!(target_name("slot\0other").is_err());
        let target = target_name("测试-slot").unwrap();
        assert_eq!(target.last(), Some(&0));
        assert_eq!(
            String::from_utf16(&target[..target.len() - 1]).unwrap(),
            format!("{KEYCHAIN_SERVICE}/测试-slot")
        );
    }

    #[test]
    fn credential_round_trip_and_missing_delete() {
        let store = WindowsCredentialStore;
        let slot = format!("test-{}", uuid::Uuid::new_v4());
        let result = (|| {
            assert_eq!(store.get(&slot).unwrap(), None);
            store.put(&slot, b"test-secret")?;
            assert_eq!(store.get(&slot).unwrap(), Some(b"test-secret".to_vec()));
            store.put(&slot, b"replacement")?;
            assert_eq!(store.get(&slot).unwrap(), Some(b"replacement".to_vec()));
            store.delete(&slot)?;
            assert_eq!(store.get(&slot).unwrap(), None);
            store.delete(&slot)
        })();
        let _ = store.delete(&slot);
        result.unwrap();
    }
}
