#[path = "../../service_docs.rs"]
mod service_docs;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let address = std::env::var("SCOUTS_LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:3011".into());
    let learning_url =
        std::env::var("LEARNING_SERVICE_URL").unwrap_or_else(|_| "http://127.0.0.1:3010".into());
    let auto_trader_url =
        std::env::var("AUTO_TRADER_URL").unwrap_or_else(|_| "http://127.0.0.1:3005".into());
    let listener = tokio::net::TcpListener::bind(&address).await?;
    println!("Scouts service listening on http://{address}");
    axum::serve(
        listener,
        scouts::router(learning_url, auto_trader_url).merge(service_docs::routes(
            "Scouts",
            &[
                ("/health", &["get"]),
                ("/process-cpu-seconds", &["get"]),
                ("/process-memory-bytes", &["get"]),
                ("/configuration", &["get", "post"]),
                ("/status", &["get"]),
                ("/outcomes", &["get", "post"]),
                ("/samples", &["post"]),
                ("/scout-outcome", &["post"]),
                ("/scout-markout", &["post"]),
                ("/scout-plan", &["post"]),
                ("/daily-inputs", &["post"]),
                ("/live-inputs", &["post"]),
                ("/dated-carry-inputs", &["post"]),
                ("/scout-lifecycle/state", &["get", "post"]),
                ("/scout-lifecycle/observe", &["post"]),
                ("/variants", &["post"]),
                ("/variants/batch", &["post"]),
                ("/live-candidates", &["post"]),
                ("/select-entry", &["post"]),
                ("/strategy-walk-forward", &["post"]),
            ],
        )),
    )
    .await?;
    Ok(())
}

/// Scouts Module. 
/// 
/// Is a module dedicated to running many strategies off of the hot path in order to gather learning information for the learning module. 
/// 
/// Each scout should be a strategy run with a set of parameters provided by the learning module and run to completion with its outcomes feeding back to the learning module. 
/// 
/// TODO: Not implemented yet but scout containers should be isolated per strategy and/or symbol and be able to run completely independantly. Comms should be SBE over Websocket (again not implemented yet)


/// TODO adjust the architecture so that learning determines which scouts are created. 
/// Scouts should be simply a parameterised version of a strategy run off of the hot path.
/// Its outcome will feedback into the learning for use in adjusting the baseline parameters used in next scout iteration and/or live trading