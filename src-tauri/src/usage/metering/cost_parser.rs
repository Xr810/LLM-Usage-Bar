use crate::error::AppError;
use rust_decimal::Decimal;
use serde_json::Value;
use std::str::FromStr;

/// Cost values explicitly returned by the upstream API.
///
/// Missing components stay absent. In particular, a missing component is not
/// converted to zero because that would make an unknown value look trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamCost {
    pub input_cost: Option<Decimal>,
    pub output_cost: Option<Decimal>,
    pub cache_read_cost: Option<Decimal>,
    pub cache_creation_cost: Option<Decimal>,
    pub total_cost: Option<Decimal>,
}

impl UpstreamCost {
    pub fn has_any_value(&self) -> bool {
        self.input_cost.is_some()
            || self.output_cost.is_some()
            || self.cache_read_cost.is_some()
            || self.cache_creation_cost.is_some()
            || self.total_cost.is_some()
    }
}

fn parse_explicit_decimal(value: &Value, field: &str) -> Result<Decimal, AppError> {
    let raw = match value {
        Value::Number(number) => number.to_string(),
        Value::String(value) => value.clone(),
        _ => {
            return Err(AppError::Message(format!(
                "upstream cost field {field} must be a decimal number or string"
            )))
        }
    };
    let parsed = Decimal::from_str(raw.trim()).map_err(|_| {
        AppError::Message(format!(
            "upstream cost field {field} must contain a valid decimal"
        ))
    })?;
    if parsed.is_sign_negative() {
        return Err(AppError::Message(format!(
            "upstream cost field {field} must be non-negative"
        )));
    }
    Ok(parsed)
}

fn optional_decimal(
    object: Option<&serde_json::Map<String, Value>>,
    field: &str,
) -> Result<Option<Decimal>, AppError> {
    object
        .and_then(|object| object.get(field))
        .filter(|value| !value.is_null())
        .map(|value| parse_explicit_decimal(value, field))
        .transpose()
}

/// Extract only explicitly documented upstream cost fields.
///
/// Token counts, prices, model names, or nearby totals are deliberately not
/// interpreted as costs. This keeps `CostSource::Upstream` reserved for values
/// that the upstream actually labelled as monetary cost.
pub fn extract_upstream_cost(body: &Value) -> Result<Option<UpstreamCost>, AppError> {
    let Some(usage) = body.get("usage").and_then(Value::as_object) else {
        return Ok(None);
    };
    let details = usage.get("cost_details").and_then(Value::as_object);
    let total = usage
        .get("total_cost")
        .filter(|value| !value.is_null())
        .or_else(|| usage.get("cost").filter(|value| !value.is_null()))
        .map(|value| parse_explicit_decimal(value, "usage.total_cost"))
        .transpose()?
        .or(optional_decimal(details, "total_cost")?);
    let parsed = UpstreamCost {
        input_cost: optional_decimal(details, "input_cost")?,
        output_cost: optional_decimal(details, "output_cost")?,
        cache_read_cost: optional_decimal(details, "cache_read_cost")?,
        cache_creation_cost: optional_decimal(details, "cache_creation_cost")?,
        total_cost: total,
    };
    Ok(parsed.has_any_value().then_some(parsed))
}

/// Inspect collected SSE JSON values and return the last explicit cost block.
/// Terminal usage events normally carry the final total, so the last match is
/// the authoritative one when an upstream emits partial usage earlier.
pub fn extract_upstream_cost_from_events(
    events: &[Value],
) -> Result<Option<UpstreamCost>, AppError> {
    let mut latest = None;
    for event in events {
        if let Some(cost) = extract_upstream_cost(event)? {
            latest = Some(cost);
        }
        if let Some(response) = event.get("response") {
            if let Some(cost) = extract_upstream_cost(response)? {
                latest = Some(cost);
            }
        }
        if let Some(message) = event.get("message") {
            if let Some(cost) = extract_upstream_cost(message)? {
                latest = Some(cost);
            }
        }
    }
    Ok(latest)
}

#[cfg(test)]
mod tests {
    use super::{extract_upstream_cost, extract_upstream_cost_from_events, UpstreamCost};
    use rust_decimal::Decimal;
    use serde_json::json;
    use std::str::FromStr;

    #[test]
    fn parses_explicit_total_and_documented_parts() {
        let parsed = extract_upstream_cost(&json!({
            "usage": {
                "total_cost": "0.42",
                "cost_details": {
                    "input_cost": 0.1,
                    "output_cost": "0.2",
                    "cache_read_cost": "0.03",
                    "cache_creation_cost": "0.09"
                }
            }
        }))
        .unwrap()
        .unwrap();

        assert_eq!(
            parsed,
            UpstreamCost {
                input_cost: Some(Decimal::from_str("0.1").unwrap()),
                output_cost: Some(Decimal::from_str("0.2").unwrap()),
                cache_read_cost: Some(Decimal::from_str("0.03").unwrap()),
                cache_creation_cost: Some(Decimal::from_str("0.09").unwrap()),
                total_cost: Some(Decimal::from_str("0.42").unwrap()),
            }
        );
    }

    #[test]
    fn explicit_numeric_zero_is_preserved() {
        let parsed = extract_upstream_cost(&json!({"usage": {"cost": 0}}))
            .unwrap()
            .unwrap();
        assert_eq!(parsed.total_cost, Some(Decimal::ZERO));
    }

    #[test]
    fn missing_explicit_cost_is_not_invented() {
        assert_eq!(
            extract_upstream_cost(&json!({
                "usage": {"input_tokens": 1, "output_tokens": 2}
            }))
            .unwrap(),
            None
        );
    }

    #[test]
    fn explicit_null_part_remains_unknown() {
        let parsed = extract_upstream_cost(&json!({
            "usage": {
                "total_cost": "0.2",
                "cost_details": {"input_cost": null}
            }
        }))
        .unwrap()
        .unwrap();
        assert_eq!(parsed.input_cost, None);
        assert_eq!(parsed.total_cost, Some(Decimal::from_str("0.2").unwrap()));
    }

    #[test]
    fn rejects_negative_and_non_numeric_explicit_values() {
        let negative = extract_upstream_cost(&json!({"usage": {"cost": "-0.1"}}))
            .unwrap_err()
            .to_string();
        assert!(negative.contains("non-negative"), "{negative}");

        let invalid = extract_upstream_cost(&json!({"usage": {"cost": "free"}}))
            .unwrap_err()
            .to_string();
        assert!(invalid.contains("decimal"), "{invalid}");
    }

    #[test]
    fn terminal_sse_response_cost_wins_over_earlier_usage_cost() {
        let parsed = extract_upstream_cost_from_events(&[
            json!({"usage": {"cost": "0.1"}}),
            json!({
                "type": "response.completed",
                "response": {"usage": {"total_cost": "0.3"}}
            }),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(parsed.total_cost, Some(Decimal::from_str("0.3").unwrap()));
    }
}
