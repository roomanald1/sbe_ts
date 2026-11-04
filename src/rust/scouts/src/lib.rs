//! Scout markout calculation, independent of order execution and learning state.

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use strategy_core::FeedbackEvent;
pub use strategy_core::ParameterSpec;
use tokio::sync::Mutex;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScoutSample {
    pub event_id: String,
    pub opened_at_ms: u64,
    pub symbol: String,
    pub strategy: String,
    pub profile_id: String,
    pub parameters: BTreeMap<String, f64>,
    pub specs: Vec<ParameterSpec>,
    pub entry_price: f64,
    pub side: i8,
}

fn scoutable_specs(specs: &[ParameterSpec]) -> impl Iterator<Item = &ParameterSpec> {
    specs.iter().filter(|spec| {
        !matches!(
            spec.key.as_str(),
            "learning_rate"
                | "scout_minimum_walk_forward_samples"
                | "scout_minimum_positive_net_rate"
        ) && spec.min < spec.max
            && spec.step > 0.0
    })
}

fn candidate_value(spec: &ParameterSpec, current: f64, direction: f64) -> Option<f64> {
    if !current.is_finite() || !spec.step.is_finite() || spec.step <= 0.0 {
        return None;
    }
    let preferred = current + direction * spec.step;
    let value = if preferred >= spec.min && preferred <= spec.max {
        preferred
    } else {
        current - direction * spec.step
    };
    (value >= spec.min && value <= spec.max && (value - current).abs() >= spec.step * 0.5)
        .then_some(value)
}

fn probe_hash(symbol: &str, strategy: &str) -> u64 {
    symbol
        .bytes()
        .chain(strategy.bytes())
        .fold(2_166_136_261_u64, |hash, byte| {
            hash.wrapping_mul(16_777_619) ^ u64::from(byte)
        })
}

pub fn probe_id(value: &str) -> String {
    let hash = value
        .bytes()
        .fold(14_695_981_039_346_656_037_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(1_099_511_628_211)
        });
    format!("{hash:016x}")
}

pub fn next_parameter_probe(
    symbol: &str,
    strategy: &str,
    parameters: &BTreeMap<String, f64>,
    specs: &[ParameterSpec],
    day: u64,
) -> Option<(String, f64)> {
    let candidates = scoutable_specs(specs)
        .filter_map(|spec| {
            let current = parameters.get(&spec.key).copied().unwrap_or(spec.default);
            candidate_value(spec, current, if day % 2 == 0 { 1.0 } else { -1.0 })
                .map(|value| (spec.key.clone(), value))
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return None;
    }
    candidates
        .get((day.wrapping_add(probe_hash(symbol, strategy)) % candidates.len() as u64) as usize)
        .cloned()
}

pub fn next_parameter_combination(
    symbol: &str,
    strategy: &str,
    parameters: &BTreeMap<String, f64>,
    specs: &[ParameterSpec],
    day: u64,
) -> Option<BTreeMap<String, f64>> {
    let candidates = scoutable_specs(specs)
        .filter_map(|spec| {
            let current = parameters.get(&spec.key).copied().unwrap_or(spec.default);
            candidate_value(spec, current, if (day >> 1) & 1 == 0 { 1.0 } else { -1.0 })
                .map(|value| (spec.key.clone(), value))
        })
        .collect::<Vec<_>>();
    if candidates.len() < 2 {
        return None;
    }
    let pair_count = candidates.len() * (candidates.len() - 1) / 2;
    let wanted = (day.wrapping_add(probe_hash(symbol, strategy)) % pair_count as u64) as usize;
    let mut index = 0;
    for left in 0..candidates.len() {
        for right in left + 1..candidates.len() {
            if index == wanted {
                return Some(BTreeMap::from([
                    candidates[left].clone(),
                    candidates[right].clone(),
                ]));
            }
            index += 1;
        }
    }
    None
}

/// Generate the complete one-step subset sweep for a profile. Each parameter
/// has one bounded neighboring value in this sweep; the returned variants are
/// every non-empty subset, from single-parameter tests through the full set.
/// The bound keeps a malformed or unexpectedly huge profile from allocating an
/// unbounded power set.
pub fn all_parameter_variants(
    parameters: &BTreeMap<String, f64>,
    specs: &[ParameterSpec],
    day: u64,
) -> Option<Vec<BTreeMap<String, f64>>> {
    const MAX_SWEEP_PARAMETERS: usize = 16;
    let direction = if day % 2 == 0 { 1.0 } else { -1.0 };
    let candidates = scoutable_specs(specs)
        .filter_map(|spec| {
            let current = parameters.get(&spec.key).copied().unwrap_or(spec.default);
            candidate_value(spec, current, direction).map(|value| (spec.key.clone(), value))
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() || candidates.len() > MAX_SWEEP_PARAMETERS {
        return None;
    }
    let variant_count = 1usize.checked_shl(candidates.len() as u32)? - 1;
    let mut variants = Vec::with_capacity(variant_count);
    for mask in 1..=variant_count {
        let mut values = BTreeMap::new();
        for (index, (key, value)) in candidates.iter().enumerate() {
            if mask & (1 << index) != 0 {
                values.insert(key.clone(), *value);
            }
        }
        variants.push(values);
    }
    Some(variants)
}

pub fn context_hash(
    parameters: &BTreeMap<String, f64>,
    excluded: &BTreeMap<String, f64>,
) -> String {
    let mut hash = 14_695_981_039_346_656_037_u64;
    for (key, value) in parameters
        .iter()
        .filter(|(key, _)| !excluded.contains_key(*key))
    {
        for byte in format!("{key}={value:.8};").bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(1_099_511_628_211);
        }
    }
    format!("{hash:016x}")
}

pub fn combination_key(
    values: &BTreeMap<String, f64>,
    full_profile: &BTreeMap<String, f64>,
) -> Option<String> {
    if values.len() < 2 || values.values().any(|value| !value.is_finite()) {
        return None;
    }
    let pair = values
        .iter()
        .map(|(key, value)| format!("{key}={value:.8}"))
        .collect::<Vec<_>>()
        .join(",");
    Some(format!(
        "combo:{pair}|ctx:{}",
        context_hash(full_profile, values)
    ))
}

pub fn combination_context(key: &str) -> Option<&str> {
    let (_, context) = key.strip_prefix("combo:")?.rsplit_once("|ctx:")?;
    (context.len() == 16 && context.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(context)
}

pub fn parse_combination(key: &str, specs: &[ParameterSpec]) -> Option<BTreeMap<String, f64>> {
    let body = key.strip_prefix("combo:")?.split_once("|ctx:")?.0;
    combination_context(key)?;
    let mut values = BTreeMap::new();
    for item in body.split(',') {
        let (name, raw) = item.split_once('=')?;
        let value = raw.parse::<f64>().ok()?;
        let spec = specs.iter().find(|spec| spec.key == name)?;
        if spec.key == "learning_rate" || !value.is_finite() || value < spec.min || value > spec.max
        {
            return None;
        }
        let step = (value - spec.min) / spec.step;
        if (step - step.round()).abs() > 1e-6 || values.insert(name.to_owned(), value).is_some() {
            return None;
        }
    }
    (values.len() >= 2).then_some(values)
}

#[derive(Clone)]
struct ServiceState {
    client: reqwest::Client,
    learning_url: String,
    seen: Arc<Mutex<std::collections::BTreeSet<String>>>,
}

pub fn router(learning_url: String) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/samples", post(finalize_sample))
        .route("/outcomes", post(forward_outcome))
        .with_state(ServiceState {
            client: reqwest::Client::new(),
            learning_url,
            seen: Arc::new(Mutex::new(std::collections::BTreeSet::new())),
        })
}

async fn finalize_sample(
    State(service): State<ServiceState>,
    Json(request): Json<FinalizeRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let event_id = request.sample.event_id.clone();
    if service.seen.lock().await.contains(&event_id) {
        return Ok(StatusCode::NO_CONTENT);
    }
    let event = request
        .sample
        .finalize(
            request.closed_at_ms,
            request.exit_price,
            request.cost_percent,
        )
        .ok_or((StatusCode::BAD_REQUEST, "invalid scout sample".into()))?;
    forward_event(&service, &event).await?;
    service.seen.lock().await.insert(event_id);
    Ok(StatusCode::NO_CONTENT)
}

async fn forward_outcome(
    State(service): State<ServiceState>,
    Json(event): Json<FeedbackEvent>,
) -> Result<StatusCode, (StatusCode, String)> {
    forward_event(&service, &event).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn forward_event(
    service: &ServiceState,
    event: &FeedbackEvent,
) -> Result<(), (StatusCode, String)> {
    service
        .client
        .post(format!(
            "{}/feedback",
            service.learning_url.trim_end_matches('/')
        ))
        .json(event)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map(|_| ())
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("learning service rejected Scout feedback: {error}"),
            )
        })
}

#[derive(Deserialize)]
struct FinalizeRequest {
    sample: ScoutSample,
    closed_at_ms: u64,
    exit_price: f64,
    cost_percent: f64,
}

impl ScoutSample {
    /// Finalize a shadow sample using the observed exit price and round-trip costs.
    pub fn finalize(
        self,
        closed_at_ms: u64,
        exit_price: f64,
        cost_percent: f64,
    ) -> Option<FeedbackEvent> {
        if self.entry_price <= 0.0
            || exit_price <= 0.0
            || !self.entry_price.is_finite()
            || !exit_price.is_finite()
            || !cost_percent.is_finite()
            || ![-1, 1].contains(&self.side)
            || closed_at_ms < self.opened_at_ms
        {
            return None;
        }
        let net_return_percent =
            ((exit_price / self.entry_price - 1.0) * f64::from(self.side) * 100.0) - cost_percent;
        Some(FeedbackEvent::ScoutOutcome {
            event_id: self.event_id,
            observed_at_ms: closed_at_ms,
            symbol: self.symbol,
            strategy: self.strategy,
            profile_id: self.profile_id,
            parameters: self.parameters,
            specs: self.specs,
            net_return_percent,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs() -> Vec<ParameterSpec> {
        vec![
            ParameterSpec {
                key: "minimum_momentum".into(),
                label: "Momentum".into(),
                default: 0.5,
                min: 0.0,
                max: 1.0,
                step: 0.1,
                unit: "ratio".into(),
                learning_direction: 1.0,
            },
            ParameterSpec {
                key: "lookback_days".into(),
                label: "Lookback".into(),
                default: 20.0,
                min: 10.0,
                max: 30.0,
                step: 1.0,
                unit: "days".into(),
                learning_direction: 1.0,
            },
            ParameterSpec {
                key: "learning_rate".into(),
                label: "Learning rate".into(),
                default: 0.05,
                min: 0.0,
                max: 0.25,
                step: 0.01,
                unit: "ratio".into(),
                learning_direction: 1.0,
            },
        ]
    }

    #[test]
    fn probe_generation_is_deterministic_bounded_and_excludes_meta_parameters() {
        let parameters = BTreeMap::from([
            ("minimum_momentum".into(), 1.0),
            ("lookback_days".into(), 10.0),
        ]);
        let first =
            next_parameter_probe("BTC-USDT", "live-momentum", &parameters, &specs(), 2).unwrap();
        assert_eq!(
            first,
            next_parameter_probe("BTC-USDT", "live-momentum", &parameters, &specs(), 2).unwrap()
        );
        assert_ne!(first.0, "learning_rate");
        assert!(
            specs()
                .iter()
                .find(|spec| spec.key == first.0)
                .is_some_and(|spec| first.1 >= spec.min && first.1 <= spec.max)
        );
        assert_eq!(
            next_parameter_combination("BTC-USDT", "live-momentum", &parameters, &specs(), 2)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn subset_sweep_includes_every_arity_without_waiting_for_later_days() {
        let mut parameter_specs = specs();
        parameter_specs.push(ParameterSpec {
            key: "take_profit_percent".into(),
            label: "Take profit".into(),
            default: 1.0,
            min: 0.1,
            max: 2.0,
            step: 0.1,
            unit: "%".into(),
            learning_direction: 1.0,
        });
        let parameters = BTreeMap::from([
            ("minimum_momentum".into(), 1.0),
            ("lookback_days".into(), 10.0),
            ("take_profit_percent".into(), 1.0),
        ]);
        let variants = all_parameter_variants(&parameters, &parameter_specs, 2).unwrap();
        assert_eq!(variants.len(), 7);
        let sizes = variants
            .iter()
            .map(BTreeMap::len)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(sizes, std::collections::BTreeSet::from([1, 2, 3]));
        assert!(variants.iter().any(|variant| variant.len() == 3));
        for variant in variants {
            for (key, value) in variant {
                let spec = parameter_specs.iter().find(|spec| spec.key == key).unwrap();
                assert!((value - parameters[&key]).abs() <= spec.step + 1e-9);
                assert!(value >= spec.min && value <= spec.max);
            }
        }
    }

    #[test]
    fn ten_parameters_produce_the_entire_one_step_subset_sweep() {
        let specs = (0..10)
            .map(|index| ParameterSpec {
                key: format!("parameter_{index}"),
                label: format!("Parameter {index}"),
                default: 5.0,
                min: 0.0,
                max: 10.0,
                step: 1.0,
                unit: "units".into(),
                learning_direction: 1.0,
            })
            .collect::<Vec<_>>();
        let parameters = specs
            .iter()
            .map(|spec| (spec.key.clone(), spec.default))
            .collect::<BTreeMap<_, _>>();

        let variants = all_parameter_variants(&parameters, &specs, 0).unwrap();
        assert_eq!(variants.len(), 1_023);
        assert!(variants.iter().any(|variant| variant.len() == 1));
        assert!(variants.iter().any(|variant| variant.len() == 2));
        assert!(variants.iter().any(|variant| variant.len() == 10));
    }

    #[test]
    fn paired_probe_key_round_trips_with_variant_bounds() {
        let values = BTreeMap::from([
            ("minimum_momentum".into(), 0.6),
            ("lookback_days".into(), 21.0),
        ]);
        let profile = BTreeMap::from([
            ("minimum_momentum".into(), 0.5),
            ("lookback_days".into(), 20.0),
        ]);
        let key = combination_key(&values, &profile).unwrap();
        assert_eq!(parse_combination(&key, &specs()).unwrap(), values);
        assert!(combination_context(&key).is_some());
    }

    #[test]
    fn sample_preserves_profile_and_computes_net_return() {
        let sample = ScoutSample {
            event_id: "scout-1".into(),
            opened_at_ms: 10,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "profile-7".into(),
            parameters: BTreeMap::from([("minimum_momentum".into(), 0.4)]),
            specs: vec![],
            entry_price: 100.0,
            side: 1,
        };
        let event = sample.finalize(20, 102.0, 0.5).unwrap();
        let FeedbackEvent::ScoutOutcome {
            event_id,
            profile_id,
            parameters,
            net_return_percent,
            ..
        } = event
        else {
            panic!("wrong event")
        };
        assert_eq!(profile_id, "profile-7");
        assert_eq!(event_id, "scout-1");
        assert_eq!(parameters["minimum_momentum"], 0.4);
        assert!((net_return_percent - 1.5).abs() < 1e-12);
    }
}
