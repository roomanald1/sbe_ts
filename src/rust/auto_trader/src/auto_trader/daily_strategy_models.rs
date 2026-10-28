#[derive(Clone, Deserialize, Serialize)]
struct StrategyDiagnostic {
    strategy: String,
    as_of_ms: u64,
    btc_close: Option<f64>,
    btc_sma: Option<f64>,
    btc_distance_percent: Option<f64>,
    btc_gate_required: bool,
    btc_gate_passed: Option<bool>,
    btc_window_days: Option<usize>,
    symbols: Vec<StrategySymbolDiagnostic>,
}

#[derive(Clone, Deserialize, Serialize)]
struct StrategySymbolDiagnostic {
    symbol: String,
    close: f64,
    eligible: bool,
    primary_rule: String,
    primary_level: f64,
    primary_distance_percent: f64,
    primary_passed: bool,
    secondary_rule: Option<String>,
    secondary_level: Option<f64>,
    secondary_distance_percent: Option<f64>,
    secondary_passed: Option<bool>,
    signal_active: bool,
}

#[derive(Clone, Deserialize)]
struct DailyStrategySnapshot {
    as_of_ms: u64,
    active_strategy_symbols: BTreeSet<(String, String)>,
    generated_signals: Vec<Signal>,
}

impl DailyStrategySnapshot {
    fn strategy_exit_required(&self, strategy: &str, symbol: &str) -> bool {
        let Some(strategy_kind) = Strategy::from_name(strategy) else {
            return false;
        };
        let active = self
            .active_strategy_symbols
            .contains(&(strategy.to_owned(), symbol.to_owned()));
        strategy_kind.should_exit_when_insight_inactive(active)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LiveInsightField {
    Momentum10s,
    Trend5m,
    TradeFlow,
    OrderBook,
    VwapDislocation,
    AbsorptionPressure,
}

impl LiveInsightField {
    fn is_present(self, signal: &Signal) -> bool {
        match self {
            Self::Momentum10s => signal.components.momentum_10s_percent.is_some(),
            Self::Trend5m => signal.components.trend_5m_volatility_units.is_some(),
            Self::TradeFlow => signal.components.trade_flow_imbalance.is_some(),
            Self::OrderBook => signal.components.book_imbalance.is_some(),
            Self::VwapDislocation => signal.components.mispricing_vs_vwap_bps.is_some(),
            Self::AbsorptionPressure => signal.components.pressure_without_price_move.is_some(),
        }
    }
}

#[derive(Clone, Copy)]
enum StrategyInsight<'a> {
    LiveSignal {
        signal: &'a Signal,
        parameters: &'a BTreeMap<String, f64>,
    },
}

#[derive(Clone)]
enum StrategyEvaluation {
    LiveSignal,
}
