use axum::{Json, Router, http::StatusCode, routing::post};
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize)]
pub struct Level {
    pub price: f64,
    pub size: f64,
}
#[derive(Deserialize)]
pub struct MatchRequest {
    pub side: String,
    pub quantity: f64,
    pub limit_price: Option<f64>,
    pub book: Book,
}
#[derive(Deserialize)]
pub struct Book {
    pub bids: Vec<Level>,
    pub asks: Vec<Level>,
}
#[derive(Serialize)]
pub struct Fill {
    pub quantity: f64,
    pub price: f64,
}
#[derive(Serialize)]
pub struct MatchResponse {
    pub reference_price: Option<f64>,
    pub fills: Vec<Fill>,
    pub remaining_quantity: f64,
    pub filled_quantity: f64,
    pub average_execution_price: Option<f64>,
}

async fn match_order(
    Json(r): Json<MatchRequest>,
) -> Result<Json<MatchResponse>, (StatusCode, String)> {
    if !matches!(r.side.as_str(), "buy" | "sell") || !r.quantity.is_finite() || r.quantity <= 0.0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid order match request".into(),
        ));
    }
    let buy = r.side == "buy";
    let levels = if buy { &r.book.asks } else { &r.book.bids };
    let mut remaining = r.quantity;
    let reference = levels.first().map(|l| l.price);
    let mut fills = Vec::with_capacity(levels.len());
    for level in levels {
        let crosses = match r.limit_price {
            Some(limit) if buy => level.price <= limit,
            Some(limit) => level.price >= limit,
            None => true,
        };
        if !crosses || remaining <= 1e-12 {
            continue;
        }
        let qty = remaining.min(level.size);
        if qty > 1e-12 {
            fills.push(Fill {
                quantity: qty,
                price: level.price,
            });
            remaining -= qty;
        }
    }
    let filled = r.quantity - remaining;
    let average = if filled > 1e-12 {
        Some(fills.iter().map(|f| f.quantity * f.price).sum::<f64>() / filled)
    } else {
        None
    };
    Ok(Json(MatchResponse {
        reference_price: reference,
        fills,
        remaining_quantity: remaining,
        filled_quantity: filled,
        average_execution_price: average,
    }))
}
pub fn router() -> Router {
    Router::new().route("/internal/orders/match", post(match_order))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn matches_buy_across_levels_and_respects_limit() {
        let result = match_order(Json(MatchRequest {
            side: "buy".into(),
            quantity: 3.0,
            limit_price: Some(11.0),
            book: Book {
                bids: vec![],
                asks: vec![
                    Level {
                        price: 10.0,
                        size: 1.0,
                    },
                    Level {
                        price: 11.0,
                        size: 2.0,
                    },
                    Level {
                        price: 12.0,
                        size: 5.0,
                    },
                ],
            },
        }))
        .await
        .unwrap()
        .0;
        assert_eq!(result.reference_price, Some(10.0));
        assert_eq!(result.filled_quantity, 3.0);
        assert_eq!(result.remaining_quantity, 0.0);
        assert_eq!(result.fills.len(), 2);
    }
    #[tokio::test]
    async fn rejects_invalid_side_and_quantity() {
        for (side, quantity) in [("hold", 1.0), ("buy", 0.0), ("sell", f64::NAN)] {
            let result = match_order(Json(MatchRequest {
                side: side.into(),
                quantity,
                limit_price: None,
                book: Book {
                    bids: vec![],
                    asks: vec![],
                },
            }))
            .await;
            assert!(matches!(result, Err((StatusCode::BAD_REQUEST, _))));
        }
    }
}
