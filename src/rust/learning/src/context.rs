use super::*;

pub(super) async fn context_thresholds(
    State(service): State<ServiceState>,
    Json(request): Json<ContextThresholdRequest>,
) -> Result<Json<ContextThresholdResponse>, (StatusCode, String)> {
    if request.learning.len() > 100_000
        || request.contexts.len() > 2_000
        || request
            .learning
            .values()
            .any(|stats| !stats.realized_pnl.is_finite())
        || request.missed_opportunity_bands.len() > 2_000
        || request
            .contexts
            .iter()
            .any(|input| input.context.is_empty() || input.context.len() > 256)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid context-threshold request".into(),
        ));
    }
    let now = now_ms();
    let mut state = service.state.lock().await;
    let mut imported = false;
    for (key, stats) in request.learning {
        if !state.context_learning.contains_key(&key) {
            state.context_learning.insert(key, stats);
            imported = true;
        }
    }
    if imported {
        persist(&service.path, &state)?;
    }
    let learning = state.context_learning.clone();
    drop(state);
    let context_inputs = if request.contexts.is_empty() {
        let mut contexts = BTreeMap::new();
        for key in learning.keys() {
            let Some(context) = key.strip_suffix(":trade") else {
                continue;
            };
            let context = context.strip_prefix("scout:").unwrap_or(context);
            if !context.is_empty() {
                contexts
                    .entry(context.to_owned())
                    .or_insert(ContextThresholdInput {
                        context: context.to_owned(),
                    });
            }
        }
        contexts.into_values().collect::<Vec<_>>()
    } else {
        request.contexts
    };
    let missed_opportunity_bands = request.missed_opportunity_bands;
    let thresholds = context_inputs
        .iter()
        .map(|input| {
            let trade_stats = context_stats_for_prefix(&learning, &input.context, now);
            let stats = entry_context_stats(&learning, &input.context, now);
            let scout_stats =
                context_stats_for_prefix(&learning, &format!("scout:{}", input.context), now);
            (
                input.context.clone(),
                ContextThresholds {
                    strength_floor: context_strength_floor(&stats),
                    exploration_score_floor: context_score_floor(&stats, &missed_opportunity_bands),
                    learning_samples: trade_stats.samples,
                    positive_rate: (trade_stats.samples > 0)
                        .then_some(trade_stats.positive as f64 / trade_stats.samples as f64),
                    scout_outcome_samples: scout_stats.samples,
                    scout_outcome_positive_rate: (scout_stats.samples > 0)
                        .then_some(scout_stats.positive as f64 / scout_stats.samples as f64),
                    scout_outcome_realized_pnl: scout_stats.realized_pnl,
                },
            )
        })
        .collect();
    Ok(Json(ContextThresholdResponse { thresholds }))
}

#[derive(Default)]
struct AggregatedContextStats {
    samples: usize,
    positive: usize,
    realized_pnl: f64,
}

fn entry_context_stats(
    learning: &BTreeMap<String, ContextLearningStat>,
    context: &str,
    now: u64,
) -> AggregatedContextStats {
    let mut result = context_stats_for_prefix(learning, context, now);
    let scout = context_stats_for_prefix(learning, &format!("scout:{context}"), now);
    result.samples = result.samples.saturating_add(scout.samples);
    result.positive = result.positive.saturating_add(scout.positive);
    result.realized_pnl += scout.realized_pnl;
    result
}

fn context_stats_for_prefix(
    learning: &BTreeMap<String, ContextLearningStat>,
    context: &str,
    now: u64,
) -> AggregatedContextStats {
    let prefix = format!("{context}:");
    let mut result = AggregatedContextStats::default();
    for (key, stored) in learning
        .iter()
        .filter(|(key, _)| key.ends_with(":trade") && key.starts_with(&prefix))
    {
        let _ = key;
        let mut stats = stored.clone();
        if stats.updated_at_ms != 0 {
            let elapsed = now.saturating_sub(stats.updated_at_ms);
            let weight = 0.5_f64.powf(elapsed as f64 / (7.0 * 24.0 * 60.0 * 60.0 * 1_000.0));
            stats.samples = (stats.samples as f64 * weight).round() as usize;
            stats.positive = ((stats.positive as f64 * weight).round() as usize).min(stats.samples);
            stats.realized_pnl *= weight;
        }
        result.samples = result.samples.saturating_add(stats.samples);
        result.positive = result.positive.saturating_add(stats.positive);
        result.realized_pnl += stats.realized_pnl;
    }
    result
}

fn context_strength_floor(stats: &AggregatedContextStats) -> f64 {
    const DEFAULT_FLOOR: f64 = 0.35;
    let win_rate = (stats.positive as f64 + 1.0) / (stats.samples as f64 + 2.0);
    if stats.samples >= 30 && stats.realized_pnl < 0.0 && win_rate < 0.25 {
        0.65
    } else if stats.samples >= 20 && stats.realized_pnl < 0.0 && win_rate < 0.45 {
        0.52
    } else if stats.samples >= 10 && stats.realized_pnl < 0.0 && win_rate < 0.45 {
        0.45
    } else {
        DEFAULT_FLOOR
    }
}

fn context_score_floor(
    stats: &AggregatedContextStats,
    bands: &[ContextMissedOpportunityBand],
) -> f64 {
    const DEFAULT_FLOOR: f64 = 0.30;
    let win_rate = (stats.positive as f64 + 1.0) / (stats.samples as f64 + 2.0);
    let learned_floor = if stats.samples >= 30 && stats.realized_pnl < 0.0 && win_rate < 0.25 {
        0.60
    } else if stats.samples >= 30 && stats.realized_pnl < 0.0 && win_rate < 0.30 {
        0.50
    } else if stats.samples >= 20 && stats.realized_pnl < 0.0 && win_rate < 0.45 {
        0.35
    } else {
        DEFAULT_FLOOR
    };
    let short_floor = bands
        .iter()
        .filter(|band| {
            band.gate == "score"
                && band.horizon_seconds == 300
                && band.threshold >= 0.20
                && band.threshold < learned_floor
                && band.stop_target_sample_count >= 20
                && band
                    .stop_target_average_net_return_percent
                    .is_some_and(|value| value > 0.0)
                && band
                    .stop_target_positive_net_rate
                    .is_some_and(|value| value >= 0.55)
        })
        .map(|band| band.threshold)
        .min_by(f64::total_cmp)
        .unwrap_or(learned_floor);
    let long_floor = bands
        .iter()
        .filter(|band| {
            band.gate == "score"
                && matches!(band.horizon_seconds, 900 | 3_600)
                && band.threshold >= 0.20
                && band.threshold < learned_floor
                && band.sample_count >= 20
                && band
                    .average_net_return_percent
                    .is_some_and(|value| value > 0.0)
                && band.positive_net_rate.is_some_and(|value| value >= 0.55)
                && band.stop_target_sample_count >= 20
                && band
                    .stop_target_average_net_return_percent
                    .is_some_and(|value| value > 0.0)
                && band
                    .stop_target_positive_net_rate
                    .is_some_and(|value| value >= 0.55)
                && band.time_to_profit_sample_count >= 20
                && band.time_to_profit_rate.is_some_and(|value| value >= 0.50)
                && band
                    .average_time_to_profit_seconds
                    .is_some_and(|value| value > 300.0 && value <= band.horizon_seconds as f64)
        })
        .map(|band| band.threshold)
        .min_by(f64::total_cmp)
        .unwrap_or(learned_floor);
    short_floor.min(long_floor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(path: &str) -> ServiceState {
        ServiceState {
            state: std::sync::Arc::new(tokio::sync::Mutex::new(LearningState::default())),
            path: std::env::temp_dir().join(format!(
                "learning-context-{path}-{}.sqlite",
                std::process::id()
            )),
        }
    }

    fn stat(samples: usize, positive: usize, pnl: f64, updated_at_ms: u64) -> ContextLearningStat {
        ContextLearningStat {
            samples,
            positive,
            realized_pnl: pnl,
            updated_at_ms,
        }
    }

    #[test]
    fn context_stats_match_only_trade_keys_under_the_requested_prefix() {
        let now = 10_000_000;
        let learning = BTreeMap::from([
            ("ctx:btc:trade".into(), stat(8, 5, 12.0, 0)),
            ("ctx:eth:trade".into(), stat(4, 1, -2.0, 0)),
            ("ctx:btc:other".into(), stat(100, 100, 100.0, 0)),
            ("ctx:btc:trade:extra".into(), stat(100, 100, 100.0, 0)),
        ]);
        let actual = context_stats_for_prefix(&learning, "ctx:btc", now);
        assert_eq!(actual.samples, 8);
        assert_eq!(actual.positive, 5);
        assert_eq!(actual.realized_pnl, 12.0);
    }

    #[tokio::test]
    async fn context_thresholds_import_stats_and_discover_trade_and_scout_contexts() {
        let service = service("discovery");
        let _ = std::fs::remove_file(&service.path);
        let request = ContextThresholdRequest {
            learning: BTreeMap::from([
                ("trend:trade".into(), stat(12, 8, 4.0, 0)),
                ("scout:trend:trade".into(), stat(6, 3, -1.0, 0)),
                ("scout:orphan:trade".into(), stat(2, 1, 0.5, 0)),
                ("not-a-trade".into(), stat(90, 90, 90.0, 0)),
            ]),
            contexts: Vec::new(),
            missed_opportunity_bands: Vec::new(),
        };

        let Json(response) = context_thresholds(State(service.clone()), Json(request))
            .await
            .unwrap();

        assert_eq!(response.thresholds.len(), 2);
        let trend = &response.thresholds["trend"];
        assert_eq!(trend.learning_samples, 12);
        assert_eq!(trend.positive_rate, Some(8.0 / 12.0));
        assert_eq!(trend.scout_outcome_samples, 6);
        assert_eq!(trend.scout_outcome_positive_rate, Some(0.5));
        assert_eq!(trend.scout_outcome_realized_pnl, -1.0);
        assert_eq!(response.thresholds["orphan"].learning_samples, 0);

        let state = load_state(&service.path).unwrap();
        assert_eq!(state.context_learning.len(), 4);
        let _ = std::fs::remove_file(&service.path);
    }

    #[tokio::test]
    async fn context_thresholds_apply_shared_missed_opportunity_bands_to_each_context() {
        let service = service("shared-bands");
        let _ = std::fs::remove_file(&service.path);
        let request = ContextThresholdRequest {
            learning: BTreeMap::new(),
            contexts: vec![
                ContextThresholdInput {
                    context: "trend:btc".into(),
                },
                ContextThresholdInput {
                    context: "trend:eth".into(),
                },
            ],
            missed_opportunity_bands: vec![ContextMissedOpportunityBand {
                gate: "score".into(),
                threshold: 0.25,
                horizon_seconds: 300,
                sample_count: 0,
                average_net_return_percent: None,
                positive_net_rate: None,
                time_to_profit_sample_count: 0,
                time_to_profit_rate: None,
                average_time_to_profit_seconds: None,
                stop_target_sample_count: 20,
                stop_target_average_net_return_percent: Some(1.0),
                stop_target_positive_net_rate: Some(0.6),
            }],
        };

        let Json(response) = context_thresholds(State(service.clone()), Json(request))
            .await
            .unwrap();

        assert_eq!(
            response.thresholds["trend:btc"].exploration_score_floor,
            0.25
        );
        assert_eq!(
            response.thresholds["trend:eth"].exploration_score_floor,
            0.25
        );
        let _ = std::fs::remove_file(&service.path);
    }

    #[tokio::test]
    async fn context_thresholds_reject_non_finite_stats_and_empty_contexts() {
        let service = service("invalid");
        let invalid_stats = ContextThresholdRequest {
            learning: BTreeMap::from([("ctx:trade".into(), stat(1, 0, f64::NAN, 0))]),
            contexts: Vec::new(),
            missed_opportunity_bands: Vec::new(),
        };
        let error = context_thresholds(State(service.clone()), Json(invalid_stats))
            .await
            .unwrap_err();
        assert_eq!(error.0, StatusCode::BAD_REQUEST);

        let invalid_context = ContextThresholdRequest {
            learning: BTreeMap::new(),
            contexts: vec![ContextThresholdInput {
                context: String::new(),
            }],
            missed_opportunity_bands: Vec::new(),
        };
        let error = context_thresholds(State(service), Json(invalid_context))
            .await
            .unwrap_err();
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn long_horizon_missed_opportunity_requires_all_return_and_timing_evidence() {
        let stats = AggregatedContextStats {
            samples: 30,
            positive: 2,
            realized_pnl: -10.0,
        };
        let acceptable = ContextMissedOpportunityBand {
            gate: "score".into(),
            horizon_seconds: 900,
            threshold: 0.25,
            sample_count: 24,
            average_net_return_percent: Some(0.4),
            positive_net_rate: Some(0.6),
            stop_target_sample_count: 24,
            stop_target_average_net_return_percent: Some(0.2),
            stop_target_positive_net_rate: Some(0.6),
            time_to_profit_sample_count: 24,
            time_to_profit_rate: Some(0.6),
            average_time_to_profit_seconds: Some(600.0),
        };
        assert_eq!(context_score_floor(&stats, &[acceptable.clone()]), 0.25);

        let mut too_slow = acceptable;
        too_slow.average_time_to_profit_seconds = Some(901.0);
        assert_eq!(context_score_floor(&stats, &[too_slow]), 0.60);
    }

    #[test]
    fn stale_context_stats_decay_and_entry_stats_include_scout_outcomes() {
        let now = 7 * 24 * 60 * 60 * 1_000;
        let learning = BTreeMap::from([
            ("ctx:btc:trade".into(), stat(10, 8, 20.0, 0)),
            ("scout:ctx:btc:trade".into(), stat(6, 2, -4.0, 0)),
        ]);
        let one_week_old = context_stats_for_prefix(&learning, "ctx:btc", now);
        assert_eq!(one_week_old.samples, 10); // each input halves
        let combined = entry_context_stats(&learning, "ctx:btc", 0);
        assert_eq!(combined.samples, 16);
        assert_eq!(combined.positive, 10);
        assert_eq!(combined.realized_pnl, 16.0);
    }

    #[test]
    fn context_strength_floor_uses_loss_sample_bands_and_defaults() {
        assert_eq!(
            context_strength_floor(&AggregatedContextStats::default()),
            0.35
        );
        assert_eq!(
            context_strength_floor(&AggregatedContextStats {
                samples: 10,
                positive: 0,
                realized_pnl: -1.0
            }),
            0.45
        );
        assert_eq!(
            context_strength_floor(&AggregatedContextStats {
                samples: 20,
                positive: 0,
                realized_pnl: -1.0
            }),
            0.52
        );
        assert_eq!(
            context_strength_floor(&AggregatedContextStats {
                samples: 30,
                positive: 0,
                realized_pnl: -1.0
            }),
            0.65
        );
    }

    #[test]
    fn context_score_floor_uses_learned_and_missed_opportunity_evidence() {
        let losing = AggregatedContextStats {
            samples: 30,
            positive: 0,
            realized_pnl: -1.0,
        };
        assert_eq!(
            context_score_floor(&AggregatedContextStats::default(), &[]),
            0.30
        );
        assert_eq!(context_score_floor(&losing, &[]), 0.60);
        let band = ContextMissedOpportunityBand {
            gate: "score".into(),
            threshold: 0.25,
            horizon_seconds: 300,
            sample_count: 0,
            average_net_return_percent: None,
            positive_net_rate: None,
            time_to_profit_sample_count: 0,
            time_to_profit_rate: None,
            average_time_to_profit_seconds: None,
            stop_target_sample_count: 20,
            stop_target_average_net_return_percent: Some(1.0),
            stop_target_positive_net_rate: Some(0.6),
        };
        assert_eq!(
            context_score_floor(&AggregatedContextStats::default(), &[band]),
            0.25
        );
    }
}
