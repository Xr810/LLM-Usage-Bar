use super::*;
use crate::{app_state::AppState, store::Database};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

const DEADLINE: Duration = Duration::from_secs(5);

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
