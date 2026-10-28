#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalSnapshot {
    pub symbol: String,
    pub timestamp: u64,
    pub direction: String,
    pub score: f64,
    pub strength: f64,
    pub components: SignalComponents,
    pub reasons: Vec<String>,
    pub stale: bool,
    /// Heuristic probability (0..1) that recent prices are trading in a range.
    #[serde(default)]
    pub ranging_probability: Option<f64>,
    /// Observed low/high over the five-minute analysis window.
    #[serde(default)]
    pub range_lower_bound: Option<f64>,
    #[serde(default)]
    pub range_upper_bound: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
#[derive(Clone, Debug, Default, Serialize)]
pub struct SideWalkForwardMetrics {
    pub sample_count: usize,
    pub direction_match_rate: Option<f64>,
    pub average_net_return_percent: Option<f64>,
    pub positive_net_rate: Option<f64>,
    pub positive_net_rate_lower_95: Option<f64>,
}

#[derive(Default)]
struct SideWalkForwardAccumulator {
    samples: usize,
    correct: usize,
    positive_net: usize,
    net_total: f64,
}
impl SideWalkForwardAccumulator {
    fn record(&mut self, correct: bool, net: f64) {
        self.samples += 1;
        if correct {
            self.correct += 1;
        }
        if net > 0.0 {
            self.positive_net += 1;
        }
        self.net_total += net;
    }
    fn metrics(&self) -> SideWalkForwardMetrics {
        let hit_rate = (self.samples > 0).then_some(self.correct as f64 / self.samples as f64);
        let positive_rate =
            (self.samples > 0).then_some(self.positive_net as f64 / self.samples as f64);
        SideWalkForwardMetrics {
            sample_count: self.samples,
            direction_match_rate: hit_rate,
            average_net_return_percent: (self.samples > 0)
                .then_some(self.net_total / self.samples as f64),
            positive_net_rate: positive_rate,
            positive_net_rate_lower_95: wilson_lower_bound(self.positive_net, self.samples),
        }
    }
}

fn wilson_lower_bound(successes: usize, samples: usize) -> Option<f64> {
    if samples == 0 {
        return None;
    }
    let n = samples as f64;
    let z = 1.96_f64;
    let rate = successes as f64 / n;
    let denominator = 1.0 + z * z / n;
    let center = rate + z * z / (2.0 * n);
    let spread = z * ((rate * (1.0 - rate) / n + z * z / (4.0 * n * n)).max(0.0)).sqrt();
    Some(((center - spread) / denominator).clamp(0.0, 1.0))
}

#[derive(Clone, Debug, Serialize)]
pub struct ProbabilityConfidenceMetrics {
    pub threshold: f64,
    pub selected_signals: usize,
    pub direction_match_rate: Option<f64>,
    pub average_net_return_percent: Option<f64>,
    pub positive_net_rate: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LearnedExitPolicy {
    pub stop_loss_percent: f64,
    pub take_profit_percent: f64,
    pub validation_samples: usize,
    pub validation_average_net_return_percent: f64,
    pub validation_positive_net_rate: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SignalWalkForward {
    pub symbol: String,
    pub horizon_seconds: u64,
    pub training_samples: usize,
    pub evaluated_signals: usize,
    pub selected_signals: usize,
    pub selected_strength_threshold: Option<f64>,
    pub direction_match_rate: Option<f64>,
    pub average_net_return_percent: Option<f64>,
    pub positive_net_rate: Option<f64>,
    pub baseline_direction_match_rate: Option<f64>,
    pub baseline_average_net_return_percent: Option<f64>,
    pub five_minute_trend_sample_count: usize,
    pub five_minute_trend_match_rate: Option<f64>,
    pub five_minute_trend_average_net_return_percent: Option<f64>,
    pub logistic_direction_match_rate: Option<f64>,
    pub logistic_average_net_return_percent: Option<f64>,
    pub logistic_positive_net_rate: Option<f64>,
    pub logistic_selected_signals: usize,
    pub trend_logistic_direction_match_rate: Option<f64>,
    pub trend_logistic_average_net_return_percent: Option<f64>,
    pub trend_logistic_positive_net_rate: Option<f64>,
    pub trend_logistic_selected_signals: usize,
    pub contrarian_trend_logistic_direction_match_rate: Option<f64>,
    pub contrarian_trend_logistic_average_net_return_percent: Option<f64>,
    pub contrarian_trend_logistic_positive_net_rate: Option<f64>,
    pub contrarian_trend_logistic_selected_signals: usize,
    pub contrarian_trend_logistic_confidence_bands: Vec<ProbabilityConfidenceMetrics>,
    pub trend_logistic_confidence_bands: Vec<ProbabilityConfidenceMetrics>,
    /// Shadow results for the live Auto Trader exploration gate, using historical score and fee-adjusted execution cost.
    pub exploration_score_bands: Vec<ProbabilityConfidenceMetrics>,
    pub signal_long: SideWalkForwardMetrics,
    pub signal_short: SideWalkForwardMetrics,
    /// All directional outcomes before any learned strength selection.
    pub all_signal_long: SideWalkForwardMetrics,
    pub all_signal_short: SideWalkForwardMetrics,
    pub logistic_long: SideWalkForwardMetrics,
    pub logistic_short: SideWalkForwardMetrics,
    /// Walk-forward results for models trained on synchronized microstructure and market-context features.
    pub research_logistic_long: SideWalkForwardMetrics,
    pub research_logistic_short: SideWalkForwardMetrics,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub learned_long_exit_policy: Option<LearnedExitPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub learned_short_exit_policy: Option<LearnedExitPolicy>,
}
/// Side-by-side executable outcome label. A new enum variant preserves the
/// existing bincode Snapshot and Outcome layouts for persisted records.
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

#[derive(Clone, Debug)]
pub struct CompletedSignalOutcome {
    pub legacy: SignalOutcome,
    pub evaluation: SignalEvaluation,
}

