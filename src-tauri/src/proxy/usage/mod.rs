//! Proxy request logging.
//!
//! Token accounting and cost arithmetic moved to `crate::usage::metering`;
//! only the proxy-specific request logger remains here.

pub mod logger;

#[allow(unused_imports)]
pub use logger::{RequestLog, UsageLogger};
