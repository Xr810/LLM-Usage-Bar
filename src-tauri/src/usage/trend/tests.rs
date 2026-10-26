use super::*;
use rusqlite::params;
use serde_json::json;

#[test]
fn guard_decisions_are_audit_only_across_usage_queries_and_retention() -> Result<(), AppError> {
    use crate::usage::aggregation;
    use chrono::TimeZone;

    let db = Database::memory()?;
    let now = chrono::Utc::now();
    let at = chrono::Local
        .from_local_datetime(
            &chrono::Local::now()
                .date_naive()
                .and_hms_opt(12, 0, 0)
                .unwrap(),
        )
        .earliest()
        .unwrap()
        .timestamp();
    let start = at - 86400;
    let end = at + 86400;
    let license = pro();
    {
        let conn = lock_conn!(db.conn);
        // Nonempty ordinary and pre-existing blocked controls must survive.
        for (id, reason, status, input, output, cost) in [
            ("used", None, 200, 13, 7, "0.25"),
            ("blocked", Some("blocked"), 403, 0, 0, "0"),
        ] {
            conn.execute(
                "INSERT INTO proxy_request_logs (
                     request_id, provider_id, app_type, model, input_tokens,
                     output_tokens, total_cost_usd, latency_ms, status_code,
                     created_at, end_reason, project_alias, client_tag_id
                 ) VALUES (?1, 'p', 'claude', 'm', ?2, ?3, ?4, 11, ?5, ?6, ?7, 'project', 'client')",
                params![id, input, output, cost, status, at, reason],
            )?;
        }
        conn.execute_batch(
            "INSERT INTO usage_providers (id, name, billing_kind, product_group_id,
                 token_sources, enabled, needs_review, created_at, updated_at)
             VALUES ('p', 'Provider', 'metered', 'product', '[\"proxy\"]', 1, 0, 1, 1);",
        )?;
        conn.execute(
            "INSERT INTO usage_events (event_id, source, provider_id, product_group_id,
                 occurred_at, model, input_tokens, output_tokens, total_cost_usd,
                 cost_source, created_at)
             VALUES ('event', 'proxy', 'p', 'product', ?1, 'm', 13, 7, '0.25', 'upstream', ?1)",
            [at],
        )?;
        conn.execute(
            "INSERT INTO rollup_dirty_hours (hour_utc) VALUES (?1)",
            [at.div_euclid(3600) * 3600],
        )?;
    }
    db.rollup_and_prune(36500)?;
    let snapshot = || -> Result<serde_json::Value, AppError> {
        let mut trends = Vec::new();
        for group in [None, Some("provider"), Some("project"), Some("client")] {
            trends.push(query_usage_trend_at(&db, json!({
                "tz": "Asia/Hong_Kong",
                "from": chrono::DateTime::from_timestamp(start, 0).unwrap().with_timezone(&chrono_tz::Asia::Hong_Kong).format("%Y-%m-%d").to_string(),
                "to": chrono::DateTime::from_timestamp(end, 0).unwrap().with_timezone(&chrono_tz::Asia::Hong_Kong).format("%Y-%m-%d").to_string(),
                "granularity": "day", "groupBy": group
            }), &license, now)?);
        }
        Ok(json!({
            "summary": db.get_usage_summary(None, None, None, None, None)?,
            "today": db.get_usage_summary(Some(at - 3600), Some(end), None, None, None)?,
            "byApp": db.get_usage_summary_by_app(None, None, None, None)?,
            "hourly": db.get_daily_trends(Some(at - 3600), Some(at + 3600), None, None, None)?,
            "daily": db.get_daily_trends(Some(start), Some(end), None, None, None)?,
            "providers": db.get_provider_stats(None, None, None, None, None)?,
            "models": db.get_model_stats(None, None, None, None, None)?,
            "sources": crate::store::usage_stats::get_data_source_breakdown(&db)?,
            "limits": db.check_provider_limits("p", "claude")?,
            "sessions": crate::usage::session_list::list_sessions_at(&db, at, false, None, None)?,
            "trends": trends,
            "eventModels": aggregation::aggregate_model_usage(&db, start, end)?,
            "agents": aggregation::aggregate_agent_usage(&db, start, end)?,
            "eventProviders": aggregation::aggregate_enabled_provider_trend(&db, start, end)?.1,
            "hourlyRequestCount": lock_conn!(db.conn).query_row(
                "SELECT COALESCE(SUM(request_count), 0) FROM usage_hourly_rollups", [], |r| r.get::<_, i64>(0)
            )?
        }))
    };
    let before = snapshot()?;
    assert_eq!(before["summary"]["totalRequests"], 2);
    assert_eq!(before["hourlyRequestCount"], 2);
    assert_eq!(before["summary"]["totalInputTokens"], 13);
    assert_eq!(before["summary"]["totalOutputTokens"], 7);
    assert_eq!(before["limits"]["dailyUsage"], "0.250000");
    let today_before = aggregation::aggregate_provider_account_range(&db, "p", start, end)?;
    assert_eq!(today_before.event_count, 1);
    assert_eq!(today_before.total_cost_usd.as_deref(), Some("0.25"));
    let mut audit_count = 0;
    for (end_reason, message) in [
        ("guard_deny", "rule"),
        ("guard_hold", "denied"),
        ("guard_hold", "timeout"),
        ("guard_hold", "queue_full"),
        ("guard_hold", "approval_required"),
        ("guard_hold_timeout", "legacy"),
        ("guard_hold_overflow", "legacy"),
        ("guard_hold_denied", "legacy"),
    ] {
        let conn = lock_conn!(db.conn);
        // Deliberately nonzero imported data proves exclusion by end_reason,
        // not an accidental zero-cost/token sum or HTTP-error exclusion.
        for status in [200, 451] {
            conn.execute(
                "INSERT INTO proxy_request_logs (
                     request_id, provider_id, app_type, model, input_tokens,
                     output_tokens, total_cost_usd, latency_ms, status_code,
                     created_at, end_reason, project_alias, client_tag_id, error_message
                 ) VALUES (?1, 'p', 'claude', 'm', 101, 203, '17.5', 999, ?2, ?3,
                           ?4, 'project', 'client', ?5)",
                params![
                    format!("audit-{audit_count}-{status}"),
                    status,
                    at,
                    end_reason,
                    message
                ],
            )?;
            audit_count += 1;
        }
        conn.execute(
            "INSERT INTO rollup_dirty_hours (hour_utc) VALUES (?1)",
            [at.div_euclid(3600) * 3600],
        )?;
        drop(conn);
        db.rollup_and_prune(36500)?;
        assert_eq!(snapshot()?, before, "{end_reason}/{message} changed usage");
    }
    assert_eq!(
        aggregation::aggregate_provider_account_range(&db, "p", start, end)?,
        today_before,
        "tray today totals and guard spend consume the event ledger, not audit rows"
    );
    assert_eq!(
        db.get_request_logs(&Default::default(), 1, 20)?.total,
        2 + audit_count,
        "Gate decisions must remain visible in the audit list"
    );
    // Move all details into the past, then exercise daily archival and query
    // the same all-time totals: denied counts/tokens/cost must not reappear.
    {
        let conn = lock_conn!(db.conn);
        conn.execute(
            "UPDATE proxy_request_logs SET created_at = ?1",
            [at - 3 * 86400],
        )?;
    }
    assert_eq!(db.rollup_and_prune(1)?, u64::from(2 + audit_count));
    let summary = db.get_usage_summary(None, None, None, None, None)?;
    assert_eq!(summary.total_requests, 2);
    assert_eq!(summary.total_input_tokens, 13);
    assert_eq!(summary.total_output_tokens, 7);
    assert_eq!(summary.total_cost.parse::<f64>().unwrap(), 0.25);
    Ok(())
}

fn instant(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

fn license(build: LicenseBuild, tier: LicenseTier) -> LicenseStatus {
    let mut license = LicenseStatus::community_free();
    license.build = build;
    license.tier = tier;
    license
}

fn pro() -> LicenseStatus {
    license(LicenseBuild::Official, LicenseTier::Pro)
}

#[test]
fn legacy_window_uses_local_days_dst_end_conventions_and_server_owned_entitlement() {
    use chrono::TimeZone;
    let hk = chrono_tz::Asia::Hong_Kong;
    // UTC is still Oct 26, but the local window must be Sep 28 through Oct 27.
    let now = instant("2026-10-26T16:30:00Z").with_timezone(&hk);
    let free = LicenseStatus::community_free();
    let range = clamp_legacy_usage_query(None, None, false, &free, now).unwrap();
    assert_eq!(
        range.start,
        Some(instant("2026-09-27T16:00:00Z").timestamp())
    );
    assert_eq!(range.end, Some(instant("2026-10-27T16:00:00Z").timestamp()));
    assert_eq!(range.clamped_from.as_deref(), Some("2026-09-28"));
    let havana = chrono_tz::America::Havana;
    let now = havana.with_ymd_and_hms(2026, 4, 6, 12, 0, 0).unwrap();
    let range = clamp_legacy_usage_query(None, None, false, &free, now).unwrap();
    // Mar 8 has no 00:00; it starts at 01:00 after the DST jump.
    assert_eq!(
        range.start,
        Some(instant("2026-03-08T05:00:00Z").timestamp())
    );
    assert_eq!(range.clamped_from.as_deref(), Some("2026-03-08"));
    let ny = chrono_tz::America::New_York;
    let now = ny.with_ymd_and_hms(2026, 11, 2, 12, 0, 0).unwrap();
    let range = clamp_legacy_usage_query(None, None, true, &free, now).unwrap();
    // The 30 local dates straddle the 25-hour fall-back day.
    assert_eq!(
        range.end.unwrap() - range.start.unwrap() + 1,
        30 * 86400 + 3600
    );
    for status in [
        license(LicenseBuild::Official, LicenseTier::Free),
        license(LicenseBuild::Community, LicenseTier::Pro),
        license(LicenseBuild::Community, LicenseTier::ProP3),
    ] {
        assert_eq!(
            clamp_legacy_usage_query(None, None, true, &status, now)
                .unwrap()
                .clamped_from,
            range.clamped_from
        );
        let error = require_usage_split(&status).unwrap_err();
        assert_eq!(serde_json::to_value(error).unwrap()["code"], "PRO_REQUIRED");
    }
    for tier in [LicenseTier::Pro, LicenseTier::ProP3] {
        let status = license(LicenseBuild::Official, tier);
        require_usage_split(&status).unwrap();
        let range = clamp_legacy_usage_query(None, None, true, &status, now).unwrap();
        assert_eq!(
            (range.start, range.end, range.clamped_from),
            (None, None, None)
        );
    }
}

fn request(from: &str, to: &str) -> serde_json::Value {
    json!({"tz":"UTC", "from":from, "to":to, "granularity":"day"})
}

fn run(db: &Database, request: serde_json::Value, license: &LicenseStatus) -> UsageTrendResponse {
    query_usage_trend_at(db, request, license, instant("2026-10-26T16:30:00Z")).unwrap()
}

fn raw(
    db: &Database,
    id: &str,
    at: &str,
    provider: &str,
    project: Option<&str>,
    client: Option<&str>,
    cost: &str,
) {
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO proxy_request_logs
         (request_id, provider_id, app_type, model, created_at, project_alias,
          client_tag_id, input_tokens, output_tokens, cache_read_tokens,
          cache_creation_tokens, total_cost_usd, latency_ms, status_code)
         VALUES (?1, ?2, 'claude', 'model', ?3, ?4, ?5, 2, 3, 5, 1, ?6, 0, 200)",
            params![id, provider, instant(at).timestamp(), project, client, cost],
        )
        .unwrap();
}

fn hourly(db: &Database, at: i64, provider: &str, requests: i64, cost: Option<i64>) {
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO usage_hourly_rollups
         (hour_utc, app_type, provider_id, model, billing_identity, data_source,
          request_count, input_tokens, output_tokens, cache_read_tokens,
          cache_creation_tokens, cost_micro_usd)
         VALUES (?1, 'claude', ?2, 'model', 'api_key', 'proxy', ?3, 2, 3, 5, 1, ?4)",
            params![at, provider, requests, cost],
        )
        .unwrap();
}

#[test]
fn raw_grouping_shared_contract_partial_cost_zero_fill_and_client_fallback() {
    let db = Database::memory().unwrap();
    raw(
        &db,
        "a",
        "2026-10-24T01:00:00Z",
        "p1",
        Some("project-a"),
        Some("client-a"),
        "1.234567",
    );
    raw(
        &db,
        "b",
        "2026-10-24T02:00:00Z",
        "p1",
        Some("project-a"),
        Some("client-a"),
        "invalid",
    );
    raw(
        &db,
        "c",
        "2026-10-25T01:00:00Z",
        "p1",
        Some("project-a"),
        Some("client-a"),
        "unknown",
    );
    raw(
        &db,
        "d",
        "2026-10-24T03:00:00Z",
        "p1",
        None,
        Some(""),
        "0.000007",
    );
    raw(
        &db,
        "excluded",
        "2026-10-24T04:00:00Z",
        "p2",
        Some("project-a"),
        Some("client-a"),
        "999",
    );
    // A competing source must not contribute to project/client totals.
    hourly(
        &db,
        instant("2026-10-24T01:00:00Z").timestamp(),
        "p1",
        999,
        Some(999),
    );
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/usage-trend-contract.json"
    ))
    .unwrap();
    let response = run(&db, fixture["request"].clone(), &pro());
    assert_eq!(serde_json::to_value(response).unwrap(), fixture["response"]);
    // The same client spans two projects: the two dimensions must disagree.
    raw(
        &db,
        "other-project",
        "2026-10-24T05:00:00Z",
        "p1",
        Some("project-b"),
        Some("client-a"),
        "0.000013",
    );
    let projects = run(&db, fixture["request"].clone(), &pro());
    assert_eq!(projects.series.len(), 3);
    assert_eq!(projects.series[1].group_key, "project-b");
    assert_eq!(projects.series[1].points[0].cost_micro_usd, Some(13));
    let mut client = fixture["request"].clone();
    client["groupBy"] = json!("client");
    let response = run(&db, client, &pro());
    assert_eq!(response.series.len(), 2);
    assert_eq!(response.series[0].group_key, "client-a");
    assert_eq!(response.series[0].group_label, "client-a");
    assert_eq!(response.series[0].points[0].requests, 3);
    assert_eq!(response.series[0].points[0].cost_micro_usd, Some(1_234_580));
    assert_eq!(response.series[0].points[1].cost_micro_usd, None);
    assert_eq!(response.series[1].group_label, "");
    assert_eq!(response.series[1].points[0].requests, 1);
}

#[test]
fn hourly_source_provider_names_filters_and_ungrouped_shape() {
    let db = Database::memory().unwrap();
    db.conn.lock().unwrap().execute("INSERT INTO providers (id, app_type, name, settings_config) VALUES ('p1', 'claude', 'Display name', '{}')", []).unwrap();
    let at = instant("2026-10-24T01:00:00Z").timestamp();
    hourly(&db, at, "p1", 7, Some(101));
    hourly(&db, at, "deleted-provider", 3, Some(202));
    hourly(&db, at, "", 2, Some(0));
    raw(
        &db,
        "not-hourly",
        "2026-10-24T01:00:00Z",
        "p1",
        None,
        None,
        "999",
    );
    let input = request("2026-10-24", "2026-10-25");
    let response = run(&db, input.clone(), &pro());
    assert_eq!(response.series.len(), 1);
    assert_eq!(response.series[0].group_key, "all");
    assert_eq!(response.series[0].group_label, "");
    assert_eq!(
        response.series[0].points[0],
        UsageTrendPoint {
            date_local: "2026-10-24".into(),
            requests: 12,
            tokens: 33,
            cost_micro_usd: Some(303)
        }
    );
    let mut grouped = input.clone();
    grouped["groupBy"] = json!("provider");
    let response = run(&db, grouped.clone(), &pro());
    assert_eq!(
        response
            .series
            .iter()
            .map(|s| (s.group_key.as_str(), s.group_label.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("deleted-provider", "deleted-provider"),
            ("p1", "Display name"),
            ("unattributed", "")
        ]
    );
    assert_eq!(response.series[2].points[0].requests, 2);
    grouped["filters"] = json!({"providerIds":["p1", "p1"]});
    let response = run(&db, grouped.clone(), &pro());
    assert_eq!(response.series[0].points[0].requests, 7);
    assert_eq!(response.series.len(), 2);
    grouped["filters"] = json!({"providerIds":[]});
    let response = run(&db, grouped, &pro());
    assert_eq!(response.series.len(), 1);
    assert_eq!(response.series[0].points[0].requests, 0);
    let mut filtered = input;
    filtered["filters"] = json!({"providerIds":["p1' OR 1=1 --"]});
    assert_eq!(run(&db, filtered, &pro()).series[0].points[0].requests, 0);
}

#[test]
fn nullable_rollup_costs_do_not_turn_unknown_into_zero() {
    let db = Database::memory().unwrap();
    // Test-only nullable relation shadows the real NOT NULL schema. No
    // production migration is permitted or needed for the reader's contract.
    db.conn.lock().unwrap().execute_batch("CREATE TEMP TABLE usage_hourly_rollups AS SELECT * FROM main.usage_hourly_rollups WHERE 0;").unwrap();
    let at = instant("2026-10-24T01:00:00Z").timestamp();
    hourly(&db, at, "p1", 1, None);
    hourly(&db, at + 3600, "p1", 2, Some(17));
    hourly(&db, at + 86400, "p1", 3, None);
    let response = run(&db, request("2026-10-24", "2026-10-26"), &pro());
    let points = &response.series[0].points;
    assert_eq!(points[0].cost_micro_usd, Some(17));
    assert_eq!(points[1].cost_micro_usd, None);
    assert_eq!(points[2].cost_micro_usd, Some(0));
    assert!(serde_json::to_value(&points[1])
        .unwrap()
        .get("costMicroUsd")
        .is_none());
}

#[test]
fn calendar_days_follow_both_dst_transitions_and_fractional_hour_start() {
    for (date, start, hours) in [
        ("2026-03-08", "2026-03-08T05:00:00Z", 23),
        ("2026-11-01", "2026-11-01T04:00:00Z", 25),
    ] {
        let db = Database::memory().unwrap();
        let start = instant(start).timestamp();
        for hour in -1..=hours {
            hourly(&db, start + hour * 3600, "p1", 1, Some(2));
        }
        let mut input = request(date, date);
        input["tz"] = json!("America/New_York");
        let point = &run(&db, input, &pro()).series[0].points[0];
        assert_eq!(point.requests, hours);
        assert_eq!(point.tokens, hours * 11);
        assert_eq!(point.cost_micro_usd, Some(hours * 2));
    }
    let db = Database::memory().unwrap();
    hourly(
        &db,
        instant("2026-10-24T18:00:00Z").timestamp(),
        "p1",
        3,
        Some(7),
    );
    hourly(
        &db,
        instant("2026-10-24T19:00:00Z").timestamp(),
        "p1",
        5,
        Some(11),
    );
    let mut input = request("2026-10-24", "2026-10-25");
    input["tz"] = json!("Asia/Kolkata");
    let response = run(&db, input.clone(), &pro());
    assert_eq!(
        response.series[0]
            .points
            .iter()
            .map(|p| p.requests)
            .collect::<Vec<_>>(),
        [3, 5]
    );
    raw(
        &db,
        "before-midnight",
        "2026-10-24T18:29:59Z",
        "p1",
        Some("project"),
        None,
        "0.000003",
    );
    raw(
        &db,
        "midnight",
        "2026-10-24T18:30:00Z",
        "p1",
        Some("project"),
        None,
        "0.000005",
    );
    input["groupBy"] = json!("project");
    let response = run(&db, input, &pro());
    assert_eq!(
        response.series[0]
            .points
            .iter()
            .map(|p| p.cost_micro_usd)
            .collect::<Vec<_>>(),
        [Some(3), Some(5)]
    );
}

#[test]
fn free_and_community_clamp_local_calendar_window_and_reject_every_group() {
    let db = Database::memory().unwrap();
    for (at, requests) in [
        ("2026-09-27T15:00:00Z", 100),
        ("2026-09-27T16:00:00Z", 3),
        ("2026-10-27T15:00:00Z", 5),
        ("2026-10-27T16:00:00Z", 200),
    ] {
        hourly(&db, instant(at).timestamp(), "p1", requests, Some(requests));
    }
    for status in [
        license(LicenseBuild::Official, LicenseTier::Free),
        license(LicenseBuild::Community, LicenseTier::Free),
        license(LicenseBuild::Community, LicenseTier::Pro),
        license(LicenseBuild::Community, LicenseTier::ProP3),
    ] {
        let mut input = request("2026-09-01", "2026-11-01");
        input["tz"] = json!("Asia/Hong_Kong"); // now is already Oct 27 here
        let response = run(&db, input.clone(), &status);
        let points = &response.series[0].points;
        assert_eq!(response.clamped_from.as_deref(), Some("2026-09-28"));
        assert_eq!(points.len(), 30);
        assert_eq!(points.first().unwrap().date_local, "2026-09-28");
        assert_eq!(points.last().unwrap().date_local, "2026-10-27");
        assert_eq!(points.first().unwrap().requests, 3);
        assert_eq!(points.last().unwrap().requests, 5);
        assert_eq!(points.iter().map(|point| point.requests).sum::<i64>(), 8);
        for group in ["provider", "project", "client"] {
            input["groupBy"] = json!(group);
            assert_eq!(
                query_usage_trend_at(&db, input.clone(), &status, instant("2026-10-26T16:30:00Z"))
                    .unwrap_err()
                    .code(),
                Some("PRO_REQUIRED")
            );
        }
        input.as_object_mut().unwrap().remove("groupBy");
        input["to"] = json!("2026-10-27");
        let response = run(&db, input.clone(), &status);
        assert_eq!(
            serde_json::to_value(&response).unwrap()["clampedFrom"],
            "2026-09-28"
        );
        input["from"] = json!("2026-09-28");
        input["to"] = json!("2026-10-27");
        assert!(run(&db, input.clone(), &status).clamped_from.is_none());
        input["from"] = json!("2026-10-26");
        input["to"] = json!("2026-11-02");
        let response = run(&db, input.clone(), &status);
        assert!(response.clamped_from.is_none());
        assert!(serde_json::to_value(&response)
            .unwrap()
            .get("clampedFrom")
            .is_none());
        assert_eq!(response.series[0].points.len(), 2);
        for (from, to) in [("2026-01-01", "2026-01-02"), ("2026-11-01", "2026-11-02")] {
            input["from"] = json!(from);
            input["to"] = json!(to);
            let response = run(&db, input.clone(), &status);
            assert_eq!(
                response.clamped_from.as_deref(),
                if from == "2026-01-01" {
                    Some("2026-09-28")
                } else {
                    None
                }
            );
            assert_eq!(response.series.len(), 1);
            assert_eq!(response.series[0].group_key, "all");
            assert!(response.series[0].points.is_empty());
        }
    }
    for tier in [LicenseTier::Pro, LicenseTier::ProP3] {
        let response = run(
            &db,
            request("2026-09-01", "2026-11-01"),
            &license(LicenseBuild::Official, tier),
        );
        assert!(response.clamped_from.is_none());
        assert_eq!(response.series[0].points.len(), 62);
    }
}

#[test]
fn malformed_query_is_coded_and_store_reads_remain_policy_free() {
    let db = Database::memory().unwrap();
    for (field, value) in [
        ("tz", json!("Not/A_Zone")),
        ("granularity", json!("hour")),
        ("filters", json!({"project": "x"})),
        ("filters", json!({"providerIds":[42]})),
        ("groupBy", json!("model")),
        ("from", json!("2026-2-01")),
        ("from", json!("-0001-01-01")),
        ("from", json!("2026-10-27")),
        ("to", json!("2026-02-30")),
    ] {
        let mut input = request("2026-10-24", "2026-10-26");
        input[field] = value;
        let error =
            query_usage_trend_at(&db, input, &pro(), instant("2026-10-26T12:00:00Z")).unwrap_err();
        assert_eq!(error.code(), Some("INVALID_ARGUMENT"));
    }
    raw(
        &db,
        "old",
        "2020-01-02T12:00:00Z",
        "p1",
        None,
        None,
        "0.123456",
    );
    let old = db
        .get_daily_trends(
            Some(instant("2020-01-01T00:00:00Z").timestamp()),
            Some(instant("2020-01-04T00:00:00Z").timestamp()),
            None,
            None,
            None,
        )
        .unwrap();
    assert_eq!(old.iter().map(|point| point.request_count).sum::<u64>(), 1);
    let json = serde_json::to_value(old).unwrap();
    assert!(json.is_array());
    assert!(json
        .as_array()
        .unwrap()
        .iter()
        .all(|point| point["totalCost"].is_string()));
}
