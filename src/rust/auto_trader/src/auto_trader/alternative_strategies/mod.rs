//! Independent paper accounts for instruments unsupported by the spot ledger.
//! No authenticated exchange or blockchain transaction submission exists here.
mod api;
mod engine;
pub mod replay;
#[cfg(test)]
mod tests;
mod types;
pub(super) use api::router;
