use ::auto_trader::auto_trader;
mod browser_rsocket;
#[path = "../../service_docs.rs"]
mod service_docs;

use axum::{Router, routing::get};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let domain_router = auto_trader::router();
    let rsocket_router = domain_router.clone();
    let app = http_router();
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
        browser_rsocket::serve(rsocket_router, address),
    )?;
    Ok(())
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
