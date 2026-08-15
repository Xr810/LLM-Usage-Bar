//! 会话日志摄入流水线(shared pipeline)。
//!
//! 盯文件变化 → 读文件 → 解析成用量记录 → 去重 → 写库,这五步里只有
//! 「解析」是每家 agent 不同的;本模块拥有其余四步,agent 只提供
//! [`SessionLogParser`] 一个解析器(见各解析器模块)。
//!
//! ## 为什么两个解析器都必须拿到整个文件而不是增量
//!
//! 两家的日志读取都是「整个文件重读」,游标里的 `line_offset` 只是行号
//! 水位线:一家在解析前跳过水位线以下的行;另一家的 `total_token_usage`
//! 是从会话开始累计的值,必须从文件第一行开始重建 `prev_total` 基线,
//! 跳过发生在算完基线之后。只给解析器增量文本会丢掉累计基线,第一条
//! 记录会被当成「从 0 涨到当前累计值」。所以 [`LogFileContext::content`]
//! 是整个文件的内容,增量由 `last_line_offset` 表达,跳过发生在解析器内部。

use crate::error::AppError;
use crate::services::usage_stats::{
    effective_usage_log_filter, find_model_pricing, should_skip_session_insert, DedupKey,
};
use crate::store::{lock_conn, Database, UsageSyncCursor};
use crate::usage::domain::TokenSource;
use crate::usage::ingestion::{LegacyLogInput, UsageIngestionInput, UsageIngestionService};
use crate::usage::metering::calculator::CostCalculator;
use crate::usage::metering::cost_parser::UpstreamCost;
use crate::usage::metering::parser::TokenUsage;
use crate::usage::session::validate_bound_session_agent;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub mod claude;
pub mod codex;
pub mod gemini;
pub mod opencode;
pub mod session_usage;
pub mod session_usage_codex;
pub mod session_usage_gemini;
pub mod session_usage_opencode;

pub(crate) type SyncCursorMap = HashMap<String, (i64, i64)>;

/// 同步结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSyncResult {
    pub imported: u32,
    pub skipped: u32,
    pub files_scanned: u32,
    pub files_pruned: u32,
    pub errors: Vec<String>,
}

/// 数据来源分布
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSourceSummary {
    pub data_source: String,
    pub request_count: u32,
    pub total_cost_usd: String,
}

/// 流水线打开文件之后交给解析器的一切。
///
/// **`content` 是整个文件的内容,不是增量**——理由见模块顶部注释。
/// 增量靠 `last_line_offset` 这条水位线表达,由解析器自己决定在哪一步跳过
/// (有的在解析前跳,有的在重建累计基线之后跳)。
pub struct LogFileContext<'a> {
    pub path: &'a Path,
    /// 已打开的句柄与元数据——实体键控的解析器要靠它算 device/inode 身份。
    pub file: &'a fs::File,
    pub metadata: &'a fs::Metadata,
    pub content: &'a str,
    /// 游标里记的行号水位线;`<=` 它的行已经入过库。
    pub last_line_offset: i64,
    /// 上一轮这个解析器存下的私有状态;首次为 None。
    /// 流水线只存不看,内容格式由解析器自己定。
    // T14b 迁入 gemini/opencode 之前没有任何非测试解析器读它:这是给
    // 解析器实现的接口字段,不是死代码。
    #[allow(dead_code)]
    pub parser_state: Option<&'a str>,
}

/// 一条用量记录的身份。**字符串由解析器拼好交进来**,core 不知道
/// 也不需要知道它们长什么样。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageIdentity {
    /// legacy 表的 request_id。
    pub request_id: String,
    /// bound 路径的 event_id。unbound-only 的解析器填 `None`。
    pub event_id: Option<String>,
    /// 上游关联 ID;没有就是 None。
    pub upstream_correlation_id: Option<String>,
    /// 写进 legacy 行 `message_id` 列的值;没有就是 None。
    pub message_id: Option<String>,
    /// 仅用于日志的短标识(插入失败时打这个)。
    pub log_label: String,
}

/// 一条从会话日志里解析出来的用量记录,已经归一化。
///
/// 字段以重构前实际写进库的那些为准——**没有新增或删减语义**。
/// 这是纯重构,落库内容必须与重构前完全一致。
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedUsage {
    pub identity: UsageIdentity,
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_creation_tokens: u32,
    /// 事件发生时刻(秒)。解析不出来时由解析器按现有代码的兜底逻辑填。
    pub occurred_at: i64,
    /// 会话 ID。**注意两家用法不同**:写库时它进 legacy 字段,
    /// 不能当跨源去重键——照抄现有代码的注释和用法。
    pub session_id: Option<String>,
    /// 这一行在文件里的行号,供流水线推进水位线。
    pub line_offset: i64,
    /// 上游已经算好的总费用。`Some` 时直接落库、不再查定价表。
    /// 类型与写库列一致——照抄现在 total_cost_usd 那一列用的类型。
    pub upstream_total_cost: Option<String>,
}

/// `parse` 的产物:归一化用量记录 + 要求流水线下轮带回的私有状态。
pub struct ParseOutput {
    pub records: Vec<ParsedUsage>,
    /// 要求下次带回来的状态。`None` 表示保持上一轮的值不变。
    pub next_state: Option<String>,
}

/// 单文件游标决议结果:流水线据此判断「跳过 / 解析 / 写回」。
#[derive(Debug, Clone)]
pub struct FileCursor {
    /// 游标键。按路径,或按文件实体身份(device/inode 哈希),由解析器决定。
    pub cursor_key: String,
    /// 写进 cursor 表的 resource_identity。
    pub resource_identity: Option<String>,
    /// 需要从路径键提升为实体键的旧游标键(仅首次升级时存在)。
    pub legacy_cursor_key: Option<String>,
    /// 上次成功同步的 mtime(纳秒)与行号水位线。
    pub last_modified: i64,
    pub last_offset: i64,
    /// 写回时记录的 size_bytes。
    pub size_bytes: i64,
    /// 文件未变化(走跳过分支)时是否仍要写回游标。
    /// 用于「路径键旧游标 → 实体键」的提升与 rename 后的路径刷新。
    pub needs_update: bool,
}

/// 每个 agent 只需要实现这一个东西。
pub trait SessionLogParser {
    /// 这个 agent 的标识,由各解析器返回自己的名字。
    fn source(&self) -> &'static str;

    /// 这个 agent 写库时用的一组常量。**必填**:新接一个 agent 时,
    /// 编译器会在这里逼你把它们说清楚,而不是让你漏掉 core 里的某个 match。
    fn write_profile(&self) -> ProviderWriteProfile;

    /// 要扫哪些目录。
    fn log_roots(&self, home: &Path) -> Vec<PathBuf>;

    /// 判断一个文件要不要读(扩展名、命名规则等)。
    fn is_log_file(&self, path: &Path) -> bool;

    /// 除了日志文件本身,还有哪些文件的 mtime 也算「这个文件变了」。
    /// 默认空。SQLite 类日志用它把 `-wal` 纳进来。
    fn extra_change_sources(&self, _path: &Path) -> Vec<PathBuf> {
        Vec::new()
    }

    /// 扫描前的剪枝。有日期分区的 agent 用它做剪枝,没有的直接返回 `files` 原样。
    fn prune(&self, files: Vec<PathBuf>) -> Vec<PathBuf> {
        files
    }

    /// 流水线要不要把文件内容读进来交给 `parse`。
    /// 默认 `true`。自己开连接读的解析器(如 SQLite 类日志)返回 `false`,
    /// 此时 `LogFileContext.content` 是空串,句柄与 metadata 仍然可用。
    fn needs_file_content(&self) -> bool {
        true
    }

    /// 把一个文件解析成用量记录。返回的记录**已经排除了水位线以下的行**。
    fn parse(&self, ctx: &LogFileContext<'_>) -> Result<ParseOutput, AppError>;

    // ── 以下是流水线需要的钩子。任务书接口之外的最小补充(报告里说明):
    // 各家在「游标怎么键控」上语义不同——有的按路径、有的按文件实体,
    // 实体哈希的格式串含 provider 名,只能由各家解析器产出。默认实现
    // 按路径键控;实体键控的解析器覆盖实现。──

    /// 决议单个文件的游标。默认按路径键控。
    fn resolve_cursor(
        &self,
        path: &Path,
        _file: &fs::File,
        _metadata: &fs::Metadata,
        cursors: &SyncCursorMap,
        cursor_details: &HashMap<String, UsageSyncCursor>,
    ) -> Result<FileCursor, AppError> {
        let path_str = path.to_string_lossy().to_string();
        let (last_modified, last_offset) = cursors.get(&path_str).copied().unwrap_or((0, 0));
        let (resource_identity, size_bytes) =
            cursor_details.get(&path_str).map_or((None, 0), |cursor| {
                (cursor.resource_identity.clone(), cursor.size_bytes)
            });
        Ok(FileCursor {
            cursor_key: path_str,
            resource_identity,
            legacy_cursor_key: None,
            last_modified,
            last_offset,
            size_bytes,
            needs_update: false,
        })
    }

    /// 文件未变化(跳过解析)时的补充处理。默认什么都不做;
    /// 实体键控的解析器在这里把路径键旧游标提升为实体键游标。
    fn on_unchanged(
        &self,
        db: &Database,
        _path: &Path,
        _file_modified: i64,
        _metadata: &fs::Metadata,
        _cursor: &FileCursor,
    ) -> Result<(), AppError> {
        let _ = db;
        Ok(())
    }

    /// 单文件失败时的日志与错误串。默认实现,各解析器可按需覆盖。
    fn record_file_error(&self, path: &Path, error: &AppError, errors: &mut Vec<String>) {
        let msg = format!("{}: {error}", path.display());
        log::warn!("[SESSION-SYNC] 文件解析失败: {msg}");
        errors.push(msg);
    }

    /// 单条记录插入失败时的处理。返回 `Some(msg)` 表示这条也要进
    /// `SessionSyncResult::errors`;返回 `None` 表示只记日志。
    /// 默认实现返回 `None`——即 claude/codex 现在的行为。
    fn log_insert_failure(&self, record: &ParsedUsage, error: &AppError) -> Option<String> {
        log::warn!(
            "[SESSION-SYNC] 插入失败 ({}): {error}",
            record.identity.log_label
        );
        None
    }

    /// 本轮有插入失败时,是否放弃推进这个文件的游标(下轮整文件重读)。
    /// 默认 `false`——即 claude/codex 现在的行为(失败只记 log,游标照常推进)。
    /// 去重按 request_id 幂等,所以整文件重读与只重试失败记录结果一致,
    /// 只是多花一次解析。
    fn retry_file_on_insert_failure(&self) -> bool {
        false
    }

    /// 一轮同步结束后的汇总日志。默认实现,各解析器可按需覆盖。
    fn log_summary(&self, result: &SessionSyncResult) {
        log::info!(
            "[SESSION-SYNC] 同步完成: 导入 {} 条, 跳过 {} 条, 扫描 {} 个文件",
            result.imported,
            result.skipped,
            result.files_scanned
        );
    }
}

/// 用给定解析器跑一轮共享流水线。
///
/// 返回统一形状的 [`SessionSyncResult`];两个 bound 入口把它照现有代码
/// 原样转换成 `ProviderSessionSyncResult`。
pub fn sync_with_parser(
    db: &Database,
    parser: &dyn SessionLogParser,
    bound_provider_id: Option<&str>,
) -> Result<SessionSyncResult, AppError> {
    // 先预载游标再收集文件:游标决议只看内存里的两张表。
    let cursor_list = db.list_usage_sync_cursors(parser.source())?;
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

    let home = crate::config::get_home_dir();
    let mut files: Vec<PathBuf> = Vec::new();
    for root in parser.log_roots(&home) {
        collect_log_files(&root, parser, &mut files);
    }
    let collected = files.len() as u32;
    let files = parser.prune(files);
    let files_pruned = collected.saturating_sub(files.len() as u32);

    let mut result = SessionSyncResult {
        imported: 0,
        skipped: 0,
        files_scanned: files.len() as u32,
        files_pruned,
        errors: vec![],
    };

    // 写库 profile 在整个循环里只取一次:它由解析器说了算,不随记录变化。
    let profile = parser.write_profile();

    for file_path in &files {
        match sync_file_with_parser(
            db,
            parser,
            &profile,
            file_path,
            bound_provider_id,
            &mut sync_cursors,
            &cursor_details,
            &mut result.errors,
        ) {
            Ok((imported, skipped)) => {
                result.imported += imported;
                result.skipped += skipped;
            }
            Err(e) => parser.record_file_error(file_path, &e, &mut result.errors),
        }
    }

    if result.imported > 0 {
        parser.log_summary(&result);
    }

    Ok(result)
}

/// 流水线的单文件一步(独立出来供测试与单文件调用复用)。
///
/// 顺序与重构前逐字一致:绑定校验 → 打开 → 同句柄取 metadata →
/// 游标决议 → mtime 未变则跳过 → 整文件解析 → 去重写库 → 推进游标。
/// 只有全部记录写库成功后才推进 offset,bound 路径下任何插入失败都会
/// 让整文件回退(游标不动),下次同步重试。
///
/// `errors` 承接解析器要求进 `SessionSyncResult::errors` 的插入失败消息
/// (`log_insert_failure` 返回 `Some` 时)。
#[allow(clippy::too_many_arguments)]
pub(crate) fn sync_file_with_parser(
    db: &Database,
    parser: &dyn SessionLogParser,
    profile: &ProviderWriteProfile,
    file_path: &Path,
    bound_provider_id: Option<&str>,
    sync_cursors: &mut SyncCursorMap,
    cursor_details: &HashMap<String, UsageSyncCursor>,
    errors: &mut Vec<String>,
) -> Result<(u32, u32), AppError> {
    if let Some(provider_id) = bound_provider_id {
        validate_bound_session_agent(db, parser.source(), provider_id)?;
    }
    let file_path_str = file_path.to_string_lossy().to_string();

    // 先打开文件,再从同一句柄取 metadata:路径若在 stat 与实际解析之间
    // 被替换,游标和内容必须来自同一个文件实体(有解析器踩过的坑,对所有源成立)。
    let file =
        open_session_file(file_path).map_err(|e| AppError::Config(format!("无法打开文件: {e}")))?;
    let metadata = file
        .metadata()
        .map_err(|e| AppError::Config(format!("无法读取文件元数据: {e}")))?;
    // 变更判定 = 主文件与解析器声明的辅助变更源(如 SQLite 的 -wal)里最晚的
    // mtime。默认没有辅助源,claude/codex 的判定值与原来一模一样。
    let mut file_modified = metadata_modified_nanos(&metadata);
    for extra in parser.extra_change_sources(file_path) {
        if let Ok(extra_metadata) = fs::metadata(&extra) {
            file_modified = file_modified.max(metadata_modified_nanos(&extra_metadata));
        }
    }

    let cursor =
        parser.resolve_cursor(file_path, &file, &metadata, sync_cursors, cursor_details)?;

    // 文件未变化则跳过解析。解析器借 on_unchanged 提升旧路径游标。
    if file_modified <= cursor.last_modified {
        parser.on_unchanged(db, file_path, file_modified, &metadata, &cursor)?;
        if let Some(legacy) = &cursor.legacy_cursor_key {
            sync_cursors.remove(legacy);
        }
        sync_cursors.insert(
            cursor.cursor_key.clone(),
            (cursor.last_modified, cursor.last_offset),
        );
        return Ok((0, 0));
    }

    // 整文件重读(不是增量)——见模块顶部注释。needs_file_content 为 false 的
    // 解析器(如 SQLite 类日志)自己开连接读,这里不读内容、content 给空串;
    // 句柄与 metadata 仍来自同一个文件实体,变更判定和实体身份照常可用。
    let content = if parser.needs_file_content() {
        read_whole_file(&file)?
    } else {
        String::new()
    };
    // 私有状态跟着游标键走:实体键控的解析器在首次升级时,旧状态还挂在
    // 路径键旧游标下——与 update_sync_state_for_resource 的保留逻辑对齐。
    let parser_state = cursor_details
        .get(&cursor.cursor_key)
        .or_else(|| {
            cursor
                .legacy_cursor_key
                .as_deref()
                .and_then(|key| cursor_details.get(key))
        })
        .and_then(|cursor| cursor.parser_state_json.as_deref());
    let ctx = LogFileContext {
        path: file_path,
        file: &file,
        metadata: &metadata,
        content: &content,
        last_line_offset: cursor.last_offset,
        parser_state,
    };
    let output = parser.parse(&ctx)?;
    // 水位线 = 全量行数。两家都按「整个文件」计数,而不是按解析出的
    // 记录计数——文件尾部的非事件行也必须越过,否则下次同步会把它们
    // 重新当新行处理(按行提取 sessionId 的解析器会因此读到旧行)。
    let line_count = content.lines().count() as i64;

    let mut imported: u32 = 0;
    let mut skipped: u32 = 0;
    let mut had_insert_failure = false;
    for record in &output.records {
        match insert_usage_record(
            db,
            profile,
            record,
            &record.identity.request_id,
            bound_provider_id,
        ) {
            Ok(true) => imported += 1,
            Ok(false) => skipped += 1,
            Err(e) => {
                if bound_provider_id.is_some() {
                    return Err(e);
                }
                had_insert_failure = true;
                if let Some(message) = parser.log_insert_failure(record, &e) {
                    errors.push(message);
                }
                skipped += 1;
            }
        }
    }

    // 插入失败驱动的文件级重试:解析器要求时,本轮有任何插入失败就不推进
    // 游标(parser_state_json 同样不写),下轮整文件重读。去重按 request_id
    // 幂等,重读不会重复入库。默认 false = claude/codex 现状。
    if had_insert_failure && parser.retry_file_on_insert_failure() {
        return Ok((imported, skipped));
    }

    update_sync_state_for_resource(
        db,
        parser.source(),
        &cursor.cursor_key,
        &file_path_str,
        cursor.resource_identity.as_deref(),
        cursor.legacy_cursor_key.as_deref(),
        file_modified,
        cursor.size_bytes,
        line_count,
        output.next_state.as_deref(),
    )?;
    if let Some(legacy) = &cursor.legacy_cursor_key {
        sync_cursors.remove(legacy);
    }
    sync_cursors.insert(cursor.cursor_key.clone(), (file_modified, line_count));

    Ok((imported, skipped))
}

/// 递归收集 `root` 下解析器认可的文件。带深度上限,防符号链接环。
fn collect_log_files(root: &Path, parser: &dyn SessionLogParser, files: &mut Vec<PathBuf>) {
    collect_log_files_depth(root, parser, files, 0, 8);
}

fn collect_log_files_depth(
    dir: &Path,
    parser: &dyn SessionLogParser,
    files: &mut Vec<PathBuf>,
    depth: u32,
    max_depth: u32,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth < max_depth {
                collect_log_files_depth(&path, parser, files, depth + 1, max_depth);
            }
        } else if parser.is_log_file(&path) {
            files.push(path);
        }
    }
}

/// 把已打开的会话文件整个读进内存。
///
/// 用 `from_utf8_lossy` 而不是 `read_to_string`:旧实现用 `BufRead::lines()`
/// 逐行容忍无效 UTF-8(坏行直接跳过),严格解码会把单行坏字节放大成
/// 整文件失败。lossy 转换保住行边界与行数,坏行替换成 U+FFFD 后仍会被
/// JSON 解析自然跳过——与旧行为等价。
fn read_whole_file(file: &fs::File) -> Result<String, AppError> {
    let mut bytes = Vec::new();
    let mut reader = file;
    reader
        .read_to_end(&mut bytes)
        .map_err(|e| AppError::Config(format!("无法读取文件: {e}")))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// 打开会话文件;测试构建下累计文件打开次数(剪枝测试断言打开数)。
fn open_session_file(path: &Path) -> std::io::Result<fs::File> {
    #[cfg(test)]
    SESSION_FILE_OPEN_COUNT.with(|count| count.set(count.get() + 1));
    fs::File::open(path)
}

// 实测剪枝效果的 File::open 计数器(仅测试构建存在)。
// 用 thread_local 而不是全局静态量,避免并行测试互相污染计数。
#[cfg(test)]
thread_local! {
    static SESSION_FILE_OPEN_COUNT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_session_file_open_count() {
    SESSION_FILE_OPEN_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn session_file_open_count() -> u64 {
    SESSION_FILE_OPEN_COUNT.with(|count| count.get())
}

// 订阅活动标记的测试计数器(仅测试构建存在):按订阅 id 记录流水线决定
// 调 mark_subscription_activity 的次数。真实调用在 quota 的全局内存表里,
// 没有对外读接口,所以用这个紧挨着调用点的替身断言「None 时不调」。
#[cfg(test)]
thread_local! {
    static SUBSCRIPTION_ACTIVITY_MARKS: std::cell::RefCell<HashMap<&'static str, u64>> =
        std::cell::RefCell::new(HashMap::new());
}

#[cfg(test)]
fn record_subscription_activity_mark(subscription_id: &'static str) {
    SUBSCRIPTION_ACTIVITY_MARKS.with(|marks| {
        *marks.borrow_mut().entry(subscription_id).or_insert(0) += 1;
    });
}

#[cfg(test)]
pub(crate) fn reset_subscription_activity_marks() {
    SUBSCRIPTION_ACTIVITY_MARKS.with(|marks| marks.borrow_mut().clear());
}

#[cfg(test)]
pub(crate) fn subscription_activity_mark_count(subscription_id: &'static str) -> u64 {
    SUBSCRIPTION_ACTIVITY_MARKS
        .with(|marks| marks.borrow().get(subscription_id).copied().unwrap_or(0))
}

/// 把一条已归一化的用量记录写进库(去重 + 插入),返回是否新插入。
///
/// 两家的写入 SQL 形状完全相同,只有 provider 常量、费用计算语义与
/// 事件标识不同——provider 常量由调用方传入的 [`ProviderWriteProfile`]
/// 提供,事件标识已经拼在 [`UsageIdentity`] 的字段里。
pub(crate) fn insert_usage_record(
    db: &Database,
    profile: &ProviderWriteProfile,
    parsed: &ParsedUsage,
    request_id: &str,
    bound_provider_id: Option<&str>,
) -> Result<bool, AppError> {
    if let Some(provider_id) = bound_provider_id {
        insert_bound_usage_record(db, provider_id, profile, parsed, request_id)
    } else {
        insert_legacy_usage_record(db, profile, parsed, request_id)
    }
}

/// 写库用的 provider 常量组。**core 不知道它们的具体值**,由解析器在
/// [`SessionLogParser::write_profile`] 里给出——这是「core 需要知道哪些
/// 参数」的契约,不进数据库 schema。
pub(crate) struct ProviderWriteProfile {
    pub(crate) app_type: &'static str,
    pub(crate) legacy_provider_id: &'static str,
    pub(crate) provider_type: &'static str,
    pub(crate) insert_error_prefix: &'static str,
    pub(crate) calculator_app: Option<&'static str>,
    /// bound 路径要用的 agent module id。unbound-only 的解析器填 `None`。
    pub(crate) agent_module_id: Option<&'static str>,
    /// `None` 表示这家没有订阅概念,插入后**不调**
    /// `mark_subscription_activity`。
    pub(crate) subscription_activity_id: Option<&'static str>,
}

/// bound 路径:经 UsageIngestionService 落库(usage_events + legacy 行)。
fn insert_bound_usage_record(
    db: &Database,
    provider_id: &str,
    profile: &ProviderWriteProfile,
    parsed: &ParsedUsage,
    request_id: &str,
) -> Result<bool, AppError> {
    let usage = TokenUsage {
        input_tokens: parsed.input_tokens,
        output_tokens: parsed.output_tokens,
        cache_read_tokens: parsed.cache_read_tokens,
        cache_creation_tokens: parsed.cache_creation_tokens,
        model: Some(parsed.model.clone()),
        message_id: parsed.identity.message_id.clone(),
    };
    // bound 路径需要这两个身份字段:unbound-only 的解析器填 None,
    // 走到这里是调用方配置错误(给 unbound-only 的来源走了 bound 入口),
    // 直接报错而不是静默编一个值。
    let event_id =
        parsed.identity.event_id.clone().ok_or_else(|| {
            AppError::Config("该解析器未提供 event_id,不能走 bound 路径".to_string())
        })?;
    let agent_module_id = profile.agent_module_id.ok_or_else(|| {
        AppError::Config("该解析器未提供 agent module id,不能走 bound 路径".to_string())
    })?;
    let outcome = UsageIngestionService::new(db).ingest(&UsageIngestionInput {
        event_id,
        source: TokenSource::SessionLog,
        provider_id: provider_id.to_string(),
        agent_module_id: agent_module_id.to_string(),
        frozen_provider_context: None,
        occurred_at: parsed.occurred_at,
        model: parsed.model.clone(),
        usage,
        upstream_cost: parsed
            .upstream_total_cost
            .as_deref()
            .map(|cost| {
                use std::str::FromStr;
                Decimal::from_str(cost)
                    .map(|total| UpstreamCost {
                        input_cost: None,
                        output_cost: None,
                        cache_read_cost: None,
                        cache_creation_cost: None,
                        total_cost: Some(total),
                    })
                    .map_err(|_| AppError::Message(format!("上游总费用无法解析为金额: {cost}")))
            })
            .transpose()?,
        request_id: Some(request_id.to_string()),
        // 会话日志的 transcript 会话 ID 识别的是「一次会话」而不是一次
        // 请求,不能当跨源去重键。
        session_id: None,
        upstream_correlation_id: parsed.identity.upstream_correlation_id.clone(),
        legacy: Some(LegacyLogInput {
            request_id: request_id.to_string(),
            provider_id: profile.legacy_provider_id.to_string(),
            app_type: profile.app_type.to_string(),
            request_model: parsed.model.clone(),
            pricing_model: parsed.model.clone(),
            latency_ms: 0,
            first_token_ms: None,
            status_code: 200,
            error_message: None,
            session_id: parsed.session_id.clone(),
            provider_type: Some(profile.provider_type.to_string()),
            is_streaming: true,
            cost_multiplier: Decimal::ONE,
        }),
    })?;
    Ok(outcome.inserted)
}

/// 无绑定路径:直写 proxy_request_logs(SQL 与重构前逐字相同)。
fn insert_legacy_usage_record(
    db: &Database,
    profile: &ProviderWriteProfile,
    parsed: &ParsedUsage,
    request_id: &str,
) -> Result<bool, AppError> {
    let conn = lock_conn!(db.conn);

    let dedup_key = DedupKey {
        app_type: profile.app_type,
        model: &parsed.model,
        input_tokens: parsed.input_tokens,
        output_tokens: parsed.output_tokens,
        cache_read_tokens: parsed.cache_read_tokens,
        cache_creation_tokens: parsed.cache_creation_tokens,
        created_at: parsed.occurred_at,
    };
    if should_skip_session_insert(&conn, request_id, &dedup_key)? {
        return Ok(false);
    }

    let usage = TokenUsage {
        input_tokens: parsed.input_tokens,
        output_tokens: parsed.output_tokens,
        cache_read_tokens: parsed.cache_read_tokens,
        cache_creation_tokens: parsed.cache_creation_tokens,
        model: Some(parsed.model.clone()),
        message_id: None,
    };

    // 上游已经算好的总费用优先:直接落库、不查定价表,形状逐字节照抄
    // opencode 现在写的那五个值(其余四列是 "0")。None 时与重构前一样
    // 从定价表计算。
    let (input_cost, output_cost, cache_read_cost, cache_creation_cost, total_cost) = match parsed
        .upstream_total_cost
        .as_deref()
    {
        Some(cost) => (
            "0".to_string(),
            "0".to_string(),
            "0".to_string(),
            "0".to_string(),
            cost.to_string(),
        ),
        None => {
            let pricing = find_model_pricing(&conn, &parsed.model);
            let multiplier = Decimal::from(1);
            match pricing {
                Some(p) => {
                    let cost = match profile.calculator_app {
                        // 有的上游 app 的 input 字段包含 cache read,计费时要先扣掉;
                        // 有的 app 的 input 已经是 fresh input(不扣)。
                        Some(app) => CostCalculator::calculate_for_app(app, &usage, &p, multiplier),
                        None => CostCalculator::calculate(&usage, &p, multiplier),
                    };
                    (
                        cost.input_cost.to_string(),
                        cost.output_cost.to_string(),
                        cost.cache_read_cost.to_string(),
                        cost.cache_creation_cost.to_string(),
                        cost.total_cost.to_string(),
                    )
                }
                None => (
                    "0".to_string(),
                    "0".to_string(),
                    "0".to_string(),
                    "0".to_string(),
                    "0".to_string(),
                ),
            }
        }
    };

    let inserted_rows = conn
        .execute(
            "INSERT OR IGNORE INTO proxy_request_logs (
            request_id, provider_id, app_type, model, request_model,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
            input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd, total_cost_usd,
            latency_ms, first_token_ms, status_code, error_message, session_id,
            provider_type, is_streaming, cost_multiplier, created_at, data_source
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
            rusqlite::params![
                request_id,
                profile.legacy_provider_id,
                profile.app_type,
                parsed.model,
                parsed.model,
                parsed.input_tokens,
                parsed.output_tokens,
                parsed.cache_read_tokens,
                parsed.cache_creation_tokens,
                input_cost,
                output_cost,
                cache_read_cost,
                cache_creation_cost,
                total_cost,
                0i64,               // latency_ms: 会话日志无此数据
                Option::<i64>::None, // first_token_ms
                200i64,             // status_code: 产生计费 token 即视为成功
                Option::<String>::None, // error_message
                parsed.session_id,
                Some(profile.provider_type),
                1i64,               // is_streaming
                "1.0",              // cost_multiplier
                parsed.occurred_at,
                profile.provider_type,
            ],
        )
        .map_err(|e| AppError::Database(format!("{}失败: {e}", profile.insert_error_prefix)))?;

    // 仅在确实写入新行时通知前端,避免 INSERT OR IGNORE 跳过时产生空刷新
    if inserted_rows > 0 {
        crate::usage_events::notify_log_recorded();
        // 用量活动 → 触发订阅额度的一次短去抖补刷。没有订阅概念的来源
        // (write_profile 里为 None)跳过这一步。
        if let Some(subscription_id) = profile.subscription_activity_id {
            #[cfg(test)]
            record_subscription_activity_mark(subscription_id);
            crate::quota::mark_subscription_activity(subscription_id);
        }
    }

    Ok(true)
}

/// 当前 Unix 秒。解析不出事件时间时的兜底,与重构前各家 `current_timestamp` 相同。
pub(crate) fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// 事件发生时刻(秒)。解析不出时兜底当前时间——照抄重构前两家的逻辑。
pub(crate) fn occurred_at_secs(timestamp: Option<&str>) -> i64 {
    timestamp
        .and_then(|timestamp| chrono::DateTime::parse_from_rfc3339(timestamp).ok())
        .map(|timestamp| timestamp.timestamp())
        .unwrap_or_else(current_timestamp)
}

/// 获取 v14 `usage_sync_cursors` 中某条目的同步进度。
///
/// Shared by all session_usage_* parsers.
#[cfg(test)]
pub(crate) fn get_sync_state(
    db: &Database,
    source: &str,
    cursor_key: &str,
) -> Result<(i64, i64), AppError> {
    Ok(db
        .get_usage_sync_cursor(source, cursor_key)?
        .map(|cursor| (cursor.modified_at_ns, cursor.line_offset))
        .unwrap_or((0, 0)))
}

/// 预载某来源的全部游标(键 → (mtime, 行水位))。
///
/// T14b 之后生产解析器都走 `sync_with_parser`(内部自行预载),这个函数
/// 只剩 claude.rs 的测试在用;保留给测试与后续来源,先压掉 dead_code。
#[allow(dead_code)]
pub(crate) fn load_sync_cursors(db: &Database, source: &str) -> Result<SyncCursorMap, AppError> {
    Ok(db
        .list_usage_sync_cursors(source)?
        .into_iter()
        .map(|cursor| {
            (
                cursor.cursor_key,
                (cursor.modified_at_ns, cursor.line_offset),
            )
        })
        .collect())
}

/// 返回文件 mtime 的纳秒时间戳。
///
/// v13 `session_log_sync.last_modified` 旧数据是秒级时间戳;迁移后的新写入
/// 使用纳秒值,旧值会自然触发一次增量重扫,并继续依赖行 offset 避免重复导入。
pub(crate) fn metadata_modified_nanos(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// 更新 v14 `usage_sync_cursors` 中某条目的同步进度。
///
/// T14b 之后生产解析器都走 `sync_with_parser`(内部自行写回),这个函数
/// 只剩 claude.rs 的测试在用;保留给测试与后续来源,先压掉 dead_code。
#[allow(dead_code)]
pub(crate) fn update_sync_state(
    db: &Database,
    source: &str,
    cursor_key: &str,
    last_modified: i64,
    last_offset: i64,
) -> Result<(), AppError> {
    let existing = db.get_usage_sync_cursor(source, cursor_key)?;
    let resource_identity = existing
        .as_ref()
        .and_then(|cursor| cursor.resource_identity.as_deref());
    let size_bytes = existing
        .as_ref()
        .map(|cursor| cursor.size_bytes)
        .unwrap_or(0);
    update_sync_state_for_resource(
        db,
        source,
        cursor_key,
        cursor_key,
        resource_identity,
        None,
        last_modified,
        size_bytes,
        last_offset,
        None,
    )
}

/// 更新以稳定资源标识为 key 的同步进度,同时保留当前展示路径。
///
/// `parser_state_json` 是解析器下轮要带回来的私有状态:传 `Some` 时覆盖,
/// 传 `None` 时保持游标里原有的值(与「parse 返回 next_state: None 表示
/// 保持上一轮」的契约一致)。
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_sync_state_for_resource(
    db: &Database,
    source: &str,
    cursor_key: &str,
    resource_path: &str,
    resource_identity: Option<&str>,
    legacy_cursor_key: Option<&str>,
    last_modified: i64,
    size_bytes: i64,
    last_offset: i64,
    parser_state_json: Option<&str>,
) -> Result<(), AppError> {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let existing = db.get_usage_sync_cursor(source, cursor_key)?;
    let legacy_existing = if existing.is_none() {
        match legacy_cursor_key {
            Some(legacy_cursor_key) => db.get_usage_sync_cursor(source, legacy_cursor_key)?,
            None => None,
        }
    } else {
        None
    };
    let previous = existing.as_ref().or(legacy_existing.as_ref());
    let cursor = UsageSyncCursor {
        source: source.to_string(),
        cursor_key: cursor_key.to_string(),
        resource_path: Some(resource_path.to_string()),
        resource_identity: resource_identity
            .map(str::to_string)
            .or_else(|| previous.and_then(|cursor| cursor.resource_identity.clone())),
        modified_at_ns: last_modified,
        size_bytes,
        byte_offset: previous.map(|cursor| cursor.byte_offset).unwrap_or(0),
        line_offset: last_offset,
        parser_state_json: parser_state_json
            .map(str::to_string)
            .or_else(|| previous.and_then(|cursor| cursor.parser_state_json.clone())),
        last_success_at: now,
    };
    match legacy_cursor_key {
        Some(legacy_cursor_key) => db.promote_usage_sync_cursor(source, legacy_cursor_key, &cursor),
        None => db.put_usage_sync_cursor(&cursor),
    }
}

/// 查询数据来源分布统计
pub fn get_data_source_breakdown(db: &Database) -> Result<Vec<DataSourceSummary>, AppError> {
    let conn = lock_conn!(db.conn);

    let effective_filter = effective_usage_log_filter("l");
    let sql = format!(
        "SELECT COALESCE(l.data_source, 'proxy') as ds, COUNT(*) as cnt,
                COALESCE(SUM(CAST(l.total_cost_usd AS REAL)), 0) as cost
         FROM proxy_request_logs l
         WHERE {effective_filter}
         GROUP BY ds
         ORDER BY cnt DESC"
    );

    let mut stmt = conn.prepare(&sql)?;

    let rows = stmt.query_map([], |row| {
        Ok(DataSourceSummary {
            data_source: row.get(0)?,
            request_count: row.get::<_, i64>(1)? as u32,
            total_cost_usd: format!("{:.6}", row.get::<_, f64>(2)?),
        })
    })?;

    let mut summaries = Vec::new();
    for row in rows {
        summaries.push(row.map_err(|e| AppError::Database(e.to_string()))?);
    }

    Ok(summaries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::claude::ClaudeParser;
    use crate::ingest::codex::CodexParser;
    use crate::ingest::gemini::GeminiParser;
    use crate::ingest::opencode::OpenCodeParser;

    fn sync_one_file(
        db: &Database,
        parser: &dyn SessionLogParser,
        file_path: &Path,
        bound_provider_id: Option<&str>,
    ) -> Result<(u32, u32), AppError> {
        let source = parser.source();
        let cursor_list = db.list_usage_sync_cursors(source)?;
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
        let profile = parser.write_profile();
        let mut errors = Vec::new();
        sync_file_with_parser(
            db,
            parser,
            &profile,
            file_path,
            bound_provider_id,
            &mut sync_cursors,
            &cursor_details,
            &mut errors,
        )
    }

    /// 同一份流水线喂四个不同 parser:各自的解析与写库结果互不串味。
    #[test]
    fn pipeline_keeps_all_four_parsers_apart() -> Result<(), AppError> {
        let db = Database::memory()?;
        let tmp = std::env::temp_dir().join(format!(
            "llm-usage-bar-pipeline-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&tmp).unwrap();
        let claude_file = tmp.join("claude.jsonl");
        let codex_file = tmp.join("codex.jsonl");
        let gemini_file = tmp.join("gemini.json");
        let opencode_db = tmp.join("opencode.db");

        let claude_line = r#"{"type":"assistant","message":{"id":"msg_pipeline","model":"claude-opus-4-6","usage":{"input_tokens":3,"output_tokens":5,"cache_read_input_tokens":1,"cache_creation_input_tokens":2},"stop_reason":"end_turn"},"timestamp":"2026-04-05T12:00:00Z","sessionId":"session-pipeline"}"#;
        let codex_line = r#"{"timestamp":"2026-07-14T00:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":4,"output_tokens":2}}}}"#;
        let gemini_json = r#"{"sessionId":"session-gemini","messages":[{"type":"gemini","tokens":{"input":7,"output":3,"cached":1,"thoughts":2},"id":"gm_pipeline","model":"gemini-2.5-pro","timestamp":"2026-04-05T12:00:00Z"}]}"#;
        fs::write(&claude_file, format!("{claude_line}\n")).unwrap();
        fs::write(&codex_file, format!("{codex_line}\n")).unwrap();
        fs::write(&gemini_file, gemini_json).unwrap();
        write_opencode_test_db(
            &opencode_db,
            &[("s-opencode", 100)],
            &[(
                "m_pipeline",
                "s-opencode",
                r#"{"role":"assistant","tokens":{"input":100,"output":20},"modelID":"m-test","time":{"created":1000,"completed":2000}}"#,
                90,
                200,
            )],
        );

        // 正确配对:各导 1 条。
        let codex_parser = CodexParser::new(PathBuf::from("/unused"), Vec::new(), 2);
        let gemini_parser = GeminiParser::new(tmp.clone());
        let opencode_parser = OpenCodeParser::new(opencode_db.clone());
        assert_eq!(sync_one_file(&db, &ClaudeParser, &claude_file, None)?.0, 1);
        assert_eq!(sync_one_file(&db, &codex_parser, &codex_file, None)?.0, 1);
        assert_eq!(sync_one_file(&db, &gemini_parser, &gemini_file, None)?.0, 1);
        assert_eq!(
            sync_one_file(&db, &opencode_parser, &opencode_db, None)?.0,
            1
        );

        // 交叉喂:文本类三家互不认对方的行。
        assert_eq!(sync_one_file(&db, &codex_parser, &claude_file, None)?.0, 0);
        assert_eq!(sync_one_file(&db, &ClaudeParser, &codex_file, None)?.0, 0);
        assert_eq!(sync_one_file(&db, &ClaudeParser, &gemini_file, None)?.0, 0);
        assert_eq!(sync_one_file(&db, &codex_parser, &gemini_file, None)?.0, 0);
        assert_eq!(sync_one_file(&db, &gemini_parser, &claude_file, None)?.0, 0);
        assert_eq!(sync_one_file(&db, &gemini_parser, &codex_file, None)?.0, 0);

        // opencode 的「文件」是 SQLite 库:喂给文本三家,Claude/Codex 一行也
        // 解析不出来;Gemini 整文件 JSON 解析失败(与旧实现一样报错、零导入)。
        assert_eq!(sync_one_file(&db, &ClaudeParser, &opencode_db, None)?.0, 0);
        assert_eq!(sync_one_file(&db, &codex_parser, &opencode_db, None)?.0, 0);
        assert!(sync_one_file(&db, &gemini_parser, &opencode_db, None).is_err());
        // 非 SQLite 文件喂给 opencode:打不开数据库(与旧实现一样报错、零导入)。
        assert!(sync_one_file(&db, &opencode_parser, &claude_file, None).is_err());
        assert!(sync_one_file(&db, &opencode_parser, &gemini_file, None).is_err());

        let conn = lock_conn!(db.conn);
        let rows: Vec<(String, String)> = {
            let mut stmt = conn
                .prepare("SELECT app_type, request_id FROM proxy_request_logs ORDER BY request_id")
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        // 四个 parser 只能各自写入自己的记录,且 app_type / request_id 互不串味。
        assert_eq!(rows.len(), 4);
        // 按 (app_type, request_id) 排序后逐一比对,不依赖 request_id 的字典序。
        let mut by_app = rows.clone();
        by_app.sort();
        assert_eq!(
            by_app[0],
            ("claude".to_string(), "session:msg_pipeline".to_string())
        );
        assert_eq!(by_app[1].0, "codex");
        assert!(
            by_app[1].1.starts_with("codex_session:file-"),
            "codex 记录必须用文件作用域身份,不能用 Claude 的 session: 前缀"
        );
        assert_eq!(
            by_app[2],
            (
                "gemini".to_string(),
                "gemini_session:session-gemini:gm_pipeline".to_string()
            )
        );
        assert_eq!(
            by_app[3],
            (
                "opencode".to_string(),
                "opencode_session:s-opencode:m_pipeline".to_string()
            )
        );
        drop(conn);

        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    /// 两家的 request_id / event_id 与重构前逐字节相同(直接断言字面量)。
    #[test]
    fn usage_identity_strings_match_pre_refactor_literals() {
        let claude = UsageIdentity {
            request_id: format!(
                "{}{}",
                crate::usage::metering::parser::SESSION_REQUEST_ID_PREFIX,
                "msg_abc"
            ),
            event_id: Some("claude-session:msg_abc".to_string()),
            upstream_correlation_id: Some("msg_abc".to_string()),
            message_id: Some("msg_abc".to_string()),
            log_label: "msg_abc".to_string(),
        };
        assert_eq!(claude.request_id, "session:msg_abc");
        assert_eq!(claude.event_id.as_deref(), Some("claude-session:msg_abc"));
        assert_eq!(claude.upstream_correlation_id.as_deref(), Some("msg_abc"));

        let codex = UsageIdentity {
            request_id: "codex_session:session-scope:3".to_string(),
            event_id: Some("codex-session:codex_session:session-scope:3".to_string()),
            upstream_correlation_id: None,
            message_id: None,
            log_label: "codex_session:session-scope:3".to_string(),
        };
        assert_eq!(codex.request_id, "codex_session:session-scope:3");
        assert_eq!(
            codex.event_id.as_deref(),
            Some("codex-session:codex_session:session-scope:3")
        );
        assert_eq!(codex.upstream_correlation_id, None);
    }

    // ── T14a 新增测试:五个新槽位的契约 ──

    /// 新接口钩子的测试替身:可配置辅助变更源、下轮私有状态、插入失败消息、
    /// 插入失败后的文件级重试要求与「是否需要文件内容」,并记录每轮 parse
    /// 实际看到的私有状态与内容长度。
    struct HookProbeParser {
        root: PathBuf,
        extra_sources: Vec<PathBuf>,
        next_state: Option<String>,
        insert_failure_message: Option<String>,
        retry_on_insert_failure: bool,
        needs_content: bool,
        observed_states: std::cell::RefCell<Vec<Option<String>>>,
        observed_content_lens: std::cell::RefCell<Vec<usize>>,
    }

    impl HookProbeParser {
        fn new(root: PathBuf) -> Self {
            Self {
                root,
                extra_sources: Vec::new(),
                next_state: None,
                insert_failure_message: None,
                retry_on_insert_failure: false,
                needs_content: true,
                observed_states: std::cell::RefCell::new(Vec::new()),
                observed_content_lens: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn with_extra_sources(mut self, sources: Vec<PathBuf>) -> Self {
            self.extra_sources = sources;
            self
        }

        fn with_next_state(mut self, state: Option<String>) -> Self {
            self.next_state = state;
            self
        }

        fn with_insert_failure_message(mut self, message: Option<String>) -> Self {
            self.insert_failure_message = message;
            self
        }

        fn with_retry_file_on_insert_failure(mut self, retry: bool) -> Self {
            self.retry_on_insert_failure = retry;
            self
        }

        fn with_needs_file_content(mut self, needs: bool) -> Self {
            self.needs_content = needs;
            self
        }
    }

    impl SessionLogParser for HookProbeParser {
        fn source(&self) -> &'static str {
            "t14a-probe"
        }

        fn write_profile(&self) -> ProviderWriteProfile {
            probe_write_profile()
        }

        fn log_roots(&self, _home: &Path) -> Vec<PathBuf> {
            vec![self.root.clone()]
        }

        fn is_log_file(&self, path: &Path) -> bool {
            path.extension().and_then(|e| e.to_str()) == Some("jsonl")
        }

        fn extra_change_sources(&self, _path: &Path) -> Vec<PathBuf> {
            self.extra_sources.clone()
        }

        fn needs_file_content(&self) -> bool {
            self.needs_content
        }

        fn parse(&self, ctx: &LogFileContext<'_>) -> Result<ParseOutput, AppError> {
            self.observed_states
                .borrow_mut()
                .push(ctx.parser_state.map(str::to_string));
            self.observed_content_lens
                .borrow_mut()
                .push(ctx.content.len());
            let mut records = Vec::new();
            for (index, line) in ctx.content.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let request_id = format!("t14a-probe:{}", index + 1);
                records.push(ParsedUsage {
                    identity: UsageIdentity {
                        request_id: request_id.clone(),
                        event_id: None,
                        upstream_correlation_id: None,
                        message_id: None,
                        log_label: request_id,
                    },
                    model: "t14a-probe-model".to_string(),
                    input_tokens: 1,
                    output_tokens: 1,
                    cache_read_tokens: 0,
                    cache_creation_tokens: 0,
                    occurred_at: 1_000,
                    session_id: None,
                    line_offset: (index + 1) as i64,
                    upstream_total_cost: None,
                });
            }
            Ok(ParseOutput {
                records,
                next_state: self.next_state.clone(),
            })
        }

        fn log_insert_failure(&self, record: &ParsedUsage, error: &AppError) -> Option<String> {
            log::warn!(
                "[T14A-PROBE] 插入失败 ({}): {error}",
                record.identity.log_label
            );
            self.insert_failure_message
                .as_ref()
                .map(|message| format!("{message} ({})", record.identity.log_label))
        }

        fn retry_file_on_insert_failure(&self) -> bool {
            self.retry_on_insert_failure
        }
    }

    /// 探针的写库常量:没有订阅概念(subscription_activity_id 为 None)。
    fn probe_write_profile() -> ProviderWriteProfile {
        ProviderWriteProfile {
            app_type: "t14a-probe",
            legacy_provider_id: "_t14a_probe",
            provider_type: "t14a_probe",
            insert_error_prefix: "插入 T14a 探针日志",
            calculator_app: None,
            agent_module_id: None,
            subscription_activity_id: None,
        }
    }

    /// 单条探针记录(legacy 路径直插测试用)。
    fn probe_record(request_id: &str) -> ParsedUsage {
        ParsedUsage {
            identity: UsageIdentity {
                request_id: request_id.to_string(),
                event_id: None,
                upstream_correlation_id: None,
                message_id: None,
                log_label: request_id.to_string(),
            },
            model: "t14a-probe-model".to_string(),
            input_tokens: 1,
            output_tokens: 1,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            occurred_at: 1_000,
            session_id: None,
            line_offset: 1,
            upstream_total_cost: None,
        }
    }

    fn probe_temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "llm-usage-bar-t14a-{label}-{}",
            uuid::Uuid::new_v4()
        ))
    }

    /// 显式设置 mtime(秒级),让「谁更新更晚」的判定不依赖文件系统分辨率。
    fn set_mtime_seconds(path: &Path, seconds: i64) {
        filetime::set_file_mtime(path, filetime::FileTime::from_unix_time(seconds, 0)).unwrap();
    }

    /// 定价行故意给非零费率:若实现误查定价表,费用列必然与 "0"/"1.25" 不同。
    fn insert_t14a_pricing_row(db: &Database) -> Result<(), AppError> {
        let conn = lock_conn!(db.conn);
        conn.execute(
            "INSERT INTO model_pricing (
                model_id, display_name, input_cost_per_million, output_cost_per_million,
                cache_read_cost_per_million, cache_creation_cost_per_million
            ) VALUES ('t14a-cost-model', 'T14a 探针', '1', '2', '0.5', '0.25')",
            [],
        )?;
        Ok(())
    }

    fn legacy_cost_columns(
        db: &Database,
        request_id: &str,
    ) -> Result<(String, String, String, String, String), AppError> {
        let conn = lock_conn!(db.conn);
        Ok(conn.query_row(
            "SELECT input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd, total_cost_usd
             FROM proxy_request_logs WHERE request_id = ?1",
            [request_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )?)
    }

    /// `upstream_total_cost = Some` 时:五个费用列是 ("0","0","0","0",cost),
    /// 即使定价表里有该模型的定价也不查。
    #[test]
    fn upstream_total_cost_wins_without_consulting_the_pricing_table() -> Result<(), AppError> {
        let db = Database::memory()?;
        insert_t14a_pricing_row(&db)?;

        let profile = probe_write_profile();
        let mut record = probe_record("t14a-upstream-cost");
        record.model = "t14a-cost-model".to_string();
        record.upstream_total_cost = Some("1.25".to_string());

        assert!(insert_usage_record(
            &db,
            &profile,
            &record,
            "t14a-upstream-cost",
            None
        )?);
        let (input, output, cache_read, cache_creation, total) =
            legacy_cost_columns(&db, "t14a-upstream-cost")?;
        assert_eq!(
            (
                input.as_str(),
                output.as_str(),
                cache_read.as_str(),
                cache_creation.as_str(),
                total.as_str()
            ),
            ("0", "0", "0", "0", "1.25")
        );
        Ok(())
    }

    /// `upstream_total_cost = None` 时:费用仍走定价表,与本任务之前完全相同。
    #[test]
    fn missing_upstream_cost_still_prices_through_the_table() -> Result<(), AppError> {
        let db = Database::memory()?;
        insert_t14a_pricing_row(&db)?;

        let profile = probe_write_profile();
        let mut record = probe_record("t14a-priced");
        record.model = "t14a-cost-model".to_string();

        assert!(insert_usage_record(
            &db,
            &profile,
            &record,
            "t14a-priced",
            None
        )?);

        // 期望值 = 重构前的定价表路径:find_model_pricing + CostCalculator::calculate
        // (calculator_app 为 None 时)。直接调用同一组函数,钉死「None 仍走定价表」。
        let pricing = {
            let conn = lock_conn!(db.conn);
            find_model_pricing(&conn, "t14a-cost-model").expect("定价行必须可查")
        };
        let usage = TokenUsage {
            input_tokens: record.input_tokens,
            output_tokens: record.output_tokens,
            cache_read_tokens: record.cache_read_tokens,
            cache_creation_tokens: record.cache_creation_tokens,
            model: Some(record.model.clone()),
            message_id: None,
        };
        let expected = CostCalculator::calculate(&usage, &pricing, Decimal::from(1));
        let (input, output, cache_read, cache_creation, total) =
            legacy_cost_columns(&db, "t14a-priced")?;
        assert_eq!(
            (input, output, cache_read, cache_creation, total),
            (
                expected.input_cost.to_string(),
                expected.output_cost.to_string(),
                expected.cache_read_cost.to_string(),
                expected.cache_creation_cost.to_string(),
                expected.total_cost.to_string(),
            )
        );
        Ok(())
    }

    /// `extra_change_sources` 返回一个更新更晚的文件时,file_modified 取到的是
    /// 那个更晚的值:辅助文件不动时旧窗口内不重扫,只有它变新才触发重扫。
    #[test]
    fn extra_change_sources_extend_the_change_detection_window() -> Result<(), AppError> {
        let db = Database::memory()?;
        let tmp = probe_temp_dir("extra-mtime");
        fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("session.jsonl");
        let extra = tmp.join("opencode.db-wal");
        let base = 1_700_000_000i64;
        fs::write(&file, "{\"n\":1}\n").unwrap();
        fs::write(&extra, "wal").unwrap();
        set_mtime_seconds(&file, base);
        set_mtime_seconds(&extra, base + 10);

        let parser = HookProbeParser::new(tmp.clone()).with_extra_sources(vec![extra.clone()]);
        assert_eq!(
            sync_one_file(&db, &parser, &file, None)?.0,
            1,
            "首轮:主文件与辅助文件的最新 mtime 成为游标"
        );

        // 主文件在旧窗口内变新(仍早于辅助文件),辅助文件不动 → 必须跳过。
        fs::write(&file, "{\"n\":1}\n{\"n\":2}\n").unwrap();
        set_mtime_seconds(&file, base + 5);
        assert_eq!(
            sync_one_file(&db, &parser, &file, None)?.0,
            0,
            "主文件 mtime 仍在旧窗口内,不应重扫"
        );

        // 只有辅助文件变新 → 也要重扫:file_modified 取到的是那个更晚的值。
        set_mtime_seconds(&extra, base + 20);
        assert_eq!(
            sync_one_file(&db, &parser, &file, None)?.0,
            1,
            "辅助文件更晚时 file_modified 必须取到那个值"
        );
        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    /// `next_state = Some(x)` 会被写进游标的 parser_state_json;下一轮
    /// ctx.parser_state 读到 Some(x)。
    #[test]
    fn parser_state_round_trips_through_the_cursor() -> Result<(), AppError> {
        let db = Database::memory()?;
        let tmp = probe_temp_dir("state-roundtrip");
        fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("session.jsonl");
        let base = 1_700_000_000i64;
        fs::write(&file, "{\"n\":1}\n").unwrap();
        set_mtime_seconds(&file, base);

        let parser =
            HookProbeParser::new(tmp.clone()).with_next_state(Some("{\"round\":1}".to_string()));
        assert_eq!(sync_one_file(&db, &parser, &file, None)?.0, 1);
        assert_eq!(
            parser.observed_states.borrow().as_slice(),
            &[None],
            "首轮没有任何上一轮状态"
        );
        let path_str = file.to_string_lossy().to_string();
        let cursor = db
            .get_usage_sync_cursor("t14a-probe", &path_str)?
            .expect("游标必须存在");
        assert_eq!(cursor.parser_state_json.as_deref(), Some("{\"round\":1}"));

        // 第二轮:文件变新 → 重扫,上一轮存的状态原样带回。
        fs::write(&file, "{\"n\":1}\n{\"n\":2}\n").unwrap();
        set_mtime_seconds(&file, base + 5);
        let second = HookProbeParser::new(tmp.clone());
        assert_eq!(
            sync_one_file(&db, &second, &file, None)?.0,
            1,
            "第二行是新记录"
        );
        assert_eq!(
            second.observed_states.borrow().as_slice(),
            &[Some("{\"round\":1}".to_string())],
            "上一轮存下的状态必须原样带回"
        );
        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    /// `next_state = None` 时游标里原有的 parser_state_json 不被清掉。
    #[test]
    fn next_state_none_keeps_the_existing_parser_state() -> Result<(), AppError> {
        let db = Database::memory()?;
        let tmp = probe_temp_dir("state-keep");
        fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("session.jsonl");
        let base = 1_700_000_000i64;
        fs::write(&file, "{\"n\":1}\n").unwrap();
        set_mtime_seconds(&file, base);

        // 预置:游标里已有上一轮留下的私有状态。
        let path_str = file.to_string_lossy().to_string();
        db.put_usage_sync_cursor(&UsageSyncCursor {
            source: "t14a-probe".to_string(),
            cursor_key: path_str.clone(),
            resource_path: Some(path_str.clone()),
            resource_identity: None,
            modified_at_ns: 0, // 保证本轮触发重扫
            size_bytes: 0,
            byte_offset: 0,
            line_offset: 0,
            parser_state_json: Some("{\"seed\":true}".to_string()),
            last_success_at: 1,
        })?;

        let parser = HookProbeParser::new(tmp.clone());
        sync_one_file(&db, &parser, &file, None)?;
        let cursor = db
            .get_usage_sync_cursor("t14a-probe", &path_str)?
            .expect("游标必须存在");
        assert_eq!(
            cursor.parser_state_json.as_deref(),
            Some("{\"seed\":true}"),
            "next_state 为 None 时旧状态必须原样保留"
        );
        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    /// `log_insert_failure` 返回 Some 时,那条消息出现在 result.errors 里。
    #[test]
    fn insert_failure_messages_land_in_result_errors_when_hook_returns_some() -> Result<(), AppError>
    {
        let db = Database::memory()?;
        let tmp = probe_temp_dir("insert-error");
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("session.jsonl"), "{\"n\":1}\n").unwrap();
        {
            let conn = lock_conn!(db.conn);
            conn.execute_batch(
                "CREATE TRIGGER t14a_fail_insert
                 BEFORE INSERT ON proxy_request_logs
                 BEGIN SELECT RAISE(FAIL, 'forced t14a insert failure'); END",
            )?;
        }

        let parser = HookProbeParser::new(tmp.clone())
            .with_insert_failure_message(Some("插入失败已上报".to_string()));
        let result = sync_with_parser(&db, &parser, None)?;
        assert_eq!((result.imported, result.skipped), (0, 1));
        assert!(
            result
                .errors
                .iter()
                .any(|message| message.contains("插入失败已上报")),
            "hook 返回的消息必须出现在 errors 里: {:?}",
            result.errors
        );

        {
            let conn = lock_conn!(db.conn);
            conn.execute_batch("DROP TRIGGER t14a_fail_insert;")?;
        }
        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    /// `subscription_activity_id = None` 时不调 mark_subscription_activity;
    /// `Some(id)` 时只给那个 id 记一次。
    #[test]
    fn subscription_activity_is_only_marked_when_the_profile_has_one() -> Result<(), AppError> {
        let db = Database::memory()?;
        reset_subscription_activity_marks();

        let no_subscription = probe_write_profile();
        assert!(insert_usage_record(
            &db,
            &no_subscription,
            &probe_record("t14a-sub-none"),
            "t14a-sub-none",
            None
        )?);
        assert_eq!(
            subscription_activity_mark_count("t14a-sentinel-sub"),
            0,
            "没有订阅概念时不得打任何标记"
        );

        let mut with_subscription = probe_write_profile();
        with_subscription.subscription_activity_id = Some("t14a-sentinel-sub");
        assert!(insert_usage_record(
            &db,
            &with_subscription,
            &probe_record("t14a-sub-some"),
            "t14a-sub-some",
            None
        )?);
        assert_eq!(subscription_activity_mark_count("t14a-sentinel-sub"), 1);
        Ok(())
    }

    // ── T14b §0.1:retry_file_on_insert_failure 的两条契约 ──

    /// 默认 `false` 时,插入失败游标照常推进——claude/codex 的现状不变。
    #[test]
    fn insert_failure_advances_the_cursor_when_retry_is_off_by_default() -> Result<(), AppError> {
        let db = Database::memory()?;
        let tmp = probe_temp_dir("retry-default");
        fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("session.jsonl");
        fs::write(&file, "{\"n\":1}\n").unwrap();
        {
            let conn = lock_conn!(db.conn);
            conn.execute_batch(
                "CREATE TRIGGER t14a_fail_all
                 BEFORE INSERT ON proxy_request_logs
                 BEGIN SELECT RAISE(FAIL, 'forced t14a insert failure'); END",
            )?;
        }

        // 探针没开重试(默认 false):插入失败只记 log,游标照常推进。
        let parser = HookProbeParser::new(tmp.clone());
        assert_eq!(sync_one_file(&db, &parser, &file, None)?, (0, 1));
        let path_str = file.to_string_lossy().to_string();
        let (last_modified, last_offset) = get_sync_state(&db, "t14a-probe", &path_str)?;
        assert!(
            last_modified > 0,
            "默认 false 时游标必须照常推进,失败记录不重试"
        );
        assert_eq!(last_offset, 1, "水位线照常推进到文件行数");

        {
            let conn = lock_conn!(db.conn);
            conn.execute_batch("DROP TRIGGER t14a_fail_all;")?;
        }
        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    /// 返回 `true` 时,插入失败后游标不推进;下一轮重读同一文件,已入库的
    /// 记录不重复插入(证明重读是幂等的)。
    #[test]
    fn insert_failure_keeps_the_cursor_back_for_a_full_file_retry() -> Result<(), AppError> {
        let db = Database::memory()?;
        let tmp = probe_temp_dir("retry-file");
        fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("session.jsonl");
        fs::write(&file, "{\"n\":1}\n{\"n\":2}\n").unwrap();
        let path_str = file.to_string_lossy().to_string();

        // 只让第二条插入失败,模拟「部分成功、部分失败」的一轮。
        {
            let conn = lock_conn!(db.conn);
            conn.execute_batch(
                "CREATE TRIGGER t14a_fail_second
                 BEFORE INSERT ON proxy_request_logs
                 WHEN NEW.request_id = 't14a-probe:2'
                 BEGIN SELECT RAISE(FAIL, 'forced t14a insert failure'); END",
            )?;
        }

        let parser = HookProbeParser::new(tmp.clone())
            .with_insert_failure_message(Some("插入失败已上报".to_string()))
            .with_retry_file_on_insert_failure(true);
        assert_eq!(
            sync_one_file(&db, &parser, &file, None)?,
            (1, 1),
            "第一条入库、第二条失败"
        );
        assert_eq!(
            get_sync_state(&db, "t14a-probe", &path_str)?,
            (0, 0),
            "有插入失败且要求重试时,游标不得推进"
        );

        // 下一轮:触发器已拆,整文件重读。第一条按 request_id 去重不重复入库,
        // 第二条补上。
        {
            let conn = lock_conn!(db.conn);
            conn.execute_batch("DROP TRIGGER t14a_fail_second;")?;
        }
        let retry = HookProbeParser::new(tmp.clone()).with_retry_file_on_insert_failure(true);
        assert_eq!(
            sync_one_file(&db, &retry, &file, None)?,
            (1, 1),
            "重读幂等:第一条被去重跳过,第二条成功入库"
        );

        let (row_count, distinct_requests): (i64, i64) = {
            let conn = lock_conn!(db.conn);
            conn.query_row(
                "SELECT COUNT(*), COUNT(DISTINCT request_id) FROM proxy_request_logs",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?
        };
        assert_eq!((row_count, distinct_requests), (2, 2), "重读不得重复插入");
        assert!(
            get_sync_state(&db, "t14a-probe", &path_str)?.0 > 0,
            "整轮成功后游标才推进"
        );

        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    // ── T14b §0.2 与 §4 的新增测试 ──

    /// 造一个最小 opencode.db:一张 session 表 + 一张 message 表。
    /// messages 每条为 (id, session_id, data_json, time_created, time_updated)。
    fn write_opencode_test_db(
        path: &Path,
        sessions: &[(&str, i64)],
        messages: &[(&str, &str, &str, i64, i64)],
    ) {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session (id TEXT PRIMARY KEY, time_updated INTEGER);
             CREATE TABLE message (
                 id TEXT PRIMARY KEY,
                 session_id TEXT,
                 time_created INTEGER,
                 time_updated INTEGER,
                 data TEXT
             );",
        )
        .unwrap();
        for (id, time_updated) in sessions {
            conn.execute(
                "INSERT INTO session VALUES (?1, ?2)",
                rusqlite::params![id, time_updated],
            )
            .unwrap();
        }
        for (id, session_id, data, created, updated) in messages {
            conn.execute(
                "INSERT INTO message VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![id, session_id, created, updated, data],
            )
            .unwrap();
        }
    }

    /// gemini / opencode 的 request_id 与重构前逐字节相同;event_id 按 §0.2
    /// 决定一填 None(重构前不存在这个字符串,不发明)。
    #[test]
    fn gemini_and_opencode_identities_match_pre_refactor_literals() -> Result<(), AppError> {
        let tmp = probe_temp_dir("identity-literals");
        fs::create_dir_all(&tmp).unwrap();

        // gemini:直接喂一个单消息文件给 parse。
        let gemini_file = tmp.join("gemini.json");
        let gemini_json = r#"{"sessionId":"session-gemini","messages":[{"type":"gemini","tokens":{"input":7,"output":3,"cached":1,"thoughts":2},"id":"gm1","model":"gemini-2.5-pro","timestamp":"2026-04-05T12:00:00Z"}]}"#;
        fs::write(&gemini_file, gemini_json).unwrap();
        let file = fs::File::open(&gemini_file).unwrap();
        let metadata = file.metadata().unwrap();
        let content = fs::read_to_string(&gemini_file).unwrap();
        let ctx = LogFileContext {
            path: &gemini_file,
            file: &file,
            metadata: &metadata,
            content: &content,
            last_line_offset: 0,
            parser_state: None,
        };
        let gemini_records = GeminiParser::new(tmp.clone()).parse(&ctx)?.records;
        assert_eq!(gemini_records.len(), 1);
        assert_eq!(
            gemini_records[0].identity.request_id,
            "gemini_session:session-gemini:gm1"
        );
        assert_eq!(gemini_records[0].identity.event_id, None);

        // opencode:造一个真实的最小库,直接喂给 parse。
        let opencode_db = tmp.join("opencode.db");
        write_opencode_test_db(
            &opencode_db,
            &[("s1", 100)],
            &[(
                "m1",
                "s1",
                r#"{"role":"assistant","tokens":{"input":100,"output":20},"modelID":"m-test","time":{"created":1000,"completed":2000}}"#,
                90,
                200,
            )],
        );
        let file = fs::File::open(&opencode_db).unwrap();
        let metadata = file.metadata().unwrap();
        let ctx = LogFileContext {
            path: &opencode_db,
            file: &file,
            metadata: &metadata,
            content: "",
            last_line_offset: 0,
            parser_state: None,
        };
        let opencode_records = OpenCodeParser::new(opencode_db.clone())
            .parse(&ctx)?
            .records;
        assert_eq!(opencode_records.len(), 1);
        assert_eq!(
            opencode_records[0].identity.request_id,
            "opencode_session:s1:m1"
        );
        assert_eq!(opencode_records[0].identity.event_id, None);

        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    /// opencode 的上游费用直通:msg.cost > 0 时五个费用列是
    /// ("0","0","0","0",cost),与重构前逐字节相同。
    #[test]
    fn opencode_upstream_cost_writes_the_five_columns_verbatim() -> Result<(), AppError> {
        let db = Database::memory()?;
        let tmp = probe_temp_dir("opencode-cost");
        fs::create_dir_all(&tmp).unwrap();
        let opencode_db = tmp.join("opencode.db");
        write_opencode_test_db(
            &opencode_db,
            &[("s1", 100)],
            &[(
                "m1",
                "s1",
                r#"{"role":"assistant","cost":0.0023113,"tokens":{"input":3272,"output":383,"reasoning":419,"cache":{"write":0,"read":52480}},"modelID":"deepseek-v4-pro","time":{"created":1779755333700,"completed":1779755350639}}"#,
                90,
                200,
            )],
        );

        let parser = OpenCodeParser::new(opencode_db.clone());
        assert_eq!(
            sync_one_file(&db, &parser, &opencode_db, None)?.0,
            1,
            "cost > 0 的消息必须入库"
        );
        let (input, output, cache_read, cache_creation, total) =
            legacy_cost_columns(&db, "opencode_session:s1:m1")?;
        assert_eq!(
            (
                input.as_str(),
                output.as_str(),
                cache_read.as_str(),
                cache_creation.as_str(),
                total.as_str()
            ),
            ("0", "0", "0", "0", "0.0023113")
        );

        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    /// needs_file_content = false 时,流水线不读文件内容:垃圾二进制文件
    /// 不产生任何读取/解码错误,parse 拿到的 content 是空串。
    #[test]
    fn parsers_can_decline_file_content_so_garbage_binary_never_gets_decoded(
    ) -> Result<(), AppError> {
        let db = Database::memory()?;
        let tmp = probe_temp_dir("no-content");
        fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("session.jsonl");
        fs::write(&file, [0xFFu8, 0xFE, 0xFD, 0xFC, 0x00, 0x80, 0x81]).unwrap();

        let parser = HookProbeParser::new(tmp.clone()).with_needs_file_content(false);
        assert_eq!(
            sync_one_file(&db, &parser, &file, None)?,
            (0, 0),
            "空内容 → 零记录,且不得有任何读取/解码错误"
        );
        assert_eq!(
            parser.observed_content_lens.borrow().as_slice(),
            &[0],
            "needs_file_content=false 时 parse 拿到的 content 必须是空串"
        );

        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }
}
