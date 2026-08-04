//! Schema v22: price the usage events that were ingested before their model had a price.
//!
//! `usage_events` costs are decided once, at insert time, and the table is
//! append-only behind an immutability trigger. That is the right contract for a
//! ledger, but it means an event whose model was missing from `model_pricing`
//! when it arrived is stored as `unavailable` and stays that way forever — even
//! after the catalogue learns that model.
//!
//! Every new model does this: it is in use before its price is published, so the
//! events from that window are permanently uncosted and every cost total that
//! spans them reads low. The existing backfill does not help — it rewrites
//! `proxy_request_logs`, which the dashboard does not read.
//!
//! This migration recomputes those rows once, using the same lookup and the same
//! calculator the ingest path uses, so a backfilled cost is identical to the one
//! the event would have been given had the price existed at the time. Rows whose
//! model is still unknown are left alone and will be picked up by a later
//! migration if the catalogue ever learns them.

use crate::error::AppError;
use crate::services::usage_stats::find_model_pricing_row;
use crate::usage::metering::calculator::CostCalculator;
use crate::usage::metering::calculator::ModelPricing;
use crate::usage::metering::parser::TokenUsage;
use rusqlite::{params, Connection};
use rust_decimal::Decimal;

const SAVEPOINT: &str = "cost_backfill_v22";

/// One event that carries tokens but no cost.
struct UncostedEvent {
    event_id: String,
    /// Stands in for the ingest-time app type. `decide_cost` falls back to
    /// `product_group_id` for session-log events, which is every row here, so
    /// using it reproduces the original cache semantics exactly.
    product_group_id: String,
    model: String,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_creation_tokens: u32,
}

pub(crate) fn migrate_v21_to_v22(conn: &Connection) -> Result<u64, AppError> {
    conn.execute_batch(&format!("SAVEPOINT {SAVEPOINT};"))
        .map_err(|e| AppError::Database(format!("v21 -> v22 建立保存点失败: {e}")))?;

    let result = backfill(conn);

    match &result {
        Ok(_) => conn
            .execute_batch(&format!("RELEASE {SAVEPOINT};"))
            .map_err(|e| AppError::Database(format!("v21 -> v22 提交保存点失败: {e}")))?,
        Err(_) => {
            // Roll back to the savepoint so a partial rewrite of an append-only
            // ledger never survives, then release it.
            let _ = conn.execute_batch(&format!("ROLLBACK TO {SAVEPOINT}; RELEASE {SAVEPOINT};"));
        }
    }

    result
}

fn backfill(conn: &Connection) -> Result<u64, AppError> {
    let pending = load_uncosted_events(conn)?;
    if pending.is_empty() {
        return Ok(0);
    }

    drop_update_trigger(conn)?;
    let outcome = rewrite(conn, &pending);
    // Restore the guarantee even if the rewrite failed; the savepoint above
    // undoes the rows, this undoes the missing trigger.
    recreate_update_trigger(conn)?;
    let updated = outcome?;

    if updated > 0 {
        log::info!("v21 -> v22 为 {updated} 条历史用量事件补算了成本");
    }
    Ok(updated)
}

fn load_uncosted_events(conn: &Connection) -> Result<Vec<UncostedEvent>, AppError> {
    let mut statement = conn
        .prepare(
            "SELECT event_id, product_group_id, model,
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens
             FROM usage_events
             WHERE total_cost_usd IS NULL
               AND (input_tokens > 0 OR output_tokens > 0
                    OR cache_read_tokens > 0 OR cache_creation_tokens > 0)",
        )
        .map_err(|e| AppError::Database(format!("v21 -> v22 查询待补算事件失败: {e}")))?;

    let rows = statement
        .query_map([], |row| {
            Ok(UncostedEvent {
                event_id: row.get(0)?,
                product_group_id: row.get(1)?,
                model: row.get(2)?,
                input_tokens: row.get::<_, i64>(3)?.max(0) as u32,
                output_tokens: row.get::<_, i64>(4)?.max(0) as u32,
                cache_read_tokens: row.get::<_, i64>(5)?.max(0) as u32,
                cache_creation_tokens: row.get::<_, i64>(6)?.max(0) as u32,
            })
        })
        .map_err(|e| AppError::Database(format!("v21 -> v22 读取待补算事件失败: {e}")))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::Database(format!("v21 -> v22 解析待补算事件失败: {e}")))
}

fn rewrite(conn: &Connection, pending: &[UncostedEvent]) -> Result<u64, AppError> {
    let mut updated = 0u64;
    let mut unpriced_models: Vec<String> = Vec::new();

    for event in pending {
        let Some(row) = find_model_pricing_row(conn, &event.model)? else {
            if !unpriced_models.iter().any(|model| model == &event.model) {
                unpriced_models.push(event.model.clone());
            }
            continue;
        };
        let pricing = ModelPricing::from_strings(&row.0, &row.1, &row.2, &row.3)
            .map_err(|error| AppError::Database(format!("v21 -> v22 定价无效: {error}")))?;

        let usage = TokenUsage {
            input_tokens: event.input_tokens,
            output_tokens: event.output_tokens,
            cache_read_tokens: event.cache_read_tokens,
            cache_creation_tokens: event.cache_creation_tokens,
            ..TokenUsage::default()
        };
        let cost = CostCalculator::calculate_for_app(
            &event.product_group_id,
            &usage,
            &pricing,
            Decimal::ONE,
        );

        conn.execute(
            "UPDATE usage_events
             SET input_cost_usd = ?1, output_cost_usd = ?2, cache_read_cost_usd = ?3,
                 cache_creation_cost_usd = ?4, total_cost_usd = ?5,
                 cost_source = 'estimated', pricing_origin = 'official'
             WHERE event_id = ?6",
            params![
                cost.input_cost.to_string(),
                cost.output_cost.to_string(),
                cost.cache_read_cost.to_string(),
                cost.cache_creation_cost.to_string(),
                cost.total_cost.to_string(),
                event.event_id,
            ],
        )
        .map_err(|e| AppError::Database(format!("v21 -> v22 写入补算成本失败: {e}")))?;
        updated += 1;
    }

    if !unpriced_models.is_empty() {
        // Say what stayed uncosted rather than let a partial result read as a
        // complete one.
        unpriced_models.sort();
        log::info!(
            "v21 -> v22 仍无官方定价、未补算的模型: {}",
            unpriced_models.join(", ")
        );
    }

    Ok(updated)
}

fn drop_update_trigger(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch("DROP TRIGGER IF EXISTS usage_events_immutable_update;")
        .map_err(|e| AppError::Database(format!("v21 -> v22 摘除不可变触发器失败: {e}")))
}

fn recreate_update_trigger(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS usage_events_immutable_update
         BEFORE UPDATE ON usage_events
         BEGIN
             SELECT RAISE(ABORT, 'usage_events are immutable');
         END;",
    )
    .map_err(|e| AppError::Database(format!("v21 -> v22 恢复不可变触发器失败: {e}")))
}

/// The ledger must be immutable again once the migration has run.
pub(crate) fn validate_schema_v22_complete(conn: &Connection) -> Result<(), AppError> {
    let present: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'trigger' AND name = 'usage_events_immutable_update'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| AppError::Database(format!("v22 校验不可变触发器失败: {e}")))?;
    if present == 0 {
        return Err(AppError::Database(
            "incomplete schema v22: usage_events_immutable_update is missing".to_string(),
        ));
    }
    Ok(())
}
