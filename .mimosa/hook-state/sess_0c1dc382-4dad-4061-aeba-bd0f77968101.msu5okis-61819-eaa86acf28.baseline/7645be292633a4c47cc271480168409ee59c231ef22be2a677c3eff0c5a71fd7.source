use super::rhythm::RhythmProfile;
use chrono::{DateTime, SecondsFormat};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const INVALID_QUOTA_PERCENT: &str = "invalid_quota_percent";
const QUOTA_UNAVAILABLE: &str = "quota_unavailable";
const DAILY_BUDGET_MISSING: &str = "daily_budget_missing";
const INVALID_DAILY_BUDGET: &str = "invalid_daily_budget";
const COST_UNAVAILABLE: &str = "cost_unavailable";
const INVALID_COST: &str = "invalid_cost";
const PARTIAL_COST: &str = "partial_cost";

pub const EXHAUSTED_PERCENT: u8 = 5;
pub const EXHAUSTED_GRACE_SECONDS: i64 = 900;
pub const MIN_RATE_SPAN_SECONDS: i64 = 600;
const SECONDS_PER_HOUR: i64 = 3_600;
// The yellow band is the fixed error bar around "runs out at reset", not a
// user preference. Keep these product constants out of persisted settings.
const PACE_GREEN_HEADROOM_PERCENT: u16 = 115;
const PACE_RED_HEADROOM_PERCENT: u16 = 85;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageStatus {
    Green,
    Yellow,
    Red,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CostQuality {
    Complete,
    Estimated,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PaceBasis {
    /// Tier 1 — rate measured from quota or cost history.
    Measured,
    /// Tier 2 — rate inferred from the current window average.
    WindowAverage,
    /// Tier 3 — no usable clock, static percentage thresholds.
    #[default]
    Static,
    /// Idle: no measurable burn.
    Idle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaceMeasurement {
    pub rate_per_second: Decimal,
    pub intervals: Vec<(i64, i64)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PaceInput {
    pub now_timestamp: i64,
    pub reset_timestamp: Option<i64>,
    pub window_length_seconds: Option<i64>,
    pub measured_rate_per_second: Option<Decimal>,
    pub measured_intervals: Vec<(i64, i64)>,
    pub rhythm_profile: Option<Arc<RhythmProfile>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceClassification {
    pub status: UsageStatus,
    pub used_percent: Option<String>,
    pub remaining_percent: Option<String>,
    pub consumed_percent: Option<String>,
    pub reason: Option<&'static str>,
    pub burn_rate_per_second: Option<Decimal>,
    pub projected_exhaust_at: Option<i64>,
    pub headroom_ratio: Option<Decimal>,
    pub pace_basis: PaceBasis,
    pub rhythm_adjustment: Option<Decimal>,
    /// What the verdict would have been had the rhythm profile not been
    /// applied. `Some` only when a profile was applied, so a caller can tell
    /// whether the rhythm actually moved the colour or merely nudged the
    /// ratio. `None` means there is nothing to compare against.
    pub flat_status: Option<UsageStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubscriptionThresholds {
    pub warning_remaining_percent: u8,
    pub critical_remaining_percent: u8,
}

impl Default for SubscriptionThresholds {
    fn default() -> Self {
        Self {
            // These static thresholds are used only when there is no reset
            // clock to project against.
            warning_remaining_percent: 50,
            critical_remaining_percent: 20,
        }
    }
}

impl From<&crate::config::settings::AppSettings> for SubscriptionThresholds {
    fn from(settings: &crate::config::settings::AppSettings) -> Self {
        Self {
            warning_remaining_percent: settings.usage_warning_remaining_percent,
            critical_remaining_percent: settings.usage_critical_remaining_percent,
        }
    }
}

impl SourceClassification {
    /// Burn rate as a decimal string in units-per-hour — quota-percent per hour
    /// for subscription windows, USD per hour for the daily budget. Every other
    /// numeric on these view structs is a decimal string, so this matches.
    pub fn burn_rate_per_hour(&self) -> Option<String> {
        self.burn_rate_per_second
            .and_then(|rate| rate.checked_mul(Decimal::from(SECONDS_PER_HOUR)))
            .map(|rate| rate.normalize().to_string())
    }

    pub fn projected_exhaust_at_rfc3339(&self) -> Option<String> {
        DateTime::from_timestamp(self.projected_exhaust_at?, 0)
            .map(|value| value.to_rfc3339_opts(SecondsFormat::AutoSi, true))
    }

    pub fn headroom_ratio_string(&self) -> Option<String> {
        self.headroom_ratio
            .map(|value| value.normalize().to_string())
    }

    pub fn rhythm_adjustment_string(&self) -> Option<String> {
        self.rhythm_adjustment
            .map(|value| value.normalize().to_string())
    }

    fn unknown(reason: &'static str) -> Self {
        Self {
            status: UsageStatus::Unknown,
            used_percent: None,
            remaining_percent: None,
            consumed_percent: None,
            reason: Some(reason),
            burn_rate_per_second: None,
            projected_exhaust_at: None,
            headroom_ratio: None,
            pace_basis: PaceBasis::Static,
            rhythm_adjustment: None,
            flat_status: None,
        }
    }
}

pub fn classify_subscription(
    five_hour_used: Option<&str>,
    seven_day_used: Option<&str>,
) -> SourceClassification {
    classify_subscription_with_thresholds(
        five_hour_used,
        seven_day_used,
        SubscriptionThresholds::default(),
    )
}

pub fn classify_subscription_with_thresholds(
    five_hour_used: Option<&str>,
    seven_day_used: Option<&str>,
    thresholds: SubscriptionThresholds,
) -> SourceClassification {
    let mut highest_used: Option<Decimal> = None;
    let mut saw_invalid = false;

    for raw in [five_hour_used, seven_day_used].into_iter().flatten() {
        match Decimal::from_str_exact(raw.trim()) {
            Ok(value) => {
                let clamped = value.clamp(Decimal::ZERO, Decimal::ONE_HUNDRED);
                highest_used = Some(match highest_used {
                    Some(current) => current.max(clamped),
                    None => clamped,
                });
            }
            Err(_) => saw_invalid = true,
        }
    }

    let Some(used) = highest_used else {
        return SourceClassification::unknown(if saw_invalid {
            INVALID_QUOTA_PERCENT
        } else {
            QUOTA_UNAVAILABLE
        });
    };

    let Some(remaining) = Decimal::ONE_HUNDRED.checked_sub(used) else {
        return SourceClassification::unknown(INVALID_QUOTA_PERCENT);
    };
    let critical = Decimal::from(thresholds.critical_remaining_percent);
    let warning = Decimal::from(thresholds.warning_remaining_percent);
    let status = if remaining < critical {
        UsageStatus::Red
    } else if remaining <= warning {
        UsageStatus::Yellow
    } else {
        UsageStatus::Green
    };

    SourceClassification {
        status,
        used_percent: Some(used.normalize().to_string()),
        remaining_percent: Some(remaining.normalize().to_string()),
        consumed_percent: None,
        reason: saw_invalid.then_some(INVALID_QUOTA_PERCENT),
        burn_rate_per_second: None,
        projected_exhaust_at: None,
        headroom_ratio: None,
        pace_basis: PaceBasis::Static,
        rhythm_adjustment: None,
        flat_status: None,
    }
}

pub fn classify_subscription_window(
    used_percent: Option<&str>,
    pace: PaceInput,
    thresholds: SubscriptionThresholds,
) -> SourceClassification {
    let mut classification = classify_subscription_with_thresholds(used_percent, None, thresholds);
    let Some(used) = classification
        .used_percent
        .as_deref()
        .and_then(|value| Decimal::from_str_exact(value.trim()).ok())
    else {
        return classification;
    };
    let Some(remaining) = Decimal::ONE_HUNDRED.checked_sub(used) else {
        return classification;
    };
    let Some(decision) = pace_decision(used, remaining, pace, thresholds, true) else {
        return classification;
    };

    classification.status = decision.status;
    classification.burn_rate_per_second = Some(decision.rate_per_second);
    classification.projected_exhaust_at = decision.projected_exhaust_at;
    classification.headroom_ratio = decision.headroom_ratio;
    classification.pace_basis = decision.basis;
    classification.rhythm_adjustment = decision.rhythm_adjustment;
    classification.flat_status = decision.flat_status;
    classification
}

pub fn classify_metered(
    numeric_cost_usd: Option<&str>,
    daily_budget_usd: Option<&str>,
    quality: CostQuality,
    pace: PaceInput,
    thresholds: SubscriptionThresholds,
) -> SourceClassification {
    let Some(raw_budget) = daily_budget_usd else {
        return SourceClassification::unknown(DAILY_BUDGET_MISSING);
    };
    let Ok(budget) = Decimal::from_str_exact(raw_budget.trim()) else {
        return SourceClassification::unknown(INVALID_DAILY_BUDGET);
    };
    if budget <= Decimal::ZERO {
        return SourceClassification::unknown(INVALID_DAILY_BUDGET);
    }
    if quality == CostQuality::Unavailable {
        return SourceClassification::unknown(COST_UNAVAILABLE);
    }

    let Some(raw_cost) = numeric_cost_usd else {
        return SourceClassification::unknown(COST_UNAVAILABLE);
    };
    let Ok(cost) = Decimal::from_str_exact(raw_cost.trim()) else {
        return SourceClassification::unknown(INVALID_COST);
    };
    if cost < Decimal::ZERO {
        return SourceClassification::unknown(INVALID_COST);
    }

    let consumed_percent = cost
        .checked_div(budget)
        .and_then(|ratio| ratio.checked_mul(Decimal::ONE_HUNDRED))
        .map(|value| value.normalize().to_string());

    let (threshold_status, decision) = if cost >= budget {
        (UsageStatus::Red, None)
    } else {
        let Some(remaining) = budget.checked_sub(cost) else {
            return SourceClassification::unknown(INVALID_COST);
        };
        match pace_decision(cost, remaining, pace, thresholds, false) {
            Some(decision) => (decision.status, Some(decision)),
            None => {
                let Some(yellow_boundary) = budget.checked_mul(Decimal::new(5, 1)) else {
                    return SourceClassification::unknown(INVALID_DAILY_BUDGET);
                };
                let Some(red_boundary) = budget.checked_mul(Decimal::new(8, 1)) else {
                    return SourceClassification::unknown(INVALID_DAILY_BUDGET);
                };
                let status = if cost >= red_boundary {
                    UsageStatus::Red
                } else if cost >= yellow_boundary {
                    UsageStatus::Yellow
                } else {
                    UsageStatus::Green
                };
                (status, None)
            }
        }
    };

    let (status, reason) = if quality == CostQuality::Partial {
        let status = match threshold_status {
            UsageStatus::Green => UsageStatus::Unknown,
            known => known,
        };
        (status, Some(PARTIAL_COST))
    } else {
        (threshold_status, None)
    };

    SourceClassification {
        status,
        used_percent: None,
        remaining_percent: None,
        consumed_percent,
        reason,
        burn_rate_per_second: decision.map(|value| value.rate_per_second),
        projected_exhaust_at: decision.and_then(|value| value.projected_exhaust_at),
        headroom_ratio: decision.and_then(|value| value.headroom_ratio),
        pace_basis: decision
            .map(|value| value.basis)
            .unwrap_or(PaceBasis::Static),
        rhythm_adjustment: decision.and_then(|value| value.rhythm_adjustment),
        flat_status: decision.and_then(|value| value.flat_status),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PaceDecision {
    status: UsageStatus,
    rate_per_second: Decimal,
    projected_exhaust_at: Option<i64>,
    headroom_ratio: Option<Decimal>,
    basis: PaceBasis,
    rhythm_adjustment: Option<Decimal>,
    flat_status: Option<UsageStatus>,
}

fn pace_decision(
    used: Decimal,
    remaining: Decimal,
    pace: PaceInput,
    thresholds: SubscriptionThresholds,
    apply_exhaustion_floor: bool,
) -> Option<PaceDecision> {
    let reset_timestamp = pace.reset_timestamp?;
    let window_length_seconds = pace.window_length_seconds?;
    if reset_timestamp <= pace.now_timestamp || window_length_seconds <= 0 {
        return None;
    }
    let time_to_reset_seconds = reset_timestamp.checked_sub(pace.now_timestamp)?;

    if let Some(rate) = pace
        .measured_rate_per_second
        .filter(|rate| *rate >= Decimal::ZERO)
    {
        if let Some(decision) = decision_for_rate(
            remaining,
            rate,
            time_to_reset_seconds,
            pace.now_timestamp,
            window_length_seconds,
            thresholds,
            PaceBasis::Measured,
            apply_exhaustion_floor,
            pace.rhythm_profile.as_deref(),
            &pace.measured_intervals,
        ) {
            return Some(decision);
        }
    }

    let elapsed_seconds = window_length_seconds.checked_sub(time_to_reset_seconds)?;
    if elapsed_seconds <= 0 {
        return None;
    }
    let average_rate = used.checked_div(Decimal::from(elapsed_seconds))?;
    let window_start = reset_timestamp.checked_sub(window_length_seconds)?;
    let average_interval = [(window_start, pace.now_timestamp)];
    decision_for_rate(
        remaining,
        average_rate,
        time_to_reset_seconds,
        pace.now_timestamp,
        window_length_seconds,
        thresholds,
        PaceBasis::WindowAverage,
        apply_exhaustion_floor,
        pace.rhythm_profile.as_deref(),
        &average_interval,
    )
}

#[allow(clippy::too_many_arguments)]
fn decision_for_rate(
    remaining: Decimal,
    rate_per_second: Decimal,
    time_to_reset_seconds: i64,
    now_timestamp: i64,
    window_length_seconds: i64,
    thresholds: SubscriptionThresholds,
    basis: PaceBasis,
    apply_exhaustion_floor: bool,
    rhythm_profile: Option<&RhythmProfile>,
    measured_intervals: &[(i64, i64)],
) -> Option<PaceDecision> {
    if rate_per_second < Decimal::ZERO || time_to_reset_seconds <= 0 {
        return None;
    }

    if let Some(profile) = rhythm_profile {
        if profile.is_flat()
            && profile.average_intensity(measured_intervals) == Some(Decimal::ONE)
            && profile.effective_seconds(
                now_timestamp,
                now_timestamp.checked_add(time_to_reset_seconds)?,
            ) == Some(Decimal::from(time_to_reset_seconds))
        {
            return flat_decision_for_rate(
                remaining,
                rate_per_second,
                time_to_reset_seconds,
                now_timestamp,
                thresholds,
                basis,
                apply_exhaustion_floor,
            );
        }
        if let Some(decision) = rhythm_decision_for_rate(
            remaining,
            rate_per_second,
            time_to_reset_seconds,
            now_timestamp,
            window_length_seconds,
            thresholds,
            basis,
            apply_exhaustion_floor,
            profile,
            measured_intervals,
        ) {
            return Some(decision);
        }
    }

    flat_decision_for_rate(
        remaining,
        rate_per_second,
        time_to_reset_seconds,
        now_timestamp,
        thresholds,
        basis,
        apply_exhaustion_floor,
    )
}

#[allow(clippy::too_many_arguments)]
fn rhythm_decision_for_rate(
    remaining: Decimal,
    rate_per_second: Decimal,
    time_to_reset_seconds: i64,
    now_timestamp: i64,
    window_length_seconds: i64,
    thresholds: SubscriptionThresholds,
    basis: PaceBasis,
    apply_exhaustion_floor: bool,
    profile: &RhythmProfile,
    measured_intervals: &[(i64, i64)],
) -> Option<PaceDecision> {
    let measured_intensity = profile
        .average_intensity(measured_intervals)
        .filter(|value| *value > Decimal::ZERO)?;
    let base_rate = rate_per_second.checked_div(measured_intensity)?;
    let reset_timestamp = now_timestamp.checked_add(time_to_reset_seconds)?;
    let effective_remaining_time = profile.effective_seconds(now_timestamp, reset_timestamp)?;
    let rhythm_adjustment =
        effective_remaining_time.checked_div(Decimal::from(time_to_reset_seconds))?;
    let effective_basis = if rate_per_second == Decimal::ZERO {
        PaceBasis::Idle
    } else {
        basis
    };
    let (headroom_ratio, projected_exhaust_at) = if rate_per_second == Decimal::ZERO {
        (None, None)
    } else {
        let projected_consumption = base_rate.checked_mul(effective_remaining_time)?;
        if projected_consumption <= Decimal::ZERO {
            return None;
        }
        let headroom_ratio = remaining.checked_div(projected_consumption);
        let horizon_end = reset_timestamp.checked_add(window_length_seconds)?;
        let projected_exhaust_at =
            profile.projected_exhaust_at(now_timestamp, horizon_end, remaining, base_rate);
        (headroom_ratio, projected_exhaust_at)
    };
    let status = status_for_rate(
        remaining,
        rate_per_second,
        time_to_reset_seconds,
        headroom_ratio,
        thresholds,
        apply_exhaustion_floor,
    )?;

    // Recomputed rather than inferred: the caller needs to know whether the
    // rhythm actually moved the colour, which a ratio alone cannot answer.
    let flat_status = flat_decision_for_rate(
        remaining,
        rate_per_second,
        time_to_reset_seconds,
        now_timestamp,
        thresholds,
        basis,
        apply_exhaustion_floor,
    )
    .map(|decision| decision.status);

    Some(PaceDecision {
        status,
        rate_per_second,
        projected_exhaust_at,
        headroom_ratio,
        basis: effective_basis,
        rhythm_adjustment: Some(rhythm_adjustment),
        flat_status,
    })
}

#[allow(clippy::too_many_arguments)]
fn flat_decision_for_rate(
    remaining: Decimal,
    rate_per_second: Decimal,
    time_to_reset_seconds: i64,
    now_timestamp: i64,
    thresholds: SubscriptionThresholds,
    basis: PaceBasis,
    apply_exhaustion_floor: bool,
) -> Option<PaceDecision> {
    let effective_basis = if rate_per_second == Decimal::ZERO {
        PaceBasis::Idle
    } else {
        basis
    };
    let (headroom_ratio, projected_exhaust_at) = if rate_per_second == Decimal::ZERO {
        (None, None)
    } else {
        let time_to_exhaust = remaining.checked_div(rate_per_second);
        let headroom_ratio = time_to_exhaust
            .and_then(|duration| duration.checked_div(Decimal::from(time_to_reset_seconds)));
        let projected_exhaust_at = time_to_exhaust
            .and_then(|duration| duration.ceil().to_i64())
            .and_then(|duration| now_timestamp.checked_add(duration));
        (headroom_ratio, projected_exhaust_at)
    };

    let status = status_for_rate(
        remaining,
        rate_per_second,
        time_to_reset_seconds,
        headroom_ratio,
        thresholds,
        apply_exhaustion_floor,
    )?;

    Some(PaceDecision {
        status,
        rate_per_second,
        projected_exhaust_at,
        headroom_ratio,
        basis: effective_basis,
        rhythm_adjustment: None,
        flat_status: None,
    })
}

fn status_for_rate(
    remaining: Decimal,
    rate_per_second: Decimal,
    time_to_reset_seconds: i64,
    headroom_ratio: Option<Decimal>,
    _thresholds: SubscriptionThresholds,
    apply_exhaustion_floor: bool,
) -> Option<UsageStatus> {
    let exhausted = apply_exhaustion_floor && remaining <= Decimal::from(EXHAUSTED_PERCENT);
    if exhausted {
        return Some(if time_to_reset_seconds > EXHAUSTED_GRACE_SECONDS {
            UsageStatus::Red
        } else {
            UsageStatus::Yellow
        });
    }
    if rate_per_second == Decimal::ZERO {
        return Some(UsageStatus::Green);
    }
    let headroom_ratio = headroom_ratio?;
    let green_at = Decimal::from(PACE_GREEN_HEADROOM_PERCENT).checked_div(Decimal::ONE_HUNDRED)?;
    let red_at = Decimal::from(PACE_RED_HEADROOM_PERCENT).checked_div(Decimal::ONE_HUNDRED)?;
    Some(if headroom_ratio >= green_at {
        UsageStatus::Green
    } else if headroom_ratio >= red_at {
        UsageStatus::Yellow
    } else {
        UsageStatus::Red
    })
}

pub fn worst_status(values: impl IntoIterator<Item = UsageStatus>) -> UsageStatus {
    values
        .into_iter()
        .fold(UsageStatus::Unknown, |worst, candidate| {
            if severity(candidate) > severity(worst) {
                candidate
            } else {
                worst
            }
        })
}

fn severity(status: UsageStatus) -> u8 {
    match status {
        UsageStatus::Unknown => 0,
        UsageStatus::Green => 1,
        UsageStatus::Yellow => 2,
        UsageStatus::Red => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::rhythm::{slot_index_at, RhythmProfile, RHYTHM_SLOT_COUNT};
    use chrono::{Local, TimeZone};

    fn static_metered(
        cost: Option<&str>,
        budget: Option<&str>,
        quality: CostQuality,
    ) -> SourceClassification {
        classify_metered(
            cost,
            budget,
            quality,
            PaceInput::default(),
            SubscriptionThresholds::default(),
        )
    }

    fn per_second(amount_per_hour: &str) -> Decimal {
        Decimal::from_str_exact(amount_per_hour)
            .unwrap()
            .checked_div(Decimal::from(3_600))
            .unwrap()
    }

    fn measured_pace(
        now_timestamp: i64,
        time_to_reset_seconds: i64,
        window_length_seconds: i64,
        amount_per_hour: &str,
    ) -> PaceInput {
        PaceInput {
            now_timestamp,
            reset_timestamp: now_timestamp.checked_add(time_to_reset_seconds),
            window_length_seconds: Some(window_length_seconds),
            measured_rate_per_second: Some(per_second(amount_per_hour)),
            ..PaceInput::default()
        }
    }

    fn local_test_hour() -> i64 {
        Local
            .with_ymd_and_hms(2026, 1, 5, 21, 0, 0)
            .earliest()
            .unwrap()
            .timestamp()
    }

    fn profile_with_future_intensity(
        now_timestamp: i64,
        reset_timestamp: i64,
        future_intensity: Decimal,
    ) -> RhythmProfile {
        let mut intensities = [Decimal::ONE; RHYTHM_SLOT_COUNT];
        let mut cursor = now_timestamp;
        while cursor < reset_timestamp {
            let slot = slot_index_at(cursor).unwrap();
            intensities[slot] = future_intensity;
            cursor += 15 * 60;
        }
        RhythmProfile::from_intensities(intensities)
    }

    #[test]
    fn flat_rhythm_reduces_exactly_to_legacy_rate_decisions() {
        let now = local_test_hour();
        let profile = RhythmProfile::flat();
        let thresholds = SubscriptionThresholds::default();
        for (remaining, hourly_rate, reset_seconds, apply_floor) in [
            ("80", "0", 3_600, true),
            ("20", "10", 3 * 3_600, true),
            ("60", "10", 3 * 3_600, true),
            ("1", "0.25", 300, false),
            ("5", "0.5", 7 * 86_400, true),
        ] {
            let remaining = Decimal::from_str_exact(remaining).unwrap();
            let rate = per_second(hourly_rate);
            let legacy = flat_decision_for_rate(
                remaining,
                rate,
                reset_seconds,
                now,
                thresholds,
                PaceBasis::Measured,
                apply_floor,
            )
            .unwrap();
            let rhythmic = decision_for_rate(
                remaining,
                rate,
                reset_seconds,
                now,
                7 * 86_400,
                thresholds,
                PaceBasis::Measured,
                apply_floor,
                Some(&profile),
                &[(now - 30 * 60, now)],
            )
            .unwrap();

            assert_eq!(rhythmic, legacy, "remaining={remaining}");
        }
    }

    #[test]
    fn quiet_upcoming_hours_relax_a_flat_red_verdict() {
        let now = local_test_hour();
        let reset = now + 3 * 3_600;
        let flat = classify_subscription_window(
            Some("80"),
            measured_pace(now, 3 * 3_600, 5 * 3_600, "10"),
            SubscriptionThresholds::default(),
        );
        let rhythmic = classify_subscription_window(
            Some("80"),
            PaceInput {
                now_timestamp: now,
                reset_timestamp: Some(reset),
                window_length_seconds: Some(5 * 3_600),
                measured_rate_per_second: Some(per_second("10")),
                measured_intervals: vec![(now - 30 * 60, now)],
                rhythm_profile: Some(Arc::new(profile_with_future_intensity(
                    now,
                    reset,
                    Decimal::new(3, 1),
                ))),
            },
            SubscriptionThresholds::default(),
        );

        assert_eq!(flat.status, UsageStatus::Red);
        assert!(matches!(
            rhythmic.status,
            UsageStatus::Green | UsageStatus::Yellow
        ));
        assert!(rhythmic
            .rhythm_adjustment
            .is_some_and(|value| value < Decimal::ONE));
    }

    #[test]
    fn busy_upcoming_hours_escalate_a_flat_green_verdict() {
        let now = local_test_hour();
        let reset = now + 3 * 3_600;
        let flat = classify_subscription_window(
            Some("40"),
            measured_pace(now, 3 * 3_600, 5 * 3_600, "10"),
            SubscriptionThresholds::default(),
        );
        let rhythmic = classify_subscription_window(
            Some("40"),
            PaceInput {
                now_timestamp: now,
                reset_timestamp: Some(reset),
                window_length_seconds: Some(5 * 3_600),
                measured_rate_per_second: Some(per_second("10")),
                measured_intervals: vec![(now - 30 * 60, now)],
                rhythm_profile: Some(Arc::new(profile_with_future_intensity(
                    now,
                    reset,
                    Decimal::from(4_i64),
                ))),
            },
            SubscriptionThresholds::default(),
        );

        assert_eq!(flat.status, UsageStatus::Green);
        assert!(matches!(
            rhythmic.status,
            UsageStatus::Yellow | UsageStatus::Red
        ));
        assert!(rhythmic
            .rhythm_adjustment
            .is_some_and(|value| value > Decimal::ONE));
    }

    #[test]
    fn subscription_remaining_boundaries_are_exact() {
        for (used, expected) in [
            ("49.999", UsageStatus::Green),
            ("50", UsageStatus::Yellow),
            ("79.999", UsageStatus::Yellow),
            ("80", UsageStatus::Yellow),
            ("80.001", UsageStatus::Red),
        ] {
            assert_eq!(
                classify_subscription(Some(used), None).status,
                expected,
                "{used}",
            );
        }
    }

    #[test]
    fn subscription_uses_custom_remaining_thresholds() {
        let thresholds = SubscriptionThresholds {
            warning_remaining_percent: 60,
            critical_remaining_percent: 30,
        };

        for (used, expected) in [
            ("39.999", UsageStatus::Green),
            ("40", UsageStatus::Yellow),
            ("70", UsageStatus::Yellow),
            ("70.001", UsageStatus::Red),
        ] {
            assert_eq!(
                classify_subscription_with_thresholds(Some(used), None, thresholds).status,
                expected,
                "{used}",
            );
        }
    }

    #[test]
    fn subscription_clamps_below_zero_and_above_one_hundred() {
        let below_zero = classify_subscription(Some("-0.001"), None);
        assert_eq!(below_zero.status, UsageStatus::Green);
        assert_eq!(below_zero.used_percent.as_deref(), Some("0"));
        assert_eq!(below_zero.remaining_percent.as_deref(), Some("100"));

        let above_one_hundred = classify_subscription(Some("100.001"), None);
        assert_eq!(above_one_hundred.status, UsageStatus::Red);
        assert_eq!(above_one_hundred.used_percent.as_deref(), Some("100"));
        assert_eq!(above_one_hundred.remaining_percent.as_deref(), Some("0"));
    }

    #[test]
    fn subscription_uses_the_highest_valid_window() {
        let classification = classify_subscription(Some("49"), Some("80.001"));

        assert_eq!(classification.status, UsageStatus::Red);
        assert_eq!(classification.used_percent.as_deref(), Some("80.001"));
        assert_eq!(classification.remaining_percent.as_deref(), Some("19.999"));
        assert_eq!(classification.consumed_percent, None);
    }

    #[test]
    fn subscription_ignores_but_reports_a_malformed_window() {
        let classification = classify_subscription(Some("55"), Some("not-a-percent"));

        assert_eq!(classification.status, UsageStatus::Yellow);
        assert_eq!(classification.used_percent.as_deref(), Some("55"));
        assert_eq!(classification.reason, Some("invalid_quota_percent"));
    }

    #[test]
    fn missing_or_entirely_malformed_subscription_values_are_unknown() {
        let missing = classify_subscription(None, None);
        assert_eq!(missing.status, UsageStatus::Unknown);
        assert_eq!(missing.used_percent, None);
        assert_eq!(missing.remaining_percent, None);

        let malformed = classify_subscription(Some("NaN"), Some("invalid"));
        assert_eq!(malformed.status, UsageStatus::Unknown);
        assert_eq!(malformed.reason, Some("invalid_quota_percent"));

        let scientific = classify_subscription(Some("5e1"), None);
        assert_eq!(scientific.status, UsageStatus::Unknown);
        assert_eq!(scientific.reason, Some("invalid_quota_percent"));
    }

    #[test]
    fn imminent_reset_turns_nearly_twenty_percent_remaining_green() {
        let now = 2_000_000_000;
        let paced = classify_subscription_window(
            Some("80.001"),
            measured_pace(now, 300, 18_000, "10"),
            SubscriptionThresholds::default(),
        );
        let static_fallback = classify_subscription_window(
            Some("80.001"),
            PaceInput::default(),
            SubscriptionThresholds::default(),
        );

        assert_eq!(paced.status, UsageStatus::Green);
        assert_eq!(paced.pace_basis, PaceBasis::Measured);
        assert!(paced.headroom_ratio.is_some());
        assert_eq!(static_fallback.status, UsageStatus::Red);
        assert_eq!(static_fallback.pace_basis, PaceBasis::Static);
    }

    #[test]
    fn fast_burn_with_four_hours_to_reset_is_red() {
        let classification = classify_subscription_window(
            Some("80"),
            measured_pace(2_000_000_000, 4 * 3_600, 18_000, "20"),
            SubscriptionThresholds::default(),
        );

        assert_eq!(classification.status, UsageStatus::Red);
        assert_eq!(classification.pace_basis, PaceBasis::Measured);
    }

    #[test]
    fn on_pace_burn_is_yellow() {
        let classification = classify_subscription_window(
            Some("80"),
            measured_pace(2_000_000_000, 2 * 3_600, 18_000, "10"),
            SubscriptionThresholds::default(),
        );

        assert_eq!(classification.status, UsageStatus::Yellow);
        assert!(classification
            .headroom_ratio
            .is_some_and(|ratio| ratio >= Decimal::new(99, 2) && ratio <= Decimal::new(101, 2)));
    }

    #[test]
    fn idle_is_green_until_the_exhaustion_floor_overrides_it() {
        let thresholds = SubscriptionThresholds::default();
        let idle = classify_subscription_window(
            Some("92"),
            measured_pace(2_000_000_000, 2 * 3_600, 18_000, "0"),
            thresholds,
        );
        let exhausted = classify_subscription_window(
            Some("97"),
            measured_pace(2_000_000_000, 2 * 3_600, 18_000, "0"),
            thresholds,
        );
        let exhausted_in_grace = classify_subscription_window(
            Some("97"),
            measured_pace(2_000_000_000, 300, 18_000, "0"),
            thresholds,
        );

        assert_eq!(idle.status, UsageStatus::Green);
        assert_eq!(idle.pace_basis, PaceBasis::Idle);
        assert_eq!(exhausted.status, UsageStatus::Red);
        assert_eq!(exhausted_in_grace.status, UsageStatus::Yellow);
    }

    #[test]
    fn missing_reset_preserves_every_static_subscription_verdict() {
        for used in ["49.999", "50", "79.999", "80", "80.001"] {
            let expected = classify_subscription(Some(used), None);
            let actual = classify_subscription_window(
                Some(used),
                PaceInput::default(),
                SubscriptionThresholds::default(),
            );

            assert_eq!(actual, expected, "{used}");
            assert_eq!(actual.pace_basis, PaceBasis::Static);
        }
    }

    #[test]
    fn past_reset_uses_static_fallback() {
        let now = 2_000_000_000;
        let classification = classify_subscription_window(
            Some("80.001"),
            PaceInput {
                now_timestamp: now,
                reset_timestamp: now.checked_sub(1),
                window_length_seconds: Some(18_000),
                measured_rate_per_second: Some(per_second("1")),
                ..PaceInput::default()
            },
            SubscriptionThresholds::default(),
        );

        assert_eq!(classification.status, UsageStatus::Red);
        assert_eq!(classification.pace_basis, PaceBasis::Static);
    }

    #[test]
    fn metered_budget_boundaries_are_exact() {
        for (cost, expected) in [
            ("4.999", UsageStatus::Green),
            ("5", UsageStatus::Yellow),
            ("7.999", UsageStatus::Yellow),
            ("8", UsageStatus::Red),
        ] {
            assert_eq!(
                static_metered(Some(cost), Some("10"), CostQuality::Complete).status,
                expected,
                "{cost}",
            );
        }
    }

    #[test]
    fn metered_pace_changes_with_the_local_reset_clock_and_hard_cap_wins() {
        let now = 2_000_000_000;
        let thresholds = SubscriptionThresholds::default();
        for (hour, expected) in [
            (9_i64, UsageStatus::Red),
            (18_i64, UsageStatus::Yellow),
            (23_i64, UsageStatus::Green),
        ] {
            let time_to_reset = (24 - hour) * 3_600;
            let classification = classify_metered(
                Some("8.50"),
                Some("10"),
                CostQuality::Complete,
                measured_pace(now, time_to_reset, 86_400, "0.25"),
                thresholds,
            );
            assert_eq!(classification.status, expected, "hour={hour}");
            assert_eq!(classification.pace_basis, PaceBasis::Measured);
        }

        let hard_cap = classify_metered(
            Some("10"),
            Some("10"),
            CostQuality::Complete,
            measured_pace(now, 300, 86_400, "0"),
            thresholds,
        );
        assert_eq!(hard_cap.status, UsageStatus::Red);
    }

    #[test]
    fn partial_cost_still_demotes_a_pace_green_verdict() {
        let classification = classify_metered(
            Some("8.50"),
            Some("10"),
            CostQuality::Partial,
            measured_pace(2_000_000_000, 3_600, 86_400, "0.25"),
            SubscriptionThresholds::default(),
        );

        assert_eq!(classification.status, UsageStatus::Unknown);
        assert_eq!(classification.reason, Some("partial_cost"));
        assert_eq!(classification.pace_basis, PaceBasis::Measured);
    }

    #[test]
    fn estimated_cost_participates_in_classification() {
        let classification = static_metered(Some("8"), Some("10"), CostQuality::Estimated);

        assert_eq!(classification.status, UsageStatus::Red);
        assert_eq!(classification.consumed_percent.as_deref(), Some("80"));
        assert_eq!(classification.reason, None);
    }

    #[test]
    fn extreme_metered_values_classify_without_overflowing() {
        let classification = static_metered(
            Some("79228162514264337593543950335"),
            Some("0.0000000000000000000000000001"),
            CostQuality::Complete,
        );

        assert_eq!(classification.status, UsageStatus::Red);
        assert_eq!(classification.reason, None);
    }

    #[test]
    fn partial_cost_only_proves_escalation() {
        let below_warning = static_metered(Some("4"), Some("10"), CostQuality::Partial);
        assert_eq!(below_warning.status, UsageStatus::Unknown);
        assert_eq!(below_warning.consumed_percent.as_deref(), Some("40"));
        assert_eq!(below_warning.reason, Some("partial_cost"));

        let warning = static_metered(Some("5"), Some("10"), CostQuality::Partial);
        assert_eq!(warning.status, UsageStatus::Yellow);
        assert_eq!(warning.reason, Some("partial_cost"));

        let critical = static_metered(Some("8"), Some("10"), CostQuality::Partial);
        assert_eq!(critical.status, UsageStatus::Red);
        assert_eq!(critical.reason, Some("partial_cost"));
    }

    #[test]
    fn missing_and_invalid_budgets_are_unknown() {
        let missing = static_metered(Some("8"), None, CostQuality::Complete);
        assert_eq!(missing.status, UsageStatus::Unknown);
        assert_eq!(missing.reason, Some("daily_budget_missing"));

        for invalid_budget in ["0", "-1", "not-a-budget", "1e1"] {
            let classification =
                static_metered(Some("8"), Some(invalid_budget), CostQuality::Complete);
            assert_eq!(
                classification.status,
                UsageStatus::Unknown,
                "{invalid_budget}",
            );
            assert_eq!(classification.reason, Some("invalid_daily_budget"));
        }
    }

    #[test]
    fn unavailable_missing_and_invalid_costs_are_unknown() {
        let unavailable = static_metered(Some("8"), Some("10"), CostQuality::Unavailable);
        assert_eq!(unavailable.status, UsageStatus::Unknown);
        assert_eq!(unavailable.reason, Some("cost_unavailable"));

        let missing = static_metered(None, Some("10"), CostQuality::Complete);
        assert_eq!(missing.status, UsageStatus::Unknown);
        assert_eq!(missing.reason, Some("cost_unavailable"));

        for invalid_cost in ["-0.01", "not-a-cost", "8e0"] {
            let classification =
                static_metered(Some(invalid_cost), Some("10"), CostQuality::Complete);
            assert_eq!(
                classification.status,
                UsageStatus::Unknown,
                "{invalid_cost}",
            );
            assert_eq!(classification.reason, Some("invalid_cost"));
        }
    }

    #[test]
    fn metered_reason_priority_is_fixed_and_never_echoes_inputs() {
        let secret_cost = "secret-cost-sentinel";
        let secret_budget = "secret-budget-sentinel";

        let missing_budget = static_metered(Some(secret_cost), None, CostQuality::Unavailable);
        assert_eq!(missing_budget.reason, Some("daily_budget_missing"));

        let invalid_budget = static_metered(
            Some(secret_cost),
            Some(secret_budget),
            CostQuality::Unavailable,
        );
        assert_eq!(invalid_budget.reason, Some("invalid_daily_budget"));

        let unavailable = static_metered(Some(secret_cost), Some("10"), CostQuality::Unavailable);
        assert_eq!(unavailable.reason, Some("cost_unavailable"));

        let invalid_cost = static_metered(Some(secret_cost), Some("10"), CostQuality::Complete);
        assert_eq!(invalid_cost.reason, Some("invalid_cost"));

        for classification in [missing_budget, invalid_budget, unavailable, invalid_cost] {
            let reason = classification.reason.unwrap();
            assert!(!reason.contains(secret_cost));
            assert!(!reason.contains(secret_budget));
        }
    }

    #[test]
    fn unknown_never_erases_a_known_alert() {
        assert_eq!(
            worst_status([
                UsageStatus::Unknown,
                UsageStatus::Green,
                UsageStatus::Red,
                UsageStatus::Yellow,
            ]),
            UsageStatus::Red,
        );
    }

    #[test]
    fn all_unknown_or_empty_aggregation_stays_unknown() {
        assert_eq!(
            worst_status([UsageStatus::Unknown, UsageStatus::Unknown]),
            UsageStatus::Unknown,
        );
        assert_eq!(worst_status([]), UsageStatus::Unknown);
    }

    #[test]
    fn worst_status_is_order_independent_for_every_permutation() {
        let values = [
            UsageStatus::Unknown,
            UsageStatus::Green,
            UsageStatus::Yellow,
            UsageStatus::Red,
        ];

        for a in 0..4 {
            for b in 0..4 {
                for c in 0..4 {
                    for d in 0..4 {
                        let permutation = [a, b, c, d];
                        let mut sorted = permutation;
                        sorted.sort_unstable();
                        if sorted != [0, 1, 2, 3] {
                            continue;
                        }

                        assert_eq!(
                            worst_status(permutation.map(|index| values[index])),
                            UsageStatus::Red,
                            "{permutation:?}",
                        );
                    }
                }
            }
        }

        assert_eq!(
            worst_status([
                UsageStatus::Unknown,
                UsageStatus::Green,
                UsageStatus::Yellow,
            ]),
            UsageStatus::Yellow,
        );
        assert_eq!(
            worst_status([UsageStatus::Unknown, UsageStatus::Green]),
            UsageStatus::Green,
        );
    }

    #[test]
    fn status_and_cost_quality_use_stable_wire_values() {
        assert_eq!(
            serde_json::to_string(&UsageStatus::Yellow).unwrap(),
            "\"yellow\""
        );
        assert_eq!(
            serde_json::to_string(&CostQuality::Estimated).unwrap(),
            "\"estimated\"",
        );
    }
}
