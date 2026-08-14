//! OpenCode 会话日志解析器。
//!
//! 流水线(盯文件 → 读文件 → 去重 → 写库)在 `ingest/mod.rs`,这里只实现
//! OpenCode 家的「解析」:它的「日志」是一个 SQLite 库
//! (`~/.local/share/opencode/opencode.db`),所以:
//!
//! - `needs_file_content` 返回 false —— 流水线不把整个 .db 读进内存,
//!   parse 自己开只读连接查表(旧实现也从不读文件内容);
//! - `extra_change_sources` 把 `-wal` 的 mtime 纳进变更判定 —— WAL 模式下
//!   新提交先落 -wal,主库 mtime 只在 checkpoint 时变;
//! - 会话级水位存在 `parser_state`(JSON 对象 {session_id: watermark}),
//!   只有水位之后的会话才产出记录;
//! - `retry_file_on_insert_failure` 为 true —— 旧实现也是「整轮无错才推进
//!   文件级游标」,文件级重试与它等价(去重按 request_id 幂等)。

use crate::agent_paths::get_opencode_db_path;
use crate::database::Database;
use crate::error::AppError;
use crate::services::ingest::{
    current_timestamp, sync_with_parser, LogFileContext, ParseOutput, ParsedUsage,
    ProviderWriteProfile, SessionLogParser, SessionSyncResult, UsageIdentity,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 从 opencode message.data JSON 中提取的 token 和费用数据
struct OpenCodeMessageData {
    input_tokens: u32,
    output_tokens: u32,
    reasoning_tokens: u32,
    cache_read_tokens: u32,
    cache_write_tokens: u32,
    cost: f64,
    model_id: String,
    timestamp_ms: i64,
}

struct OpenCodeMessageQueryResult {
    messages: Vec<(String, OpenCodeMessageData)>,
    has_incomplete_usage: bool,
}

/// OpenCode 会话日志解析器。无内部状态,只记住数据库路径。
pub struct OpenCodeParser {
    db_path: PathBuf,
}

impl OpenCodeParser {
    pub fn new(db_path: PathBuf) -> Self {
        Self { db_path }
    }
}

impl SessionLogParser for OpenCodeParser {
    fn source(&self) -> &'static str {
        "opencode"
    }

    fn write_profile(&self) -> ProviderWriteProfile {
        ProviderWriteProfile {
            app_type: "opencode",
            legacy_provider_id: "_opencode_session",
            provider_type: "opencode_session",
            insert_error_prefix: "插入 OpenCode 会话日志",
            calculator_app: Some("opencode"),
            agent_module_id: None,
            subscription_activity_id: None,
        }
    }

    fn log_roots(&self, _home: &Path) -> Vec<PathBuf> {
        self.db_path
            .parent()
            .map(|parent| vec![parent.to_path_buf()])
            .unwrap_or_default()
    }

    fn is_log_file(&self, path: &Path) -> bool {
        path == self.db_path
    }

    fn extra_change_sources(&self, _path: &Path) -> Vec<PathBuf> {
        // opencode 的数据库运行在 WAL 模式:新提交先落在 -wal 文件里,
        // 主库文件只有在 checkpoint 时才更新。必须把 -wal 的 mtime 算进
        // 变更判定,否则会在 checkpoint 之前漏掉刚写入的会话。
        vec![self.db_path.with_extension("db-wal")]
    }

    fn needs_file_content(&self) -> bool {
        false
    }

    fn retry_file_on_insert_failure(&self) -> bool {
        true
    }

    fn parse(&self, ctx: &LogFileContext<'_>) -> Result<ParseOutput, AppError> {
        parse_opencode_database(ctx)
    }

    fn log_insert_failure(&self, record: &ParsedUsage, error: &AppError) -> Option<String> {
        let msg = format!(
            "OpenCode 消息插入失败 {}: {error}",
            record.identity.log_label
        );
        log::warn!("[OPENCODE-SYNC] {msg}");
        Some(msg)
    }

    fn log_summary(&self, result: &SessionSyncResult) {
        log::info!(
            "[OPENCODE-SYNC] 同步完成: 导入 {} 条, 跳过 {} 条, 扫描 {} 个文件",
            result.imported,
            result.skipped,
            result.files_scanned
        );
    }
}

/// 同步 OpenCode 使用数据
pub fn sync_opencode_usage(db: &Database) -> Result<SessionSyncResult, AppError> {
    sync_with_parser(db, &OpenCodeParser::new(get_opencode_db_path()), None)
}

/// 把整个 opencode.db 解析成归一化用量记录。
///
/// 会话级水位从 `ctx.parser_state` 载入(JSON 对象 {session_id: watermark});
/// 只有水位之后的会话才查询消息并产出记录;整轮算出的新水位经
/// `ParseOutput.next_state` 写回。某个会话查询失败时跳过该会话且不推进
/// 它的水位,下轮数据库再有变化时重试(错误串进不了 result.errors,
/// 与旧实现一致的是「哪些记录入库」)。
fn parse_opencode_database(ctx: &LogFileContext<'_>) -> Result<ParseOutput, AppError> {
    // 打开 opencode 的 SQLite 数据库(只读)。ctx.file 的句柄只供流水线做
    // 变更判定;这里另开连接查表。
    let opencode_conn =
        rusqlite::Connection::open_with_flags(ctx.path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| AppError::Database(format!("无法打开 opencode.db: {e}")))?;

    let mut watermarks: HashMap<String, i64> = match ctx.parser_state {
        Some(state) => serde_json::from_str(state).unwrap_or_default(),
        None => HashMap::new(),
    };

    // 查询所有会话
    let sessions = query_sessions(&opencode_conn)?;

    let mut records = Vec::new();
    for (session_id, time_updated) in &sessions {
        // 检查会话是否需要重新同步
        if watermarks
            .get(session_id)
            .is_some_and(|watermark| *time_updated <= *watermark)
        {
            continue; // 会话未更新,跳过
        }

        // 查询该会话的所有 assistant 消息
        let query_result = match query_assistant_messages(&opencode_conn, session_id) {
            Ok(query_result) => query_result,
            Err(e) => {
                // 坏会话只跳过、水位不推进,下轮重试;错误串进不了
                // result.errors(报告里说明的差异②)。
                log::warn!("[OPENCODE-SYNC] OpenCode 会话消息查询失败 {session_id}: {e}");
                continue;
            }
        };

        for (message_id, msg_data) in &query_result.messages {
            let request_id = format!("opencode_session:{session_id}:{message_id}");

            records.push(ParsedUsage {
                identity: UsageIdentity {
                    request_id: request_id.clone(),
                    event_id: None,
                    upstream_correlation_id: None,
                    message_id: None,
                    log_label: request_id,
                },
                model: msg_data.model_id.clone(),
                input_tokens: msg_data.input_tokens,
                // output 包含 reasoning tokens(按输出计费)
                output_tokens: msg_data.output_tokens + msg_data.reasoning_tokens,
                cache_read_tokens: msg_data.cache_read_tokens,
                cache_creation_tokens: msg_data.cache_write_tokens,
                occurred_at: if msg_data.timestamp_ms > 0 {
                    msg_data.timestamp_ms / 1000
                } else {
                    current_timestamp()
                },
                session_id: Some(session_id.clone()),
                line_offset: 0,
                upstream_total_cost: (msg_data.cost > 0.0).then(|| msg_data.cost.to_string()),
            });
        }

        // 有半截消息的会话不推进水位:下轮补到 time.completed 后再收。
        if !query_result.has_incomplete_usage {
            watermarks.insert(session_id.clone(), *time_updated);
        }
    }

    let next_state = serde_json::to_string(&watermarks).ok();
    Ok(ParseOutput {
        records,
        next_state,
    })
}

/// 查询所有会话的 (id, sync_watermark)
fn query_sessions(conn: &rusqlite::Connection) -> Result<Vec<(String, i64)>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT s.id,
                    MAX(s.time_updated, COALESCE(MAX(m.time_updated), s.time_updated)) AS sync_watermark
             FROM session s
             LEFT JOIN message m ON m.session_id = s.id
             GROUP BY s.id
             ORDER BY sync_watermark",
        )
        .map_err(|e| AppError::Database(format!("准备会话查询失败: {e}")))?;

    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|e| AppError::Database(format!("查询会话失败: {e}")))?;

    let mut sessions = Vec::new();
    for row in rows {
        sessions.push(row.map_err(|e| AppError::Database(format!("读取会话行失败: {e}")))?);
    }

    Ok(sessions)
}

/// 查询某会话的已完成 assistant 消息,并标记是否还有未完成 usage 消息。
fn query_assistant_messages(
    conn: &rusqlite::Connection,
    session_id: &str,
) -> Result<OpenCodeMessageQueryResult, AppError> {
    let mut stmt = conn
        .prepare("SELECT id, data FROM message WHERE session_id = ?1 ORDER BY time_created")
        .map_err(|e| AppError::Database(format!("准备消息查询失败: {e}")))?;

    let rows = stmt
        .query_map([session_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| AppError::Database(format!("查询消息失败: {e}")))?;

    let mut messages = Vec::new();
    let mut has_incomplete_usage = false;
    for row in rows {
        let (message_id, data_json) =
            row.map_err(|e| AppError::Database(format!("读取消息行失败: {e}")))?;

        // 只处理 assistant 消息
        let value: serde_json::Value = match serde_json::from_str(&data_json) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if value.get("role").and_then(|r| r.as_str()) != Some("assistant") {
            continue;
        }

        // 必须有 tokens 字段
        if value.get("tokens").is_none() {
            continue;
        }

        // 跳过未完成的消息：进行中只有半截 token，且因 INSERT OR IGNORE 无法回填
        if value.get("time").and_then(|t| t.get("completed")).is_none() {
            has_incomplete_usage = true;
            continue;
        }

        if let Some(msg_data) = parse_message_data(&value) {
            messages.push((message_id, msg_data));
        }
    }

    Ok(OpenCodeMessageQueryResult {
        messages,
        has_incomplete_usage,
    })
}

/// 解析 opencode message.data JSON 为结构化数据
fn parse_message_data(value: &serde_json::Value) -> Option<OpenCodeMessageData> {
    let tokens = value.get("tokens")?;

    let input_tokens = tokens.get("input").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let output_tokens = tokens.get("output").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let reasoning_tokens = tokens
        .get("reasoning")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let cache_obj = tokens.get("cache");
    let cache_read_tokens = cache_obj
        .and_then(|c| c.get("read"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let cache_write_tokens = cache_obj
        .and_then(|c| c.get("write"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    // 跳过全零 token 的消息
    if input_tokens == 0
        && output_tokens == 0
        && reasoning_tokens == 0
        && cache_read_tokens == 0
        && cache_write_tokens == 0
    {
        return None;
    }

    let cost = value.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.0);

    let model_id = value
        .get("modelID")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let timestamp_ms = value
        .get("time")
        .and_then(|t| t.get("created"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    Some(OpenCodeMessageData {
        input_tokens,
        output_tokens,
        reasoning_tokens,
        cache_read_tokens,
        cache_write_tokens,
        cost,
        model_id,
        timestamp_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_message_data_full() {
        let json: serde_json::Value = serde_json::json!({
            "role": "assistant",
            "cost": 0.0023113,
            "tokens": {
                "total": 56554,
                "input": 3272,
                "output": 383,
                "reasoning": 419,
                "cache": {
                    "write": 0,
                    "read": 52480
                }
            },
            "modelID": "deepseek-v4-pro",
            "providerID": "deepseek",
            "time": {
                "created": 1779755333700i64,
                "completed": 1779755350639i64
            }
        });
        let data = parse_message_data(&json).unwrap();
        assert_eq!(data.input_tokens, 3272);
        assert_eq!(data.output_tokens, 383);
        assert_eq!(data.reasoning_tokens, 419);
        assert_eq!(data.cache_read_tokens, 52480);
        assert_eq!(data.cache_write_tokens, 0);
        assert!((data.cost - 0.0023113).abs() < 1e-10);
        assert_eq!(data.model_id, "deepseek-v4-pro");
        assert_eq!(data.timestamp_ms, 1779755333700);
    }

    #[test]
    fn test_parse_message_data_missing_cache() {
        let json: serde_json::Value = serde_json::json!({
            "role": "assistant",
            "cost": 0.0,
            "tokens": {
                "input": 1000,
                "output": 200
            },
            "modelID": "mimo-v2.5-pro",
            "time": { "created": 1779755333700i64 }
        });
        let data = parse_message_data(&json).unwrap();
        assert_eq!(data.input_tokens, 1000);
        assert_eq!(data.output_tokens, 200);
        assert_eq!(data.reasoning_tokens, 0);
        assert_eq!(data.cache_read_tokens, 0);
        assert_eq!(data.cache_write_tokens, 0);
    }

    #[test]
    fn test_parse_message_data_skips_zero_tokens() {
        let json: serde_json::Value = serde_json::json!({
            "role": "assistant",
            "tokens": {
                "input": 0,
                "output": 0,
                "reasoning": 0,
                "cache": { "read": 0, "write": 0 }
            },
            "modelID": "test"
        });
        assert!(parse_message_data(&json).is_none());
    }

    #[test]
    fn test_parse_message_data_ignores_role() {
        // parse_message_data does not filter by role; that's the caller's job
        let json: serde_json::Value = serde_json::json!({
            "role": "user",
            "tokens": { "input": 100, "output": 0 }
        });
        let data = parse_message_data(&json).unwrap();
        assert_eq!(data.input_tokens, 100);
    }

    #[test]
    fn test_query_assistant_messages_skips_incomplete() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE message (id TEXT, session_id TEXT, time_created INTEGER, data TEXT);",
        )
        .unwrap();

        let done = serde_json::json!({
            "role": "assistant",
            "tokens": { "input": 1000, "output": 200 },
            "modelID": "m",
            "time": { "created": 1, "completed": 2 }
        })
        .to_string();
        let in_progress = serde_json::json!({
            "role": "assistant",
            "tokens": { "input": 500, "output": 0 },
            "modelID": "m",
            "time": { "created": 3 }
        })
        .to_string();

        conn.execute(
            "INSERT INTO message VALUES ('done', 's1', 1, ?1), ('wip', 's1', 2, ?2)",
            rusqlite::params![done, in_progress],
        )
        .unwrap();

        let result = query_assistant_messages(&conn, "s1").unwrap();
        // 只返回已完成（带 time.completed）的消息，半截的被跳过
        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.messages[0].0, "done");
        assert!(result.has_incomplete_usage);
    }

    #[test]
    fn test_query_sessions_uses_message_update_watermark() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE session (id TEXT, time_updated INTEGER);
             CREATE TABLE message (
                 id TEXT,
                 session_id TEXT,
                 time_created INTEGER,
                 time_updated INTEGER,
                 data TEXT
             );
             INSERT INTO session VALUES ('s1', 100);
             INSERT INTO message VALUES ('m1', 's1', 90, 200, '{}');",
        )
        .unwrap();

        let sessions = query_sessions(&conn).unwrap();
        assert_eq!(sessions, vec![("s1".to_string(), 200)]);
    }
}
