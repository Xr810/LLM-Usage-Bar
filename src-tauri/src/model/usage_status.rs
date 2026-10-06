//! Shared usage verdicts and their presentation values. Classification policy
//! and rhythm calculations belong to `usage`, not these domain types.

use chrono::{DateTime, SecondsFormat};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

pub(crate) const FIVE_HOUR_WINDOW_KIND: &str = "five_hour";
pub(crate) const SEVEN_DAY_WINDOW_KIND: &str = "seven_day";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageStatus {
    Green,
    Yellow,
    Red,
    #[default]
    Unknown,
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
    /// Verdict without the rhythm profile; present only when it was applied.
    pub flat_status: Option<UsageStatus>,
}

impl SourceClassification {
    /// Quota-percent per hour for subscription windows, USD per hour for budgets.
    pub fn burn_rate_per_hour(&self) -> Option<String> {
        self.burn_rate_per_second
            .and_then(|rate| rate.checked_mul(Decimal::from(3_600)))
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_status_types_preserve_wire_values_and_defaults() {
        for (value, wire) in [
            (UsageStatus::Green, "green"),
            (UsageStatus::Yellow, "yellow"),
            (UsageStatus::Red, "red"),
            (UsageStatus::Unknown, "unknown"),
        ] {
            let json = serde_json::Value::String(wire.into());
            assert_eq!(serde_json::to_value(value).unwrap(), json);
            assert_eq!(serde_json::from_value::<UsageStatus>(json).unwrap(), value);
        }
        for (value, wire) in [
            (PaceBasis::Measured, "measured"),
            (PaceBasis::WindowAverage, "window_average"),
            (PaceBasis::Static, "static"),
            (PaceBasis::Idle, "idle"),
        ] {
            let json = serde_json::Value::String(wire.into());
            assert_eq!(serde_json::to_value(value).unwrap(), json);
            assert_eq!(serde_json::from_value::<PaceBasis>(json).unwrap(), value);
        }
        assert_eq!(UsageStatus::default(), UsageStatus::Unknown);
        assert_eq!(PaceBasis::default(), PaceBasis::Static);
    }

    #[test]
    fn classification_formats_units_without_losing_absence_or_overflow() {
        let mut classification = SourceClassification {
            status: UsageStatus::Yellow,
            used_percent: None,
            remaining_percent: None,
            consumed_percent: None,
            reason: None,
            burn_rate_per_second: Some(Decimal::new(125, 5)),
            projected_exhaust_at: Some(3_661),
            headroom_ratio: Some(Decimal::new(1230, 3)),
            pace_basis: PaceBasis::Measured,
            rhythm_adjustment: Some(Decimal::new(850, 3)),
            flat_status: Some(UsageStatus::Red),
        };
        assert_eq!(classification.burn_rate_per_hour().as_deref(), Some("4.5"));
        assert_eq!(
            classification.projected_exhaust_at_rfc3339().as_deref(),
            Some("1970-01-01T01:01:01Z")
        );
        assert_eq!(
            classification.headroom_ratio_string().as_deref(),
            Some("1.23")
        );
        assert_eq!(
            classification.rhythm_adjustment_string().as_deref(),
            Some("0.85")
        );

        classification.burn_rate_per_second = Some(Decimal::MAX);
        classification.projected_exhaust_at = Some(i64::MAX);
        assert_eq!(classification.burn_rate_per_hour(), None);
        assert_eq!(classification.projected_exhaust_at_rfc3339(), None);

        classification.burn_rate_per_second = None;
        classification.projected_exhaust_at = None;
        classification.headroom_ratio = None;
        classification.rhythm_adjustment = None;
        assert_eq!(classification.burn_rate_per_hour(), None);
        assert_eq!(classification.projected_exhaust_at_rfc3339(), None);
        assert_eq!(classification.headroom_ratio_string(), None);
        assert_eq!(classification.rhythm_adjustment_string(), None);
    }
}
