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
    primary_metric_value: Option<f64>,
    primary_metric_unit: Option<String>,
    primary_passed: bool,
    secondary_rule: Option<String>,
    secondary_level: Option<f64>,
    secondary_distance_percent: Option<f64>,
    secondary_passed: Option<bool>,
    #[serde(default)]
    evaluated: bool,
    #[serde(default)]
    signal_active: bool,
    leader_gap_percent: Option<f64>,
    relative_strength_gap_pp: Option<f64>,
    rsi_exit_level: Option<f64>,
    maximum_hold_days: Option<f64>,
    trend_return_windows: Option<Vec<TrendReturnWindow>>,
}

#[derive(Clone, Deserialize, Serialize)]
struct TrendReturnWindow {
    days: usize,
    reference_close: f64,
    return_percent: f64,
}

#[derive(Clone, Deserialize)]
struct DailyStrategySnapshot {
    as_of_ms: u64,
    #[serde(default)]
    sweep_id: String,
    active_strategy_symbols: BTreeSet<(String, String)>,
    generated_signals: Vec<Signal>,
    // A symbol is inactive only when the latest batch explicitly evaluated it.
    // Missing symbols are unknown (for example, if a batch is partial).
    #[serde(skip)]
    evaluated_strategy_symbols: BTreeSet<(String, String)>,
    #[serde(skip)]
    diagnostics: Vec<StrategyDiagnostic>,
}

impl DailyStrategySnapshot {
    fn evaluate_position(
        &self,
        strategy: &str,
        symbol: &str,
        strategy_enabled: bool,
        previous_decision: Option<&serde_json::Value>,
        minimum_hold_active: bool,
    ) -> StrategyDecision {
        let Some(strategy_kind) = Strategy::from_name(strategy) else {
            return if strategy_enabled { StrategyDecision::Hold } else { StrategyDecision::Exit };
        };
        let signal_active = self.signal_active(strategy, symbol);
        let rebalance = self.strategy_rebalance_required(strategy, symbol, previous_decision);
        let parameters = BTreeMap::new();
        strategy_kind.evaluate(&StrategySignal {
            signal: None, specific_signal: None, daily_features: None, direction: None,
            strategy: Some(strategy_kind), parameters: &parameters, position_open: true,
            strategy_enabled, signal_active: if minimum_hold_active { None } else { signal_active },
            rebalance_required: rebalance && !minimum_hold_active,
        }).unwrap_or(StrategyDecision::Hold)
    }

    fn signal_active(&self, strategy: &str, symbol: &str) -> Option<bool> {
        let key = (strategy.to_owned(), symbol.to_owned());
        self.evaluated_strategy_symbols.contains(&key).then(|| {
            self.active_strategy_symbols.contains(&key)
        })
    }

    fn symbol_diagnostic(&self, strategy: &str, symbol: &str) -> Option<serde_json::Value> {
        self.diagnostics.iter()
            .find(|diagnostic| diagnostic.strategy == strategy)
            .and_then(|diagnostic| diagnostic.symbols.iter().find(|item| item.symbol == symbol))
            .and_then(|item| serde_json::to_value(item).ok())
    }

    fn strategy_rebalance_required(
        &self,
        strategy: &str,
        symbol: &str,
        previous_decision: Option<&serde_json::Value>,
    ) -> bool {
        if Strategy::from_name(strategy) != Some(Strategy::RiskAdjustedMonthlyMomentum) {
            return false;
        }
        let Some(previous_anchor) = previous_decision
            .and_then(|decision| decision.get("monthly_anchor_close_time_ms"))
            .and_then(serde_json::Value::as_u64)
        else {
            return false;
        };
        self.generated_signals.iter().any(|signal| {
            if signal.strategy.as_deref() != Some(strategy) || signal.symbol != symbol {
                return false;
            }
            let parameters = BTreeMap::new();
            let current_anchor = signal.strategy_decision.as_ref()
                .and_then(|decision| decision.get("monthly_anchor_close_time_ms"))
                .and_then(serde_json::Value::as_u64);
            current_anchor.is_some_and(|anchor| anchor > previous_anchor)
                && Strategy::RiskAdjustedMonthlyMomentum.evaluate(&StrategySignal::entry(signal, &parameters))
                    .is_ok_and(|decision| decision == StrategyDecision::Enter)
        })
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SignalDirection {
    Long,
    Short,
}

impl SignalDirection {
    fn from_wire(value: &str) -> Option<Self> {
        match value.to_ascii_uppercase().as_str() {
            "UP" | "LONG" | "BULLISH" => Some(Self::Long),
            "DOWN" | "SHORT" | "BEARISH" => Some(Self::Short),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DatedCarrySettlementSource {
    Official,
    #[serde(rename = "index_1m_estimate")]
    Index1mEstimate,
}

impl DatedCarrySettlementSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Official => "official",
            Self::Index1mEstimate => "index_1m_estimate",
        }
    }
}


#[derive(Clone, Copy, Deserialize)]
struct DailyStrategySignalFeatures {
    #[serde(default)]
    observed_close: Option<f64>,
    #[serde(default)]
    primary_level: Option<f64>,
    #[serde(default)]
    secondary_level: Option<f64>,
    #[serde(default)]
    btc_gate_required: bool,
    #[serde(default)]
    btc_close: Option<f64>,
    #[serde(default)]
    btc_sma: Option<f64>,
    #[serde(default)]
    primary_metric_value: Option<f64>,
    #[serde(default)]
    strategy_order_size_multiplier: Option<f64>,
    #[serde(default)]
    monthly_anchor_close_time_ms: Option<u64>,
    #[serde(default)]
    cross_sectional_metric_value: Option<f64>,
    #[serde(default)]
    cross_sectional_threshold: Option<f64>,
}

impl DailyStrategySignalFeatures {
    fn close_above(&self, level: Option<f64>) -> bool {
        self.observed_close.zip(level).is_some_and(|(close, level)| {
            close.is_finite() && level.is_finite() && close > level
        })
    }

    fn btc_gate_passes(&self) -> bool {
        !self.btc_gate_required || self.btc_close.zip(self.btc_sma).is_some_and(|(close, sma)| {
            close.is_finite() && sma.is_finite() && close > sma
        })
    }

    fn cross_sectional_leader(&self) -> bool {
        self.cross_sectional_metric_value.zip(self.cross_sectional_threshold)
            .is_some_and(|(value, threshold)| value.is_finite() && threshold.is_finite() && value >= threshold)
    }
}

#[derive(Clone, Copy)]
enum StrategySpecificSignal<'a> {
    DatedCarryEntry {
        emitted_at_ms: u64,
        evaluated_at_ms: u64,
        candidate: &'a DatedCarryCandidate,
    },
    DatedCarrySettlement {
        emitted_at_ms: u64,
        evaluated_at_ms: u64,
        spot_symbol: &'a str,
        future_symbol: &'a str,
        expiry_ms: u64,
        spot_exit_twap: f64,
        settlement_price: f64,
        settlement_source: Option<DatedCarrySettlementSource>,
    },
}

/// Strategies receive only the published signal plus lifecycle context.
/// Wire-format strings are converted to enums before strategy evaluation.
#[derive(Clone, Copy)]
struct StrategySignal<'a> {
    signal: Option<&'a Signal>,
    specific_signal: Option<StrategySpecificSignal<'a>>,
    daily_features: Option<DailyStrategySignalFeatures>,
    direction: Option<SignalDirection>,
    strategy: Option<Strategy>,
    parameters: &'a BTreeMap<String, f64>,
    position_open: bool,
    strategy_enabled: bool,
    signal_active: Option<bool>,
    rebalance_required: bool,
}

impl<'a> StrategySignal<'a> {
    fn entry(signal: &'a Signal, parameters: &'a BTreeMap<String, f64>) -> Self {
        Self {
            signal: Some(signal),
            specific_signal: None,
            daily_features: signal.strategy_decision.as_ref()
                .and_then(|decision| serde_json::from_value(decision.clone()).ok()),
            direction: SignalDirection::from_wire(&signal.direction),
            strategy: signal.strategy.as_deref().and_then(Strategy::from_name),
            parameters,
            position_open: false,
            strategy_enabled: true,
            signal_active: None,
            rebalance_required: false,
        }
    }

    fn position(
        parameters: &'a BTreeMap<String, f64>,
        strategy_enabled: bool,
        signal_active: Option<bool>,
        rebalance_required: bool,
    ) -> Self {
        Self {
            signal: None,
            specific_signal: None,
            daily_features: None,
            direction: None,
            strategy: None,
            parameters,
            position_open: true,
            strategy_enabled,
            signal_active,
            rebalance_required,
        }
    }

    fn dated_carry_entry(
        candidate: &'a DatedCarryCandidate,
        emitted_at_ms: u64,
        evaluated_at_ms: u64,
        parameters: &'a BTreeMap<String, f64>,
    ) -> Self {
        Self {
            signal: None,
            specific_signal: Some(StrategySpecificSignal::DatedCarryEntry {
                candidate, emitted_at_ms, evaluated_at_ms,
            }),
            daily_features: None,
            direction: None,
            strategy: Some(Strategy::DatedCarry),
            parameters,
            position_open: false,
            strategy_enabled: true,
            signal_active: None,
            rebalance_required: false,
        }
    }

    fn dated_carry_settlement(
        emitted_at_ms: u64,
        evaluated_at_ms: u64,
        spot_symbol: &'a str,
        future_symbol: &'a str,
        expiry_ms: u64,
        spot_exit_twap: f64,
        settlement_price: f64,
        settlement_source: Option<DatedCarrySettlementSource>,
        parameters: &'a BTreeMap<String, f64>,
    ) -> Self {
        Self {
            signal: None,
            specific_signal: Some(StrategySpecificSignal::DatedCarrySettlement {
                emitted_at_ms, evaluated_at_ms, spot_symbol, future_symbol,
                expiry_ms, spot_exit_twap, settlement_price, settlement_source,
            }),
            daily_features: None,
            direction: None,
            strategy: Some(Strategy::DatedCarry),
            parameters,
            position_open: true,
            strategy_enabled: true,
            signal_active: None,
            rebalance_required: false,
        }
    }

    /// Called by every strategy evaluator before checking its entry rule.
    fn position_decision(
        &self,
        exit_when_disabled: bool,
        exit_when_signal_inactive: bool,
    ) -> Option<StrategyDecision> {
        if !self.position_open { return None; }
        if !self.strategy_enabled && exit_when_disabled { return Some(StrategyDecision::Exit); }
        if exit_when_signal_inactive { return Some(StrategyDecision::Exit); }
        if self.rebalance_required { return Some(StrategyDecision::Rebalance); }
        Some(StrategyDecision::Hold)
    }

    fn scout_breakout_distance_percent(&self) -> Option<f64> {
        self.daily_features.and_then(|features| {
            features.observed_close.zip(features.primary_level).and_then(|(close, level)| {
                (close.is_finite() && close > 0.0 && level.is_finite() && level > 0.0)
                    .then_some((close / level - 1.0) * 100.0)
            })
        })
    }

    fn within_learned_breakout_entry_distance(&self, entry_distance_percent: f64) -> bool {
        self.daily_features.is_some_and(|features| {
            features.close_above(features.primary_level)
                || (entry_distance_percent > 0.0
                    && features.observed_close.zip(features.primary_level)
                        .is_some_and(|(close, level)| (close / level - 1.0) * 100.0 >= -entry_distance_percent))
        })
    }

    fn within_scout_breakout_buffer(&self) -> bool {
        self.scout_breakout_distance_percent()
            .is_some_and(|distance| (-SCOUT_BREAKOUT_BUFFER_PERCENT..=0.0).contains(&distance))
    }
}

#[cfg(test)]
mod risk_adjusted_monthly_momentum_tests {
    use super::*;

    fn candidate(anchor: u64) -> Signal {
        Signal {
            symbol: "ETH-USDT".into(),
            direction: "bullish".into(),
            strength: 1.0,
            timestamp: 10,
            stale: false,
            score: 1.0,
            ranging_probability: None,
            components: SignalComponents::default(),
            strategy_candidate: true,
            strategy: Some("risk-adjusted-monthly-momentum".into()),
            strategy_decision: Some(serde_json::json!({
                "observed_close": 120.0,
                "primary_level": 100.0,
                "secondary_level": null,
                "btc_gate_required": true,
                "btc_close": 120.0,
                "btc_sma": 100.0,
                "strategy_order_size_multiplier": 0.5,
                "monthly_anchor_close_time_ms": anchor,
                "cross_sectional_metric_value": 10.0,
                "cross_sectional_threshold": 5.0
            })),
            market_regime: None,
            news_attribution: None,
            entry_gate: None,
        }
    }

    #[test]
    fn same_symbol_rebalances_only_when_month_anchor_advances() {
        let snapshot = DailyStrategySnapshot {
            as_of_ms: 10,
            sweep_id: String::new(),
            active_strategy_symbols: BTreeSet::from([(
                "risk-adjusted-monthly-momentum".into(),
                "ETH-USDT".into(),
            )]),
            generated_signals: vec![candidate(2_000)],
            evaluated_strategy_symbols: BTreeSet::from([(
                "risk-adjusted-monthly-momentum".into(), "ETH-USDT".into(),
            )]),
            diagnostics: Vec::new(),
        };
        assert!(snapshot.strategy_rebalance_required(
            "risk-adjusted-monthly-momentum",
            "ETH-USDT",
            Some(&serde_json::json!({"monthly_anchor_close_time_ms": 1_000})),
        ));
        assert!(!snapshot.strategy_rebalance_required(
            "risk-adjusted-monthly-momentum",
            "ETH-USDT",
            Some(&serde_json::json!({"monthly_anchor_close_time_ms": 2_000})),
        ));
        assert!(!snapshot.strategy_rebalance_required(
            "relative-strength-90-sma200",
            "ETH-USDT",
            Some(&serde_json::json!({"monthly_anchor_close_time_ms": 1_000})),
        ));
    }
}
