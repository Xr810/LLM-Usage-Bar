use crate::services::tray_usage::TrayUsageService;
use crate::usage::tray_snapshot::{TrayUsageSnapshot, TrayUsageWindows};
use futures::future::BoxFuture;
use std::sync::Arc;
use std::time::Duration;
use tauri::async_runtime::JoinHandle;
use tokio::sync::watch;

const MAX_POLL_INTERVAL: Duration = Duration::from_secs(15 * 60);

pub type TraySnapshotPublisher = Arc<dyn Fn(&TrayUsageSnapshot) + Send + Sync + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LocalWindowBoundary {
    today_start_at: i64,
    next_local_midnight_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PollDecision {
    Rebuild,
    Wait {
        duration: Duration,
        retain: LocalWindowBoundary,
    },
    Retry(Duration),
}

type Observe = Arc<dyn Fn() -> (i64, Option<LocalWindowBoundary>) + Send + Sync>;
type Waiter = Arc<dyn Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync>;
type Rebuild = Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>;

fn decide_poll(
    now_at: i64,
    retained: Option<LocalWindowBoundary>,
    current: Option<LocalWindowBoundary>,
) -> PollDecision {
    let Some(current) = current else {
        return PollDecision::Retry(MAX_POLL_INTERVAL);
    };
    if retained.is_some_and(|retained| {
        now_at >= retained.next_local_midnight_at
            || current.today_start_at != retained.today_start_at
    }) || now_at >= current.next_local_midnight_at
    {
        return PollDecision::Rebuild;
    }

    let seconds_until_boundary = current.next_local_midnight_at.saturating_sub(now_at) as u64;
    PollDecision::Wait {
        duration: Duration::from_secs(seconds_until_boundary).min(MAX_POLL_INTERVAL),
        retain: current,
    }
}

async fn run_scheduler_loop(
    observe: Observe,
    waiter: Waiter,
    rebuild: Rebuild,
    mut cancel_rx: watch::Receiver<bool>,
) {
    let mut retained = None;
    loop {
        if *cancel_rx.borrow() {
            return;
        }
        let (now_at, current) = observe();
        match decide_poll(now_at, retained, current) {
            PollDecision::Rebuild => {
                if wait_until_cancelled(&mut cancel_rx, rebuild()).await {
                    return;
                }
                retained = None;
            }
            PollDecision::Wait { duration, retain } => {
                retained = Some(retain);
                if wait_until_cancelled(&mut cancel_rx, waiter(duration)).await {
                    return;
                }
            }
            PollDecision::Retry(duration) => {
                log::warn!("tray usage local calendar window unavailable");
                if wait_until_cancelled(&mut cancel_rx, waiter(duration)).await {
                    return;
                }
            }
        }
    }
}

async fn wait_until_cancelled(
    cancel_rx: &mut watch::Receiver<bool>,
    wait: BoxFuture<'static, ()>,
) -> bool {
    tokio::select! {
        biased;
        changed = cancel_rx.changed() => {
            changed.is_err() || *cancel_rx.borrow()
        }
        _ = wait => false,
    }
}

pub struct TrayUsageSchedulerHandle {
    cancel: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
}

impl TrayUsageSchedulerHandle {
    pub async fn stop(mut self) {
        let _ = self.cancel.send(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for TrayUsageSchedulerHandle {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub fn start_local_midnight_scheduler(
    service: Arc<TrayUsageService>,
    publish: TraySnapshotPublisher,
) -> TrayUsageSchedulerHandle {
    let observe: Observe = Arc::new(observe_local_window);
    let waiter: Waiter = Arc::new(|duration| {
        Box::pin(async move {
            tokio::time::sleep(duration).await;
        })
    });
    let rebuild: Rebuild = Arc::new(move || {
        let service = service.clone();
        let publish = publish.clone();
        Box::pin(async move {
            service
                .rebuild_from_persisted(move |snapshot| publish(snapshot))
                .await;
        })
    });
    let (cancel, cancel_rx) = watch::channel(false);
    let task = tauri::async_runtime::spawn(run_scheduler_loop(observe, waiter, rebuild, cancel_rx));
    TrayUsageSchedulerHandle {
        cancel,
        task: Some(task),
    }
}

fn observe_local_window() -> (i64, Option<LocalWindowBoundary>) {
    let now = chrono::Local::now();
    let now_at = now.timestamp();
    let boundary = TrayUsageWindows::from_local_now(now).map(|windows| LocalWindowBoundary {
        today_start_at: windows.today_start_at,
        next_local_midnight_at: windows.next_local_midnight_at,
    });
    (now_at, boundary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::future::pending;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use tokio::sync::Notify;
    use tokio::time::timeout;

    fn boundary(today_start_at: i64, next_local_midnight_at: i64) -> LocalWindowBoundary {
        LocalWindowBoundary {
            today_start_at,
            next_local_midnight_at,
        }
    }

    #[test]
    fn ordinary_wait_is_capped_at_fifteen_minutes() {
        let current = boundary(0, 10_000);
        assert_eq!(
            decide_poll(100, None, Some(current)),
            PollDecision::Wait {
                duration: Duration::from_secs(900),
                retain: current,
            }
        );
    }

    #[test]
    fn nearer_calendar_boundary_wins_over_the_cap() {
        let current = boundary(0, 450);
        assert_eq!(
            decide_poll(100, None, Some(current)),
            PollDecision::Wait {
                duration: Duration::from_secs(350),
                retain: current,
            }
        );
    }

    #[test]
    fn reaching_or_passing_the_retained_boundary_rebuilds() {
        let retained = boundary(0, 1_000);
        let current = boundary(0, 1_000);
        assert_eq!(
            decide_poll(1_000, Some(retained), Some(current)),
            PollDecision::Rebuild
        );
        assert_eq!(
            decide_poll(1_001, Some(retained), Some(current)),
            PollDecision::Rebuild
        );
    }

    #[test]
    fn synthetic_short_and_long_days_follow_the_supplied_boundary() {
        let short_day = boundary(0, 82_800);
        assert_eq!(
            decide_poll(82_700, Some(short_day), Some(short_day)),
            PollDecision::Wait {
                duration: Duration::from_secs(100),
                retain: short_day,
            }
        );

        let long_day = boundary(0, 90_000);
        assert_eq!(
            decide_poll(86_400, Some(long_day), Some(long_day)),
            PollDecision::Wait {
                duration: Duration::from_secs(900),
                retain: long_day,
            }
        );
    }

    #[test]
    fn changed_local_day_rebuilds_before_the_old_deadline() {
        assert_eq!(
            decide_poll(
                5_000,
                Some(boundary(0, 90_000)),
                Some(boundary(3_600, 93_600)),
            ),
            PollDecision::Rebuild
        );
    }

    #[test]
    fn missing_calendar_window_retries_with_a_fixed_cap() {
        assert_eq!(
            decide_poll(100, Some(boundary(0, 1_000)), None),
            PollDecision::Retry(Duration::from_secs(900))
        );
    }

    #[tokio::test]
    async fn rebuild_reobserves_a_fresh_post_rebuild_boundary() {
        let observations = Arc::new(Mutex::new(VecDeque::from([
            (100, Some(boundary(0, 100))),
            (101, Some(boundary(100, 500))),
        ])));
        let rebuilds = Arc::new(AtomicUsize::new(0));
        let waited = Arc::new(Mutex::new(Vec::new()));
        let (cancel, cancel_rx) = watch::channel(false);

        let observe: Observe = {
            let observations = observations.clone();
            Arc::new(move || observations.lock().unwrap().pop_front().unwrap())
        };
        let waiter: Waiter = {
            let waited = waited.clone();
            let cancel = cancel.clone();
            Arc::new(move |duration| {
                waited.lock().unwrap().push(duration);
                let cancel = cancel.clone();
                Box::pin(async move {
                    let _ = cancel.send(true);
                })
            })
        };
        let rebuild: Rebuild = {
            let rebuilds = rebuilds.clone();
            Arc::new(move || {
                rebuilds.fetch_add(1, Ordering::SeqCst);
                Box::pin(async {})
            })
        };

        timeout(
            Duration::from_secs(1),
            run_scheduler_loop(observe, waiter, rebuild, cancel_rx),
        )
        .await
        .unwrap();

        assert_eq!(rebuilds.load(Ordering::SeqCst), 1);
        assert_eq!(
            waited.lock().unwrap().as_slice(),
            [Duration::from_secs(399)]
        );
        assert!(observations.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn cancellation_interrupts_a_pending_waiter() {
        let entered = Arc::new(Notify::new());
        let observe: Observe = Arc::new(|| (100, Some(boundary(0, 10_000))));
        let waiter: Waiter = {
            let entered = entered.clone();
            Arc::new(move |_| {
                let entered = entered.clone();
                Box::pin(async move {
                    entered.notify_one();
                    pending::<()>().await;
                })
            })
        };
        let rebuild: Rebuild = Arc::new(|| Box::pin(async {}));
        let (cancel, cancel_rx) = watch::channel(false);
        let task = tokio::spawn(run_scheduler_loop(observe, waiter, rebuild, cancel_rx));
        entered.notified().await;
        let _ = cancel.send(true);

        timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn cancellation_interrupts_an_inflight_rebuild() {
        let rebuild_started = Arc::new(Notify::new());
        let observe: Observe = Arc::new(|| (100, Some(boundary(0, 100))));
        let waiter: Waiter = Arc::new(|_| Box::pin(pending::<()>()));
        let rebuild: Rebuild = {
            let rebuild_started = rebuild_started.clone();
            Arc::new(move || {
                let rebuild_started = rebuild_started.clone();
                Box::pin(async move {
                    rebuild_started.notify_one();
                    pending::<()>().await;
                })
            })
        };
        let (cancel, cancel_rx) = watch::channel(false);

        let task = tokio::spawn(run_scheduler_loop(observe, waiter, rebuild, cancel_rx));
        timeout(Duration::from_secs(1), rebuild_started.notified())
            .await
            .expect("rebuild should start");
        cancel.send(true).unwrap();
        timeout(Duration::from_secs(1), task)
            .await
            .expect("scheduler should cancel an in-flight rebuild")
            .expect("scheduler task should not panic");
    }

    #[test]
    fn production_observer_uses_the_calendar_window_projector() {
        let (now_at, boundary) = observe_local_window();
        let boundary = boundary.expect("local calendar window");
        assert!(boundary.today_start_at <= now_at);
        assert!(boundary.next_local_midnight_at > now_at);
    }
}
