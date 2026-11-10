use ::auto_trader::auto_trader;
#[path = "../../service_docs.rs"]
mod service_docs;

use axum::routing::get;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let app = auto_trader::router()
        .merge(service_docs::routes(
            "AutoTrader",
            &[
                ("/health", &["get"]),
                ("/positions/auto-trader", &["get", "put"]),
                ("/positions/auto-trader/settings", &["put"]),
                ("/positions/auto-trader/dated-carry", &["get", "put"]),
                ("/positions/auto-trader/parameters", &["get"]),
                ("/positions/auto-trader/activity", &["get"]),
                ("/positions/auto-trader/learning-summary", &["get"]),
            ],
        ))
        .route("/health", get(|| async { "ok" }))
        .route("/process-cpu-seconds", get(process_cpu_seconds))
        .route("/process-memory-bytes", get(process_memory_bytes));
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", 3005)).await?;
    ::auto_trader::service_log!(
        "INFO",
        "AutoTrader service listening on http://0.0.0.0:3005"
    );
    axum::serve(listener, app).await?;
    Ok(())
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


///
/// Autotrader module 
/// 
/// 
/// This module is responsible for taking learning outcomes (parameter profiles) and live signals in order to execute trade strategies. 
/// 
/// Learning profiles and live signals are communicated via SBE messages over websockets (RSocket) - TODO 
/// 
/// Each live signal will evaluate against the strategies evaluate() function. The evaluate function will decide whether to enter, exit or hold the positions. It will also determine the nature of the execution IoC, Limit, VWAP etc based on its learning parameters.
/// 
/// Actual execution is done via the Execution module. Also this module does not hold any ledger, it is the responsiblity of the postion manager to hold position statuses. This module is simply a trade creator. 
/// 
/// All trades created via this module will communicate via SBE over RSocket to the learning module to inform it of the creation and exit of any trades. This will help inform the future trade runs. 
/// 

