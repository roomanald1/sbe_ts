#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum AutoOrderType {
    Market,
    Ioc,
    Fok,
    Limit,
    Gtd,
    PostOnlyLimit,
    Barrier,
    StopMarket,
    StopLimit,
    TrailingStop,
    Vwap,
    Twap,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Strategy {
    #[serde(rename = "live-insights")]
    LiveInsights,
    #[serde(rename = "live-momentum")]
    LiveMomentum,
    #[serde(rename = "live-trade-flow")]
    LiveTradeFlow,
    #[serde(rename = "live-order-book")]
    LiveOrderBook,
    #[serde(rename = "live-vwap-dislocation")]
    LiveVwapDislocation,
    #[serde(rename = "live-absorption-pressure")]
    LiveAbsorptionPressure,
    #[default]
    #[serde(rename = "donchian-55-btc200")]
    Donchian55Btc200,
    #[serde(rename = "donchian-20-sma200")]
    Donchian20Sma200,
    #[serde(rename = "donchian-20-btc200")]
    Donchian20Btc200,
    #[serde(rename = "donchian-10-btc200")]
    Donchian10Btc200,
    #[serde(rename = "donchian-20-btc200-hold3")]
    Donchian20Btc200Hold3,
    #[serde(rename = "risk-adjusted-trend-portfolio")]
    RiskAdjustedTrendPortfolio,
    #[serde(rename = "trend-sleeves-btc200")]
    TrendSleevesBtc200,
    #[serde(rename = "trend-50-200")]
    Trend50200,
    #[serde(rename = "relative-strength-90-sma200")]
    RelativeStrength90Sma200,
    #[serde(rename = "risk-adjusted-monthly-momentum")]
    RiskAdjustedMonthlyMomentum,
    #[serde(rename = "rsi2-oversold-rebound")]
    Rsi2OversoldRebound,
    #[serde(rename = "dated-futures-carry")]
    DatedCarry,
}

struct StrategyDefinition {
    strategy: Strategy,
    name: &'static str,
    live_insight: bool,
    live_insight_fields: &'static [LiveInsightField],
    legacy_composite: bool,
    expands_to: &'static [Strategy],
    evaluate: fn(&StrategySignal<'_>) -> Result<StrategyDecision, String>,
    contract: fn() -> StrategyContract,
    should_exit_when_disabled: fn(bool) -> bool,
    should_exit_when_insight_inactive: Option<fn(bool) -> bool>,
    parameter_specs: fn() -> Vec<StrategyParameterSpec>,
}

fn no_strategy_parameters() -> Vec<StrategyParameterSpec> {
    Vec::new()
}

#[derive(Serialize)]
struct StrategyContract {
    insight: &'static str,
    entry: &'static str,
    exit: &'static str,
    entry_order: &'static str,
    exit_order: &'static str,
    behavior: &'static str,
}

pub(super) mod strategy_live_momentum {
    pub(super) mod evaluate {
        include!("strategy_live_momentum/evaluate.rs");
    }
    pub(super) mod data {
        include!("strategy_live_momentum/data.rs");
    }
    pub(super) mod definition {
        include!("strategy_live_momentum/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_live_momentum/parameters.rs");
    }
}
pub(super) mod strategy_live_trade_flow {
    pub(super) mod evaluate {
        include!("strategy_live_trade_flow/evaluate.rs");
    }
    pub(super) mod data {
        include!("strategy_live_trade_flow/data.rs");
    }
    pub(super) mod definition {
        include!("strategy_live_trade_flow/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_live_trade_flow/parameters.rs");
    }
}
pub(super) mod strategy_live_order_book {
    pub(super) mod evaluate {
        include!("strategy_live_order_book/evaluate.rs");
    }
    pub(super) mod data {
        include!("strategy_live_order_book/data.rs");
    }
    pub(super) mod definition {
        include!("strategy_live_order_book/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_live_order_book/parameters.rs");
    }
}
pub(super) mod strategy_live_vwap_dislocation {
    pub(super) mod evaluate {
        include!("strategy_live_vwap_dislocation/evaluate.rs");
    }
    pub(super) mod data {
        include!("strategy_live_vwap_dislocation/data.rs");
    }
    pub(super) mod definition {
        include!("strategy_live_vwap_dislocation/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_live_vwap_dislocation/parameters.rs");
    }
}
pub(super) mod strategy_live_absorption_pressure {
    pub(super) mod evaluate {
        include!("strategy_live_absorption_pressure/evaluate.rs");
    }
    pub(super) mod data {
        include!("strategy_live_absorption_pressure/data.rs");
    }
    pub(super) mod definition {
        include!("strategy_live_absorption_pressure/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_live_absorption_pressure/parameters.rs");
    }
}
pub(super) mod strategy_donchian_55_btc200 {
    pub(super) mod evaluate {
        include!("strategy_donchian_55_btc200/evaluate.rs");
    }
    pub(super) mod definition {
        include!("strategy_donchian_55_btc200/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_donchian_55_btc200/parameters.rs");
    }
}
pub(super) mod strategy_donchian_20_sma200 {
    pub(super) mod evaluate {
        include!("strategy_donchian_20_sma200/evaluate.rs");
    }
    pub(super) mod definition {
        include!("strategy_donchian_20_sma200/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_donchian_20_sma200/parameters.rs");
    }
}
pub(super) mod strategy_donchian_20_btc200 {
    pub(super) mod evaluate {
        include!("strategy_donchian_20_btc200/evaluate.rs");
    }
    pub(super) mod definition {
        include!("strategy_donchian_20_btc200/contract.rs");
    }
}
pub(super) mod strategy_donchian_10_btc200 {
    pub(super) mod evaluate {
        include!("strategy_donchian_10_btc200/evaluate.rs");
    }
    pub(super) mod definition {
        include!("strategy_donchian_10_btc200/contract.rs");
    }
}
pub(super) mod strategy_donchian_20_btc200_hold3 {
    pub(super) mod evaluate {
        include!("strategy_donchian_20_btc200_hold3/evaluate.rs");
    }
    pub(super) mod definition {
        include!("strategy_donchian_20_btc200_hold3/contract.rs");
    }
}
pub(super) mod strategy_risk_adjusted_trend_portfolio {
    pub(super) mod evaluate {
        include!("strategy_risk_adjusted_trend_portfolio/evaluate.rs");
    }
    pub(super) mod definition {
        include!("strategy_risk_adjusted_trend_portfolio/contract.rs");
    }
}
pub(super) mod strategy_trend_sleeves_btc200 {
    pub(super) mod evaluate {
        include!("strategy_trend_sleeves_btc200/evaluate.rs");
    }
    pub(super) mod definition {
        include!("strategy_trend_sleeves_btc200/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_trend_sleeves_btc200/parameters.rs");
    }
}
pub(super) mod strategy_trend_50_200 {
    pub(super) mod evaluate {
        include!("strategy_trend_50_200/evaluate.rs");
    }
    pub(super) mod definition {
        include!("strategy_trend_50_200/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_trend_50_200/parameters.rs");
    }
}
pub(super) mod strategy_relative_strength_90_sma200 {
    pub(super) mod evaluate {
        include!("strategy_relative_strength_90_sma200/evaluate.rs");
    }
    pub(super) mod definition {
        include!("strategy_relative_strength_90_sma200/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_relative_strength_90_sma200/parameters.rs");
    }
}
pub(super) mod strategy_risk_adjusted_monthly_momentum {
    pub(super) mod evaluate {
        include!("strategy_risk_adjusted_monthly_momentum/evaluate.rs");
    }
    pub(super) mod definition {
        include!("strategy_risk_adjusted_monthly_momentum/contract.rs");
    }
}
pub(super) mod strategy_rsi2_oversold_rebound {
    pub(super) mod evaluate {
        include!("strategy_rsi2_oversold_rebound/evaluate.rs");
    }
    pub(super) mod definition {
        include!("strategy_rsi2_oversold_rebound/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_rsi2_oversold_rebound/parameters.rs");
    }
}
pub(super) mod strategy_dated_futures_carry {
    pub(super) mod evaluate {
        include!("strategy_dated_futures_carry/evaluate.rs");
    }
    pub(super) mod definition {
        include!("strategy_dated_futures_carry/contract.rs");
    }
}
impl Strategy {
    fn all() -> impl Iterator<Item = Strategy> {
        STRATEGY_DEFINITIONS
            .iter()
            .map(|definition| definition.strategy)
    }
    fn live_strategies() -> impl Iterator<Item = Strategy> {
        Self::all().filter(|strategy| strategy.is_live_insight())
    }
    fn definition(self) -> &'static StrategyDefinition {
        STRATEGY_DEFINITIONS
            .iter()
            .find(|definition| definition.strategy == self)
            .expect("every Strategy variant must have a strategy definition")
    }
    fn as_str(self) -> &'static str {
        self.definition().name
    }
    fn from_name(name: &str) -> Option<Self> {
        STRATEGY_DEFINITIONS
            .iter()
            .find(|definition| definition.name == name)
            .map(|definition| definition.strategy)
    }
    fn is_dated_carry(self) -> bool {
        matches!(self, Self::DatedCarry)
    }
    fn is_live_insight(self) -> bool {
        self.definition().live_insight
    }
    fn receives_live_insight(self, signal: &Signal) -> bool {
        self.definition()
            .live_insight_fields
            .iter()
            .any(|field| field.is_present(signal))
    }
    fn any_live_strategy_receives(signal: &Signal) -> bool {
        STRATEGY_DEFINITIONS.iter().any(|definition| {
            definition
                .live_insight_fields
                .iter()
                .any(|field| field.is_present(signal))
        })
    }
    fn is_legacy_composite(self) -> bool {
        self.definition().legacy_composite
    }
    fn parameter_specs(self) -> Vec<StrategyParameterSpec> {
        (self.definition().parameter_specs)()
    }
    fn expanded_strategies(self) -> &'static [Strategy] {
        self.definition().expands_to
    }
    /// Entry and exit rules for this strategy are evaluated from signal context.
    fn evaluate(self, input: &StrategySignal<'_>) -> Result<StrategyDecision, String> {
        (self.definition().evaluate)(input)
    }
    fn contract(self) -> StrategyContract {
        (self.definition().contract)()
    }
    fn should_exit_when_disabled(self, enabled: bool) -> bool {
        (self.definition().should_exit_when_disabled)(enabled)
    }
    fn should_exit_when_insight_inactive(self, active: bool) -> bool {
        self.definition()
            .should_exit_when_insight_inactive
            .is_some_and(|evaluate_exit| evaluate_exit(active))
    }

    fn uses_protective_stops(self) -> bool {
        match self {
            Self::Donchian20Btc200 => {
                strategy_donchian_20_btc200::evaluate::uses_protective_stops()
            }
            Self::Donchian10Btc200 => {
                strategy_donchian_10_btc200::evaluate::uses_protective_stops()
            }
            Self::Donchian20Btc200Hold3 => {
                strategy_donchian_20_btc200_hold3::evaluate::uses_protective_stops()
            }
            Self::RiskAdjustedTrendPortfolio => {
                strategy_risk_adjusted_trend_portfolio::evaluate::uses_protective_stops()
            }
            Self::RiskAdjustedMonthlyMomentum => {
                strategy_risk_adjusted_monthly_momentum::evaluate::uses_protective_stops()
            }
            _ => true,
        }
    }

    fn uses_fixed_take_profit(self) -> bool {
        !matches!(
            self,
            Self::Donchian55Btc200
                | Self::Donchian20Btc200
                | Self::Donchian10Btc200
                | Self::Donchian20Btc200Hold3
                | Self::RiskAdjustedTrendPortfolio
                | Self::RiskAdjustedMonthlyMomentum
        )
    }
}

fn legacy_live_insights_contract() -> StrategyContract {
    StrategyContract {
        insight: "legacy live composite market insight",
        entry: "legacy stored selection; migrates to the enabled live component strategies",
        exit: "shared StopMarket protective stop and IOC stop/target exits; disabling this strategy closes its attributed positions",
        entry_order: "passive PostOnlyLimit by default; IOC is reserved for high urgency; VWAP/TWAP use configured duration and participation",
        exit_order: "StopMarket protective order; IOC close order for strategy and take-profit exits",
        behavior: "legacy configuration compatibility",
    }
}
fn legacy_live_insights_entry(input: &StrategySignal<'_>) -> Result<StrategyDecision, String> {
    if let Some(decision) = input.position_decision(
        legacy_live_insights_exit_when_disabled(input.strategy_enabled),
        false,
    ) {
        return Ok(decision);
    }
    if input.signal.is_some() {
        Ok(StrategyDecision::Enter)
    } else {
        Ok(StrategyDecision::Reject)
    }
}
fn legacy_live_insights_exit_when_disabled(enabled: bool) -> bool {
    !enabled
}
fn legacy_live_insights_parameters() -> Vec<StrategyParameterSpec> {
    vec![
        param(
            StrategyParameterKey::StopLossPercent,
            "Protective stop distance",
            0.50,
            0.10,
            1.50,
            0.05,
            "%",
            -1.0,
        ),
        param(
            StrategyParameterKey::TakeProfitPercent,
            "Protective target distance",
            0.80,
            0.20,
            3.00,
            0.05,
            "%",
            1.0,
        ),
    ]
}
static LEGACY_LIVE_INSIGHTS_DEFINITION: StrategyDefinition = StrategyDefinition {
    strategy: Strategy::LiveInsights,
    name: "live-insights",
    live_insight: true,
    live_insight_fields: &[
        LiveInsightField::Momentum10s,
        LiveInsightField::Trend5m,
        LiveInsightField::TradeFlow,
        LiveInsightField::OrderBook,
        LiveInsightField::VwapDislocation,
        LiveInsightField::AbsorptionPressure,
    ],
    legacy_composite: true,
    expands_to: &[
        Strategy::LiveMomentum,
        Strategy::LiveTradeFlow,
        Strategy::LiveOrderBook,
        Strategy::LiveVwapDislocation,
        Strategy::LiveAbsorptionPressure,
    ],
    evaluate: legacy_live_insights_entry,
    contract: legacy_live_insights_contract,
    should_exit_when_disabled: legacy_live_insights_exit_when_disabled,
    should_exit_when_insight_inactive: None,
    parameter_specs: legacy_live_insights_parameters,
};

static STRATEGY_DEFINITIONS: [&StrategyDefinition; 18] = [
    &LEGACY_LIVE_INSIGHTS_DEFINITION,
    &strategy_live_momentum::definition::DEFINITION,
    &strategy_live_trade_flow::definition::DEFINITION,
    &strategy_live_order_book::definition::DEFINITION,
    &strategy_live_vwap_dislocation::definition::DEFINITION,
    &strategy_live_absorption_pressure::definition::DEFINITION,
    &strategy_donchian_55_btc200::definition::DEFINITION,
    &strategy_donchian_20_sma200::definition::DEFINITION,
    &strategy_donchian_20_btc200::definition::DEFINITION,
    &strategy_donchian_10_btc200::definition::DEFINITION,
    &strategy_donchian_20_btc200_hold3::definition::DEFINITION,
    &strategy_risk_adjusted_trend_portfolio::definition::DEFINITION,
    &strategy_trend_sleeves_btc200::definition::DEFINITION,
    &strategy_trend_50_200::definition::DEFINITION,
    &strategy_relative_strength_90_sma200::definition::DEFINITION,
    &strategy_risk_adjusted_monthly_momentum::definition::DEFINITION,
    &strategy_rsi2_oversold_rebound::definition::DEFINITION,
    &strategy_dated_futures_carry::definition::DEFINITION,
];

#[cfg(test)]
mod donchian_20_btc200_registry_tests {
    use super::*;

    #[test]
    fn selected_breakout_exits_only_on_daily_invalidation() {
        let strategy = Strategy::from_name("donchian-20-btc200").unwrap();
        assert_eq!(strategy, Strategy::Donchian20Btc200);
        assert!(!strategy.should_exit_when_insight_inactive(true));
        assert!(strategy.should_exit_when_insight_inactive(false));
        assert!(strategy.should_exit_when_disabled(false));
        let mut snapshot = DailyStrategySnapshot {
            as_of_ms: 0,
            active_strategy_symbols: BTreeSet::new(),
            generated_signals: Vec::new(),
        };
        assert_eq!(
            snapshot.evaluate_position("donchian-20-btc200", "ETH-USDT", true, None, false),
            StrategyDecision::Exit
        );
        snapshot
            .active_strategy_symbols
            .insert(("donchian-20-btc200".into(), "ETH-USDT".into()));
        assert_eq!(
            snapshot.evaluate_position("donchian-20-btc200", "ETH-USDT", true, None, false),
            StrategyDecision::Hold
        );
    }
}

#[cfg(test)]
mod risk_adjusted_monthly_momentum_registry_tests {
    use super::*;

    #[test]
    fn strategy_is_registered_and_uses_signal_exit_without_fixed_stops() {
        let strategy = Strategy::from_name("risk-adjusted-monthly-momentum").unwrap();
        assert_eq!(strategy, Strategy::RiskAdjustedMonthlyMomentum);
        assert!(!strategy.uses_protective_stops());
        assert!(strategy.should_exit_when_insight_inactive(false));
        assert!(!strategy.should_exit_when_insight_inactive(true));
        assert!(strategy.should_exit_when_disabled(false));
    }
}

fn benchmark_signal(strategy: &str, candidate: bool) -> Signal {
    Signal {
        symbol: "BTC-USDT".into(),
        direction: "bullish".into(),
        strength: 1.0,
        timestamp: 1,
        stale: false,
        score: 1.0,
        ranging_probability: None,
        components: SignalComponents::default(),
        strategy_candidate: candidate,
        strategy: Some(strategy.into()),
        strategy_decision: Some(serde_json::json!({
            "observed_close": 120.0,
            "primary_level": 100.0,
            "secondary_level": 100.0,
            "btc_gate_required": true,
            "btc_close": 120.0,
            "btc_sma": 100.0,
            "primary_metric_value": 10.0,
            "strategy_order_size_multiplier": 0.5,
            "monthly_anchor_close_time_ms": 1,
            "cross_sectional_metric_value": 10.0,
            "cross_sectional_threshold": 5.0
        })),
        market_regime: None,
        news_attribution: None,
        entry_gate: None,
    }
}

/// Measure registered strategy entry and position evaluation paths.
pub fn benchmark_strategy_evaluation(iterations: usize) -> Vec<(String, f64, f64)> {
    use std::{hint::black_box, time::Instant};
    assert!(iterations > 0, "benchmark iterations must be positive");
    let parameters = BTreeMap::new();
    Strategy::all()
        .map(|strategy| {
            let signal = benchmark_signal(strategy.as_str(), true);
            let entry = StrategySignal::entry(&signal, &parameters);
            let started = Instant::now();
            for _ in 0..iterations {
                black_box(strategy.evaluate(black_box(&entry)).unwrap());
            }
            let entry_ns = started.elapsed().as_nanos() as f64 / iterations as f64;
            let position = StrategySignal::position(&parameters, true, Some(true), false);
            let started = Instant::now();
            for _ in 0..iterations {
                black_box(strategy.evaluate(black_box(&position)).unwrap());
            }
            let position_ns = started.elapsed().as_nanos() as f64 / iterations as f64;
            (strategy.as_str().to_owned(), entry_ns, position_ns)
        })
        .collect()
}

/// Measure the main non-strategy hot paths used while preparing auto-trader decisions.
pub fn benchmark_autotrader_paths(iterations: usize) -> Vec<(String, f64)> {
    use std::{hint::black_box, time::Instant};
    assert!(iterations > 0, "benchmark iterations must be positive");
    let signal = benchmark_signal("donchian-55-btc200", true);
    let parameters = ExecutionParameters::default();
    let signal_payload = serde_json::json!({
        "symbol": signal.symbol,
        "direction": signal.direction,
        "strength": signal.strength,
        "timestamp": signal.timestamp,
        "strategy": signal.strategy,
        "strategy_decision": signal.strategy_decision,
    });
    let market = serde_json::json!({
        "bids": (0..20).map(|i| serde_json::json!({"price": 100.0 - i as f64 * 0.01, "size": 1.0 + i as f64})).collect::<Vec<_>>(),
        "asks": (0..20).map(|i| serde_json::json!({"price": 100.02 + i as f64 * 0.01, "size": 1.5 + i as f64})).collect::<Vec<_>>()
    });
    let mut results = Vec::new();

    let started = Instant::now();
    for _ in 0..iterations {
        black_box(serde_json::to_vec(black_box(&signal_payload)).unwrap());
    }
    results.push(("signal serialization".into(), started.elapsed().as_nanos() as f64 / iterations as f64));

    let started = Instant::now();
    for _ in 0..iterations {
        black_box(assess_deep_book(black_box(&market), "BTC-USDT", true, 2_000.0, 10.0));
    }
    results.push(("market depth assessment".into(), started.elapsed().as_nanos() as f64 / iterations as f64));

    let started = Instant::now();
    for index in 0..iterations {
        black_box(build_order_request("BTC-USDT", "buy", 0.25, 100.0, 100.02, AutoOrderType::Limit, parameters, "bench-order", index as u64));
    }
    results.push(("limit order request construction".into(), started.elapsed().as_nanos() as f64 / iterations as f64));

    let overrides = BTreeMap::new();
    let started = Instant::now();
    for _ in 0..iterations {
        black_box(validate_strategy_parameter_overrides(black_box(&overrides)).unwrap());
    }
    results.push(("strategy parameter validation".into(), started.elapsed().as_nanos() as f64 / iterations as f64));
    results
}

#[cfg(test)]
mod strategy_signal_contract_tests {
    use super::*;

    #[test]
    fn every_daily_strategy_enters_only_from_its_own_benchmark_signal() {
        let daily = Strategy::all()
            .filter(|strategy| !strategy.is_live_insight() && !strategy.is_dated_carry());
        let mut checked = 0;
        for strategy in daily {
            checked += 1;
            let own_candidate = benchmark_signal(strategy.as_str(), true);
            let other_candidate = benchmark_signal("another-strategy", true);
            let inactive = benchmark_signal(strategy.as_str(), false);
            let parameters = BTreeMap::new();
            let evaluate = |signal| {
                strategy
                    .evaluate(&StrategySignal::entry(signal, &parameters))
                    .unwrap()
            };
            assert_eq!(evaluate(&own_candidate), StrategyDecision::Enter);
            assert_eq!(evaluate(&other_candidate), StrategyDecision::Reject);
            assert_eq!(evaluate(&inactive), StrategyDecision::Reject);
        }
        assert!(checked > 0, "registry must include daily strategies");
    }

    #[test]
    fn every_registered_strategy_returns_an_explicit_position_decision() {
        let parameters = BTreeMap::new();
        for strategy in Strategy::all() {
            let eval = |strategy_enabled, signal_active, rebalance_required| {
                strategy
                    .evaluate(&StrategySignal::position(
                        &parameters,
                        strategy_enabled,
                        signal_active,
                        rebalance_required,
                    ))
                    .unwrap()
            };
            let inactive = eval(true, Some(false), false);
            if !strategy.is_live_insight() && !strategy.is_dated_carry() {
                assert_eq!(inactive, StrategyDecision::Exit, "{}", strategy.as_str());
            } else {
                assert_eq!(inactive, StrategyDecision::Hold, "{}", strategy.as_str());
            }
            assert_eq!(
                eval(true, Some(true), false),
                StrategyDecision::Hold,
                "{}",
                strategy.as_str()
            );
            if strategy != Strategy::DatedCarry {
                assert_eq!(
                    eval(false, None, false),
                    StrategyDecision::Exit,
                    "{}",
                    strategy.as_str()
                );
            }
            if strategy == Strategy::RiskAdjustedMonthlyMomentum {
                assert_eq!(eval(true, Some(true), true), StrategyDecision::Rebalance);
            }
        }
    }

    #[test]
    fn every_live_strategy_evaluates_its_signal_and_rejects_missing_signal() {
        for strategy in Strategy::live_strategies() {
            let parameters = strategy
                .parameter_specs()
                .into_iter()
                .map(|spec| (spec.key.as_str().to_owned(), spec.default))
                .collect::<BTreeMap<_, _>>();
            let mut components = SignalComponents::default();
            match strategy {
                Strategy::LiveInsights => components.momentum_10s_percent = Some(0.1),
                Strategy::LiveMomentum => components.momentum_10s_percent = Some(0.1),
                Strategy::LiveTradeFlow => components.trade_flow_imbalance = Some(0.5),
                Strategy::LiveOrderBook => components.book_imbalance = Some(0.5),
                Strategy::LiveVwapDislocation => components.mispricing_vs_vwap_bps = Some(2.0),
                Strategy::LiveAbsorptionPressure => {
                    components.pressure_without_price_move = Some(-0.5)
                }
                _ => continue,
            }
            let signal = Signal {
                symbol: "BTC-USDT".into(),
                direction: "bullish".into(),
                strength: 1.0,
                timestamp: 1,
                stale: false,
                score: 1.0,
                ranging_probability: None,
                components,
                strategy_candidate: false,
                strategy: None,
                strategy_decision: None,
                market_regime: None,
                news_attribution: None,
                entry_gate: None,
            };
            let input = StrategySignal::entry(&signal, &parameters);
            assert_eq!(
                strategy.evaluate(&input).unwrap(),
                StrategyDecision::Enter,
                "{}",
                strategy.as_str()
            );
            let mut opposing = signal.clone();
            match strategy {
                Strategy::LiveMomentum => opposing.components.momentum_10s_percent = Some(-0.1),
                Strategy::LiveTradeFlow => opposing.components.trade_flow_imbalance = Some(-0.5),
                Strategy::LiveOrderBook => opposing.components.book_imbalance = Some(-0.5),
                Strategy::LiveVwapDislocation => {
                    opposing.components.mispricing_vs_vwap_bps = Some(-2.0)
                }
                Strategy::LiveAbsorptionPressure => {
                    opposing.components.pressure_without_price_move = Some(0.5)
                }
                Strategy::LiveInsights => {}
                _ => continue,
            }
            if strategy != Strategy::LiveInsights {
                assert_eq!(
                    strategy
                        .evaluate(&StrategySignal::entry(&opposing, &parameters))
                        .unwrap(),
                    StrategyDecision::Reject,
                    "{} accepted an opposing feature signal",
                    strategy.as_str()
                );
            }
            if strategy != Strategy::LiveInsights {
                let empty_parameters = BTreeMap::new();
                let missing_thresholds = StrategySignal::entry(&signal, &empty_parameters);
                assert_eq!(
                    strategy.evaluate(&missing_thresholds).unwrap(),
                    StrategyDecision::Reject,
                    "{} must fail closed when its parameter profile is missing",
                    strategy.as_str()
                );
            }
            let missing = StrategySignal::position(&parameters, true, None, false);
            assert_eq!(
                strategy.evaluate(&missing).unwrap(),
                StrategyDecision::Hold,
                "{}",
                strategy.as_str()
            );
        }
    }

    #[test]
    fn legacy_live_insights_parameter_bounds_are_enforced() {
        assert_parameter_bounds_for_strategy(Strategy::LiveInsights);
    }

    fn assert_parameter_bounds_for_strategy(strategy: Strategy) {
        for spec in all_strategy_parameter_specs(strategy) {
            let key = format!("BOUNDS:{}:{}", strategy.as_str(), spec.key.as_str());
            let mut values = BTreeMap::from([(key.clone(), spec.min)]);
            assert!(
                validate_strategy_parameter_overrides(&values).is_ok(),
                "{}:{} minimum",
                strategy.as_str(),
                spec.key.as_str()
            );
            values.insert(key.clone(), spec.max);
            assert!(
                validate_strategy_parameter_overrides(&values).is_ok(),
                "{}:{} maximum",
                strategy.as_str(),
                spec.key.as_str()
            );
            values.insert(key.clone(), spec.min - spec.step);
            assert!(
                validate_strategy_parameter_overrides(&values).is_err(),
                "{}:{} below minimum",
                strategy.as_str(),
                spec.key.as_str()
            );
            values.insert(key, spec.max + spec.step);
            assert!(
                validate_strategy_parameter_overrides(&values).is_err(),
                "{}:{} above maximum",
                strategy.as_str(),
                spec.key.as_str()
            );
        }
    }
}
