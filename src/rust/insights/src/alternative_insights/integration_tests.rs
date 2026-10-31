//! Explicit read-only network smoke. Uses the production FeedHandlers router on
//! an ephemeral test listener; never starts or cycles the application stack.
use super::*;
#[path = "../../../auto_trader/src/auto_trader/alternative_strategies/engine.rs"]
#[allow(dead_code)]
mod engine;
#[path = "../../../feed_handlers/src/alternative_market_data/mod.rs"]
mod ingress;
#[path = "../../../auto_trader/src/auto_trader/alternative_strategies/types.rs"]
#[allow(dead_code)]
mod types;

#[tokio::test]
#[ignore = "Explicit public-provider connectivity check; no credentials or transactions"]
async fn native_feeds_to_insights_to_paper_strategies() {
    use axum::{Json, Router, extract::Query, routing::get};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let feed = FeedClient::new(base);
    // The existing daily-candle endpoint is represented using the same native
    // ingress and wire shape, so this test needs no running cache/service.
    let history_feed = feed.clone();
    let app = Router::new()
        .route(
            "/market-data/daily-candles",
            get(move |Query(q): Query<BTreeMap<String, String>>| {
                let feed = history_feed.clone();
                async move {
                    let mut result = BTreeMap::<String, Value>::new();
                    for pair in q.get("symbols").unwrap().split(',') {
                        let raw = feed
                            .binance(
                                "klines",
                                pair,
                                &[("interval", "1d".into()), ("limit", "100".into())],
                            )
                            .await
                            .unwrap();
                        let rows=raw.data.as_array().unwrap().iter().map(|r|json!({
                    "close_time_ms":r[6],"high":r[2].as_str().unwrap().parse::<f64>().unwrap(),
                    "close":r[4].as_str().unwrap().parse::<f64>().unwrap()
                })).collect::<Vec<_>>();
                        result.insert(pair.into(), json!(rows));
                    }
                    Json(result)
                }
            }),
        )
        .merge(ingress::router());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let book = types::Book::initialized();
    let sleeves = serde_json::to_value(&book).unwrap();
    let sleeves = &sleeves["sleeves"];
    let (r, c, ctx) = tokio::join!(
        residual(&feed, &sleeves[IDS[0]]),
        options::build(&feed, &sleeves[IDS[1]]),
        chain::Context::load(&feed)
    );
    let ctx = ctx.expect("Ethereum head, oracle and simulation capability");
    let invalid_loan = feed
        .post(
            "/market-data/alternative/liquidation-simulation",
            json!({"user":"0x0000000000000000000000000000000000000000",
            "repayment_wei":"0x2386f26fc10000","block":ctx.tag}),
        )
        .await
        .unwrap();
    assert!(
        !invalid_loan.data["error"].is_null(),
        "A loan without debt must never simulate success"
    );
    let mut lp = None;
    let mut index = aave::AaveIndex::default();
    let (staking, liquidity, aave) = tokio::join!(
        chain::staking(&feed, &ctx, &sleeves[IDS[2]]),
        liquidity::build(&feed, &ctx, &sleeves[IDS[4]], &mut lp),
        aave::build(&feed, &ctx, &sleeves[IDS[3]], &mut index)
    );
    let outputs = [r, c, staking, aave, liquidity];
    for (id, result) in IDS.into_iter().zip(outputs) {
        let output = result.unwrap_or_else(|e| panic!("{id}: {e:#}"));
        assert!(output.insight.ready, "{id}: {}", output.insight.summary);
        println!(
            "{id}: {} | {}",
            output.insight.signal, output.insight.summary
        );
        let mut sleeve = types::Sleeve::new(id);
        sleeve.config.enabled = true;
        for value in output.observations {
            let observation: types::Observation = serde_json::from_value(value).unwrap();
            engine::apply(&mut sleeve, &observation, true)
                .unwrap_or_else(|e| panic!("{id} rejected native insight: {e}"));
        }
        assert!(sleeve.equity_usd.is_finite());
    }
    task.abort();
}
