//! Versioned database transformations. The schema dispatcher owns their order;
//! feature modules must not own migrations of persisted data.

pub(crate) mod agent_module_migration;
pub mod budget_migration;
pub(crate) mod cost_backfill_migration;
pub(crate) mod cursor_migration;
pub(crate) mod migration;
pub(crate) mod module_migration;
pub(crate) mod provider_pricing_migration;
pub(crate) mod quota_retry_migration;
pub(crate) mod system_provider_migration;
pub(crate) mod usage_light_prediction_migration;
