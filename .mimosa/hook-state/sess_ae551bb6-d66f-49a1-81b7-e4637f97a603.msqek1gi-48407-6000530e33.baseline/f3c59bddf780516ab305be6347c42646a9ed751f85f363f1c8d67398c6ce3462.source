//! Token accounting and cost arithmetic.
//!
//! These are the primitives every usage source shares: the token-usage shape
//! parsed out of a session log or an upstream response, the per-million-token
//! cost arithmetic, and the parser for costs an upstream reports directly.
//! They are independent of how the usage was observed.

pub mod calculator;
pub mod cost_parser;
pub mod parser;

pub use calculator::{CostBreakdown, CostCalculator, ModelPricing};
pub use parser::{ApiType, TokenUsage};
