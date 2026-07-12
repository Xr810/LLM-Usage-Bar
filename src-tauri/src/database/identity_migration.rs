//! Safe, one-time migration from the prior current-app database filename.

use crate::database::{Database, SCHEMA_VERSION};
use crate::error::AppError;
use crate::product_identity::{
    DATABASE_FILE, DATABASE_IDENTITY_ARCHIVE_FILE, DATABASE_IDENTITY_LEASE_FILE,
    DATABASE_IDENTITY_SOURCE_SCHEMA_VERSION, LEGACY_DATABASE_FILE, LEGACY_DATA_DIR,
};
use rusqlite::backup::{Backup, StepResult};
use rusqlite::{Connection, OpenFlags};
use std::ffi::{CString, OsString};
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tempfile::NamedTempFile;

const BACKUP_DEADLINE: Duration = Duration::from_secs(5);
const BACKUP_RETRY_PAUSE: Duration = Duration::from_millis(10);
const BACKUP_PAGES_PER_STEP: i32 = 128;
const RETIREMENT_FENCE_REACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);
const RETIREMENT_FENCE_USER_VERSION: i32 = i32::MAX;
const RETIREMENT_FENCE_TRIGGER_PREFIX: &str = "__llm_usage_bar_retirement_fence_";
#[cfg(not(test))]
const LEASE_WAIT_DEADLINE: Duration = Duration::from_secs(12);
#[cfg(test)]
const LEASE_WAIT_DEADLINE: Duration = Duration::from_secs(2);
const LEASE_RETRY_PAUSE: Duration = Duration::from_millis(20);

const REQUIRED_V13_TABLES: &[&str] = &[
    "providers",
    "provider_endpoints",
    "mcp_servers",
    "prompts",
    "skills",
    "skill_repos",
    "settings",
    "proxy_config",
    "provider_health",
    "proxy_request_logs",
    "model_pricing",
    "stream_check_logs",
    "proxy_live_backup",
    "usage_daily_rollups",
    "session_log_sync",
    "profiles",
    "usage_providers",
    "route_bindings",
    "usage_source_bindings",
    "usage_events",
    "usage_event_links",
    "quota_snapshots",
    "quota_fetch_state",
];

const REQUIRED_V13_TRIGGERS: &[&str] = &[
    "usage_events_immutable_update",
    "usage_events_immutable_delete",
    "quota_snapshots_append_only_update",
    "quota_snapshots_append_only_delete",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DatabaseIdentityOutcome {
    pub database_path: PathBuf,
    pub archived_prior_path: Option<PathBuf>,
    pub retained_prior_path: Option<PathBuf>,
    pub migrated: bool,
    pub durability_warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(windows)]
    volume_serial_number: u32,
    #[cfg(windows)]
    file_index: u64,
    #[cfg(all(not(unix), not(windows)))]
    canonical_path: PathBuf,
    #[cfg(all(not(unix), not(windows)))]
    length: u64,
    #[cfg(all(not(unix), not(windows)))]
    created: Option<SystemTime>,
}

impl FileIdentity {
    fn from_path(path: &Path) -> Result<Self, AppError> {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            };

            let file = OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(path)
                .map_err(|error| AppError::io(path, error))?;
            return Self::from_file(path, &file);
        }

        #[cfg(not(windows))]
        let metadata = std::fs::metadata(path).map_err(|error| AppError::io(path, error))?;
        #[cfg(not(windows))]
        return Self::from_metadata(path, &metadata);
    }

    fn from_file(path: &Path, file: &File) -> Result<Self, AppError> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Foundation::HANDLE;
            use windows_sys::Win32::Storage::FileSystem::{
                GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
            };

            let mut information = BY_HANDLE_FILE_INFORMATION::default();
            // SAFETY: `file` owns a live Windows handle for the duration of the
            // call and `information` is a correctly sized writable binding type.
            let succeeded = unsafe {
                GetFileInformationByHandle(
                    file.as_raw_handle() as HANDLE,
                    std::ptr::addr_of_mut!(information),
                )
            };
            if succeeded == 0 {
                return Err(AppError::io(path, std::io::Error::last_os_error()));
            }
            return Ok(Self {
                volume_serial_number: information.dwVolumeSerialNumber,
                file_index: ((information.nFileIndexHigh as u64) << 32)
                    | information.nFileIndexLow as u64,
            });
        }

        #[cfg(not(windows))]
        let metadata = file.metadata().map_err(|error| AppError::io(path, error))?;
        #[cfg(not(windows))]
        return Self::from_metadata(path, &metadata);
    }

    #[cfg(unix)]
    fn from_metadata(_path: &Path, metadata: &std::fs::Metadata) -> Result<Self, AppError> {
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    #[cfg(all(not(unix), not(windows)))]
    fn from_metadata(path: &Path, metadata: &std::fs::Metadata) -> Result<Self, AppError> {
        Ok(Self {
            canonical_path: std::fs::canonicalize(path)
                .map_err(|error| AppError::io(path, error))?,
            length: metadata.len(),
            created: metadata.created().ok(),
        })
    }
}

fn absolute_lexical(path: &Path) -> Result<PathBuf, AppError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| AppError::IoContext {
                context: "resolve current directory for database identity guard".to_string(),
                source: error,
            })?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    Ok(normalized)
}

/// Resolve all existing ancestors, then append missing components lexically.
/// This catches aliases through a symlinked ancestor without creating anything.
fn canonicalize_deepest_existing(path: &Path) -> Result<PathBuf, AppError> {
    let mut cursor = absolute_lexical(path)?;
    let mut missing = Vec::<OsString>::new();
    loop {
        match std::fs::canonicalize(&cursor) {
            Ok(mut resolved) => {
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return absolute_lexical(&resolved);
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let Some(name) = cursor.file_name().map(ToOwned::to_owned) else {
                    return Err(AppError::io(&cursor, error));
                };
                missing.push(name);
                if !cursor.pop() {
                    return Err(AppError::io(path, error));
                }
            }
            Err(error) => return Err(AppError::io(&cursor, error)),
        }
    }
}

fn path_starts_with_platform(path: &Path, base: &Path) -> bool {
    let mut path_components = path.components();
    for base_component in base.components() {
        let Some(path_component) = path_components.next() else {
            return false;
        };
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        let equal = path_component
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&base_component.as_os_str().to_string_lossy());
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let equal = path_component == base_component;
        if !equal {
            return false;
        }
    }
    true
}

fn database_sidecar_path(database: &Path, suffix: &str) -> PathBuf {
    let mut name = database.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    database.with_file_name(name)
}

fn retry_stale_legacy_sidecar_cleanup(
    directory: &PinnedDirectory,
    old_path: &Path,
) -> Result<(), AppError> {
    if path_entry_exists(old_path)? {
        return Ok(());
    }
    for (suffix, purpose) in [("-wal", "retry-legacy-wal"), ("-shm", "retry-legacy-shm")] {
        let sidecar = database_sidecar_path(old_path, suffix);
        if !path_entry_exists(&sidecar)? {
            continue;
        }
        let identity = FileIdentity::from_path(&sidecar)?;
        directory.revalidate(purpose)?;
        if path_entry_exists(old_path)? {
            return Ok(());
        }
        quarantine_remove(directory, &sidecar, &identity, purpose)?;
    }
    Ok(())
}

fn database_identity_candidates(app_dir: &Path) -> Vec<PathBuf> {
    let old = app_dir.join(LEGACY_DATABASE_FILE);
    let new = app_dir.join(DATABASE_FILE);
    let archive = app_dir.join(DATABASE_IDENTITY_ARCHIVE_FILE);
    vec![
        old.clone(),
        database_sidecar_path(&old, "-wal"),
        database_sidecar_path(&old, "-shm"),
        new.clone(),
        database_sidecar_path(&new, "-wal"),
        database_sidecar_path(&new, "-shm"),
        archive.clone(),
        database_sidecar_path(&archive, "-wal"),
        database_sidecar_path(&archive, "-shm"),
        app_dir.join(DATABASE_IDENTITY_LEASE_FILE),
    ]
}

fn path_entry_exists(path: &Path) -> Result<bool, AppError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(AppError::io(path, error)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AtomicMoveOutcome {
    Moved,
    DestinationExists,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AtomicMoveMode {
    Platform,
    #[cfg(test)]
    UnsupportedForTest,
}

fn atomic_move_noreplace(source: &Path, destination: &Path) -> Result<AtomicMoveOutcome, AppError> {
    atomic_move_noreplace_with_mode(source, destination, AtomicMoveMode::Platform)
}

fn atomic_move_noreplace_with_mode(
    source: &Path,
    destination: &Path,
    mode: AtomicMoveMode,
) -> Result<AtomicMoveOutcome, AppError> {
    #[cfg(test)]
    if mode == AtomicMoveMode::UnsupportedForTest {
        return Err(AppError::IoContext {
            context: "atomic no-replace move is unsupported on this platform".to_string(),
            source: std::io::Error::new(ErrorKind::Unsupported, "injected unsupported backend"),
        });
    }
    let _ = mode;

    if source.parent() != destination.parent() {
        return Err(AppError::InvalidInput(format!(
            "atomic no-replace move must stay in one directory: {} -> {}",
            source.display(),
            destination.display()
        )));
    }

    #[cfg(target_os = "linux")]
    let result = {
        use std::os::unix::ffi::OsStrExt;

        const AT_FDCWD: i32 = -100;
        const RENAME_NOREPLACE: u32 = 1;
        unsafe extern "C" {
            fn renameat2(
                olddirfd: i32,
                oldpath: *const std::ffi::c_char,
                newdirfd: i32,
                newpath: *const std::ffi::c_char,
                flags: u32,
            ) -> i32;
        }
        let old = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
            AppError::InvalidInput(format!("source path contains NUL: {}", source.display()))
        })?;
        let new = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
            AppError::InvalidInput(format!(
                "destination path contains NUL: {}",
                destination.display()
            ))
        })?;
        // SAFETY: both C strings remain alive for this call and point to paths
        // in the same pinned directory. RENAME_NOREPLACE forbids clobbering.
        let status = unsafe {
            renameat2(
                AT_FDCWD,
                old.as_ptr(),
                AT_FDCWD,
                new.as_ptr(),
                RENAME_NOREPLACE,
            )
        };
        (status == 0)
            .then_some(())
            .ok_or_else(std::io::Error::last_os_error)
    };

    #[cfg(target_os = "macos")]
    let result = {
        use std::os::unix::ffi::OsStrExt;

        const AT_FDCWD: i32 = -2;
        const RENAME_EXCL: u32 = 0x0000_0004;
        unsafe extern "C" {
            fn renameatx_np(
                fromfd: i32,
                from: *const std::ffi::c_char,
                tofd: i32,
                to: *const std::ffi::c_char,
                flags: u32,
            ) -> i32;
        }
        let old = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
            AppError::InvalidInput(format!("source path contains NUL: {}", source.display()))
        })?;
        let new = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
            AppError::InvalidInput(format!(
                "destination path contains NUL: {}",
                destination.display()
            ))
        })?;
        // SAFETY: both C strings remain valid for the duration of the call;
        // RENAME_EXCL makes the move fail rather than overwrite destination.
        let status =
            unsafe { renameatx_np(AT_FDCWD, old.as_ptr(), AT_FDCWD, new.as_ptr(), RENAME_EXCL) };
        (status == 0)
            .then_some(())
            .ok_or_else(std::io::Error::last_os_error)
    };

    #[cfg(windows)]
    let result = {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::MoveFileExW;

        let old = source
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let new = destination
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        // SAFETY: both UTF-16 buffers are NUL-terminated and live through the
        // call. A zero flag set intentionally omits MOVEFILE_REPLACE_EXISTING.
        let status = unsafe { MoveFileExW(old.as_ptr(), new.as_ptr(), 0) };
        (status != 0)
            .then_some(())
            .ok_or_else(std::io::Error::last_os_error)
    };

    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    let result: std::io::Result<()> = Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "atomic no-replace move has no audited backend for this target",
    ));

    match result {
        Ok(()) => Ok(AtomicMoveOutcome::Moved),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            Ok(AtomicMoveOutcome::DestinationExists)
        }
        Err(error) => Err(AppError::IoContext {
            context: format!(
                "atomic no-replace move {} -> {} failed",
                source.display(),
                destination.display()
            ),
            source: error,
        }),
    }
}

fn legacy_namespace_error(path: &Path) -> AppError {
    AppError::InvalidInput(format!(
        "database identity migration refuses protected legacy namespace {LEGACY_DATA_DIR}: {}",
        path.display()
    ))
}

#[derive(Debug, Clone)]
struct PinnedDirectory {
    canonical_path: PathBuf,
    identity: FileIdentity,
}

impl PinnedDirectory {
    fn revalidate(&self, operation: &str) -> Result<(), AppError> {
        let current = FileIdentity::from_path(&self.canonical_path)?;
        if current != self.identity {
            return Err(AppError::Lock(format!(
                "database directory identity changed before {operation}: {}",
                self.canonical_path.display()
            )));
        }
        Ok(())
    }

    fn join(&self, name: impl AsRef<Path>) -> PathBuf {
        self.canonical_path.join(name)
    }
}

/// Read-only guard. This must run before mkdir, lease creation, SQLite open, or
/// any authoritative-path early return.
fn reject_legacy_namespace_before_any_io(app_dir: &Path) -> Result<(), AppError> {
    let home = dirs::home_dir().ok_or_else(|| {
        AppError::InvalidInput(
            "cannot resolve home directory for protected .cc-switch guard".to_string(),
        )
    })?;
    reject_legacy_namespace_with_protected(app_dir, &home.join(LEGACY_DATA_DIR))
}

fn reject_legacy_namespace_with_protected(
    app_dir: &Path,
    protected: &Path,
) -> Result<(), AppError> {
    let app_lexical = absolute_lexical(app_dir)?;
    let protected_lexical = absolute_lexical(protected)?;
    if path_starts_with_platform(&app_lexical, &protected_lexical) {
        return Err(legacy_namespace_error(app_dir));
    }

    let app_resolved = canonicalize_deepest_existing(app_dir)?;
    let protected_resolved = canonicalize_deepest_existing(protected)?;
    if path_starts_with_platform(&app_resolved, &protected_resolved) {
        return Err(legacy_namespace_error(app_dir));
    }

    let local_candidates = database_identity_candidates(app_dir);
    let protected_candidates = database_identity_candidates(protected);
    for candidate in &local_candidates {
        let lexical = absolute_lexical(candidate)?;
        if path_starts_with_platform(&lexical, &protected_lexical) {
            return Err(legacy_namespace_error(candidate));
        }
        let resolved = canonicalize_deepest_existing(candidate)?;
        if path_starts_with_platform(&resolved, &protected_resolved) {
            return Err(legacy_namespace_error(candidate));
        }
    }

    #[cfg(any(unix, windows))]
    {
        let mut protected_objects = Vec::new();
        if path_entry_exists(protected)? {
            protected_objects.push(FileIdentity::from_path(protected)?);
        }
        for candidate in &protected_candidates {
            if path_entry_exists(candidate)? {
                protected_objects.push(FileIdentity::from_path(candidate)?);
            }
        }
        if path_entry_exists(app_dir)? {
            let identity = FileIdentity::from_path(app_dir)?;
            if protected_objects.contains(&identity) {
                return Err(legacy_namespace_error(app_dir));
            }
        }
        for candidate in &local_candidates {
            if path_entry_exists(candidate)? {
                let identity = FileIdentity::from_path(candidate)?;
                if protected_objects.contains(&identity) {
                    return Err(legacy_namespace_error(candidate));
                }
            }
        }
    }

    Ok(())
}

fn pin_safe_existing_directory(
    app_dir: &Path,
    protected_override: Option<&Path>,
) -> Result<Option<PinnedDirectory>, AppError> {
    match protected_override {
        Some(protected) => reject_legacy_namespace_with_protected(app_dir, protected)?,
        None => reject_legacy_namespace_before_any_io(app_dir)?,
    }
    if !path_entry_exists(app_dir)? {
        return Ok(None);
    }
    let canonical_path =
        std::fs::canonicalize(app_dir).map_err(|error| AppError::io(app_dir, error))?;
    let metadata =
        std::fs::metadata(&canonical_path).map_err(|error| AppError::io(&canonical_path, error))?;
    if !metadata.is_dir() {
        return Err(AppError::InvalidInput(format!(
            "database application path is not a directory: {}",
            app_dir.display()
        )));
    }
    match protected_override {
        Some(protected) => reject_legacy_namespace_with_protected(&canonical_path, protected)?,
        None => reject_legacy_namespace_before_any_io(&canonical_path)?,
    }
    let identity = FileIdentity::from_path(&canonical_path)?;
    Ok(Some(PinnedDirectory {
        canonical_path,
        identity,
    }))
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), AppError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| AppError::io(path, error))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), AppError> {
    Ok(())
}

struct DatabaseMigrationLease {
    directory: PinnedDirectory,
    path: PathBuf,
    identity: FileIdentity,
    nonce: String,
    file: Option<File>,
    armed: bool,
}

/// Owns the just-created lease name until initialization is complete. Cleanup
/// is attempted only after the opened handle yielded a stable object identity;
/// if even handle metadata is unavailable, the entry is deliberately retained
/// fail-closed for operator inspection instead of deleting by path.
struct LeaseCreationGuard {
    directory: PinnedDirectory,
    path: PathBuf,
    file: Option<File>,
    identity: Option<FileIdentity>,
    committed: bool,
}

impl LeaseCreationGuard {
    fn new(directory: PinnedDirectory, path: PathBuf, file: File) -> Self {
        Self {
            directory,
            path,
            file: Some(file),
            identity: None,
            committed: false,
        }
    }

    fn finish(mut self, directory: PinnedDirectory, nonce: String) -> DatabaseMigrationLease {
        let lease = DatabaseMigrationLease {
            directory,
            path: self.path.clone(),
            identity: self
                .identity
                .clone()
                .expect("lease identity is established before finish"),
            nonce,
            file: Some(
                self.file
                    .take()
                    .expect("lease handle is present before finish"),
            ),
            armed: true,
        };
        self.committed = true;
        lease
    }
}

impl Drop for LeaseCreationGuard {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let Some(identity) = self.identity.as_ref() else {
            return;
        };
        drop(self.file.take());
        let _ = quarantine_remove(&self.directory, &self.path, identity, "lease-init-cleanup");
    }
}

impl DatabaseMigrationLease {
    fn acquire(directory: &PinnedDirectory) -> Result<Self, AppError> {
        directory.revalidate("migration lease creation")?;
        let path = directory.join(DATABASE_IDENTITY_LEASE_FILE);
        let deadline = Instant::now() + LEASE_WAIT_DEADLINE;
        loop {
            directory.revalidate("migration lease create_new attempt")?;
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => {
                    let mut creation =
                        LeaseCreationGuard::new(directory.clone(), path.clone(), file);
                    let identity = match FileIdentity::from_file(
                        &path,
                        creation.file.as_ref().expect("creation handle"),
                    ) {
                        Ok(identity) => identity,
                        Err(error) => {
                            return Err(AppError::Lock(format!(
                                "could not identify newly created database migration lease {}; retained fail-closed: {error}",
                                path.display()
                            )));
                        }
                    };
                    creation.identity = Some(identity);
                    let nonce = uuid::Uuid::new_v4().to_string();
                    let created_unix_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis();
                    let owner = format!(
                        "{{\"version\":1,\"pid\":{},\"created_unix_ms\":{},\"state\":\"running\",\"nonce\":\"{}\"}}\n",
                        std::process::id(),
                        created_unix_ms,
                        nonce
                    );
                    if let Err(error) = creation
                        .file
                        .as_mut()
                        .expect("creation handle")
                        .write_all(owner.as_bytes())
                        .and_then(|_| creation.file.as_ref().expect("creation handle").sync_all())
                    {
                        return Err(AppError::io(&path, error));
                    }
                    let lease = creation.finish(directory.clone(), nonce);
                    if let Err(error) = sync_directory(&directory.canonical_path) {
                        drop(lease);
                        return Err(error);
                    }
                    return Ok(lease);
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    if Instant::now() >= deadline {
                        return Err(AppError::Lock(format!(
                            "database migration lease already exists; inspect and remove deliberately if no owner is active: {}",
                            path.display()
                        )));
                    }
                    thread::sleep(LEASE_RETRY_PAUSE);
                }
                Err(error) => return Err(AppError::io(&path, error)),
            }
        }
    }

    fn path_is_owned(&self) -> Result<bool, AppError> {
        let mut file = File::open(&self.path).map_err(|error| AppError::io(&self.path, error))?;
        if FileIdentity::from_file(&self.path, &file)? != self.identity {
            return Ok(false);
        }
        let mut metadata = String::new();
        file.read_to_string(&mut metadata)
            .map_err(|error| AppError::io(&self.path, error))?;
        let nonce_field = format!("\"nonce\":\"{}\"", self.nonce);
        Ok(metadata.contains(&nonce_field))
    }

    fn release_inner(&mut self) -> Result<(), AppError> {
        if !self.path_is_owned()? {
            return Err(AppError::Lock(format!(
                "database migration lease ownership changed; evidence retained at {}",
                self.path.display()
            )));
        }
        drop(self.file.take());
        quarantine_remove(&self.directory, &self.path, &self.identity, "lease-cleanup")
    }

    fn release(mut self) -> Result<(), AppError> {
        let result = self.release_inner();
        self.armed = false;
        result
    }
}

fn quarantine_remove(
    directory: &PinnedDirectory,
    path: &Path,
    expected: &FileIdentity,
    purpose: &str,
) -> Result<(), AppError> {
    match quarantine_remove_with_directory_sync(directory, path, expected, purpose, sync_directory)?
    {
        QuarantineRemoveOutcome::RemovedDurably => Ok(()),
        QuarantineRemoveOutcome::RemovedWithDurabilityWarning(error) => Err(error),
    }
}

enum QuarantineRemoveOutcome {
    RemovedDurably,
    RemovedWithDurabilityWarning(AppError),
}

fn quarantine_remove_with_directory_sync(
    directory: &PinnedDirectory,
    path: &Path,
    expected: &FileIdentity,
    purpose: &str,
    sync_after_unlink: impl FnOnce(&Path) -> Result<(), AppError>,
) -> Result<QuarantineRemoveOutcome, AppError> {
    directory.revalidate(purpose)?;
    let quarantine = directory.join(format!(
        ".llm-usage-bar-quarantine-{}-{}",
        purpose,
        uuid::Uuid::new_v4()
    ));
    match atomic_move_noreplace(path, &quarantine)? {
        AtomicMoveOutcome::Moved => {}
        AtomicMoveOutcome::DestinationExists => {
            return Err(AppError::Lock(format!(
                "unique quarantine unexpectedly exists: {}",
                quarantine.display()
            )));
        }
    }

    let moved_identity = FileIdentity::from_path(&quarantine);
    if !matches!(moved_identity, Ok(ref identity) if identity == expected) {
        let mismatch = match moved_identity {
            Ok(_) => "moved object identity did not match expected object".to_string(),
            Err(error) => format!("could not identify quarantined object: {error}"),
        };
        return match atomic_move_noreplace(&quarantine, path) {
            Ok(AtomicMoveOutcome::Moved) => match sync_directory(&directory.canonical_path) {
                Ok(()) => Err(AppError::Lock(format!(
                    "{mismatch}; replacement was atomically restored to {}",
                    path.display()
                ))),
                Err(sync) => Err(AppError::Lock(format!(
                    "{mismatch}; replacement was restored to {}, but directory sync failed: {sync}",
                    path.display()
                ))),
            },
            Ok(AtomicMoveOutcome::DestinationExists) => Err(AppError::Lock(format!(
                "{mismatch}; restore destination was occupied; evidence retained at {}",
                quarantine.display()
            ))),
            Err(restore) => Err(AppError::Lock(format!(
                "{mismatch}; atomic restore failed ({restore}); evidence retained at {}",
                quarantine.display()
            ))),
        };
    }

    if let Err(remove_error) = std::fs::remove_file(&quarantine) {
        return match atomic_move_noreplace(&quarantine, path) {
            Ok(AtomicMoveOutcome::Moved) => {
                let sync = sync_directory(&directory.canonical_path).err();
                Err(AppError::IoContext {
                    context: format!(
                        "quarantine unlink failed; original object restored to {}{}",
                        path.display(),
                        sync.as_ref()
                            .map(|error| format!(", but directory sync failed: {error}"))
                            .unwrap_or_default()
                    ),
                    source: remove_error,
                })
            }
            Ok(AtomicMoveOutcome::DestinationExists) => Err(AppError::Database(format!(
                "quarantine unlink failed ({remove_error}); restore destination occupied; evidence retained at {}",
                quarantine.display()
            ))),
            Err(restore) => Err(AppError::Database(format!(
                "quarantine unlink failed ({remove_error}); restore failed ({restore}); evidence retained at {}",
                quarantine.display()
            ))),
        };
    }
    Ok(match sync_after_unlink(&directory.canonical_path) {
        Ok(()) => QuarantineRemoveOutcome::RemovedDurably,
        Err(error) => QuarantineRemoveOutcome::RemovedWithDurabilityWarning(error),
    })
}

impl Drop for DatabaseMigrationLease {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Err(error) = self.release_inner() {
            log::warn!(
                "Fallback database migration lease cleanup failed for {}: {error}",
                self.path.display()
            );
        }
        self.armed = false;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SchemaValidationPolicy {
    ExactMigrationSource,
    CurrentAuthoritative,
}

/// Holds SQLite's single-writer reservation while snapshots are published, then
/// persists a trigger fence before briefly releasing and reacquiring the write
/// lock. A writer already waiting in SQLite can therefore acquire the retired
/// source only long enough to fail its DML against the durable fence.
struct SourceWriteBarrier {
    connection: Connection,
    transaction_active: bool,
    retirement_fence: Option<RetirementFence>,
}

#[derive(Clone)]
struct RetirementFence {
    original_user_version: i32,
    trigger_names: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceRetirementStrategy {
    HoldBarrierThroughAtomicRetirement,
    #[cfg(windows)]
    WindowsFailClosed,
}

fn source_retirement_strategy() -> SourceRetirementStrategy {
    #[cfg(windows)]
    {
        SourceRetirementStrategy::WindowsFailClosed
    }
    #[cfg(not(windows))]
    {
        SourceRetirementStrategy::HoldBarrierThroughAtomicRetirement
    }
}

fn require_supported_source_retirement() -> Result<(), AppError> {
    match source_retirement_strategy() {
        SourceRetirementStrategy::HoldBarrierThroughAtomicRetirement => Ok(()),
        #[cfg(windows)]
        SourceRetirementStrategy::WindowsFailClosed => Err(AppError::Database(
            "safe old-database retirement is unavailable on Windows: SQLite's own write-barrier handle denies delete sharing, and releasing it would create a data-loss race"
                .to_string(),
        )),
    }
}

impl SourceWriteBarrier {
    fn acquire(source: &Path) -> Result<Self, AppError> {
        let connection = Connection::open_with_flags(
            source,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|error| {
            database_error(
                &format!("open source write barrier {}", source.display()),
                error,
            )
        })?;
        connection
            .busy_timeout(Duration::ZERO)
            .map_err(|error| database_error("set source write barrier timeout", error))?;
        connection
            .execute_batch("BEGIN IMMEDIATE;")
            .map_err(|error| database_error("acquire source BEGIN IMMEDIATE barrier", error))?;
        Ok(Self {
            connection,
            transaction_active: true,
            retirement_fence: None,
        })
    }

    fn install_and_reacquire_retirement_fence(&mut self) -> Result<(), AppError> {
        if !self.transaction_active || self.retirement_fence.is_some() {
            return Err(AppError::Database(
                "source retirement fence entered from an invalid transaction state".to_string(),
            ));
        }

        let original_user_version = Database::get_user_version(&self.connection)?;
        let tables = source_user_tables(&self.connection)?;
        if tables.is_empty() {
            return Err(AppError::Database(
                "source retirement fence found no user tables".to_string(),
            ));
        }
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let mut trigger_names = Vec::with_capacity(tables.len() * 3);
        let install = (|| -> Result<(), AppError> {
            for (table_index, table) in tables.iter().enumerate() {
                for (operation, suffix) in [
                    ("INSERT", "insert"),
                    ("UPDATE", "update"),
                    ("DELETE", "delete"),
                ] {
                    let trigger_name =
                        format!("{RETIREMENT_FENCE_TRIGGER_PREFIX}{nonce}_{table_index}_{suffix}");
                    trigger_names.push(trigger_name.clone());
                    let sql = format!(
                        "CREATE TRIGGER {} BEFORE {operation} ON {} BEGIN SELECT RAISE(ABORT, 'database retired during product identity migration'); END;",
                        quote_sql_identifier(&trigger_name),
                        quote_sql_identifier(table),
                    );
                    self.connection.execute_batch(&sql).map_err(|error| {
                        database_error("install source retirement trigger", error)
                    })?;
                }
            }
            Database::set_user_version(&self.connection, RETIREMENT_FENCE_USER_VERSION)?;
            self.connection
                .execute_batch("COMMIT;")
                .map_err(|error| database_error("commit source retirement fence", error))?;
            Ok(())
        })();

        if let Err(error) = install {
            let rollback = self.connection.execute_batch("ROLLBACK;").err();
            return match rollback {
                Some(rollback) => {
                    // SQLite may have applied any prefix of the trigger/user
                    // version writes and may still own the transaction. Keep
                    // the complete planned metadata and treat the connection as
                    // active/uncertain so the caller must restore the fence
                    // before it is allowed to roll published snapshots back.
                    self.retirement_fence = Some(RetirementFence {
                        original_user_version,
                        trigger_names,
                    });
                    self.transaction_active = true;
                    Err(AppError::Database(format!(
                        "{error}; additionally, source retirement fence transaction rollback failed: {rollback}"
                    )))
                }
                None => {
                    self.transaction_active = false;
                    Err(error)
                }
            };
        }

        self.transaction_active = false;
        self.retirement_fence = Some(RetirementFence {
            original_user_version,
            trigger_names,
        });
        self.connection
            .busy_timeout(RETIREMENT_FENCE_REACQUIRE_TIMEOUT)
            .map_err(|error| database_error("set retirement-fence reacquire timeout", error))?;
        self.connection
            .execute_batch("BEGIN IMMEDIATE;")
            .map_err(|error| {
                database_error("reacquire source barrier after retirement fence", error)
            })?;
        self.transaction_active = true;
        Ok(())
    }

    fn restore_retirement_fence(&mut self) -> Result<(), AppError> {
        let Some(fence) = self.retirement_fence.clone() else {
            return Ok(());
        };
        if !self.transaction_active {
            self.connection
                .busy_timeout(RETIREMENT_FENCE_REACQUIRE_TIMEOUT)
                .map_err(|error| database_error("set retirement-fence rollback timeout", error))?;
            self.connection
                .execute_batch("BEGIN IMMEDIATE;")
                .map_err(|error| {
                    database_error("acquire source barrier to rollback fence", error)
                })?;
            self.transaction_active = true;
        }

        let restore = (|| -> Result<(), AppError> {
            for trigger_name in &fence.trigger_names {
                self.connection
                    .execute_batch(&format!(
                        "DROP TRIGGER IF EXISTS {};",
                        quote_sql_identifier(trigger_name)
                    ))
                    .map_err(|error| database_error("drop source retirement trigger", error))?;
            }
            Database::set_user_version(&self.connection, fence.original_user_version)?;
            self.connection.execute_batch("COMMIT;").map_err(|error| {
                database_error("commit source retirement fence rollback", error)
            })?;
            Ok(())
        })();
        if let Err(error) = restore {
            let rollback = self.connection.execute_batch("ROLLBACK;").err();
            self.transaction_active = false;
            return match rollback {
                Some(rollback) => Err(AppError::Database(format!(
                    "{error}; additionally, retirement-fence rollback transaction failed: {rollback}"
                ))),
                None => Err(error),
            };
        }

        self.transaction_active = false;
        self.retirement_fence = None;
        Ok(())
    }

    fn disarm_retirement_fence(&mut self) {
        self.retirement_fence = None;
    }
}

impl Drop for SourceWriteBarrier {
    fn drop(&mut self) {
        if self.retirement_fence.is_some() {
            if let Err(error) = self.restore_retirement_fence() {
                log::error!("Failed to restore durable source retirement fence: {error}");
            }
        }
        if self.transaction_active {
            let _ = self.connection.execute_batch("ROLLBACK;");
            self.transaction_active = false;
        }
    }
}

fn quote_sql_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn source_user_tables(connection: &Connection) -> Result<Vec<String>, AppError> {
    let mut statement = connection
        .prepare(
            "SELECT name, COALESCE(sql, '') FROM sqlite_schema
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
             ORDER BY name;",
        )
        .map_err(|error| database_error("prepare retirement-fence table inventory", error))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| database_error("query retirement-fence table inventory", error))?;
    let mut tables = Vec::new();
    for row in rows {
        let (name, sql) =
            row.map_err(|error| database_error("read retirement-fence table inventory", error))?;
        if sql
            .trim_start()
            .to_ascii_uppercase()
            .starts_with("CREATE VIRTUAL TABLE")
        {
            return Err(AppError::Database(format!(
                "cannot install a persistent retirement fence on virtual table {name}"
            )));
        }
        tables.push(name);
    }
    Ok(tables)
}

fn database_error(context: &str, error: impl std::fmt::Display) -> AppError {
    AppError::Database(format!("{context}: {error}"))
}

fn validate_quick_check(conn: &Connection) -> Result<(), AppError> {
    let mut statement = conn
        .prepare("PRAGMA quick_check;")
        .map_err(|error| database_error("prepare PRAGMA quick_check", error))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| database_error("run PRAGMA quick_check", error))?;
    let mut diagnostics = Vec::new();
    for row in rows {
        diagnostics
            .push(row.map_err(|error| database_error("read PRAGMA quick_check result", error))?);
    }
    if diagnostics.as_slice() != ["ok"] {
        return Err(AppError::Database(format!(
            "PRAGMA quick_check failed: {}",
            diagnostics.join("; ")
        )));
    }
    Ok(())
}

fn validate_required_v13_tables(conn: &Connection) -> Result<(), AppError> {
    let mut missing = Vec::new();
    for table in REQUIRED_V13_TABLES {
        if !Database::table_exists(conn, table)? {
            missing.push(*table);
        }
    }
    if !missing.is_empty() {
        return Err(AppError::Database(format!(
            "database is missing required v13 tables: {}",
            missing.join(", ")
        )));
    }
    let mut missing_triggers = Vec::new();
    for trigger in REQUIRED_V13_TRIGGERS {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'trigger' AND name = ?1)",
                [trigger],
                |row| row.get(0),
            )
            .map_err(|error| database_error("query required v13 trigger", error))?;
        if !exists {
            missing_triggers.push(*trigger);
        }
    }
    if !missing_triggers.is_empty() {
        return Err(AppError::Database(format!(
            "database is missing required v13 triggers: {}",
            missing_triggers.join(", ")
        )));
    }
    Ok(())
}

fn validate_database(conn: &Connection, policy: SchemaValidationPolicy) -> Result<(), AppError> {
    validate_quick_check(conn)?;
    validate_required_v13_tables(conn)?;
    let version = Database::get_user_version(conn)?;
    match policy {
        SchemaValidationPolicy::ExactMigrationSource
            if version != DATABASE_IDENTITY_SOURCE_SCHEMA_VERSION =>
        {
            Err(AppError::Database(format!(
                "database filename migration requires schema v{}, found v{version}",
                DATABASE_IDENTITY_SOURCE_SCHEMA_VERSION
            )))
        }
        SchemaValidationPolicy::CurrentAuthoritative
            if !(DATABASE_IDENTITY_SOURCE_SCHEMA_VERSION..=SCHEMA_VERSION).contains(&version) =>
        {
            Err(AppError::Database(format!(
                "authoritative database schema v{version} is outside supported range v{}..=v{}",
                DATABASE_IDENTITY_SOURCE_SCHEMA_VERSION, SCHEMA_VERSION
            )))
        }
        _ => Ok(()),
    }
}

fn open_read_only(path: &Path) -> Result<Connection, AppError> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| database_error(&format!("open {} read-only", path.display()), error))
}

fn run_backup_to_done(source: &Connection, destination: &mut Connection) -> Result<(), AppError> {
    let backup = Backup::new(source, destination)
        .map_err(|error| database_error("initialize SQLite online backup", error))?;
    let deadline = Instant::now() + BACKUP_DEADLINE;
    loop {
        if Instant::now() >= deadline {
            return Err(AppError::Database(format!(
                "SQLite online backup did not reach Done before {:?} deadline",
                BACKUP_DEADLINE
            )));
        }
        let step_result = backup
            .step(BACKUP_PAGES_PER_STEP)
            .map_err(|error| database_error("step SQLite online backup", error))?;
        match step_result {
            StepResult::Done => break,
            StepResult::More if Instant::now() < deadline => thread::yield_now(),
            StepResult::Busy | StepResult::Locked if Instant::now() < deadline => {
                thread::sleep(BACKUP_RETRY_PAUSE);
            }
            StepResult::More | StepResult::Busy | StepResult::Locked => {
                return Err(AppError::Database(format!(
                    "SQLite online backup did not reach Done before {:?} deadline",
                    BACKUP_DEADLINE
                )));
            }
            _ => {
                return Err(AppError::Database(
                    "SQLite online backup returned an unsupported step result".to_string(),
                ));
            }
        }
    }
    Ok(())
}

fn backup_into_existing_destination(source: &Path, destination: &Path) -> Result<(), AppError> {
    let source_conn = open_read_only(source)?;
    let mut destination_conn = Connection::open_with_flags(
        destination,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| {
        database_error(
            &format!("open backup destination {}", destination.display()),
            error,
        )
    })?;
    run_backup_to_done(&source_conn, &mut destination_conn)?;
    validate_database(
        &destination_conn,
        SchemaValidationPolicy::ExactMigrationSource,
    )?;
    drop(destination_conn);
    drop(source_conn);
    File::open(destination)
        .and_then(|file| file.sync_all())
        .map_err(|error| AppError::io(destination, error))?;
    Ok(())
}

fn backup_and_validate(source: &Path, destination: &Path) -> Result<(), AppError> {
    let created = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| AppError::io(destination, error))?;
    drop(created);
    match backup_into_existing_destination(source, destination) {
        Ok(()) => Ok(()),
        Err(original) => match std::fs::remove_file(destination) {
            Ok(()) => Err(original),
            Err(error) if error.kind() == ErrorKind::NotFound => Err(original),
            Err(cleanup) => Err(AppError::Database(format!(
                "{original}; additionally, failed to remove owned backup destination {}: {cleanup}",
                destination.display()
            ))),
        },
    }
}

fn validate_existing_database(path: &Path, policy: SchemaValidationPolicy) -> Result<(), AppError> {
    let conn = open_read_only(path)?;
    validate_database(&conn, policy)
}

fn create_validated_snapshot(
    source: &Path,
    directory: &PinnedDirectory,
) -> Result<NamedTempFile, AppError> {
    directory.revalidate("snapshot creation")?;
    let snapshot = tempfile::Builder::new()
        .prefix(".llm-usage-bar-db-snapshot-")
        .tempfile_in(&directory.canonical_path)
        .map_err(|error| AppError::io(&directory.canonical_path, error))?;
    backup_into_existing_destination(source, snapshot.path())?;
    snapshot
        .as_file()
        .sync_all()
        .map_err(|error| AppError::io(snapshot.path(), error))?;
    Ok(snapshot)
}

#[derive(Debug)]
struct PublishedFile {
    path: PathBuf,
    identity: FileIdentity,
}

enum PublishOutcome {
    Published(PublishedFile),
    Existing,
}

struct OwnedSnapshotName {
    directory: PinnedDirectory,
    path: PathBuf,
    identity: FileIdentity,
    armed: bool,
}

impl OwnedSnapshotName {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for OwnedSnapshotName {
    fn drop(&mut self) {
        if !self.armed || !path_entry_exists(&self.path).unwrap_or(true) {
            return;
        }
        if let Err(error) = quarantine_remove(
            &self.directory,
            &self.path,
            &self.identity,
            "snapshot-cleanup",
        ) {
            log::warn!(
                "Failed to clean owned migration snapshot {}: {error}",
                self.path.display()
            );
        }
    }
}

fn publish_snapshot_noclobber(
    snapshot: NamedTempFile,
    target: &Path,
    directory: &PinnedDirectory,
) -> Result<PublishOutcome, AppError> {
    directory.revalidate("snapshot publication")?;
    let snapshot_identity = FileIdentity::from_file(snapshot.path(), snapshot.as_file())?;
    snapshot
        .as_file()
        .sync_all()
        .map_err(|error| AppError::io(snapshot.path(), error))?;
    let (file, temp_path) = snapshot.into_parts();
    let source_path = match temp_path.keep() {
        Ok(path) => path,
        Err(error) => {
            let source = error.error;
            let path = error.path.to_path_buf();
            drop(error.path);
            return Err(AppError::io(path, source));
        }
    };
    let mut owned_source = OwnedSnapshotName {
        directory: directory.clone(),
        path: source_path.clone(),
        identity: snapshot_identity.clone(),
        armed: true,
    };
    match atomic_move_noreplace(&source_path, target)? {
        AtomicMoveOutcome::Moved => {
            owned_source.disarm();
            let identity = match FileIdentity::from_file(target, &file) {
                Ok(identity) => identity,
                Err(original) => {
                    #[cfg(any(unix, windows))]
                    {
                        return match quarantine_remove(
                            directory,
                            target,
                            &snapshot_identity,
                            "publish-identity-cleanup",
                        ) {
                            Ok(()) => Err(original),
                            Err(cleanup) => Err(AppError::Database(format!(
                                "{original}; additionally, post-publish identity cleanup failed: {cleanup}"
                            ))),
                        };
                    }
                    #[cfg(all(not(unix), not(windows)))]
                    {
                        return Err(original);
                    }
                }
            };
            #[cfg(any(unix, windows))]
            if identity != snapshot_identity {
                return Err(AppError::Database(format!(
                    "published object identity changed unexpectedly: {}",
                    target.display()
                )));
            }
            if let Err(error) = file.sync_all() {
                let original = AppError::io(target, error);
                return match quarantine_remove(directory, target, &identity, "publish-sync-cleanup")
                {
                    Ok(()) => Err(original),
                    Err(cleanup) => Err(AppError::Database(format!(
                        "{original}; additionally, post-publish cleanup failed: {cleanup}"
                    ))),
                };
            }
            Ok(PublishOutcome::Published(PublishedFile {
                path: target.to_path_buf(),
                identity,
            }))
        }
        AtomicMoveOutcome::DestinationExists => {
            drop(file);
            Ok(PublishOutcome::Existing)
        }
    }
}

fn rollback_published(
    directory: &PinnedDirectory,
    files: &[&PublishedFile],
) -> Result<(), AppError> {
    let mut failures = Vec::new();
    for (index, published) in files.iter().enumerate() {
        if let Err(error) = quarantine_remove(
            directory,
            &published.path,
            &published.identity,
            &format!("rollback-{index}"),
        ) {
            failures.push(format!("rollback {}: {error}", published.path.display()));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(AppError::Database(format!(
            "database identity rollback cleanup failed: {}",
            failures.join("; ")
        )))
    }
}

fn rollback_after(
    original: AppError,
    directory: &PinnedDirectory,
    published: &[&PublishedFile],
) -> Result<DatabaseIdentityOutcome, AppError> {
    match rollback_published(directory, published) {
        Ok(()) => Err(original),
        Err(rollback) => Err(AppError::Database(format!(
            "{original}; additionally, {rollback}"
        ))),
    }
}

fn rollback_after_retirement_fence(
    original: AppError,
    source_barrier: &mut SourceWriteBarrier,
    directory: &PinnedDirectory,
    published: &[&PublishedFile],
) -> Result<DatabaseIdentityOutcome, AppError> {
    match source_barrier.restore_retirement_fence() {
        Ok(()) => rollback_after(original, directory, published),
        Err(restore) => Err(AppError::Database(format!(
            "{original}; additionally, the durable source retirement fence could not be rolled back ({restore}); published database snapshots were retained"
        ))),
    }
}

fn require_unchanged_file_identity(
    path: &Path,
    expected: &FileIdentity,
    checkpoint: &str,
) -> Result<(), AppError> {
    let current = FileIdentity::from_path(path)?;
    if &current != expected {
        return Err(AppError::Lock(format!(
            "{checkpoint}: filesystem object identity changed at {}",
            path.display()
        )));
    }
    Ok(())
}

trait MigrationHooks {
    fn after_directory_pinned(&self, _directory: &PinnedDirectory) -> Result<(), AppError> {
        Ok(())
    }

    fn after_snapshots(&self, _old_path: &Path) -> Result<(), AppError> {
        Ok(())
    }

    fn after_source_barrier(&self, _old_path: &Path) -> Result<(), AppError> {
        Ok(())
    }

    fn before_source_retirement(
        &self,
        _old_path: &Path,
        _new_path: &Path,
        _archive_path: &Path,
    ) -> Result<(), AppError> {
        Ok(())
    }
}

struct NoopMigrationHooks;
impl MigrationHooks for NoopMigrationHooks {}

fn append_durability_warning(target: &mut Option<String>, warning: String) {
    match target {
        Some(existing) => {
            existing.push_str("; ");
            existing.push_str(&warning);
        }
        None => *target = Some(warning),
    }
}

fn finish_with_explicit_lease_release(
    lease: DatabaseMigrationLease,
    migration_result: Result<DatabaseIdentityOutcome, AppError>,
) -> Result<DatabaseIdentityOutcome, AppError> {
    let release_result = lease.release();
    match (migration_result, release_result) {
        (Ok(outcome), Ok(())) => Ok(outcome),
        (Ok(mut outcome), Err(release)) => {
            append_durability_warning(
                &mut outcome.durability_warning,
                format!("database migration lease cleanup failed: {release}"),
            );
            Ok(outcome)
        }
        (Err(original), Ok(())) => Err(original),
        (Err(original), Err(release)) => Err(AppError::Database(format!(
            "{original}; additionally, database migration lease cleanup failed: {release}"
        ))),
    }
}

fn prepare_database_identity_impl(
    app_dir: &Path,
    fault: Option<MigrationFaultPoint>,
    hooks: &dyn MigrationHooks,
    protected_override: Option<&Path>,
) -> Result<DatabaseIdentityOutcome, AppError> {
    let Some(directory) = pin_safe_existing_directory(app_dir, protected_override)? else {
        let prospective_directory = canonicalize_deepest_existing(app_dir)?;
        return Ok(DatabaseIdentityOutcome {
            database_path: prospective_directory.join(DATABASE_FILE),
            archived_prior_path: None,
            retained_prior_path: None,
            migrated: false,
            durability_warning: None,
        });
    };
    hooks.after_directory_pinned(&directory)?;
    directory.revalidate("migration lease")?;

    let new_path = directory.join(DATABASE_FILE);
    let old_path = directory.join(LEGACY_DATABASE_FILE);
    let archive_path = directory.join(DATABASE_IDENTITY_ARCHIVE_FILE);

    let lease = DatabaseMigrationLease::acquire(&directory)?;
    let migration_result = (|| -> Result<DatabaseIdentityOutcome, AppError> {
        directory.revalidate("authoritative state recheck")?;

        if path_entry_exists(&new_path)? {
            // Existing current filenames may be the fixed-v13 crash-window output
            // or any schema this binary supports after startup schema migration.
            validate_existing_database(&new_path, SchemaValidationPolicy::CurrentAuthoritative)?;
            let retained_prior_path = path_entry_exists(&old_path)?.then_some(old_path.clone());
            let durability_warning = if retained_prior_path.is_none() {
                retry_stale_legacy_sidecar_cleanup(&directory, &old_path)
                    .err()
                    .map(|error| {
                        format!(
                            "identity-checked legacy sidecar cleanup failed beside {}: {error}",
                            old_path.display()
                        )
                    })
            } else {
                None
            };
            return Ok(DatabaseIdentityOutcome {
                database_path: new_path,
                archived_prior_path: None,
                retained_prior_path,
                migrated: false,
                durability_warning,
            });
        }

        if !path_entry_exists(&old_path)? {
            if path_entry_exists(&archive_path)? {
                return Err(AppError::Database(format!(
                    "database archive exists without an authoritative database: {}",
                    archive_path.display()
                )));
            }
            return Ok(DatabaseIdentityOutcome {
                database_path: new_path,
                archived_prior_path: None,
                retained_prior_path: None,
                migrated: false,
                durability_warning: None,
            });
        }

        if path_entry_exists(&archive_path)? {
            return Err(AppError::Database(format!(
                "refusing to overwrite pre-existing database archive: {}",
                archive_path.display()
            )));
        }

        let old_identity = FileIdentity::from_path(&old_path)?;
        require_supported_source_retirement()?;
        let mut source_barrier = Some(SourceWriteBarrier::acquire(&old_path)?);
        hooks.after_source_barrier(&old_path)?;
        require_unchanged_file_identity(&old_path, &old_identity, "after source write barrier")?;
        let new_snapshot = create_validated_snapshot(&old_path, &directory)?;
        // The archive is a second SQLite backup of the validated first snapshot,
        // never a copy-on-name, hard link, or alias of old/new.
        let archive_snapshot = create_validated_snapshot(new_snapshot.path(), &directory)?;
        if FileIdentity::from_path(new_snapshot.path())?
            == FileIdentity::from_path(archive_snapshot.path())?
        {
            return Err(AppError::Database(
                "new and archive snapshots unexpectedly share a filesystem object".to_string(),
            ));
        }
        hooks.after_snapshots(&old_path)?;
        require_unchanged_file_identity(&old_path, &old_identity, "after source snapshot backup")?;

        let published_new = match publish_snapshot_noclobber(new_snapshot, &new_path, &directory)? {
            PublishOutcome::Existing => {
                validate_existing_database(
                    &new_path,
                    SchemaValidationPolicy::CurrentAuthoritative,
                )?;
                return Ok(DatabaseIdentityOutcome {
                    database_path: new_path,
                    archived_prior_path: None,
                    retained_prior_path: Some(old_path),
                    migrated: false,
                    durability_warning: None,
                });
            }
            PublishOutcome::Published(published) => published,
        };
        if let Err(error) = sync_directory(&directory.canonical_path) {
            return rollback_after(error, &directory, &[&published_new]);
        }

        if fault == Some(MigrationFaultPoint::ArchivePublish) {
            return rollback_after(
                AppError::Database("injected archive publication failure".to_string()),
                &directory,
                &[&published_new],
            );
        }

        let published_archive =
            match publish_snapshot_noclobber(archive_snapshot, &archive_path, &directory) {
                Ok(PublishOutcome::Published(published)) => published,
                Ok(PublishOutcome::Existing) => {
                    return rollback_after(
                        AppError::Database(format!(
                            "refusing to overwrite concurrently-created database archive: {}",
                            archive_path.display()
                        )),
                        &directory,
                        &[&published_new],
                    );
                }
                Err(error) => return rollback_after(error, &directory, &[&published_new]),
            };
        if published_new.identity == published_archive.identity {
            return rollback_after(
                AppError::Database(
                    "published database and archive share a filesystem object".to_string(),
                ),
                &directory,
                &[&published_archive, &published_new],
            );
        }
        if let Err(error) = sync_directory(&directory.canonical_path) {
            return rollback_after(error, &directory, &[&published_archive, &published_new]);
        }

        if fault == Some(MigrationFaultPoint::OldSourceRemove) {
            return rollback_after(
                AppError::Database("injected old source removal failure".to_string()),
                &directory,
                &[&published_archive, &published_new],
            );
        }

        if let Err(error) = hooks.before_source_retirement(&old_path, &new_path, &archive_path) {
            return rollback_after(error, &directory, &[&published_archive, &published_new]);
        }
        if let Err(error) =
            require_unchanged_file_identity(&old_path, &old_identity, "before source retirement")
        {
            return rollback_after(error, &directory, &[&published_archive, &published_new]);
        }
        if let Err(error) = require_unchanged_file_identity(
            &new_path,
            &published_new.identity,
            "published new database identity changed before source retirement",
        ) {
            return rollback_after(error, &directory, &[&published_archive, &published_new]);
        }
        if let Err(error) = require_unchanged_file_identity(
            &archive_path,
            &published_archive.identity,
            "published archive identity changed before source retirement",
        ) {
            return rollback_after(error, &directory, &[&published_archive, &published_new]);
        }

        if let Err(error) = source_barrier
            .as_mut()
            .expect("source barrier remains owned before retirement")
            .install_and_reacquire_retirement_fence()
        {
            return rollback_after_retirement_fence(
                error,
                source_barrier
                    .as_mut()
                    .expect("source barrier remains owned after fence failure"),
                &directory,
                &[&published_archive, &published_new],
            );
        }

        for (path, expected, checkpoint) in [
            (
                old_path.as_path(),
                &old_identity,
                "immediately before old-source unlink",
            ),
            (
                new_path.as_path(),
                &published_new.identity,
                "published new database identity changed immediately before old-source unlink",
            ),
            (
                archive_path.as_path(),
                &published_archive.identity,
                "published archive identity changed immediately before old-source unlink",
            ),
        ] {
            if let Err(error) = require_unchanged_file_identity(path, expected, checkpoint) {
                return rollback_after_retirement_fence(
                    error,
                    source_barrier
                        .as_mut()
                        .expect("source barrier remains owned for final identity rollback"),
                    &directory,
                    &[&published_archive, &published_new],
                );
            }
        }

        let retire_result = if fault == Some(MigrationFaultPoint::OldSourceDirectorySyncAfterUnlink)
        {
            quarantine_remove_with_directory_sync(
                &directory,
                &old_path,
                &old_identity,
                "retire-old-main",
                |_| {
                    Err(AppError::Database(
                        "injected directory sync failure after old-source unlink".to_string(),
                    ))
                },
            )
        } else {
            quarantine_remove_with_directory_sync(
                &directory,
                &old_path,
                &old_identity,
                "retire-old-main",
                sync_directory,
            )
        };
        let mut durability_warning = match retire_result {
        Ok(QuarantineRemoveOutcome::RemovedDurably) => None,
        Ok(QuarantineRemoveOutcome::RemovedWithDurabilityWarning(error)) => {
            Some(format!(
                "legacy database was unlinked, but directory durability could not be confirmed: {error}"
            ))
        }
        Err(error) => {
            return rollback_after_retirement_fence(
                error,
                source_barrier
                    .as_mut()
                    .expect("source barrier remains owned before retirement rollback"),
                &directory,
                &[&published_archive, &published_new],
            );
        }
    };
        source_barrier
            .as_mut()
            .expect("source barrier remains owned after committed retirement")
            .disarm_retirement_fence();
        drop(source_barrier.take());

        // Open legacy SQLite connections may still own these names and can
        // recreate them after any fixed-path deletion. Leave visible sidecars
        // untouched here; the next authoritative-new startup retries cleanup
        // only when the old main remains absent and after capturing each
        // sidecar's object identity.
        let mut deferred_sidecars = Vec::new();
        let mut sidecar_inspection_failures = Vec::new();
        for sidecar in [
            database_sidecar_path(&old_path, "-wal"),
            database_sidecar_path(&old_path, "-shm"),
        ] {
            match path_entry_exists(&sidecar) {
                Ok(true) => deferred_sidecars.push(sidecar),
                Ok(false) => {}
                Err(error) => sidecar_inspection_failures.push(error.to_string()),
            }
        }
        if !deferred_sidecars.is_empty() {
            append_durability_warning(
                &mut durability_warning,
                format!(
                    "legacy sidecar cleanup deferred to the next authoritative-new startup: {}",
                    deferred_sidecars
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
        }
        if !sidecar_inspection_failures.is_empty() {
            append_durability_warning(
                &mut durability_warning,
                format!(
                    "legacy sidecar deferral could not be fully inspected: {}",
                    sidecar_inspection_failures.join("; ")
                ),
            );
        }
        // Both published snapshots were file-synced and the directory was synced
        // before the old-main commit point. A post-delete directory-sync failure
        // cannot be rolled back safely without recreating the old object; treat the
        // completed migration as authoritative and leave a diagnostic. If a crash
        // resurrects the old directory entry, the validated new filename still wins.
        if let Err(error) = sync_directory(&directory.canonical_path) {
            append_durability_warning(
                &mut durability_warning,
                format!("database directory sync failed after committed legacy cleanup: {error}"),
            );
        }

        Ok(DatabaseIdentityOutcome {
            database_path: new_path,
            archived_prior_path: Some(archive_path),
            retained_prior_path: None,
            migrated: true,
            durability_warning,
        })
    })();
    finish_with_explicit_lease_release(lease, migration_result)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MigrationFaultPoint {
    ArchivePublish,
    OldSourceRemove,
    OldSourceDirectorySyncAfterUnlink,
}

pub(crate) fn prepare_database_identity(
    app_dir: &Path,
) -> Result<DatabaseIdentityOutcome, AppError> {
    prepare_database_identity_impl(app_dir, None, &NoopMigrationHooks, None)
}

#[cfg(test)]
fn prepare_database_identity_with_test_fault(
    app_dir: &Path,
    fault: MigrationFaultPoint,
) -> Result<DatabaseIdentityOutcome, AppError> {
    prepare_database_identity_impl(app_dir, Some(fault), &NoopMigrationHooks, None)
}

#[cfg(test)]
fn prepare_database_identity_with_hooks(
    app_dir: &Path,
    hooks: &dyn MigrationHooks,
    protected_override: Option<&Path>,
) -> Result<DatabaseIdentityOutcome, AppError> {
    prepare_database_identity_impl(app_dir, None, hooks, protected_override)
}

// Database filename migration tests live beside the private implementation so
// they can exercise the real SQLite backup and no-clobber publication path.

#[cfg(test)]
mod tests {
    use super::{
        atomic_move_noreplace_with_mode, backup_and_validate, database_sidecar_path,
        pin_safe_existing_directory, prepare_database_identity,
        prepare_database_identity_with_hooks, prepare_database_identity_with_test_fault,
        quarantine_remove, rollback_after_retirement_fence, AtomicMoveMode, DatabaseMigrationLease,
        FileIdentity, MigrationFaultPoint, MigrationHooks, PinnedDirectory, PublishedFile,
        SourceWriteBarrier, RETIREMENT_FENCE_TRIGGER_PREFIX,
    };
    use crate::database::{Database, SCHEMA_VERSION};
    use crate::product_identity::{
        DATABASE_FILE, DATABASE_IDENTITY_SOURCE_SCHEMA_VERSION, LEGACY_DATABASE_FILE,
    };
    use rusqlite::{Connection, OpenFlags};
    use sha2::{Digest, Sha256};
    use std::fs::File;
    use std::io::Read;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc::sync_channel, Arc, Barrier, Mutex};

    const ARCHIVE_FILE: &str = "cc-switch.db.pre-llm-usage-bar-v14";
    const CRASH_FIXTURE_ENV: &str = "LLM_USAGE_BAR_CRASH_WAL_FIXTURE";
    const MIGRATION_LEASE_FILE: &str = ".llm-usage-bar-database-migration.lock";

    #[derive(Debug, PartialEq, Eq)]
    struct EntrySnapshot {
        relative_path: PathBuf,
        kind: &'static str,
        length: Option<u64>,
        modified_unix_nanos: Option<u128>,
        candidate_digest: Option<[u8; 32]>,
        link_target: Option<PathBuf>,
    }

    fn create_real_v13_fixture(path: &Path) -> Connection {
        assert_eq!(
            SCHEMA_VERSION, DATABASE_IDENTITY_SOURCE_SCHEMA_VERSION,
            "when the application schema advances, this helper must construct a real fixed-v13 fixture"
        );
        let conn = Connection::open(path).expect("open real v13 fixture");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("enable fixture foreign keys");
        Database::create_tables_on_conn(&conn).expect("create real application tables");
        Database::apply_schema_migrations_on_conn(&conn)
            .expect("migrate fixture to current v13 schema");
        assert_eq!(
            Database::get_user_version(&conn).expect("read fixture user_version"),
            DATABASE_IDENTITY_SOURCE_SCHEMA_VERSION
        );
        conn.execute_batch(
            "CREATE TABLE identity_migration_marker (
                value TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT 0
            );",
        )
        .expect("create migration marker table");
        conn
    }

    fn write_real_v13_marker_db(path: &Path, marker: &str) {
        let conn = create_real_v13_fixture(path);
        conn.execute(
            "INSERT INTO identity_migration_marker (value) VALUES (?1)",
            [marker],
        )
        .expect("write fixture marker");
        drop(conn);
    }

    fn read_marker(path: &Path) -> String {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open migrated marker database read-only");
        conn.query_row(
            "SELECT value FROM identity_migration_marker ORDER BY rowid DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("read migration marker")
    }

    fn assert_snapshot_excludes_retirement_fence(path: &Path) {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open migration snapshot read-only");
        assert_eq!(
            Database::get_user_version(&connection).expect("read snapshot user_version"),
            DATABASE_IDENTITY_SOURCE_SCHEMA_VERSION,
            "migration snapshot must retain the source schema version"
        );
        let fence_pattern = format!("{RETIREMENT_FENCE_TRIGGER_PREFIX}%");
        let fence_trigger_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'trigger' AND name LIKE ?1;",
                [fence_pattern],
                |row| row.get(0),
            )
            .expect("count retirement-fence triggers in snapshot");
        assert_eq!(
            fence_trigger_count, 0,
            "retirement fence must never leak into a published snapshot"
        );
    }

    fn enable_wal(conn: &Connection) {
        let mode: String = conn
            .query_row("PRAGMA journal_mode = WAL;", [], |row| row.get(0))
            .expect("enable WAL and consume journal_mode result row");
        assert_eq!(mode.to_ascii_lowercase(), "wal");
        conn.pragma_update(None, "wal_autocheckpoint", 0)
            .expect("disable automatic checkpoint");
    }

    fn wal_path(database: &Path) -> PathBuf {
        let mut name = database
            .file_name()
            .expect("database filename")
            .to_os_string();
        name.push("-wal");
        database.with_file_name(name)
    }

    fn shm_path(database: &Path) -> PathBuf {
        let mut name = database
            .file_name()
            .expect("database filename")
            .to_os_string();
        name.push("-shm");
        database.with_file_name(name)
    }

    fn directory_entry_names(dir: &Path) -> Vec<String> {
        let mut names = std::fs::read_dir(dir)
            .expect("read test directory")
            .map(|entry| {
                entry
                    .expect("read test directory entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    fn canonical_test_dir(dir: &Path) -> PathBuf {
        std::fs::canonicalize(dir).expect("canonicalize test app directory")
    }

    fn assert_old_only(dir: &Path, expected_marker: &str) {
        let old = dir.join(LEGACY_DATABASE_FILE);
        assert_eq!(directory_entry_names(dir), vec![LEGACY_DATABASE_FILE]);
        assert_eq!(read_marker(&old), expected_marker);
        assert!(!dir.join(DATABASE_FILE).exists());
        assert!(!dir.join(ARCHIVE_FILE).exists());
    }

    fn hash_file(path: &Path) -> (u64, [u8; 32]) {
        let mut file = File::open(path).expect("open file for immutable snapshot");
        let mut hasher = Sha256::new();
        let mut length = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).expect("hash snapshot file");
            if read == 0 {
                break;
            }
            length += read as u64;
            hasher.update(&buffer[..read]);
        }
        (length, hasher.finalize().into())
    }

    fn is_migration_candidate(relative_path: &Path) -> bool {
        if relative_path
            .parent()
            .is_some_and(|parent| parent != Path::new(""))
        {
            return false;
        }
        let Some(name) = relative_path.file_name().and_then(|name| name.to_str()) else {
            return false;
        };
        [
            LEGACY_DATABASE_FILE,
            "cc-switch.db-wal",
            "cc-switch.db-shm",
            DATABASE_FILE,
            ARCHIVE_FILE,
            MIGRATION_LEASE_FILE,
        ]
        .contains(&name)
    }

    fn modified_unix_nanos(metadata: &std::fs::Metadata) -> Option<u128> {
        metadata
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_nanos())
    }

    fn snapshot_directory_tree(root: &Path) -> Vec<EntrySnapshot> {
        fn walk(root: &Path, directory: &Path, entries: &mut Vec<EntrySnapshot>) {
            let mut children = std::fs::read_dir(directory)
                .expect("enumerate protected legacy directory")
                .map(|entry| entry.expect("read protected directory entry").path())
                .collect::<Vec<_>>();
            children.sort();

            for path in children {
                let metadata = std::fs::symlink_metadata(&path)
                    .expect("read protected entry metadata without following links");
                let file_type = metadata.file_type();
                let modified_unix_nanos = modified_unix_nanos(&metadata);
                let relative_path = path
                    .strip_prefix(root)
                    .expect("protected entry is inside root")
                    .to_path_buf();
                if file_type.is_dir() {
                    entries.push(EntrySnapshot {
                        relative_path,
                        kind: "directory",
                        length: None,
                        modified_unix_nanos,
                        candidate_digest: None,
                        link_target: None,
                    });
                    walk(root, &path, entries);
                } else if file_type.is_file() {
                    let length = metadata.len();
                    let candidate_digest = is_migration_candidate(&relative_path).then(|| {
                        let (hashed_length, digest) = hash_file(&path);
                        assert_eq!(hashed_length, length, "candidate changed while hashing");
                        digest
                    });
                    entries.push(EntrySnapshot {
                        relative_path,
                        kind: "file",
                        length: Some(length),
                        modified_unix_nanos,
                        candidate_digest,
                        link_target: None,
                    });
                } else if file_type.is_symlink() {
                    entries.push(EntrySnapshot {
                        relative_path,
                        kind: "symlink",
                        length: None,
                        modified_unix_nanos,
                        candidate_digest: None,
                        link_target: Some(
                            std::fs::read_link(&path).expect("snapshot protected symlink target"),
                        ),
                    });
                } else {
                    entries.push(EntrySnapshot {
                        relative_path,
                        kind: "other",
                        length: None,
                        modified_unix_nanos,
                        candidate_digest: None,
                        link_target: None,
                    });
                }
            }
        }

        let mut entries = Vec::new();
        walk(root, root, &mut entries);
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        entries
    }

    fn real_legacy_directory() -> Option<PathBuf> {
        let candidate = dirs::home_dir()?.join(".cc-switch");
        let metadata = std::fs::symlink_metadata(&candidate).ok()?;
        if !metadata.file_type().is_dir() {
            return None;
        }
        Some(
            std::fs::canonicalize(candidate)
                .expect("canonicalize the real protected legacy directory"),
        )
    }

    fn assert_guard_rejects_without_changing_real_legacy(app_dir: &Path, real_legacy: &Path) {
        let before = snapshot_directory_tree(real_legacy);
        let call = catch_unwind(AssertUnwindSafe(|| prepare_database_identity(app_dir)));
        let after = snapshot_directory_tree(real_legacy);
        assert_eq!(
            after, before,
            "guard call changed protected ~/.cc-switch entries or file bytes"
        );
        let result = call.expect("legacy namespace guard must not panic");
        let error = result.expect_err("legacy namespace alias must be rejected");
        assert!(
            error.to_string().contains(".cc-switch"),
            "error must identify the forbidden namespace: {error}"
        );
    }

    fn spawn_crash_wal_fixture(database: &Path) {
        let output = Command::new(std::env::current_exe().expect("current test executable"))
            .arg("--exact")
            .arg("database::identity_migration::tests::crash_wal_fixture_child")
            .arg("--nocapture")
            .arg("--test-threads=1")
            .env(CRASH_FIXTURE_ENV, database)
            .output()
            .expect("spawn crash-WAL fixture subprocess");
        assert!(
            output.status.success(),
            "crash fixture failed: status={:?}\nstdout={}\nstderr={}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(unix)]
    fn assert_different_file_objects(left: &Path, right: &Path) {
        use std::os::unix::fs::MetadataExt;

        let left = std::fs::metadata(left).expect("left metadata");
        let right = std::fs::metadata(right).expect("right metadata");
        assert_ne!(
            (left.dev(), left.ino()),
            (right.dev(), right.ino()),
            "new database and archive must be independent filesystem objects"
        );
    }

    #[cfg(not(unix))]
    fn assert_different_file_objects(left: &Path, right: &Path) {
        assert_ne!(
            std::fs::canonicalize(left).expect("canonical left"),
            std::fs::canonicalize(right).expect("canonical right")
        );
    }

    #[test]
    fn backup_captures_committed_uncheckpointed_wal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = canonical_test_dir(dir.path()).join(LEGACY_DATABASE_FILE);
        let snapshot = dir.path().join("backup-test.db");
        let writer = create_real_v13_fixture(&old);

        enable_wal(&writer);

        let reader = Connection::open(&old).expect("open concurrent reader");
        reader
            .execute_batch("BEGIN; SELECT * FROM identity_migration_marker;")
            .expect("pin reader snapshot");
        writer
            .execute(
                "INSERT INTO identity_migration_marker (value) VALUES ('from-uncheckpointed-wal')",
                [],
            )
            .expect("commit marker to WAL");

        assert!(
            std::fs::metadata(wal_path(&old))
                .expect("WAL exists before backup")
                .len()
                > 32,
            "test precondition: committed row must still live in a nonempty WAL"
        );

        backup_and_validate(&old, &snapshot).expect("backup live WAL database");
        assert_eq!(read_marker(&snapshot), "from-uncheckpointed-wal");

        drop(reader);
        drop(writer);
    }

    #[test]
    fn crash_wal_fixture_child() {
        let Some(path) = std::env::var_os(CRASH_FIXTURE_ENV).map(PathBuf::from) else {
            return;
        };

        let writer = create_real_v13_fixture(&path);
        enable_wal(&writer);
        writer
            .execute(
                "INSERT INTO identity_migration_marker (value) VALUES ('from-crash-residue-wal')",
                [],
            )
            .expect("commit crash-residue marker");
        assert!(
            std::fs::metadata(wal_path(&path))
                .expect("child WAL metadata")
                .len()
                > 32
        );

        // Deliberately bypass all SQLite destructors and last-connection checkpoints.
        std::process::exit(0);
    }

    #[test]
    fn crash_residue_wal_is_preserved_in_independent_new_and_archive_snapshots() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = dir.path().join(LEGACY_DATABASE_FILE);
        let new = canonical_test_dir(dir.path()).join(DATABASE_FILE);
        let archive = canonical_test_dir(dir.path()).join(ARCHIVE_FILE);
        spawn_crash_wal_fixture(&old);

        assert!(
            std::fs::metadata(wal_path(&old))
                .expect("crash-residue WAL exists")
                .len()
                > 32,
            "test precondition: subprocess must leave committed WAL residue"
        );

        let outcome =
            prepare_database_identity(dir.path()).expect("migrate crash-residue database");
        assert_eq!(outcome.database_path, new);
        assert_eq!(
            outcome.archived_prior_path.as_deref(),
            Some(archive.as_path())
        );
        assert!(outcome.migrated);
        assert_eq!(read_marker(&new), "from-crash-residue-wal");
        assert_eq!(read_marker(&archive), "from-crash-residue-wal");
        assert_different_file_objects(&new, &archive);

        let new_conn = Connection::open(&new).expect("open new database for mutation");
        new_conn
            .execute(
                "UPDATE identity_migration_marker SET value = 'new-was-mutated'",
                [],
            )
            .expect("mutate new database");
        drop(new_conn);
        assert_eq!(read_marker(&new), "new-was-mutated");
        assert_eq!(
            read_marker(&archive),
            "from-crash-residue-wal",
            "archive must not be a hard link or other alias of the new database"
        );
        assert!(!old.exists());
        assert!(wal_path(&old).exists());
        assert!(shm_path(&old).exists());
        assert!(outcome
            .durability_warning
            .as_deref()
            .is_some_and(|warning| warning.contains("sidecar cleanup deferred")));

        let retry = prepare_database_identity(dir.path())
            .expect("authoritative-new startup retries deferred sidecars");
        assert_eq!(retry.database_path, new);
        assert!(!retry.migrated);
        assert!(!wal_path(&old).exists());
        assert!(!shm_path(&old).exists());
    }

    #[test]
    fn migrates_real_v13_database_and_archives_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = dir.path().join(LEGACY_DATABASE_FILE);
        let new = canonical_test_dir(dir.path()).join(DATABASE_FILE);
        let archive = canonical_test_dir(dir.path()).join(ARCHIVE_FILE);
        let writer = create_real_v13_fixture(&old);
        enable_wal(&writer);
        writer
            .execute(
                "INSERT INTO identity_migration_marker (value) VALUES ('ordinary-v13-row')",
                [],
            )
            .expect("write ordinary marker");
        drop(writer);

        let outcome = prepare_database_identity(dir.path()).expect("migrate ordinary v13 database");
        assert_eq!(outcome.database_path, new);
        assert_eq!(
            outcome.archived_prior_path.as_deref(),
            Some(archive.as_path())
        );
        assert_eq!(outcome.retained_prior_path, None);
        assert!(outcome.migrated);
        assert_eq!(read_marker(&new), "ordinary-v13-row");
        assert_eq!(read_marker(&archive), "ordinary-v13-row");
        assert!(!old.exists());
    }

    #[test]
    fn validated_new_filename_wins_when_both_files_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = canonical_test_dir(dir.path()).join(LEGACY_DATABASE_FILE);
        let new = canonical_test_dir(dir.path()).join(DATABASE_FILE);
        write_real_v13_marker_db(&old, "old-row");
        write_real_v13_marker_db(&new, "new-row");
        let old_before = std::fs::read(&old).expect("snapshot old database");

        let outcome = prepare_database_identity(dir.path()).expect("select validated new database");
        assert_eq!(outcome.database_path, new);
        assert_eq!(outcome.archived_prior_path, None);
        assert_eq!(outcome.retained_prior_path.as_deref(), Some(old.as_path()));
        assert!(!outcome.migrated);
        assert_eq!(read_marker(&outcome.database_path), "new-row");
        assert_eq!(std::fs::read(&old).expect("read retained old"), old_before);
        assert!(!dir.path().join(ARCHIVE_FILE).exists());
    }

    #[test]
    fn validated_new_filename_retries_stale_legacy_sidecar_cleanup_without_old_main() {
        let dir = tempfile::tempdir().expect("tempdir");
        let canonical_dir = canonical_test_dir(dir.path());
        let old = canonical_dir.join(LEGACY_DATABASE_FILE);
        let new = canonical_dir.join(DATABASE_FILE);
        let old_wal = database_sidecar_path(&old, "-wal");
        let old_shm = database_sidecar_path(&old, "-shm");
        write_real_v13_marker_db(&new, "authoritative-new");
        std::fs::write(&old_wal, b"stale legacy wal").expect("write stale legacy wal");
        std::fs::write(&old_shm, b"stale legacy shm").expect("write stale legacy shm");

        let outcome = prepare_database_identity(dir.path()).expect("select authoritative new");

        assert_eq!(outcome.database_path, new);
        assert!(!outcome.migrated);
        assert!(!old.exists());
        assert!(!old_wal.exists(), "stale legacy WAL must be retried safely");
        assert!(!old_shm.exists(), "stale legacy SHM must be retried safely");
    }

    #[test]
    fn validated_new_filename_surfaces_stale_sidecar_cleanup_warning() {
        let dir = tempfile::tempdir().expect("tempdir");
        let canonical_dir = canonical_test_dir(dir.path());
        let old = canonical_dir.join(LEGACY_DATABASE_FILE);
        let new = canonical_dir.join(DATABASE_FILE);
        let malformed_old_wal = database_sidecar_path(&old, "-wal");
        write_real_v13_marker_db(&new, "authoritative-new-with-warning");
        std::fs::create_dir(&malformed_old_wal).expect("create undeletable sidecar-shaped entry");

        let outcome = prepare_database_identity(dir.path()).expect("select authoritative new");

        assert_eq!(outcome.database_path, new);
        assert!(!outcome.migrated);
        assert!(
            outcome
                .durability_warning
                .as_deref()
                .is_some_and(|warning| warning.contains("legacy sidecar cleanup failed")),
            "sidecar cleanup failure must be visible to startup diagnostics: {:?}",
            outcome.durability_warning
        );
        assert!(
            malformed_old_wal.is_dir(),
            "failed cleanup must restore the mismatched sidecar-shaped entry"
        );
    }

    #[test]
    fn fresh_install_returns_new_authoritative_path_without_creating_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let outcome = prepare_database_identity(dir.path()).expect("prepare fresh install");

        assert_eq!(
            outcome.database_path,
            canonical_test_dir(dir.path()).join(DATABASE_FILE)
        );
        assert_eq!(outcome.archived_prior_path, None);
        assert_eq!(outcome.retained_prior_path, None);
        assert!(!outcome.migrated);
        assert!(directory_entry_names(dir.path()).is_empty());
    }

    #[test]
    fn invalid_new_filename_is_rejected_instead_of_masking_valid_old_database() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = dir.path().join(LEGACY_DATABASE_FILE);
        let new = dir.path().join(DATABASE_FILE);
        write_real_v13_marker_db(&old, "valid-old");
        std::fs::write(&new, b"not a sqlite database").expect("write invalid new database");
        let old_before = std::fs::read(&old).expect("snapshot old");
        let new_before = std::fs::read(&new).expect("snapshot new");

        prepare_database_identity(dir.path()).expect_err("invalid new database must not win");

        assert_eq!(std::fs::read(&old).expect("read old"), old_before);
        assert_eq!(std::fs::read(&new).expect("read invalid new"), new_before);
        assert!(!dir.path().join(ARCHIVE_FILE).exists());
    }

    #[test]
    fn invalid_sqlite_source_is_rejected_without_modification() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = dir.path().join(LEGACY_DATABASE_FILE);
        std::fs::write(&old, b"not sqlite").expect("write invalid old database");
        let before = std::fs::read(&old).expect("snapshot invalid old");

        prepare_database_identity(dir.path()).expect_err("invalid SQLite must be rejected");

        assert_eq!(std::fs::read(&old).expect("read invalid old"), before);
        assert_eq!(
            directory_entry_names(dir.path()),
            vec![LEGACY_DATABASE_FILE]
        );
    }

    #[test]
    fn source_failing_quick_check_is_rejected_without_publication() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = dir.path().join(LEGACY_DATABASE_FILE);
        let conn = create_real_v13_fixture(&old);
        conn.execute(
            "INSERT INTO identity_migration_marker (value) VALUES ('quick-check-source')",
            [],
        )
        .expect("write marker");
        conn.execute_batch(
            "PRAGMA writable_schema = ON;
             UPDATE sqlite_schema SET rootpage = 2147483647
             WHERE type = 'table' AND name = 'providers';
             PRAGMA writable_schema = OFF;",
        )
        .expect("create structurally corrupt quick_check fixture");
        drop(conn);
        let before = std::fs::read(&old).expect("snapshot quick_check source");

        let error = prepare_database_identity(dir.path())
            .expect_err("quick_check corruption must be rejected");

        assert!(
            error.to_string().to_lowercase().contains("quick_check"),
            "validation error must name quick_check: {error}"
        );
        assert_eq!(
            std::fs::read(&old).expect("read quick_check source"),
            before
        );
        assert_eq!(
            directory_entry_names(dir.path()),
            vec![LEGACY_DATABASE_FILE]
        );
    }

    #[test]
    fn non_v13_source_user_version_is_rejected_without_modification() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = dir.path().join(LEGACY_DATABASE_FILE);
        let conn = create_real_v13_fixture(&old);
        conn.execute(
            "INSERT INTO identity_migration_marker (value) VALUES ('future-source')",
            [],
        )
        .expect("write marker");
        Database::set_user_version(&conn, DATABASE_IDENTITY_SOURCE_SCHEMA_VERSION + 1)
            .expect("set unsupported migration-source version");
        drop(conn);
        let before = std::fs::read(&old).expect("snapshot future database");

        prepare_database_identity(dir.path()).expect_err("future database must be rejected");

        assert_eq!(std::fs::read(&old).expect("read future database"), before);
        assert_eq!(
            directory_entry_names(dir.path()),
            vec![LEGACY_DATABASE_FILE]
        );
    }

    #[test]
    fn missing_required_v13_table_is_rejected_without_modification() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = dir.path().join(LEGACY_DATABASE_FILE);
        let conn = create_real_v13_fixture(&old);
        conn.execute_batch(
            "INSERT INTO identity_migration_marker (value) VALUES ('missing-table-source');
             DROP TABLE usage_events;",
        )
        .expect("remove required table");
        drop(conn);
        let before = std::fs::read(&old).expect("snapshot incomplete database");

        prepare_database_identity(dir.path()).expect_err("missing required table must be rejected");

        assert_eq!(
            std::fs::read(&old).expect("read incomplete database"),
            before
        );
        assert_eq!(
            directory_entry_names(dir.path()),
            vec![LEGACY_DATABASE_FILE]
        );
    }

    #[test]
    fn existing_archive_is_never_overwritten() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = dir.path().join(LEGACY_DATABASE_FILE);
        let archive = dir.path().join(ARCHIVE_FILE);
        write_real_v13_marker_db(&old, "archive-collision-source");
        std::fs::write(&archive, b"preexisting archive evidence")
            .expect("write preexisting archive");
        let old_before = std::fs::read(&old).expect("snapshot old");
        let archive_before = std::fs::read(&archive).expect("snapshot archive");

        prepare_database_identity(dir.path()).expect_err("archive collision must fail closed");

        assert_eq!(std::fs::read(&old).expect("read old"), old_before);
        assert_eq!(
            std::fs::read(&archive).expect("read archive"),
            archive_before
        );
        assert_eq!(
            directory_entry_names(dir.path()),
            vec![LEGACY_DATABASE_FILE, ARCHIVE_FILE]
        );
    }

    #[test]
    fn concurrent_callers_share_one_no_clobber_migration_result() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_real_v13_marker_db(&dir.path().join(LEGACY_DATABASE_FILE), "concurrent-source");
        let app_dir = Arc::new(dir.path().to_path_buf());
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();

        for _ in 0..2 {
            let app_dir = Arc::clone(&app_dir);
            let barrier = Arc::clone(&barrier);
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                prepare_database_identity(&app_dir)
            }));
        }
        barrier.wait();
        let outcomes = workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .expect("migration worker must not panic")
                    .expect("concurrent migration must succeed")
            })
            .collect::<Vec<_>>();

        assert_eq!(
            outcomes.iter().filter(|outcome| outcome.migrated).count(),
            1
        );
        assert!(outcomes.iter().all(
            |outcome| outcome.database_path == canonical_test_dir(&app_dir).join(DATABASE_FILE)
        ));
        assert_eq!(
            read_marker(&app_dir.join(DATABASE_FILE)),
            "concurrent-source"
        );
        assert_eq!(
            read_marker(&app_dir.join(ARCHIVE_FILE)),
            "concurrent-source"
        );
        assert!(!app_dir.join(LEGACY_DATABASE_FILE).exists());
        assert!(!app_dir.join(MIGRATION_LEASE_FILE).exists());
    }

    #[test]
    fn archive_publish_failure_after_new_publish_rolls_back_to_old_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_real_v13_marker_db(
            &dir.path().join(LEGACY_DATABASE_FILE),
            "archive-fault-source",
        );

        prepare_database_identity_with_test_fault(dir.path(), MigrationFaultPoint::ArchivePublish)
            .expect_err("injected archive publication failure");

        assert_old_only(dir.path(), "archive-fault-source");
    }

    #[test]
    fn old_delete_failure_after_publications_rolls_back_to_old_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_real_v13_marker_db(
            &dir.path().join(LEGACY_DATABASE_FILE),
            "delete-fault-source",
        );

        prepare_database_identity_with_test_fault(dir.path(), MigrationFaultPoint::OldSourceRemove)
            .expect_err("injected old source removal failure");

        assert_old_only(dir.path(), "delete-fault-source");
    }

    #[test]
    fn post_unlink_directory_sync_failure_keeps_committed_publications() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = dir.path().join(LEGACY_DATABASE_FILE);
        let new = canonical_test_dir(dir.path()).join(DATABASE_FILE);
        let archive = canonical_test_dir(dir.path()).join(ARCHIVE_FILE);
        write_real_v13_marker_db(&old, "post-unlink-sync-source");

        let outcome = prepare_database_identity_with_test_fault(
            dir.path(),
            MigrationFaultPoint::OldSourceDirectorySyncAfterUnlink,
        )
        .expect("unlink is the irreversible migration commit point");

        assert!(outcome.migrated);
        assert!(
            outcome.durability_warning.is_some(),
            "post-commit sync failure must be surfaced to startup diagnostics"
        );
        assert_eq!(outcome.database_path, new);
        assert_eq!(
            outcome.archived_prior_path.as_deref(),
            Some(archive.as_path())
        );
        assert!(!old.exists());
        assert_eq!(read_marker(&new), "post-unlink-sync-source");
        assert_eq!(read_marker(&archive), "post-unlink-sync-source");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn successful_migration_defers_live_legacy_sidecars_to_next_startup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = canonical_test_dir(dir.path()).join(LEGACY_DATABASE_FILE);
        let old_wal = database_sidecar_path(&old, "-wal");
        let old_shm = database_sidecar_path(&old, "-shm");
        let fixture = create_real_v13_fixture(&old);
        enable_wal(&fixture);
        fixture
            .execute(
                "INSERT INTO identity_migration_marker (value) VALUES ('live-sidecar-source')",
                [],
            )
            .expect("write live-sidecar fixture marker");
        assert!(old_wal.exists());
        assert!(old_shm.exists());

        let outcome = prepare_database_identity(dir.path()).expect("migrate live WAL source");

        assert!(outcome.migrated);
        assert!(
            outcome
                .durability_warning
                .as_deref()
                .is_some_and(|warning| warning.contains("sidecar cleanup deferred")),
            "deferred cleanup must be visible to startup diagnostics: {:?}",
            outcome.durability_warning
        );
        assert!(
            old_wal.exists(),
            "live WAL must not be deleted by fixed path"
        );
        assert!(
            old_shm.exists(),
            "live SHM must not be deleted by fixed path"
        );
        drop(fixture);
    }

    fn assert_existing_lease_fails_closed(contents: &[u8]) {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = dir.path().join(LEGACY_DATABASE_FILE);
        let lease = dir.path().join(MIGRATION_LEASE_FILE);
        write_real_v13_marker_db(&old, "lease-protected-source");
        std::fs::write(&lease, contents).expect("write preexisting lease");
        let old_before = std::fs::read(&old).expect("snapshot lease-protected database");
        let lease_before = std::fs::read(&lease).expect("snapshot lease");

        let error =
            prepare_database_identity(dir.path()).expect_err("preexisting lease must fail closed");

        assert!(
            error.to_string().contains(MIGRATION_LEASE_FILE),
            "lease error must identify operator-recovery path: {error}"
        );
        assert_eq!(std::fs::read(&old).expect("read old"), old_before);
        assert_eq!(std::fs::read(&lease).expect("read lease"), lease_before);
        assert_eq!(
            directory_entry_names(dir.path()),
            vec![MIGRATION_LEASE_FILE, LEGACY_DATABASE_FILE]
        );
    }

    #[test]
    fn live_lease_fails_closed_without_unlinking_owner_evidence() {
        let metadata = format!(
            "{{\"version\":1,\"pid\":{},\"created_unix_ms\":1,\"state\":\"running\"}}",
            std::process::id()
        );
        assert_existing_lease_fails_closed(metadata.as_bytes());
    }

    #[test]
    fn database_migration_lease_has_explicit_fallible_release() {
        let dir = tempfile::tempdir().expect("tempdir");
        let directory = pin_safe_existing_directory(dir.path(), None)
            .expect("pin lease test directory")
            .expect("existing directory");
        let lease_path = canonical_test_dir(dir.path()).join(MIGRATION_LEASE_FILE);
        let lease = DatabaseMigrationLease::acquire(&directory).expect("acquire lease");
        assert!(lease_path.exists());

        lease.release().expect("release lease explicitly");

        assert!(!lease_path.exists());
    }

    #[test]
    fn malformed_or_partial_lease_fails_closed() {
        assert_existing_lease_fails_closed(br#"{"version":1,"pid":"#);
    }

    #[test]
    fn age_old_apparently_dead_owner_lease_still_fails_closed() {
        assert_existing_lease_fails_closed(
            br#"{"version":1,"pid":4294967295,"created_unix_ms":0,"state":"running"}"#,
        );
    }

    #[test]
    #[serial_test::serial]
    fn lexical_alias_to_real_legacy_directory_is_rejected_before_any_write() {
        let Some(real_legacy) = real_legacy_directory() else {
            eprintln!("skipping real-directory guard: ~/.cc-switch does not exist");
            return;
        };
        let name = real_legacy
            .file_name()
            .expect("real legacy directory name")
            .to_os_string();
        let lexical_alias = real_legacy.join("..").join(name);

        assert_guard_rejects_without_changing_real_legacy(&lexical_alias, &real_legacy);
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn symlink_alias_to_real_legacy_directory_is_rejected_before_any_write() {
        let Some(real_legacy) = real_legacy_directory() else {
            eprintln!("skipping real-directory guard: ~/.cc-switch does not exist");
            return;
        };
        let temp = tempfile::tempdir().expect("tempdir");
        let symlink_alias = temp.path().join("legacy-directory-alias");
        std::os::unix::fs::symlink(&real_legacy, &symlink_alias)
            .expect("create symlink alias to protected legacy directory");

        assert_guard_rejects_without_changing_real_legacy(&symlink_alias, &real_legacy);
        assert_eq!(
            directory_entry_names(temp.path()),
            vec!["legacy-directory-alias"]
        );
    }

    #[test]
    #[serial_test::serial]
    fn hardlink_old_path_alias_to_real_legacy_database_is_rejected_before_any_write() {
        let Some(real_legacy) = real_legacy_directory() else {
            eprintln!("skipping hardlink guard: ~/.cc-switch does not exist");
            return;
        };
        let real_database = real_legacy.join(LEGACY_DATABASE_FILE);
        let Ok(metadata) = std::fs::symlink_metadata(&real_database) else {
            eprintln!("skipping hardlink guard: real legacy database does not exist");
            return;
        };
        if !metadata.file_type().is_file() {
            eprintln!("skipping hardlink guard: real legacy database is not a regular file");
            return;
        }

        let before = snapshot_directory_tree(&real_legacy);
        let temp = tempfile::tempdir().expect("tempdir");
        let hardlink_alias = temp.path().join(LEGACY_DATABASE_FILE);
        if let Err(error) = std::fs::hard_link(&real_database, &hardlink_alias) {
            let after = snapshot_directory_tree(&real_legacy);
            assert_eq!(after, before);
            eprintln!("skipping hardlink guard because filesystem rejected hard link: {error}");
            return;
        }

        let call = catch_unwind(AssertUnwindSafe(|| prepare_database_identity(temp.path())));
        let after = snapshot_directory_tree(&real_legacy);
        assert_eq!(
            after, before,
            "hardlink guard call changed protected ~/.cc-switch entries or file bytes"
        );
        let result = call.expect("hardlink alias guard must not panic");
        let error = result.expect_err("hardlink alias to protected database must be rejected");
        assert!(
            error.to_string().contains(LEGACY_DATABASE_FILE),
            "error must identify protected database alias: {error}"
        );
        assert!(!temp.path().join(DATABASE_FILE).exists());
        assert!(!temp.path().join(ARCHIVE_FILE).exists());
        assert!(!temp.path().join(MIGRATION_LEASE_FILE).exists());
    }

    #[test]
    fn missing_app_directory_returns_fresh_path_without_creating_directory() {
        let root = tempfile::tempdir().expect("tempdir");
        let missing = root.path().join("not-created-by-migration");

        let outcome = prepare_database_identity(&missing).expect("fresh identity outcome");

        assert_eq!(
            outcome.database_path,
            canonical_test_dir(root.path())
                .join("not-created-by-migration")
                .join(DATABASE_FILE)
        );
        assert!(!missing.exists(), "identity preparation must not mkdir");
    }

    #[cfg(unix)]
    #[test]
    fn missing_app_directory_returns_canonical_prospective_database_path() {
        let root = tempfile::tempdir().expect("tempdir");
        let real_parent = root.path().join("real-parent");
        std::fs::create_dir(&real_parent).expect("create real parent");
        let alias_parent = root.path().join("alias-parent");
        std::os::unix::fs::symlink(&real_parent, &alias_parent).expect("create parent alias");
        let missing = alias_parent.join("future-app-dir");
        let expected = canonical_test_dir(&real_parent)
            .join("future-app-dir")
            .join(DATABASE_FILE);

        let outcome = prepare_database_identity(&missing).expect("fresh prospective identity");

        assert_eq!(outcome.database_path, expected);
        assert!(!missing.exists(), "identity preparation must not mkdir");
    }

    #[test]
    fn source_write_barrier_allows_backup_but_rejects_competing_commit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = dir.path().join(LEGACY_DATABASE_FILE);
        write_real_v13_marker_db(&old, "before-barrier");
        let barrier = SourceWriteBarrier::acquire(&old).expect("acquire source write barrier");
        let snapshot = dir.path().join("barrier-snapshot.db");

        backup_and_validate(&old, &snapshot).expect("backup while BEGIN IMMEDIATE is held");
        let competitor = Connection::open(&old).expect("open competing writer");
        competitor
            .busy_timeout(std::time::Duration::ZERO)
            .expect("zero competitor timeout");
        let error = competitor
            .execute(
                "INSERT INTO identity_migration_marker (value) VALUES ('after-snapshot')",
                [],
            )
            .expect_err("competing commit must not pass the write barrier");
        assert!(matches!(
            error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
        ));
        drop(barrier);
        assert_eq!(read_marker(&snapshot), "before-barrier");
    }

    #[test]
    fn failed_fence_install_with_failed_rollback_remains_recoverable_and_uncertain() {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};

        let dir = tempfile::tempdir().expect("tempdir");
        let old = dir.path().join(LEGACY_DATABASE_FILE);
        write_real_v13_marker_db(&old, "uncertain-fence-source");
        let mut barrier = SourceWriteBarrier::acquire(&old).expect("acquire source barrier");
        barrier
            .connection
            .authorizer(Some(|context: AuthContext<'_>| match context.action {
                AuthAction::CreateTrigger { .. }
                | AuthAction::Transaction {
                    operation: TransactionOperation::Rollback,
                } => Authorization::Deny,
                _ => Authorization::Allow,
            }))
            .expect("install fence-failure authorizer");

        barrier
            .install_and_reacquire_retirement_fence()
            .expect_err("trigger install and transaction rollback are both denied");

        assert!(
            barrier.retirement_fence.is_some(),
            "failed rollback must retain enough fence metadata for recovery"
        );
        barrier
            .connection
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .expect("clear fence-failure authorizer");
        barrier
            .restore_retirement_fence()
            .expect("uncertain fence state must be recoverable");
        assert!(barrier.retirement_fence.is_none());
        assert_eq!(
            Database::get_user_version(&barrier.connection).expect("read restored user_version"),
            DATABASE_IDENTITY_SOURCE_SCHEMA_VERSION
        );
    }

    #[test]
    fn unrecoverable_uncertain_fence_retains_published_snapshots() {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};

        let dir = tempfile::tempdir().expect("tempdir");
        let directory = pin_safe_existing_directory(dir.path(), None)
            .expect("pin uncertain-fence directory")
            .expect("existing directory");
        let old = canonical_test_dir(dir.path()).join(LEGACY_DATABASE_FILE);
        let new = canonical_test_dir(dir.path()).join(DATABASE_FILE);
        let archive = canonical_test_dir(dir.path()).join(ARCHIVE_FILE);
        write_real_v13_marker_db(&old, "unrecoverable-fence-source");
        std::fs::write(&new, b"published-new-evidence").expect("write published new evidence");
        std::fs::write(&archive, b"published-archive-evidence")
            .expect("write published archive evidence");
        let published_new = PublishedFile {
            path: new.clone(),
            identity: FileIdentity::from_path(&new).expect("identify published new"),
        };
        let published_archive = PublishedFile {
            path: archive.clone(),
            identity: FileIdentity::from_path(&archive).expect("identify published archive"),
        };
        let mut barrier = SourceWriteBarrier::acquire(&old).expect("acquire source barrier");
        let mut create_trigger_count = 0_u32;
        barrier
            .connection
            .authorizer(Some(move |context: AuthContext<'_>| match context.action {
                AuthAction::CreateTrigger { .. } => {
                    create_trigger_count += 1;
                    if create_trigger_count == 1 {
                        Authorization::Allow
                    } else {
                        Authorization::Deny
                    }
                }
                AuthAction::DropTrigger { .. }
                | AuthAction::Transaction {
                    operation: TransactionOperation::Rollback,
                } => Authorization::Deny,
                _ => Authorization::Allow,
            }))
            .expect("install unrecoverable-fence authorizer");
        let install_error = barrier
            .install_and_reacquire_retirement_fence()
            .expect_err("fence install and rollback must fail");

        let error = rollback_after_retirement_fence(
            install_error,
            &mut barrier,
            &directory,
            &[&published_archive, &published_new],
        )
        .expect_err("unrestored fence must fail without publication rollback");

        assert!(error.to_string().contains("snapshots were retained"));
        assert_eq!(
            std::fs::read(&new).expect("read retained new"),
            b"published-new-evidence"
        );
        assert_eq!(
            std::fs::read(&archive).expect("read retained archive"),
            b"published-archive-evidence"
        );
    }

    #[test]
    fn migration_hook_proves_post_snapshot_competing_commit_cannot_succeed() {
        struct CompetingWriterHook {
            attempted: Arc<AtomicBool>,
        }
        impl MigrationHooks for CompetingWriterHook {
            fn after_snapshots(&self, old_path: &Path) -> Result<(), crate::error::AppError> {
                let competitor = Connection::open(old_path)?;
                competitor.busy_timeout(std::time::Duration::ZERO)?;
                let error = competitor
                    .execute(
                        "INSERT INTO identity_migration_marker (value) VALUES ('lost-race-row')",
                        [],
                    )
                    .expect_err("post-snapshot competing writer must remain blocked");
                if !matches!(
                    error.sqlite_error_code(),
                    Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
                ) {
                    return Err(error.into());
                }
                self.attempted.store(true, Ordering::SeqCst);
                Ok(())
            }
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let old = dir.path().join(LEGACY_DATABASE_FILE);
        write_real_v13_marker_db(&old, "snapshot-boundary");
        let attempted = Arc::new(AtomicBool::new(false));
        let hook = CompetingWriterHook {
            attempted: Arc::clone(&attempted),
        };

        let outcome = prepare_database_identity_with_hooks(dir.path(), &hook, None)
            .expect("migration with competing writer probe");

        assert!(attempted.load(Ordering::SeqCst));
        assert_eq!(read_marker(&outcome.database_path), "snapshot-boundary");
        assert_eq!(
            read_marker(outcome.archived_prior_path.as_deref().unwrap()),
            "snapshot-boundary"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn migration_rejects_writer_already_waiting_with_nonzero_busy_timeout() {
        struct WaitingWriterHook {
            writer: Mutex<Option<std::thread::JoinHandle<rusqlite::Result<usize>>>>,
        }

        impl MigrationHooks for WaitingWriterHook {
            fn after_snapshots(&self, old_path: &Path) -> Result<(), crate::error::AppError> {
                let competitor = Connection::open(old_path)?;
                competitor.busy_timeout(std::time::Duration::from_secs(5))?;
                let (started_tx, started_rx) = sync_channel(0);
                let writer = std::thread::spawn(move || {
                    started_tx
                        .send(())
                        .expect("migration test must observe writer start");
                    competitor.execute(
                        "INSERT INTO identity_migration_marker (value) VALUES ('waiting-writer-row')",
                        [],
                    )
                });
                started_rx
                    .recv()
                    .expect("waiting writer must reach its INSERT");
                // The migration still owns BEGIN IMMEDIATE, so a writer that
                // has not returned after this interval is inside SQLite's
                // nonzero busy wait rather than merely waiting to be scheduled.
                std::thread::sleep(std::time::Duration::from_millis(100));
                assert!(
                    !writer.is_finished(),
                    "writer must be waiting on the barrier"
                );
                *self.writer.lock().expect("lock writer handle") = Some(writer);
                Ok(())
            }
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let old = dir.path().join(LEGACY_DATABASE_FILE);
        let fixture = create_real_v13_fixture(&old);
        enable_wal(&fixture);
        fixture
            .execute(
                "INSERT INTO identity_migration_marker (value) VALUES ('waiting-writer-boundary')",
                [],
            )
            .expect("write waiting-writer fixture marker");
        drop(fixture);
        let hook = WaitingWriterHook {
            writer: Mutex::new(None),
        };

        let outcome = prepare_database_identity_with_hooks(dir.path(), &hook, None)
            .expect("migration with waiting writer");
        let writer = hook
            .writer
            .lock()
            .expect("lock writer handle")
            .take()
            .expect("waiting writer handle");
        let error = writer
            .join()
            .expect("waiting writer must not panic")
            .expect_err("retirement fence must reject an already-waiting write");

        assert_eq!(
            error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::ConstraintViolation),
            "persistent retirement fence must reject the write: {error}"
        );
        assert_eq!(
            read_marker(&outcome.database_path),
            "waiting-writer-boundary"
        );
        assert_eq!(
            read_marker(outcome.archived_prior_path.as_deref().unwrap()),
            "waiting-writer-boundary"
        );
        assert_snapshot_excludes_retirement_fence(&outcome.database_path);
        assert_snapshot_excludes_retirement_fence(outcome.archived_prior_path.as_deref().unwrap());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn source_identity_change_after_write_barrier_fails_before_snapshot_publication() {
        struct ReplaceSourceAfterBarrier;
        impl MigrationHooks for ReplaceSourceAfterBarrier {
            fn after_source_barrier(&self, old_path: &Path) -> Result<(), crate::error::AppError> {
                let displaced = old_path.with_extension("db.displaced-after-barrier");
                std::fs::rename(old_path, &displaced)
                    .map_err(|error| crate::error::AppError::io(old_path, error))?;
                write_real_v13_marker_db(old_path, "replacement-after-barrier");
                Ok(())
            }
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let old = dir.path().join(LEGACY_DATABASE_FILE);
        write_real_v13_marker_db(&old, "original-before-barrier");

        let error =
            prepare_database_identity_with_hooks(dir.path(), &ReplaceSourceAfterBarrier, None)
                .expect_err("source replacement after barrier must fail closed");

        assert!(
            error.to_string().contains("after source write barrier"),
            "error must identify the failed identity checkpoint: {error}"
        );
        assert_eq!(read_marker(&old), "replacement-after-barrier");
        assert!(!dir.path().join(DATABASE_FILE).exists());
        assert!(!dir.path().join(ARCHIVE_FILE).exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn source_identity_change_after_backups_fails_before_snapshot_publication() {
        struct ReplaceSourceAfterBackups;
        impl MigrationHooks for ReplaceSourceAfterBackups {
            fn after_snapshots(&self, old_path: &Path) -> Result<(), crate::error::AppError> {
                let displaced = old_path.with_extension("db.displaced-after-backups");
                std::fs::rename(old_path, &displaced)
                    .map_err(|error| crate::error::AppError::io(old_path, error))?;
                write_real_v13_marker_db(old_path, "replacement-after-backups");
                Ok(())
            }
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let old = dir.path().join(LEGACY_DATABASE_FILE);
        write_real_v13_marker_db(&old, "original-before-backups");

        let error =
            prepare_database_identity_with_hooks(dir.path(), &ReplaceSourceAfterBackups, None)
                .expect_err("source replacement after backups must fail closed");

        assert!(
            error.to_string().contains("after source snapshot backup"),
            "error must identify the failed identity checkpoint: {error}"
        );
        assert_eq!(read_marker(&old), "replacement-after-backups");
        assert!(!dir.path().join(DATABASE_FILE).exists());
        assert!(!dir.path().join(ARCHIVE_FILE).exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn published_identity_change_before_retirement_fails_closed() {
        struct ReplacePublishedNew;
        impl MigrationHooks for ReplacePublishedNew {
            fn before_source_retirement(
                &self,
                _old_path: &Path,
                new_path: &Path,
                _archive_path: &Path,
            ) -> Result<(), crate::error::AppError> {
                std::fs::remove_file(new_path)
                    .map_err(|error| crate::error::AppError::io(new_path, error))?;
                write_real_v13_marker_db(new_path, "replacement-published-new");
                Ok(())
            }
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let old = dir.path().join(LEGACY_DATABASE_FILE);
        let new = canonical_test_dir(dir.path()).join(DATABASE_FILE);
        write_real_v13_marker_db(&old, "original-published-source");

        let error = prepare_database_identity_with_hooks(dir.path(), &ReplacePublishedNew, None)
            .expect_err("published replacement before retirement must fail closed");

        assert!(
            error
                .to_string()
                .contains("published new database identity changed"),
            "error must identify the pre-retirement checkpoint: {error}"
        );
        assert_eq!(read_marker(&new), "replacement-published-new");
        assert_eq!(read_marker(&old), "original-published-source");
        assert!(!dir.path().join(ARCHIVE_FILE).exists());
    }

    #[cfg(unix)]
    #[test]
    fn pinned_canonical_directory_ignores_later_alias_swap_to_protected_root() {
        struct SwapAliasHook {
            alias: PathBuf,
            protected: PathBuf,
        }
        impl MigrationHooks for SwapAliasHook {
            fn after_directory_pinned(
                &self,
                _directory: &PinnedDirectory,
            ) -> Result<(), crate::error::AppError> {
                std::fs::remove_file(&self.alias)
                    .map_err(|error| crate::error::AppError::io(&self.alias, error))?;
                std::os::unix::fs::symlink(&self.protected, &self.alias)
                    .map_err(|error| crate::error::AppError::io(&self.alias, error))
            }
        }

        let root = tempfile::tempdir().expect("tempdir");
        let safe = root.path().join("safe-app-dir");
        let protected = root.path().join("fake-protected-legacy");
        let alias = root.path().join("app-dir-alias");
        std::fs::create_dir(&safe).expect("create safe directory");
        std::fs::create_dir(&protected).expect("create fake protected directory");
        std::os::unix::fs::symlink(&safe, &alias).expect("create initial safe alias");
        let protected_before = directory_entry_names(&protected);
        let hook = SwapAliasHook {
            alias: alias.clone(),
            protected: protected.clone(),
        };

        let outcome = prepare_database_identity_with_hooks(&alias, &hook, Some(&protected))
            .expect("pinned safe directory remains authoritative");

        assert_eq!(
            outcome.database_path,
            canonical_test_dir(&safe).join(DATABASE_FILE)
        );
        assert_eq!(directory_entry_names(&protected), protected_before);
        assert!(directory_entry_names(&safe).is_empty());
        assert_eq!(
            std::fs::canonicalize(&alias).unwrap(),
            std::fs::canonicalize(&protected).unwrap()
        );
    }

    #[test]
    fn successful_publication_leaves_no_hidden_snapshot_or_quarantine_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_real_v13_marker_db(&dir.path().join(LEGACY_DATABASE_FILE), "no-hidden-link");

        prepare_database_identity(dir.path()).expect("migrate database");

        let names = directory_entry_names(dir.path());
        assert_eq!(names, vec![ARCHIVE_FILE, DATABASE_FILE]);
        assert!(names
            .iter()
            .all(|name| !name.starts_with(".llm-usage-bar-")));
    }

    #[test]
    fn unsupported_atomic_move_fails_closed_without_changing_either_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source");
        let destination = dir.path().join("destination");
        std::fs::write(&source, b"source-evidence").expect("write source");

        let error = atomic_move_noreplace_with_mode(
            &source,
            &destination,
            AtomicMoveMode::UnsupportedForTest,
        )
        .expect_err("unsupported backend must fail closed");

        assert!(error.to_string().contains("unsupported"));
        assert_eq!(std::fs::read(&source).unwrap(), b"source-evidence");
        assert!(!destination.exists());
    }

    #[test]
    fn quarantine_identity_mismatch_restores_replacement_instead_of_deleting_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let directory = pin_safe_existing_directory(dir.path(), None)
            .expect("pin test directory")
            .expect("existing test directory");
        let path = directory.join("published.db");
        std::fs::write(&path, b"owned-object").expect("write owned object");
        let expected = FileIdentity::from_path(&path).expect("capture owned identity");
        std::fs::remove_file(&path).expect("remove owned object before replacement");
        std::fs::write(&path, b"replacement-evidence").expect("write replacement");

        let error = quarantine_remove(&directory, &path, &expected, "mismatch-test")
            .expect_err("replacement identity must not be deleted");

        assert!(error.to_string().contains("restored"));
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement-evidence");
        assert_eq!(directory_entry_names(dir.path()), vec!["published.db"]);
    }
}
