use crate::{Signal, Strategy, StrategyDecision, StrategySignal};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Deserialize, Serialize)]
pub struct StrategyDiagnostic {
    pub strategy: String,
    pub as_of_ms: u64,
    pub btc_close: Option<f64>,
    pub btc_sma: Option<f64>,
    pub btc_distance_percent: Option<f64>,
    pub btc_gate_required: bool,
    pub btc_gate_passed: Option<bool>,
    pub btc_window_days: Option<usize>,
    pub symbols: Vec<StrategySymbolDiagnostic>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct StrategySymbolDiagnostic {
    pub symbol: String,
    pub close: f64,
    pub eligible: bool,
    pub primary_rule: String,
    pub primary_level: f64,
    pub primary_distance_percent: f64,
    pub primary_metric_value: Option<f64>,
    pub primary_metric_unit: Option<String>,
    pub primary_passed: bool,
    pub secondary_rule: Option<String>,
    pub secondary_level: Option<f64>,
    pub secondary_distance_percent: Option<f64>,
    pub secondary_passed: Option<bool>,
    #[serde(default)]
    pub evaluated: bool,
    #[serde(default)]
    pub signal_active: bool,
    pub leader_gap_percent: Option<f64>,
    pub relative_strength_gap_pp: Option<f64>,
    pub rsi_exit_level: Option<f64>,
    pub maximum_hold_days: Option<f64>,
    pub trend_return_windows: Option<Vec<TrendReturnWindow>>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct TrendReturnWindow {
    pub days: usize,
    pub reference_close: f64,
    pub return_percent: f64,
}

#[derive(Clone, Deserialize)]
pub struct DailyStrategySnapshot {
    pub as_of_ms: u64,
    #[serde(default)]
    pub sweep_id: String,
    pub active_strategy_symbols: BTreeSet<(String, String)>,
    pub generated_signals: Vec<Signal>,
    // A symbol is inactive only when the latest batch explicitly evaluated it.
    // Missing symbols are unknown (for example, if a batch is partial).
    #[serde(skip)]
    pub evaluated_strategy_symbols: BTreeSet<(String, String)>,
    #[serde(skip)]
    pub diagnostics: Vec<StrategyDiagnostic>,
}

impl DailyStrategySnapshot {
    pub fn evaluate_position(
        &self,
        strategy: &str,
        symbol: &str,
        strategy_enabled: bool,
        previous_decision: Option<&serde_json::Value>,
        minimum_hold_active: bool,
    ) -> StrategyDecision {
        let Some(strategy_kind) = Strategy::from_name(strategy) else {
            return if strategy_enabled {
                StrategyDecision::Hold
            } else {
                StrategyDecision::Exit
            };
        };
        let signal_active = self.signal_active(strategy, symbol);
        let rebalance = self.strategy_rebalance_required(strategy, symbol, previous_decision);
        let parameters = previous_decision
            .and_then(|decision| decision.get("strategy_parameters"))
            .and_then(serde_json::Value::as_object)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|(key, value)| value.as_f64().map(|value| (key.clone(), value)))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        let daily_features = self
            .generated_signals
            .iter()
            .find(|signal| signal.strategy.as_deref() == Some(strategy) && signal.symbol == symbol)
            .and_then(|signal| signal.strategy_decision.as_ref())
            .and_then(|decision| serde_json::from_value(decision.clone()).ok());
        let mut input = StrategySignal::position(
            &parameters,
            strategy_enabled,
            if minimum_hold_active {
                None
            } else {
                signal_active
            },
            rebalance && !minimum_hold_active,
        );
        input.strategy = Some(strategy_kind);
        input.daily_features = daily_features;
        strategy_kind
            .evaluate(&input)
            .unwrap_or(StrategyDecision::Hold)
    }

    pub fn signal_active(&self, strategy: &str, symbol: &str) -> Option<bool> {
        let key = (strategy.to_owned(), symbol.to_owned());
        self.evaluated_strategy_symbols
            .contains(&key)
            .then(|| self.active_strategy_symbols.contains(&key))
    }

    pub fn symbol_diagnostic(&self, strategy: &str, symbol: &str) -> Option<serde_json::Value> {
        self.diagnostics
            .iter()
            .find(|diagnostic| diagnostic.strategy == strategy)
            .and_then(|diagnostic| diagnostic.symbols.iter().find(|item| item.symbol == symbol))
            .and_then(|item| serde_json::to_value(item).ok())
    }

    pub fn strategy_rebalance_required(
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
            let current_anchor = signal
                .strategy_decision
                .as_ref()
                .and_then(|decision| decision.get("monthly_anchor_close_time_ms"))
                .and_then(serde_json::Value::as_u64);
            current_anchor.is_some_and(|anchor| anchor > previous_anchor)
                && Strategy::RiskAdjustedMonthlyMomentum
                    .evaluate(&StrategySignal::entry(signal, &parameters))
                    .is_ok_and(|decision| decision == StrategyDecision::Enter)
        })
    }
}

#[cfg(test)]
mod risk_adjusted_monthly_momentum_tests {
    use super::*;
    use crate::SignalComponents;

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
                "risk-adjusted-monthly-momentum".into(),
                "ETH-USDT".into(),
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

#[derive(Deserialize)]
pub struct DailyStrategySignalBatch {
    pub snapshot: DailyStrategySnapshot,
    pub diagnostics: Vec<StrategyDiagnostic>,
}
