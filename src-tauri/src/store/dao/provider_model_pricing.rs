//! Per-Provider-account model pricing — what the user actually pays.
//!
//! Only metered accounts may carry these rows. Subscription spend is a
//! synthetic equivalent-API figure and is always valued at the official
//! `model_pricing` catalogue, so accepting a custom price for a subscription
//! account would silently produce a number that means nothing.

use crate::error::AppError;
use crate::model::{ModelPriceInput, ProviderModelPricingView};
use crate::services::usage_stats::clean_model_id_for_pricing;
use crate::store::{lock_conn, Database};
use rusqlite::{params, OptionalExtension, Row};
use rust_decimal::Decimal;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_timestamp() -> Result<i64, AppError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|error| AppError::Database(format!("system clock before unix epoch: {error}")))
}

/// Normalize one price component. A blank means "inherit the official rate".
/// Zero remains a legitimate explicit price (a free tier), so present values
/// reject only negatives and unparseable text.
fn canonicalize_price(label: &str, raw: &str) -> Result<Option<String>, AppError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let value = Decimal::from_str(raw).map_err(|_| {
        AppError::localized(
            "usage.invalidPrice",
            format!("{label} 价格无效: {raw}"),
            format!("{label} price is invalid: {raw}"),
        )
    })?;
    if value < Decimal::ZERO {
        return Err(AppError::localized(
            "usage.invalidPrice",
            format!("{label} 价格必须为非负数: {raw}"),
            format!("{label} price must be non-negative: {raw}"),
        ));
    }
    Ok(Some(value.normalize().to_string()))
}

fn row_to_view(row: &Row<'_>) -> rusqlite::Result<ProviderModelPricingView> {
    Ok(ProviderModelPricingView {
        provider_id: row.get(0)?,
        model_id: row.get(1)?,
        display_name: row.get(2)?,
        input_cost_per_million: row.get(3)?,
        output_cost_per_million: row.get(4)?,
        cache_read_cost_per_million: row.get(5)?,
        cache_creation_cost_per_million: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

const SELECT_COLUMNS: &str = "provider_id, model_id, display_name,
     input_cost_per_million, output_cost_per_million,
     cache_read_cost_per_million, cache_creation_cost_per_million, updated_at";

impl Database {
    pub fn list_provider_model_pricing(
        &self,
        provider_id: &str,
    ) -> Result<Vec<ProviderModelPricingView>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut statement = conn.prepare(&format!(
            "SELECT {SELECT_COLUMNS} FROM provider_model_pricing
             WHERE provider_id = ?1 ORDER BY model_id"
        ))?;
        let rows = statement.query_map([provider_id], row_to_view)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| AppError::Database(format!("查询 Provider 自定义定价失败: {error}")))
    }

    /// Insert or replace one `(provider_id, model_id)` price.
    ///
    /// Takes effect for events ingested from now on. Events already stored keep
    /// the price that applied when they were recorded — `usage_events` is
    /// append-only evidence and is never rewritten.
    pub fn upsert_provider_model_pricing(
        &self,
        provider_id: &str,
        model_id: &str,
        display_name: &str,
        price: &ModelPriceInput,
    ) -> Result<(), AppError> {
        let model_id = clean_model_id_for_pricing(model_id);
        if model_id.is_empty() {
            return Err(AppError::localized(
                "usage.modelIdRequired",
                "模型 ID 不能为空",
                "Model ID is required",
            ));
        }
        let input_cost = canonicalize_price("input_cost", &price.input_cost_per_million)?;
        let output_cost = canonicalize_price("output_cost", &price.output_cost_per_million)?;
        let cache_read_cost =
            canonicalize_price("cache_read_cost", &price.cache_read_cost_per_million)?;
        let cache_creation_cost = canonicalize_price(
            "cache_creation_cost",
            &price.cache_creation_cost_per_million,
        )?;
        let now = now_timestamp()?;

        let conn = lock_conn!(self.conn);
        let billing_kind: Option<String> = conn
            .query_row(
                "SELECT billing_kind FROM usage_providers WHERE id = ?1",
                [provider_id],
                |row| row.get(0),
            )
            .optional()?;
        match billing_kind.as_deref() {
            Some("metered") => {}
            Some(_) => {
                return Err(AppError::localized(
                    "usage.customPricingMeteredOnly",
                    "订阅计划按官方价折算等额 API 费用，不能设置自定义价格",
                    "Subscription plans are valued at official list prices and cannot take a custom price",
                ))
            }
            None => return Err(AppError::Message("usage provider not found".to_string())),
        }

        conn.execute(
            "INSERT INTO provider_model_pricing (
                provider_id, model_id, display_name,
                input_cost_per_million, output_cost_per_million,
                cache_read_cost_per_million, cache_creation_cost_per_million,
                created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
             ON CONFLICT(provider_id, model_id) DO UPDATE SET
                display_name = excluded.display_name,
                input_cost_per_million = excluded.input_cost_per_million,
                output_cost_per_million = excluded.output_cost_per_million,
                cache_read_cost_per_million = excluded.cache_read_cost_per_million,
                cache_creation_cost_per_million = excluded.cache_creation_cost_per_million,
                updated_at = excluded.updated_at",
            params![
                provider_id,
                model_id,
                display_name.trim(),
                input_cost,
                output_cost,
                cache_read_cost,
                cache_creation_cost,
                now,
            ],
        )
        .map_err(|error| AppError::Database(format!("保存 Provider 自定义定价失败: {error}")))?;
        Ok(())
    }

    /// Drop one custom price. Later events fall back to the upstream-reported
    /// cost, and then to the official catalogue.
    pub fn delete_provider_model_pricing(
        &self,
        provider_id: &str,
        model_id: &str,
    ) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        let model_id = clean_model_id_for_pricing(model_id);
        conn.execute(
            "DELETE FROM provider_model_pricing WHERE provider_id = ?1 AND model_id = ?2",
            params![provider_id, model_id],
        )
        .map_err(|error| AppError::Database(format!("删除 Provider 自定义定价失败: {error}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BillingKind, TokenSource, UsageProviderInput};

    fn price(input: &str, output: &str, cache_read: &str, cache_creation: &str) -> ModelPriceInput {
        ModelPriceInput {
            input_cost_per_million: input.to_string(),
            output_cost_per_million: output.to_string(),
            cache_read_cost_per_million: cache_read.to_string(),
            cache_creation_cost_per_million: cache_creation.to_string(),
        }
    }

    fn provider(id: &str, billing_kind: BillingKind) -> UsageProviderInput {
        UsageProviderInput {
            id: id.to_string(),
            name: format!("Provider {id}"),
            billing_kind,
            product_group_id: "claude".to_string(),
            token_sources: vec![TokenSource::SessionLog],
            session_source_bindings: None,
            quota_source: None,
            quota_interval_seconds: None,
            route_app_type: Some("claude".to_string()),
            route_config: None,
            quota_config: None,
            enabled: true,
        }
    }

    fn db_with(providers: &[(&str, BillingKind)]) -> Database {
        let db = Database::memory().unwrap();
        for (id, kind) in providers {
            db.save_usage_provider(&provider(id, *kind)).unwrap();
        }
        db
    }

    #[test]
    fn custom_prices_are_isolated_per_provider_account() {
        let db = db_with(&[
            ("relay-a", BillingKind::Metered),
            ("relay-b", BillingKind::Metered),
        ]);

        db.upsert_provider_model_pricing(
            "relay-a",
            "claude-sonnet-5",
            "",
            &price("1.5", "7.5", "0", "0"),
        )
        .unwrap();
        db.upsert_provider_model_pricing(
            "relay-b",
            "claude-sonnet-5",
            "",
            &price("2.1", "9", "0", "0"),
        )
        .unwrap();

        let a = db.list_provider_model_pricing("relay-a").unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].input_cost_per_million.as_deref(), Some("1.5"));
        let b = db.list_provider_model_pricing("relay-b").unwrap();
        assert_eq!(b[0].input_cost_per_million.as_deref(), Some("2.1"));
    }

    #[test]
    fn upsert_replaces_an_existing_price_in_place() {
        let db = db_with(&[("relay-a", BillingKind::Metered)]);

        db.upsert_provider_model_pricing(
            "relay-a",
            "claude-sonnet-5",
            "",
            &price("3", "15", "0", "0"),
        )
        .unwrap();
        db.upsert_provider_model_pricing(
            "relay-a",
            "claude-sonnet-5",
            "",
            &price("1.5", "7.5", "0.15", "1.875"),
        )
        .unwrap();

        let rows = db.list_provider_model_pricing("relay-a").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].input_cost_per_million.as_deref(), Some("1.5"));
        assert_eq!(
            rows[0].cache_creation_cost_per_million.as_deref(),
            Some("1.875")
        );
    }

    #[test]
    fn model_ids_are_stored_with_the_same_normalization_used_by_lookup() {
        let db = db_with(&[("relay-a", BillingKind::Metered)]);

        db.upsert_provider_model_pricing(
            "relay-a",
            "  anthropic/Claude-Sonnet-5  ",
            "",
            &price("1", "2", "0", "0"),
        )
        .unwrap();

        let rows = db.list_provider_model_pricing("relay-a").unwrap();
        assert_eq!(rows[0].model_id, "claude-sonnet-5");
    }

    #[test]
    fn subscription_accounts_reject_custom_prices() {
        let db = db_with(&[("claude-max", BillingKind::Subscription)]);

        let error = db
            .upsert_provider_model_pricing(
                "claude-max",
                "claude-sonnet-5",
                "",
                &price("1", "2", "0", "0"),
            )
            .unwrap_err();

        assert!(
            format!("{error}").contains("官方价") || format!("{error}").contains("official"),
            "unexpected error: {error}"
        );
        assert!(db
            .list_provider_model_pricing("claude-max")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn negative_and_unparseable_prices_are_rejected() {
        let db = db_with(&[("relay-a", BillingKind::Metered)]);

        assert!(db
            .upsert_provider_model_pricing("relay-a", "m", "", &price("-1", "2", "0", "0"))
            .is_err());
        assert!(db
            .upsert_provider_model_pricing("relay-a", "m", "", &price("free", "2", "0", "0"))
            .is_err());
        assert!(
            db.upsert_provider_model_pricing("relay-a", "m", "", &price("0", "0", "0", "0"))
                .is_ok(),
            "zero is a real price"
        );
    }

    #[test]
    fn blank_prices_are_stored_as_null_for_official_fallback() {
        let db = db_with(&[("relay-a", BillingKind::Metered)]);

        db.upsert_provider_model_pricing("relay-a", "model-a", "", &price("1.500", " ", "\t", ""))
            .unwrap();

        let rows = db.list_provider_model_pricing("relay-a").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].input_cost_per_million.as_deref(), Some("1.5"));
        assert_eq!(rows[0].output_cost_per_million, None);
        assert_eq!(rows[0].cache_read_cost_per_million, None);
        assert_eq!(rows[0].cache_creation_cost_per_million, None);
    }

    #[test]
    fn deleting_a_price_removes_only_that_pair() {
        let db = db_with(&[("relay-a", BillingKind::Metered)]);
        db.upsert_provider_model_pricing("relay-a", "model-a", "", &price("1", "2", "0", "0"))
            .unwrap();
        db.upsert_provider_model_pricing("relay-a", "model-b", "", &price("3", "4", "0", "0"))
            .unwrap();

        db.delete_provider_model_pricing("relay-a", "model-a")
            .unwrap();

        let rows = db.list_provider_model_pricing("relay-a").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model_id, "model-b");
    }
}
