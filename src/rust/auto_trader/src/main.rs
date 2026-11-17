use ::auto_trader::auto_trader;
mod browser_rsocket;
#[path = "../../service_docs.rs"]
mod service_docs;

use axum::{Router, routing::get};
use futures_util::future::join_all;
use std::{collections::HashMap, time::Duration};
use strategy::Strategy;
use tokio::time::{Instant, interval};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (domain_router, socket_api) = auto_trader::router_with_socket();
    let rsocket_router = domain_router.clone();
    let app = http_router();
    tokio::spawn(publish_live_profiles(socket_api.clone()));
    let port = std::env::var("AUTO_TRADER_RSOCKET_PORT").unwrap_or_else(|_| "10006".to_owned());
    let address = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", 3005)).await?;
    ::auto_trader::service_log!(
        "INFO",
        "AutoTrader service listening on http://0.0.0.0:3005"
    );
    tokio::try_join!(
        async {
            axum::serve(listener, app)
                .await
                .map_err(anyhow::Error::from)
        },
        browser_rsocket::serve(rsocket_router, address, socket_api),
    )?;
    Ok(())
}

async fn publish_live_profiles(socket_api: auto_trader::AutoTraderSocketApi) {
    let mut profile_updates = socket_api.subscribe_live_profile_updates();
    let mut retry = interval(Duration::from_secs(15));
    retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_published = HashMap::<String, (u64, Instant)>::new();

    loop {
        tokio::select! {
            _ = retry.tick() => {}
            result = profile_updates.recv() => {
                match result {
                    Ok(()) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        while profile_updates.try_recv().is_ok() {}
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
        }

        let profile_sets = socket_api.live_profile_sets().await;
        let pushes = profile_sets.into_iter().filter_map(|profile_set| {
            let key = profile_set.strategy.as_str().to_owned();
            if last_published.get(&key).is_some_and(|(version, sent_at)| {
                *version == profile_set.set_version && sent_at.elapsed() < Duration::from_secs(15)
            }) {
                return None;
            }
            let endpoint = live_worker_endpoint(profile_set.strategy);
            Some(async move {
                let result =
                    ::auto_trader::service_rpc::push_live_profile_set(&endpoint, &profile_set)
                        .await;
                (key, profile_set.set_version, endpoint, result)
            })
        });

        for (key, version, endpoint, result) in join_all(pushes).await {
            match result {
                Ok(()) => {
                    last_published.insert(key, (version, Instant::now()));
                }
                Err(error) => {
                    ::auto_trader::service_log!(
                        "WARN",
                        "Could not push live profile set to {endpoint}: {error:#}"
                    );
                }
            }
        }
    }
}

fn live_worker_endpoint(strategy: Strategy) -> String {
    let service_name = if strategy.is_live_insight() {
        format!("strategy-{}", strategy.as_str())
    } else {
        format!("strategy-live-{}", strategy.as_str())
    };
    format!("ws://{service_name}:10201")
}

fn http_router() -> Router {
    service_docs::routes(
        "AutoTrader",
        &[
            ("/health", &["get"]),
            ("/process-cpu-seconds", &["get"]),
            ("/process-memory-bytes", &["get"]),
        ],
    )
    .route("/health", get(|| async { "ok" }))
    .route("/process-cpu-seconds", get(process_cpu_seconds))
    .route("/process-memory-bytes", get(process_memory_bytes))
}

async fn process_memory_bytes() -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix("VmRSS:")?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()
            })
        })
        .map(|kilobytes| (kilobytes * 1024).to_string())
        .unwrap_or_else(|| "0".to_owned())
}

async fn process_cpu_seconds() -> String {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let fields = stat
        .rsplit_once(')')
        .map(|(_, fields)| fields)
        .unwrap_or("");
    let values: Vec<u64> = fields
        .split_whitespace()
        .filter_map(|value| value.parse().ok())
        .collect();
    values
        .get(11)
        .zip(values.get(12))
        .map(|(user, system)| (*user + *system) as f64 / 100.0)
        .unwrap_or(0.0)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn external_http_listener_exposes_only_ops_not_domain_routes() {
        let app = http_router();
        let health = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(health.status(), StatusCode::OK);

        let domain = app
            .oneshot(
                Request::builder()
                    .uri("/positions/auto-trader")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(domain.status(), StatusCode::NOT_FOUND);
    }
}

// AutoTrader evaluates signals with strategy profile parameters and delegates
// order placement and position state to their owning services. Scout
// walk-forward evidence is requested from Learning over RSocket.
