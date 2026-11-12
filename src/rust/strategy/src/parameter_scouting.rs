use std::collections::BTreeMap;

use crate::ParameterSpec;

pub(crate) fn scoutable_specs(specs: &[ParameterSpec]) -> impl Iterator<Item = &ParameterSpec> {
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

pub(crate) fn candidate_value(spec: &ParameterSpec, current: f64, direction: f64) -> Option<f64> {
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

/// Generate bounded probes in both directions for each tunable parameter
/// without expanding the profile into a combinatorial power set. Daily
/// parameter evidence is learned per parameter, so combinations do not
/// contribute to its promotion results.
pub fn single_parameter_variants(
    parameters: &BTreeMap<String, f64>,
    specs: &[ParameterSpec],
    day: u64,
) -> Option<Vec<BTreeMap<String, f64>>> {
    const MAX_SWEEP_VARIANTS: usize = 64;
    let first_direction = if day % 2 == 0 { 1.0 } else { -1.0 };
    let candidates = scoutable_specs(specs)
        .flat_map(|spec| {
            let current = parameters.get(&spec.key).copied().unwrap_or(spec.default);
            [first_direction, -first_direction]
                .into_iter()
                .filter_map(move |direction| {
                    candidate_value(spec, current, direction)
                        .map(|value| BTreeMap::from([(spec.key.clone(), value)]))
                })
        })
        .collect::<Vec<_>>();
    (!candidates.is_empty() && candidates.len() <= MAX_SWEEP_VARIANTS).then_some(candidates)
}

pub(crate) fn probe_hash(symbol: &str, strategy: &str) -> u64 {
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

pub(crate) fn context_hash(
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

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct StrategyEvidenceCandidate {
    pub symbol: String,
    pub strategy: String,
    pub signal_timestamp_ms: u64,
    pub parameters: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ParameterVariantsRequest {
    pub parameters: BTreeMap<String, f64>,
    pub specs: Vec<ParameterSpec>,
    pub day: u64,
    #[serde(default)]
    pub rejected_values: BTreeMap<String, Vec<f64>>,
}
