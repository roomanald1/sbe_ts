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
    #[serde(rename = "trend-sleeves-btc200")]
    TrendSleevesBtc200,
    #[serde(rename = "trend-50-200")]
    Trend50200,
    #[serde(rename = "relative-strength-90-sma200")]
    RelativeStrength90Sma200,
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
    evaluate: fn(&StrategyInsight<'_>) -> Result<Option<StrategyEvaluation>, String>,
    contract: fn() -> StrategyContract,
    should_exit_when_disabled: fn(bool) -> bool,
    should_exit_when_insight_inactive: Option<fn(bool) -> bool>,
    parameter_specs: fn() -> Vec<StrategyParameterSpec>,
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

pub(super) mod strategy_live_insights {
    pub(super) mod entry {
        include!("strategy_live_insights/entry.rs");
    }
    pub(super) mod data {
        include!("strategy_live_insights/data.rs");
    }
    pub(super) mod exit {
        include!("strategy_live_insights/exit.rs");
    }
    pub(super) mod definition {
        include!("strategy_live_insights/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_live_insights/parameters.rs");
    }
}
pub(super) mod strategy_live_momentum {
    pub(super) mod entry {
        include!("strategy_live_momentum/entry.rs");
    }
    pub(super) mod data {
        include!("strategy_live_momentum/data.rs");
    }
    pub(super) mod exit {
        include!("strategy_live_momentum/exit.rs");
    }
    pub(super) mod definition {
        include!("strategy_live_momentum/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_live_momentum/parameters.rs");
    }
}
pub(super) mod strategy_live_trade_flow {
    pub(super) mod entry {
        include!("strategy_live_trade_flow/entry.rs");
    }
    pub(super) mod data {
        include!("strategy_live_trade_flow/data.rs");
    }
    pub(super) mod exit {
        include!("strategy_live_trade_flow/exit.rs");
    }
    pub(super) mod definition {
        include!("strategy_live_trade_flow/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_live_trade_flow/parameters.rs");
    }
}
pub(super) mod strategy_live_order_book {
    pub(super) mod entry {
        include!("strategy_live_order_book/entry.rs");
    }
    pub(super) mod data {
        include!("strategy_live_order_book/data.rs");
    }
    pub(super) mod exit {
        include!("strategy_live_order_book/exit.rs");
    }
    pub(super) mod definition {
        include!("strategy_live_order_book/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_live_order_book/parameters.rs");
    }
}
pub(super) mod strategy_live_vwap_dislocation {
    pub(super) mod entry {
        include!("strategy_live_vwap_dislocation/entry.rs");
    }
    pub(super) mod data {
        include!("strategy_live_vwap_dislocation/data.rs");
    }
    pub(super) mod exit {
        include!("strategy_live_vwap_dislocation/exit.rs");
    }
    pub(super) mod definition {
        include!("strategy_live_vwap_dislocation/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_live_vwap_dislocation/parameters.rs");
    }
}
pub(super) mod strategy_live_absorption_pressure {
    pub(super) mod entry {
        include!("strategy_live_absorption_pressure/entry.rs");
    }
    pub(super) mod data {
        include!("strategy_live_absorption_pressure/data.rs");
    }
    pub(super) mod exit {
        include!("strategy_live_absorption_pressure/exit.rs");
    }
    pub(super) mod definition {
        include!("strategy_live_absorption_pressure/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_live_absorption_pressure/parameters.rs");
    }
}
pub(super) mod strategy_donchian_55_btc200 {
    pub(super) mod exit {
        include!("strategy_donchian_55_btc200/exit.rs");
    }
    pub(super) mod definition {
        include!("strategy_donchian_55_btc200/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_donchian_55_btc200/parameters.rs");
    }
}
pub(super) mod strategy_donchian_20_sma200 {
    pub(super) mod exit {
        include!("strategy_donchian_20_sma200/exit.rs");
    }
    pub(super) mod definition {
        include!("strategy_donchian_20_sma200/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_donchian_20_sma200/parameters.rs");
    }
}
pub(super) mod strategy_trend_sleeves_btc200 {
    pub(super) mod exit {
        include!("strategy_trend_sleeves_btc200/exit.rs");
    }
    pub(super) mod definition {
        include!("strategy_trend_sleeves_btc200/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_trend_sleeves_btc200/parameters.rs");
    }
}
pub(super) mod strategy_trend_50_200 {
    pub(super) mod exit {
        include!("strategy_trend_50_200/exit.rs");
    }
    pub(super) mod definition {
        include!("strategy_trend_50_200/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_trend_50_200/parameters.rs");
    }
}
pub(super) mod strategy_relative_strength_90_sma200 {
    pub(super) mod exit {
        include!("strategy_relative_strength_90_sma200/exit.rs");
    }
    pub(super) mod definition {
        include!("strategy_relative_strength_90_sma200/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_relative_strength_90_sma200/parameters.rs");
    }
}
pub(super) mod strategy_rsi2_oversold_rebound {
    pub(super) mod exit {
        include!("strategy_rsi2_oversold_rebound/exit.rs");
    }
    pub(super) mod definition {
        include!("strategy_rsi2_oversold_rebound/contract.rs");
    }
    pub(super) mod parameters {
        include!("strategy_rsi2_oversold_rebound/parameters.rs");
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
    fn evaluate_insight(
        self,
        insight: &StrategyInsight<'_>,
    ) -> Result<Option<StrategyEvaluation>, String> {
        (self.definition().evaluate)(insight)
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

}

fn insights_only_evaluate(_: &StrategyInsight<'_>) -> Result<Option<StrategyEvaluation>, String> {
    Ok(None)
}
fn dated_carry_contract() -> StrategyContract {
    StrategyContract {
        insight: "Insights publishes a paired dated-futures carry signal",
        entry: "Executable future bid exceeds spot ask by the cost budget near expiry",
        exit: "Spot TWAP over the 30-minute settlement window and futures cash settlement",
        entry_order: "Paired paper spot buy and dated-future short",
        exit_order: "Paper spot TWAP and future delivery",
        behavior: "Signal-driven, paired paper carry strategy",
    }
}
fn dated_carry_no_exit(_: bool) -> bool { false }
fn dated_carry_no_parameters() -> Vec<StrategyParameterSpec> { Vec::new() }
static DATED_CARRY_DEFINITION: StrategyDefinition = StrategyDefinition {
    strategy: Strategy::DatedCarry,
    name: "dated-futures-carry",
    live_insight: false,
    live_insight_fields: &[],
    legacy_composite: false,
    expands_to: &[],
    evaluate: insights_only_evaluate,
    contract: dated_carry_contract,
    should_exit_when_disabled: dated_carry_no_exit,
    should_exit_when_insight_inactive: None,
    parameter_specs: dated_carry_no_parameters,
};

static STRATEGY_DEFINITIONS: [&StrategyDefinition; 13] = [
    &strategy_live_insights::definition::DEFINITION,
    &strategy_live_momentum::definition::DEFINITION,
    &strategy_live_trade_flow::definition::DEFINITION,
    &strategy_live_order_book::definition::DEFINITION,
    &strategy_live_vwap_dislocation::definition::DEFINITION,
    &strategy_live_absorption_pressure::definition::DEFINITION,
    &strategy_donchian_55_btc200::definition::DEFINITION,
    &strategy_donchian_20_sma200::definition::DEFINITION,
    &strategy_trend_sleeves_btc200::definition::DEFINITION,
    &strategy_trend_50_200::definition::DEFINITION,
    &strategy_relative_strength_90_sma200::definition::DEFINITION,
    &strategy_rsi2_oversold_rebound::definition::DEFINITION,
    &DATED_CARRY_DEFINITION,
];
