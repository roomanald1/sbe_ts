#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewsHeadlineAttribution {
    pub title: String,
    pub url: String,
    pub published_at: String,
    pub received_at_ms: u64,
    pub matched_asset: String,
    pub direction: String,
    pub score: f64,
    pub sentiment_model: String,
    pub sentiment_confidence: f64,
    pub matched_keywords: Vec<String>,
    pub signal_reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewsAttribution {
    pub pre_news_score: f64,
    pub post_news_score: f64,
    pub aggregate_score: f64,
    pub weight: f64,
    pub article_count: usize,
    pub source: String,
    pub articles: Vec<NewsHeadlineAttribution>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalSnapshot {
    pub symbol: String,
    pub timestamp: u64,
    pub direction: String,
    pub score: f64,
    pub strength: f64,
    pub components: SignalComponents,
    pub reasons: Vec<String>,
    /// News evidence that contributed to this signal, captured for decision audits.
    #[serde(default)]
    pub news_attribution: Option<NewsAttribution>,
    pub stale: bool,
    /// Heuristic probability (0..1) that recent prices are trading in a range.
    #[serde(default)]
    pub ranging_probability: Option<f64>,
    /// Observed low/high over the five-minute analysis window.
    #[serde(default)]
    pub range_lower_bound: Option<f64>,
    #[serde(default)]
    pub range_upper_bound: Option<f64>,
    /// Fraction of the engine's directional inputs that were available.
    #[serde(default)]
    pub evidence_coverage: Option<f64>,
    /// Outcome-calibrated hit-rate estimate. `strength` remains raw conviction.
    #[serde(default)]
    pub estimated_accuracy: Option<f64>,
}

impl SignalSnapshot {
    pub fn refresh_raw_strength(&mut self) {
        if let Some(coverage) = self
            .evidence_coverage
            .filter(|coverage| coverage.is_finite() && (0.0..=1.0).contains(coverage))
        {
            self.strength = (self.score.abs() * coverage).clamp(0.0, 1.0);
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WindowedSignalValue {
    pub window_seconds: u16,
    pub value: Option<f64>,
    #[serde(default)]
    pub sample_count: u16,
    #[serde(default)]
    pub span_ms: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DepthSignalValue {
    pub depth_levels: u8,
    pub value: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AbsorptionPressureValue {
    pub window_seconds: u16,
    pub replenishment_window_ms: u32,
    pub touch_tolerance_bps: f64,
    pub value: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SignalComponents {
    pub trend_1m_percent: Option<f64>,
    pub momentum_10s_percent: Option<f64>,
    pub trade_flow_imbalance: Option<f64>,
    pub book_imbalance: Option<f64>,
    pub volatility_1m_percent: Option<f64>,
    #[serde(default)]
    pub trend_5m_percent: Option<f64>,
    #[serde(default)]
    pub trend_5m_volatility_units: Option<f64>,
    #[serde(default)]
    pub mispricing_vs_vwap_bps: Option<f64>,
    #[serde(default)]
    pub pressure_without_price_move: Option<f64>,
    #[serde(default)]
    pub aggressor_cost_bps: Option<f64>,
    /// Feature values at supported live-strategy lookbacks; window is in seconds.
    #[serde(default)]
    pub momentum_by_window: Vec<WindowedSignalValue>,
    #[serde(default)]
    pub trend_volatility_units_by_window: Vec<WindowedSignalValue>,
    #[serde(default)]
    pub trade_flow_by_window: Vec<WindowedSignalValue>,
    /// Bid/ask size imbalance using the first N displayed book levels.
    #[serde(default)]
    pub book_imbalance_by_depth: Vec<DepthSignalValue>,
    #[serde(default)]
    pub vwap_dislocation_by_window: Vec<WindowedSignalValue>,
    #[serde(default)]
    pub absorption_pressure_by_window: Vec<AbsorptionPressureValue>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalOutcome {
    pub record_type: String,
    pub symbol: String,
    pub signal_timestamp: u64,
    #[serde(default)]
    pub evaluated_timestamp: u64,
    pub horizon_seconds: u64,
    pub return_percent: f64,
    pub direction_match: bool,
    #[serde(default)]
    pub direction: String,
    #[serde(default)]
    pub strength: f64,
    #[serde(default)]
    pub net_return_percent: Option<f64>,
    #[serde(default)]
    pub execution_cost_percent: Option<f64>,
    #[serde(default)]
    pub trend_5m_volatility_units: Option<f64>,
    #[serde(default)]
    pub trend_5m_direction_match: Option<bool>,
    #[serde(default)]
    pub trend_5m_net_return_percent: Option<f64>,
    #[serde(default)]
    pub features: Option<[f64; 4]>,
}
#[derive(Clone, Debug, Serialize)]
pub struct SignalPerformance {
    pub symbol: String,
    pub horizon_seconds: u64,
    pub sample_count: usize,
    pub directional_sample_count: usize,
    pub neutral_sample_count: usize,
    pub bullish_match_rate: Option<f64>,
    pub bearish_match_rate: Option<f64>,
    pub direction_match_rate: Option<f64>,
    pub average_return_percent: f64,
    pub net_sample_count: usize,
    pub average_net_return_percent: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalResearchFeatures {
    pub symbol: String,
    pub timestamp: u64,
    pub spread_bps: Option<f64>,
    pub microprice_offset_bps: Option<f64>,
    pub bid_notional_10bps: Option<f64>,
    pub ask_notional_10bps: Option<f64>,
    pub depth_imbalance_10bps: Option<f64>,
    pub depth_imbalance_change_1s: Option<f64>,
    pub depth_imbalance_change_5s: Option<f64>,
    pub trade_flow_5s: Option<f64>,
    pub trade_flow_15s: Option<f64>,
    pub trade_flow_60s: Option<f64>,
    pub trade_flow_acceleration: Option<f64>,
    pub signed_trade_notional_5s: Option<f64>,
    pub signed_trade_notional_15s: Option<f64>,
    pub trade_count_5s: u64,
    pub trade_count_60s: u64,
    pub average_trade_notional_5s: Option<f64>,
    pub price_response_bps_per_1000usd_5s: Option<f64>,
    pub btc_return_1m_percent: Option<f64>,
    pub btc_return_5m_percent: Option<f64>,
    pub eth_return_1m_percent: Option<f64>,
    pub eth_return_5m_percent: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExternalMarketFeatures {
    pub underlying: String,
    pub timestamp: u64,
    pub swap_instrument: String,
    pub funding_rate: Option<f64>,
    pub next_funding_rate: Option<f64>,
    pub open_interest_contracts: Option<f64>,
    pub open_interest_currency: Option<f64>,
    pub open_interest_change_percent: Option<f64>,
    pub mark_price: Option<f64>,
    pub index_price: Option<f64>,
    pub perp_basis_bps: Option<f64>,
    pub options_atm_iv: Option<f64>,
    pub options_25d_skew: Option<f64>,
    pub options_expiry_timestamp: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalBookSnapshot {
    pub symbol: String,
    pub timestamp: u64,
    pub bid_levels: Vec<(f64, f64)>,
    pub ask_levels: Vec<(f64, f64)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalEvaluation {
    pub symbol: String,
    pub signal_timestamp: u64,
    pub evaluated_timestamp: u64,
    pub horizon_seconds: u64,
    pub signal_direction: String,
    pub signal_score: f64,
    pub ranging_probability: Option<f64>,
    pub reference_entry_price: f64,
    pub reference_exit_price: f64,
    pub entry_bid: Option<f64>,
    pub entry_ask: Option<f64>,
    pub entry_quote_timestamp: Option<u64>,
    pub exit_bid: Option<f64>,
    pub exit_ask: Option<f64>,
    pub exit_quote_timestamp: Option<u64>,
    pub taker_fee_rate: f64,
    pub long_gross_percent: f64,
    pub short_gross_percent: f64,
    pub long_net_percent: Option<f64>,
    pub short_net_percent: Option<f64>,
    pub path_high: f64,
    pub path_high_timestamp: u64,
    pub path_low: f64,
    pub path_low_timestamp: u64,
    /// Directional time to positive net after spread and configured taker fees.
    #[serde(default)]
    pub long_time_to_positive_net_seconds: Option<u64>,
    #[serde(default)]
    pub short_time_to_positive_net_seconds: Option<u64>,
}

// Retained only to decode historical kind-6 records. New evaluations never
// generate or persist quote paths.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CounterfactualQuoteSample {
    pub timestamp: u64,
    pub bid_execution_price: Option<f64>,
    pub ask_execution_price: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CounterfactualQuotePath {
    pub symbol: String,
    pub signal_timestamp: u64,
    pub horizon_seconds: u64,
    pub samples: Vec<CounterfactualQuoteSample>,
}

#[derive(Clone, Debug)]
pub struct CompletedSignalOutcome {
    pub legacy: SignalOutcome,
    pub evaluation: SignalEvaluation,
    pub scout_id: Option<String>,
    pub scout_exit_reason: Option<&'static str>,
    pub scout_observation_only: bool,
}
