#[path = "../../insights/src/observability/latency.rs"]
mod latency;
mod live_signals;
#[path = "../../insights/src/observability/logging.rs"]
mod logging;
#[path = "../../service_docs.rs"]
mod service_docs;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    live_signals::run().await
}


///
/// Live Signals Module
/// 
/// The purpose of this module is to convert raw feedhandler outputs into learning signals. 
/// It should be taking fast moving feedhandler data and converting it into meaningful trade signals. 
/// 
/// For instance: Take raw book updates and convert to a signal for bid/ask mispricing. 
/// 
/// 
/// 
/// TODO - check that this is really what this module is adding .... does it really add value or is it better to skip the extra hop and do this directly in scouts/autotrader