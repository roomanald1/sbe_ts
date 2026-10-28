#[derive(Serialize)]

struct PositionInsightsInput {
    positions: Vec<Position>,
    markets: HashMap<String, PositionMarketInput>,
}
#[derive(Serialize)]
struct PositionMarketInput {
    bids: Vec<BookLevel>,
    asks: Vec<BookLevel>,
    price_samples: Vec<(u64, f64)>,
}
#[derive(Deserialize, Default)]
struct InsightsInputQuery {
    symbol: Option<String>,
}
async fn get_market_book(
    State(store): State<Arc<PositionStore>>,
    Query(query): Query<InsightsInputQuery>,
) -> Result<Json<BookSnapshot>, (StatusCode, String)> {
    let symbol = query
        .symbol
        .map(|symbol| symbol.to_ascii_uppercase())
        .filter(|symbol| {
            valid_position_symbol(symbol) && store.spot_symbols.iter().any(|spot| spot == symbol)
        })
        .ok_or((
            StatusCode::BAD_REQUEST,
            "A valid subscribed spot symbol is required".to_owned(),
        ))?;
    store.ensure_subscription(&symbol);
    let now = now_ms().map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let books = store.books_snapshot();
    let book = books
        .get(&symbol)
        .filter(|book| now.saturating_sub(book.updated_at_ms) <= 5_000)
        .cloned()
        .ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            format!("No fresh local order book for {symbol}"),
        ))?;
    Ok(Json((*book).clone()))
}
async fn get_insights_inputs(
    State(store): State<Arc<PositionStore>>,
    Query(query): Query<InsightsInputQuery>,
) -> Result<Json<PositionInsightsInput>, (StatusCode, String)> {
    let positions = summarize_positions(&store.ledger.lock().await.trades)
        .positions
        .into_iter()
        .filter(|position| position.quantity.abs() > EPSILON)
        .collect::<Vec<_>>();
    let mut symbols = positions
        .iter()
        .map(|position| position.symbol.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let requested_symbol = query
        .symbol
        .map(|symbol| symbol.to_ascii_uppercase())
        .filter(|symbol| {
            valid_position_symbol(symbol) && store.spot_symbols.iter().any(|spot| spot == symbol)
        });
    if let Some(symbol) = requested_symbol.as_ref() {
        symbols.insert(symbol.clone());
    }
    for symbol in &symbols {
        store.ensure_subscription(symbol);
    }
    // Return the latest snapshot, including an empty book while a new
    // subscription is warming up. Consumers handle missing bid/ask data as a
    // temporarily unavailable market; blocking here made transient feed delays
    // fail the entire auto-trader cycle with HTTP 503.
    let books = store.books_snapshot();
    let histories = store.price_samples.lock().await;
    let markets = symbols
        .into_iter()
        .map(|symbol| {
            let book = books.get(&symbol);
            (
                symbol.clone(),
                PositionMarketInput {
                    bids: book
                        .map(|snapshot| snapshot.bids.clone())
                        .unwrap_or_default(),
                    asks: book
                        .map(|snapshot| snapshot.asks.clone())
                        .unwrap_or_default(),
                    price_samples: histories.get(&symbol).cloned().unwrap_or_default(),
                },
            )
        })
        .collect();
    Ok(Json(PositionInsightsInput { positions, markets }))
}


