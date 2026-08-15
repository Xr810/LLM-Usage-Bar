use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::services::usage_stats::clean_model_id_for_pricing;
use rusqlite::params;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::async_runtime::JoinHandle;
use tokio::sync::watch;

pub const SOURCE_URL: &str = "https://models.dev/api.json";
pub const LAST_REFRESH_SETTING_KEY: &str = "official_pricing_last_refresh_at";
pub const LAST_IMPORTED_COUNT_SETTING_KEY: &str = "official_pricing_last_imported_count";

const STARTUP_STALE_AFTER: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const PERIODIC_REFRESH_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const FAILURE_RETRY_DELAYS: [Duration; 5] = [
    Duration::from_secs(30),
    Duration::from_secs(60),
    Duration::from_secs(300),
    Duration::from_secs(600),
    Duration::from_secs(1_800),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshOutcome {
    pub fetched_at: i64,
    pub models_imported: u64,
    pub models_skipped: u64,
    pub source_url: String,
}

#[derive(Debug, Deserialize)]
struct ProviderDocument {
    #[serde(default)]
    models: HashMap<String, ModelDocument>,
}

#[derive(Debug, Deserialize)]
struct ModelDocument {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    cost: Option<Value>,
}

#[derive(Debug)]
struct ImportedModel {
    model_id: String,
    display_name: String,
    input_cost: String,
    output_cost: String,
    cache_read_cost: String,
    cache_write_cost: String,
}

pub async fn refresh_official_pricing(db: &Database) -> Result<RefreshOutcome, AppError> {
    let response = crate::http_client::get()
        .get(SOURCE_URL)
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|error| AppError::Message(format!("official pricing request failed: {error}")))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::HttpStatus {
            status: status.as_u16(),
            body,
        });
    }

    let bytes = response.bytes().await.map_err(|error| {
        AppError::Message(format!("official pricing response read failed: {error}"))
    })?;
    let document: HashMap<String, ProviderDocument> =
        serde_json::from_slice(&bytes).map_err(|error| AppError::json(SOURCE_URL, error))?;
    import_document(db, document, unix_timestamp()?)
}

pub fn last_refresh_at(db: &Database) -> Result<Option<i64>, AppError> {
    db.get_setting(LAST_REFRESH_SETTING_KEY)?
        .map(|value| {
            value.parse::<i64>().map_err(|error| {
                AppError::Config(format!(
                    "invalid {LAST_REFRESH_SETTING_KEY} setting: {error}"
                ))
            })
        })
        .transpose()
}

pub fn last_imported_count(db: &Database) -> Result<Option<u64>, AppError> {
    db.get_setting(LAST_IMPORTED_COUNT_SETTING_KEY)?
        .map(|value| {
            value.parse::<u64>().map_err(|error| {
                AppError::Config(format!(
                    "invalid {LAST_IMPORTED_COUNT_SETTING_KEY} setting: {error}"
                ))
            })
        })
        .transpose()
}

fn import_document(
    db: &Database,
    document: HashMap<String, ProviderDocument>,
    fetched_at: i64,
) -> Result<RefreshOutcome, AppError> {
    let (models, models_skipped) = filtered_models(document);
    let models_imported = models.len() as u64;

    {
        let mut conn = lock_conn!(db.conn);
        let transaction = conn.transaction()?;
        for model in models {
            transaction.execute(
                "INSERT INTO model_pricing (
                    model_id, display_name, input_cost_per_million, output_cost_per_million,
                    cache_read_cost_per_million, cache_creation_cost_per_million
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(model_id) DO UPDATE SET
                    display_name = excluded.display_name,
                    input_cost_per_million = excluded.input_cost_per_million,
                    output_cost_per_million = excluded.output_cost_per_million,
                    cache_read_cost_per_million = excluded.cache_read_cost_per_million,
                    cache_creation_cost_per_million = excluded.cache_creation_cost_per_million",
                params![
                    model.model_id,
                    model.display_name,
                    model.input_cost,
                    model.output_cost,
                    model.cache_read_cost,
                    model.cache_write_cost,
                ],
            )?;
        }
        transaction.commit()?;
    }

    db.set_setting(
        LAST_IMPORTED_COUNT_SETTING_KEY,
        &models_imported.to_string(),
    )?;
    db.set_setting(LAST_REFRESH_SETTING_KEY, &fetched_at.to_string())?;

    Ok(RefreshOutcome {
        fetched_at,
        models_imported,
        models_skipped,
        source_url: SOURCE_URL.to_string(),
    })
}

fn filtered_models(document: HashMap<String, ProviderDocument>) -> (Vec<ImportedModel>, u64) {
    let mut imported = Vec::new();
    let mut skipped = 0;

    for (provider_id, provider) in document {
        for (map_model_id, model) in provider.models {
            let published_id = model.id.as_deref().unwrap_or(&map_model_id);
            let model_id = clean_model_id_for_pricing(published_id);
            if model_id.is_empty() || accepted_provider(&model_id) != Some(provider_id.as_str()) {
                skipped += 1;
                continue;
            }

            let Some((input_cost, output_cost, cache_read_cost, cache_write_cost)) =
                parse_cost(model.cost.as_ref())
            else {
                skipped += 1;
                continue;
            };

            imported.push(ImportedModel {
                display_name: model
                    .name
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| published_id.to_string()),
                model_id,
                input_cost,
                output_cost,
                cache_read_cost,
                cache_write_cost,
            });
        }
    }

    (imported, skipped)
}

fn accepted_provider(model_id: &str) -> Option<&'static str> {
    if model_id.starts_with("claude-") {
        Some("anthropic")
    } else if model_id.starts_with("gpt-")
        || model_id.starts_with("o1")
        || model_id.starts_with("o3")
        || model_id.starts_with("o4")
        || model_id.starts_with("o5")
        || model_id.starts_with("codex-")
    {
        Some("openai")
    } else if model_id.starts_with("gemini-") {
        Some("google")
    } else if model_id.starts_with("deepseek-") {
        Some("deepseek")
    } else if model_id.starts_with("qwen-") {
        Some("alibaba")
    } else if model_id.starts_with("kimi-") || model_id.starts_with("moonshot-") {
        Some("moonshotai")
    } else if model_id.starts_with("glm-") {
        Some("zhipuai")
    } else if model_id.starts_with("grok-") {
        Some("xai")
    } else if model_id.starts_with("mistral-") || model_id.starts_with("codestral-") {
        Some("mistral")
    } else {
        None
    }
}

fn parse_cost(cost: Option<&Value>) -> Option<(String, String, String, String)> {
    let object = cost?.as_object()?;
    let input = decimal_string(object.get("input")?)?;
    let output = decimal_string(object.get("output")?)?;
    let cache_read = object
        .get("cache_read")
        .map(decimal_string)
        .unwrap_or_else(|| Some("0".to_string()))?;
    let cache_write = object
        .get("cache_write")
        .map(decimal_string)
        .unwrap_or_else(|| Some("0".to_string()))?;
    Some((input, output, cache_read, cache_write))
}

fn decimal_string(value: &Value) -> Option<String> {
    let raw = match value {
        Value::Number(number) => number.to_string(),
        _ => return None,
    };
    let decimal = Decimal::from_str(&raw).ok()?;
    (decimal >= Decimal::ZERO).then(|| decimal.normalize().to_string())
}

fn unix_timestamp() -> Result<i64, AppError> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| AppError::Message(format!("system clock before unix epoch: {error}")))?
        .as_secs();
    i64::try_from(seconds)
        .map_err(|_| AppError::Message("system timestamp exceeds i64".to_string()))
}

pub struct OfficialPricingSchedulerHandle {
    cancel_tx: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
}

impl OfficialPricingSchedulerHandle {
    pub async fn stop(mut self) {
        let _ = self.cancel_tx.send(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for OfficialPricingSchedulerHandle {
    fn drop(&mut self) {
        let _ = self.cancel_tx.send(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub fn start_scheduler(db: Arc<Database>) -> OfficialPricingSchedulerHandle {
    let (cancel_tx, mut cancel_rx) = watch::channel(false);
    let task = tauri::async_runtime::spawn(async move {
        let startup_due = match unix_timestamp() {
            Ok(now) => match last_refresh_at(&db) {
                Ok(last_refresh) => last_refresh.is_none_or(|last| {
                    now.saturating_sub(last) >= STARTUP_STALE_AFTER.as_secs() as i64
                }),
                Err(error) => {
                    log::warn!("official pricing startup freshness check failed: {error}");
                    true
                }
            },
            Err(error) => {
                log::warn!("official pricing startup freshness check failed: {error}");
                false
            }
        };

        if startup_due && !refresh_with_backoff(&db, &mut cancel_rx).await {
            return;
        }

        loop {
            if !wait_or_cancel(PERIODIC_REFRESH_INTERVAL, &mut cancel_rx).await {
                return;
            }
            if !refresh_with_backoff(&db, &mut cancel_rx).await {
                return;
            }
        }
    });
    OfficialPricingSchedulerHandle {
        cancel_tx,
        task: Some(task),
    }
}

async fn refresh_with_backoff(db: &Database, cancel_rx: &mut watch::Receiver<bool>) -> bool {
    let mut failure_count = 0usize;
    loop {
        match refresh_official_pricing(db).await {
            Ok(outcome) => {
                log::info!(
                    "official pricing refreshed: imported={}, skipped={}",
                    outcome.models_imported,
                    outcome.models_skipped
                );
                return true;
            }
            Err(error) => {
                log::warn!("official pricing refresh failed: {error}");
                let delay = FAILURE_RETRY_DELAYS
                    [failure_count.min(FAILURE_RETRY_DELAYS.len().saturating_sub(1))];
                failure_count = failure_count.saturating_add(1);
                if !wait_or_cancel(delay, cancel_rx).await {
                    return false;
                }
            }
        }
    }
}

async fn wait_or_cancel(duration: Duration, cancel_rx: &mut watch::Receiver<bool>) -> bool {
    if *cancel_rx.borrow() {
        return false;
    }
    tokio::select! {
        changed = cancel_rx.changed() => changed.is_ok() && !*cancel_rx.borrow(),
        _ = tokio::time::sleep(duration) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn import_fixture(db: &Database, fixture: Value, fetched_at: i64) -> RefreshOutcome {
        let document = serde_json::from_value(fixture).unwrap();
        import_document(db, document, fetched_at).unwrap()
    }

    #[test]
    fn first_party_vendor_imports_while_reseller_is_skipped() {
        let db = Database::memory().unwrap();
        let outcome = import_fixture(
            &db,
            json!({
                "anthropic": {"models": {
                    "claude-sonnet-4-5": {"name": "Claude Sonnet 4.5", "cost": {"input": 3, "output": 15}}
                }},
                "venice": {"models": {
                    "claude-sonnet-4-5": {"name": "Claude Sonnet 4.5", "cost": {"input": 3.75, "output": 18.75}}
                }}
            }),
            100,
        );

        assert_eq!(outcome.models_imported, 1);
        assert_eq!(outcome.models_skipped, 1);
        let conn = db.conn.lock().unwrap();
        let prices = conn
            .query_row(
                "SELECT input_cost_per_million, output_cost_per_million FROM model_pricing WHERE model_id = 'claude-sonnet-4-5'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap();
        assert_eq!(prices, ("3".to_string(), "15".to_string()));
    }

    #[test]
    fn absent_cache_fields_default_to_zero() {
        let db = Database::memory().unwrap();
        import_fixture(
            &db,
            json!({"deepseek": {"models": {
                "deepseek-chat": {"cost": {"input": 0.27, "output": 1.1}}
            }}}),
            101,
        );

        let conn = db.conn.lock().unwrap();
        let prices = conn
            .query_row(
                "SELECT cache_read_cost_per_million, cache_creation_cost_per_million FROM model_pricing WHERE model_id = 'deepseek-chat'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap();
        assert_eq!(prices, ("0".to_string(), "0".to_string()));
    }

    #[test]
    fn malformed_or_missing_cost_is_skipped() {
        let db = Database::memory().unwrap();
        let outcome = import_fixture(
            &db,
            json!({"openai": {"models": {
                "gpt-no-cost": {},
                "gpt-missing-output": {"cost": {"input": 1}},
                "gpt-malformed": {"cost": {"input": "1", "output": 2}},
                "gpt-valid": {"cost": {"input": 1, "output": 2}}
            }}}),
            102,
        );

        assert_eq!(outcome.models_imported, 1);
        assert_eq!(outcome.models_skipped, 3);
    }

    #[test]
    fn upsert_updates_existing_row_without_duplicating_it() {
        let db = Database::memory().unwrap();
        let fixture = |input, output| {
            json!({"google": {"models": {
                "gemini-test": {"name": "Gemini Test", "cost": {"input": input, "output": output}}
            }}})
        };
        import_fixture(&db, fixture(1, 2), 103);
        import_fixture(&db, fixture(3, 4), 104);

        let conn = db.conn.lock().unwrap();
        let (count, input, output) = conn
            .query_row(
                "SELECT COUNT(*), MIN(input_cost_per_million), MIN(output_cost_per_million) FROM model_pricing WHERE model_id = 'gemini-test'",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(input, "3");
        assert_eq!(output, "4");
        drop(conn);
        assert_eq!(last_refresh_at(&db).unwrap(), Some(104));
        assert_eq!(last_imported_count(&db).unwrap(), Some(1));
    }
}
