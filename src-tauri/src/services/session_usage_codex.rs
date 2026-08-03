//! Codex 会话日志使用追踪
//!
//! 从 ~/.codex/sessions/ 下的 JSONL 会话文件中提取精确 token 使用数据，
//! 替代原有的 state_5.sqlite 估算方案。
//!
//! ## 数据流
//! ```text
//! ~/.codex/sessions/YYYY/MM/DD/*.jsonl → 增量解析 → delta 计算 → 费用计算 → proxy_request_logs 表
//! ```
//!
//! ## 解析的事件类型
//! - `session_meta` → 提取 session_id
//! - `turn_context` → 提取当前 model
//! - `event_msg` (type=token_count) → 提取累计 token 用量，计算 delta

use crate::codex_config::get_codex_config_dir;
use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::services::session_usage::{
    metadata_modified_nanos, update_sync_state_for_resource, SessionSyncResult,
};
use crate::services::usage_stats::{find_model_pricing, should_skip_session_insert, DedupKey};
use crate::usage::domain::{TokenSource, CODEX_AGENT_MODULE_ID};
use crate::usage::ingestion::{LegacyLogInput, UsageIngestionInput, UsageIngestionService};
use crate::usage::metering::calculator::{CostCalculator, ModelPricing};
use crate::usage::metering::parser::TokenUsage;
use crate::usage::session::{validate_bound_session_agent, ProviderSessionSyncResult};
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// 累计 token 用量（跟踪 total_token_usage 字段）
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
/// 不随 rename 变化，且不同文件复用同一路径时不会沿用旧 cursor 或事件 ID。
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
/// 处理规则（按顺序）：
/// 1. 转小写：`GLM-4.6` → `glm-4.6`
/// 2. 剥离 provider 前缀：`openai/gpt-5.4` → `gpt-5.4`
/// 3. 剥离 ISO 日期后缀：`gpt-5.4-2026-03-05` → `gpt-5.4`
/// 4. 剥离紧凑日期后缀：`gpt-5.4-20260305` → `gpt-5.4`
fn normalize_codex_model(raw: &str) -> String {
    // Step 1: 小写
    let mut name = raw.to_lowercase();

    // Step 2: 剥离 "provider/" 前缀（如 openai/, azure/）
    if let Some(pos) = name.rfind('/') {
        name = name[pos + 1..].to_string();
    }

    // Step 3: 剥离 ISO 日期后缀 -YYYY-MM-DD（正好 11 字符）
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

    // Step 4: 剥离紧凑日期后缀 -YYYYMMDD（正好 9 字符）
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

/// 同步 Codex 使用数据（从 JSONL 会话日志）
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

    let files = collect_codex_session_files(&codex_dir);

    let mut result = SessionSyncResult {
        imported: 0,
        skipped: 0,
        files_scanned: files.len() as u32,
        errors: vec![],
    };

    if files.is_empty() {
        return Ok(result);
    }

    for file_path in &files {
        match sync_single_codex_file(db, file_path, bound_provider_id) {
            Ok((imported, skipped)) => {
                result.imported += imported;
                result.skipped += skipped;
            }
            Err(e) => {
                let msg = format!("Codex 会话文件解析失败 {}: {e}", file_path.display());
                log::warn!("[CODEX-SYNC] {msg}");
                result.errors.push(msg);
            }
        }
    }

    if result.imported > 0 {
        log::info!(
            "[CODEX-SYNC] 同步完成: 导入 {} 条, 跳过 {} 条, 扫描 {} 个文件",
            result.imported,
            result.skipped,
            result.files_scanned
        );
    }

    Ok(result)
}

/// 收集所有 Codex 会话 JSONL 文件
fn collect_codex_session_files(codex_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();

    // 1. 扫描 sessions/YYYY/MM/DD/*.jsonl（日期分区目录）
    let sessions_dir = codex_dir.join("sessions");
    if sessions_dir.is_dir() {
        collect_jsonl_recursive(&sessions_dir, &mut files, 0, 3);
    }

    // 2. 扫描 archived_sessions/*.jsonl（扁平归档目录）
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

    files
}

/// 递归扫描目录下的 .jsonl 文件（限制最大深度）
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

/// 同步单个 Codex JSONL 文件，返回 (imported, skipped)
fn sync_single_codex_file(
    db: &Database,
    file_path: &Path,
    bound_provider_id: Option<&str>,
) -> Result<(u32, u32), AppError> {
    if let Some(provider_id) = bound_provider_id {
        validate_bound_session_agent(db, "codex", provider_id)?;
    }
    let file_path_str = file_path.to_string_lossy().to_string();

    // 先打开文件，再从同一句柄取 metadata 和实体标识，
    // 避免路径在 metadata 查询与实际解析之间被替换。
    let file =
        fs::File::open(file_path).map_err(|e| AppError::Config(format!("无法打开文件: {e}")))?;
    let metadata = file
        .metadata()
        .map_err(|e| AppError::Config(format!("无法读取文件元数据: {e}")))?;
    let file_identity = codex_file_identity(file_path, &file, &metadata)?;
    let cursor_key = &file_identity.resource_identity;
    let file_modified = metadata_modified_nanos(&metadata);
    let file_size = metadata.len().min(i64::MAX as u64) as i64;

    // 新 cursor 以文件实体为 key；首次升级时可从同路径的旧 cursor 提升。
    let resource_cursor = db.get_usage_sync_cursor("codex", cursor_key)?;
    let legacy_path_cursor = if resource_cursor.is_none() {
        db.get_usage_sync_cursor("codex", &file_path_str)?
            .filter(|cursor| {
                cursor
                    .resource_identity
                    .as_deref()
                    .is_none_or(|identity| identity == file_identity.resource_identity.as_str())
            })
    } else {
        None
    };
    let sync_cursor = resource_cursor.as_ref().or(legacy_path_cursor.as_ref());
    let legacy_cursor_key = legacy_path_cursor
        .as_ref()
        .map(|cursor| cursor.cursor_key.as_str());
    let (last_modified, last_offset) = sync_cursor
        .map(|cursor| (cursor.modified_at_ns, cursor.line_offset))
        .unwrap_or((0, 0));

    // 文件未变化则跳过
    if file_modified <= last_modified {
        let cursor_needs_promotion_or_path_refresh =
            resource_cursor.as_ref().is_none_or(|cursor| {
                cursor.resource_path.as_deref() != Some(file_path_str.as_str())
                    || cursor.resource_identity.as_deref()
                        != Some(file_identity.resource_identity.as_str())
            });
        if cursor_needs_promotion_or_path_refresh {
            update_sync_state_for_resource(
                db,
                "codex",
                cursor_key,
                &file_path_str,
                Some(&file_identity.resource_identity),
                legacy_cursor_key,
                last_modified.max(file_modified),
                file_size,
                last_offset,
            )?;
        }
        return Ok((0, 0));
    }

    // 逐行解析已用于确定实体标识的同一文件句柄。
    let reader = BufReader::new(file);

    let mut state = FileParseState {
        session_id: None,
        current_model: "unknown".to_string(),
        prev_total: None,
        event_index: 0,
    };

    let mut line_offset: i64 = 0;
    let mut imported: u32 = 0;
    let mut skipped: u32 = 0;

    for line_result in reader.lines() {
        line_offset += 1;

        let line = match line_result {
            Ok(l) => l,
            Err(_) => continue, // 容忍不完整的最后一行
        };

        if line.trim().is_empty() {
            continue;
        }

        // 快速过滤：在 JSON 反序列化前跳过无关行
        let is_event_msg = line.contains("\"event_msg\"");
        let is_turn_context = line.contains("\"turn_context\"");
        let is_session_meta = line.contains("\"session_meta\"");

        if !is_event_msg && !is_turn_context && !is_session_meta {
            continue;
        }
        if is_event_msg && !line.contains("\"token_count\"") {
            continue;
        }

        let value: serde_json::Value = match serde_json::from_str(&line) {
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

                // 提取模型（token_count 事件也可能携带 model）
                if let Some(model) = info
                    .get("model")
                    .or_else(|| info.get("model_name"))
                    .or_else(|| payload.get("model"))
                    .and_then(|v| v.as_str())
                {
                    state.current_model = normalize_codex_model(model);
                }

                // 优先用 total_token_usage（累计值），fallback 到 last_token_usage（增量值）
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
                    // 累计值模式：计算与上次的 delta
                    let d = compute_delta(&state.prev_total, &cumulative);
                    state.prev_total = Some(cumulative);
                    d
                } else {
                    // 增量值模式：直接使用 last_token_usage 的值
                    DeltaTokens {
                        input: cumulative.input as u32,
                        cached_input: cumulative.cached_input as u32,
                        output: cumulative.output as u32,
                    }
                };

                // 钳制：cached 不应超过 input（防护异常数据）
                let delta = DeltaTokens {
                    cached_input: delta.cached_input.min(delta.input),
                    ..delta
                };

                if delta.is_zero() {
                    continue; // 跳过 task 边界的零 delta 事件
                }

                state.event_index += 1;

                // 跳过已处理的行（但仍需解析以恢复状态）
                if line_offset <= last_offset {
                    continue;
                }

                // 生成唯一 request_id
                let request_scope = state
                    .session_id
                    .as_deref()
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("file-{}", file_identity.event_scope));
                let request_id = format!("codex_session:{request_scope}:{}", state.event_index);

                // 提取时间戳
                let timestamp = value
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let insert_result = if let Some(provider_id) = bound_provider_id {
                    insert_bound_codex_session_entry(
                        db,
                        provider_id,
                        &request_id,
                        &delta,
                        &state.current_model,
                        state.session_id.as_deref(),
                        timestamp.as_deref(),
                    )
                } else {
                    insert_codex_session_entry(
                        db,
                        &request_id,
                        &delta,
                        &state.current_model,
                        state.session_id.as_deref(),
                        timestamp.as_deref(),
                    )
                };
                match insert_result {
                    Ok(true) => imported += 1,
                    Ok(false) => skipped += 1,
                    Err(e) => {
                        if bound_provider_id.is_some() {
                            return Err(e);
                        }
                        log::warn!("[CODEX-SYNC] 插入失败 ({}): {e}", request_id);
                        skipped += 1;
                    }
                }
            }
            _ => {}
        }
    }

    // 更新同步状态
    update_sync_state_for_resource(
        db,
        "codex",
        cursor_key,
        &file_path_str,
        Some(&file_identity.resource_identity),
        legacy_cursor_key,
        file_modified,
        file_size,
        line_offset,
    )?;

    Ok((imported, skipped))
}

fn insert_bound_codex_session_entry(
    db: &Database,
    provider_id: &str,
    request_id: &str,
    delta: &DeltaTokens,
    model: &str,
    transcript_session_id: Option<&str>,
    timestamp: Option<&str>,
) -> Result<bool, AppError> {
    let occurred_at = timestamp
        .and_then(|timestamp| chrono::DateTime::parse_from_rfc3339(timestamp).ok())
        .map(|timestamp| timestamp.timestamp())
        .unwrap_or_else(current_timestamp);
    let usage = TokenUsage {
        input_tokens: delta.input,
        output_tokens: delta.output,
        cache_read_tokens: delta.cached_input,
        cache_creation_tokens: 0,
        model: Some(model.to_string()),
        message_id: None,
    };
    let outcome = UsageIngestionService::new(db).ingest(&UsageIngestionInput {
        event_id: format!("codex-session:{request_id}"),
        source: TokenSource::SessionLog,
        provider_id: provider_id.to_string(),
        agent_module_id: CODEX_AGENT_MODULE_ID.to_string(),
        frozen_provider_context: None,
        occurred_at,
        model: model.to_string(),
        usage,
        upstream_cost: None,
        request_id: Some(request_id.to_string()),
        // A transcript session spans multiple API calls and is therefore not a
        // request-level dedup identity.
        session_id: None,
        upstream_correlation_id: None,
        legacy: Some(LegacyLogInput {
            request_id: request_id.to_string(),
            provider_id: "_codex_session".to_string(),
            app_type: "codex".to_string(),
            request_model: model.to_string(),
            pricing_model: model.to_string(),
            latency_ms: 0,
            first_token_ms: None,
            status_code: 200,
            error_message: None,
            session_id: transcript_session_id.map(str::to_string),
            provider_type: Some("codex_session".to_string()),
            is_streaming: true,
            cost_multiplier: Decimal::ONE,
        }),
    })?;
    Ok(outcome.inserted)
}

fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// 插入单条 Codex 会话记录到 proxy_request_logs
fn insert_codex_session_entry(
    db: &Database,
    request_id: &str,
    delta: &DeltaTokens,
    model: &str,
    session_id: Option<&str>,
    timestamp: Option<&str>,
) -> Result<bool, AppError> {
    let conn = lock_conn!(db.conn);

    let created_at = timestamp
        .and_then(|ts| {
            chrono::DateTime::parse_from_rfc3339(ts)
                .ok()
                .map(|dt| dt.timestamp())
        })
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        });

    let dedup_key = DedupKey {
        app_type: "codex",
        model,
        input_tokens: delta.input,
        output_tokens: delta.output,
        cache_read_tokens: delta.cached_input,
        cache_creation_tokens: 0,
        created_at,
    };
    if should_skip_session_insert(&conn, request_id, &dedup_key)? {
        return Ok(false);
    }

    // 计算费用
    let usage = TokenUsage {
        input_tokens: delta.input,
        output_tokens: delta.output,
        cache_read_tokens: delta.cached_input,
        cache_creation_tokens: 0,
        model: Some(model.to_string()),
        message_id: None,
    };

    let pricing = find_codex_pricing(&conn, model);
    let multiplier = Decimal::from(1);
    let (input_cost, output_cost, cache_read_cost, cache_creation_cost, total_cost) = match pricing
    {
        Some(p) => {
            let cost = CostCalculator::calculate_for_app("codex", &usage, &p, multiplier);
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
                "_codex_session",    // provider_id
                "codex",             // app_type
                model,
                model,               // request_model = model
                delta.input,
                delta.output,
                delta.cached_input,
                0i64,                // cache_creation_tokens: Codex 日志无此数据
                input_cost,
                output_cost,
                cache_read_cost,
                cache_creation_cost,
                total_cost,
                0i64,                // latency_ms
                Option::<i64>::None, // first_token_ms
                200i64,              // status_code
                Option::<String>::None, // error_message
                session_id.map(|s| s.to_string()),
                Some("codex_session"), // provider_type
                1i64,                // is_streaming
                "1.0",               // cost_multiplier
                created_at,
                "codex_session",     // data_source
            ],
        )
        .map_err(|e| AppError::Database(format!("插入 Codex 会话日志失败: {e}")))?;

    if inserted_rows > 0 {
        crate::usage_events::notify_log_recorded();
    }

    Ok(true)
}

/// 查找 Codex 模型定价（带归一化）
fn find_codex_pricing(conn: &rusqlite::Connection, model_id: &str) -> Option<ModelPricing> {
    find_model_pricing(conn, &normalize_codex_model(model_id))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // task 边界：相同的累计值
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
        // 异常情况：当前值小于前值（不应发生，但需防护）
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
        let files = collect_codex_session_files(Path::new("/nonexistent/path"));
        assert!(files.is_empty());
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
        db.put_usage_sync_cursor(&crate::database::UsageSyncCursor {
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
            cached_input: 80, // delta = 80（异常：大于 input delta）
            output: 60,
        };
        let delta = compute_delta(&prev, &current);
        // 钳制前：cached_input = 80, input = 10
        assert_eq!(delta.cached_input, 80);
        assert_eq!(delta.input, 10);
        // 实际钳制在调用侧：delta.cached_input.min(delta.input)
        let clamped = delta.cached_input.min(delta.input);
        assert_eq!(clamped, 10);
    }
}
