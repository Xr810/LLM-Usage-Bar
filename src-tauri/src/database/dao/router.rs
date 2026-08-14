use crate::database::{lock_conn, Database};
use crate::error::AppError;
use rusqlite::params;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq)]
pub struct RouterProvider {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub wire_api: WireApi,
    pub priority: i64,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireApi {
    Responses,
    ChatCompletions,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelRoute {
    pub provider_id: String,
    pub logical_model: String,
    pub upstream_model: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouterAttempt {
    pub started_at: i64,
    pub logical_model: String,
    pub provider_id: String,
    pub outcome: AttemptOutcome,
    pub failure_kind: Option<String>,
    pub http_status: Option<u16>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptOutcome {
    Success,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouterUsageSummary {
    pub provider_id: String,
    pub attempts: i64,
    pub failures: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

fn now_timestamp() -> Result<i64, AppError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|error| AppError::Database(format!("system clock before unix epoch: {error}")))
}

fn wire_api_to_db(api: WireApi) -> &'static str {
    match api {
        WireApi::Responses => "responses",
        WireApi::ChatCompletions => "chat_completions",
    }
}

fn wire_api_from_db(raw: &str) -> Result<WireApi, AppError> {
    match raw {
        "responses" => Ok(WireApi::Responses),
        "chat_completions" => Ok(WireApi::ChatCompletions),
        other => Err(AppError::Database(format!("非法 wire_api 值: {other}"))),
    }
}

fn outcome_to_db(outcome: AttemptOutcome) -> &'static str {
    match outcome {
        AttemptOutcome::Success => "success",
        AttemptOutcome::Failed => "failed",
        AttemptOutcome::Skipped => "skipped",
    }
}

impl Database {
    /// 全部启用的 provider，按 priority 升序。
    pub fn list_router_providers(&self) -> Result<Vec<RouterProvider>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut statement = conn.prepare(
            "SELECT id, display_name, base_url, wire_api, priority, enabled
             FROM router_providers
             WHERE enabled = 1
             ORDER BY priority ASC, id ASC",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, bool>(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(
                |(id, display_name, base_url, wire_raw, priority, enabled)| {
                    Ok(RouterProvider {
                        id,
                        display_name,
                        base_url,
                        wire_api: wire_api_from_db(&wire_raw)?,
                        priority,
                        enabled,
                    })
                },
            )
            .collect()
    }

    /// 某个逻辑模型在各家的映射，按 provider 的 priority 升序。
    /// 返回的每一项都保证 provider 是 enabled 的。
    pub fn list_model_routes(&self, logical_model: &str) -> Result<Vec<ModelRoute>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut statement = conn.prepare(
            "SELECT map.provider_id, map.logical_model, map.upstream_model
             FROM router_model_map map
             JOIN router_providers provider ON provider.id = map.provider_id
             WHERE map.logical_model = ?1 AND provider.enabled = 1
             ORDER BY provider.priority ASC, provider.id ASC",
        )?;
        let routes = statement
            .query_map([logical_model], |row| {
                Ok(ModelRoute {
                    provider_id: row.get(0)?,
                    logical_model: row.get(1)?,
                    upstream_model: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)?;
        Ok(routes)
    }

    pub fn upsert_router_provider(&self, p: &RouterProvider) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        let now = now_timestamp()?;
        conn.execute(
            "INSERT INTO router_providers (
                 id, display_name, base_url, wire_api, priority, enabled,
                 created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT(id) DO UPDATE SET
                 display_name = excluded.display_name,
                 base_url = excluded.base_url,
                 wire_api = excluded.wire_api,
                 priority = excluded.priority,
                 enabled = excluded.enabled,
                 updated_at = excluded.updated_at",
            params![
                p.id,
                p.display_name,
                p.base_url,
                wire_api_to_db(p.wire_api),
                p.priority,
                p.enabled,
                now,
            ],
        )?;
        Ok(())
    }

    pub fn delete_router_provider(&self, id: &str) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute("DELETE FROM router_providers WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn upsert_model_route(&self, r: &ModelRoute) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "INSERT INTO router_model_map (
                 provider_id, logical_model, upstream_model, created_at
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(provider_id, logical_model) DO UPDATE SET
                 upstream_model = excluded.upstream_model",
            params![
                r.provider_id,
                r.logical_model,
                r.upstream_model,
                now_timestamp()?,
            ],
        )?;
        Ok(())
    }

    pub fn delete_model_routes_for_provider(&self, provider_id: &str) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "DELETE FROM router_model_map WHERE provider_id = ?1",
            [provider_id],
        )?;
        Ok(())
    }

    pub fn record_router_attempt(&self, a: &RouterAttempt) -> Result<i64, AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "INSERT INTO router_attempts (
                 started_at, logical_model, provider_id, outcome, failure_kind,
                 http_status, input_tokens, output_tokens, duration_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                a.started_at,
                a.logical_model,
                a.provider_id,
                outcome_to_db(a.outcome),
                a.failure_kind,
                a.http_status.map(i64::from),
                a.input_tokens,
                a.output_tokens,
                a.duration_ms,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// 按 provider 汇总某段时间的用量，供分账面板使用。
    ///
    /// 时间范围是半开区间 `[start_at, end_at)`，按 `started_at`（epoch 毫秒）过滤。
    pub fn sum_router_usage_by_provider(
        &self,
        start_at: i64,
        end_at: i64,
    ) -> Result<Vec<RouterUsageSummary>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut statement = conn.prepare(
            "SELECT provider_id,
                    COUNT(*) AS attempts,
                    COALESCE(SUM(CASE WHEN outcome = 'failed' THEN 1 ELSE 0 END), 0) AS failures,
                    COALESCE(SUM(input_tokens), 0) AS input_tokens,
                    COALESCE(SUM(output_tokens), 0) AS output_tokens
             FROM router_attempts
             WHERE started_at >= ?1 AND started_at < ?2
             GROUP BY provider_id
             ORDER BY provider_id",
        )?;
        let summaries = statement
            .query_map(params![start_at, end_at], |row| {
                Ok(RouterUsageSummary {
                    provider_id: row.get(0)?,
                    attempts: row.get(1)?,
                    failures: row.get(2)?,
                    input_tokens: row.get(3)?,
                    output_tokens: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)?;
        Ok(summaries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;

    fn outcome_from_db(raw: &str) -> Result<AttemptOutcome, AppError> {
        match raw {
            "success" => Ok(AttemptOutcome::Success),
            "failed" => Ok(AttemptOutcome::Failed),
            "skipped" => Ok(AttemptOutcome::Skipped),
            other => Err(AppError::Database(format!(
                "非法 attempt outcome 值: {other}"
            ))),
        }
    }

    fn provider(id: &str, priority: i64, enabled: bool) -> RouterProvider {
        RouterProvider {
            id: id.to_string(),
            display_name: format!("Provider {id}"),
            base_url: format!("https://{id}.example/v1"),
            wire_api: WireApi::Responses,
            priority,
            enabled,
        }
    }

    fn route(provider_id: &str, logical_model: &str, upstream_model: &str) -> ModelRoute {
        ModelRoute {
            provider_id: provider_id.to_string(),
            logical_model: logical_model.to_string(),
            upstream_model: upstream_model.to_string(),
        }
    }

    fn attempt(
        started_at: i64,
        provider_id: &str,
        outcome: AttemptOutcome,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
    ) -> RouterAttempt {
        RouterAttempt {
            started_at,
            logical_model: "gpt-5.6-sol".to_string(),
            provider_id: provider_id.to_string(),
            outcome,
            failure_kind: if outcome == AttemptOutcome::Failed {
                Some("timeout".to_string())
            } else {
                None
            },
            http_status: None,
            input_tokens,
            output_tokens,
            duration_ms: None,
        }
    }

    #[test]
    fn list_router_providers_orders_by_priority_and_skips_disabled() {
        let db = Database::memory().unwrap();
        db.upsert_router_provider(&provider("a", 20, true)).unwrap();
        db.upsert_router_provider(&provider("b", 10, true)).unwrap();
        db.upsert_router_provider(&provider("c", 30, false))
            .unwrap();

        let listed = db.list_router_providers().unwrap();

        assert_eq!(
            listed.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            vec!["b", "a"],
        );
    }

    #[test]
    fn list_model_routes_filters_by_logical_model_orders_by_priority_and_skips_disabled() {
        let db = Database::memory().unwrap();
        db.upsert_router_provider(&provider("a", 20, true)).unwrap();
        db.upsert_router_provider(&provider("b", 10, true)).unwrap();
        db.upsert_router_provider(&provider("c", 5, false)).unwrap();
        db.upsert_model_route(&route("a", "gpt-5.6-sol", "sol"))
            .unwrap();
        db.upsert_model_route(&route("b", "gpt-5.6-sol", "gpt-5.6-sol"))
            .unwrap();
        db.upsert_model_route(&route("c", "gpt-5.6-sol", "hidden"))
            .unwrap();
        db.upsert_model_route(&route("a", "other-model", "other"))
            .unwrap();

        let routes = db.list_model_routes("gpt-5.6-sol").unwrap();

        // c 的 priority 最小但 enabled = 0，必须被跳过；结果按 priority 升序。
        assert_eq!(
            routes,
            vec![
                route("b", "gpt-5.6-sol", "gpt-5.6-sol"),
                route("a", "gpt-5.6-sol", "sol"),
            ],
        );
    }

    #[test]
    fn list_model_routes_returns_empty_vec_for_unknown_model() {
        let db = Database::memory().unwrap();

        assert_eq!(db.list_model_routes("nonexistent").unwrap(), Vec::new());
    }

    #[test]
    fn delete_router_provider_cascades_to_model_map() {
        let db = Database::memory().unwrap();
        db.upsert_router_provider(&provider("a", 1, true)).unwrap();
        db.upsert_router_provider(&provider("b", 2, true)).unwrap();
        db.upsert_model_route(&route("a", "m1", "u1")).unwrap();
        db.upsert_model_route(&route("b", "m1", "u2")).unwrap();

        db.delete_router_provider("a").unwrap();

        assert_eq!(
            db.list_model_routes("m1").unwrap(),
            vec![route("b", "m1", "u2")],
        );
        let remaining: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM router_model_map", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn record_router_attempt_feeds_provider_summary_within_time_range() {
        let db = Database::memory().unwrap();
        db.record_router_attempt(&attempt(
            1000,
            "a",
            AttemptOutcome::Success,
            Some(10),
            Some(20),
        ))
        .unwrap();
        db.record_router_attempt(&attempt(
            1500,
            "a",
            AttemptOutcome::Failed,
            Some(1),
            Some(2),
        ))
        .unwrap();
        db.record_router_attempt(&attempt(
            2000,
            "b",
            AttemptOutcome::Success,
            Some(30),
            Some(40),
        ))
        .unwrap();
        db.record_router_attempt(&attempt(2500, "b", AttemptOutcome::Failed, None, None))
            .unwrap();
        db.record_router_attempt(&attempt(
            3000,
            "a",
            AttemptOutcome::Success,
            Some(100),
            Some(200),
        ))
        .unwrap();

        let summary = db.sum_router_usage_by_provider(1500, 2500).unwrap();

        // [1500, 2500) 半开区间:1000 与 3000 在范围外,2500 是开边界,都不计入。
        assert_eq!(
            summary,
            vec![
                RouterUsageSummary {
                    provider_id: "a".to_string(),
                    attempts: 1,
                    failures: 1,
                    input_tokens: 1,
                    output_tokens: 2,
                },
                RouterUsageSummary {
                    provider_id: "b".to_string(),
                    attempts: 1,
                    failures: 0,
                    input_tokens: 30,
                    output_tokens: 40,
                },
            ],
        );
    }

    #[test]
    fn wire_api_and_attempt_outcome_round_trip_through_storage() {
        let db = Database::memory().unwrap();
        let mut stored_provider = provider("packyapi", 1, true);
        stored_provider.wire_api = WireApi::ChatCompletions;
        db.upsert_router_provider(&stored_provider).unwrap();

        let listed = db.list_router_providers().unwrap();
        assert_eq!(listed[0].wire_api, WireApi::ChatCompletions);

        for (outcome, expected) in [
            (AttemptOutcome::Success, "success"),
            (AttemptOutcome::Failed, "failed"),
            (AttemptOutcome::Skipped, "skipped"),
        ] {
            db.record_router_attempt(&attempt(1, "packyapi", outcome, None, None))
                .unwrap();
            let raw: String = {
                let conn = db.conn.lock().unwrap();
                conn.query_row(
                    "SELECT outcome FROM router_attempts ORDER BY id DESC LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap()
            };
            assert_eq!(raw, expected);
            assert_eq!(outcome_from_db(&raw).unwrap(), outcome);
        }
    }

    #[test]
    fn invalid_wire_api_in_db_returns_error_instead_of_panicking() {
        let db = Database::memory().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO router_providers (
                     id, display_name, base_url, wire_api, priority, enabled,
                     created_at, updated_at
                 ) VALUES ('broken', 'Broken', 'https://broken.example', 'bogus', 1, 1, 0, 0)",
                [],
            )
            .unwrap();
        }

        let error = db.list_router_providers().unwrap_err();

        assert!(error.to_string().contains("wire_api"));
    }

    #[test]
    fn upserts_overwrite_provider_and_route_fields() {
        let db = Database::memory().unwrap();
        let mut updated = provider("packyapi", 5, true);
        db.upsert_router_provider(&updated).unwrap();
        updated.display_name = "PackyAPI v2".to_string();
        updated.wire_api = WireApi::ChatCompletions;
        updated.priority = 7;
        updated.enabled = false;
        db.upsert_router_provider(&updated).unwrap();

        let conn = db.conn.lock().unwrap();
        let (name, wire, priority, enabled): (String, String, i64, i64) = conn
            .query_row(
                "SELECT display_name, wire_api, priority, enabled
                 FROM router_providers WHERE id = 'packyapi'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        drop(conn);
        assert_eq!(name, "PackyAPI v2");
        assert_eq!(wire, "chat_completions");
        assert_eq!((priority, enabled), (7, 0));
        assert!(db.list_router_providers().unwrap().is_empty());

        db.upsert_model_route(&route("packyapi", "gpt-5.6-sol", "sol"))
            .unwrap();
        db.upsert_model_route(&route("packyapi", "gpt-5.6-sol", "sol-2"))
            .unwrap();
        let (count, upstream): (i64, String) = {
            let conn = db.conn.lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*), MAX(upstream_model)
                 FROM router_model_map
                 WHERE provider_id = 'packyapi' AND logical_model = 'gpt-5.6-sol'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
        };
        assert_eq!(count, 1);
        assert_eq!(upstream, "sol-2");
    }
}
