use crate::error::AppError;
use crate::providers::shared::pricing_guard::{
    below_half, decide_price, halved_providers, resolve_collisions, CatalogModel, Collision,
    PendingQuote, PriceDecision, PRICING_MODEL_ID_OVERRIDES, REASON_INVALID_VALUE,
};
use crate::store::usage_stats::clean_model_id_for_pricing;
use crate::store::{lock_conn, Database};
use rusqlite::{params, OptionalExtension};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::async_runtime::JoinHandle;
use tokio::sync::watch;

pub const SOURCE_URL: &str = "https://models.dev/api.json";
pub const LAST_ATTEMPT_SETTING_KEY: &str = "official_pricing_last_attempt_at";
/// Compatibility key: this timestamp now advances only on accepted batches.
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
    pub last_attempt_at: i64,
    pub last_success_at: Option<i64>,
    pub models_imported: u64,
    pub models_skipped: u64,
    pub source_url: String,
    pub rejected: Vec<RejectedPrice>,
    pub pending: Vec<PendingPrice>,
    pub accepted_after_confirm: Vec<String>,
    pub collisions: Vec<CollisionReport>,
    pub batch_rejected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectedPrice {
    pub model_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPrice {
    pub model_id: String,
    pub first_seen_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollisionReport {
    pub model_id: String,
    pub published_ids: Vec<String>,
    pub chosen: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingBody {
    input: String,
    output: String,
}

pub async fn refresh_official_pricing(db: &Database) -> Result<RefreshOutcome, AppError> {
    db.set_setting(LAST_ATTEMPT_SETTING_KEY, &unix_timestamp()?.to_string())?;
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
    db.set_setting(LAST_ATTEMPT_SETTING_KEY, &fetched_at.to_string())?;
    let (models, mut rejected, models_skipped) = filtered_models(document);
    let (models, collisions) = resolve_collisions(models, PRICING_MODEL_ID_OVERRIDES);
    for collision in &collisions {
        log::warn!(
            "official pricing id collision on {}: published={:?} chosen={}",
            collision.model_id,
            collision.published_ids,
            collision.chosen
        );
    }
    let catalog_count = models.len() as u64;
    let previous_count = last_imported_count(db)?;
    if previous_count.is_some_and(|previous| below_half(catalog_count, previous)) {
        log::warn!(
            "official pricing batch discarded: {catalog_count} models is below half of the last imported count {}",
            previous_count.unwrap_or(0)
        );
        return Ok(empty_outcome(
            fetched_at,
            last_refresh_at(db)?,
            models_skipped,
            rejected,
            collisions,
            true,
        ));
    }

    warn_halved_providers(db, &models)?;

    let mut pending = Vec::new();
    let mut accepted_after_confirm = Vec::new();
    {
        let mut conn = lock_conn!(db.conn);
        let transaction = conn.transaction()?;
        for model in &models {
            apply_model(
                &transaction,
                model,
                fetched_at,
                &mut rejected,
                &mut pending,
                &mut accepted_after_confirm,
            )?;
        }
        transaction.commit()?;
    }

    rejected.sort_by(|left, right| left.model_id.cmp(&right.model_id));
    pending.sort_by(|left, right| left.model_id.cmp(&right.model_id));
    accepted_after_confirm.sort();

    db.set_setting(LAST_IMPORTED_COUNT_SETTING_KEY, &catalog_count.to_string())?;
    db.set_setting(LAST_REFRESH_SETTING_KEY, &fetched_at.to_string())?;

    Ok(RefreshOutcome {
        fetched_at,
        last_attempt_at: fetched_at,
        last_success_at: Some(fetched_at),
        models_imported: catalog_count,
        models_skipped,
        source_url: SOURCE_URL.to_string(),
        rejected,
        pending,
        accepted_after_confirm,
        collisions: collision_reports(collisions),
        batch_rejected: false,
    })
}

fn empty_outcome(
    fetched_at: i64,
    last_success_at: Option<i64>,
    models_skipped: u64,
    mut rejected: Vec<RejectedPrice>,
    collisions: Vec<Collision>,
    batch_rejected: bool,
) -> RefreshOutcome {
    rejected.sort_by(|left, right| left.model_id.cmp(&right.model_id));
    RefreshOutcome {
        fetched_at,
        last_attempt_at: fetched_at,
        last_success_at,
        models_imported: 0,
        models_skipped,
        source_url: SOURCE_URL.to_string(),
        rejected,
        pending: Vec::new(),
        accepted_after_confirm: Vec::new(),
        collisions: collision_reports(collisions),
        batch_rejected,
    }
}

fn collision_reports(collisions: Vec<Collision>) -> Vec<CollisionReport> {
    collisions
        .into_iter()
        .map(|collision| CollisionReport {
            model_id: collision.model_id,
            published_ids: collision.published_ids,
            chosen: collision.chosen,
        })
        .collect()
}

fn warn_halved_providers(db: &Database, models: &[CatalogModel]) -> Result<(), AppError> {
    let conn = lock_conn!(db.conn);
    let mut statement = conn.prepare("SELECT model_id FROM model_pricing")?;
    let stored_ids = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let mut stored_counts: BTreeMap<String, u64> = BTreeMap::new();
    for model_id in stored_ids {
        if let Some(provider) = accepted_provider(&model_id) {
            *stored_counts.entry(provider.to_string()).or_default() += 1;
        }
    }
    let mut incoming_counts: BTreeMap<String, u64> = BTreeMap::new();
    for model in models {
        *incoming_counts
            .entry(model.provider_id.clone())
            .or_default() += 1;
    }
    let stored: Vec<(String, u64)> = stored_counts.into_iter().collect();
    let incoming: Vec<(String, u64)> = incoming_counts.into_iter().collect();
    for provider in halved_providers(&stored, &incoming) {
        log::warn!(
            "official pricing provider {provider} dropped below half of its stored catalog; importing anyway"
        );
    }
    Ok(())
}

fn apply_model(
    transaction: &rusqlite::Transaction<'_>,
    model: &CatalogModel,
    fetched_at: i64,
    rejected: &mut Vec<RejectedPrice>,
    pending: &mut Vec<PendingPrice>,
    accepted_after_confirm: &mut Vec<String>,
) -> Result<(), AppError> {
    let stored = load_stored(transaction, &model.model_id)?;
    let decision = decide_price(
        stored.as_ref().map(|row| row.input),
        stored.as_ref().map(|row| row.output),
        stored.as_ref().and_then(|row| row.pending.as_ref()),
        model.input,
        model.output,
        fetched_at,
    );
    match decision {
        PriceDecision::Accept { confirmed } => {
            write_accepted(transaction, model)?;
            if stored.is_none() && (model.input.is_zero() || model.output.is_zero()) {
                log::warn!(
                    "official pricing first import has a zero price: model={}",
                    model.model_id
                );
            }
            if confirmed {
                accepted_after_confirm.push(model.model_id.clone());
            }
        }
        PriceDecision::Pending { reason, reset } => {
            let (first_seen_at, seen_count) = if reset {
                (fetched_at, 1)
            } else {
                let previous = stored.as_ref().and_then(|row| row.pending.as_ref());
                (
                    previous
                        .map(|quote| quote.first_seen_at)
                        .unwrap_or(fetched_at),
                    previous
                        .map(|quote| quote.seen_count.saturating_add(1))
                        .unwrap_or(1),
                )
            };
            write_pending(
                transaction,
                &model.model_id,
                model,
                first_seen_at,
                seen_count,
            )?;
            rejected.push(RejectedPrice {
                model_id: model.model_id.clone(),
                reason: reason.to_string(),
            });
            pending.push(PendingPrice {
                model_id: model.model_id.clone(),
                first_seen_at,
            });
        }
    }
    Ok(())
}

struct StoredRow {
    input: Decimal,
    output: Decimal,
    pending: Option<PendingQuote>,
}

fn load_stored(
    transaction: &rusqlite::Transaction<'_>,
    model_id: &str,
) -> Result<Option<StoredRow>, AppError> {
    let row = transaction
        .query_row(
            "SELECT input_cost_per_million, output_cost_per_million,
                    pending_json, pending_first_seen_at, pending_seen_count
             FROM model_pricing WHERE model_id = ?1",
            [model_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((input, output, pending_json, first_seen, seen_count)) = row else {
        return Ok(None);
    };
    let input = Decimal::from_str(&input).map_err(|error| {
        AppError::Database(format!(
            "invalid stored input price for {model_id}: {error}"
        ))
    })?;
    let output = Decimal::from_str(&output).map_err(|error| {
        AppError::Database(format!(
            "invalid stored output price for {model_id}: {error}"
        ))
    })?;
    let pending = pending_json.and_then(|json| {
        let body: PendingBody = serde_json::from_str(&json).ok()?;
        Some(PendingQuote {
            input: Decimal::from_str(&body.input).ok()?,
            output: Decimal::from_str(&body.output).ok()?,
            first_seen_at: first_seen?,
            seen_count: seen_count.unwrap_or(1),
        })
    });
    Ok(Some(StoredRow {
        input,
        output,
        pending,
    }))
}

fn write_accepted(
    transaction: &rusqlite::Transaction<'_>,
    model: &CatalogModel,
) -> Result<(), AppError> {
    transaction.execute(
        "INSERT INTO model_pricing (
            model_id, display_name, input_cost_per_million, output_cost_per_million,
            cache_read_cost_per_million, cache_creation_cost_per_million,
            pending_json, pending_first_seen_at, pending_seen_count
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, NULL)
         ON CONFLICT(model_id) DO UPDATE SET
            display_name = excluded.display_name,
            input_cost_per_million = excluded.input_cost_per_million,
            output_cost_per_million = excluded.output_cost_per_million,
            cache_read_cost_per_million = excluded.cache_read_cost_per_million,
            cache_creation_cost_per_million = excluded.cache_creation_cost_per_million,
            pending_json = NULL,
            pending_first_seen_at = NULL,
            pending_seen_count = NULL",
        params![
            model.model_id,
            model.display_name,
            model.input.normalize().to_string(),
            model.output.normalize().to_string(),
            model.cache_read,
            model.cache_write,
        ],
    )?;
    Ok(())
}

fn write_pending(
    transaction: &rusqlite::Transaction<'_>,
    model_id: &str,
    model: &CatalogModel,
    first_seen_at: i64,
    seen_count: i64,
) -> Result<(), AppError> {
    let body = serde_json::to_string(&PendingBody {
        input: model.input.normalize().to_string(),
        output: model.output.normalize().to_string(),
    })
    .map_err(|error| AppError::Database(format!("pending price encode failed: {error}")))?;
    transaction.execute(
        "UPDATE model_pricing
         SET pending_json = ?1, pending_first_seen_at = ?2, pending_seen_count = ?3
         WHERE model_id = ?4",
        params![body, first_seen_at, seen_count, model_id],
    )?;
    Ok(())
}

fn filtered_models(
    document: HashMap<String, ProviderDocument>,
) -> (Vec<CatalogModel>, Vec<RejectedPrice>, u64) {
    let mut imported = Vec::new();
    let mut rejected = Vec::new();
    let mut skipped = 0;

    for (provider_id, provider) in document {
        for (map_model_id, model) in provider.models {
            let published_id = model
                .id
                .as_deref()
                .unwrap_or(map_model_id.as_str())
                .to_string();
            let model_id = clean_model_id_for_pricing(&published_id);
            if model_id.is_empty() || accepted_provider(&model_id) != Some(provider_id.as_str()) {
                skipped += 1;
                continue;
            }

            let Some((input, output, cache_read, cache_write)) = parse_cost(model.cost.as_ref())
            else {
                skipped += 1;
                rejected.push(RejectedPrice {
                    model_id,
                    reason: REASON_INVALID_VALUE.to_string(),
                });
                continue;
            };

            imported.push(CatalogModel {
                display_name: model
                    .name
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| published_id.clone()),
                published_id,
                provider_id: provider_id.clone(),
                model_id,
                input,
                output,
                cache_read,
                cache_write,
            });
        }
    }

    (imported, rejected, skipped)
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

fn parse_cost(cost: Option<&Value>) -> Option<(Decimal, Decimal, Option<String>, Option<String>)> {
    let object = cost?.as_object()?;
    let input = decimal_value(object.get("input")?)?;
    let output = decimal_value(object.get("output")?)?;
    let cache_read = match object.get("cache_read") {
        None => None,
        Some(value) => Some(decimal_value(value)?.normalize().to_string()),
    };
    let cache_write = match object.get("cache_write") {
        None => None,
        Some(value) => Some(decimal_value(value)?.normalize().to_string()),
    };
    Some((input, output, cache_read, cache_write))
}

fn decimal_value(value: &Value) -> Option<Decimal> {
    let raw = match value {
        Value::Number(number) => number.to_string(),
        _ => return None,
    };
    let decimal = Decimal::from_str(&raw).ok()?;
    (decimal >= Decimal::ZERO).then_some(decimal)
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
    fn first_zero_price_warns_once() {
        // Isolate the process-global logger from the credential-redaction tests.
        const CHILD: &str = "LUB_ZERO_PRICE_LOG_TEST";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "providers::shared::official_pricing::tests::first_zero_price_warns_once",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            return;
        }
        struct Logger(std::sync::Mutex<Vec<String>>);
        impl log::Log for Logger {
            fn enabled(&self, _: &log::Metadata<'_>) -> bool {
                true
            }
            fn log(&self, record: &log::Record<'_>) {
                if record.level() == log::Level::Warn {
                    self.0.lock().unwrap().push(record.args().to_string());
                }
            }
            fn flush(&self) {}
        }
        static LOGGER: Logger = Logger(std::sync::Mutex::new(Vec::new()));
        log::set_logger(&LOGGER).unwrap();
        log::set_max_level(log::LevelFilter::Warn);
        let db = Database::memory().unwrap();
        let fixture = json!({"anthropic": {"models": {
            "claude-free-input": {"cost": {"input": 0, "output": 2}},
            "claude-free-output": {"cost": {"input": 3, "output": 0}},
            "claude-paid": {"cost": {"input": 3, "output": 4}}
        }}});
        import_fixture(&db, fixture.clone(), 100);
        import_fixture(&db, fixture, 200);
        let logs = LOGGER.0.lock().unwrap();
        let warnings: Vec<_> = logs
            .iter()
            .filter(|line| line.contains("first import has a zero price"))
            .collect();
        assert_eq!(warnings.len(), 2);
        assert!(warnings
            .iter()
            .any(|line| line.contains("claude-free-input")));
        assert!(warnings
            .iter()
            .any(|line| line.contains("claude-free-output")));
        assert!(warnings.iter().all(|line| !line.contains("claude-paid")));
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
    fn absent_cache_fields_are_stored_as_null_and_explicit_zero_stays_zero() {
        let db = Database::memory().unwrap();
        import_fixture(
            &db,
            json!({"deepseek": {"models": {
                "deepseek-chat": {"cost": {"input": 0.27, "output": 1.1}},
                "deepseek-reasoner": {"cost": {"input": 0.55, "output": 2.19, "cache_read": 0, "cache_write": 0}}
            }}}),
            101,
        );

        let conn = db.conn.lock().unwrap();
        let missing = conn
            .query_row(
                "SELECT cache_read_cost_per_million, cache_creation_cost_per_million FROM model_pricing WHERE model_id = 'deepseek-chat'",
                [],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .unwrap();
        assert_eq!(missing, (None, None));
        let explicit = conn
            .query_row(
                "SELECT cache_read_cost_per_million, cache_creation_cost_per_million FROM model_pricing WHERE model_id = 'deepseek-reasoner'",
                [],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .unwrap();
        assert_eq!(explicit, (Some("0".to_string()), Some("0".to_string())));
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

    fn price_of(db: &Database, model_id: &str) -> (String, String, Option<String>) {
        let conn = db.conn.lock().unwrap();
        conn.query_row(
            "SELECT input_cost_per_million, output_cost_per_million, pending_json
             FROM model_pricing WHERE model_id = ?1",
            [model_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
    }

    #[test]
    fn five_times_and_one_fifth_write_through_and_a_larger_jump_stays_pending() {
        let db = Database::memory().unwrap();
        import_fixture(
            &db,
            json!({"anthropic": {"models": {
                "claude-guard": {"cost": {"input": 2, "output": 10, "cache_read": 1}}
            }}}),
            1_000,
        );

        let five_times = import_fixture(
            &db,
            json!({"anthropic": {"models": {
                "claude-guard": {"cost": {"input": 10, "output": 10, "cache_read": 1}}
            }}}),
            1_100,
        );
        assert!(five_times.rejected.is_empty());
        assert_eq!(price_of(&db, "claude-guard").0, "10");

        import_fixture(
            &db,
            json!({"anthropic": {"models": {
                "claude-guard": {"cost": {"input": 10, "output": 2, "cache_read": 1}}
            }}}),
            1_200,
        );
        assert_eq!(
            price_of(&db, "claude-guard").1,
            "2",
            "1/5 output is written"
        );

        let blocked = import_fixture(
            &db,
            json!({"anthropic": {"models": {
                "claude-guard": {"cost": {"input": 10, "output": 21, "cache_read": 1000}}
            }}}),
            1_300,
        );
        assert_eq!(blocked.rejected[0].reason, "change_too_large");
        assert_eq!(blocked.pending[0].model_id, "claude-guard");
        let (input, output, pending) = price_of(&db, "claude-guard");
        assert_eq!((input, output), ("10".to_string(), "2".to_string()));
        assert!(
            pending.is_some(),
            "the live price stays put while cache also jumped"
        );

        let too_soon = import_fixture(
            &db,
            json!({"anthropic": {"models": {
                "claude-guard": {"cost": {"input": 10, "output": 21, "cache_read": 1000}}
            }}}),
            1_300 + crate::providers::shared::pricing_guard::CONFIRM_INTERVAL_SECS - 1,
        );
        assert!(too_soon.accepted_after_confirm.is_empty());
        assert_eq!(price_of(&db, "claude-guard").1, "2");

        let confirmed = import_fixture(
            &db,
            json!({"anthropic": {"models": {
                "claude-guard": {"cost": {"input": 10, "output": 21, "cache_read": 1000}}
            }}}),
            1_300 + crate::providers::shared::pricing_guard::CONFIRM_INTERVAL_SECS,
        );
        assert_eq!(
            confirmed.accepted_after_confirm,
            vec!["claude-guard".to_string()]
        );
        assert!(confirmed.pending.is_empty());
        let (input, output, pending) = price_of(&db, "claude-guard");
        assert_eq!((input, output), ("10".to_string(), "21".to_string()));
        assert_eq!(pending, None);
        let conn = db.conn.lock().unwrap();
        let cache: Option<String> = conn
            .query_row(
                "SELECT cache_read_cost_per_million FROM model_pricing WHERE model_id = 'claude-guard'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cache.as_deref(), Some("1000"));
    }

    #[test]
    fn a_drop_to_zero_waits_and_a_rise_from_zero_does_not() {
        let db = Database::memory().unwrap();
        import_fixture(
            &db,
            json!({"openai": {"models": {
                "gpt-guard": {"cost": {"input": 4, "output": 8}}
            }}}),
            2_000,
        );
        let blocked = import_fixture(
            &db,
            json!({"openai": {"models": {
                "gpt-guard": {"cost": {"input": 0, "output": 8}}
            }}}),
            2_100,
        );
        assert_eq!(blocked.rejected[0].reason, "zero_price");
        assert_eq!(price_of(&db, "gpt-guard").0, "4");

        let confirmed = import_fixture(
            &db,
            json!({"openai": {"models": {
                "gpt-guard": {"cost": {"input": 0, "output": 8}}
            }}}),
            2_100 + crate::providers::shared::pricing_guard::CONFIRM_INTERVAL_SECS,
        );
        assert_eq!(
            confirmed.accepted_after_confirm,
            vec!["gpt-guard".to_string()]
        );
        assert_eq!(price_of(&db, "gpt-guard").0, "0");

        let risen = import_fixture(
            &db,
            json!({"openai": {"models": {
                "gpt-guard": {"cost": {"input": 4, "output": 8}}
            }}}),
            90_000,
        );
        assert!(risen.rejected.is_empty());
        assert_eq!(price_of(&db, "gpt-guard").0, "4");
    }

    #[test]
    fn a_catalog_below_half_the_last_import_writes_nothing() {
        let db = Database::memory().unwrap();
        import_fixture(
            &db,
            json!({"openai": {"models": {
                "gpt-a": {"cost": {"input": 1, "output": 2}},
                "gpt-b": {"cost": {"input": 1, "output": 2}},
                "gpt-c": {"cost": {"input": 1, "output": 2}},
                "gpt-d": {"cost": {"input": 1, "output": 2}}
            }}}),
            3_000,
        );
        let rejected = import_fixture(
            &db,
            json!({"openai": {"models": {
                "gpt-a": {"cost": {"input": 100, "output": 2}}
            }}}),
            3_100,
        );
        assert!(rejected.batch_rejected);
        assert_eq!(rejected.models_imported, 0);
        assert_eq!(price_of(&db, "gpt-a").0, "1");
        assert_eq!(last_imported_count(&db).unwrap(), Some(4));
        assert_eq!(last_refresh_at(&db).unwrap(), Some(3_000));
        assert_eq!(
            db.get_setting(LAST_ATTEMPT_SETTING_KEY).unwrap().as_deref(),
            Some("3100")
        );
        assert_eq!(rejected.last_attempt_at, 3_100);
        assert_eq!(rejected.last_success_at, Some(3_000));

        let exact_half = import_fixture(
            &db,
            json!({"openai": {"models": {
                "gpt-a": {"cost": {"input": 2, "output": 2}},
                "gpt-b": {"cost": {"input": 2, "output": 2}}
            }}}),
            3_200,
        );
        assert!(!exact_half.batch_rejected);
        assert_eq!(price_of(&db, "gpt-a").0, "2");
        assert_eq!(last_refresh_at(&db).unwrap(), Some(3_200));
        assert_eq!(exact_half.last_success_at, Some(3_200));
    }

    #[test]
    fn shuffled_colliding_ids_import_the_same_row_ten_times() {
        let fixture = json!({"anthropic": {"models": {
            "aaa/claude-guard-collide": {
                "id": "aaa/claude-guard-collide",
                "cost": {"input": 99, "output": 99}
            },
            "exact": {
                "id": "claude-guard-collide",
                "cost": {"input": 3, "output": 15}
            }
        }}});
        for index in 0..10 {
            let db = Database::memory().unwrap();
            let outcome = import_fixture(&db, fixture.clone(), 4_000 + index);
            assert_eq!(outcome.models_imported, 1);
            assert_eq!(outcome.collisions.len(), 1);
            assert_eq!(outcome.collisions[0].chosen, "claude-guard-collide");
            assert_eq!(price_of(&db, "claude-guard-collide").0, "3");
        }
    }

    #[test]
    fn one_provider_falling_below_half_still_imports() {
        let db = Database::memory().unwrap();
        import_fixture(
            &db,
            json!({"xai": {"models": {
                "grok-a": {"cost": {"input": 1, "output": 2}},
                "grok-b": {"cost": {"input": 1, "output": 2}},
                "grok-c": {"cost": {"input": 1, "output": 2}},
                "grok-d": {"cost": {"input": 1, "output": 2}}
            }}}),
            5_000,
        );
        let outcome = import_fixture(
            &db,
            json!({"openai": {"models": {
                "gpt-keep": {"cost": {"input": 1, "output": 2}}
            }}, "xai": {"models": {
                "grok-a": {"cost": {"input": 5, "output": 2}}
            }}}),
            5_100,
        );
        assert!(
            !outcome.batch_rejected,
            "two models are half of the last four, so the batch is kept"
        );
        assert_eq!(price_of(&db, "grok-a").0, "5");
        assert_eq!(price_of(&db, "gpt-keep").0, "1");
    }
}
