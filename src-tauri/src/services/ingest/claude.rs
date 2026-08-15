//! Claude Code 会话日志解析器。
//!
//! 流水线(盯文件 → 读文件 → 去重 → 写库)在 `ingest/mod.rs`,这里只实现
//! Claude 家的「解析」:从 ~/.claude/projects/ 下的 JSONL 会话文件中提取
//! assistant 消息的 token 使用数据。文件扫描由流水线按 `log_roots` +
//! `is_log_file` 完成,`collect_jsonl_files`(含子 agent / Workflow 目录的
//! 固定深度规则)仅保留给测试对照。

use crate::config::get_claude_config_dir;
use crate::error::AppError;
use crate::services::ingest::{
    occurred_at_secs, sync_with_parser, LogFileContext, ParseOutput, ParsedUsage,
    ProviderWriteProfile, SessionLogParser, SessionSyncResult, UsageIdentity,
};
use crate::store::Database;
use crate::usage::domain::CLAUDE_CODE_AGENT_MODULE_ID;
use crate::usage::metering::parser::SESSION_REQUEST_ID_PREFIX;
use crate::usage::session::{validate_bound_session_agent, ProviderSessionSyncResult};
use crate::usage::system_providers::CLAUDE_SUBSCRIPTION_ID;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[cfg(test)]
use crate::services::ingest::{
    get_sync_state, insert_usage_record, load_sync_cursors, sync_file_with_parser,
    update_sync_state, SyncCursorMap,
};
#[cfg(test)]
use crate::store::lock_conn;
#[cfg(test)]
use crate::store::UsageSyncCursor;
#[cfg(test)]
use crate::usage::domain::TokenSource;
#[cfg(test)]
use crate::usage::ingestion::{UsageIngestionInput, UsageIngestionService};
#[cfg(test)]
use crate::usage::metering::parser::TokenUsage;
#[cfg(test)]
use std::fs;

/// Claude Code 的会话日志解析器。无内部状态。
pub struct ClaudeParser;

impl SessionLogParser for ClaudeParser {
    fn source(&self) -> &'static str {
        "claude"
    }

    fn write_profile(&self) -> ProviderWriteProfile {
        ProviderWriteProfile {
            app_type: "claude",
            legacy_provider_id: "_session",
            provider_type: "session_log",
            insert_error_prefix: "插入会话日志",
            calculator_app: None,
            agent_module_id: Some(CLAUDE_CODE_AGENT_MODULE_ID),
            subscription_activity_id: Some(CLAUDE_SUBSCRIPTION_ID),
        }
    }

    fn log_roots(&self, _home: &Path) -> Vec<PathBuf> {
        vec![get_claude_config_dir().join("projects")]
    }

    fn is_log_file(&self, path: &Path) -> bool {
        path.extension().and_then(|e| e.to_str()) == Some("jsonl")
    }

    // prune 用默认实现:Claude 无剪枝,原样返回。

    fn parse(&self, ctx: &LogFileContext<'_>) -> Result<ParseOutput, AppError> {
        parse_claude_log_file(ctx)
    }
}

/// 同步 Claude Code 会话日志到使用统计数据库
pub fn sync_claude_session_logs(db: &Database) -> Result<SessionSyncResult, AppError> {
    sync_with_parser(db, &ClaudeParser, None)
}

/// Provider-aware entry point used by the v13 usage path. It refuses to scan
/// before an explicit Claude source binding exists and only advances a file's
/// offset after all parsed records have been ingested successfully.
pub fn sync_claude_session_logs_bound(
    db: &Database,
    provider_id: &str,
) -> Result<ProviderSessionSyncResult, AppError> {
    let legacy = db.with_bound_usage_source("claude", provider_id, || {
        validate_bound_session_agent(db, "claude", provider_id)?;
        sync_with_parser(db, &ClaudeParser, Some(provider_id))
    })?;
    let Some(legacy) = legacy else {
        return Ok(ProviderSessionSyncResult {
            warnings: vec!["no usage source binding for claude".to_string()],
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

/// 从 JSONL 中解析出的 assistant 消息使用数据
#[derive(Debug)]
struct ParsedAssistantUsage {
    message_id: String,
    model: String,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_creation_tokens: u32,
    stop_reason: Option<String>,
    timestamp: Option<String>,
    session_id: Option<String>,
    /// 该消息代表行在文件里的行号。
    line_offset: i64,
}

/// 把整个文件解析成归一化用量记录,返回的记录已排除水位线以下的行。
///
/// 跳过发生在**解析之前**(Claude 的位置):水位线以下的行既不提取
/// sessionId 也不产生记录。
fn parse_claude_log_file(ctx: &LogFileContext<'_>) -> Result<ParseOutput, AppError> {
    let mut messages: HashMap<String, ParsedAssistantUsage> = HashMap::new();
    let mut current_session_id: Option<String> = None;
    let mut line_offset: i64 = 0;

    for line in ctx.content.lines() {
        line_offset += 1;

        // 跳过已处理的行
        if line_offset <= ctx.last_line_offset {
            continue;
        }

        if line.trim().is_empty() {
            continue;
        }

        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // 提取 session ID(从 system 或首条消息)
        if current_session_id.is_none() {
            if let Some(sid) = value.get("sessionId").and_then(|v| v.as_str()) {
                current_session_id = Some(sid.to_string());
            }
        }

        // 只处理 assistant 类型的消息
        if value.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }

        let message = match value.get("message") {
            Some(m) => m,
            None => continue,
        };

        let msg_id = match message.get("id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => continue,
        };

        let usage = match message.get("usage") {
            Some(u) => u,
            None => continue,
        };

        let parsed = ParsedAssistantUsage {
            message_id: msg_id.clone(),
            model: message
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            input_tokens: usage
                .get("input_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            output_tokens: usage
                .get("output_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            cache_read_tokens: usage
                .get("cache_read_input_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            cache_creation_tokens: usage
                .get("cache_creation_input_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            stop_reason: message
                .get("stop_reason")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            timestamp: value
                .get("timestamp")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            session_id: current_session_id.clone(),
            line_offset,
        };

        // 按 message.id 去重:优先保留有 stop_reason 的条目,否则保留最新的
        let should_replace = match messages.get(&msg_id) {
            None => true,
            Some(existing) => {
                // 新条目有 stop_reason 而旧条目没有 → 替换
                if parsed.stop_reason.is_some() && existing.stop_reason.is_none() {
                    true
                }
                // 两个都有或都没有 stop_reason → 取 output_tokens 更大的
                else if parsed.stop_reason.is_some() == existing.stop_reason.is_some() {
                    parsed.output_tokens > existing.output_tokens
                } else {
                    false
                }
            }
        };

        if should_replace {
            messages.insert(msg_id, parsed);
        }
    }

    let mut records = Vec::new();
    for msg in messages.values() {
        // 只要产生了真实计费 token 就导入,不再强制要求 stop_reason 或 output>0。
        //
        // Anthropic 在受理请求时即对 input + cache_read + cache_creation 计费
        // (这些在请求开始就确定),output 按实际生成量计。Workflow / 子 agent 的
        // 并行短命请求经常只写了 message_start 快照(output=1、stop_reason=None)
        // 却没有写最终块,但其 cache/input 成本已被真实计费。旧逻辑用 stop_reason
        // 非空 + output>0 双重过滤,会把这类请求整条丢弃,实测系统性低估约 4.1%,
        // 且 92% 集中在 workflow/subagent。这里改为「任一计费维度 > 0 即导入」。
        //
        // 去重选择逻辑(上方按 message.id 取 stop_reason 优先 / output 最大者)保持
        // 不变:它选出的代表行的 input/cache 本就准确;request_id = session:msg_id
        // 主键 + INSERT OR IGNORE 保证一个 message 仍只落库一次,放宽 gate 不会双算。
        let has_billable_tokens = msg.input_tokens > 0
            || msg.output_tokens > 0
            || msg.cache_read_tokens > 0
            || msg.cache_creation_tokens > 0;
        if !has_billable_tokens {
            continue;
        }

        records.push(ParsedUsage {
            identity: UsageIdentity {
                request_id: format!("{SESSION_REQUEST_ID_PREFIX}{}", msg.message_id),
                event_id: Some(format!("claude-session:{}", msg.message_id)),
                upstream_correlation_id: Some(msg.message_id.clone()),
                message_id: Some(msg.message_id.clone()),
                log_label: msg.message_id.clone(),
            },
            model: msg.model.clone(),
            input_tokens: msg.input_tokens,
            output_tokens: msg.output_tokens,
            cache_read_tokens: msg.cache_read_tokens,
            cache_creation_tokens: msg.cache_creation_tokens,
            occurred_at: occurred_at_secs(msg.timestamp.as_deref()),
            session_id: msg.session_id.clone(),
            line_offset: msg.line_offset,
            upstream_total_cost: None,
        });
    }

    Ok(ParseOutput {
        records,
        next_state: None,
    })
}

/// 旧签名的测试适配器:把内部表示转成 ParsedUsage 后走统一写库。
/// 生产路径已由流水线的 `insert_usage_record` 接管;这个函数只服务于
/// 保留原断言不变的旧测试。
#[cfg(test)]
fn insert_session_log_entry(
    db: &Database,
    request_id: &str,
    msg: &ParsedAssistantUsage,
) -> Result<bool, AppError> {
    insert_usage_record(
        db,
        &ClaudeParser.write_profile(),
        &ParsedUsage {
            identity: UsageIdentity {
                request_id: format!("{SESSION_REQUEST_ID_PREFIX}{}", msg.message_id),
                event_id: Some(format!("claude-session:{}", msg.message_id)),
                upstream_correlation_id: Some(msg.message_id.clone()),
                message_id: Some(msg.message_id.clone()),
                log_label: msg.message_id.clone(),
            },
            model: msg.model.clone(),
            input_tokens: msg.input_tokens,
            output_tokens: msg.output_tokens,
            cache_read_tokens: msg.cache_read_tokens,
            cache_creation_tokens: msg.cache_creation_tokens,
            occurred_at: occurred_at_secs(msg.timestamp.as_deref()),
            session_id: msg.session_id.clone(),
            line_offset: msg.line_offset,
            upstream_total_cost: None,
        },
        request_id,
        None,
    )
}

#[cfg(test)]
fn sync_single_file(
    db: &Database,
    file_path: &Path,
    bound_provider_id: Option<&str>,
) -> Result<(u32, u32), AppError> {
    let cursor_list = db.list_usage_sync_cursors("claude")?;
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
    let profile = ClaudeParser.write_profile();
    let mut errors = Vec::new();
    sync_file_with_parser(
        db,
        &ClaudeParser,
        &profile,
        file_path,
        bound_provider_id,
        &mut sync_cursors,
        &cursor_details,
        &mut errors,
    )
}

/// 收集目录下所有 .jsonl 文件(含子 agent 文件)。
///
/// 保留给测试对照:生产路径由流水线按 `log_roots` + `is_log_file` 递归收集,
/// 结构规则相同(主会话 / subagents / workflows)。
///
/// 扫描固定深度,不使用递归,避免死循环:
///   projects_dir/项目目录/*.jsonl                                      (主会话)
///   projects_dir/项目目录/SESSION_ID/subagents/*.jsonl                  (Task/Agent 子 agent)
///   projects_dir/项目目录/SESSION_ID/subagents/workflows/wf_*/*.jsonl   (Workflow 子 agent)
///
/// 最后一层是 Claude Code Workflow 功能产生的子 agent transcript,比普通子
/// agent 多嵌套一层 `workflows/wf_<ID>/`。漏掉这一层会让 Workflow 的 token
/// 用量完全不计入统计;`journal.jsonl` 不含 `type=="assistant"` 行,解析时
/// 会被 `sync_single_file` 天然跳过,因此这里无需按文件名过滤。
#[cfg(test)]
fn collect_jsonl_files(projects_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();

    let entries = match fs::read_dir(projects_dir) {
        Ok(e) => e,
        Err(_) => return files,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // 每个项目目录下的 .jsonl 文件
        if let Ok(sub_entries) = fs::read_dir(&path) {
            for sub_entry in sub_entries.flatten() {
                let sub_path = sub_entry.path();
                if sub_path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                    // 主会话 JSONL 文件
                    files.push(sub_path);
                } else if sub_path.is_dir() {
                    // 扫描子 agent 目录: 项目/SESSION_ID/subagents/*.jsonl
                    let subagents_dir = sub_path.join("subagents");
                    if subagents_dir.is_dir() {
                        push_jsonl_children(&subagents_dir, &mut files);

                        // 额外下探 Workflow 子 agent:
                        // 项目/SESSION_ID/subagents/workflows/wf_<ID>/*.jsonl
                        let workflows_dir = subagents_dir.join("workflows");
                        if workflows_dir.is_dir() {
                            if let Ok(wf_entries) = fs::read_dir(&workflows_dir) {
                                for wf_entry in wf_entries.flatten() {
                                    let wf_path = wf_entry.path();
                                    if wf_path.is_dir() {
                                        push_jsonl_children(&wf_path, &mut files);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    files
}

/// 将 `dir` 下直接子层的所有 `.jsonl` 文件追加到 `files`(不递归)。
#[cfg(test)]
fn push_jsonl_children(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                files.push(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_state_round_trips_through_v14_cursor_table() -> Result<(), AppError> {
        let db = Database::memory()?;
        let cursor_key = "/tmp/claude-session.jsonl";

        assert_eq!(get_sync_state(&db, "claude", cursor_key)?, (0, 0));
        update_sync_state(&db, "claude", cursor_key, 42, 7)?;

        assert_eq!(get_sync_state(&db, "claude", cursor_key)?, (42, 7));
        let cursor = db
            .get_usage_sync_cursor("claude", cursor_key)?
            .expect("sync state must be stored in the v14 cursor table");
        assert_eq!(cursor.resource_path.as_deref(), Some(cursor_key));
        let conn = lock_conn!(db.conn);
        assert!(
            !Database::table_exists(&conn, "session_log_sync")?,
            "v14 session sync must not recreate the retired table"
        );

        Ok(())
    }

    #[test]
    fn preloaded_sync_cursors_match_individual_queries() -> Result<(), AppError> {
        let db = Database::memory()?;
        update_sync_state(&db, "claude", "/tmp/first.jsonl", 42, 7)?;
        update_sync_state(&db, "claude", "/tmp/second.jsonl", 84, 11)?;
        update_sync_state(&db, "codex", "/tmp/other-source.jsonl", 99, 13)?;

        let cursors = load_sync_cursors(&db, "claude")?;
        for cursor_key in [
            "/tmp/first.jsonl",
            "/tmp/second.jsonl",
            "/tmp/missing.jsonl",
        ] {
            let preloaded = cursors.get(cursor_key).copied().unwrap_or((0, 0));
            assert_eq!(preloaded, get_sync_state(&db, "claude", cursor_key)?);
        }
        assert!(!cursors.contains_key("/tmp/other-source.jsonl"));

        Ok(())
    }

    #[test]
    fn test_parse_usage_from_jsonl_line() {
        let line = r#"{"type":"assistant","message":{"id":"msg_test123","model":"claude-opus-4-6","usage":{"input_tokens":3,"output_tokens":150,"cache_read_input_tokens":5000,"cache_creation_input_tokens":10000},"stop_reason":"end_turn"},"timestamp":"2026-04-05T12:00:00Z","sessionId":"session-abc"}"#;

        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(
            value.get("type").and_then(|t| t.as_str()),
            Some("assistant")
        );

        let message = value.get("message").unwrap();
        let usage = message.get("usage").unwrap();

        assert_eq!(usage.get("input_tokens").unwrap().as_u64().unwrap(), 3);
        assert_eq!(usage.get("output_tokens").unwrap().as_u64().unwrap(), 150);
        assert_eq!(
            usage
                .get("cache_read_input_tokens")
                .unwrap()
                .as_u64()
                .unwrap(),
            5000
        );
        assert_eq!(
            usage
                .get("cache_creation_input_tokens")
                .unwrap()
                .as_u64()
                .unwrap(),
            10000
        );
        assert_eq!(
            message.get("stop_reason").unwrap().as_str().unwrap(),
            "end_turn"
        );
    }

    #[test]
    fn test_dedup_by_message_id() {
        // 同一个 message.id 有多条,应该取 stop_reason 有值的那条
        let mut messages: HashMap<String, ParsedAssistantUsage> = HashMap::new();

        // 中间条目(无 stop_reason)
        let intermediate = ParsedAssistantUsage {
            message_id: "msg_1".to_string(),
            model: "claude-opus-4-6".to_string(),
            input_tokens: 3,
            output_tokens: 26,
            cache_read_tokens: 5000,
            cache_creation_tokens: 10000,
            stop_reason: None,
            timestamp: Some("2026-04-05T12:00:00Z".to_string()),
            session_id: None,
            line_offset: 0,
        };
        messages.insert("msg_1".to_string(), intermediate);

        // 最终条目(有 stop_reason)
        let final_entry = ParsedAssistantUsage {
            message_id: "msg_1".to_string(),
            model: "claude-opus-4-6".to_string(),
            input_tokens: 3,
            output_tokens: 1349,
            cache_read_tokens: 5000,
            cache_creation_tokens: 10000,
            stop_reason: Some("end_turn".to_string()),
            timestamp: Some("2026-04-05T12:00:00Z".to_string()),
            session_id: None,
            line_offset: 0,
        };

        // 应该替换
        let should_replace = final_entry.stop_reason.is_some()
            && messages.get("msg_1").unwrap().stop_reason.is_none();
        assert!(should_replace);

        messages.insert("msg_1".to_string(), final_entry);
        assert_eq!(messages.get("msg_1").unwrap().output_tokens, 1349);
    }

    #[test]
    fn test_insert_claude_session_skips_matching_proxy_log() -> Result<(), AppError> {
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
                    "proxy-different-id",
                    "openai-compatible",
                    "claude",
                    "claude-sonnet-4-5",
                    "claude-sonnet-4-5",
                    100,
                    20,
                    10,
                    5,
                    "0.10",
                    100,
                    200,
                    1000,
                    "proxy"
                ],
            )?;
        }

        let msg = ParsedAssistantUsage {
            message_id: "msg_1".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            input_tokens: 100,
            output_tokens: 20,
            cache_read_tokens: 10,
            cache_creation_tokens: 5,
            stop_reason: Some("end_turn".to_string()),
            timestamp: Some("1970-01-01T00:16:45Z".to_string()),
            session_id: Some("session-1".to_string()),
            line_offset: 0,
        };

        let inserted = insert_session_log_entry(&db, "session:msg_1", &msg)?;
        assert!(!inserted);

        let conn = lock_conn!(db.conn);
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |row| {
            row.get(0)
        })?;
        assert_eq!(count, 1);

        Ok(())
    }

    #[test]
    fn test_collect_jsonl_files_includes_subagents() {
        let tmp = std::env::temp_dir().join(format!("llm-usage-bar-test-{}", uuid::Uuid::new_v4()));
        let project = tmp.join("project");
        let session_dir = project.join("test-session");
        let subagents_dir = session_dir.join("subagents");
        fs::create_dir_all(&subagents_dir).unwrap();

        fs::write(project.join("main.jsonl"), "{}").unwrap();
        fs::write(subagents_dir.join("agent-abc.jsonl"), "{}").unwrap();

        let files = collect_jsonl_files(&tmp);
        assert_eq!(files.len(), 2);
        let paths: Vec<String> = files
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        assert!(paths.iter().any(|p| p.contains("main.jsonl")));
        assert!(paths.iter().any(|p| p.contains("agent-abc.jsonl")));

        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_collect_jsonl_files_includes_workflow_subagents() {
        // Claude Code Workflow 把子 agent transcript 嵌在
        // 项目/SESSION_ID/subagents/workflows/wf_<ID>/ 下,比普通子 agent 深一层。
        let tmp = std::env::temp_dir().join(format!("llm-usage-bar-test-{}", uuid::Uuid::new_v4()));
        let project = tmp.join("project");
        let session_dir = project.join("test-session");
        let subagents_dir = session_dir.join("subagents");
        let wf_dir = subagents_dir.join("workflows").join("wf_test123");
        fs::create_dir_all(&wf_dir).unwrap();

        fs::write(project.join("main.jsonl"), "{}").unwrap();
        fs::write(subagents_dir.join("agent-plain.jsonl"), "{}").unwrap();
        fs::write(wf_dir.join("agent-wf.jsonl"), "{}").unwrap();
        // journal.jsonl 也会被收集,但解析时因无 assistant 行而产出 0 条
        fs::write(wf_dir.join("journal.jsonl"), "{}").unwrap();

        let files = collect_jsonl_files(&tmp);
        let paths: Vec<String> = files
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        // 主会话 + 普通子 agent + Workflow 子 agent(agent-wf + journal) = 4
        assert_eq!(files.len(), 4);
        assert!(paths.iter().any(|p| p.contains("main.jsonl")));
        assert!(paths.iter().any(|p| p.contains("agent-plain.jsonl")));
        assert!(
            paths.iter().any(|p| p.contains("agent-wf.jsonl")),
            "Workflow 子 agent transcript 必须被收集"
        );

        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_sync_imports_billable_message_without_stop_reason() -> Result<(), AppError> {
        // 回归:stop_reason 缺失但有真实 cache/input 成本的 message(Workflow /
        // 子 agent 常见的「只有 message_start 快照、没写最终块」形态)必须被计入,
        // 不能因缺 stop_reason 或 output==0 而整条丢弃;全 0 token 的占位行仍应跳过。
        let db = Database::memory()?;
        let tmp = std::env::temp_dir().join(format!("llm-usage-bar-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("agent-wf.jsonl");

        // 第一行:无 stop_reason、output=1,但 cache_read/cache_creation 很大 → 应导入
        // 第二行:全部 token 为 0 → 应跳过(无计费意义)
        let billable = r#"{"type":"assistant","message":{"id":"msg_nostop","model":"claude-opus-4-8","usage":{"input_tokens":2,"output_tokens":1,"cache_read_input_tokens":48719,"cache_creation_input_tokens":2061}},"timestamp":"2026-06-07T13:01:23Z","sessionId":"session-wf"}"#;
        let empty = r#"{"type":"assistant","message":{"id":"msg_empty","model":"claude-opus-4-8","usage":{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}},"timestamp":"2026-06-07T13:01:24Z","sessionId":"session-wf"}"#;
        fs::write(&file, format!("{billable}\n{empty}\n")).unwrap();

        let (imported, _skipped) = sync_single_file(&db, &file, None)?;
        assert_eq!(
            imported, 1,
            "有 cache 成本但无 stop_reason 的 message 必须被导入"
        );

        let conn = lock_conn!(db.conn);
        let cache_read: i64 = conn.query_row(
            "SELECT cache_read_tokens FROM proxy_request_logs WHERE request_id = 'session:msg_nostop'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(cache_read, 48719, "cache_read 必须被完整记录");
        let empty_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM proxy_request_logs WHERE request_id = 'session:msg_empty')",
            [],
            |row| row.get(0),
        )?;
        assert!(!empty_exists, "全 0 token 的 message 应被跳过");
        drop(conn);

        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    #[test]
    fn bound_sync_advances_offset_only_after_full_file_ingestion() -> Result<(), AppError> {
        let db = Database::memory()?;
        db.save_usage_provider(&crate::usage::domain::UsageProviderInput {
            id: "claude-sub".to_string(),
            name: "Claude subscription".to_string(),
            billing_kind: crate::usage::domain::BillingKind::Subscription,
            product_group_id: "claude".to_string(),
            token_sources: vec![crate::usage::domain::TokenSource::SessionLog],
            session_source_bindings: None,
            quota_source: None,
            quota_interval_seconds: Some(300),
            route_app_type: None,
            route_config: None,
            quota_config: None,
            enabled: true,
        })?;
        db.set_usage_source_binding("claude", "claude-sub")?;
        db.save_agent_provider_binding(&crate::usage::domain::AgentProviderBindingInput {
            id: None,
            agent_module_id: "claude-code".to_string(),
            provider_id: "claude-sub".to_string(),
            enabled: true,
        })?;

        let tmp = std::env::temp_dir().join(format!(
            "llm-usage-bar-bound-session-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("session.jsonl");
        let billable = r#"{"type":"assistant","message":{"id":"msg_bound","model":"claude-sonnet-4-5","usage":{"input_tokens":2,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0},"stop_reason":"end_turn"},"timestamp":"2026-06-07T13:01:23Z","sessionId":"session-bound"}"#;
        fs::write(&file, format!("{billable}\n")).unwrap();
        let file_key = file.to_string_lossy().to_string();

        {
            let conn = lock_conn!(db.conn);
            conn.execute_batch(
                "CREATE TRIGGER fail_bound_session_event
                 BEFORE INSERT ON usage_events
                 BEGIN SELECT RAISE(FAIL, 'forced bound session failure'); END",
            )?;
        }
        assert!(sync_single_file(&db, &file, Some("claude-sub")).is_err());
        assert_eq!(get_sync_state(&db, "claude", &file_key)?, (0, 0));

        {
            let conn = lock_conn!(db.conn);
            conn.execute_batch("DROP TRIGGER fail_bound_session_event;")?;
        }
        let (imported, skipped) = sync_single_file(&db, &file, Some("claude-sub"))?;
        assert_eq!((imported, skipped), (1, 0));
        assert_eq!(get_sync_state(&db, "claude", &file_key)?.1, 1);

        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }

    #[test]
    fn bound_claude_parser_links_exact_raw_message_id_from_proxy() -> Result<(), AppError> {
        let db = Database::memory()?;
        db.save_usage_provider(&crate::usage::domain::UsageProviderInput {
            id: "claude-sub".to_string(),
            name: "Claude subscription".to_string(),
            billing_kind: crate::usage::domain::BillingKind::Subscription,
            product_group_id: "claude".to_string(),
            token_sources: vec![
                crate::usage::domain::TokenSource::Proxy,
                crate::usage::domain::TokenSource::SessionLog,
            ],
            session_source_bindings: None,
            quota_source: None,
            quota_interval_seconds: Some(300),
            route_app_type: None,
            route_config: None,
            quota_config: None,
            enabled: true,
        })?;
        db.set_usage_source_binding("claude", "claude-sub")?;
        for agent_module_id in ["claude-code", "codex"] {
            let binding =
                db.save_agent_provider_binding(&crate::usage::domain::AgentProviderBindingInput {
                    id: None,
                    agent_module_id: agent_module_id.to_string(),
                    provider_id: "claude-sub".to_string(),
                    enabled: false,
                })?;
            let conn = lock_conn!(db.conn);
            conn.execute(
                "UPDATE agent_provider_bindings SET enabled = 1 WHERE id = ?1",
                [&binding.id],
            )?;
        }

        UsageIngestionService::new(&db).ingest(&UsageIngestionInput {
            event_id: "proxy-event".to_string(),
            source: TokenSource::Proxy,
            provider_id: "claude-sub".to_string(),
            agent_module_id: "claude-code".to_string(),
            frozen_provider_context: Some(crate::usage::ingestion::FrozenUsageProviderContext {
                product_group_id: "claude".to_string(),
                route_app_type: "claude".to_string(),
            }),
            occurred_at: 1_000,
            model: "claude-sonnet-4-5".to_string(),
            usage: TokenUsage {
                input_tokens: 2,
                output_tokens: 1,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                model: Some("claude-sonnet-4-5".to_string()),
                message_id: Some("msg_exact".to_string()),
            },
            upstream_cost: None,
            request_id: None,
            session_id: None,
            upstream_correlation_id: Some("msg_exact".to_string()),
            legacy: None,
        })?;

        let tmp = std::env::temp_dir().join(format!(
            "llm-usage-bar-bound-link-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("session.jsonl");
        let billable = r#"{"type":"assistant","message":{"id":"msg_exact","model":"claude-sonnet-4-5","usage":{"input_tokens":2,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0},"stop_reason":"end_turn"},"timestamp":"1970-01-01T00:16:40Z","sessionId":"conversation-wide"}"#;
        fs::write(&file, format!("{billable}\n")).unwrap();

        assert_eq!(sync_single_file(&db, &file, Some("claude-sub"))?, (1, 0));
        let conn = lock_conn!(db.conn);
        let link: (String, String, String, String) = conn.query_row(
            "SELECT link.canonical_event_id, link.duplicate_event_id,
                    link.link_kind, link.link_value
             FROM usage_event_links AS link",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        assert_eq!(
            link,
            (
                "proxy-event".to_string(),
                "claude-session:msg_exact".to_string(),
                "upstream_correlation_id".to_string(),
                "msg_exact".to_string()
            )
        );
        let agent_module_id: Option<String> = conn.query_row(
            "SELECT agent_module_id FROM usage_events
             WHERE event_id = 'claude-session:msg_exact'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(agent_module_id.as_deref(), Some("claude-code"));
        drop(conn);

        fs::remove_dir_all(&tmp).ok();
        Ok(())
    }
}
