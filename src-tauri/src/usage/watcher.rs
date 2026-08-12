//! 文件监听层:把四个数据源的目录变化映射成 `WatcherSchedule::mark_dirty`,
//! 并承载「事件驱动同步」的驱动辅助(挑 due、退避回填、慢速兜底)。
//!
//! 跨平台由 notify 承担:macOS 走 FSEvents(默认 feature)、Windows 走
//! ReadDirectoryChangesW、Linux 走 inotify。notify 8.2 的 FSEvents 后端把
//! latency 硬编码为 0(kFSEventStreamCreateFlagNoDefer)且不暴露配置口,
//! 「合并密集写入」的语义因此由本层与 watcher_state 的脏代数 + 最小同步间隔
//! 共同实现,而不是靠 FSEvents 的延迟。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use notify::{RecursiveMode, Watcher};
use tokio::sync::Notify;

use super::watcher_state::{SourceId, WatcherSchedule};

/// 驱动循环每次空闲等待的上限(秒):文件事件到达时会被立刻唤醒,
/// 超时仅用于慢速兜底检查,所以等待本身几乎不耗电。
pub(crate) const DRIVER_WAKE_TIMEOUT_SECS: u64 = 15;

/// 慢速兜底周期(秒):每 15 分钟把四个源全部标脏一次。
///
/// FSEvents 在睡眠/唤醒、网络卷、事件队列溢出时会丢事件;没有这条兜底,
/// 监听彻底失效时用量会「永久停更」——这是最难排查的故障形态。保留这条
/// 是为了保证最坏情况下系统退化为「每 15 分钟同步一次」,而不是「永远不同步」。
pub(crate) const SLOW_FALLBACK_INTERVAL_SECS: u64 = 15 * 60;

/// 同一个源两次同步之间的最小间隔(秒)。
///
/// 这是「退避/去抖窗口」,不是轮询周期:事件到来时调度器只挑「脏了 &&
/// 没在同步 && 过了最小间隔」的源,60 秒天然替代旧的 60 秒全量轮询节流。
pub(crate) const SYNC_MIN_INTERVAL_SECS: u64 = 60;

/// 当前 Unix 秒(u64),驱动循环与调度器的时间基准。
/// 生产环境使用;测试用假时钟注入 `drive_due_syncs` / `apply_fallback_if_due`。
pub(crate) fn unix_seconds_now() -> u64 {
    chrono::Utc::now().timestamp().max(0) as u64
}

/// 四个数据源的监听根目录。
///
/// 路径来源(不在本文件里重新发明,与同步扫描共用同一批函数):
/// - claude:  `config::get_claude_config_dir()/projects`
/// - codex:   `agent_paths::get_codex_config_dir()`
/// - gemini:  `agent_paths::get_gemini_dir()`
/// - opencode:`agent_paths::get_opencode_db_path()` 所在目录
///   (监听目录而不是 db 文件本身:能捕获 db 首次创建与 -wal/-shm 的写入)
#[derive(Debug, Clone)]
pub(crate) struct SourceRoots {
    claude_projects: PathBuf,
    codex: PathBuf,
    gemini: PathBuf,
    opencode_root: PathBuf,
}

impl SourceRoots {
    /// 按当前运行配置解析四个源的监听根目录。
    pub(crate) fn from_runtime_config() -> Self {
        let opencode_db = crate::agent_paths::get_opencode_db_path();
        // parent 为 None 的极端情况(如 OPENCODE_DB="/")回退为监听文件本身。
        let opencode_root = opencode_db
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or(opencode_db);
        Self {
            claude_projects: crate::config::get_claude_config_dir().join("projects"),
            codex: crate::agent_paths::get_codex_config_dir(),
            gemini: crate::agent_paths::get_gemini_dir(),
            opencode_root,
        }
    }

    /// 纯函数:绝对路径 → 所属数据源;不属于任何源返回 None。
    ///
    /// 四个根在真实配置下互不为前缀,顺序只影响理论上重叠的测试构造。
    pub(crate) fn source_for_path(&self, path: &Path) -> Option<SourceId> {
        if path.starts_with(&self.claude_projects) {
            Some(SourceId::Claude)
        } else if path.starts_with(&self.codex) {
            Some(SourceId::Codex)
        } else if path.starts_with(&self.gemini) {
            Some(SourceId::Gemini)
        } else if path.starts_with(&self.opencode_root) {
            Some(SourceId::OpenCode)
        } else {
            None
        }
    }
}

/// 已启动的监听器句柄:Drop 时停止底层监听线程。
pub(crate) struct UsageWatcher {
    /// 只为了持有:监听线程的生命周期绑定在这个句柄上,字段本身从不被读取。
    _watcher: notify::RecommendedWatcher,
    schedule: Arc<Mutex<WatcherSchedule>>,
}

impl UsageWatcher {
    /// 启动监听。事件回调在 notify 自己的线程上运行,只做两件轻量的事:
    /// mark_dirty(内存里的脏代数 +1)与唤醒驱动循环,不在回调里做任何 I/O。
    ///
    /// 根目录不存在的源会被跳过(用户没装 codex/gemini 是常态),单个源的
    /// watch 失败只影响它自己;任何情况下都不让 app 启动失败——监听彻底
    /// 不可用时返回 None,由 15 分钟慢速兜底接管。
    pub(crate) fn start(
        roots: SourceRoots,
        schedule: Arc<Mutex<WatcherSchedule>>,
        wake: Arc<Notify>,
    ) -> Option<Self> {
        let targets: [(SourceId, &Path, RecursiveMode); 4] = [
            (
                SourceId::Claude,
                &roots.claude_projects,
                RecursiveMode::Recursive,
            ),
            (SourceId::Codex, &roots.codex, RecursiveMode::Recursive),
            (SourceId::Gemini, &roots.gemini, RecursiveMode::Recursive),
            (
                SourceId::OpenCode,
                &roots.opencode_root,
                RecursiveMode::Recursive,
            ),
        ];

        // targets 借用了 roots 的引用,回调闭包要 move 进自己的副本,避免借用冲突。
        let roots_for_callback = roots.clone();
        let schedule_callback = Arc::clone(&schedule);
        let wake_callback = Arc::clone(&wake);
        let mut watcher =
            match notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                match result {
                    Ok(event) => {
                        for path in &event.paths {
                            if let Some(source) = roots_for_callback.source_for_path(path) {
                                if let Ok(mut schedule) = schedule_callback.lock() {
                                    schedule.mark_dirty(source);
                                }
                                wake_callback.notify_one();
                            }
                        }
                    }
                    Err(error) => {
                        // 监听线程自身的错误(如 inotify 队列溢出):只打 debug 避免刷屏,
                        // 丢掉的更新由 15 分钟慢速兜底补偿。
                        log::debug!("文件监听错误: {error}");
                    }
                }
            }) {
                Ok(watcher) => watcher,
                Err(error) => {
                    // 只有当前平台没有任何可用监听后端时才会走到这里:
                    // 退化为纯慢速兜底,绝不让 app 启动失败。
                    log::warn!("文件监听不可用,退化为 15 分钟慢速兜底: {error}");
                    return None;
                }
            };

        for (source, root, mode) in targets {
            // 根目录不存在说明用户没装对应 CLI,属常态:跳过、不重试、不刷屏。
            if !root.exists() {
                log::debug!("跳过不存在的监听根目录 {root:?}({source:?})");
                continue;
            }
            if let Err(error) = watcher.watch(root, mode) {
                // 单个源监听失败不影响其他源;该源退化为慢速兜底覆盖。
                log::warn!("监听 {root:?} 失败({source:?}): {error}");
            }
        }

        Some(Self {
            _watcher: watcher,
            schedule,
        })
    }

    /// 关闭调度(驱动循环看到 is_shutdown 后退出);监听线程随本句柄 Drop 停止。
    pub(crate) fn stop(&self) {
        if let Ok(mut schedule) = self.schedule.lock() {
            schedule.shutdown();
        }
    }
}

/// 挑出当前 due 的源并逐个同步,把结果回填给调度器。
///
/// 同步在锁外执行,事件线程的 mark_dirty 不会被长时间阻塞;begin_due/finish
/// 的 generation + epoch 配对由 watcher_state 保证:同步期间的新事件只推进
/// 脏代数,同一源绝不会并发跑两趟。`now` 与 `sync` 均可注入,便于假时钟测试。
pub(crate) fn drive_due_syncs<F, C>(schedule: &Arc<Mutex<WatcherSchedule>>, now: C, mut sync: F)
where
    C: Fn() -> u64,
    F: FnMut(SourceId) -> Result<(), ()>,
{
    let due = schedule.lock().unwrap().begin_due(now());
    for (source, generation, epoch) in due {
        let result = sync(source);
        let done_at = now();
        schedule
            .lock()
            .unwrap()
            .finish(source, generation, epoch, done_at, result);
    }
}

/// 慢速兜底:距上次兜底超过 [`SLOW_FALLBACK_INTERVAL_SECS`] 就把四个源全部标脏。
///
/// 见 [`SLOW_FALLBACK_INTERVAL_SECS`] 的注释:这是「监听彻底失效时退化为
/// 每 15 分钟同步一次」的安全底线,不是优化项。
pub(crate) fn apply_fallback_if_due(
    schedule: &Arc<Mutex<WatcherSchedule>>,
    now: u64,
    last_fallback_second: &mut u64,
) {
    if now.saturating_sub(*last_fallback_second) >= SLOW_FALLBACK_INTERVAL_SECS {
        schedule.lock().unwrap().mark_all_dirty();
        *last_fallback_second = now;
    }
}

/// 进程内唯一监听实例的全局句柄(app 退出时从这里取走并停止)。
static ACTIVE_USAGE_WATCHER: OnceLock<Mutex<Option<UsageWatcher>>> = OnceLock::new();

/// 启动文件监听并登记全局句柄。重复调用会替换旧实例(旧实例 Drop 时停止监听线程)。
pub(crate) fn start_usage_watcher(schedule: Arc<Mutex<WatcherSchedule>>, wake: Arc<Notify>) {
    let watcher = UsageWatcher::start(SourceRoots::from_runtime_config(), schedule, wake);
    if let Ok(mut slot) = ACTIVE_USAGE_WATCHER.get_or_init(Default::default).lock() {
        *slot = watcher;
    }
}

/// 应用退出前调用:关闭调度(驱动循环看到 is_shutdown 后退出)并销毁监听线程。
/// 幂等:重复调用无副作用。
pub(crate) fn stop_usage_watcher() {
    if let Ok(mut slot) = ACTIVE_USAGE_WATCHER.get_or_init(Default::default).lock() {
        if let Some(watcher) = slot.take() {
            watcher.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    /// 用临时目录构造四个互不重叠的监听根,测试绝不触碰真实 HOME。
    fn test_roots() -> (tempfile::TempDir, SourceRoots) {
        let dir = tempfile::tempdir().expect("create tempdir");
        let roots = SourceRoots {
            claude_projects: dir.path().join("claude-projects"),
            codex: dir.path().join("codex"),
            gemini: dir.path().join("gemini"),
            opencode_root: dir.path().join("opencode-data"),
        };
        (dir, roots)
    }

    fn test_schedule() -> Arc<Mutex<WatcherSchedule>> {
        Arc::new(Mutex::new(WatcherSchedule::new(SYNC_MIN_INTERVAL_SECS)))
    }

    #[test]
    fn source_for_path_maps_each_root_and_deep_paths() {
        let (dir, roots) = test_roots();

        // 四个源各一个正例
        assert_eq!(
            roots.source_for_path(&roots.claude_projects.join("p/session/1.jsonl")),
            Some(SourceId::Claude)
        );
        assert_eq!(
            roots.source_for_path(&roots.codex.join("sessions/abc/rollouts/1.jsonl")),
            Some(SourceId::Codex)
        );
        assert_eq!(
            roots.source_for_path(&roots.gemini.join("history/session.json")),
            Some(SourceId::Gemini)
        );
        assert_eq!(
            roots.source_for_path(&roots.opencode_root.join("opencode.db")),
            Some(SourceId::OpenCode)
        );

        // 根目录本身也算
        assert_eq!(
            roots.source_for_path(&roots.claude_projects),
            Some(SourceId::Claude)
        );

        // 子目录深处的路径(如 claude 的 projects/项目/SESSION/subagents/workflows)
        assert_eq!(
            roots.source_for_path(
                &roots
                    .claude_projects
                    .join("proj/SESSION/subagents/workflows/deep.jsonl")
            ),
            Some(SourceId::Claude)
        );

        // 不属于任何源的路径
        assert_eq!(
            roots.source_for_path(Path::new("/tmp/unrelated/x.jsonl")),
            None
        );
        // 前缀相似但不同源("codx" ≠ "codex")
        assert_eq!(roots.source_for_path(&dir.path().join("codx/y")), None);
    }

    #[test]
    fn driver_picks_dirty_sources_and_finishes_them() {
        let schedule = test_schedule();
        let clock = Cell::new(0u64);
        let calls = RefCell::new(Vec::new());

        schedule.lock().unwrap().mark_dirty(SourceId::Claude);
        drive_due_syncs(
            &schedule,
            || clock.get(),
            |source| {
                calls.borrow_mut().push(source);
                Ok(())
            },
        );

        assert_eq!(*calls.borrow(), vec![SourceId::Claude]);
        assert!(!schedule.lock().unwrap().state(SourceId::Claude).is_dirty());
    }

    #[test]
    fn source_is_never_picked_twice_while_a_sync_is_running() {
        let schedule = test_schedule();
        let clock = Cell::new(0u64);
        let calls = RefCell::new(0u32);

        schedule.lock().unwrap().mark_dirty(SourceId::Codex);
        {
            let schedule_in_sync = Arc::clone(&schedule);
            drive_due_syncs(
                &schedule,
                || clock.get(),
                |source| {
                    assert_eq!(source, SourceId::Codex);
                    *calls.borrow_mut() += 1;
                    // 同步进行中:该源已被标记 syncing,begin_due 不会再挑出它
                    assert!(schedule_in_sync
                        .lock()
                        .unwrap()
                        .begin_due(clock.get())
                        .is_empty());
                    // 同步期间的新事件:推进脏代数,留到下一轮
                    schedule_in_sync.lock().unwrap().mark_dirty(SourceId::Codex);
                    Ok(())
                },
            );
        }

        // 同步结束后仍脏,但没到 60 秒最小间隔,不能立刻重跑
        assert!(schedule.lock().unwrap().state(SourceId::Codex).is_dirty());
        drive_due_syncs(&schedule, || clock.get(), |_| unreachable!("not due yet"));
        assert_eq!(*calls.borrow(), 1);

        // 过了最小间隔,带着新代数再跑一次
        clock.set(SYNC_MIN_INTERVAL_SECS);
        drive_due_syncs(
            &schedule,
            || clock.get(),
            |_| {
                *calls.borrow_mut() += 1;
                Ok(())
            },
        );
        assert_eq!(*calls.borrow(), 2);
        assert!(!schedule.lock().unwrap().state(SourceId::Codex).is_dirty());
    }

    #[test]
    fn failed_syncs_back_off_by_one_two_four_then_six_intervals() {
        let schedule = test_schedule();
        let clock = Cell::new(0u64);
        let calls = RefCell::new(0u32);

        schedule.lock().unwrap().mark_dirty(SourceId::Gemini);

        // 60 秒最小间隔 × 退避倍数 [1, 2, 4, 6]
        for (index, deadline) in [60u64, 180, 420, 780].iter().enumerate() {
            drive_due_syncs(
                &schedule,
                || clock.get(),
                |_| {
                    *calls.borrow_mut() += 1;
                    Err(())
                },
            );
            let state = schedule.lock().unwrap().state(SourceId::Gemini).clone();
            assert_eq!(state.next_allowed_second(), *deadline);
            assert_eq!(state.consecutive_failures(), (index + 1) as u32);
            // 未到下一次允许时间:即使标脏也不会挑出
            schedule.lock().unwrap().mark_dirty(SourceId::Gemini);
            drive_due_syncs(&schedule, || clock.get(), |_| unreachable!("backed off"));
            clock.set(*deadline);
        }
        assert_eq!(*calls.borrow(), 4);

        // 恢复成功后失败计数清零,按 60 秒最小间隔排下一次
        clock.set(780);
        drive_due_syncs(
            &schedule,
            || clock.get(),
            |_| {
                *calls.borrow_mut() += 1;
                Ok(())
            },
        );
        let state = schedule.lock().unwrap().state(SourceId::Gemini).clone();
        assert_eq!(state.consecutive_failures(), 0);
        assert_eq!(state.next_allowed_second(), 780 + SYNC_MIN_INTERVAL_SECS);
        assert!(!state.is_dirty());
    }

    #[test]
    fn slow_fallback_marks_all_sources_dirty_every_fifteen_minutes() {
        let schedule = test_schedule();
        let mut last = 0u64;

        // 未到 15 分钟:什么都不标
        apply_fallback_if_due(&schedule, SLOW_FALLBACK_INTERVAL_SECS - 1, &mut last);
        for source in SourceId::ALL {
            assert!(!schedule.lock().unwrap().state(source).is_dirty());
        }

        // 到 15 分钟:四个源全部标脏
        apply_fallback_if_due(&schedule, SLOW_FALLBACK_INTERVAL_SECS, &mut last);
        assert_eq!(last, SLOW_FALLBACK_INTERVAL_SECS);
        for source in SourceId::ALL {
            assert!(schedule.lock().unwrap().state(source).is_dirty());
        }

        // 立刻再调不重复触发(以 last_fallback 为锚点)
        apply_fallback_if_due(&schedule, SLOW_FALLBACK_INTERVAL_SECS + 100, &mut last);
        assert_eq!(last, SLOW_FALLBACK_INTERVAL_SECS);

        // 下一个 15 分钟窗口再次触发
        apply_fallback_if_due(&schedule, SLOW_FALLBACK_INTERVAL_SECS * 2, &mut last);
        assert_eq!(last, SLOW_FALLBACK_INTERVAL_SECS * 2);
    }

    #[test]
    fn watcher_starts_with_missing_roots_and_stop_shuts_schedule_down() {
        let (_dir, roots) = test_roots(); // 所有根目录都不存在
        let schedule = test_schedule();
        let wake = Arc::new(Notify::new());

        // 根目录缺失不能让构造失败(用户没装 codex/gemini 是常态)
        let watcher = UsageWatcher::start(roots, Arc::clone(&schedule), wake)
            .expect("missing roots must not fail startup");

        assert!(!schedule.lock().unwrap().is_shutdown());
        watcher.stop();
        assert!(schedule.lock().unwrap().is_shutdown());
        // shutdown 之后不再产生新任务
        assert!(schedule.lock().unwrap().begin_due(u64::MAX).is_empty());
        drop(watcher);
    }

    #[test]
    fn watcher_watches_existing_roots_without_error() {
        let (dir, roots) = test_roots();
        for root in [
            &roots.claude_projects,
            &roots.codex,
            &roots.gemini,
            &roots.opencode_root,
        ] {
            std::fs::create_dir_all(root).expect("create root");
        }
        let schedule = test_schedule();
        let wake = Arc::new(Notify::new());
        let watcher = UsageWatcher::start(roots, schedule, wake).expect("start watcher");
        drop(watcher); // Drop 停止监听线程,不 panic 即可
        drop(dir);
    }
}
