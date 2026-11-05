use insights::daily_summary;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    daily_summary::initialize().await?;
    eprintln!("Insights database maintenance service started");
    daily_summary::run_maintenance_service().await
}
