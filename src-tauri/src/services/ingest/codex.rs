//! Codex 会话日志解析器。
//!
//! 流水线(盯文件 → 读文件 → 去重 → 写库)在 `ingest/mod.rs`,这里只实现
//! Codex 家的「解析」与它独有的两件事:
//!
//! 1. **游标按文件实体键控**(device/inode 哈希),而不是按路径——
//!    rename 之后游标不串,路径复用时不沿用旧事件 ID;
//! 2. **日期分区剪枝**——sessions/YYYY/MM/DD 分区满足四条规则才整体跳过。
//!
//! ## 解析的事件类型
//! - `session_meta` → 提取 session_id
//! - `turn_context` → 提取当前 model
//! - `event_msg` (type=token_count) → 提取累计 token 用量,计算 delta
//!
//! `total_token_usage` 是**从会话开始累计**的值,单次用量靠
//! `compute_delta(&state.prev_total, &cumulative)` 算。所以必须从文件
//! 第一行开始重建 `prev_total`,哪怕这些行早就入过库——跳过水位线
//! 以下的行发生在算完基线**之后**(与 Claude 相反,这就是两家的差别)。

use crate::agent_paths::get_codex_config_dir;
use crate::error::AppError;
use crate::services::ingest::{
    metadata_modified_nanos, occurred_at_secs, sync_with_parser, update_sync_state_for_resource,
    FileCursor, LogFileContext, ParseOutput, ParsedUsage, ProviderWriteProfile, SessionLogParser,
    SessionSyncResult, SyncCursorMap, UsageIdentity,
};
use crate::store::{Database, UsageSyncCursor};
use crate::usage::domain::CODEX_AGENT_MODULE_ID;
use crate::usage::session::{validate_bound_session_agent, ProviderSessionSyncResult};
use crate::usage::system_providers::CHATGPT_SUBSCRIPTION_ID;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(test)]
use crate::services::ingest::{insert_usage_record, sync_file_with_parser};
#[cfg(test)]
use crate::store::lock_conn;
#[cfg(test)]
use crate::usage::domain::TokenSource;

/// 累计 token 用量(跟踪 total_token_usage 字段)
#[derive(Debug, Clone, Default)]
struct CumulativeTokens {
    input: u64,
    cached_input: u64,
    output: u64,
}

/// 单次 API 调用的 token 增量
#[derive(Debug)]
struct DeltaTokens {
    input: u32,
    cached_input: u32,
    output: u32,
}

impl DeltaTokens {
    fn is_zero(&self) -> bool {
        self.input == 0 && self.cached_input == 0 && self.output == 0
    }
}

/// 单文件解析时的运行状态
struct FileParseState {
    session_id: Option<String>,
    current_model: String,
    prev_total: Option<CumulativeTokens>,
    event_index: u32,
}

struct CodexFileIdentity {
    resource_identity: String,
    event_scope: String,
}

/// 从已打开文件的 OS 实体标识生成不含路径的稳定作用域。
///
/// Unix 的 `(device, inode)` 与 Windows 的 `(volume, file index)` 在文件存续期内
/// 不随 rename 变化,且不同文件复用同一路径时不会沿用旧 cursor 或事件 ID。
fn codex_file_identity(
    _path: &Path,
    _file: &fs::File,
    _metadata: &fs::Metadata,
) -> Result<CodexFileIdentity, AppError> {
    let mut hasher = Sha256::new();
    hasher.update(b"codex-jsonl-file-v1\0");

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        hasher.update(b"unix\0");
        hasher.update(_metadata.dev().to_le_bytes());
        hasher.update(_metadata.ino().to_le_bytes());
    }

    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::HANDLE;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };

        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: `_file` owns a live Windows handle for the duration of this
        // call and `information` is a correctly sized writable binding type.
        let succeeded = unsafe {
            GetFileInformationByHandle(
                _file.as_raw_handle() as HANDLE,
                std::ptr::addr_of_mut!(information),
            )
        };
        if succeeded == 0 {
            return Err(AppError::io(_path, std::io::Error::last_os_error()));
        }
        let file_index =
            ((information.nFileIndexHigh as u64) << 32) | information.nFileIndexLow as u64;
        hasher.update(b"windows\0");
        hasher.update(information.dwVolumeSerialNumber.to_le_bytes());
        hasher.update(file_index.to_le_bytes());
    }

    #[cfg(all(not(unix), not(windows)))]
    {
        return Err(AppError::Config(format!(
            "当前平台不支持稳定的 Codex 会话文件标识: {}",
            _path.display()
        )));
    }

    let event_scope = hex::encode(hasher.finalize());
    Ok(CodexFileIdentity {
        resource_identity: format!("codex-jsonl-file-v1:{event_scope}"),
        event_scope,
    })
}

/// 归一化 Codex 模型名
///
/// 处理规则(按顺序):
/// 1. 转小写:`GLM-4.6` → `glm-4.6`
/// 2. 剥离 provider 前缀:`openai/gpt-5.4` → `gpt-5.4`
/// 3. 剥离 ISO 日期后缀:`gpt-5.4-2026-03-05` → `gpt-5.4`
/// 4. 剥离紧凑日期后缀:`gpt-5.4-20260305` → `gpt-5.4`
fn normalize_codex_model(raw: &str) -> String {
    // Step 1: 小写
    let mut name = raw.to_lowercase();

    // Step 2: 剥离 "provider/" 前缀(如 openai/, azure/)
    if let Some(pos) = name.rfind('/') {
        name = name[pos + 1..].to_string();
    }

    // Step 3: 剥离 ISO 日期后缀 -YYYY-MM-DD(正好 11 字符)
    if name.len() > 11 && name.is_char_boundary(name.len() - 11) {
        let suffix = &name[name.len() - 11..];
        if suffix.is_ascii()
            && suffix.as_bytes()[0] == b'-'
            && suffix[1..5].chars().all(|c| c.is_ascii_digit())
            && suffix.as_bytes()[5] == b'-'
            && suffix[6..8].chars().all(|c| c.is_ascii_digit())
            && suffix.as_bytes()[8] == b'-'
            && suffix[9..11].chars().all(|c| c.is_ascii_digit())
        {
            name.truncate(name.len() - 11);
        }
    }

    // Step 4: 剥离紧凑日期后缀 -YYYYMMDD(正好 9 字符)
    if name.len() > 9 {
        let parts: Vec<&str> = name.rsplitn(2, '-').collect();
        if parts.len() == 2 {
            if let Some(suffix) = parts.first() {
                if suffix.len() == 8 && suffix.chars().all(|c| c.is_ascii_digit()) {
                    name = parts[1].to_string();
                }
            }
        }
    }

    name
}

/// 计算两次累计值之间的 delta
fn compute_delta(prev: &Option<CumulativeTokens>, current: &CumulativeTokens) -> DeltaTokens {
    match prev {
        None => DeltaTokens {
            input: current.input as u32,
            cached_input: current.cached_input as u32,
            output: current.output as u32,
        },
        Some(p) => DeltaTokens {
            input: current.input.saturating_sub(p.input) as u32,
            cached_input: current.cached_input.saturating_sub(p.cached_input) as u32,
            output: current.output.saturating_sub(p.output) as u32,
        },
    }
}

/// 从 JSON Value 中提取累计 token 用量
fn parse_cumulative_tokens(total_usage: &serde_json::Value) -> Option<CumulativeTokens> {
    if total_usage.is_null() || !total_usage.is_object() {
        return None;
    }
    Some(CumulativeTokens {
        input: total_usage
            .get("input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cached_input: total_usage
            .get("cached_input_tokens")
            .or_else(|| total_usage.get("cache_read_input_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        output: total_usage
            .get("output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
    })
}

/// 日期分区保留窗口(天):分区日期早于「今天 - 该窗口」时才可能被整体剪枝。
/// 给时区偏移和跨天写入留出余量——今天、昨天、前天的分区永远不会被剪。
const CODEX_PARTITION_FRESH_DAYS: i64 = 2;

/// Codex 会话日志解析器。持有剪枝所需的游标快照与保留窗口。
pub struct CodexParser {
    codex_dir: PathBuf,
    cursors: Vec<UsageSyncCursor>,
    fresh_days: i64,
}

impl CodexParser {
    pub fn new(codex_dir: PathBuf, cursors: Vec<UsageSyncCursor>, fresh_days: i64) -> Self {
        Self {
            codex_dir,
            cursors,
            fresh_days,
        }
    }
}

impl SessionLogParser for CodexParser {
    fn source(&self) -> &'static str {
        "codex"
    }

    fn write_profile(&self) -> ProviderWriteProfile {
        ProviderWriteProfile {
            app_type: "codex",
            legacy_provider_id: "_codex_session",
            provider_type: "codex_session",
            insert_error_prefix: "插入 Codex 会话日志",
            calculator_app: Some("codex"),
            agent_module_id: Some(CODEX_AGENT_MODULE_ID),
            subscription_activity_id: Some(CHATGPT_SUBSCRIPTION_ID),
        }
    }

    fn log_roots(&self, _home: &Path) -> Vec<PathBuf> {
        vec![self.codex_dir.clone()]
    }

    fn is_log_file(&self, path: &Path) -> bool {
        path.extension().and_then(|e| e.to_str()) == Some("jsonl")
    }

    /// 日期分区剪枝。入参只是流水线用来算剪枝计数的全量文件列表;
    /// 真正的剪枝按 sessions 目录结构重新走一遍(实测逻辑原样保留),
    /// 返回应扫描的文件列表。
    fn prune(&self, _files: Vec<PathBuf>) -> Vec<PathBuf> {
        let by_path: HashMap<&str, &UsageSyncCursor> = self
            .cursors
            .iter()
            .filter_map(|cursor| cursor.resource_path.as_deref().map(|path| (path, cursor)))
            .collect();
        collect_codex_session_files_with_window(&self.codex_dir, &by_path, self.fresh_days).0
    }

    fn parse(&self, ctx: &LogFileContext<'_>) -> Result<ParseOutput, AppError> {
        parse_codex_log_file(ctx)
    }

    /// Codex 游标按文件实体键控:新 cursor 以实体身份为 key,
    /// 首次升级时从同路径的旧 cursor 提升。
    fn resolve_cursor(
        &self,
        path: &Path,
        file: &fs::File,
        metadata: &fs::Metadata,
        cursors: &SyncCursorMap,
        cursor_details: &HashMap<String, UsageSyncCursor>,
    ) -> Result<FileCursor, AppError> {
        let file_path_str = path.to_string_lossy().to_string();
        let file_identity = codex_file_identity(path, file, metadata)?;
        let cursor_key = file_identity.resource_identity.clone();
        let file_size = metadata.len().min(i64::MAX as u64) as i64;

        let resource_cursor = cursor_details.get(&cursor_key);
        let legacy_path_cursor = if resource_cursor.is_none() {
            cursor_details.get(&file_path_str).filter(|cursor| {
                cursor
                    .resource_identity
                    .as_deref()
                    .is_none_or(|identity| identity == file_identity.resource_identity.as_str())
            })
        } else {
            None
        };
        let legacy_cursor_key = legacy_path_cursor
            .as_ref()
            .map(|cursor| cursor.cursor_key.clone());
        let (last_modified, last_offset) = cursors
            .get(&cursor_key)
            .or_else(|| {
                legacy_cursor_key
                    .as_deref()
                    .and_then(|key| cursors.get(key))
            })
            .copied()
            .unwrap_or((0, 0));

        let needs_update = resource_cursor.is_none_or(|cursor| {
            cursor.resource_path.as_deref() != Some(file_path_str.as_str())
                || cursor.resource_identity.as_deref()
                    != Some(file_identity.resource_identity.as_str())
        });

        Ok(FileCursor {
            cursor_key,
            resource_identity: Some(file_identity.resource_identity),
            legacy_cursor_key,
            last_modified,
            last_offset,
            size_bytes: file_size,
            needs_update,
        })
    }

    /// 文件未变化时:路径键旧游标 → 实体键的提升,以及 rename 后的路径刷新。
    fn on_unchanged(
        &self,
        db: &Database,
        path: &Path,
        file_modified: i64,
        metadata: &fs::Metadata,
        cursor: &FileCursor,
    ) -> Result<(), AppError> {
        if !cursor.needs_update {
            return Ok(());
        }
        let file_path_str = path.to_string_lossy().to_string();
        let file_size = metadata.len().min(i64::MAX as u64) as i64;
        update_sync_state_for_resource(
            db,
            self.source(),
            &cursor.cursor_key,
            &file_path_str,
            cursor.resource_identity.as_deref(),
            cursor.legacy_cursor_key.as_deref(),
            cursor.last_modified.max(file_modified),
            file_size,
            cursor.last_offset,
            None,
        )
    }

    fn record_file_error(&self, path: &Path, error: &AppError, errors: &mut Vec<String>) {
        let msg = format!("Codex 会话文件解析失败 {}: {error}", path.display());
        log::warn!("[CODEX-SYNC] {msg}");
        errors.push(msg);
    }

    fn log_insert_failure(&self, record: &ParsedUsage, error: &AppError) -> Option<String> {
        log::warn!(
            "[CODEX-SYNC] 插入失败 ({}): {error}",
            record.identity.log_label
        );
        None
    }

    fn log_summary(&self, result: &SessionSyncResult) {
        log::info!(
            "[CODEX-SYNC] 同步完成: 导入 {} 条, 跳过 {} 条, 扫描 {} 个文件, 剪枝 {} 个文件",
            result.imported,
            result.skipped,
            result.files_scanned,
            result.files_pruned
        );
    }
}

/// 同步 Codex 使用数据(从 JSONL 会话日志)
pub fn sync_codex_usage(db: &Database) -> Result<SessionSyncResult, AppError> {
    sync_codex_usage_impl(db, None)
}

/// Provider-aware Codex import. The binding check happens before any file scan,
/// and bound-path insertion errors abort the file before its offset is stored.
pub fn sync_codex_usage_bound(
    db: &Database,
    provider_id: &str,
) -> Result<ProviderSessionSyncResult, AppError> {
    let legacy = db.with_bound_usage_source("codex", provider_id, || {
        validate_bound_session_agent(db, "codex", provider_id)?;
        sync_codex_usage_impl(db, Some(provider_id))
    })?;
    let Some(legacy) = legacy else {
        return Ok(ProviderSessionSyncResult {
            warnings: vec!["no usage source binding for codex".to_string()],
            ..ProviderSessionSyncResult::default()
        });
    };
    Ok(ProviderSessionSyncResult {
        imported: legacy.imported,
        skipped: legacy.skipped,
        files_scanned: legacy.files_scanned,
        errors: legacy.errors,
        warnings: vec![],
    })
}

fn sync_codex_usage_impl(
    db: &Database,
    bound_provider_id: Option<&str>,
) -> Result<SessionSyncResult, AppError> {
    let codex_dir = get_codex_config_dir();
    sync_codex_usage_impl_with_window(
        db,
        bound_provider_id,
        &codex_dir,
        CODEX_PARTITION_FRESH_DAYS,
    )
}

/// `fresh_days` 可注入的实现本体:测试用极大窗口等价关闭剪枝,对照改动前行为。
fn sync_codex_usage_impl_with_window(
    db: &Database,
    bound_provider_id: Option<&str>,
    codex_dir: &Path,
    fresh_days: i64,
) -> Result<SessionSyncResult, AppError> {
    // 先预载游标,再收集文件:分区剪枝按 resource_path 索引游标,
    // 只需 read_dir 的文件名即可判断,不必为每个文件打开并计算实体标识。
    let cursors = db.list_usage_sync_cursors("codex")?;
    let parser = CodexParser::new(codex_dir.to_path_buf(), cursors, fresh_days);
    sync_with_parser(db, &parser, bound_provider_id)
}

/// 收集所有 Codex 会话 JSONL 文件;满足剪枝条件的旧日期分区被整体跳过。
/// 返回 (文件列表, 被剪掉的旧分区文件数)。
fn collect_codex_session_files_with_window(
    codex_dir: &Path,
    by_path: &HashMap<&str, &UsageSyncCursor>,
    fresh_days: i64,
) -> (Vec<PathBuf>, u32) {
    let mut files = Vec::new();
    let mut files_pruned: u32 = 0;

    // 1. 扫描 sessions/YYYY/MM/DD/*.jsonl(日期分区目录)
    let sessions_dir = codex_dir.join("sessions");
    if sessions_dir.is_dir() {
        collect_sessions_files(
            &sessions_dir,
            &mut files,
            &mut files_pruned,
            by_path,
            fresh_days,
        );
    }

    // 2. 扫描 archived_sessions/*.jsonl(扁平归档目录,不参与剪枝)
    let archived_dir = codex_dir.join("archived_sessions");
    if archived_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&archived_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                    files.push(path);
                }
            }
        }
    }

    (files, files_pruned)
}

/// 解析分区目录名:必须是纯 ASCII 数字且长度正好 `digits` 位。
fn parse_partition_component(name: &str, digits: usize) -> Option<u32> {
    if name.len() != digits || !name.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    name.parse::<u32>().ok()
}

/// sessions/ 顶层:年份目录必须是 4 位数字;其它目录沿用通用递归,不参与剪枝。
fn collect_sessions_files(
    sessions_dir: &Path,
    files: &mut Vec<PathBuf>,
    files_pruned: &mut u32,
    by_path: &HashMap<&str, &UsageSyncCursor>,
    fresh_days: i64,
) {
    let Ok(entries) = fs::read_dir(sessions_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                files.push(path);
            }
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        match parse_partition_component(&name, 4) {
            // 非年份目录(如 sessions/tmp/):沿用通用递归
            None => collect_jsonl_recursive(&path, files, 1, 3),
            Some(year) => collect_month_dirs(&path, year, files, files_pruned, by_path, fresh_days),
        }
    }
}

/// sessions/YYYY/ 层:月份目录必须是 2 位数字。
fn collect_month_dirs(
    year_dir: &Path,
    year: u32,
    files: &mut Vec<PathBuf>,
    files_pruned: &mut u32,
    by_path: &HashMap<&str, &UsageSyncCursor>,
    fresh_days: i64,
) {
    let Ok(entries) = fs::read_dir(year_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                files.push(path);
            }
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        match parse_partition_component(&name, 2) {
            None => collect_jsonl_recursive(&path, files, 2, 3),
            Some(month) => {
                collect_day_dirs(&path, year, month, files, files_pruned, by_path, fresh_days);
            }
        }
    }
}

/// sessions/YYYY/MM/ 层:识别合法日期分区 sessions/YYYY/MM/DD/,尝试整体剪枝。
fn collect_day_dirs(
    month_dir: &Path,
    year: u32,
    month: u32,
    files: &mut Vec<PathBuf>,
    files_pruned: &mut u32,
    by_path: &HashMap<&str, &UsageSyncCursor>,
    fresh_days: i64,
) {
    let Ok(entries) = fs::read_dir(month_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                files.push(path);
            }
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let date = match parse_partition_component(&name, 2)
            .and_then(|day| chrono::NaiveDate::from_ymd_opt(year as i32, month, day))
        {
            // 目录名不是合法日历日期(如 02/30):沿用通用递归,不参与剪枝
            None => {
                collect_jsonl_recursive(&path, files, 3, 3);
                continue;
            }
            Some(date) => date,
        };
        match try_prune_codex_partition(&path, date, by_path, fresh_days) {
            Some(pruned) => *files_pruned += pruned,
            None => collect_jsonl_recursive(&path, files, 3, 3),
        }
    }
}

/// 判断一个 sessions/YYYY/MM/DD 分区能否整体跳过,能则返回被剪掉的 .jsonl 文件数。
///
/// 四条规则必须同时满足:
///   (a) 分区日期早于「今天(本地时区)- 保留窗口」;
///   (b) 分区下每个 .jsonl 都在游标表中有记录且 modified_at_ns > 0(确实成功同步过);
///   (c) 每个 .jsonl 的当前 mtime 不晚于它自己游标里的 modified_at_ns
///       (与 `sync_file_with_parser` 的跳过条件 `file_modified <=
///       last_modified` 逐字对齐:只有「逐文件扫描也会全部跳过」的分区才配被剪);
///   (d) 分区目录 mtime 不晚于分区内所有游标里最大的 modified_at_ns
///       (目录 mtime 在文件新增/删除时会变,兜住「老分区里多了个新文件」)。
///
/// 规则 (c) 不可省:`codex resume` 会往原 rollout 文件里继续 append,而 append
/// 不改父目录 mtime,只靠 (d) 会让「几天前的分区里被续写的会话」永久不再同步。
/// 实测 1296 个会话文件里有 4 个最后一条记录晚于分区日期 2 天以上,最长 +63 天。
///
/// 任何一条不满足都返回 None,分区照常全扫。判断全程只 read_dir / stat,
/// 不打开任何会话文件(实测 1296 个文件纯 stat 约 5ms,逐个 open 约 250ms)。
fn try_prune_codex_partition(
    partition_dir: &Path,
    partition_date: chrono::NaiveDate,
    by_path: &HashMap<&str, &UsageSyncCursor>,
    fresh_days: i64,
) -> Option<u32> {
    // (a) 日期窗口;窗口极大时 duration/cutoff 溢出,等价于关闭剪枝
    let today = chrono::Local::now().date_naive();
    let cutoff = chrono::Duration::try_days(fresh_days)
        .and_then(|window| today.checked_sub_signed(window))?;
    if partition_date >= cutoff {
        return None;
    }

    // (b) 每个 .jsonl 都有游标且 modified_at_ns > 0
    let entries = fs::read_dir(partition_dir).ok()?;
    let mut pruned: u32 = 0;
    let mut max_cursor_modified: i64 = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let path_str = path.to_string_lossy();
        let cursor = by_path.get(path_str.as_ref())?;
        if cursor.modified_at_ns <= 0 {
            return None;
        }

        // (c) 文件自身 mtime 不晚于它的游标:read_dir 手上就有 DirEntry,
        // 这里只多一次 stat,不打开文件。
        let file_modified = metadata_modified_nanos(&entry.metadata().ok()?);
        if file_modified > cursor.modified_at_ns {
            return None;
        }

        max_cursor_modified = max_cursor_modified.max(cursor.modified_at_ns);
        pruned += 1;
    }
    if pruned == 0 {
        // 空分区没有文件可剪
        return Some(0);
    }

    // (d) 目录 mtime 不晚于分区内最大游标时间
    let dir_modified = fs::metadata(partition_dir)
        .ok()
        .map(|metadata| metadata_modified_nanos(&metadata))?;
    if dir_modified > max_cursor_modified {
        return None;
    }

    Some(pruned)
}

/// 递归扫描目录下的 .jsonl 文件(限制最大深度)
fn collect_jsonl_recursive(dir: &Path, files: &mut Vec<PathBuf>, depth: u32, max_depth: u32) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && depth < max_depth {
            collect_jsonl_recursive(&path, files, depth + 1, max_depth);
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
}

/// 把整个文件解析成归一化用量记录,返回的记录已排除水位线以下的行。
///
/// 跳过发生在**重建累计基线之后**(Codex 的位置):即使水位线以下的行
/// 早已入过库,也仍要先解析它们、推进 `prev_total` 与 `event_index`,
/// 这样水位线之上的第一条记录的 delta 才是「当前累计 - 上一行累计」。
fn parse_codex_log_file(ctx: &LogFileContext<'_>) -> Result<ParseOutput, AppError> {
    let file_identity = codex_file_identity(ctx.path, ctx.file, ctx.metadata)?;
    let mut state = FileParseState {
        session_id: None,
        current_model: "unknown".to_string(),
        prev_total: None,
        event_index: 0,
    };

    let mut records = Vec::new();
    let mut line_offset: i64 = 0;

    for line in ctx.content.lines() {
        line_offset += 1;

        if line.trim().is_empty() {
            continue;
        }

        // 快速过滤:在 JSON 反序列化前跳过无关行
        let is_event_msg = line.contains("\"event_msg\"");
        let is_turn_context = line.contains("\"turn_context\"");
        let is_session_meta = line.contains("\"session_meta\"");

        if !is_event_msg && !is_turn_context && !is_session_meta {
            continue;
        }
        if is_event_msg && !line.contains("\"token_count\"") {
            continue;
        }

        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let event_type = match value.get("type").and_then(|t| t.as_str()) {
            Some(t) => t,
            None => continue,
        };

        match event_type {
            "session_meta" if state.session_id.is_none() => {
                let payload = value.get("payload");
                state.session_id = payload
                    .and_then(|p| {
                        p.get("session_id")
                            .or_else(|| p.get("sessionId"))
                            .or_else(|| p.get("id"))
                    })
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
            }
            "turn_context" => {
                if let Some(payload) = value.get("payload") {
                    // model 可能在 payload.model 或 payload.info.model
                    if let Some(model) = payload
                        .get("model")
                        .or_else(|| payload.get("info").and_then(|info| info.get("model")))
                        .and_then(|v| v.as_str())
                    {
                        state.current_model = normalize_codex_model(model);
                    }
                }
            }
            "event_msg" => {
                let payload = match value.get("payload") {
                    Some(p) => p,
                    None => continue,
                };

                // 只处理 token_count 类型
                if payload.get("type").and_then(|t| t.as_str()) != Some("token_count") {
                    continue;
                }

                let info = match payload.get("info") {
                    Some(i) if !i.is_null() => i,
                    _ => continue, // 跳过 info 为 null 的首个事件
                };

                // 提取模型(token_count 事件也可能携带 model)
                if let Some(model) = info
                    .get("model")
                    .or_else(|| info.get("model_name"))
                    .or_else(|| payload.get("model"))
                    .and_then(|v| v.as_str())
                {
                    state.current_model = normalize_codex_model(model);
                }

                // 优先用 total_token_usage(累计值),fallback 到 last_token_usage(增量值)
                let (cumulative, is_total) = if let Some(total) = info.get("total_token_usage") {
                    (parse_cumulative_tokens(total), true)
                } else if let Some(last) = info.get("last_token_usage") {
                    (parse_cumulative_tokens(last), false)
                } else {
                    continue;
                };

                let cumulative = match cumulative {
                    Some(c) => c,
                    None => continue,
                };

                let delta = if is_total {
                    // 累计值模式:计算与上次的 delta
                    let d = compute_delta(&state.prev_total, &cumulative);
                    state.prev_total = Some(cumulative);
                    d
                } else {
                    // 增量值模式:直接使用 last_token_usage 的值
                    DeltaTokens {
                        input: cumulative.input as u32,
                        cached_input: cumulative.cached_input as u32,
                        output: cumulative.output as u32,
                    }
                };

                // 钳制:cached 不应超过 input(防护异常数据)
                let delta = DeltaTokens {
                    cached_input: delta.cached_input.min(delta.input),
                    ..delta
                };

                if delta.is_zero() {
                    continue; // 跳过 task 边界的零 delta 事件
                }

                state.event_index += 1;

                // 跳过已处理的行(但仍需解析以恢复状态)
                if line_offset <= ctx.last_line_offset {
                    continue;
                }

                // 生成唯一 request_id
                let request_scope = state
                    .session_id
                    .as_deref()
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("file-{}", file_identity.event_scope));

                // 提取时间戳
                let timestamp = value
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let request_id = format!("codex_session:{request_scope}:{}", state.event_index);
                records.push(ParsedUsage {
                    identity: UsageIdentity {
                        request_id: request_id.clone(),
                        event_id: Some(format!("codex-session:{request_id}")),
                        upstream_correlation_id: None,
                        message_id: None,
                        log_label: request_id,
                    },
                    model: state.current_model.clone(),
                    input_tokens: delta.input,
                    output_tokens: delta.output,
                    cache_read_tokens: delta.cached_input,
                    cache_creation_tokens: 0,
                    occurred_at: occurred_at_secs(timestamp.as_deref()),
                    session_id: state.session_id.clone(),
                    line_offset,
                    upstream_total_cost: None,
                });
            }
            _ => {}
        }
    }

    Ok(ParseOutput {
        records,
        next_state: None,
    })
}

#[cfg(test)]
fn sync_single_codex_file(
    db: &Database,
    file_path: &Path,
    bound_provider_id: Option<&str>,
) -> Result<(u32, u32), AppError> {
    let cursor_list = db.list_usage_sync_cursors("codex")?;
    let mut sync_cursors: SyncCursorMap = cursor_list
        .iter()
        .map(|cursor| {
            (
                cursor.cursor_key.clone(),
                (cursor.modified_at_ns, cursor.line_offset),
            )
        })
        .collect();
    let cursor_details: HashMap<String, UsageSyncCursor> = cursor_list
        .into_iter()
        .map(|cursor| (cursor.cursor_key.clone(), cursor))
        .collect();
    let parser = CodexParser::new(
        PathBuf::from("/unused"),
        Vec::new(),
        CODEX_PARTITION_FRESH_DAYS,
    );
    let profile = parser.write_profile();
    let mut errors = Vec::new();
    sync_file_with_parser(
        db,
        &parser,
        &profile,
        file_path,
        bound_provider_id,
        &mut sync_cursors,
        &cursor_details,
        &mut errors,
    )
}

/// 旧签名的测试适配器:把 DeltaTokens 转成 ParsedUsage 后走统一写库。
/// 生产路径已由流水线的 `insert_usage_record` 接管;这个函数只服务于
/// 保留原断言不变的旧测试。
#[cfg(test)]
fn insert_codex_session_entry(
    db: &Database,
    request_id: &str,
    delta: &DeltaTokens,
    model: &str,
    session_id: Option<&str>,
    timestamp: Option<&str>,
) -> Result<bool, AppError> {
    let parser = CodexParser::new(
        PathBuf::from("/unused"),
        Vec::new(),
        CODEX_PARTITION_FRESH_DAYS,
    );
    let identity_request_id = "codex_session:test-scope:1".to_string();
    insert_usage_record(
        db,
        &parser.write_profile(),
        &ParsedUsage {
            identity: UsageIdentity {
                request_id: identity_request_id.clone(),
                event_id: Some(format!("codex-session:{identity_request_id}")),
                upstream_correlation_id: None,
                message_id: None,
                log_label: identity_request_id,
            },
            model: model.to_string(),
            input_tokens: delta.input,
            output_tokens: delta.output,
            cache_read_tokens: delta.cached_input,
            cache_creation_tokens: 0,
            occurred_at: occurred_at_secs(timestamp),
            session_id: session_id.map(str::to_string),
            line_offset: 0,
            upstream_total_cost: None,
        },
        request_id,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    fn save_session_provider(db: &Database, provider_id: &str) -> Result<(), AppError> {
        db.save_usage_provider(&crate::usage::domain::UsageProviderInput {
            id: provider_id.to_string(),
            name: format!("{provider_id} session provider"),
            billing_kind: crate::usage::domain::BillingKind::Subscription,
            product_group_id: "codex".to_string(),
            token_sources: vec![TokenSource::SessionLog],
            session_source_bindings: None,
            quota_source: None,
            quota_interval_seconds: Some(300),
            route_app_type: None,
            route_config: None,
            quota_config: None,
            enabled: true,
        })?;
        db.set_usage_source_binding("codex", provider_id)?;
        Ok(())
    }

    fn save_enabled_agent_binding(
        db: &Database,
        agent_module_id: &str,
        provider_id: &str,
    ) -> Result<(), AppError> {
        let binding =
            db.save_agent_provider_binding(&crate::usage::domain::AgentProviderBindingInput {
                id: None,
                agent_module_id: agent_module_id.to_string(),
                provider_id: provider_id.to_string(),
                enabled: true,
            })?;
        assert!(binding.enabled);
        Ok(())
    }

    fn codex_token_count_log(session_id: Option<&str>, input_tokens: u64) -> String {
        let session_meta = session_id.map_or_else(String::new, |session_id| {
            format!("{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{session_id}\"}}}}\n")
        });
        format!(
            "{session_meta}{{\"type\":\"turn_context\",\"payload\":{{\"model\":\"gpt-5.4\"}}}}\n\
             {{\"timestamp\":\"2026-07-14T00:00:00Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"total_token_usage\":{{\"input_tokens\":{input_tokens},\"cached_input_tokens\":1,\"output_tokens\":2}}}}}}}}\n"
        )
    }

    #[test]
    fn test_delta_first_event() {
        let prev = None;
        let current = CumulativeTokens {
            input: 17934,
            cached_input: 9600,
            output: 454,
        };
        let delta = compute_delta(&prev, &current);
        assert_eq!(delta.input, 17934);
        assert_eq!(delta.cached_input, 9600);
        assert_eq!(delta.output, 454);
        assert!(!delta.is_zero());
    }

    #[test]
    fn test_delta_subsequent_event() {
        let prev = Some(CumulativeTokens {
            input: 17934,
            cached_input: 9600,
            output: 454,
        });
        let current = CumulativeTokens {
            input: 36722,
            cached_input: 27904,
            output: 804,
        };
        let delta = compute_delta(&prev, &current);
        assert_eq!(delta.input, 36722 - 17934);
        assert_eq!(delta.cached_input, 27904 - 9600);
        assert_eq!(delta.output, 804 - 454);
    }

    #[test]
    fn test_delta_zero_at_task_boundary() {
        let prev = Some(CumulativeTokens {
            input: 58346,
            cached_input: 46976,
            output: 1045,
        });
        // task 边界:相同的累计值
        let current = CumulativeTokens {
            input: 58346,
            cached_input: 46976,
            output: 1045,
        };
        let delta = compute_delta(&prev, &current);
        assert!(delta.is_zero());
    }

    #[test]
    fn test_delta_saturating_sub() {
        // 异常情况:当前值小于前值(不应发生,但需防护)
        let prev = Some(CumulativeTokens {
            input: 100,
            cached_input: 50,
            output: 30,
        });
        let current = CumulativeTokens {
            input: 80,
            cached_input: 40,
            output: 20,
        };
        let delta = compute_delta(&prev, &current);
        assert_eq!(delta.input, 0);
        assert_eq!(delta.cached_input, 0);
        assert_eq!(delta.output, 0);
        assert!(delta.is_zero());
    }

    #[test]
    fn test_parse_cumulative_tokens_valid() {
        let json: serde_json::Value = serde_json::json!({
            "input_tokens": 17934,
            "cached_input_tokens": 9600,
            "output_tokens": 454,
            "reasoning_output_tokens": 233,
            "total_tokens": 18388
        });
        let tokens = parse_cumulative_tokens(&json).unwrap();
        assert_eq!(tokens.input, 17934);
        assert_eq!(tokens.cached_input, 9600);
        assert_eq!(tokens.output, 454);
    }

    #[test]
    fn test_parse_cumulative_tokens_null() {
        let json = serde_json::Value::Null;
        assert!(parse_cumulative_tokens(&json).is_none());
    }

    #[test]
    fn test_parse_cumulative_tokens_alt_field_names() {
        // 某些版本可能使用 cache_read_input_tokens 而非 cached_input_tokens
        let json: serde_json::Value = serde_json::json!({
            "input_tokens": 1000,
            "cache_read_input_tokens": 500,
            "output_tokens": 200
        });
        let tokens = parse_cumulative_tokens(&json).unwrap();
        assert_eq!(tokens.cached_input, 500);
    }

    #[test]
    fn test_collect_codex_session_files_nonexistent() {
        let (files, files_pruned) = collect_codex_session_files_with_window(
            Path::new("/nonexistent/path"),
            &HashMap::new(),
            CODEX_PARTITION_FRESH_DAYS,
        );
        assert!(files.is_empty());
        assert_eq!(files_pruned, 0);
    }

    // ── 日期分区剪枝 ──

    /// 构造按 resource_path 索引的游标表(剪枝判断只看这张表)
    fn by_path_from_cursors(cursors: &[UsageSyncCursor]) -> HashMap<&str, &UsageSyncCursor> {
        cursors
            .iter()
            .filter_map(|cursor| cursor.resource_path.as_deref().map(|path| (path, cursor)))
            .collect()
    }

    /// 造一个指向给定文件的游标记录(资源身份留空,模拟旧的路径键游标)
    fn cursor_for_path(path: &Path, modified_at_ns: i64) -> UsageSyncCursor {
        let path_str = path.to_string_lossy().to_string();
        UsageSyncCursor {
            source: "codex".to_string(),
            cursor_key: path_str.clone(),
            resource_path: Some(path_str),
            resource_identity: None,
            modified_at_ns,
            size_bytes: 0,
            byte_offset: 0,
            line_offset: 0,
            parser_state_json: None,
            last_success_at: 1,
        }
    }

    /// 返回 sessions/YYYY/MM/DD 分区目录路径
    fn partition_dir(root: &Path, date: chrono::NaiveDate) -> PathBuf {
        root.join("sessions")
            .join(format!("{:04}", date.year()))
            .join(format!("{:02}", date.month()))
            .join(format!("{:02}", date.day()))
    }

    /// 在分区目录里写一个 .jsonl 文件并返回其路径
    fn write_partition_file(root: &Path, date: chrono::NaiveDate, name: &str) -> PathBuf {
        let dir = partition_dir(root, date);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join(name);
        fs::write(&file, "{}\n").unwrap();
        file
    }

    /// 距今 N 天的日期(本地时区)
    fn date_days_ago(days: i64) -> chrono::NaiveDate {
        chrono::Local::now().date_naive() - chrono::Duration::days(days)
    }

    /// 游标时间戳 = 文件 mtime + 1 小时,保证目录 mtime 不晚于最大游标时间
    fn cursor_modified_after_file(path: &Path) -> i64 {
        let metadata = fs::metadata(path).unwrap();
        metadata_modified_nanos(&metadata) + 3_600_000_000_000
    }

    fn dir_mtime_nanos(dir: &Path) -> i64 {
        metadata_modified_nanos(&fs::metadata(dir).unwrap())
    }

    /// 显式设置文件/目录的 mtime(秒级)。
    ///
    /// 剪枝测试要造「目录比游标新」「文件被续写」这些场景。CI(ubuntu)文件系统的
    /// mtime 分辨率可能是秒级,依赖「同一秒内真实操作的先后顺序」的断言测不出来
    /// (macOS 上 APFS 是纳秒级所以本地全绿,ubuntu 上同秒事件判不出先后)。统一
    /// 用显式时间戳,相邻值至少差 2 秒,任何分辨率下判定结果都确定。
    fn set_mtime_seconds(path: &Path, seconds: i64) {
        filetime::set_file_mtime(path, filetime::FileTime::from_unix_time(seconds, 0)).unwrap();
    }

    fn new_temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("llm-usage-bar-{label}-{}", uuid::Uuid::new_v4()))
    }

    fn reset_codex_file_open_count() {
        crate::services::ingest::reset_session_file_open_count();
    }

    fn codex_file_open_count() -> u64 {
        crate::services::ingest::session_file_open_count()
    }

    #[test]
    fn prune_skips_old_partition_when_every_file_has_a_positive_cursor() {
        let tmp = new_temp_dir("codex-prune-old");
        let date = date_days_ago(30);
        let first = write_partition_file(&tmp, date, "first.jsonl");
        let second = write_partition_file(&tmp, date, "second.jsonl");
        let cursors = vec![
            cursor_for_path(&first, cursor_modified_after_file(&first)),
            cursor_for_path(&second, cursor_modified_after_file(&second)),
        ];
        let by_path = by_path_from_cursors(&cursors);

        let (files, files_pruned) =
            collect_codex_session_files_with_window(&tmp, &by_path, CODEX_PARTITION_FRESH_DAYS);
        assert_eq!(files_pruned, 2);
        assert!(
            files.is_empty(),
            "被剪掉的分区文件不应出现在 files 里: {files:?}"
        );
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn prune_requires_every_partition_file_to_have_a_cursor() {
        let tmp = new_temp_dir("codex-prune-missing-cursor");
        let date = date_days_ago(30);
        let first = write_partition_file(&tmp, date, "first.jsonl");
        let second = write_partition_file(&tmp, date, "second.jsonl");
        // second.jsonl 在 by_path 里查不到 → 整个分区不剪
        let cursors = vec![cursor_for_path(&first, cursor_modified_after_file(&first))];
        let by_path = by_path_from_cursors(&cursors);

        let (files, files_pruned) =
            collect_codex_session_files_with_window(&tmp, &by_path, CODEX_PARTITION_FRESH_DAYS);
        assert_eq!(files_pruned, 0, "缺失游标的文件必须照常扫描");
        assert_eq!(files.len(), 2);
        assert!(files.contains(&first) && files.contains(&second));
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn prune_requires_positive_cursor_timestamps() {
        let tmp = new_temp_dir("codex-prune-zero-cursor");
        let date = date_days_ago(30);
        let first = write_partition_file(&tmp, date, "first.jsonl");
        let second = write_partition_file(&tmp, date, "second.jsonl");
        // modified_at_ns == 0 表示从未成功同步过 → 不剪
        let cursors = vec![cursor_for_path(&first, 0), cursor_for_path(&second, 0)];
        let by_path = by_path_from_cursors(&cursors);

        let (files, files_pruned) =
            collect_codex_session_files_with_window(&tmp, &by_path, CODEX_PARTITION_FRESH_DAYS);
        assert_eq!(files_pruned, 0);
        assert_eq!(files.len(), 2);
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn prune_requires_partition_dir_mtime_not_newer_than_cursor_times() {
        let tmp = new_temp_dir("codex-prune-dir-mtime");
        let date = date_days_ago(30);
        let first = write_partition_file(&tmp, date, "first.jsonl");
        let second = write_partition_file(&tmp, date, "second.jsonl");
        // 游标比文件 mtime 晚 2 秒(规则 (c) 通过),再把目录 mtime 显式抬到比最大
        // 游标还新(规则 (d) 单独生效)。用显式时间戳而非「立刻写个临时文件再删」:
        // 秒级分辨率的文件系统上,同秒内发生的目录触碰显示不出先后(见
        // set_mtime_seconds 的注释)。
        let base = metadata_modified_nanos(&fs::metadata(&first).unwrap()).max(0) / 1_000_000_000;
        let cursors = vec![
            cursor_for_path(&first, (base + 2) * 1_000_000_000),
            cursor_for_path(&second, (base + 2) * 1_000_000_000),
        ];
        let by_path = by_path_from_cursors(&cursors);
        set_mtime_seconds(&partition_dir(&tmp, date), base + 3);

        let (files, files_pruned) =
            collect_codex_session_files_with_window(&tmp, &by_path, CODEX_PARTITION_FRESH_DAYS);
        assert_eq!(files_pruned, 0, "目录 mtime 比最大游标时间新时必须全扫");
        assert_eq!(files.len(), 2);
        fs::remove_dir_all(&tmp).ok();
    }

    /// `codex resume` 会往几天前分区里的原 rollout 文件继续 append。append 不改
    /// 父目录 mtime,所以只有逐文件比对 mtime(规则 (c))才能兜住——否则该分区
    /// 一旦被剪就永久不再同步,续写的用量静默丢失。
    #[test]
    fn prune_rescans_partition_when_a_synced_file_was_appended_to() {
        let tmp = new_temp_dir("codex-prune-resumed-append");
        let date = date_days_ago(30);
        let untouched = write_partition_file(&tmp, date, "untouched.jsonl");
        let resumed = write_partition_file(&tmp, date, "resumed.jsonl");

        // 两个文件都已成功同步:游标 = 各自 mtime + 2 秒(规则 (c) 恰好通过,
        // 目录 mtime 也早于游标,规则 (d) 同样通过)
        let base = metadata_modified_nanos(&fs::metadata(&resumed).unwrap()).max(0) / 1_000_000_000;
        let cursors = vec![
            cursor_for_path(&untouched, (base + 2) * 1_000_000_000),
            cursor_for_path(&resumed, (base + 2) * 1_000_000_000),
        ];
        let by_path = by_path_from_cursors(&cursors);

        // 前置断言:此刻分区确实是可剪的
        let (files, files_pruned) =
            collect_codex_session_files_with_window(&tmp, &by_path, CODEX_PARTITION_FRESH_DAYS);
        assert_eq!(files_pruned, 2, "同步干净的老分区本应被剪");
        assert!(files.is_empty());

        // 模拟 resume:往 resumed.jsonl 续写,游标保持不变
        let dir = partition_dir(&tmp, date);
        let dir_mtime_before = dir_mtime_nanos(&dir);
        let mut handle = fs::OpenOptions::new().append(true).open(&resumed).unwrap();
        std::io::Write::write_all(&mut handle, b"{\"appended\":true}\n").unwrap();
        drop(handle);
        assert_eq!(
            dir_mtime_nanos(&dir),
            dir_mtime_before,
            "append 不应改变父目录 mtime——正因如此规则 (d) 兜不住这一场景"
        );
        // 真实 append 已经发生,但秒级分辨率的文件系统上同秒内的 mtime 变化
        // 测不出来,所以显式把文件 mtime 抬到游标之后,让规则 (c) 的判定确定化。
        set_mtime_seconds(&resumed, base + 4);

        let (files, files_pruned) =
            collect_codex_session_files_with_window(&tmp, &by_path, CODEX_PARTITION_FRESH_DAYS);
        assert_eq!(files_pruned, 0, "分区内有文件被续写时必须整体全扫");
        assert_eq!(files.len(), 2);
        assert!(files.contains(&resumed) && files.contains(&untouched));
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn prune_never_touches_recent_partitions() {
        let tmp = new_temp_dir("codex-prune-recent");
        let mut cursors = Vec::new();
        let mut recent_files = Vec::new();
        // 今天、昨天、前天都在保留窗口内,永不剪
        for days in [0, 1, 2] {
            let date = date_days_ago(days);
            let file = write_partition_file(&tmp, date, "recent.jsonl");
            cursors.push(cursor_for_path(&file, cursor_modified_after_file(&file)));
            recent_files.push(file);
        }
        let by_path = by_path_from_cursors(&cursors);

        let (files, files_pruned) =
            collect_codex_session_files_with_window(&tmp, &by_path, CODEX_PARTITION_FRESH_DAYS);
        assert_eq!(files_pruned, 0, "今天/昨天/前天的分区永不剪枝");
        assert_eq!(files.len(), 3);
        for file in &recent_files {
            assert!(files.contains(file), "{} 必须被扫描", file.display());
        }
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn prune_ignores_non_date_and_invalid_date_directories() {
        let tmp = new_temp_dir("codex-prune-non-date");
        // sessions/tmp/ 不是 4 位年份目录
        let tmp_dir = tmp.join("sessions").join("tmp");
        fs::create_dir_all(&tmp_dir).unwrap();
        let scratch_file = tmp_dir.join("scratch.jsonl");
        fs::write(&scratch_file, "{}\n").unwrap();
        // sessions/2026/02/30 是纯数字但非法日历日期
        let invalid_dir = tmp.join("sessions").join("2026").join("02").join("30");
        fs::create_dir_all(&invalid_dir).unwrap();
        let invalid_file = invalid_dir.join("bad.jsonl");
        fs::write(&invalid_file, "{}\n").unwrap();

        let cursors = vec![
            cursor_for_path(&scratch_file, cursor_modified_after_file(&scratch_file)),
            cursor_for_path(&invalid_file, cursor_modified_after_file(&invalid_file)),
        ];
        let by_path = by_path_from_cursors(&cursors);

        let (files, files_pruned) =
            collect_codex_session_files_with_window(&tmp, &by_path, CODEX_PARTITION_FRESH_DAYS);
        assert_eq!(files_pruned, 0);
        assert_eq!(files.len(), 2, "非日期/非法日期目录必须照常扫描");
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn prune_never_touches_archived_sessions() {
        let tmp = new_temp_dir("codex-prune-archived");
        let archived = tmp.join("archived_sessions");
        fs::create_dir_all(&archived).unwrap();
        let file = archived.join("old-session.jsonl");
        fs::write(&file, "{}\n").unwrap();
        let cursors = vec![cursor_for_path(&file, cursor_modified_after_file(&file))];
        let by_path = by_path_from_cursors(&cursors);

        let (files, files_pruned) =
            collect_codex_session_files_with_window(&tmp, &by_path, CODEX_PARTITION_FRESH_DAYS);
        assert_eq!(files_pruned, 0, "archived_sessions 不参与剪枝");
        assert_eq!(files, vec![file]);
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn disabling_the_prune_window_matches_pre_change_collection() {
        let tmp = new_temp_dir("codex-prune-disabled");
        let old_file = write_partition_file(&tmp, date_days_ago(30), "old.jsonl");
        let today_file = write_partition_file(&tmp, date_days_ago(0), "today.jsonl");
        let cursors = vec![
            cursor_for_path(&old_file, cursor_modified_after_file(&old_file)),
            cursor_for_path(&today_file, cursor_modified_after_file(&today_file)),
        ];
        let by_path = by_path_from_cursors(&cursors);

        // 关闭剪枝(fresh_days 极大,cutoff 下溢):收集结果必须与改动前全量收集一致
        let (disabled_files, disabled_pruned) =
            collect_codex_session_files_with_window(&tmp, &by_path, i64::MAX);
        // 空游标表 → 任何分区都不满足规则 (b),等价于旧版的全量收集
        let (baseline_files, baseline_pruned) = collect_codex_session_files_with_window(
            &tmp,
            &HashMap::new(),
            CODEX_PARTITION_FRESH_DAYS,
        );

        assert_eq!(disabled_pruned, 0);
        assert_eq!(baseline_pruned, 0);
        assert_eq!(
            disabled_files, baseline_files,
            "关闭剪枝后的收集结果必须与改动前全量收集逐文件一致"
        );
        assert_eq!(disabled_files.len(), 2);
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn sync_with_pruning_disabled_matches_pre_change_results() -> Result<(), AppError> {
        let db = Database::memory()?;
        let tmp = new_temp_dir("codex-sync-disabled");
        let date = date_days_ago(30);
        let first = write_partition_file(&tmp, date, "first.jsonl");
        let second = write_partition_file(&tmp, date, "second.jsonl");
        // 两个文件都已成功同步(游标时间戳晚于文件 mtime,扫描时必然 skip)
        for file in [&first, &second] {
            let metadata = fs::metadata(file).unwrap();
            db.put_usage_sync_cursor(&UsageSyncCursor {
                source: "codex".to_string(),
                cursor_key: file.to_string_lossy().to_string(),
                resource_path: Some(file.to_string_lossy().to_string()),
                resource_identity: None,
                modified_at_ns: metadata_modified_nanos(&metadata) + 3_600_000_000_000,
                size_bytes: metadata.len() as i64,
                byte_offset: 0,
                line_offset: 0,
                parser_state_json: None,
                last_success_at: 1,
            })?;
        }

        // 关闭剪枝:与改动前一致 —— 两个文件都被扫描并被游标 skip
        let disabled = sync_codex_usage_impl_with_window(&db, None, &tmp, i64::MAX)?;
        assert_eq!(
            (
                disabled.files_scanned,
                disabled.files_pruned,
                disabled.imported,
                disabled.skipped
            ),
            (2, 0, 0, 0),
            "关闭剪枝时必须与改动前逐字段一致"
        );

        // 默认窗口:老分区被整体剪掉,不再打开文件
        let enabled =
            sync_codex_usage_impl_with_window(&db, None, &tmp, CODEX_PARTITION_FRESH_DAYS)?;
        assert_eq!(
            (
                enabled.files_scanned,
                enabled.files_pruned,
                enabled.imported,
                enabled.skipped
            ),
            (0, 2, 0, 0)
        );
        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    #[test]
    fn prune_skips_file_opens_for_fully_synced_old_partitions() -> Result<(), AppError> {
        // 实测:模拟 1294 个文件的会话目录树,对比关闭/开启剪枝时的 File::open 次数。
        let db = Database::memory()?;
        let tmp = new_temp_dir("codex-prune-open-count");
        let mut files: Vec<PathBuf> = Vec::new();
        // 三个 30 天前的老分区 × 400 个文件 = 1200 个
        for (i, days) in [30i64, 31, 32].iter().enumerate() {
            let date = date_days_ago(*days);
            for j in 0..400 {
                files.push(write_partition_file(&tmp, date, &format!("{i}-{j}.jsonl")));
            }
        }
        // 今天分区 94 个文件,永不剪
        let recent = date_days_ago(0);
        for j in 0..94 {
            files.push(write_partition_file(
                &tmp,
                recent,
                &format!("recent-{j}.jsonl"),
            ));
        }
        assert_eq!(files.len(), 1294);

        // 全部文件都成功同步过(游标时间戳晚于文件 mtime,扫描时必然 skip)
        for file in &files {
            let metadata = fs::metadata(file).unwrap();
            db.put_usage_sync_cursor(&UsageSyncCursor {
                source: "codex".to_string(),
                cursor_key: file.to_string_lossy().to_string(),
                resource_path: Some(file.to_string_lossy().to_string()),
                resource_identity: None,
                modified_at_ns: metadata_modified_nanos(&metadata) + 3_600_000_000_000,
                size_bytes: metadata.len() as i64,
                byte_offset: 0,
                line_offset: 0,
                parser_state_json: None,
                last_success_at: 1,
            })?;
        }

        // 关闭剪枝(改动前行为):每个文件都 File::open 一次
        reset_codex_file_open_count();
        let baseline = sync_codex_usage_impl_with_window(&db, None, &tmp, i64::MAX)?;
        assert_eq!(baseline.files_scanned, 1294);
        assert_eq!(baseline.files_pruned, 0);
        assert_eq!(codex_file_open_count(), 1294, "关闭剪枝时必须打开全部文件");

        // 开启剪枝(改动后行为):只有今天分区的 94 个文件被打开
        reset_codex_file_open_count();
        let pruned =
            sync_codex_usage_impl_with_window(&db, None, &tmp, CODEX_PARTITION_FRESH_DAYS)?;
        assert_eq!(pruned.files_scanned, 94);
        assert_eq!(pruned.files_pruned, 1200);
        assert_eq!(
            codex_file_open_count(),
            94,
            "老分区必须连 File::open 都不做"
        );
        assert_eq!(pruned.imported, 0);
        assert_eq!(pruned.skipped, 0);

        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    #[test]
    fn test_insert_codex_session_skips_matching_proxy_log() -> Result<(), AppError> {
        let db = Database::memory()?;
        {
            let conn = lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model, request_model,
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    total_cost_usd, latency_ms, status_code, created_at, data_source
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    "codex-proxy",
                    "openai",
                    "codex",
                    "gpt-5.4",
                    "gpt-5.4",
                    10,
                    2,
                    1,
                    7,
                    "0.01",
                    100,
                    200,
                    1000,
                    "proxy"
                ],
            )?;
        }

        let delta = DeltaTokens {
            input: 10,
            cached_input: 1,
            output: 2,
        };
        let inserted = insert_codex_session_entry(
            &db,
            "codex-session-dup",
            &delta,
            "gpt-5.4",
            Some("session-1"),
            Some("1970-01-01T00:16:45Z"),
        )?;
        assert!(!inserted);

        let conn = lock_conn!(db.conn);
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |row| {
            row.get(0)
        })?;
        assert_eq!(count, 1);

        Ok(())
    }

    #[test]
    fn bound_codex_parser_freezes_codex_agent_when_provider_has_multiple_agent_bindings(
    ) -> Result<(), AppError> {
        let db = Database::memory()?;
        save_session_provider(&db, "shared-session")?;
        save_enabled_agent_binding(&db, "codex", "shared-session")?;
        save_enabled_agent_binding(&db, "claude-code", "shared-session")?;

        let tmp = std::env::temp_dir().join(format!(
            "llm-usage-bar-bound-codex-agent-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("session.jsonl");
        fs::write(&file, codex_token_count_log(Some("codex-session"), 10)).unwrap();

        assert_eq!(
            sync_single_codex_file(&db, &file, Some("shared-session"))?,
            (1, 0)
        );
        let ownership: (String, Option<String>) = {
            let conn = lock_conn!(db.conn);
            conn.query_row(
                "SELECT provider_id, agent_module_id FROM usage_events",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?
        };
        assert_eq!(ownership.0, "shared-session");
        assert_eq!(ownership.1.as_deref(), Some("codex"));

        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    #[test]
    fn bound_codex_parser_imports_without_a_codex_agent_binding() -> Result<(), AppError> {
        let db = Database::memory()?;
        save_session_provider(&db, "wrong-agent-session")?;
        save_enabled_agent_binding(&db, "claude-code", "wrong-agent-session")?;

        let tmp = std::env::temp_dir().join(format!(
            "llm-usage-bar-wrong-codex-agent-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("session.jsonl");
        fs::write(&file, codex_token_count_log(Some("wrong-agent"), 10)).unwrap();
        assert_eq!(
            sync_single_codex_file(&db, &file, Some("wrong-agent-session"))?,
            (1, 0)
        );
        let ownership: (String, Option<String>) = {
            let conn = lock_conn!(db.conn);
            conn.query_row(
                "SELECT provider_id, agent_module_id FROM usage_events",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?
        };
        assert_eq!(ownership.0, "wrong-agent-session");
        assert_eq!(ownership.1.as_deref(), Some("codex"));

        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    #[test]
    fn codex_files_without_session_meta_use_distinct_event_identity() -> Result<(), AppError> {
        let db = Database::memory()?;
        save_session_provider(&db, "codex-session")?;
        save_enabled_agent_binding(&db, "codex", "codex-session")?;

        let tmp = std::env::temp_dir().join(format!(
            "llm-usage-bar-codex-file-scope-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&tmp).unwrap();
        let first = tmp.join("first.jsonl");
        let second = tmp.join("second.jsonl");
        fs::write(&first, codex_token_count_log(None, 10)).unwrap();
        fs::write(&second, codex_token_count_log(None, 10)).unwrap();

        assert_eq!(
            sync_single_codex_file(&db, &first, Some("codex-session"))?,
            (1, 0)
        );
        assert_eq!(
            sync_single_codex_file(&db, &second, Some("codex-session"))?,
            (1, 0),
            "the first token event in a different no-meta file must not collide"
        );
        let (event_count, request_id_count): (i64, i64) = {
            let conn = lock_conn!(db.conn);
            conn.query_row(
                "SELECT COUNT(*), COUNT(DISTINCT request_id) FROM usage_events",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?
        };
        assert_eq!(event_count, 2);
        assert_eq!(request_id_count, 2);

        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    #[test]
    fn codex_file_without_session_meta_keeps_event_identity_after_archive_move(
    ) -> Result<(), AppError> {
        let db = Database::memory()?;
        save_session_provider(&db, "codex-session")?;
        save_enabled_agent_binding(&db, "codex", "codex-session")?;

        let tmp = std::env::temp_dir().join(format!(
            "llm-usage-bar-codex-archive-scope-test-{}",
            uuid::Uuid::new_v4()
        ));
        let sessions_dir = tmp.join("sessions/2026/07/14");
        let archived_dir = tmp.join("archived_sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        fs::create_dir_all(&archived_dir).unwrap();
        let active = sessions_dir.join("no-meta.jsonl");
        let archived = archived_dir.join("no-meta.jsonl");
        fs::write(&active, codex_token_count_log(None, 10)).unwrap();

        assert_eq!(
            sync_single_codex_file(&db, &active, Some("codex-session"))?,
            (1, 0)
        );
        let original_request_id: String = {
            let conn = lock_conn!(db.conn);
            conn.query_row("SELECT request_id FROM usage_events", [], |row| row.get(0))?
        };

        fs::rename(&active, &archived).unwrap();
        let (imported, _skipped) = sync_single_codex_file(&db, &archived, Some("codex-session"))?;
        assert_eq!(
            imported, 0,
            "archiving the same no-meta file must not import a second copy"
        );
        let (event_count, request_id): (i64, String) = {
            let conn = lock_conn!(db.conn);
            conn.query_row(
                "SELECT COUNT(*), MIN(request_id) FROM usage_events",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?
        };
        assert_eq!(event_count, 1);
        assert_eq!(request_id, original_request_id);
        let cursors = db.list_usage_sync_cursors("codex")?;
        assert_eq!(cursors.len(), 1);
        assert_eq!(
            cursors[0].resource_path.as_deref(),
            Some(archived.to_string_lossy().as_ref())
        );
        assert!(cursors[0].resource_identity.is_some());

        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    #[test]
    fn codex_reused_no_meta_path_gets_a_new_file_identity() -> Result<(), AppError> {
        let db = Database::memory()?;
        save_session_provider(&db, "codex-session")?;
        save_enabled_agent_binding(&db, "codex", "codex-session")?;

        let tmp = std::env::temp_dir().join(format!(
            "llm-usage-bar-codex-reused-path-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&tmp).unwrap();
        let current = tmp.join("no-meta.jsonl");
        let retired = tmp.join("retired.jsonl");
        fs::write(&current, codex_token_count_log(None, 10)).unwrap();

        assert_eq!(
            sync_single_codex_file(&db, &current, Some("codex-session"))?,
            (1, 0)
        );
        fs::rename(&current, &retired).unwrap();
        fs::write(&current, codex_token_count_log(None, 20)).unwrap();

        assert_eq!(
            sync_single_codex_file(&db, &current, Some("codex-session"))?,
            (1, 0),
            "a different file reusing the same path must not inherit the old cursor or event IDs"
        );
        let (event_count, request_id_count): (i64, i64) = {
            let conn = lock_conn!(db.conn);
            conn.query_row(
                "SELECT COUNT(*), COUNT(DISTINCT request_id) FROM usage_events",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?
        };
        assert_eq!(event_count, 2);
        assert_eq!(request_id_count, 2);
        let cursors = db.list_usage_sync_cursors("codex")?;
        assert_eq!(cursors.len(), 2);
        assert_ne!(cursors[0].resource_identity, cursors[1].resource_identity);

        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    #[test]
    fn legacy_codex_cursor_promotion_does_not_poison_a_reused_path() -> Result<(), AppError> {
        let db = Database::memory()?;
        save_session_provider(&db, "codex-session")?;
        save_enabled_agent_binding(&db, "codex", "codex-session")?;

        let tmp = std::env::temp_dir().join(format!(
            "llm-usage-bar-codex-legacy-cursor-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&tmp).unwrap();
        let current = tmp.join("no-meta.jsonl");
        let retired = tmp.join("retired.jsonl");
        fs::write(&current, codex_token_count_log(None, 10)).unwrap();

        let current_key = current.to_string_lossy().to_string();
        let metadata = fs::metadata(&current).unwrap();
        db.put_usage_sync_cursor(&crate::store::UsageSyncCursor {
            source: "codex".to_string(),
            cursor_key: current_key.clone(),
            resource_path: Some(current_key.clone()),
            resource_identity: None,
            modified_at_ns: metadata_modified_nanos(&metadata),
            size_bytes: metadata.len() as i64,
            byte_offset: 0,
            line_offset: 2,
            parser_state_json: None,
            last_success_at: 1,
        })?;

        assert_eq!(
            sync_single_codex_file(&db, &current, Some("codex-session"))?,
            (0, 0),
            "an unchanged legacy cursor should promote without replaying old lines"
        );
        fs::rename(&current, &retired).unwrap();
        fs::write(&current, codex_token_count_log(None, 20)).unwrap();

        assert_eq!(
            sync_single_codex_file(&db, &current, Some("codex-session"))?,
            (1, 0),
            "a new file must not inherit the promoted legacy path cursor"
        );
        let cursors = db.list_usage_sync_cursors("codex")?;
        assert_eq!(cursors.len(), 2);
        assert!(
            cursors
                .iter()
                .all(|cursor| cursor.cursor_key.starts_with("codex-jsonl-file-v1:")),
            "promotion must retire the legacy path-keyed cursor"
        );

        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    // ── 模型名归一化测试 ──

    #[test]
    fn test_normalize_codex_model_lowercase() {
        assert_eq!(normalize_codex_model("GLM-4.6"), "glm-4.6");
        assert_eq!(normalize_codex_model("DeepSeek-Chat"), "deepseek-chat");
        assert_eq!(normalize_codex_model("GPT-5.4"), "gpt-5.4");
    }

    #[test]
    fn test_normalize_codex_model_strip_prefix() {
        assert_eq!(normalize_codex_model("openai/gpt-5.4"), "gpt-5.4");
        assert_eq!(
            normalize_codex_model("azure/gpt-5.2-codex"),
            "gpt-5.2-codex"
        );
        assert_eq!(normalize_codex_model("OPENAI/GPT-5.4"), "gpt-5.4");
    }

    #[test]
    fn test_normalize_codex_model_strip_iso_date() {
        assert_eq!(normalize_codex_model("gpt-5.4-2026-03-05"), "gpt-5.4");
        assert_eq!(
            normalize_codex_model("gpt-5.4-pro-2026-03-05"),
            "gpt-5.4-pro"
        );
    }

    #[test]
    fn test_normalize_codex_model_strip_compact_date() {
        assert_eq!(normalize_codex_model("gpt-5.4-20260305"), "gpt-5.4");
        assert_eq!(
            normalize_codex_model("claude-opus-4-6-20260206"),
            "claude-opus-4-6"
        );
    }

    #[test]
    fn test_normalize_codex_model_no_change() {
        assert_eq!(normalize_codex_model("gpt-5.4"), "gpt-5.4");
        assert_eq!(normalize_codex_model("gpt-5.2-codex"), "gpt-5.2-codex");
        assert_eq!(normalize_codex_model("o3"), "o3");
        assert_eq!(normalize_codex_model("deepseek-chat"), "deepseek-chat");
    }

    #[test]
    fn test_normalize_codex_model_combined() {
        // prefix + uppercase + ISO date
        assert_eq!(
            normalize_codex_model("openai/GPT-5.4-2026-03-05"),
            "gpt-5.4"
        );
        // prefix + compact date
        assert_eq!(normalize_codex_model("openai/gpt-5.4-20260305"), "gpt-5.4");
    }

    #[test]
    fn test_cached_clamped_to_input() {
        // cached > input 的异常场景应被 min() 钳制
        let prev = Some(CumulativeTokens {
            input: 100,
            cached_input: 0,
            output: 50,
        });
        let current = CumulativeTokens {
            input: 110,       // delta = 10
            cached_input: 80, // delta = 80(异常:大于 input delta)
            output: 60,
        };
        let delta = compute_delta(&prev, &current);
        // 钳制前:cached_input = 80, input = 10
        assert_eq!(delta.cached_input, 80);
        assert_eq!(delta.input, 10);
        // 实际钳制在调用侧:delta.cached_input.min(delta.input)
        let clamped = delta.cached_input.min(delta.input);
        assert_eq!(clamped, 10);
    }

    // ── T1 新增测试 ──

    /// 水位线不为 0 时,累计基线仍然从文件第一行重建:第三行的增量是
    /// total3 - total2,而不是 total3。这是本任务最容易写错的地方。
    #[test]
    fn codex_waterline_rebuilds_cumulative_baseline_from_line_one() -> Result<(), AppError> {
        let tmp = std::env::temp_dir().join(format!(
            "llm-usage-bar-codex-waterline-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&tmp).unwrap();
        let file_path = tmp.join("session.jsonl");
        let event_line = |input: u64| {
            format!(
                "{{\"timestamp\":\"2026-07-14T00:00:00Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"total_token_usage\":{{\"input_tokens\":{input},\"cached_input_tokens\":{input},\"output_tokens\":{input}}}}}}}}}\n"
            )
        };
        // 三行累计序列:total1=100 < total2=300 < total3=500。
        fs::write(
            &file_path,
            format!("{}{}{}", event_line(100), event_line(300), event_line(500)),
        )
        .unwrap();

        let file = fs::File::open(&file_path).unwrap();
        let metadata = file.metadata().unwrap();
        let content = fs::read_to_string(&file_path).unwrap();
        let ctx = LogFileContext {
            path: &file_path,
            file: &file,
            metadata: &metadata,
            content: &content,
            last_line_offset: 2, // 水位线设在第 2 行
            parser_state: None,
        };

        let parser = CodexParser::new(tmp.clone(), Vec::new(), CODEX_PARTITION_FRESH_DAYS);
        let records = parser.parse(&ctx)?.records;
        assert_eq!(records.len(), 1, "只有第三行在水位线之上");
        let record = &records[0];
        assert_eq!(
            record.input_tokens, 200,
            "第三行的增量必须是 total3 - total2 = 200,而不是 total3 = 500"
        );
        assert_eq!(record.output_tokens, 200);
        assert_eq!(record.cache_read_tokens, 200);
        // UsageIdentity 现在是结构体,序号不再单列:request_id 的格式是
        // codex_session:<scope>:<index>,从最后一段取回 event_index。
        let event_index: u32 = record
            .identity
            .request_id
            .rsplit(':')
            .next()
            .expect("Codex 解析器必须产出 CodexEvent 身份")
            .parse()
            .expect("Codex 解析器必须产出 CodexEvent 身份");
        assert_eq!(
            event_index, 3,
            "水位线以下的事件也要消费序号,第三行必须是第 3 号事件"
        );

        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    /// 文件身份来自 OS 实体(device/inode),不是路径:同一文件改名身份不变,
    /// 不同文件(即使路径相同)身份不同。
    #[test]
    fn codex_file_identity_uses_device_and_inode_not_the_path() -> Result<(), AppError> {
        let tmp = std::env::temp_dir().join(format!(
            "llm-usage-bar-codex-identity-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&tmp).unwrap();
        let first = tmp.join("first.jsonl");
        let second = tmp.join("second.jsonl");
        fs::write(&first, "{\"same\":\"content\"}\n").unwrap();
        fs::write(&second, "{\"same\":\"content\"}\n").unwrap();

        let file1 = fs::File::open(&first).unwrap();
        let metadata1 = file1.metadata().unwrap();
        let identity1 = codex_file_identity(&first, &file1, &metadata1)?;

        // 相同内容的不同文件:身份必须不同(inode 不同)。
        let file2 = fs::File::open(&second).unwrap();
        let metadata2 = file2.metadata().unwrap();
        let identity2 = codex_file_identity(&second, &file2, &metadata2)?;
        assert_ne!(identity1.resource_identity, identity2.resource_identity);

        // 同一文件改名:路径变了,身份不变。
        drop(file1);
        let renamed = tmp.join("renamed.jsonl");
        fs::rename(&first, &renamed).unwrap();
        let file3 = fs::File::open(&renamed).unwrap();
        let metadata3 = file3.metadata().unwrap();
        let identity3 = codex_file_identity(&renamed, &file3, &metadata3)?;
        assert_eq!(identity1.resource_identity, identity3.resource_identity);
        assert_eq!(identity1.event_scope, identity3.event_scope);

        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }
}
