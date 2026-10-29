#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Dashboard {
    best: Vec<Signal>,
    walk_forward: Arc<Vec<WalkForward>>,
    #[serde(default)]
    missed_opportunity_bands: Vec<MissedOpportunityGateBand>,
    #[serde(default)]
    markets: Arc<BTreeMap<String, serde_json::Value>>,
    #[serde(skip)]
    queued_at: Option<Instant>,
    #[serde(skip)]
    queue_wait_ms: Option<f64>,
}

fn is_stablecoin_pair(symbol: &str) -> bool {
    const STABLECOINS: &[&str] = &[
        "BUSD", "DAI", "FDUSD", "FRAX", "GUSD", "LUSD", "PYUSD", "RLUSD", "TUSD",
        "USDC", "USDD", "USDE", "USDP", "USDS", "USDT", "USDG", "EURC",
    ];
    let Some((base, quote)) = symbol.split_once('-') else {
        return false;
    };
    STABLECOINS.contains(&base.to_ascii_uppercase().as_str())
        && STABLECOINS.contains(&quote.to_ascii_uppercase().as_str())
}
#[derive(Clone, Deserialize)]
struct Signal {
    symbol: String,
    direction: String,
    strength: f64,
    timestamp: u64,
    stale: bool,
    #[serde(default)]
    score: f64,
    #[serde(default)]
    ranging_probability: Option<f64>,
    #[serde(default)]
    components: SignalComponents,
    #[serde(default, alias = "research_strategy_candidate")]
    strategy_candidate: bool,
    #[serde(default, alias = "research_strategy")]
    strategy: Option<String>,
    #[serde(default)]
    strategy_decision: Option<serde_json::Value>,
    #[serde(default)]
    market_regime: Option<String>,
    #[serde(default)]
    news_attribution: Option<serde_json::Value>,
    #[serde(default)]
    entry_gate: Option<serde_json::Value>,
}
#[derive(Clone, Default, Deserialize, Serialize)]
struct SignalComponents {
    momentum_10s_percent: Option<f64>,
    trend_5m_volatility_units: Option<f64>,
    #[serde(default)]
    trade_flow_imbalance: Option<f64>,
    #[serde(default)]
    book_imbalance: Option<f64>,
    volatility_1m_percent: Option<f64>,
    mispricing_vs_vwap_bps: Option<f64>,
    pressure_without_price_move: Option<f64>,
    aggressor_cost_bps: Option<f64>,
}
#[derive(Clone, Deserialize, Serialize)]
struct LearnedExitPolicy {
    stop_loss_percent: f64,
    take_profit_percent: f64,
    validation_samples: usize,
    validation_average_net_return_percent: f64,
    validation_positive_net_rate: f64,
}
#[derive(Clone, Deserialize, Serialize)]
struct WalkForward {
    symbol: String,
    horizon_seconds: u64,
    #[serde(default)]
    learned_long_exit_policy: Option<LearnedExitPolicy>,
    #[serde(default)]
    learned_short_exit_policy: Option<LearnedExitPolicy>,
    #[serde(default)]
    all_signal_long: SideWalkForwardMetrics,
    #[serde(default)]
    all_signal_short: SideWalkForwardMetrics,
    #[serde(default)]
    research_logistic_long: SideWalkForwardMetrics,
    #[serde(default)]
    research_logistic_short: SideWalkForwardMetrics,
}
#[derive(Clone, Default, Deserialize, Serialize)]
struct SideWalkForwardMetrics {
    sample_count: usize,
    direction_match_rate: Option<f64>,
    average_net_return_percent: Option<f64>,
    positive_net_rate: Option<f64>,
    positive_net_rate_lower_95: Option<f64>,
}
#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum MissedOpportunityGate {
    Score,
    Strength,
    #[serde(other)]
    Other,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MissedOpportunityGateBand {
    gate: MissedOpportunityGate,
    threshold: f64,
    #[serde(default)]
    horizon_seconds: u64,
    #[serde(default)]
    sample_count: usize,
    #[serde(default)]
    average_net_return_percent: Option<f64>,
    #[serde(default)]
    positive_net_rate: Option<f64>,
    #[serde(default)]
    time_to_profit_sample_count: usize,
    #[serde(default)]
    time_to_profit_rate: Option<f64>,
    #[serde(default)]
    average_time_to_profit_seconds: Option<f64>,
    #[serde(default)]
    stop_target_sample_count: usize,
    #[serde(default)]
    stop_target_average_net_return_percent: Option<f64>,
    #[serde(default)]
    stop_target_positive_net_rate: Option<f64>,
}

#[derive(Clone, Copy)]
struct DeepBookAssessment {
    available_within_band: f64,
    notional_cap: f64,
    impact_bps: f64,
    bid_depth_10bps: f64,
    ask_depth_10bps: f64,
    depth_imbalance_10bps: f64,
}

fn market_levels(market: &serde_json::Value, side: &str) -> Vec<(f64, f64)> {
    market
        .get(side)
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|level| {
            let price = level.get("price")?.as_f64()?;
            let size = level.get("size")?.as_f64()?;
            (price.is_finite() && size.is_finite() && price > 0.0 && size > 0.0)
                .then_some((price, size))
        })
        .collect()
}

fn quote_depth_within_band(levels: &[(f64, f64)], mid: f64, is_bid: bool) -> f64 {
    let band_price = if is_bid {
        mid * (1.0 - DEEP_BOOK_BAND_BPS / 10_000.0)
    } else {
        mid * (1.0 + DEEP_BOOK_BAND_BPS / 10_000.0)
    };
    levels
        .iter()
        .filter(|(price, _)| {
            if is_bid {
                *price >= band_price
            } else {
                *price <= band_price
            }
        })
        .map(|(price, size)| price * size)
        .sum()
}

fn assess_deep_book(
    market: &serde_json::Value,
    symbol: &str,
    is_buy: bool,
    target_notional: f64,
    minimum_order_notional: f64,
) -> Option<DeepBookAssessment> {
    let quote = symbol.rsplit('-').next().unwrap_or_default();
    if !matches!(quote, "USDT" | "USDC") {
        return None;
    }
    let bids = market_levels(market, "bids");
    let asks = market_levels(market, "asks");
    let best_bid = bids.first()?.0;
    let best_ask = asks.first()?.0;
    if best_ask < best_bid {
        return None;
    }
    let mid = (best_bid + best_ask) / 2.0;
    let bid_depth_10bps = quote_depth_within_band(&bids, mid, true);
    let ask_depth_10bps = quote_depth_within_band(&asks, mid, false);
    let total_depth = bid_depth_10bps + ask_depth_10bps;
    let depth_imbalance_10bps = if total_depth > 0.0 {
        (bid_depth_10bps - ask_depth_10bps) / total_depth
    } else {
        0.0
    };
    let levels = if is_buy { &asks } else { &bids };
    let available_within_band = if is_buy {
        ask_depth_10bps
    } else {
        bid_depth_10bps
    };
    let notional_cap = available_within_band * DEEP_BOOK_MAX_PARTICIPATION;
    let planned_notional = target_notional.min(notional_cap);
    if planned_notional < minimum_order_notional {
        return Some(DeepBookAssessment {
            available_within_band,
            notional_cap,
            impact_bps: f64::INFINITY,
            bid_depth_10bps,
            ask_depth_10bps,
            depth_imbalance_10bps,
        });
    }
    let mut remaining = planned_notional / mid;
    let mut filled = 0.0;
    let mut quote_value = 0.0;
    for (price, size) in levels {
        let quantity = remaining.min(*size);
        filled += quantity;
        quote_value += quantity * price;
        remaining -= quantity;
        if remaining <= 1e-12 {
            break;
        }
    }
    let impact_bps = if remaining > 1e-12 || filled <= 0.0 {
        f64::INFINITY
    } else {
        let average = quote_value / filled;
        if is_buy {
            (average / mid - 1.0) * 10_000.0
        } else {
            (1.0 - average / mid) * 10_000.0
        }
    };
    Some(DeepBookAssessment {
        available_within_band,
        notional_cap,
        impact_bps,
        bid_depth_10bps,
        ask_depth_10bps,
        depth_imbalance_10bps,
    })
}
