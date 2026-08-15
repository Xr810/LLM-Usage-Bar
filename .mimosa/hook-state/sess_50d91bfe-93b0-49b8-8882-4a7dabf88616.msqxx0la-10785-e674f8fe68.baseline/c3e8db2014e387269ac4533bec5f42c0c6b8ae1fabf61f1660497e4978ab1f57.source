use crate::error::AppError;
use crate::store::AppState;
use crate::usage::aggregation::aggregate_enabled_provider_trend;
use crate::usage::domain::UsageTrendBucketView;
use tauri::State;

#[tauri::command]
pub fn get_provider_usage_activity(
    state: State<'_, AppState>,
    start_at: i64,
    end_at: i64,
) -> Result<Vec<UsageTrendBucketView>, AppError> {
    let (_, buckets) = aggregate_enabled_provider_trend(&state.db, start_at, end_at)?;
    Ok(buckets)
}
