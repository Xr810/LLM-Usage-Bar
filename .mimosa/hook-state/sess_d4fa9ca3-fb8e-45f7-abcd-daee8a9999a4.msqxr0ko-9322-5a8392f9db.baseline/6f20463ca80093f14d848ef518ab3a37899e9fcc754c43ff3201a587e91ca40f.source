const DEFAULT_INTERVAL_SECONDS: u64 = 300;
const MIN_INTERVAL_SECONDS: u64 = 60;
const MAX_INTERVAL_SECONDS: u64 = 86_400;
const FAILURE_BACKOFF_MULTIPLIERS: [u64; 4] = [1, 2, 4, 6];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceId {
    Claude,
    Codex,
    Gemini,
    OpenCode,
}

impl SourceId {
    pub const ALL: [Self; 4] = [Self::Claude, Self::Codex, Self::Gemini, Self::OpenCode];

    const fn index(self) -> usize {
        match self {
            Self::Claude => 0,
            Self::Codex => 1,
            Self::Gemini => 2,
            Self::OpenCode => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceScheduleState {
    dirty_generation: u64,
    completed_generation: u64,
    syncing_generation: Option<u64>,
    syncing_schedule_epoch: Option<u64>,
    next_allowed_second: u64,
    consecutive_failures: u32,
    shutdown: bool,
}

impl SourceScheduleState {
    const fn new() -> Self {
        Self {
            dirty_generation: 0,
            completed_generation: 0,
            syncing_generation: None,
            syncing_schedule_epoch: None,
            next_allowed_second: 0,
            consecutive_failures: 0,
            shutdown: false,
        }
    }

    pub fn dirty_generation(&self) -> u64 {
        self.dirty_generation
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty_generation > self.completed_generation
    }

    pub fn is_syncing(&self) -> bool {
        self.syncing_generation.is_some()
    }

    pub fn next_allowed_second(&self) -> u64 {
        self.next_allowed_second
    }

    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatcherSchedule {
    interval_seconds: u64,
    schedule_epoch: u64,
    sources: [SourceScheduleState; 4],
    shutdown: bool,
}

impl Default for WatcherSchedule {
    fn default() -> Self {
        Self::new(DEFAULT_INTERVAL_SECONDS)
    }
}

impl WatcherSchedule {
    pub fn new(interval_seconds: u64) -> Self {
        Self {
            interval_seconds: normalize_interval(interval_seconds),
            schedule_epoch: 0,
            sources: std::array::from_fn(|_| SourceScheduleState::new()),
            shutdown: false,
        }
    }

    pub fn interval_seconds(&self) -> u64 {
        self.interval_seconds
    }

    pub fn schedule_epoch(&self) -> u64 {
        self.schedule_epoch
    }

    pub fn state(&self, source: SourceId) -> &SourceScheduleState {
        &self.sources[source.index()]
    }

    pub fn mark_dirty(&mut self, source: SourceId) {
        let state = &mut self.sources[source.index()];
        if state.shutdown {
            return;
        }
        state.dirty_generation = state.dirty_generation.saturating_add(1);
    }

    pub fn mark_all_dirty(&mut self) {
        for source in SourceId::ALL {
            self.mark_dirty(source);
        }
    }

    pub fn begin_due(&mut self, now: u64) -> Vec<(SourceId, u64, u64)> {
        if self.shutdown {
            return Vec::new();
        }

        let epoch = self.schedule_epoch;
        let mut due = Vec::new();
        for source in SourceId::ALL {
            let state = &mut self.sources[source.index()];
            if state.shutdown
                || !state.is_dirty()
                || state.is_syncing()
                || now < state.next_allowed_second
            {
                continue;
            }

            let generation = state.dirty_generation;
            state.syncing_generation = Some(generation);
            state.syncing_schedule_epoch = Some(epoch);
            due.push((source, generation, epoch));
        }
        due
    }

    pub fn finish(
        &mut self,
        source: SourceId,
        generation: u64,
        schedule_epoch: u64,
        now: u64,
        result: Result<(), ()>,
    ) {
        if self.shutdown {
            return;
        }

        let current_epoch = self.schedule_epoch;
        let interval_seconds = self.interval_seconds;
        let state = &mut self.sources[source.index()];
        if state.shutdown
            || state.syncing_generation != Some(generation)
            || state.syncing_schedule_epoch != Some(schedule_epoch)
        {
            return;
        }

        state.syncing_generation = None;
        state.syncing_schedule_epoch = None;

        let delay = match result {
            Ok(()) => {
                state.completed_generation = state.completed_generation.max(generation);
                state.consecutive_failures = 0;
                interval_seconds
            }
            Err(()) => {
                state.consecutive_failures = state.consecutive_failures.saturating_add(1);
                let backoff_index = state.consecutive_failures.saturating_sub(1).min(3) as usize;
                interval_seconds.saturating_mul(FAILURE_BACKOFF_MULTIPLIERS[backoff_index])
            }
        };

        if schedule_epoch == current_epoch {
            state.next_allowed_second = now.saturating_add(delay);
        }
    }

    pub fn set_interval(&mut self, now: u64, interval_seconds: u64) {
        if self.shutdown {
            return;
        }

        self.interval_seconds = normalize_interval(interval_seconds);
        self.schedule_epoch = self.schedule_epoch.wrapping_add(1);
        let next_allowed_second = now.saturating_add(self.interval_seconds);
        for state in &mut self.sources {
            state.next_allowed_second = next_allowed_second;
        }
    }

    pub fn shutdown(&mut self) {
        if self.shutdown {
            return;
        }

        self.shutdown = true;
        for state in &mut self.sources {
            state.shutdown = true;
            state.syncing_generation = None;
            state.syncing_schedule_epoch = None;
        }
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown
    }
}

const fn normalize_interval(interval_seconds: u64) -> u64 {
    if interval_seconds < MIN_INTERVAL_SECONDS || interval_seconds > MAX_INTERVAL_SECONDS {
        DEFAULT_INTERVAL_SECONDS
    } else {
        interval_seconds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only_due(source: SourceId, generation: u64, epoch: u64) -> Vec<(SourceId, u64, u64)> {
        vec![(source, generation, epoch)]
    }

    #[test]
    fn default_and_invalid_intervals_normalize_to_five_minutes() {
        assert_eq!(WatcherSchedule::default().interval_seconds(), 300);
        assert_eq!(WatcherSchedule::new(0).interval_seconds(), 300);
        assert_eq!(WatcherSchedule::new(59).interval_seconds(), 300);
        assert_eq!(WatcherSchedule::new(86_401).interval_seconds(), 300);
        assert_eq!(WatcherSchedule::new(u64::MAX).interval_seconds(), 300);
    }

    #[test]
    fn event_storm_coalesces_and_never_starts_twice_inside_five_minutes() {
        let mut schedule = WatcherSchedule::new(300);
        for _ in 0..100 {
            schedule.mark_dirty(SourceId::Codex);
        }

        assert_eq!(schedule.begin_due(0), only_due(SourceId::Codex, 100, 0));
        assert!(schedule.begin_due(0).is_empty());

        schedule.mark_dirty(SourceId::Codex);
        schedule.finish(SourceId::Codex, 100, 0, 0, Ok(()));
        assert!(schedule.begin_due(299).is_empty());
        assert_eq!(schedule.begin_due(300), only_due(SourceId::Codex, 101, 0));
    }

    #[test]
    fn events_during_sync_preserve_the_newer_generation() {
        let mut schedule = WatcherSchedule::new(300);
        schedule.mark_dirty(SourceId::Claude);
        assert_eq!(schedule.begin_due(0), only_due(SourceId::Claude, 1, 0));

        schedule.mark_dirty(SourceId::Claude);
        schedule.mark_dirty(SourceId::Claude);
        schedule.finish(SourceId::Claude, 1, 0, 10, Ok(()));

        let state = schedule.state(SourceId::Claude);
        assert!(state.is_dirty());
        assert_eq!(state.dirty_generation(), 3);
        assert!(!state.is_syncing());
        assert!(schedule.begin_due(309).is_empty());
        assert_eq!(schedule.begin_due(310), only_due(SourceId::Claude, 3, 0));
    }

    #[test]
    fn stale_completion_cannot_clear_or_reschedule_the_active_sync() {
        let mut schedule = WatcherSchedule::new(300);
        schedule.mark_dirty(SourceId::Gemini);
        assert_eq!(schedule.begin_due(0), only_due(SourceId::Gemini, 1, 0));

        schedule.finish(SourceId::Gemini, 0, 0, 50, Err(()));
        schedule.finish(SourceId::Gemini, 1, 9, 50, Err(()));

        let state = schedule.state(SourceId::Gemini);
        assert!(state.is_syncing());
        assert_eq!(state.consecutive_failures(), 0);
        assert_eq!(state.next_allowed_second(), 0);

        schedule.finish(SourceId::Gemini, 1, 0, 50, Ok(()));
        let state = schedule.state(SourceId::Gemini);
        assert!(!state.is_dirty());
        assert!(!state.is_syncing());
        assert_eq!(state.next_allowed_second(), 350);
    }

    #[test]
    fn default_failure_backoff_is_five_ten_twenty_then_thirty_minutes() {
        let mut schedule = WatcherSchedule::new(300);
        schedule.mark_dirty(SourceId::OpenCode);

        let attempts = [0, 300, 900, 2_100, 3_900];
        let next_deadlines = [300, 900, 2_100, 3_900];

        for (index, now) in attempts.into_iter().take(4).enumerate() {
            assert_eq!(schedule.begin_due(now), only_due(SourceId::OpenCode, 1, 0));
            schedule.finish(SourceId::OpenCode, 1, 0, now, Err(()));
            assert_eq!(
                schedule.state(SourceId::OpenCode).next_allowed_second(),
                next_deadlines[index]
            );
            assert!(schedule.begin_due(next_deadlines[index] - 1).is_empty());
        }

        assert_eq!(
            schedule.begin_due(attempts[4]),
            only_due(SourceId::OpenCode, 1, 0)
        );
        schedule.finish(SourceId::OpenCode, 1, 0, attempts[4], Ok(()));
        assert_eq!(schedule.state(SourceId::OpenCode).consecutive_failures(), 0);
        assert_eq!(
            schedule.state(SourceId::OpenCode).next_allowed_second(),
            4_200
        );

        schedule.mark_dirty(SourceId::OpenCode);
        assert!(schedule.begin_due(4_199).is_empty());
        assert_eq!(
            schedule.begin_due(4_200),
            only_due(SourceId::OpenCode, 2, 0)
        );
        schedule.finish(SourceId::OpenCode, 2, 0, 4_200, Err(()));
        assert_eq!(
            schedule.state(SourceId::OpenCode).next_allowed_second(),
            4_500
        );
    }

    #[test]
    fn one_minute_interval_backs_off_one_two_four_then_six_minutes() {
        let mut schedule = WatcherSchedule::new(60);
        schedule.mark_dirty(SourceId::Codex);

        for (now, next) in [(0, 60), (60, 180), (180, 420), (420, 780)] {
            assert_eq!(schedule.begin_due(now), only_due(SourceId::Codex, 1, 0));
            schedule.finish(SourceId::Codex, 1, 0, now, Err(()));
            assert_eq!(schedule.state(SourceId::Codex).next_allowed_second(), next);
            assert!(schedule.begin_due(next - 1).is_empty());
        }
    }

    #[test]
    fn custom_intervals_gate_each_success_window() {
        for interval in [60, 600, 3_600] {
            let mut schedule = WatcherSchedule::new(interval);
            schedule.mark_dirty(SourceId::Claude);
            assert_eq!(schedule.begin_due(0), only_due(SourceId::Claude, 1, 0));
            schedule.finish(SourceId::Claude, 1, 0, 0, Ok(()));
            schedule.mark_dirty(SourceId::Claude);
            assert!(schedule.begin_due(interval - 1).is_empty());
            assert_eq!(
                schedule.begin_due(interval),
                only_due(SourceId::Claude, 2, 0)
            );
        }
    }

    #[test]
    fn set_interval_rearms_from_now_without_dirtying_or_running_immediately() {
        let mut schedule = WatcherSchedule::new(300);
        schedule.mark_dirty(SourceId::Codex);

        schedule.set_interval(10, 600);

        assert_eq!(schedule.interval_seconds(), 600);
        assert_eq!(schedule.schedule_epoch(), 1);
        assert!(!schedule.state(SourceId::Claude).is_dirty());
        assert!(schedule.begin_due(10).is_empty());
        assert!(schedule.begin_due(609).is_empty());
        assert_eq!(schedule.begin_due(610), only_due(SourceId::Codex, 1, 1));
    }

    #[test]
    fn old_epoch_completion_cannot_overwrite_rearmed_deadline() {
        let mut schedule = WatcherSchedule::new(300);
        schedule.mark_dirty(SourceId::Gemini);
        assert_eq!(schedule.begin_due(0), only_due(SourceId::Gemini, 1, 0));
        schedule.mark_dirty(SourceId::Gemini);

        schedule.set_interval(10, 600);
        assert_eq!(schedule.state(SourceId::Gemini).next_allowed_second(), 610);
        schedule.finish(SourceId::Gemini, 1, 0, 20, Ok(()));

        let state = schedule.state(SourceId::Gemini);
        assert!(state.is_dirty());
        assert!(!state.is_syncing());
        assert_eq!(state.next_allowed_second(), 610);
        assert!(schedule.begin_due(609).is_empty());
        assert_eq!(schedule.begin_due(610), only_due(SourceId::Gemini, 2, 1));
    }

    #[test]
    fn overflow_marks_every_source_dirty_once_for_the_next_batch() {
        let mut schedule = WatcherSchedule::new(300);
        schedule.mark_all_dirty();

        assert_eq!(
            schedule.begin_due(0),
            SourceId::ALL
                .into_iter()
                .map(|source| (source, 1, 0))
                .collect::<Vec<_>>()
        );
        assert!(schedule.begin_due(0).is_empty());
    }

    #[test]
    fn shutdown_is_idempotent_and_prevents_new_work() {
        let mut schedule = WatcherSchedule::new(300);
        schedule.mark_dirty(SourceId::Claude);
        let active = schedule.begin_due(0)[0];

        schedule.shutdown();
        schedule.shutdown();
        schedule.mark_dirty(SourceId::Claude);
        schedule.mark_all_dirty();
        schedule.finish(active.0, active.1, active.2, 0, Ok(()));

        assert!(schedule.is_shutdown());
        assert!(schedule.begin_due(u64::MAX).is_empty());
        for source in SourceId::ALL {
            let state = schedule.state(source);
            assert!(state.is_shutdown());
            assert!(!state.is_syncing());
        }
    }
}
