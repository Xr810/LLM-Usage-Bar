// Compatibility exports; storage owns migrations, queries and persistence.
pub use crate::metering;
pub use crate::store::migrations::budget_migration;
pub(crate) use crate::store::usage_light_prediction;
pub use crate::store::{sql_helpers, usage_stats};
pub mod aggregation;
pub(crate) mod budget_alert;
pub(crate) mod claude_oauth;
pub(crate) mod cli_probe;
pub mod dashboard;
pub mod ingestion;
pub(crate) mod rhythm;
pub mod session;
pub mod status;
pub(crate) mod subscription_pace;
// Compatibility path for feature consumers; the catalog is owned by model.
pub(crate) use crate::model::system_providers;
pub mod tray_snapshot;
pub mod tray_usage;
pub mod tray_usage_scheduler;
pub mod usage_cache;
pub(crate) mod watcher;
// watcher_state:生产路径只使用 begin_due/finish/mark_* 等子集;其余 getter 与
// set_interval 由模块内单元测试覆盖,非测试构建下允许未被引用(dead_code)。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod watcher_state;
