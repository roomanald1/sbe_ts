//! Provider data enters only through FeedHandlers; this module produces
//! point-in-time strategy insights and publishes them to AutoTrader.
use crate::{InsightDailyBar, SignalStore, now_ms};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::Mutex;
mod aave;
mod chain;
mod factors;
mod liquidity;
mod options;
mod provider;
use provider::*;
const DAY: u64 = 86_400_000;
const IDS: [&str; 5] = [
    "residual-momentum",
    "filtered-covered-call",
    "staking-redemption-arbitrage",
    "liquidation-keeper",
    "adaptive-stablecoin-liquidity",
];
#[derive(Clone, Serialize, Deserialize, Default)]
struct Insight {
    strategy: String,
    ready: bool,
    signal: String,
    source_timestamp_ms: u64,
    computed_at_ms: u64,
    summary: String,
    features: Value,
}
struct Output {
    insight: Insight,
    observations: Vec<Value>,
}
impl Output {
    fn ready(
        id: &str,
        time: u64,
        signal: &str,
        summary: String,
        features: Value,
        observations: Vec<Value>,
    ) -> Self {
        Self {
            insight: Insight {
                strategy: id.into(),
                ready: true,
                signal: signal.into(),
                source_timestamp_ms: time,
                computed_at_ms: now_ms(),
                summary,
                features,
            },
            observations,
        }
    }
    fn unavailable(id: &str, error: impl std::fmt::Display) -> Self {
        Self {
            insight: Insight {
                strategy: id.into(),
                ready: false,
                signal: "unavailable".into(),
                computed_at_ms: now_ms(),
                summary: error.to_string(),
                ..Insight::default()
            },
            observations: Vec::new(),
        }
    }
}
fn observation(id: &str, source: &str, time: u64, data: Value, unique: &str) -> Value {
    json!({"id":format!("{id}:{unique}"),"observed_at_ms":time,"source":source,"strategy":id,"data":data})
}
#[derive(Serialize, Deserialize, Default)]
struct SourceState {
    aave: aave::AaveIndex,
    lp: Option<liquidity::LpTracker>,
}
fn state_path() -> PathBuf {
    std::env::var_os("ALTERNATIVE_INSIGHTS_STATE_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("SIGNALS_DB_FILE")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("SIGNALS_FILE").map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from("signals.sqlite"))
                .with_extension("alternative-sources.json")
        })
}
async fn save_state(path: &PathBuf, state: &SourceState) -> anyhow::Result<()> {
    let tmp = path.with_extension("alternative-sources.tmp");
    let bytes = serde_json::to_vec(state)?;
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(tmp, path).await?;
    Ok(())
}
async fn residual(feed: &FeedClient, sleeve: &Value) -> anyhow::Result<Output> {
    let pairs = [
        "BTCUSDT", "ETHUSDT", "BNBUSDT", "XRPUSDT", "SOLUSDT", "ADAUSDT", "DOGEUSDT", "LTCUSDT",
    ];
    let candles = feed
        .http
        .get(format!("{}/market-data/daily-candles", feed.base))
        .query(&[("symbols", pairs.join(","))])
        .send()
        .await?
        .error_for_status()?
        .json::<BTreeMap<String, Vec<InsightDailyBar>>>()
        .await?;
    let cap = sleeve["config"]["max_trade_usd"].as_f64().unwrap_or(100.0);
    let candles = &candles;
    let requests=pairs.iter().map(|pair|async move {
        let raw=feed.binance("depth",pair,&[("limit","100".into())]).await?;
        let bars=candles.get(*pair).ok_or_else(||anyhow::anyhow!("{pair} candles missing"))?;
        let time=now_ms();let bars=bars.iter().filter(|b|b.close_time_ms<time).collect::<Vec<_>>();
        anyhow::ensure!(bars.len()>=45,"Need 45 closed daily candles for {pair}");
        let last=bars[bars.len()-1].close_time_ms;
        anyhow::ensure!(time-last<=2*DAY,"{pair} history stale");
        let best=levels(&raw.data,"asks")?.first().ok_or_else(||anyhow::anyhow!("Empty asks"))?.0;
        let held=sleeve["positions"][*pair]["quantity"].as_f64().unwrap_or(0.0).abs();
        let quantity=2.1*cap/best+held;
        Ok::<_,anyhow::Error>(json!({
            "symbol":pair,"closes":bars.iter().skip(bars.len()-45).map(|b|json!({"timestamp_ms":b.close_time_ms,"close":b.close})).collect::<Vec<_>>(),
            "quote":quote(&raw.data,quantity,raw.requested_at_ms)?,"short_available":true
        }))
    });
    let mut assets = Vec::new();
    for result in futures_util::future::join_all(requests).await {
        assets.push(result?);
    }
    let time = now_ms();
    let scores = factors::scores(&assets)?;
    let mut ranking = scores
        .iter()
        .map(|(k, v)| (k.clone(), *v))
        .collect::<Vec<_>>();
    ranking.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let shorts = ranking
        .iter()
        .take(2)
        .map(|(k, _)| k.clone())
        .collect::<Vec<_>>();
    let longs = ranking
        .iter()
        .rev()
        .take(2)
        .map(|(k, _)| k.clone())
        .collect::<Vec<_>>();
    let o = observation(
        IDS[0],
        "FeedHandlers Binance candles/depth; Insights lagged factor inputs; simulated borrow",
        time,
        json!({"assets":assets,"scores":scores}),
        &time.to_string(),
    );
    Ok(Output::ready(
        IDS[0],
        time,
        "weekly-ranking",
        format!(
            "Weekly residual leaders {}; laggards {}",
            longs.join(", "),
            shorts.join(", ")
        ),
        json!({"assets":8,"formation_days":14,"holding_days":7,"borrow":"paper APR","residual_scores":scores,"long_candidates":longs,"short_candidates":shorts}),
        vec![o],
    ))
}
async fn bounded<T>(
    future: impl std::future::Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    tokio::time::timeout(std::time::Duration::from_secs(25), future)
        .await
        .map_err(|_| anyhow::anyhow!("Native feed computation exceeded 25 seconds"))?
}
pub(crate) async fn run(store: SignalStore) {
    let feed = FeedClient::new(
        std::env::var("FEED_HANDLERS_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".into()),
    );
    let auto = std::env::var("AUTO_TRADER_URL").unwrap_or_else(|_| "http://127.0.0.1:3005".into());
    let path = state_path();
    let mut persistent = match tokio::fs::read(&path).await {
        Ok(bytes) => match serde_json::from_slice::<SourceState>(&bytes) {
            Ok(s) => s,
            Err(e) => {
                crate::service_log!(
                    "ERROR",
                    "Alternative source state corrupt; worker stopped without overwrite: {e}"
                );
                return;
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => SourceState::default(),
        Err(e) => {
            crate::service_log!("ERROR", "Alternative source state unreadable: {e}");
            return;
        }
    };
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let result=async {
            let state=feed.http.get(format!("{auto}/positions/auto-trader/alternatives")).send().await?.error_for_status()?.json::<Value>().await?;
            let sleeves=&state["book"]["sleeves"];
            let (r,c,chain)=tokio::join!(bounded(residual(&feed,&sleeves[IDS[0]])),bounded(options::build(&feed,&sleeves[IDS[1]])),bounded(chain::Context::load(&feed)));
            let mut outputs=vec![r.unwrap_or_else(|e|Output::unavailable(IDS[0],e)),c.unwrap_or_else(|e|Output::unavailable(IDS[1],e))];
            match chain {
                Ok(ctx)=>{
                    let (staking,lp,aave)=tokio::join!(
                        bounded(chain::staking(&feed,&ctx,&sleeves[IDS[2]])),
                        bounded(liquidity::build(&feed,&ctx,&sleeves[IDS[4]],&mut persistent.lp)),
                        bounded(aave::build(&feed,&ctx,&sleeves[IDS[3]],&mut persistent.aave))
                    );
                    outputs.push(staking.unwrap_or_else(|e|Output::unavailable(IDS[2],e)));
                    outputs.push(aave.unwrap_or_else(|e|Output::unavailable(IDS[3],e)));
                    outputs.push(lp.unwrap_or_else(|e|Output::unavailable(IDS[4],e)));
                }
                Err(e)=>for id in &IDS[2..] {outputs.push(Output::unavailable(id,&e));},
            }
            // Persist fee baselines/cursors before publishing. A failed publish
            // retains cumulative growth for the next delivery and does not lose fees.
            save_state(&path,&persistent).await?;
            let insights=outputs.iter().map(|o|o.insight.clone()).collect::<Vec<_>>();
            let observations=outputs.into_iter().flat_map(|o|o.observations).collect::<Vec<_>>();
            let time=now_ms();
            let batch=json!({"as_of_ms":time,"insights":insights,"observations":observations});
            let response=feed.http.post(format!("{auto}/internal/auto-trader/alternative-insights")).json(&batch).send().await?;
            anyhow::ensure!(response.status().is_success(),"Insight delivery rejected: {}",response.text().await?);
            let accepted=response.json::<Value>().await?;
            for insight in &insights {
                store.save_auto_trader_audit(json!({
                    "record_schema_version":1,"decision_id":format!("{}:{time}",insight.strategy),"timestamp":time,
                    "stage":"alternative:insight","strategy":insight.strategy,
                    "status":accepted["sleeves"][&insight.strategy]["feed"]["signal"],"strategy_decision":accepted["sleeves"][&insight.strategy]["feed"]
                })).await?;
            }
            for observation in observations {
                store.save_auto_trader_audit(json!({"record_schema_version":1,"decision_id":observation["id"],
                    "timestamp":observation["observed_at_ms"],"stage":"alternative:observation",
                    "strategy":observation["strategy"],"status":if accepted["sleeves"][observation["strategy"].as_str().unwrap_or("")]["feed"]["ready"]==true {"propagated"}else{"rejected"},"strategy_decision":observation})).await?;
            }
            Ok::<(),anyhow::Error>(())
        }.await;
        if let Err(e) = result {
            crate::service_log!("WARN", "Alternative insight pipeline: {e}");
        }
    }
}

#[cfg(test)]
mod integration_tests;
