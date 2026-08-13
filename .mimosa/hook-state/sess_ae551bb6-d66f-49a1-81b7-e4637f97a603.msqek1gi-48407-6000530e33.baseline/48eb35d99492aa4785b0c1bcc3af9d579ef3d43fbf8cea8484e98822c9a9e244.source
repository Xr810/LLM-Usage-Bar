use crate::database::Database;
use fs2::FileExt;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

static LOCAL_GATES: Lazy<Mutex<HashMap<String, Weak<RwLock<()>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn gate_identity(db: &Arc<Database>) -> String {
    db.database_path()
        .map(|path| format!("path:{}", path.to_string_lossy()))
        .unwrap_or_else(|| format!("memory:{:p}", Arc::as_ptr(db)))
}

fn local_gate(db: &Arc<Database>) -> Arc<RwLock<()>> {
    let identity = gate_identity(db);
    let mut gates = LOCAL_GATES
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(gate) = gates.get(&identity).and_then(Weak::upgrade) {
        return gate;
    }
    let gate = Arc::new(RwLock::new(()));
    gates.insert(identity, Arc::downgrade(&gate));
    gate
}

fn lock_path(db: &Database) -> Option<PathBuf> {
    db.database_path()
        .and_then(|path| path.parent())
        .map(|parent| parent.join(".llm-usage-bar-credential-lifecycle.lock"))
}

fn open_and_lock(path: PathBuf, exclusive: bool) -> Result<File, ()> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|_| ())?;
    if exclusive {
        FileExt::lock_exclusive(&file).map_err(|_| ())?;
    } else {
        FileExt::lock_shared(&file).map_err(|_| ())?;
    }
    Ok(file)
}

struct FileLock(Option<File>);

impl Drop for FileLock {
    fn drop(&mut self) {
        if let Some(file) = self.0.as_ref() {
            let _ = FileExt::unlock(file);
        }
    }
}

pub(super) struct SharedLifecycleGuard {
    _local: OwnedRwLockReadGuard<()>,
    _file: FileLock,
}

pub(super) struct ExclusiveLifecycleGuard {
    _local: OwnedRwLockWriteGuard<()>,
    _file: FileLock,
}

pub(super) struct CredentialLifecycleLock {
    local: Arc<RwLock<()>>,
    path: Option<PathBuf>,
}

impl CredentialLifecycleLock {
    pub(super) fn new(db: &Arc<Database>) -> Self {
        Self {
            local: local_gate(db),
            path: lock_path(db),
        }
    }

    pub(super) async fn shared(&self) -> Result<SharedLifecycleGuard, ()> {
        let local = self.local.clone().read_owned().await;
        let file = match self.path.clone() {
            Some(path) => Some(
                tokio::task::spawn_blocking(move || open_and_lock(path, false))
                    .await
                    .map_err(|_| ())??,
            ),
            None => None,
        };
        Ok(SharedLifecycleGuard {
            _local: local,
            _file: FileLock(file),
        })
    }

    pub(super) async fn exclusive(&self) -> Result<ExclusiveLifecycleGuard, ()> {
        let local = self.local.clone().write_owned().await;
        let file = match self.path.clone() {
            Some(path) => Some(
                tokio::task::spawn_blocking(move || open_and_lock(path, true))
                    .await
                    .map_err(|_| ())??,
            ),
            None => None,
        };
        Ok(ExclusiveLifecycleGuard {
            _local: local,
            _file: FileLock(file),
        })
    }
}
