//! Safe, one-time migration from the prior current-app database filename.

use crate::database::{Database, SCHEMA_VERSION};
use crate::error::AppError;
use crate::product_identity::{
    DATABASE_FILE, DATABASE_IDENTITY_ARCHIVE_FILE, DATABASE_IDENTITY_LEASE_FILE,
    DATABASE_IDENTITY_SOURCE_SCHEMA_VERSION, LEGACY_DATABASE_FILE, LEGACY_DATA_DIR,
};
use rusqlite::backup::{Backup, StepResult};
use rusqlite::{Connection, OpenFlags};
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tempfile::NamedTempFile;

const BACKUP_DEADLINE: Duration = Duration::from_secs(5);
const BACKUP_RETRY_PAUSE: Duration = Duration::from_millis(10);
const BACKUP_PAGES_PER_STEP: i32 = 128;
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

fn legacy_namespace_error(path: &Path) -> AppError {
    AppError::InvalidInput(format!(
        "database identity migration refuses protected legacy namespace {LEGACY_DATA_DIR}: {}",
        path.display()
    ))
}

/// Read-only guard. This must run before mkdir, lease creation, SQLite open, or
/// any authoritative-path early return.
fn reject_legacy_namespace_before_any_io(app_dir: &Path) -> Result<(), AppError> {
    let home = dirs::home_dir().ok_or_else(|| {
        AppError::InvalidInput(
            "cannot resolve home directory for protected .cc-switch guard".to_string(),
        )
    })?;
    let protected = home.join(LEGACY_DATA_DIR);
    let app_lexical = absolute_lexical(app_dir)?;
    let protected_lexical = absolute_lexical(&protected)?;
    if path_starts_with_platform(&app_lexical, &protected_lexical) {
        return Err(legacy_namespace_error(app_dir));
    }

    let app_resolved = canonicalize_deepest_existing(app_dir)?;
    let protected_resolved = canonicalize_deepest_existing(&protected)?;
    if path_starts_with_platform(&app_resolved, &protected_resolved) {
        return Err(legacy_namespace_error(app_dir));
    }

    let local_candidates = database_identity_candidates(app_dir);
    let protected_candidates = database_identity_candidates(&protected);
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
        if path_entry_exists(&protected)? {
            protected_objects.push(FileIdentity::from_path(&protected)?);
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
    path: PathBuf,
    identity: FileIdentity,
    nonce: String,
    _file: File,
}

/// Owns the just-created lease name until initialization is complete. Cleanup
/// is attempted only after the opened handle yielded a stable object identity;
/// if even handle metadata is unavailable, the entry is deliberately retained
/// fail-closed for operator inspection instead of deleting by path.
struct LeaseCreationGuard {
    path: PathBuf,
    file: Option<File>,
    identity: Option<FileIdentity>,
    committed: bool,
}

impl LeaseCreationGuard {
    fn new(path: PathBuf, file: File) -> Self {
        Self {
            path,
            file: Some(file),
            identity: None,
            committed: false,
        }
    }

    fn finish(mut self, nonce: String) -> DatabaseMigrationLease {
        let lease = DatabaseMigrationLease {
            path: self.path.clone(),
            identity: self
                .identity
                .clone()
                .expect("lease identity is established before finish"),
            nonce,
            _file: self
                .file
                .take()
                .expect("lease handle is present before finish"),
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
        if remove_path_if_identity(&self.path, identity).is_ok() {
            if let Some(parent) = self.path.parent() {
                let _ = sync_directory(parent);
            }
        }
    }
}

impl DatabaseMigrationLease {
    fn acquire(app_dir: &Path) -> Result<Self, AppError> {
        let path = app_dir.join(DATABASE_IDENTITY_LEASE_FILE);
        let deadline = Instant::now() + LEASE_WAIT_DEADLINE;
        loop {
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => {
                    let mut creation = LeaseCreationGuard::new(path.clone(), file);
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
                    let lease = creation.finish(nonce);
                    if let Err(error) = sync_directory(app_dir) {
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
}

fn remove_path_if_identity(path: &Path, expected: &FileIdentity) -> Result<(), AppError> {
    let actual = FileIdentity::from_path(path)?;
    if &actual != expected {
        return Err(AppError::Lock(format!(
            "refused to remove replaced database migration lease: {}",
            path.display()
        )));
    }
    std::fs::remove_file(path).map_err(|error| AppError::io(path, error))
}

fn remove_path_if_identity_and_sync(path: &Path, expected: &FileIdentity) -> Result<(), AppError> {
    remove_path_if_identity(path, expected)?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

impl Drop for DatabaseMigrationLease {
    fn drop(&mut self) {
        let owned = (|| -> Result<bool, AppError> {
            let mut file =
                File::open(&self.path).map_err(|error| AppError::io(&self.path, error))?;
            if FileIdentity::from_file(&self.path, &file)? != self.identity {
                return Ok(false);
            }
            let mut metadata = String::new();
            file.read_to_string(&mut metadata)
                .map_err(|error| AppError::io(&self.path, error))?;
            let nonce_field = format!("\"nonce\":\"{}\"", self.nonce);
            Ok(metadata.contains(&nonce_field))
        })()
        .unwrap_or(false);
        if owned && remove_path_if_identity(&self.path, &self.identity).is_ok() {
            if let Some(parent) = self.path.parent() {
                let _ = sync_directory(parent);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SchemaValidationPolicy {
    ExactMigrationSource,
    CurrentAuthoritative,
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

fn create_validated_snapshot(source: &Path, app_dir: &Path) -> Result<NamedTempFile, AppError> {
    let snapshot = tempfile::Builder::new()
        .prefix(".llm-usage-bar-db-snapshot-")
        .tempfile_in(app_dir)
        .map_err(|error| AppError::io(app_dir, error))?;
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

fn publish_snapshot_noclobber(
    snapshot: NamedTempFile,
    target: &Path,
) -> Result<PublishOutcome, AppError> {
    let snapshot_identity = FileIdentity::from_file(snapshot.path(), snapshot.as_file())?;
    snapshot
        .as_file()
        .sync_all()
        .map_err(|error| AppError::io(snapshot.path(), error))?;
    match snapshot.persist_noclobber(target) {
        Ok(file) => {
            let identity = match FileIdentity::from_file(target, &file) {
                Ok(identity) => identity,
                Err(original) => {
                    #[cfg(any(unix, windows))]
                    {
                        return match remove_path_if_identity_and_sync(target, &snapshot_identity) {
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
                return match remove_path_if_identity_and_sync(target, &identity) {
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
        Err(error) if error.error.kind() == ErrorKind::AlreadyExists => {
            drop(error.file);
            Ok(PublishOutcome::Existing)
        }
        Err(error) => {
            let source = error.error;
            drop(error.file);
            Err(AppError::io(target, source))
        }
    }
}

fn rollback_published(files: &[&PublishedFile]) -> Result<(), AppError> {
    let mut failures = Vec::new();
    for published in files {
        match FileIdentity::from_path(&published.path) {
            Ok(identity) if identity == published.identity => {
                if let Err(error) = std::fs::remove_file(&published.path) {
                    failures.push(format!("remove {}: {error}", published.path.display()));
                }
            }
            Ok(_) => failures.push(format!(
                "refused to remove replaced object {}",
                published.path.display()
            )),
            Err(error) => failures.push(format!(
                "verify owned object {}: {error}",
                published.path.display()
            )),
        }
    }
    for parent in files.iter().filter_map(|published| published.path.parent()) {
        if let Err(error) = sync_directory(parent) {
            failures.push(format!(
                "sync rollback directory {}: {error}",
                parent.display()
            ));
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
    published: &[&PublishedFile],
) -> Result<DatabaseIdentityOutcome, AppError> {
    match rollback_published(published) {
        Ok(()) => Err(original),
        Err(rollback) => Err(AppError::Database(format!(
            "{original}; additionally, {rollback}"
        ))),
    }
}

fn prepare_database_identity_impl(
    app_dir: &Path,
    fault: Option<MigrationFaultPoint>,
) -> Result<DatabaseIdentityOutcome, AppError> {
    reject_legacy_namespace_before_any_io(app_dir)?;

    let new_path = app_dir.join(DATABASE_FILE);
    let old_path = app_dir.join(LEGACY_DATABASE_FILE);
    let archive_path = app_dir.join(DATABASE_IDENTITY_ARCHIVE_FILE);

    if !path_entry_exists(app_dir)? {
        // The anti-alias guard above resolves every existing ancestor before
        // this mkdir. Creating the directory lets even a fresh install take
        // the same exclusive lease and state recheck as an upgrade.
        std::fs::create_dir_all(app_dir).map_err(|error| AppError::io(app_dir, error))?;
        reject_legacy_namespace_before_any_io(app_dir)?;
        if let Some(parent) = app_dir.parent() {
            sync_directory(parent)?;
        }
    }
    if !std::fs::metadata(app_dir)
        .map_err(|error| AppError::io(app_dir, error))?
        .is_dir()
    {
        return Err(AppError::InvalidInput(format!(
            "database application path is not a directory: {}",
            app_dir.display()
        )));
    }

    let _lease = DatabaseMigrationLease::acquire(app_dir)?;
    // State may have changed while waiting for another valid owner. Re-run the
    // read-only anti-alias guard before any SQLite open.
    reject_legacy_namespace_before_any_io(app_dir)?;

    if path_entry_exists(&new_path)? {
        // Existing current filenames may be the fixed-v13 crash-window output
        // or any schema this binary supports after startup schema migration.
        validate_existing_database(&new_path, SchemaValidationPolicy::CurrentAuthoritative)?;
        return Ok(DatabaseIdentityOutcome {
            database_path: new_path,
            archived_prior_path: None,
            retained_prior_path: path_entry_exists(&old_path)?.then_some(old_path),
            migrated: false,
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
        });
    }

    if path_entry_exists(&archive_path)? {
        return Err(AppError::Database(format!(
            "refusing to overwrite pre-existing database archive: {}",
            archive_path.display()
        )));
    }

    let old_identity = FileIdentity::from_path(&old_path)?;
    let new_snapshot = create_validated_snapshot(&old_path, app_dir)?;
    // The archive is a second SQLite backup of the validated first snapshot,
    // never a copy-on-name, hard link, or alias of old/new.
    let archive_snapshot = create_validated_snapshot(new_snapshot.path(), app_dir)?;
    if FileIdentity::from_path(new_snapshot.path())?
        == FileIdentity::from_path(archive_snapshot.path())?
    {
        return Err(AppError::Database(
            "new and archive snapshots unexpectedly share a filesystem object".to_string(),
        ));
    }

    let published_new = match publish_snapshot_noclobber(new_snapshot, &new_path)? {
        PublishOutcome::Existing => {
            validate_existing_database(&new_path, SchemaValidationPolicy::CurrentAuthoritative)?;
            return Ok(DatabaseIdentityOutcome {
                database_path: new_path,
                archived_prior_path: None,
                retained_prior_path: Some(old_path),
                migrated: false,
            });
        }
        PublishOutcome::Published(published) => published,
    };
    if let Err(error) = sync_directory(app_dir) {
        return rollback_after(error, &[&published_new]);
    }

    if fault == Some(MigrationFaultPoint::ArchivePublish) {
        return rollback_after(
            AppError::Database("injected archive publication failure".to_string()),
            &[&published_new],
        );
    }

    let published_archive = match publish_snapshot_noclobber(archive_snapshot, &archive_path) {
        Ok(PublishOutcome::Published(published)) => published,
        Ok(PublishOutcome::Existing) => {
            return rollback_after(
                AppError::Database(format!(
                    "refusing to overwrite concurrently-created database archive: {}",
                    archive_path.display()
                )),
                &[&published_new],
            );
        }
        Err(error) => return rollback_after(error, &[&published_new]),
    };
    if published_new.identity == published_archive.identity {
        return rollback_after(
            AppError::Database(
                "published database and archive share a filesystem object".to_string(),
            ),
            &[&published_archive, &published_new],
        );
    }
    if let Err(error) = sync_directory(app_dir) {
        return rollback_after(error, &[&published_archive, &published_new]);
    }

    if fault == Some(MigrationFaultPoint::OldSourceRemove) {
        return rollback_after(
            AppError::Database("injected old source removal failure".to_string()),
            &[&published_archive, &published_new],
        );
    }

    match FileIdentity::from_path(&old_path) {
        Ok(identity) if identity == old_identity => {}
        Ok(_) => {
            return rollback_after(
                AppError::Database(format!(
                    "old database object changed during migration: {}",
                    old_path.display()
                )),
                &[&published_archive, &published_new],
            );
        }
        Err(error) => {
            return rollback_after(error, &[&published_archive, &published_new]);
        }
    }
    if let Err(error) = std::fs::remove_file(&old_path) {
        return rollback_after(
            AppError::io(&old_path, error),
            &[&published_archive, &published_new],
        );
    }

    // Sidecars are evidence until the old main filename has been removed. Once
    // that commit point succeeds they are stale and may be cleaned best-effort.
    for sidecar in [
        database_sidecar_path(&old_path, "-wal"),
        database_sidecar_path(&old_path, "-shm"),
    ] {
        match std::fs::remove_file(&sidecar) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => log::warn!(
                "Failed to remove committed legacy database sidecar {}: {error}",
                sidecar.display()
            ),
        }
    }
    // Both published snapshots were file-synced and the directory was synced
    // before the old-main commit point. A post-delete directory-sync failure
    // cannot be rolled back safely without recreating the old object; treat the
    // completed migration as authoritative and leave a diagnostic. If a crash
    // resurrects the old directory entry, the validated new filename still wins.
    if let Err(error) = sync_directory(app_dir) {
        log::warn!("Failed to sync database directory after committed legacy cleanup: {error}");
    }

    Ok(DatabaseIdentityOutcome {
        database_path: new_path,
        archived_prior_path: Some(archive_path),
        retained_prior_path: None,
        migrated: true,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MigrationFaultPoint {
    ArchivePublish,
    OldSourceRemove,
}

pub(crate) fn prepare_database_identity(
    app_dir: &Path,
) -> Result<DatabaseIdentityOutcome, AppError> {
    prepare_database_identity_impl(app_dir, None)
}

#[cfg(test)]
fn prepare_database_identity_with_test_fault(
    app_dir: &Path,
    fault: MigrationFaultPoint,
) -> Result<DatabaseIdentityOutcome, AppError> {
    prepare_database_identity_impl(app_dir, Some(fault))
}

// Database filename migration tests live beside the private implementation so
// they can exercise the real SQLite backup and no-clobber publication path.

#[cfg(test)]
mod tests {
    use super::{
        backup_and_validate, prepare_database_identity, prepare_database_identity_with_test_fault,
        MigrationFaultPoint,
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
    use std::sync::{Arc, Barrier};

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
        let old = dir.path().join(LEGACY_DATABASE_FILE);
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
        let new = dir.path().join(DATABASE_FILE);
        let archive = dir.path().join(ARCHIVE_FILE);
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
        assert!(!wal_path(&old).exists());
        assert!(!shm_path(&old).exists());
    }

    #[test]
    fn migrates_real_v13_database_and_archives_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = dir.path().join(LEGACY_DATABASE_FILE);
        let new = dir.path().join(DATABASE_FILE);
        let archive = dir.path().join(ARCHIVE_FILE);
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
        let old = dir.path().join(LEGACY_DATABASE_FILE);
        let new = dir.path().join(DATABASE_FILE);
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
    fn fresh_install_returns_new_authoritative_path_without_creating_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let outcome = prepare_database_identity(dir.path()).expect("prepare fresh install");

        assert_eq!(outcome.database_path, dir.path().join(DATABASE_FILE));
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
        assert!(outcomes
            .iter()
            .all(|outcome| outcome.database_path == app_dir.join(DATABASE_FILE)));
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
}
