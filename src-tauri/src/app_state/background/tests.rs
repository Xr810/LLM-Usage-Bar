use super::*;
use crate::{app_state::AppState, store::Database};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

const DEADLINE: Duration = Duration::from_secs(5);

fn sync_fixture(
    db: &Database,
    parser: &dyn crate::ingest::SessionLogParser,
    file: &std::path::Path,
    stopped: &dyn Fn() -> bool,
) -> (u32, u32) {
    let cursors = db.list_usage_sync_cursors(parser.source()).unwrap();
    let mut offsets = cursors
        .iter()
        .map(|c| (c.cursor_key.clone(), (c.modified_at_ns, c.line_offset)))
        .collect();
    let details = cursors
        .into_iter()
        .map(|c| (c.cursor_key.clone(), c))
        .collect();
    crate::ingest::sync_file_with_parser_until(
        db,
        parser,
        &parser.write_profile(),
        file,
        Some("shutdown-fixture"),
        &mut offsets,
        &details,
        &mut Vec::new(),
        stopped,
    )
    .unwrap()
}

/// Pause inside the real INSERT, not before sync or after an arbitrary sleep.
/// One already-committed event and one in-flight event must survive; the other
/// 9,999 events must wait for restart without advancing the old file cursor.
async fn shutdown_during_real_ingestion(codex: bool) {
    use crate::ingest::{claude::ClaudeParser, codex::CodexParser, SessionLogParser};
    use crate::model::{BillingKind, TokenSource, UsageProviderInput};
    use std::sync::atomic::Ordering;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ingestion.db");
    let file = directory.path().join("session.jsonl");
    let source = if codex { "codex" } else { "claude" };
    let db = Arc::new(Database::init_at(&path).unwrap());
    db.save_usage_provider(&UsageProviderInput {
        id: "shutdown-fixture".into(),
        name: "Disposable shutdown fixture".into(),
        billing_kind: BillingKind::Subscription,
        product_group_id: if codex { "codex" } else { "claude" }.into(),
        token_sources: vec![TokenSource::SessionLog],
        session_source_bindings: None,
        quota_source: None,
        quota_interval_seconds: Some(300),
        route_app_type: None,
        route_config: None,
        quota_config: None,
        enabled: true,
    })
    .unwrap();
    db.set_usage_source_binding(source, "shutdown-fixture")
        .unwrap();

    let line = |i: u32| {
        if codex {
            // Asymmetric cumulative values expose replay baseline mistakes:
            // stored input includes cached input: the delta is 10, with 7 cached.
            // Only cost calculation subtracts cache to obtain 3 billable input.
            format!(
                r#"{{"timestamp":"2026-07-14T00:00:00Z","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{},"cached_input_tokens":{},"output_tokens":{}}}}}}}}}"#,
                i * 10,
                i * 7,
                i * 5
            )
        } else {
            format!(
                r#"{{"type":"assistant","message":{{"id":"msg_shutdown_{i}","model":"claude-sonnet-4-5","usage":{{"input_tokens":3,"output_tokens":5,"cache_read_input_tokens":7,"cache_creation_input_tokens":11}},"stop_reason":"end_turn"}},"timestamp":"2026-07-14T00:00:00Z","sessionId":"shutdown"}}"#
            )
        }
    };
    std::fs::write(&file, format!("{}\n", line(1))).unwrap();
    let parser: Box<dyn SessionLogParser> = if codex {
        Box::new(CodexParser::new(directory.path().into(), vec![], 2))
    } else {
        Box::new(ClaudeParser)
    };
    assert_eq!(sync_fixture(&db, parser.as_ref(), &file, &|| false), (1, 0));
    let old_cursor = db.list_usage_sync_cursors(source).unwrap();
    let mut content = format!("{}\n", line(1));
    for i in 2..=10_001 {
        content.push_str(&format!("{}\n", line(i)));
    }
    std::fs::write(&file, content).unwrap();
    // No sleep to hope mtime changed: explicitly give the changed fixture a
    // timestamp later than the committed cursor, including coarse filesystems.
    filetime::set_file_mtime(
        &file,
        filetime::FileTime::from_unix_time(old_cursor[0].modified_at_ns / 1_000_000_000 + 2, 0),
    )
    .unwrap();

    let state = AppState::new(db.clone());
    let stop = state.session_sync_stop();
    let (entered, ready) = oneshot::channel();
    let mut entered = Some(entered);
    let (release, resume) = mpsc::channel();
    db.conn
        .lock()
        .unwrap()
        .update_hook(Some(move |action, _: &str, table: &str, _: i64| {
            if action == rusqlite::hooks::Action::SQLITE_INSERT && table == "usage_events" {
                if let Some(entered) = entered.take() {
                    entered.send(()).unwrap();
                    resume.recv_timeout(DEADLINE).unwrap();
                }
            }
        }))
        .unwrap();
    let worker_db = db.clone();
    let worker_file = file.clone();
    let root = directory.path().to_path_buf();
    let (finished, mut observed) = oneshot::channel();
    state.spawn_background(async move {
        let parser: Box<dyn SessionLogParser> = if codex {
            Box::new(CodexParser::new(root, vec![], 2))
        } else {
            Box::new(ClaudeParser)
        };
        let result = sync_fixture(&worker_db, parser.as_ref(), &worker_file, &|| {
            stop.load(Ordering::Acquire)
        });
        finished.send(result).unwrap();
    });
    tokio::time::timeout(DEADLINE, ready)
        .await
        .unwrap()
        .unwrap();
    let started = Instant::now();
    let stopping = state.take_background_tasks().stop();
    tokio::pin!(stopping);
    assert!(futures::poll!(&mut stopping).is_pending());
    // Repeated cleanup cannot take the first owner's in-flight handle or reset
    // the stop signal. Desktop duplicate quit gating has separate tests.
    state.take_background_tasks().stop().await;
    state.take_background_tasks().stop().await;
    assert!(state.session_sync_stop().load(Ordering::Acquire));
    release.send(()).unwrap();
    tokio::time::timeout(DEADLINE, &mut stopping).await.unwrap();
    assert_eq!(observed.try_recv().unwrap(), (1, 0));
    eprintln!("{source} 10k in-flight ingestion shutdown: {:?} (fixture limit {DEADLINE:?}, not product SLA)", started.elapsed());
    drop(stopping);
    drop(state);
    assert_eq!(
        Arc::strong_count(&db),
        1,
        "worker must release its database owner"
    );
    drop(db);

    let reopened = Database::init_at(&path).unwrap();
    let cursors = reopened.list_usage_sync_cursors(source).unwrap();
    assert_eq!(
        cursors, old_cursor,
        "partial file must not publish offset, mtime, identity promotion or parser state"
    );
    {
        let conn = reopened.conn.lock().unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM usage_events", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            conn.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
    }
    assert_eq!(
        sync_fixture(&reopened, parser.as_ref(), &file, &|| false),
        (9_999, 1)
    );
    assert_eq!(
        sync_fixture(&reopened, parser.as_ref(), &file, &|| false),
        (0, 0)
    );
    assert_eq!(
        reopened.list_usage_sync_cursors(source).unwrap()[0].line_offset,
        10_001
    );
    let conn = reopened.conn.lock().unwrap();
    // Record actual persisted values before assertions, so a failure can be
    // diagnosed without mistaking billable input for the storage contract.
    for table in ["usage_events", "proxy_request_logs"] {
        let mut statement = conn.prepare(&format!(
            "SELECT input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens, COUNT(*) FROM {table} GROUP BY input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens"
        )).unwrap();
        let tuples: Vec<(i64, i64, i64, i64, i64)> = statement
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        eprintln!("{source} restored {table} token tuples (input, output, read, creation, count): {tuples:?}");
    }
    // Independently derived from the fixture: cumulative input rises by 10
    // including cache, whereas Claude explicitly supplies 3 fresh input.
    let expected_input = if codex { 10 } else { 3 };
    let expected_creation = if codex { 0 } else { 11 };
    let counts: (i64, i64, i64) = conn.query_row(
        "SELECT COUNT(*), COUNT(DISTINCT event_id), SUM(CASE WHEN input_tokens=?1 AND output_tokens=5 AND cache_read_tokens=7 AND cache_creation_tokens=?2 THEN 1 ELSE 0 END) FROM usage_events",
        [expected_input, expected_creation], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).unwrap();
    assert_eq!(counts, (10_001, 10_001, 10_001));
    let legacy: (i64, i64, i64) = conn.query_row(
        "SELECT COUNT(*), COUNT(DISTINCT request_id), SUM(CASE WHEN input_tokens=?1 AND output_tokens=5 AND cache_read_tokens=7 AND cache_creation_tokens=?2 THEN 1 ELSE 0 END) FROM proxy_request_logs",
        [expected_input, expected_creation], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).unwrap();
    assert_eq!(legacy, (10_001, 10_001, 10_001));
    assert_eq!(
        conn.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
}

#[tokio::test]
async fn shutdown_during_real_claude_ingestion_replays_unfinished_file() {
    shutdown_during_real_ingestion(false).await;
}

#[tokio::test]
async fn shutdown_during_real_codex_ingestion_preserves_cumulative_baseline() {
    shutdown_during_real_ingestion(true).await;
}

#[tokio::test]
async fn shutdown_during_startup_joins_initializer_before_it_can_spawn_workers() {
    let tasks = Arc::new(BackgroundTasks::default());
    let startup_tasks = tasks.clone();
    let (started, ready) = oneshot::channel();
    let (resume, paused) = oneshot::channel::<()>();
    let (spawned, mut worker_result) = oneshot::channel::<()>();
    let (finished, mut startup_result) = oneshot::channel::<()>();
    tasks.spawn(async move {
        let _guard = finished;
        started.send(()).unwrap();
        paused.await.unwrap();
        startup_tasks.spawn(async move {
            spawned.send(()).unwrap();
        });
    });
    tokio::time::timeout(DEADLINE, ready)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(DEADLINE, tasks.take().stop())
        .await
        .unwrap();
    // Check completion immediately: requesting abort without joining is not
    // sufficient, and the initializer must not reach child registration.
    assert_eq!(
        startup_result.try_recv(),
        Err(oneshot::error::TryRecvError::Closed)
    );
    assert_eq!(
        worker_result.try_recv(),
        Err(oneshot::error::TryRecvError::Closed)
    );
    assert!(resume.send(()).is_err());
    assert_eq!(Arc::strong_count(&tasks), 1);
}

#[tokio::test]
async fn shutdown_before_startup_rejects_every_worker_and_is_repeatable() {
    let state = AppState::new(Arc::new(Database::memory().unwrap()));
    let shutdown = state.take_background_tasks();
    assert!(!state.start_quota_scheduler(Arc::new(|_| Box::pin(async {}))));
    assert!(!state.start_midnight_scheduler(Arc::new(|_| {})));
    assert!(!state.start_official_pricing_scheduler());
    assert!(!state.start_provider_key_usage_scheduler());
    // A rejected future must be dropped, not detached or queued for execution.
    let (dropped, observed) = oneshot::channel::<()>();
    state.spawn_background(async move {
        dropped.send(()).unwrap();
    });
    assert!(tokio::time::timeout(DEADLINE, observed)
        .await
        .unwrap()
        .is_err());
    tokio::time::timeout(DEADLINE, shutdown.stop())
        .await
        .unwrap();
    tokio::time::timeout(DEADLINE, state.take_background_tasks().stop())
        .await
        .unwrap();
    assert!(!state.start_official_pricing_scheduler());
}

#[tokio::test]
async fn shutdown_joins_completed_panicked_and_suspended_workers() {
    let tasks = BackgroundTasks::default();
    for panics in [false, true] {
        let (finished, observed) = oneshot::channel::<()>();
        tasks.spawn(async move {
            let _guard = finished;
            assert!(!panics, "intentional worker failure");
        });
        assert!(tokio::time::timeout(DEADLINE, observed)
            .await
            .unwrap()
            .is_err());
    }
    let (started, ready) = oneshot::channel();
    let (dropped, mut observed) = oneshot::channel::<()>();
    tasks.spawn(async move {
        let _guard = dropped;
        started.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    tokio::time::timeout(DEADLINE, ready)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(DEADLINE, tasks.take().stop())
        .await
        .unwrap();
    assert_eq!(
        observed.try_recv(),
        Err(oneshot::error::TryRecvError::Closed),
        "shutdown must join cancelled tasks"
    );
}

#[tokio::test]
async fn shutdown_waits_for_inflight_transaction_and_preserves_committed_rows() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shutdown.db");
    let db = Arc::new(Database::init_at(&path).unwrap());
    let state = AppState::new(db.clone());
    let (entered, ready) = oneshot::channel();
    let (release, resume) = mpsc::channel();
    let (dropped, mut observed) = oneshot::channel::<()>();
    state.spawn_background(async move {
        let _guard = dropped;
        {
            let mut connection = db.conn.lock().unwrap();
            connection
                .execute_batch("CREATE TABLE shutdown_probe (id INTEGER PRIMARY KEY, amount TEXT);")
                .unwrap();
            let transaction = connection.transaction().unwrap();
            transaction
                .execute("INSERT INTO shutdown_probe VALUES (1, '12.34567')", [])
                .unwrap();
            entered.send(()).unwrap();
            // Deliberately remain in a synchronous write step while abort is
            // requested. A timeout bounds a broken test without timing the race.
            resume.recv_timeout(DEADLINE).unwrap();
            transaction
                .execute("INSERT INTO shutdown_probe VALUES (2, '0.00009')", [])
                .unwrap();
            transaction.commit().unwrap();
        }
        std::future::pending::<()>().await;
    });
    tokio::time::timeout(DEADLINE, ready)
        .await
        .unwrap()
        .unwrap();
    let started = Instant::now();
    let stopping = state.take_background_tasks().stop();
    tokio::pin!(stopping);
    assert!(futures::poll!(&mut stopping).is_pending());
    release.send(()).unwrap();
    tokio::time::timeout(DEADLINE, &mut stopping).await.unwrap();
    assert_eq!(
        observed.try_recv(),
        Err(oneshot::error::TryRecvError::Closed)
    );
    eprintln!(
        "in-flight transaction shutdown: {:?} (limit {DEADLINE:?})",
        started.elapsed()
    );
    drop(state);

    // Reopen the real on-disk database, rather than inspecting the worker's
    // connection or accepting 'no panic' as evidence of cancellation safety.
    let reopened = Database::init_at(&path).unwrap();
    let connection = reopened.conn.lock().unwrap();
    let rows: Vec<(i64, String)> = connection
        .prepare("SELECT id, amount FROM shutdown_probe ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows, vec![(1, "12.34567".into()), (2, "0.00009".into())]);
    assert_eq!(
        connection
            .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
}
