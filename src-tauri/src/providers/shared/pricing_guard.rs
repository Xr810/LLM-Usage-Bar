//! Import-time price guard (BE-5, ADR v7.7.1).
//!
//! The guard looks at input and output only. Cache rates follow that decision
//! and never block a row on their own. A blocked price stays pending until a
//! later fetch, at least 24 hours after the first sighting, repeats it.

use rust_decimal::Decimal;
use std::collections::BTreeMap;

pub const CONFIRM_INTERVAL_SECS: i64 = 24 * 60 * 60;
/// Cleaned model id → published id that wins a collision. Empty until a real
/// override is added. Tests pass their own slice into [`resolve_collisions`].
pub const PRICING_MODEL_ID_OVERRIDES: &[(&str, &str)] = &[];

pub const REASON_CHANGE_TOO_LARGE: &str = "change_too_large";
pub const REASON_ZERO_PRICE: &str = "zero_price";
pub const REASON_INVALID_VALUE: &str = "invalid_value";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogModel {
    pub published_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub display_name: String,
    pub input: Decimal,
    pub output: Decimal,
    pub cache_read: Option<String>,
    pub cache_write: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collision {
    pub model_id: String,
    pub published_ids: Vec<String>,
    pub chosen: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingQuote {
    pub input: Decimal,
    pub output: Decimal,
    pub first_seen_at: i64,
    pub seen_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PriceDecision {
    /// Write the new price and clear any pending quote.
    Accept { confirmed: bool },
    /// Keep the stored price. `reset` starts the 24 hour window over.
    Pending { reason: &'static str, reset: bool },
}

pub fn below_half(new_count: u64, previous_count: u64) -> bool {
    new_count.saturating_mul(2) < previous_count
}

pub fn decide_price(
    old_input: Option<Decimal>,
    old_output: Option<Decimal>,
    pending: Option<&PendingQuote>,
    new_input: Decimal,
    new_output: Decimal,
    now: i64,
) -> PriceDecision {
    let (Some(old_input), Some(old_output)) = (old_input, old_output) else {
        return PriceDecision::Accept { confirmed: false };
    };
    let Some(reason) = block_reason(old_input, old_output, new_input, new_output) else {
        return PriceDecision::Accept { confirmed: false };
    };
    if let Some(pending) = pending {
        if pending.input == new_input
            && pending.output == new_output
            && now.saturating_sub(pending.first_seen_at) >= CONFIRM_INTERVAL_SECS
        {
            return PriceDecision::Accept { confirmed: true };
        }
        let same = pending.input == new_input && pending.output == new_output;
        return PriceDecision::Pending {
            reason,
            reset: !same,
        };
    }
    PriceDecision::Pending {
        reason,
        reset: true,
    }
}

pub fn resolve_collisions(
    models: Vec<CatalogModel>,
    overrides: &[(&str, &str)],
) -> (Vec<CatalogModel>, Vec<Collision>) {
    let mut groups: BTreeMap<String, Vec<CatalogModel>> = BTreeMap::new();
    for model in models {
        groups
            .entry(model.model_id.clone())
            .or_default()
            .push(model);
    }

    let mut chosen = Vec::new();
    let mut collisions = Vec::new();
    for (model_id, group) in groups {
        if group.len() == 1 {
            chosen.push(group.into_iter().next().unwrap());
            continue;
        }
        let index = choose_index(&group, overrides);
        let mut published_ids: Vec<String> = group
            .iter()
            .map(|model| model.published_id.clone())
            .collect();
        published_ids.sort();
        let winner = group[index].published_id.clone();
        collisions.push(Collision {
            model_id,
            published_ids,
            chosen: winner,
        });
        chosen.push(group.into_iter().nth(index).unwrap());
    }
    (chosen, collisions)
}

/// Providers whose incoming count is strictly below half of the stored count.
/// A provider that drops to zero is included. An empty stored catalog is not.
pub fn halved_providers(stored: &[(String, u64)], incoming: &[(String, u64)]) -> Vec<String> {
    let mut incoming_counts: BTreeMap<&str, u64> = BTreeMap::new();
    for (provider, count) in incoming {
        *incoming_counts.entry(provider.as_str()).or_default() += *count;
    }
    let mut warned = Vec::new();
    for (provider, stored_count) in stored {
        if *stored_count == 0 {
            continue;
        }
        let incoming_count = incoming_counts.get(provider.as_str()).copied().unwrap_or(0);
        if below_half(incoming_count, *stored_count) {
            warned.push(provider.clone());
        }
    }
    warned.sort();
    warned
}

fn block_reason(
    old_input: Decimal,
    old_output: Decimal,
    new_input: Decimal,
    new_output: Decimal,
) -> Option<&'static str> {
    let input = component_reason(old_input, new_input);
    let output = component_reason(old_output, new_output);
    match (input, output) {
        (Some(REASON_ZERO_PRICE), _) | (_, Some(REASON_ZERO_PRICE)) => Some(REASON_ZERO_PRICE),
        (Some(reason), _) | (_, Some(reason)) => Some(reason),
        (None, None) => None,
    }
}

fn component_reason(old: Decimal, new: Decimal) -> Option<&'static str> {
    if old > Decimal::ZERO && new.is_zero() {
        return Some(REASON_ZERO_PRICE);
    }
    if old.is_zero() || new.is_zero() {
        return None;
    }
    let ten = Decimal::from(10);
    if new > old * ten || new * ten < old {
        Some(REASON_CHANGE_TOO_LARGE)
    } else {
        None
    }
}

fn choose_index(group: &[CatalogModel], overrides: &[(&str, &str)]) -> usize {
    let model_id = &group[0].model_id;
    if let Some((_, published)) = overrides.iter().find(|(cleaned, _)| cleaned == model_id) {
        if let Some(index) = group
            .iter()
            .position(|model| model.published_id == *published)
        {
            return index;
        }
    }
    let exact: Vec<usize> = group
        .iter()
        .enumerate()
        .filter(|(_, model)| model.published_id == model.model_id)
        .map(|(index, _)| index)
        .collect();
    if !exact.is_empty() {
        return *exact
            .iter()
            .min_by(|left, right| {
                group[**left]
                    .published_id
                    .as_bytes()
                    .cmp(group[**right].published_id.as_bytes())
            })
            .unwrap();
    }
    group
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| {
            left.published_id
                .as_bytes()
                .cmp(right.published_id.as_bytes())
        })
        .map(|(index, _)| index)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn model(published: &str, cleaned: &str, input: i64, output: i64) -> CatalogModel {
        CatalogModel {
            published_id: published.to_string(),
            provider_id: "anthropic".to_string(),
            model_id: cleaned.to_string(),
            display_name: published.to_string(),
            input: Decimal::from(input),
            output: Decimal::from(output),
            cache_read: None,
            cache_write: None,
        }
    }

    fn quote(input: i64, output: i64, first_seen_at: i64) -> PendingQuote {
        PendingQuote {
            input: Decimal::from(input),
            output: Decimal::from(output),
            first_seen_at,
            seen_count: 1,
        }
    }

    #[test]
    fn exact_five_times_and_one_fifth_pass_and_ten_times_is_the_open_boundary() {
        let old_in = Some(Decimal::from(2));
        let old_out = Some(Decimal::from(10));
        assert_eq!(
            decide_price(
                old_in,
                old_out,
                None,
                Decimal::from(10),
                Decimal::from(10),
                0
            ),
            PriceDecision::Accept { confirmed: false },
            "5× input"
        );
        assert_eq!(
            decide_price(old_in, old_out, None, Decimal::from(2), Decimal::from(2), 0),
            PriceDecision::Accept { confirmed: false },
            "1/5 output"
        );
        assert_eq!(
            decide_price(
                old_in,
                old_out,
                None,
                Decimal::from(20),
                Decimal::from(10),
                0
            ),
            PriceDecision::Accept { confirmed: false },
            "exactly 10×"
        );
        assert_eq!(
            decide_price(old_in, old_out, None, Decimal::from(2), Decimal::from(1), 0),
            PriceDecision::Accept { confirmed: false },
            "exactly 1/10"
        );
        assert_eq!(
            decide_price(
                old_in,
                old_out,
                None,
                Decimal::from(21),
                Decimal::from(10),
                0
            ),
            PriceDecision::Pending {
                reason: REASON_CHANGE_TOO_LARGE,
                reset: true,
            }
        );
    }

    #[test]
    fn pending_accepts_only_after_twenty_four_hours_of_the_same_price() {
        let pending = quote(21, 10, 1_000);
        let blocked = decide_price(
            Some(Decimal::from(2)),
            Some(Decimal::from(10)),
            Some(&pending),
            Decimal::from(21),
            Decimal::from(10),
            1_000 + CONFIRM_INTERVAL_SECS - 1,
        );
        assert_eq!(
            blocked,
            PriceDecision::Pending {
                reason: REASON_CHANGE_TOO_LARGE,
                reset: false,
            }
        );
        let accepted = decide_price(
            Some(Decimal::from(2)),
            Some(Decimal::from(10)),
            Some(&pending),
            Decimal::from(21),
            Decimal::from(10),
            1_000 + CONFIRM_INTERVAL_SECS,
        );
        assert_eq!(accepted, PriceDecision::Accept { confirmed: true });
    }

    #[test]
    fn a_changed_pending_price_restarts_the_window() {
        let pending = quote(21, 10, 1_000);
        let decision = decide_price(
            Some(Decimal::from(2)),
            Some(Decimal::from(10)),
            Some(&pending),
            Decimal::from(30),
            Decimal::from(10),
            1_000 + CONFIRM_INTERVAL_SECS,
        );
        assert_eq!(
            decision,
            PriceDecision::Pending {
                reason: REASON_CHANGE_TOO_LARGE,
                reset: true,
            }
        );
    }

    #[test]
    fn zero_price_needs_two_confirmations_and_zero_to_positive_does_not() {
        assert_eq!(
            decide_price(
                Some(Decimal::from(4)),
                Some(Decimal::from(8)),
                None,
                Decimal::ZERO,
                Decimal::from(8),
                0
            ),
            PriceDecision::Pending {
                reason: REASON_ZERO_PRICE,
                reset: true,
            }
        );
        assert_eq!(
            decide_price(
                Some(Decimal::ZERO),
                Some(Decimal::ZERO),
                None,
                Decimal::from(4),
                Decimal::from(8),
                0
            ),
            PriceDecision::Accept { confirmed: false }
        );
        assert_eq!(
            decide_price(None, None, None, Decimal::from(4), Decimal::ZERO, 0),
            PriceDecision::Accept { confirmed: false }
        );
    }

    #[test]
    fn half_batch_keeps_an_exact_half_and_discards_one_below() {
        assert!(!below_half(5, 10));
        assert!(below_half(4, 10));
        assert!(!below_half(6, 11));
        assert!(below_half(5, 11));
    }

    #[test]
    fn collisions_prefer_override_then_exact_id_then_byte_order_not_price() {
        let expensive = model("aaa/claude-sonnet-4-5", "claude-sonnet-4-5", 99, 99);
        let exact = model("claude-sonnet-4-5", "claude-sonnet-4-5", 3, 15);
        let other = model("zzz/claude-sonnet-4-5", "claude-sonnet-4-5", 1, 1);
        let mut first = None;
        for seed in 0..10 {
            let mut batch = vec![expensive.clone(), exact.clone(), other.clone()];
            shuffle(&mut batch, seed);
            let (chosen, collisions) = resolve_collisions(batch, &[]);
            assert_eq!(chosen.len(), 1);
            assert_eq!(chosen[0].input, Decimal::from(3));
            assert_eq!(collisions[0].chosen, "claude-sonnet-4-5");
            first.get_or_insert(chosen[0].input);
        }
        let (overridden, _) = resolve_collisions(
            vec![exact, expensive.clone()],
            &[("claude-sonnet-4-5", "aaa/claude-sonnet-4-5")],
        );
        assert_eq!(overridden[0].published_id, "aaa/claude-sonnet-4-5");
        assert_eq!(overridden[0].input, Decimal::from(99));

        let (byte_order, collisions) = resolve_collisions(
            vec![
                model("zzz/claude-sonnet-4-5", "claude-sonnet-4-5", 99, 1),
                model("aaa/claude-sonnet-4-5", "claude-sonnet-4-5", 1, 1),
            ],
            &[],
        );
        assert_eq!(byte_order[0].published_id, "aaa/claude-sonnet-4-5");
        assert_eq!(byte_order[0].input, Decimal::from(1));
        assert_eq!(
            collisions[0].published_ids,
            vec![
                "aaa/claude-sonnet-4-5".to_string(),
                "zzz/claude-sonnet-4-5".to_string()
            ]
        );
    }

    #[test]
    fn one_provider_dropping_below_half_is_reported_without_using_price() {
        let stored = vec![("xai".to_string(), 4), ("openai".to_string(), 2)];
        let incoming = vec![("openai".to_string(), 2)];
        assert_eq!(
            halved_providers(&stored, &incoming),
            vec!["xai".to_string()]
        );
    }

    fn shuffle(models: &mut [CatalogModel], seed: u64) {
        let mut state = seed | 1;
        for index in (1..models.len()).rev() {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let swap_with = (state >> 33) as usize % (index + 1);
            models.swap(index, swap_with);
        }
    }
}
