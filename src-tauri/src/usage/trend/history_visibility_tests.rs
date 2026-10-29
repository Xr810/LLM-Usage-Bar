//! Cross-layer history-read contract, including backup exclusion.
mod tests {
    use crate::store::usage_stats::{get_data_source_breakdown, LogFilters};
    use crate::usage::{
        aggregation, dashboard::UsageDashboardService, session_list::list_sessions_at,
    };
    use crate::{Database, HistoryVisibilityPolicy};
    use chrono::{Local, NaiveDate, TimeZone};
    use rusqlite::params;
    use serde_json::{json, Value};
    use std::sync::Arc;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };

    struct Policy {
        cutoff: Mutex<Option<NaiveDate>>,
        calls: AtomicUsize,
    }
    impl HistoryVisibilityPolicy for Policy {
        fn cutoff(&self) -> Option<NaiveDate> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.cutoff.lock().unwrap()
        }
    }

    fn seed(at: i64, old: bool) -> Arc<Database> {
        let db = Arc::new(Database::memory().unwrap());
        {
            let conn = db.conn.lock().unwrap();
            // Monitor only the fixture account: startup presets carry random
            // binding IDs/timestamps unrelated to history visibility.
            conn.execute("UPDATE usage_providers SET enabled = 0", [])
                .unwrap();
            conn.execute_batch("INSERT INTO usage_providers (id,name,billing_kind,product_group_id,token_sources,enabled,needs_review,created_at,updated_at)
                VALUES ('history-test','History','metered','product','[\"proxy\"]',1,0,1,1);").unwrap();
            for (id, timestamp, tokens, cost) in [
                ("old-history-row", at - 1, 13, "3.25"),
                ("visible-history-row", at, 7, "1.50"),
            ] {
                if !old && timestamp < at {
                    continue;
                }
                conn.execute("INSERT INTO proxy_request_logs (request_id,provider_id,app_type,model,input_tokens,total_cost_usd,latency_ms,status_code,created_at)
                    VALUES (?1,'history-test','claude',?1,?2,?3,11,200,?4)", params![id,tokens,cost,timestamp]).unwrap();
                conn.execute("INSERT INTO usage_events (event_id,source,provider_id,agent_module_id,product_group_id,occurred_at,model,input_tokens,output_tokens,total_cost_usd,cost_source,created_at)
                    VALUES (?1,'proxy','history-test','claude-code','product',?2,?1,?3,0,?4,'upstream',?2)", params![id,timestamp,tokens,cost]).unwrap();
                conn.execute("INSERT INTO router_attempts (started_at,logical_model,provider_id,outcome,input_tokens)
                    VALUES (?1,?2,'history-test','success',?3)", params![timestamp * 1000,id,tokens]).unwrap();
                conn.execute("INSERT INTO usage_hourly_rollups (hour_utc,app_type,provider_id,model,billing_identity,data_source,request_count,input_tokens,cost_micro_usd)
                    VALUES (?1,'claude','history-test',?2,'','proxy',1,?3,?4)", params![timestamp.div_euclid(3600)*3600,id,tokens,if tokens==13 {3_250_000} else {1_500_000}]).unwrap();
            }
            for (offset, tokens) in [(-2, 31), (1, 11)] {
                if !old && offset < 0 {
                    continue;
                }
                let date = Local.timestamp_opt(at, 0).unwrap().date_naive()
                    + chrono::Duration::days(offset);
                conn.execute("INSERT INTO usage_daily_rollups (date,app_type,provider_id,model,request_count,success_count,input_tokens,total_cost_usd)
                    VALUES (?1,'claude','history-test','rollup',1,1,?2,'2.75')", params![date.to_string(),tokens]).unwrap();
            }
        }
        db
    }

    fn reads(db: &Database, at: i64) -> Value {
        let start = at - 86400 * 2;
        let end = at + 86400 * 2;
        json!({
            "rows": db.get_request_logs(&LogFilters::default(),0,50).unwrap(),
            "oldDetail": db.get_request_detail("old-history-row").unwrap(),
            "newDetail": db.get_request_detail("visible-history-row").unwrap(),
            "summary": db.get_usage_summary(None,None,None,None,None).unwrap(),
            "daily": db.get_daily_trends(Some(start),Some(end),None,None,None).unwrap().into_iter().filter(|row| row.request_count > 0).collect::<Vec<_>>(),
            "hourly": db.get_daily_trends(Some(at-3600),Some(at+3600),None,None,None).unwrap().into_iter().filter(|row| row.request_count > 0).collect::<Vec<_>>(),
            "apps": db.get_usage_summary_by_app(None,None,None,None).unwrap(),
            "providers": db.get_provider_stats(None,None,None,None,None).unwrap(),
            "models": db.get_model_stats(None,None,None,None,None).unwrap(),
            "sources": get_data_source_breakdown(db).unwrap(),
            "sessions": list_sessions_at(db,at,false,None,None).unwrap(),
            "routerUsage": db.sum_router_usage_by_provider(start * 1000,end * 1000).unwrap().into_iter().map(|row| json!({
                "providerId":row.provider_id,"attempts":row.attempts,"failures":row.failures,"inputTokens":row.input_tokens,"outputTokens":row.output_tokens
            })).collect::<Vec<_>>(),
            "limits": db.check_provider_limits("history-test","claude").unwrap(),
            "events": db.list_usage_events("history-test",start,end,1,50).unwrap(),
            "agentEvents": db.list_agent_usage_events("claude-code",None,start,end,1,50).unwrap(),
            "diagnostics": db.get_unassigned_usage_diagnostics().unwrap(),
            "modelUsage": aggregation::aggregate_model_usage(db,start,end).unwrap(),
            "agentUsage": aggregation::aggregate_agent_usage(db,start,end).unwrap(),
            "heatmap": aggregation::aggregate_enabled_provider_trend(db,start,end).unwrap().1,
            "accountHeatmap": aggregation::aggregate_provider_account_trend(db,"history-test",start,end).unwrap().1,
            "topModel": aggregation::most_used_provider_model(db,"history-test",start,end).unwrap(),
            "dashboard": UsageDashboardService::new(db).with_pace_now_timestamp(at + 3600).get_provider_dashboard(start,end).unwrap(),
            "agentDashboard": UsageDashboardService::new(db).with_pace_now_timestamp(at + 3600).get_dashboard(start,end,"claude-code").unwrap(),
        })
    }

    #[test]
    fn injected_visibility_filters_all_history_reads_but_not_backup_or_sync() {
        let day = Local::now().date_naive();
        let at = Local
            .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .unwrap()
            .timestamp();
        let db = seed(at, true);
        let reference = seed(at, false);
        let baseline = reads(&db, at);
        assert_eq!(baseline["rows"]["total"], 2);
        let backup = db.export_sql_string().unwrap();
        let sync = db.export_sql_string_for_sync().unwrap();
        let policy = Arc::new(Policy {
            cutoff: Mutex::new(Some(day)),
            calls: AtomicUsize::new(0),
        });
        let mut injections = crate::Injections::community();
        let injected = policy.clone();
        injections.history_visibility = Box::new(move |_| Ok(injected));
        let _state = crate::AppState::new_with_injections(
            db.clone(),
            crate::secrets::unavailable_credential_store(),
            Arc::new(crate::quota::QuotaService::new(db.clone())),
            injections,
        );
        let actual = reads(&db, at);
        let expected = reads(&reference, at);
        assert_eq!(actual["rows"]["total"], 1);
        assert_eq!(actual["summary"]["totalInputTokens"], 18);
        assert_eq!(actual["limits"]["dailyUsage"], "1.500000");
        assert_eq!(actual["routerUsage"][0]["inputTokens"], 7);
        for (path, expected) in expected.as_object().unwrap() {
            assert_eq!(&actual[path], expected, "history path: {path}");
        }
        // Count and page must share one policy snapshot, even if policy changes.
        policy.calls.store(0, Ordering::SeqCst);
        assert_eq!(
            db.list_usage_events("history-test", at - 1, at + 1, 1, 1)
                .unwrap()
                .total,
            1
        );
        assert_eq!(policy.calls.load(Ordering::SeqCst), 1);
        policy.calls.store(0, Ordering::SeqCst);
        assert_eq!(
            db.get_request_logs(&LogFilters::default(), 0, 1)
                .unwrap()
                .total,
            1
        );
        assert_eq!(policy.calls.load(Ordering::SeqCst), 1);
        // A view may tighten the cutoff, never widen it; an entirely hidden
        // range stays empty rather than resetting to the previous day.
        assert_eq!(
            db.list_usage_events("history-test", at + 1, at + 2, 1, 50)
                .unwrap()
                .total,
            0
        );
        assert!(db
            .get_daily_trends(Some(at - 86400), Some(at - 1), None, None, None)
            .unwrap()
            .is_empty());
        let mut license = crate::policy::community_license_status();
        license.build = crate::policy::LicenseBuild::Official;
        license.tier = crate::policy::LicenseTier::Pro;
        // Trend queries use their requested timezone, independently of Local.
        let trend_db = seed(
            day.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp(),
            true,
        );
        let mut injections = crate::Injections::community();
        let injected = policy.clone();
        injections.history_visibility = Box::new(move |_| Ok(injected));
        let _trend_state = crate::AppState::new_with_injections(
            trend_db.clone(),
            crate::secrets::unavailable_credential_store(),
            Arc::new(crate::quota::QuotaService::new(trend_db.clone())),
            injections,
        );
        for group in [None, Some("provider"), Some("project"), Some("client")] {
            let trend = crate::usage::trend::query_usage_trend_at(&trend_db, json!({
                "tz":"UTC", "from":(day - chrono::Duration::days(2)).to_string(), "to":(day + chrono::Duration::days(1)).to_string(), "granularity":"day", "groupBy":group
            }), &license, chrono::Utc::now()).unwrap();
            assert_eq!(trend.clamped_from, Some(day.to_string()));
            assert_eq!(
                trend
                    .series
                    .iter()
                    .flat_map(|s| &s.points)
                    .map(|p| p.tokens)
                    .sum::<i64>(),
                7
            );
        }
        assert_eq!(db.export_sql_string().unwrap(), backup);
        assert_eq!(db.export_sql_string_for_sync().unwrap(), sync);
        *policy.cutoff.lock().unwrap() = None;
        assert_eq!(reads(&db, at), baseline);
    }
}
