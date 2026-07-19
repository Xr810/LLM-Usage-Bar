use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

const INVALID_QUOTA_PERCENT: &str = "invalid_quota_percent";
const QUOTA_UNAVAILABLE: &str = "quota_unavailable";
const DAILY_BUDGET_MISSING: &str = "daily_budget_missing";
const INVALID_DAILY_BUDGET: &str = "invalid_daily_budget";
const COST_UNAVAILABLE: &str = "cost_unavailable";
const INVALID_COST: &str = "invalid_cost";
const PARTIAL_COST: &str = "partial_cost";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageStatus {
    Green,
    Yellow,
    Red,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceClassification {
    pub status: UsageStatus,
    pub used_percent: Option<String>,
    pub remaining_percent: Option<String>,
    pub consumed_percent: Option<String>,
    pub reason: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubscriptionThresholds {
    pub warning_remaining_percent: u8,
    pub critical_remaining_percent: u8,
}

impl Default for SubscriptionThresholds {
    fn default() -> Self {
        Self {
            warning_remaining_percent: 50,
            critical_remaining_percent: 20,
        }
    }
}

impl SourceClassification {
    fn unknown(reason: &'static str) -> Self {
        Self {
            status: UsageStatus::Unknown,
            used_percent: None,
            remaining_percent: None,
            consumed_percent: None,
            reason: Some(reason),
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
    }
}

pub fn classify_metered(
    numeric_cost_usd: Option<&str>,
    daily_budget_usd: Option<&str>,
    quality: CostQuality,
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

    let Some(yellow_boundary) = budget.checked_mul(Decimal::new(5, 1)) else {
        return SourceClassification::unknown(INVALID_DAILY_BUDGET);
    };
    let Some(red_boundary) = budget.checked_mul(Decimal::new(8, 1)) else {
        return SourceClassification::unknown(INVALID_DAILY_BUDGET);
    };
    let threshold_status = if cost >= red_boundary {
        UsageStatus::Red
    } else if cost >= yellow_boundary {
        UsageStatus::Yellow
    } else {
        UsageStatus::Green
    };
    let consumed_percent = cost
        .checked_div(budget)
        .and_then(|ratio| ratio.checked_mul(Decimal::ONE_HUNDRED))
        .map(|value| value.normalize().to_string());

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
    }
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
    fn metered_budget_boundaries_are_exact() {
        for (cost, expected) in [
            ("4.999", UsageStatus::Green),
            ("5", UsageStatus::Yellow),
            ("7.999", UsageStatus::Yellow),
            ("8", UsageStatus::Red),
        ] {
            assert_eq!(
                classify_metered(Some(cost), Some("10"), CostQuality::Complete).status,
                expected,
                "{cost}",
            );
        }
    }

    #[test]
    fn estimated_cost_participates_in_classification() {
        let classification = classify_metered(Some("8"), Some("10"), CostQuality::Estimated);

        assert_eq!(classification.status, UsageStatus::Red);
        assert_eq!(classification.consumed_percent.as_deref(), Some("80"));
        assert_eq!(classification.reason, None);
    }

    #[test]
    fn extreme_metered_values_classify_without_overflowing() {
        let classification = classify_metered(
            Some("79228162514264337593543950335"),
            Some("0.0000000000000000000000000001"),
            CostQuality::Complete,
        );

        assert_eq!(classification.status, UsageStatus::Red);
        assert_eq!(classification.reason, None);
    }

    #[test]
    fn partial_cost_only_proves_escalation() {
        let below_warning = classify_metered(Some("4"), Some("10"), CostQuality::Partial);
        assert_eq!(below_warning.status, UsageStatus::Unknown);
        assert_eq!(below_warning.consumed_percent.as_deref(), Some("40"));
        assert_eq!(below_warning.reason, Some("partial_cost"));

        let warning = classify_metered(Some("5"), Some("10"), CostQuality::Partial);
        assert_eq!(warning.status, UsageStatus::Yellow);
        assert_eq!(warning.reason, Some("partial_cost"));

        let critical = classify_metered(Some("8"), Some("10"), CostQuality::Partial);
        assert_eq!(critical.status, UsageStatus::Red);
        assert_eq!(critical.reason, Some("partial_cost"));
    }

    #[test]
    fn missing_and_invalid_budgets_are_unknown() {
        let missing = classify_metered(Some("8"), None, CostQuality::Complete);
        assert_eq!(missing.status, UsageStatus::Unknown);
        assert_eq!(missing.reason, Some("daily_budget_missing"));

        for invalid_budget in ["0", "-1", "not-a-budget", "1e1"] {
            let classification =
                classify_metered(Some("8"), Some(invalid_budget), CostQuality::Complete);
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
        let unavailable = classify_metered(Some("8"), Some("10"), CostQuality::Unavailable);
        assert_eq!(unavailable.status, UsageStatus::Unknown);
        assert_eq!(unavailable.reason, Some("cost_unavailable"));

        let missing = classify_metered(None, Some("10"), CostQuality::Complete);
        assert_eq!(missing.status, UsageStatus::Unknown);
        assert_eq!(missing.reason, Some("cost_unavailable"));

        for invalid_cost in ["-0.01", "not-a-cost", "8e0"] {
            let classification =
                classify_metered(Some(invalid_cost), Some("10"), CostQuality::Complete);
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

        let missing_budget = classify_metered(Some(secret_cost), None, CostQuality::Unavailable);
        assert_eq!(missing_budget.reason, Some("daily_budget_missing"));

        let invalid_budget = classify_metered(
            Some(secret_cost),
            Some(secret_budget),
            CostQuality::Unavailable,
        );
        assert_eq!(invalid_budget.reason, Some("invalid_daily_budget"));

        let unavailable = classify_metered(Some(secret_cost), Some("10"), CostQuality::Unavailable);
        assert_eq!(unavailable.reason, Some("cost_unavailable"));

        let invalid_cost = classify_metered(Some(secret_cost), Some("10"), CostQuality::Complete);
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
