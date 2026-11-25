//! Embeddable Position Management service API.
//!
//! The binary and integration fixtures both use the same route and persistence
//! modules so service behavior is exercised without a parallel implementation.

#[path = "feeds/feed_client.rs"]
mod feed_client;
#[path = "observability/logging.rs"]
pub mod logging;
pub mod positions;
#[path = "users/users.rs"]
mod users;

/// Build the Position Management HTTP router and start its configured RSocket
/// responder, using the existing `POSITIONS_FILE` and port environment settings.
pub async fn router() -> anyhow::Result<axum::Router> {
    positions::router().await
}

/// Expose the service's current internal queue metrics for the binary health API.
pub fn queue_metrics() -> Vec<serde_json::Value> {
    positions::queue_metrics()
}
