//! Gemini CLI 会话日志解析器。
//!
//! 流水线(盯文件 → 读文件 → 去重 → 写库)在 `ingest/mod.rs`,这里只实现
//! Gemini 家的「解析」:从 `~/.gemini/tmp/<project_hash>/chats/session-*.json`
//! (每个文件是单个 JSON 对象,内含 messages 数组)提取 per-message 的
//! token 用量。tokens 是逐条独立的,不需要 delta,也不需要跨行状态。
//!
//! 文件收集沿用旧实现的路径结构:`tmp/<project>/chats/` 的直接子文件,
//! 由 `log_roots`(tmp 目录)+ `is_log_file`(校验父目录链)复刻,递归扫描
//! 不会把 chats 子目录里的 session-*.json 算进来。

use crate::agent_paths::get_gemini_dir;
use crate::database::Database;
use crate::error::AppError;
use crate::services::ingest::{
    occurred_at_secs, sync_with_parser, LogFileContext, ParseOutput, ParsedUsage,
    ProviderWriteProfile, SessionLogParser, SessionSyncResult, UsageIdentity,
};
use std::path::{Path, PathBuf};

#[cfg(test)]
use crate::database::lock_conn;
#[cfg(test)]
use crate::services::ingest::insert_usage_record;
#[cfg(test)]
use std::fs;

/// 从 Gemini message 中提取的 token 数据
#[derive(Debug)]
struct GeminiTokens {
    input: u32,
    output: u32,
    cached: u32,
    thoughts: u32,
}

/// Gemini 会话日志解析器。无内部状态,只记住 gemini 配置目录用于文件收集。
pub struct GeminiParser {
    gemini_dir: PathBuf,
}

impl GeminiParser {
    pub fn new(gemini_dir: PathBuf) -> Self {
        Self { gemini_dir }
    }

    fn tmp_dir(&self) -> PathBuf {
        self.gemini_dir.join("tmp")
    }
}

impl SessionLogParser for GeminiParser {
    fn source(&self) -> &'static str {
        "gemini"
    }

    fn write_profile(&self) -> ProviderWriteProfile {
        ProviderWriteProfile {
            app_type: "gemini",
            legacy_provider_id: "_gemini_session",
            provider_type: "gemini_session",
            insert_error_prefix: "插入 Gemini 会话日志",
            calculator_app: Some("gemini"),
            agent_module_id: None,
            subscription_activity_id: None,
        }
    }

    fn log_roots(&self, _home: &Path) -> Vec<PathBuf> {
        vec![self.tmp_dir()]
    }

    fn is_log_file(&self, path: &Path) -> bool {
        // 旧实现只读 tmp/<project>/chats/ 的直接子文件,不递归。用父目录链
        // 精确复刻:文件名形如 session-*.json、父目录名必须是 chats、
        // chats 的父目录的父目录必须是 tmp。
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            return false;
        };
        if !(file_name.starts_with("session-") && file_name.ends_with(".json")) {
            return false;
        }
        let Some(chats) = path.parent() else {
            return false;
        };
        if chats.file_name().and_then(|name| name.to_str()) != Some("chats") {
            return false;
        }
        let Some(project) = chats.parent() else {
            return false;
        };
        project.parent() == Some(self.tmp_dir().as_path())
    }

    fn parse(&self, ctx: &LogFileContext<'_>) -> Result<ParseOutput, AppError> {
        parse_gemini_session_file(ctx)
    }

    fn record_file_error(&self, path: &Path, error: &AppError, errors: &mut Vec<String>) {
        let msg = format!("Gemini 会话文件解析失败 {}: {error}", path.display());
        log::warn!("[GEMINI-SYNC] {msg}");
        errors.push(msg);
    }

    fn log_insert_failure(&self, record: &ParsedUsage, error: &AppError) -> Option<String> {
        log::warn!(
            "[GEMINI-SYNC] 插入失败 ({}): {error}",
            record.identity.log_label
        );
        None
    }

    fn log_summary(&self, result: &SessionSyncResult) {
        log::info!(
            "[GEMINI-SYNC] 同步完成: 导入 {} 条, 跳过 {} 条, 扫描 {} 个文件",
            result.imported,
            result.skipped,
            result.files_scanned
        );
    }
}

/// 同步 Gemini 使用数据(从 JSON 会话日志)
pub fn sync_gemini_usage(db: &Database) -> Result<SessionSyncResult, AppError> {
    sync_with_parser(db, &GeminiParser::new(get_gemini_dir()), None)
}

/// 把整个文件解析成归一化用量记录。
///
/// Gemini 的 JSON 文件是「整文件重读 + 按 request_id 去重」,旧实现不读
/// 游标水位线,这里同样忽略 `last_line_offset`:每轮全量产出,重复的由
/// 流水线去重短路。
fn parse_gemini_session_file(ctx: &LogFileContext<'_>) -> Result<ParseOutput, AppError> {
    let value: serde_json::Value = serde_json::from_str(ctx.content)
        .map_err(|e| AppError::Config(format!("JSON 解析失败: {e}")))?;

    // 提取顶层 sessionId
    let session_id = value
        .get("sessionId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // 遍历 messages 数组
    let messages = match value.get("messages").and_then(|v| v.as_array()) {
        Some(msgs) => msgs,
        None => {
            return Ok(ParseOutput {
                records: Vec::new(),
                next_state: None,
            })
        }
    };

    let mut records = Vec::new();
    for msg in messages {
        // 只处理 type == "gemini" 的消息
        if msg.get("type").and_then(|t| t.as_str()) != Some("gemini") {
            continue;
        }

        // 提取 tokens 对象
        let tokens_obj = match msg.get("tokens") {
            Some(t) if t.is_object() => t,
            _ => continue,
        };

        let tokens = parse_gemini_tokens(tokens_obj);
        if tokens.input == 0 && tokens.output == 0 && tokens.thoughts == 0 && tokens.cached == 0 {
            continue; // 跳过全零的空 token 消息
        }

        // 提取消息 ID 和模型
        let message_id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
        let model = msg
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let timestamp = msg.get("timestamp").and_then(|v| v.as_str());

        // 生成唯一 request_id
        let session_id_str = session_id.as_deref().unwrap_or("unknown");
        let request_id = format!("gemini_session:{session_id_str}:{message_id}");

        records.push(ParsedUsage {
            identity: UsageIdentity {
                request_id: request_id.clone(),
                event_id: None,
                upstream_correlation_id: None,
                message_id: None,
                log_label: request_id,
            },
            model: model.to_string(),
            input_tokens: tokens.input,
            // 合并 thoughts 到 output(思考 token 按输出计费)
            output_tokens: tokens.output + tokens.thoughts,
            cache_read_tokens: tokens.cached,
            cache_creation_tokens: 0,
            occurred_at: occurred_at_secs(timestamp),
            session_id: session_id.clone(),
            line_offset: 1,
            upstream_total_cost: None,
        });
    }

    Ok(ParseOutput {
        records,
        next_state: None,
    })
}

/// 从 tokens JSON 对象中提取 token 数据
fn parse_gemini_tokens(tokens: &serde_json::Value) -> GeminiTokens {
    GeminiTokens {
        input: tokens.get("input").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        output: tokens.get("output").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        cached: tokens.get("cached").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        thoughts: tokens.get("thoughts").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
    }
}

/// 旧签名的测试适配器:把内部表示转成 ParsedUsage 后走统一写库。
/// 生产路径已由流水线的 `insert_usage_record` 接管;这个函数只服务于
/// 保留原断言不变的旧测试。
#[cfg(test)]
fn insert_gemini_session_entry(
    db: &Database,
    request_id: &str,
    tokens: &GeminiTokens,
    model: &str,
    session_id: Option<&str>,
    timestamp: Option<&str>,
) -> Result<bool, AppError> {
    insert_usage_record(
        db,
        &GeminiParser::new(PathBuf::from("/unused")).write_profile(),
        &ParsedUsage {
            identity: UsageIdentity {
                request_id: request_id.to_string(),
                event_id: None,
                upstream_correlation_id: None,
                message_id: None,
                log_label: request_id.to_string(),
            },
            model: model.to_string(),
            input_tokens: tokens.input,
            output_tokens: tokens.output + tokens.thoughts,
            cache_read_tokens: tokens.cached,
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

/// 收集所有 Gemini 会话 JSON 文件。
///
/// 保留给测试对照:生产路径由流水线按 `log_roots` + `is_log_file` 收集,
/// 结构规则相同(tmp/<project_hash>/chats/session-*.json)。
#[cfg(test)]
fn collect_gemini_session_files(gemini_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();

    let tmp_dir = gemini_dir.join("tmp");
    if !tmp_dir.is_dir() {
        return files;
    }

    // 遍历 tmp/<project_hash>/chats/session-*.json
    let project_dirs = match fs::read_dir(&tmp_dir) {
        Ok(entries) => entries,
        Err(_) => return files,
    };

    for entry in project_dirs.flatten() {
        let chats_dir = entry.path().join("chats");
        if !chats_dir.is_dir() {
            continue;
        }

        let chat_files = match fs::read_dir(&chats_dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for file_entry in chat_files.flatten() {
            let path = file_entry.path();
            let is_session = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("session-") && n.ends_with(".json"))
                .unwrap_or(false);
            if is_session {
                files.push(path);
            }
        }
    }

    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_gemini_session_files_nonexistent() {
        let files = collect_gemini_session_files(Path::new("/nonexistent/path"));
        assert!(files.is_empty());
    }

    #[test]
    fn test_insert_gemini_session_skips_matching_proxy_log() -> Result<(), AppError> {
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
                    "gemini-proxy",
                    "google",
                    "gemini",
                    "gemini-2.5-pro",
                    "gemini-2.5-pro",
                    10,
                    7,
                    1,
                    0,
                    "0.01",
                    100,
                    200,
                    1000,
                    "proxy"
                ],
            )?;
        }

        let tokens = GeminiTokens {
            input: 10,
            output: 2,
            cached: 1,
            thoughts: 5,
        };
        let inserted = insert_gemini_session_entry(
            &db,
            "gemini-session-dup",
            &tokens,
            "gemini-2.5-pro",
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
    fn test_parse_gemini_tokens() {
        let json: serde_json::Value = serde_json::json!({
            "input": 8522,
            "output": 29,
            "cached": 3138,
            "thoughts": 405,
            "tool": 0,
            "total": 8956
        });
        let tokens = parse_gemini_tokens(&json);
        assert_eq!(tokens.input, 8522);
        assert_eq!(tokens.output, 29);
        assert_eq!(tokens.cached, 3138);
        assert_eq!(tokens.thoughts, 405);
        // output + thoughts = 29 + 405 = 434（用于计费）
        assert_eq!(tokens.output + tokens.thoughts, 434);
    }

    #[test]
    fn test_parse_gemini_tokens_missing_fields() {
        // 缺少某些字段时应返回 0
        let json: serde_json::Value = serde_json::json!({
            "input": 100,
            "output": 50
        });
        let tokens = parse_gemini_tokens(&json);
        assert_eq!(tokens.input, 100);
        assert_eq!(tokens.output, 50);
        assert_eq!(tokens.cached, 0);
        assert_eq!(tokens.thoughts, 0);
    }

    #[test]
    fn test_parse_gemini_tokens_all_zero() {
        let json: serde_json::Value = serde_json::json!({
            "input": 0,
            "output": 0,
            "cached": 0,
            "thoughts": 0,
            "tool": 0,
            "total": 0
        });
        let tokens = parse_gemini_tokens(&json);
        assert_eq!(tokens.input, 0);
        assert_eq!(tokens.output, 0);
        // 全零（包括 cached=0）会被 sync 逻辑跳过
        assert!(
            tokens.input == 0 && tokens.output == 0 && tokens.thoughts == 0 && tokens.cached == 0
        );
    }

    #[test]
    fn test_parse_gemini_tokens_cache_only_not_skipped() {
        // 纯缓存命中消息（input/output/thoughts=0 但 cached>0）不应被跳过
        let json: serde_json::Value = serde_json::json!({
            "input": 0,
            "output": 0,
            "cached": 5000,
            "thoughts": 0
        });
        let tokens = parse_gemini_tokens(&json);
        assert_eq!(tokens.cached, 5000);
        // 跳过条件：所有四个字段都为 0 才跳过
        let should_skip =
            tokens.input == 0 && tokens.output == 0 && tokens.thoughts == 0 && tokens.cached == 0;
        assert!(!should_skip, "纯缓存命中记录不应被跳过");
    }
}
