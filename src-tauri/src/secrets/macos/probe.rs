//! Opt-in native probe. This module is compiled only in macOS test binaries.
//! All item queries are singleton-scoped; no error path retries implicitly.
use super::{CredentialStore, CredentialStoreError, MacOsCredentialStore};
use core_foundation::array::CFArray;
use core_foundation::base::TCFType;
use core_foundation::data::CFData;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::{CFString, CFStringRef};
use security_framework::base::{Error, Result};
use security_framework::os::macos::keychain::{CreateOptions, SecKeychain};
use security_framework::passwords::{
    delete_generic_password_options, generic_password, PasswordOptions,
};
use security_framework_sys::base::{errSecDuplicateItem, SecKeychainRef};
use security_framework_sys::item::{
    kSecMatchSearchList, kSecUseAuthenticationUI, kSecUseKeychain, kSecValueData,
};
use security_framework_sys::keychain_item::{SecItemAdd, SecItemUpdate};
use std::sync::Arc;
use std::time::Instant;

#[link(name = "Security", kind = "framework")]
extern "C" {
    static kSecUseAuthenticationUIFail: CFStringRef;
    fn SecKeychainLock(keychain: SecKeychainRef) -> i32;
    fn SecKeychainCopySearchList(list: *mut core_foundation::array::CFArrayRef) -> i32;
}

pub(super) struct Target {
    keychain: SecKeychain,
    service: String,
}

impl Target {
    // PasswordOptions exposes no public keychain setter in the locked 3.7.0
    // crate. Only this test module uses its deprecated query field.
    #[allow(deprecated)]
    fn options(&self, slot: &str, adding: bool) -> PasswordOptions {
        let mut options = PasswordOptions::new_generic_password(&self.service, slot);
        unsafe {
            if adding {
                options.query.push((
                    CFString::wrap_under_get_rule(kSecUseKeychain),
                    self.keychain.clone().into_CFType(),
                ));
            } else {
                options.query.push((
                    CFString::wrap_under_get_rule(kSecMatchSearchList),
                    CFArray::from_CFTypes(std::slice::from_ref(&self.keychain)).into_CFType(),
                ));
            }
            // Per-query rejection, not a process/global interaction-policy change.
            options.query.push((
                CFString::wrap_under_get_rule(kSecUseAuthenticationUI),
                CFString::wrap_under_get_rule(kSecUseAuthenticationUIFail).into_CFType(),
            ));
        }
        options
    }

    fn timed<T>(&self, operation: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let started = Instant::now();
        let result = f();
        eprintln!(
            "disposable_keychain operation={operation} os_status={} elapsed_us={}",
            result.as_ref().err().map_or(0, |error| error.code()),
            started.elapsed().as_micros()
        );
        result
    }

    #[allow(deprecated)]
    pub(super) fn put(&self, slot: &str, secret: &[u8]) -> Result<()> {
        self.timed("put", || {
            let options = self.options(slot, true);
            let value = unsafe {
                (
                    CFString::wrap_under_get_rule(kSecValueData),
                    CFData::from_buffer(secret).into_CFType(),
                )
            };
            let mut add_query = options.query;
            add_query.push(value.clone());
            let add = CFDictionary::from_CFType_pairs(&add_query);
            // Explicit destination for add. A duplicate is updated with a NEW
            // singleton search query, never the implicit production query.
            let mut status = unsafe { SecItemAdd(add.as_concrete_TypeRef(), std::ptr::null_mut()) };
            if status == errSecDuplicateItem {
                let query = self.options(slot, false);
                let search = CFDictionary::from_CFType_pairs(&query.query);
                let value = CFDictionary::from_CFType_pairs(&[value]);
                status = unsafe {
                    SecItemUpdate(search.as_concrete_TypeRef(), value.as_concrete_TypeRef())
                };
            }
            if status == 0 {
                Ok(())
            } else {
                Err(Error::from_code(status))
            }
        })
    }

    pub(super) fn get(&self, slot: &str) -> Result<Vec<u8>> {
        self.timed("get", || generic_password(self.options(slot, false)))
    }

    pub(super) fn delete(&self, slot: &str) -> Result<()> {
        self.timed("delete", || {
            delete_generic_password_options(self.options(slot, false))
        })
    }
}

// Read only references, never item contents or search/default setters.
fn keychain_preferences() -> (Vec<SecKeychain>, std::result::Result<SecKeychain, i32>) {
    let mut list = std::ptr::null();
    let status = unsafe { SecKeychainCopySearchList(&mut list) };
    assert_eq!(status, 0, "cannot snapshot search-list metadata");
    assert!(!list.is_null());
    let list = unsafe { CFArray::<SecKeychain>::wrap_under_create_rule(list) };
    let refs = list
        .iter()
        // Retain the snapshot objects; don't compare released pointer addresses.
        .map(|keychain| unsafe { SecKeychain::wrap_under_get_rule(keychain.as_concrete_TypeRef()) })
        .collect();
    let default = SecKeychain::default().map_err(|error| error.code());
    (refs, default)
}

struct DisposableKeychain {
    // Drop the reference BEFORE deleting the owned directory. Never call
    // SecKeychainDelete: that API can write search-list preferences.
    keychain: Option<SecKeychain>,
    directory: Option<tempfile::TempDir>,
}

impl DisposableKeychain {
    fn create() -> Self {
        let directory = tempfile::Builder::new()
            .prefix("llm-issue66-private-")
            .tempdir()
            .unwrap();
        let path = directory
            .path()
            .canonicalize()
            .unwrap()
            .join("probe.keychain");
        // Apple's shouldAddToSearchList only registers login/System paths.
        // Reject the entire special-name substring, even in a parent directory.
        assert!(!path.to_string_lossy().contains("/login.keychain"));
        assert_ne!(
            path,
            std::path::Path::new("/Library/Keychains/System.keychain")
        );
        assert!(!path.exists());
        let password = uuid::Uuid::new_v4().to_string();
        let started = Instant::now();
        let result = CreateOptions::new()
            .password(&password)
            .prompt_user(false)
            .create(&path);
        eprintln!(
            "disposable_keychain operation=create os_status={} elapsed_us={}",
            result.as_ref().err().map_or(0, |error| error.code()),
            started.elapsed().as_micros()
        );
        Self {
            keychain: Some(result.expect("private keychain creation failed")),
            directory: Some(directory),
        }
    }

    fn store(&self, service: &str) -> MacOsCredentialStore {
        MacOsCredentialStore {
            target: Some(Target {
                keychain: self.keychain.as_ref().unwrap().clone(),
                service: service.to_string(),
            }),
        }
    }

    fn lock(&self) {
        let started = Instant::now();
        let reference = self.keychain.as_ref().unwrap().as_concrete_TypeRef();
        assert!(!reference.is_null());
        // This reference was returned from our own fresh-file creation only.
        let status = unsafe { SecKeychainLock(reference) };
        eprintln!(
            "disposable_keychain operation=lock_owned os_status={status} elapsed_us={}",
            started.elapsed().as_micros()
        );
        assert_eq!(status, 0);
    }
}

impl Drop for DisposableKeychain {
    fn drop(&mut self) {
        drop(self.keychain.take());
        if let Some(directory) = self.directory.take() {
            let path = directory.path().to_path_buf();
            let result = directory.close();
            eprintln!(
                "disposable_keychain cleanup_owned_directory={} file_absent={}",
                result.is_ok(),
                !path.exists()
            );
            if !std::thread::panicking() {
                result.expect("owned keychain directory cleanup failed");
                assert!(!path.exists());
            }
        }
    }
}

#[tokio::test]
#[ignore = "explicit opt-in: real operations on two newly created private Keychain files only"]
async fn disposable_keychain_native_probe() {
    let before = keychain_preferences();
    {
        let first = DisposableKeychain::create();
        let second = DisposableKeychain::create();
        assert!(
            before == keychain_preferences(),
            "creation changed Keychain preferences"
        );
        let service = format!("llm-issue66-probe-{}", uuid::Uuid::new_v4());
        let slot = uuid::Uuid::new_v4().to_string();
        let first_store = first.store(&service);
        let second_store = second.store(&service);
        eprintln!("disposable_keychain phase=healthy_and_cross_file_isolation");
        first_store.put(&slot, b"synthetic-first").unwrap();
        second_store.put(&slot, b"synthetic-second").unwrap();
        first_store.put(&slot, b"synthetic-updated").unwrap();
        assert_eq!(
            first_store.get(&slot).unwrap().unwrap(),
            b"synthetic-updated"
        );
        assert_eq!(
            second_store.get(&slot).unwrap().unwrap(),
            b"synthetic-second"
        );
        first_store.delete(&slot).unwrap();
        assert_eq!(first_store.get(&slot).unwrap(), None);
        first_store.delete(&slot).unwrap();
        assert_eq!(
            second_store.get(&slot).unwrap().unwrap(),
            b"synthetic-second"
        );
        first_store.put(&slot, b"synthetic-before-lock").unwrap();

        eprintln!("disposable_keychain phase=owned_locked_rejection");
        first.lock();
        assert_eq!(
            first_store.get(&slot),
            Err(CredentialStoreError::OperationFailed)
        );
        assert_eq!(
            first_store.put(&slot, b"synthetic-rejected"),
            Err(CredentialStoreError::OperationFailed)
        );
        // Some OS versions permit deleting a locked item without decrypting it.
        // Report that real status rather than inventing a universal permission rule.
        let locked_delete = first_store.delete(&slot);
        assert!(matches!(
            locked_delete,
            Ok(()) | Err(CredentialStoreError::OperationFailed)
        ));

        eprintln!("disposable_keychain phase=real_adapter_startup_degradation");
        let db = Arc::new(crate::store::Database::memory().unwrap());
        let credentials =
            crate::secrets::BindingCredentialService::new(db.clone(), Arc::new(first_store));
        credentials.reconcile_startup_journals().await.unwrap();
        credentials.initialize_startup_binding_keys().await.unwrap();
        for snapshot in db.credential_binding_snapshots(None).unwrap() {
            assert!(snapshot.credential_slot.is_none());
            assert!(snapshot.fingerprint.is_none());
            assert_eq!(snapshot.credential_version, 0);
        }
        let views = credentials
            .list_agent_provider_bindings(None)
            .await
            .unwrap();
        let failed_bindings: Vec<_> = views
            .iter()
            .filter(|view| view.provider_id == "system-openrouter-api")
            .collect();
        assert_eq!(failed_bindings.len(), 3);
        assert!(failed_bindings
            .iter()
            .all(|view| view.enabled && !view.effective_enabled));
        // Cleanup may succeed on a locked file; no published credential is
        // allowed either way. Remaining entries must be unpublished pending work.
        let journal = db.credential_journal_entries().unwrap();
        assert!(journal.iter().all(|entry| entry.status == "pending"));
        eprintln!(
            "disposable_keychain fail_closed_bindings=3 remaining_journals={}",
            journal.len()
        );
        assert_eq!(
            second_store.get(&slot).unwrap().unwrap(),
            b"synthetic-second"
        );
        second_store.delete(&slot).unwrap();
        assert_eq!(second_store.get(&slot).unwrap(), None);
        // Stores and service drop before the owned file references/directories.
    }
    assert!(
        before == keychain_preferences(),
        "probe changed Keychain preferences"
    );
    eprintln!("disposable_keychain default_and_search_list_unchanged=true");
}
