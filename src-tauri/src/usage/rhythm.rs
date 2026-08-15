use super::aggregation::aggregate_provider_account_range;
use super::domain::QuotaSnapshot;
use super::subscription_pace::valid_quota_pair;
use crate::error::AppError;
use crate::store::Database;
use chrono::{Datelike, Duration, Local, LocalResult, NaiveDate, TimeZone, Timelike};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

pub(crate) const RHYTHM_SLOT_COUNT: usize = 7 * 24;
pub(crate) const DAILY_BUDGET_WINDOW_KIND: &str = "daily_budget";

const COLD_START_MIN_SPAN_SECONDS: i64 = 14 * 86_400;
const COLD_START_MIN_OBSERVED_HOURS: usize = 200;
const PROFILE_CACHE_SECONDS: i64 = 15 * 60;
/// Three hours makes a slot's own evidence and its parent equally influential.
/// That is long enough for a single sparse sample to remain strongly shrunk.
pub(crate) const SHRINKAGE_PRIOR_SECONDS: i64 = 3 * 3_600;
fn min_intensity() -> Decimal {
    Decimal::new(25, 2)
}

fn max_intensity() -> Decimal {
    Decimal::from(4_i64)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RhythmProfile {
    intensities: [Decimal; RHYTHM_SLOT_COUNT],
}

impl RhythmProfile {
    #[cfg(test)]
    pub(crate) fn from_intensities(intensities: [Decimal; RHYTHM_SLOT_COUNT]) -> Self {
        Self { intensities }
    }

    #[cfg(test)]
    pub(crate) fn flat() -> Self {
        Self {
            intensities: [Decimal::ONE; RHYTHM_SLOT_COUNT],
        }
    }

    pub(crate) fn intensity_at(&self, timestamp: i64) -> Option<Decimal> {
        self.intensities.get(slot_index_at(timestamp)?).copied()
    }

    pub(crate) fn is_flat(&self) -> bool {
        self.intensities
            .iter()
            .all(|intensity| *intensity == Decimal::ONE)
    }

    pub(crate) fn average_intensity(&self, intervals: &[(i64, i64)]) -> Option<Decimal> {
        let mut weighted_seconds = Decimal::ZERO;
        let mut total_seconds = 0_i64;
        for &(start_at, end_at) in intervals {
            for_each_hour_segment(start_at, end_at, |segment_start, segment_end| {
                let seconds = segment_end.checked_sub(segment_start)?;
                let weighted = self
                    .intensity_at(segment_start)?
                    .checked_mul(Decimal::from(seconds))?;
                weighted_seconds = weighted_seconds.checked_add(weighted)?;
                total_seconds = total_seconds.checked_add(seconds)?;
                Some(())
            })?;
        }
        if total_seconds <= 0 {
            return None;
        }
        weighted_seconds.checked_div(Decimal::from(total_seconds))
    }

    pub(crate) fn effective_seconds(&self, start_at: i64, end_at: i64) -> Option<Decimal> {
        let mut effective = Decimal::ZERO;
        for_each_hour_segment(start_at, end_at, |segment_start, segment_end| {
            let seconds = segment_end.checked_sub(segment_start)?;
            let weighted = self
                .intensity_at(segment_start)?
                .checked_mul(Decimal::from(seconds))?;
            effective = effective.checked_add(weighted)?;
            Some(())
        })?;
        Some(effective)
    }

    pub(crate) fn projected_exhaust_at(
        &self,
        start_at: i64,
        horizon_end: i64,
        remaining: Decimal,
        base_rate_per_second: Decimal,
    ) -> Option<i64> {
        if remaining < Decimal::ZERO
            || base_rate_per_second <= Decimal::ZERO
            || horizon_end <= start_at
        {
            return None;
        }
        let mut accumulated = Decimal::ZERO;
        let mut projected = None;
        for_each_hour_segment(start_at, horizon_end, |segment_start, segment_end| {
            if projected.is_some() {
                return Some(());
            }
            let segment_rate =
                base_rate_per_second.checked_mul(self.intensity_at(segment_start)?)?;
            if segment_rate <= Decimal::ZERO {
                return Some(());
            }
            let seconds = segment_end.checked_sub(segment_start)?;
            let consumption = segment_rate.checked_mul(Decimal::from(seconds))?;
            let next = accumulated.checked_add(consumption)?;
            if next >= remaining {
                let still_needed = remaining.checked_sub(accumulated)?;
                let seconds_needed = still_needed.checked_div(segment_rate)?.ceil().to_i64()?;
                projected = segment_start.checked_add(seconds_needed);
            } else {
                accumulated = next;
            }
            Some(())
        })?;
        projected.filter(|timestamp| *timestamp <= horizon_end)
    }

    #[cfg(test)]
    pub(crate) fn intensities(&self) -> &[Decimal; RHYTHM_SLOT_COUNT] {
        &self.intensities
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Observation {
    consumption: Decimal,
    seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ObservedHour {
    date: NaiveDate,
    hour: u32,
    offset_seconds: i32,
}

#[derive(Debug)]
struct RhythmObservations {
    slots: [Observation; RHYTHM_SLOT_COUNT],
    observed_hours: BTreeSet<ObservedHour>,
    first_observed_at: Option<i64>,
    last_observed_at: Option<i64>,
}

impl Default for RhythmObservations {
    fn default() -> Self {
        Self {
            slots: [Observation::default(); RHYTHM_SLOT_COUNT],
            observed_hours: BTreeSet::new(),
            first_observed_at: None,
            last_observed_at: None,
        }
    }
}

impl RhythmObservations {
    fn add_interval(&mut self, start_at: i64, end_at: i64, consumption: Decimal) -> Option<()> {
        let span_seconds = end_at.checked_sub(start_at)?;
        if span_seconds <= 0 || consumption < Decimal::ZERO {
            return None;
        }
        let rate = consumption.checked_div(Decimal::from(span_seconds))?;
        for_each_hour_segment(start_at, end_at, |segment_start, segment_end| {
            let seconds = segment_end.checked_sub(segment_start)?;
            let slot = slot_index_at(segment_start)?;
            let segment_consumption = rate.checked_mul(Decimal::from(seconds))?;
            let observation = self.slots.get_mut(slot)?;
            observation.consumption = observation.consumption.checked_add(segment_consumption)?;
            observation.seconds = observation.seconds.checked_add(seconds)?;
            self.observed_hours.insert(observed_hour(segment_start)?);
            Some(())
        })?;
        self.first_observed_at = Some(
            self.first_observed_at
                .map_or(start_at, |current| current.min(start_at)),
        );
        self.last_observed_at = Some(
            self.last_observed_at
                .map_or(end_at, |current| current.max(end_at)),
        );
        Some(())
    }

    fn has_enough_history(&self) -> bool {
        let span = self
            .first_observed_at
            .zip(self.last_observed_at)
            .and_then(|(first, last)| last.checked_sub(first));
        span.is_some_and(|seconds| seconds >= COLD_START_MIN_SPAN_SECONDS)
            && self.observed_hours.len() >= COLD_START_MIN_OBSERVED_HOURS
    }

    fn estimate_profile(&self) -> Option<RhythmProfile> {
        if !self.has_enough_history() {
            return None;
        }

        let mut hour_of_day = [Observation::default(); 24];
        let mut day_type = [Observation::default(); 2];
        let mut global = Observation::default();
        for (index, observation) in self.slots.iter().copied().enumerate() {
            add_observation(hour_of_day.get_mut(index % 24)?, observation)?;
            add_observation(day_type.get_mut(day_type_for_slot(index))?, observation)?;
            add_observation(&mut global, observation)?;
        }
        if global.seconds <= 0 || global.consumption <= Decimal::ZERO {
            return None;
        }
        let global_rate = observation_rate(global)?;
        let day_estimates: [Decimal; 2] = std::array::from_fn(|index| {
            shrunk_rate(day_type[index], global_rate).unwrap_or(global_rate)
        });
        let mut estimates = [Decimal::ZERO; RHYTHM_SLOT_COUNT];
        for (index, estimate) in estimates.iter_mut().enumerate() {
            let parent = shrunk_rate(
                hour_of_day[index % 24],
                day_estimates[day_type_for_slot(index)],
            )?;
            *estimate = shrunk_rate(self.slots[index], parent)?;
        }

        let bounded = normalize_and_bound(estimates)?;
        let mut smoothed = [Decimal::ZERO; RHYTHM_SLOT_COUNT];
        for (index, value) in smoothed.iter_mut().enumerate() {
            let previous = bounded[(index + RHYTHM_SLOT_COUNT - 1) % RHYTHM_SLOT_COUNT];
            let current = bounded[index];
            let next = bounded[(index + 1) % RHYTHM_SLOT_COUNT];
            *value = previous
                .checked_add(current)?
                .checked_add(next)?
                .checked_div(Decimal::from(3_i64))?;
        }
        Some(RhythmProfile {
            intensities: normalize_and_bound(smoothed)?,
        })
    }
}

fn add_observation(target: &mut Observation, value: Observation) -> Option<()> {
    target.consumption = target.consumption.checked_add(value.consumption)?;
    target.seconds = target.seconds.checked_add(value.seconds)?;
    Some(())
}

fn observation_rate(observation: Observation) -> Option<Decimal> {
    if observation.seconds <= 0 {
        return None;
    }
    observation
        .consumption
        .checked_div(Decimal::from(observation.seconds))
}

fn shrunk_rate(observation: Observation, parent: Decimal) -> Option<Decimal> {
    if observation.seconds <= 0 {
        return Some(parent);
    }
    let denominator = observation.seconds.checked_add(SHRINKAGE_PRIOR_SECONDS)?;
    let weight = Decimal::from(observation.seconds).checked_div(Decimal::from(denominator))?;
    let parent_weight = Decimal::ONE.checked_sub(weight)?;
    observation_rate(observation)?
        .checked_mul(weight)?
        .checked_add(parent.checked_mul(parent_weight)?)
}

fn normalize_and_bound(
    mut values: [Decimal; RHYTHM_SLOT_COUNT],
) -> Option<[Decimal; RHYTHM_SLOT_COUNT]> {
    let target = Decimal::from(i64::try_from(RHYTHM_SLOT_COUNT).ok()?);
    let total = checked_sum(values.iter().copied())?;
    if total <= Decimal::ZERO {
        return None;
    }
    let scale = target.checked_div(total)?;
    for value in &mut values {
        *value = value
            .checked_mul(scale)?
            .clamp(min_intensity(), max_intensity());
    }

    for _ in 0..RHYTHM_SLOT_COUNT {
        let current = checked_sum(values.iter().copied())?;
        let difference = target.checked_sub(current)?;
        if difference == Decimal::ZERO {
            break;
        }
        let candidates = values
            .iter()
            .enumerate()
            .filter_map(|(index, value)| {
                ((difference > Decimal::ZERO && *value < max_intensity())
                    || (difference < Decimal::ZERO && *value > min_intensity()))
                .then_some(index)
            })
            .collect::<Vec<_>>();
        let count = i64::try_from(candidates.len()).ok()?;
        if count == 0 {
            return None;
        }
        let share = difference.checked_div(Decimal::from(count))?;
        for index in candidates {
            let value = values.get_mut(index)?;
            *value = value
                .checked_add(share)?
                .clamp(min_intensity(), max_intensity());
        }
    }

    let residual = target.checked_sub(checked_sum(values.iter().copied())?)?;
    if residual != Decimal::ZERO {
        let candidate = values.iter_mut().find(|value| {
            (residual > Decimal::ZERO && **value < max_intensity())
                || (residual < Decimal::ZERO && **value > min_intensity())
        })?;
        *candidate = candidate.checked_add(residual)?;
        if *candidate < min_intensity() || *candidate > max_intensity() {
            return None;
        }
    }
    (checked_sum(values.iter().copied())? == target).then_some(values)
}

fn checked_sum(values: impl IntoIterator<Item = Decimal>) -> Option<Decimal> {
    values
        .into_iter()
        .try_fold(Decimal::ZERO, |total, value| total.checked_add(value))
}

fn day_type_for_slot(slot: usize) -> usize {
    usize::from(slot / 24 >= 5)
}

pub(crate) fn slot_index_at(timestamp: i64) -> Option<usize> {
    let local = Local.timestamp_opt(timestamp, 0).single()?;
    let day = usize::try_from(local.weekday().num_days_from_monday()).ok()?;
    let hour = usize::try_from(local.hour()).ok()?;
    day.checked_mul(24)?.checked_add(hour)
}

fn observed_hour(timestamp: i64) -> Option<ObservedHour> {
    let local = Local.timestamp_opt(timestamp, 0).single()?;
    Some(ObservedHour {
        date: local.date_naive(),
        hour: local.hour(),
        offset_seconds: local.offset().local_minus_utc(),
    })
}

fn for_each_hour_segment(
    start_at: i64,
    end_at: i64,
    mut visit: impl FnMut(i64, i64) -> Option<()>,
) -> Option<()> {
    if end_at <= start_at {
        return None;
    }
    let mut cursor = start_at;
    while cursor < end_at {
        let next = next_local_hour_boundary(cursor)?.min(end_at);
        if next <= cursor {
            return None;
        }
        visit(cursor, next)?;
        cursor = next;
    }
    Some(())
}

fn next_local_hour_boundary(timestamp: i64) -> Option<i64> {
    let local = Local.timestamp_opt(timestamp, 0).single()?;
    let hour_start = local.date_naive().and_hms_opt(local.hour(), 0, 0)?;
    let mut candidate = hour_start.checked_add_signed(Duration::hours(1))?;
    for _ in 0..4 {
        let next = match Local.from_local_datetime(&candidate) {
            LocalResult::Single(value) => Some(value.timestamp()),
            LocalResult::Ambiguous(first, second) => [first.timestamp(), second.timestamp()]
                .into_iter()
                .filter(|value| *value > timestamp)
                .min(),
            LocalResult::None => None,
        };
        if let Some(next) = next.filter(|value| *value > timestamp) {
            return Some(next);
        }
        candidate = candidate.checked_add_signed(Duration::hours(1))?;
    }
    None
}

pub(crate) fn cached_quota_profile(
    db: &Database,
    provider_id: &str,
    window_kind: &str,
    now_timestamp: i64,
) -> Result<Option<Arc<RhythmProfile>>, AppError> {
    cached_profile(db, provider_id, window_kind, now_timestamp, || {
        let snapshots = db.quota_snapshots_since(provider_id, i64::MIN)?;
        Ok(quota_profile_from_snapshots(
            &snapshots,
            window_kind,
            now_timestamp,
        ))
    })
}

pub(crate) fn cached_metered_profile(
    db: &Database,
    cache_provider_id: &str,
    provider_ids: &[&str],
    now_timestamp: i64,
) -> Result<Option<Arc<RhythmProfile>>, AppError> {
    cached_profile(
        db,
        cache_provider_id,
        DAILY_BUDGET_WINDOW_KIND,
        now_timestamp,
        || metered_profile(db, provider_ids, now_timestamp),
    )
}

fn quota_profile_from_snapshots(
    snapshots: &[QuotaSnapshot],
    window_kind: &str,
    now_timestamp: i64,
) -> Option<RhythmProfile> {
    let mut observations = RhythmObservations::default();
    for pair in snapshots.windows(2) {
        let [earlier, later] = pair else {
            continue;
        };
        let Some(valid) = valid_quota_pair(earlier, later, window_kind, now_timestamp) else {
            continue;
        };
        observations.add_interval(valid.start_at, valid.end_at, valid.used_delta)?;
    }
    observations.estimate_profile()
}

fn metered_profile(
    db: &Database,
    provider_ids: &[&str],
    now_timestamp: i64,
) -> Result<Option<RhythmProfile>, AppError> {
    let Some((start_at, latest_at)) = metered_history_bounds(db, provider_ids, now_timestamp)?
    else {
        return Ok(None);
    };
    let Some(end_at) = latest_at.checked_add(1) else {
        return Ok(None);
    };
    if end_at <= start_at {
        return Ok(None);
    }

    let mut observations = RhythmObservations::default();
    let mut cursor = start_at;
    while cursor < end_at {
        let Some(segment_end) = next_local_hour_boundary(cursor).map(|next| next.min(end_at))
        else {
            return Ok(None);
        };
        let mut consumption = Decimal::ZERO;
        let mut usable = true;
        for provider_id in provider_ids {
            let aggregate = aggregate_provider_account_range(db, provider_id, cursor, segment_end)?;
            let value = if aggregate.event_count == 0 {
                Some(Decimal::ZERO)
            } else {
                aggregate
                    .total_cost_usd
                    .as_deref()
                    .and_then(|raw| Decimal::from_str_exact(raw.trim()).ok())
                    .filter(|value| *value >= Decimal::ZERO)
            };
            let Some(value) = value else {
                usable = false;
                break;
            };
            let Some(next) = consumption.checked_add(value) else {
                return Ok(None);
            };
            consumption = next;
        }
        if usable {
            let Some(()) = observations.add_interval(cursor, segment_end, consumption) else {
                return Ok(None);
            };
        }
        cursor = segment_end;
    }
    Ok(observations.estimate_profile())
}

fn metered_history_bounds(
    db: &Database,
    provider_ids: &[&str],
    now_timestamp: i64,
) -> Result<Option<(i64, i64)>, AppError> {
    let conn = db
        .conn
        .lock()
        .map_err(|error| AppError::Database(format!("Mutex lock failed: {error}")))?;
    let mut first = None;
    let mut last = None;
    for provider_id in provider_ids {
        let bounds = conn.query_row(
            "SELECT MIN(occurred_at), MAX(occurred_at)
             FROM usage_events
             WHERE provider_id = ?1 AND occurred_at <= ?2",
            rusqlite::params![provider_id, now_timestamp],
            |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<i64>>(1)?)),
        )?;
        if let (Some(provider_first), Some(provider_last)) = bounds {
            first = Some(first.map_or(provider_first, |value: i64| value.min(provider_first)));
            last = Some(last.map_or(provider_last, |value: i64| value.max(provider_last)));
        }
    }
    Ok(first.zip(last))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProfileCacheKey {
    database_identity: u64,
    provider_id: String,
    window_kind: String,
}

#[derive(Debug, Clone)]
struct CachedProfile {
    built_at: i64,
    profile: Option<Arc<RhythmProfile>>,
}

static PROFILE_CACHE: OnceLock<Mutex<HashMap<ProfileCacheKey, CachedProfile>>> = OnceLock::new();

fn cached_profile(
    db: &Database,
    provider_id: &str,
    window_kind: &str,
    now_timestamp: i64,
    build: impl FnOnce() -> Result<Option<RhythmProfile>, AppError>,
) -> Result<Option<Arc<RhythmProfile>>, AppError> {
    let cache = PROFILE_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache
        .lock()
        .map_err(|error| AppError::Message(format!("rhythm_profile_cache_poisoned: {error}")))?;
    let key = ProfileCacheKey {
        database_identity: db.rhythm_cache_identity,
        provider_id: provider_id.to_string(),
        window_kind: window_kind.to_string(),
    };
    if let Some(cached) = cache.get(&key) {
        if now_timestamp
            .checked_sub(cached.built_at)
            .is_some_and(|age| (0..PROFILE_CACHE_SECONDS).contains(&age))
        {
            return Ok(cached.profile.clone());
        }
    }

    match build() {
        Ok(profile) => {
            let profile = profile.map(Arc::new);
            cache.insert(
                key,
                CachedProfile {
                    built_at: now_timestamp,
                    profile: profile.clone(),
                },
            );
            Ok(profile)
        }
        Err(error) => {
            cache.insert(
                key,
                CachedProfile {
                    built_at: now_timestamp,
                    profile: None,
                },
            );
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::subscription_pace::{quota_rate_from_snapshots, FIVE_HOUR_WINDOW_KIND};
    use serde_json::json;

    fn snapshot(id: usize, fetched_at: i64, used: &str, reset: &str) -> QuotaSnapshot {
        QuotaSnapshot {
            snapshot_id: format!("snapshot-{id}"),
            provider_id: "provider".to_string(),
            fetched_at,
            five_hour_utilization_percent: Some(used.to_string()),
            five_hour_resets_at: Some(reset.to_string()),
            seven_day_utilization_percent: None,
            seven_day_resets_at: None,
            manual_resets_remaining: None,
            raw_payload: json!({}),
            created_at: fetched_at,
        }
    }

    fn hourly_snapshots(hours: i64) -> Vec<QuotaSnapshot> {
        let start = 1_700_000_000_i64;
        (0..=hours)
            .map(|hour| {
                snapshot(
                    usize::try_from(hour).unwrap(),
                    start + hour * 3_600,
                    &hour.to_string(),
                    "2030-01-01T00:00:00Z",
                )
            })
            .collect()
    }

    #[test]
    fn three_days_of_history_stays_behind_the_cold_start_gate() {
        let snapshots = hourly_snapshots(3 * 24);
        let now = snapshots.last().unwrap().fetched_at;

        assert!(quota_profile_from_snapshots(&snapshots, FIVE_HOUR_WINDOW_KIND, now).is_none());
    }

    #[test]
    fn fourteen_days_and_two_hundred_observed_hours_open_the_gate() {
        let snapshots = hourly_snapshots(15 * 24);
        let now = snapshots.last().unwrap().fetched_at;
        let profile = quota_profile_from_snapshots(&snapshots, FIVE_HOUR_WINDOW_KIND, now).unwrap();
        let sum = checked_sum(profile.intensities().iter().copied()).unwrap();

        assert_eq!(sum, Decimal::from(RHYTHM_SLOT_COUNT as i64));
    }

    #[test]
    fn shrinkage_keeps_one_hour_near_parent_but_many_hours_can_move() {
        let parent = Decimal::ONE;
        let high_rate = Decimal::from(4_i64);
        let one_hour = Observation {
            consumption: high_rate.checked_mul(Decimal::from(3_600_i64)).unwrap(),
            seconds: 3_600,
        };
        let many_hours = Observation {
            consumption: high_rate
                .checked_mul(Decimal::from(36 * 3_600_i64))
                .unwrap(),
            seconds: 36 * 3_600,
        };

        let sparse = shrunk_rate(one_hour, parent).unwrap();
        let dense = shrunk_rate(many_hours, parent).unwrap();

        assert!(sparse < Decimal::from(2_i64));
        assert!(dense > Decimal::from(3_i64));
        assert!(dense > sparse);
    }

    #[test]
    fn pathological_slot_is_bounded_and_mean_remains_exactly_one() {
        let mut values = [Decimal::ONE; RHYTHM_SLOT_COUNT];
        values[37] = Decimal::from(1_000_000_i64);

        let normalized = normalize_and_bound(values).unwrap();
        let sum = checked_sum(normalized.iter().copied()).unwrap();

        assert!(normalized
            .iter()
            .all(|value| { *value >= min_intensity() && *value <= max_intensity() }));
        assert_eq!(sum, Decimal::from(RHYTHM_SLOT_COUNT as i64));
        assert_eq!(
            sum.checked_div(Decimal::from(RHYTHM_SLOT_COUNT as i64)),
            Some(Decimal::ONE)
        );
    }

    #[test]
    fn reset_straddling_pair_is_excluded_from_rate_and_profile() {
        let snapshots = vec![
            snapshot(1, 1_700_000_000, "10", "2030-01-01T00:00:00Z"),
            snapshot(2, 1_700_003_600, "90", "2030-01-01T05:00:00Z"),
        ];
        let now = 1_700_003_600;

        assert_eq!(
            quota_rate_from_snapshots(&snapshots, FIVE_HOUR_WINDOW_KIND, now),
            None
        );
        assert!(quota_profile_from_snapshots(&snapshots, FIVE_HOUR_WINDOW_KIND, now).is_none());
    }
}
