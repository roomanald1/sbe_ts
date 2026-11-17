use crate::market_data::{BookSnapshot, MarketData};
use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Deserialize)]
pub struct MatchRequest {
    pub symbol: String,
    pub side: String,
    pub quantity: f64,
    pub limit_price: Option<f64>,
}
#[derive(Serialize)]
pub struct Fill {
    pub quantity: f64,
    pub price: f64,
}
#[derive(Serialize)]
pub struct MatchResponse {
    pub reference_price: Option<f64>,
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
    pub worst_ask: Option<f64>,
    pub book_updated_at_ms: u64,
    pub fills: Vec<Fill>,
    pub remaining_quantity: f64,
    pub filled_quantity: f64,
    pub average_execution_price: Option<f64>,
}

async fn match_order(
    State(market): State<Arc<MarketData>>,
    Json(request): Json<MatchRequest>,
) -> Result<Json<MatchResponse>, (StatusCode, String)> {
    if !matches!(request.side.as_str(), "buy" | "sell")
        || !request.quantity.is_finite()
        || request.quantity <= 0.0
        || request
            .limit_price
            .is_some_and(|limit| !limit.is_finite() || limit <= 0.0)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid order match request".into(),
        ));
    }
    market
        .ensure_book_subscription(&request.symbol)
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    let book = market
        .book(&request.symbol)
        .filter(|book| now_ms().is_ok_and(|now| now.saturating_sub(book.updated_at_ms) <= 5_000))
        .ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "Live market book unavailable".into(),
        ))?;
    Ok(Json(match_book(&request, &book)))
}

fn match_book(request: &MatchRequest, book: &BookSnapshot) -> MatchResponse {
    let buy = request.side == "buy";
    let levels = if buy { &book.asks } else { &book.bids };
    let mut remaining = request.quantity;
    let reference_price = levels.first().map(|level| level.price);
    let mut fills = Vec::with_capacity(levels.len());
    for level in levels {
        let crosses = match request.limit_price {
            Some(limit) if buy => level.price <= limit,
            Some(limit) => level.price >= limit,
            None => true,
        };
        if !crosses || remaining <= 1e-12 {
            continue;
        }
        let quantity = remaining.min(level.size);
        if quantity > 1e-12 {
            fills.push(Fill {
                quantity,
                price: level.price,
            });
            remaining -= quantity;
        }
    }
    let filled = request.quantity - remaining;
    let average = (filled > 1e-12).then(|| {
        fills
            .iter()
            .map(|fill| fill.quantity * fill.price)
            .sum::<f64>()
            / filled
    });
    MatchResponse {
        reference_price,
        best_bid: book.bids.first().map(|level| level.price),
        best_ask: book.asks.first().map(|level| level.price),
        worst_ask: book.asks.last().map(|level| level.price),
        book_updated_at_ms: book.updated_at_ms,
        fills,
        remaining_quantity: remaining,
        filled_quantity: filled,
        average_execution_price: average,
    }
}

fn now_ms() -> anyhow::Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as u64)
}

pub fn router(market: Arc<MarketData>) -> Router {
    Router::new()
        .route("/internal/orders/match", post(match_order))
        .with_state(market)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market_data::BookLevel;

    fn book() -> BookSnapshot {
        BookSnapshot {
            bids: vec![BookLevel {
                price: 10.0,
                size: 2.0,
            }],
            asks: vec![
                BookLevel {
                    price: 10.5,
                    size: 1.0,
                },
                BookLevel {
                    price: 11.0,
                    size: 2.0,
                },
            ],
            updated_at_ms: 1,
        }
    }

    #[test]
    fn matches_buy_across_levels_respects_limit_and_returns_book_marks() {
        let result = match_book(
            &MatchRequest {
                symbol: "BTC-USDT".into(),
                side: "buy".into(),
                quantity: 3.0,
                limit_price: Some(11.0),
            },
            &book(),
        );
        assert_eq!(result.reference_price, Some(10.5));
        assert_eq!(result.best_bid, Some(10.0));
        assert_eq!(result.best_ask, Some(10.5));
        assert_eq!(result.worst_ask, Some(11.0));
        assert_eq!(result.filled_quantity, 3.0);
        assert_eq!(result.remaining_quantity, 0.0);
        assert_eq!(result.fills.len(), 2);
    }

    #[test]
    fn respects_sell_limits_and_returns_unfilled_quantity() {
        let result = match_book(
            &MatchRequest {
                symbol: "BTC-USDT".into(),
                side: "sell".into(),
                quantity: 3.0,
                limit_price: Some(10.0),
            },
            &book(),
        );
        assert_eq!(result.filled_quantity, 2.0);
        assert_eq!(result.remaining_quantity, 1.0);
    }
}
