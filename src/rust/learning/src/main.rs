#[path = "../../service_docs.rs"]
mod service_docs;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let address = std::env::var("LEARNING_LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:3010".into());
    let rsocket_address =
        std::env::var("LEARNING_RSOCKET_LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:10010".into());
    let worker_endpoints = std::env::var("SCOUT_WORKER_ENDPOINTS").unwrap_or_default();
    let path = std::env::var("LEARNING_STATE_FILE")
        .map(Into::into)
        .unwrap_or_else(|_| "learning_state.sqlite".into());
    let listener = tokio::net::TcpListener::bind(&address).await?;
    println!("Learning service listening on http://{address}");
    let (_domain_router, coordinator) =
        learning::router_with_scout_coordinator(path, &worker_endpoints)
            .map_err(anyhow::Error::msg)?;
    let app = learning::operational_router().merge(service_docs::routes(
        "Learning",
        &[
            ("/health", &["get"]),
            ("/process-cpu-seconds", &["get"]),
            ("/process-memory-bytes", &["get"]),
        ],
    ));
    println!("Learning Scout coordinator serving RSocket on ws://{rsocket_address}");
    tokio::try_join!(
        async move {
            axum::serve(listener, app).await?;
            anyhow::Ok(())
        },
        coordinator.serve(rsocket_address),
    )?;
    Ok(())
}

#[cfg(test)]
mod operational_http_tests {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[tokio::test]
    async fn learning_http_surface_exposes_operations_but_not_domain_routes() {
        let app = learning::operational_router();
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
        assert_eq!(health.status(), axum::http::StatusCode::OK);

        let profiles = app
            .oneshot(
                Request::builder()
                    .uri("/profiles")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(profiles.status(), axum::http::StatusCode::NOT_FOUND);
    }
}
