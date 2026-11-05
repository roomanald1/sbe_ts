fn expand_daily_scout_candidates(candidates: &[ScoutPlanCandidate]) -> Vec<ScoutPlanCandidate> {
    const LEVELS: &[f64] = &[0.25, 0.5, 1.0, 2.0, 3.0, 5.0, 7.5, 10.0];
    let mut expanded = Vec::new();
    for candidate in candidates {
        let Some(policy) = candidate.daily_policy.as_ref() else {
            let mut candidate = candidate.clone();
            if let Some(policy) = candidate.live_policy.as_ref() {
                let fresh = policy.strategy_enabled
                    && policy
                        .book_updated_at_ms
                        .is_some_and(|updated| policy.now_ms.saturating_sub(updated) <= 5_000)
                    && policy
                        .best_bid
                        .zip(policy.best_ask)
                        .is_some_and(|(bid, ask)| {
                            bid.is_finite() && ask.is_finite() && bid > 0.0 && ask >= bid
                        });
                candidate.eligible &= fresh;
                if fresh {
                    let mid = policy.best_bid.unwrap()
                        + (policy.best_ask.unwrap() - policy.best_bid.unwrap()) / 2.0;
                    if let Some(template) = candidate.audit_templates.first_mut() {
                        template.base_audit["scout_entry_price"] = serde_json::json!(mid);
                        if let Some(decision) = template.base_audit.get_mut("strategy_decision") {
                            decision["observed_close"] = serde_json::json!(mid);
                        }
                    }
                }
            }
            expanded.push(candidate);
            continue;
        };
        let Some(template) = candidate.audit_templates.first() else {
            continue;
        };
        let strategy = template
            .base_audit
            .get("strategy")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let breakout_distance_percent = if strategy.starts_with("donchian-") {
            let decision = template.base_audit.get("strategy_decision");
            let Some((close, level)) = decision
                .and_then(|decision| decision.get("observed_close"))
                .and_then(serde_json::Value::as_f64)
                .zip(
                    decision
                        .and_then(|decision| decision.get("primary_level"))
                        .and_then(serde_json::Value::as_f64),
                )
            else {
                continue;
            };
            if !close.is_finite() || close <= 0.0 || !level.is_finite() || level <= 0.0 {
                continue;
            }
            (close / level - 1.0) * 100.0
        } else {
            0.0
        };
        let recordable = policy.parameter_key.is_some()
            || policy.evaluator_decision == "scout"
            || (policy.selected_scout && policy.evaluator_decision == "enter")
            || (policy.evaluator_decision == "reject"
                && policy.strategy_candidate
                && policy.daily_features_present
                && policy.independent_non_donchian_strategy);
        if !candidate.eligible || !recordable {
            continue;
        }
        let direct_entry = policy.selected_scout && policy.evaluator_decision == "enter";
        let levels = if direct_entry {
            vec![0.0]
        } else if let Some((key, value)) =
            policy.parameter_key.as_deref().zip(policy.parameter_value)
        {
            vec![if key == "breakout_entry_distance_percent" {
                value
            } else {
                0.0
            }]
        } else {
            LEVELS.to_vec()
        };
        let has_probe = policy.parameter_key.is_some();
        for level in levels.into_iter().filter(|level| {
            direct_entry
                || has_probe
                || (breakout_distance_percent <= 0.0 && breakout_distance_percent >= -*level)
        }) {
            let mut selected = candidate.clone();
            selected.daily_policy = None;
            selected.generate_group_id = true;
            selected.sample_group_id.clear();
            selected.cooldown_key.clear();
            selected.cooldown_ms = if has_probe { 86_400_000 } else { 300_000 };
            selected.profile_key = format!(
                "{}|{}",
                strategy,
                template
                    .base_audit
                    .get("scout_parameter_profile_symbol")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| template
                        .base_audit
                        .get("symbol")
                        .and_then(serde_json::Value::as_str))
                    .unwrap_or_default()
            );
            if let Some(first) = selected.audit_templates.first_mut() {
                first.level_percent = level;
                if let Some(context) = first
                    .base_audit
                    .get("market_context")
                    .and_then(serde_json::Value::as_str)
                {
                    first.learning_context = format!("scout:{context}:level:{level:.2}:scenario");
                }
            }
            expanded.push(selected);
        }
    }
    expanded
}

fn generated_scout_group_and_cooldown(
    candidate: &ScoutPlanCandidate,
) -> Option<(String, String)> {
    let template = candidate.audit_templates.first()?;
    let audit = &template.base_audit;
    let strategy = audit.get("strategy")?.as_str()?;
    let symbol = audit.get("symbol")?.as_str()?;
    let direction = audit.get("direction")?.as_str()?;
    let signal_time = audit
        .get("signal_timestamp")
        .and_then(serde_json::Value::as_u64)
        .or_else(|| audit.get("timestamp").and_then(serde_json::Value::as_u64))
        .unwrap_or_default();
    let decision = audit.get("strategy_decision");
    let probe_key = decision
        .and_then(|value| value.get("scout_parameter_key"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            audit
                .get("scout_comparison_parameter_key")
                .and_then(serde_json::Value::as_str)
        });
    let probe_value = decision
        .and_then(|value| value.get("scout_parameter_value"))
        .and_then(serde_json::Value::as_f64)
        .or_else(|| {
            audit
                .get("scout_comparison_parameter_value")
                .and_then(serde_json::Value::as_f64)
        });
    let probe = probe_key
        .zip(probe_value)
        .map(|(key, value)| format!("|probe:{}", probe_id(&format!("{key}:{value:.8}"))));
    let probe = probe.unwrap_or_default();
    let level = template.level_percent;
    let (group, cooldown) = if strategy.starts_with("live-") {
        if probe_key.is_some() || audit.get("scout_comparison_id").is_some() {
            let group = format!(
                "live-scout-parameter:{strategy}|{symbol}|{direction}|signal:{signal_time}{probe}"
            );
            let cooldown = format!("live-parameter|{strategy}|{symbol}|{direction}{probe}");
            (group, cooldown)
        } else {
            (
                format!("live-scout:{strategy}|{symbol}|{direction}|signal:{signal_time}"),
                format!("live|{strategy}|{symbol}|{direction}"),
            )
        }
    } else {
        let sweep = if probe_key.is_some() {
            format!("|sweep:{signal_time}")
        } else {
            String::new()
        };
        (
            format!("daily-scout-level:{level:.2}|{strategy}|{symbol}|{direction}{sweep}{probe}"),
            format!("daily|{strategy}|{symbol}|{direction}|level:{level:.2}{probe}"),
        )
    };
    Some((group, cooldown))
}

fn scout_comparison_changes_exit(candidate: &ScoutPlanCandidate) -> bool {
    let Some(baseline) = candidate.audit_templates.iter().find(|template| {
        template
            .base_audit
            .get("scout_comparison_role")
            .and_then(serde_json::Value::as_str)
            == Some("baseline")
    }) else {
        return true;
    };
    let Some(variant) = candidate.audit_templates.iter().find(|template| {
        template
            .base_audit
            .get("scout_comparison_role")
            .and_then(serde_json::Value::as_str)
            == Some("variant")
    }) else {
        return true;
    };
    if baseline.parameter_entry_eligible != Some(true)
        || variant.parameter_entry_eligible != Some(true)
    {
        return true;
    }
    let key = variant
        .base_audit
        .get("scout_comparison_parameter_key")
        .or_else(|| baseline.base_audit.get("scout_comparison_parameter_key"))
        .and_then(serde_json::Value::as_str);
    let Some(key) = key else {
        return true;
    };
    let keys = if key.starts_with("combo:") {
        let Some(parsed) = parse_combination(key, &variant.parameter_specs) else {
            return true;
        };
        parsed.into_keys().collect::<Vec<_>>()
    } else {
        vec![key.to_owned()]
    };
    keys.iter().any(|key| {
        matches!(
            key.as_str(),
            "stop_loss_percent"
                | "take_profit_percent"
                | "profit_protection_activation_percent"
                | "profit_protection_giveback_percent"
                | "profit_protection_minimum_lock_percent"
                | "minimum_hold_days"
                | "maximum_hold_days"
        )
    })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScoutAuditTemplate {
    pub base_audit: serde_json::Value,
    pub notional_usd: f64,
    pub learning_context: String,
    pub stop_loss_percent: f64,
    pub take_profit_percent: f64,
    pub level_percent: f64,
    #[serde(default)]
    pub strategy_only: bool,
    #[serde(default)]
    pub parameter_entry_eligible: Option<bool>,
    pub parameter_specs: Vec<ParameterSpec>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum ScoutSamplePayload {
    DatedCarryProbe(DatedCarryProbe),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DatedCarryProbe {
    pub sample_id: String,
    pub parameter_key: String,
    pub parameter_value: f64,
    pub opened_at_ms: u64,
    pub candidate: strategies::DatedCarryEntryCandidate,
    pub strategy: String,
    pub profile_symbol: String,
    pub parameters: BTreeMap<String, f64>,
    pub tested_parameters: BTreeMap<String, f64>,
    pub parameter_specs: Vec<ParameterSpec>,
    pub would_enter: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScoutLifecycleObserveRequest {
    pub request_id: String,
    pub now_ms: u64,
    pub candidates: Vec<strategies::DatedCarryEntryCandidate>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ScoutLifecycleObserveResponse {
    pub finalized_count: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScoutPlanRequest {
    pub request_id: String,
    pub now_ms: u64,
    pub max_active_batches: usize,
    pub max_active_variants_per_profile: usize,
    #[serde(default)]
    pub legacy_cooldowns: BTreeMap<String, u64>,
    #[serde(default)]
    pub legacy_comparisons: Vec<LegacyScoutComparison>,
    pub candidates: Vec<ScoutPlanCandidate>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LegacyScoutComparison {
    pub comparison_id: String,
    #[serde(default)]
    pub baseline_return_percent: Option<f64>,
    #[serde(default)]
    pub variant_return_percent: Option<f64>,
    #[serde(default)]
    pub learning_key: String,
    #[serde(default)]
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ScoutPlanResponse {
    pub audits: Vec<serde_json::Value>,
    pub selected_groups: Vec<String>,
}

pub async fn request_scout_plan(
    base_url: &str,
    request: &ScoutPlanRequest,
) -> Result<ScoutPlanResponse, String> {
    reqwest::Client::new()
        .post(format!("{}/scout-plan", base_url.trim_end_matches('/')))
        .timeout(std::time::Duration::from_secs(5))
        .json(request)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json()
        .await
        .map_err(|error| error.to_string())
}

pub async fn request_scout_lifecycle_observe(
    base_url: &str,
    request: &ScoutLifecycleObserveRequest,
) -> Result<ScoutLifecycleObserveResponse, String> {
    reqwest::Client::new()
        .post(format!(
            "{}/scout-lifecycle/observe",
            base_url.trim_end_matches('/')
        ))
        .timeout(std::time::Duration::from_secs(5))
        .json(request)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json()
        .await
        .map_err(|error| error.to_string())
}

#[derive(Clone)]
struct ScoutState {
    data: Arc<Mutex<PersistedState>>,
    path: PathBuf,
    flush_lock: Arc<Mutex<()>>,
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

