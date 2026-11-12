#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let role = std::env::var("STRATEGY_X_WORKER_ROLE")
        .map_err(|_| anyhow::anyhow!("STRATEGY_X_WORKER_ROLE is required (live or scout)"))?;
    let strategy_name = std::env::var("STRATEGY_X_WORKER_STRATEGY").map_err(|_| {
        anyhow::anyhow!("STRATEGY_X_WORKER_STRATEGY is required for worker role {role}")
    })?;
    let strategy = strategy::Strategy::from_name(&strategy_name)
        .ok_or_else(|| anyhow::anyhow!("unknown STRATEGY_X_WORKER_STRATEGY {strategy_name}"))?;
    match role.as_str() {
        "live" => {
            strategy::serve_live_worker(strategy::LiveWorkerConfig::from_env(strategy)?).await
        }
        "scout" => {
            strategy::serve_strategy_worker(strategy::ScoutWorkerConfig::from_env(strategy)?).await
        }
        _ => Err(anyhow::anyhow!(
            "STRATEGY_X_WORKER_ROLE must be live or scout, received {role}"
        )),
    }
}

// Learning owns selection and scheduling. Each process must be configured as
// exactly one live or Scout worker for exactly one registered strategy.
