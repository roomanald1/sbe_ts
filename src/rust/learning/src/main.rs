#[path = "../../service_docs.rs"]
mod service_docs;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let address = std::env::var("LEARNING_LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:3010".into());
    let path = std::env::var("LEARNING_STATE_FILE")
        .map(Into::into)
        .unwrap_or_else(|_| "learning_state.sqlite".into());
    let listener = tokio::net::TcpListener::bind(&address).await?;
    println!("Learning service listening on http://{address}");
    let app = learning::router(path)
        .map_err(anyhow::Error::msg)?
        .merge(service_docs::routes(
            "Learning",
            &[
                ("/health", &["get"]),
                ("/process-cpu-seconds", &["get"]),
                ("/process-memory-bytes", &["get"]),
                ("/profiles", &["get", "post"]),
                ("/decision-snapshot", &["get"]),
                ("/summary", &["get"]),
                ("/feedback", &["post"]),
                ("/config/scout-parameter-samples", &["put"]),
                ("/context-thresholds", &["post"]),
                ("/execution-decision", &["post"]),
                ("/execution-outcomes", &["post"]),
            ],
        ));
    axum::serve(listener, app).await?;
    Ok(())
}

//TODO convert the comment below to a README

///
/// Learning Module
/// 
/// Takes outcomes from scouts and live trades and use that to inform future trade decisions. 
/// 
/// Live Trades: 
/// Live trades will report their instantiation AND outcomes to this module for learning including the net profit/loss, time in market etc. On a live trade starting, learning should instantiate variant scouts with differing exit strategies (linked by ID) so that we 
/// can find out whether we missed opportunities using the baseline parameters.
///   
/// Scouting:
/// Each strategy + instrumentId will have its own profile. Each profile will have variants. Its the responsibility of this module to request a new scout instance for these variants cycling around in order to retrieve the best, most complete set of outcomes. 
/// The scouting module will run and report back their outcomes. These outcomes are then collected and used to recalculate the main profiles parameters. The scouting will run the strategy as if it were live alongside its baseline.
/// 
/// For instance @ time t0 it will kick off two scouts:
/// 
/// Scout1:  BaseLine = (Strategy A - BTC-USDT) - [Parameter 1 = 5, Parameter 2 = 7] => outcome = +0.2%
/// Scout2:  Variant1 = (Strategy A - BTC-USDT) - [Parameter 1 = 6, Parameter 2 = 7] => outcome = +0.3%
/// 
/// In this instance the variant performed better than the baseline. After repeated instances of this type of outcome, the better performing parameter will be promoted to the baseline. 
///
/// (New) BaseLine = (Strategy A - BTC-USDT) - [Parameter 1 = 6, Parameter 2 = 7] 
/// 
/// The Autotrader will use these parameter profiles when building live trades. 