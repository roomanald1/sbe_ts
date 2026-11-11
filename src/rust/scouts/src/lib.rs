//! Scout scout planning, markout feedback, and lifecycle coordination.

mod scout_worker;
mod scouts;
pub use scout_worker::{ScoutWorkerConfig, serve_strategy_worker};
pub use scouts::*;
