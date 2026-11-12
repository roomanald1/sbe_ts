fn sample_microprice(bid: f64, bid_size: f64, ask: f64, ask_size: f64) -> f64 {
    (ask * bid_size + bid * ask_size) / (bid_size + ask_size)
}

fn directional_net_return(
    direction: &str,
    entry_price: f64,
    exit_price: f64,
    fee_rate: f64,
) -> Option<f64> {
    if !entry_price.is_finite()
        || !exit_price.is_finite()
        || entry_price <= 0.0
        || exit_price <= 0.0
    {
        return None;
    }
    let gross_return = match direction {
        "bullish" => (exit_price / entry_price - 1.0) * 100.0,
        "bearish" => ((entry_price - exit_price) / entry_price) * 100.0,
        _ => return None,
    };
    Some(gross_return - 2.0 * fee_rate * 100.0)
}

const COUNTERFACTUAL_DISPLAYED_DEPTH_FRACTION: f64 = 0.10;

pub(super) fn depth_adjusted_quote_execution_price(levels: &[(f64, f64)]) -> Option<f64> {
    let order_quantity = levels
        .iter()
        .filter_map(|(price, size)| {
            (price.is_finite() && *price > 0.0 && size.is_finite() && *size > 0.0).then_some(*size)
        })
        .sum::<f64>()
        * COUNTERFACTUAL_DISPLAYED_DEPTH_FRACTION;
    if !order_quantity.is_finite() || order_quantity <= 0.0 {
        return None;
    }
    quote_path_vwap(levels, order_quantity)
}

fn quote_path_vwap(levels: &[(f64, f64)], order_quantity: f64) -> Option<f64> {
    let mut remaining = order_quantity;
    let mut notional = 0.0;
    for (price, size) in levels.iter().copied() {
        if !price.is_finite() || price <= 0.0 || !size.is_finite() || size <= 0.0 {
            continue;
        }
        let filled = remaining.min(size);
        notional += price * filled;
        remaining -= filled;
        if remaining <= 1e-12 {
            return Some(notional / order_quantity);
        }
    }
    None
}

pub fn configured_signal_taker_fee_rate() -> f64 {
    let configured = std::env::var("SIGNALS_TAKER_FEE_RATE")
        .or_else(|_| std::env::var("PAPER_OKX_SPOT_TAKER_FEE_RATE"))
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .map(f64::abs)
        .unwrap_or(0.001);
    configured.clamp(0.0, 0.02)
}

#[cfg(test)]
mod outcome_model_tests {
    use super::*;

    #[test]
    fn microprice_and_directional_returns_handle_size_direction_and_invalid_inputs() {
        assert_eq!(sample_microprice(99.0, 3.0, 101.0, 1.0), 100.5);
        assert!(
            (directional_net_return("bullish", 100.0, 102.0, 0.001).unwrap() - 1.8).abs() < 1e-10
        );
        assert!(
            (directional_net_return("bearish", 100.0, 98.0, 0.001).unwrap() - 1.8).abs() < 1e-10
        );
        assert_eq!(directional_net_return("neutral", 100.0, 98.0, 0.0), None);
        assert_eq!(directional_net_return("bullish", 0.0, 98.0, 0.0), None);
    }

    #[test]
    fn depth_adjusted_execution_price_accounts_for_visible_book_depth() {
        let levels = [
            (100.0, 0.2),
            (99.5, 9.8),
            (0.0, 0.0),
            (0.0, 0.0),
            (0.0, 0.0),
            (0.0, 0.0),
            (0.0, 0.0),
        ];
        let execution_price = depth_adjusted_quote_execution_price(&levels).unwrap();
        assert!((execution_price - 99.6).abs() < 1e-10);
    }

}
