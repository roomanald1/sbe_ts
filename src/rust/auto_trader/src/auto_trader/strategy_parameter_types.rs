#[derive(Clone, Copy)]
struct StrategyParameterSpec {
    key: &'static str,
    label: &'static str,
    default: f64,
    min: f64,
    max: f64,
    step: f64,
    unit: &'static str,
    /// Positive means successful outcomes move this value upward; negative moves it downward.
    learning_direction: f64,
}

#[derive(Clone, Serialize)]
struct StrategyParameterValue {
    key: String,
    label: String,
    value: f64,
    learned_value: f64,
    configured_value: Option<f64>,
    default_value: f64,
    min: f64,
    max: f64,
    step: f64,
    unit: String,
    overridden: bool,
}

#[derive(Clone, Serialize)]
struct StrategyParameterProfile {
    symbol: String,
    strategy: String,
    learning_samples: usize,
    positive_rate: Option<f64>,
    parameters: Vec<StrategyParameterValue>,
}
