use anyhow::Result;
use axum::{Json, Router, http::StatusCode, routing::get};
use flate2::{Compression, read::MultiGzDecoder, write::GzEncoder};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, mpsc, oneshot};

const PRICE_WINDOW_MS: u64 = 3_610_000;
const TRADE_WINDOW_MS: u64 = 300_000;
const ABSORPTION_WINDOW_MS: u64 = 60_000;
const MAX_BOOK_AGE_MS: u64 = 5_000;
const MAX_PRICE_LOOKBACK_GAP_MS: u64 = 5_000;
static POSITION_INSIGHTS_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
const WALK_FORWARD_MIN_TRAINING_SAMPLES: usize = 30;
/// Heavy research capture is sampled independently from live signal generation.
// Persist one research/signal sample every ten seconds. Live signal snapshots
// are still computed on every feed event, while this cadence limits the
// historical database growth from snapshots, book data, and their outcomes.
pub const RESEARCH_SAMPLE_INTERVAL_MS: u64 = 10_000;
/// Recompute and publish live signals at most once per symbol every two seconds.
pub const LIVE_SIGNAL_SNAPSHOT_INTERVAL_MS: u64 = 2_000;
const WALK_FORWARD_MIN_THRESHOLD_SAMPLES: usize = 10;
const WALK_FORWARD_TRAINING_WINDOW: usize = 200;
/// Keep supporting signal records bounded; outcomes have a separate broader
/// limit because the analysis needs 31 non-overlapping directional outcomes
/// per symbol and horizon after scanning the shared market universe.
pub const WALK_FORWARD_RECORD_LIMIT: usize = 2_000;
/// At the current sampling rate, 1-minute and 5-minute cohorts need hundreds
/// to thousands of raw outcomes each to produce 31 non-overlapping costed
/// directional samples. Keep the scan bounded, but large enough for the
/// current ~100-symbol universe.
pub const WALK_FORWARD_OUTCOME_LIMIT: usize = 1_250_000;
const CALIBRATION_SNAPSHOT_LIMIT: usize = 2_000;
const CALIBRATION_OUTCOME_LIMIT: usize = 20_000;
const WALK_FORWARD_LOGISTIC_REFRESH_INTERVAL: usize = 100;
const TREND_LOGISTIC_CONFIDENCE_THRESHOLDS: [f64; 3] = [0.55, 0.60, 0.65];
const STRENGTH_THRESHOLDS: [f64; 8] = [0.0, 0.1, 0.15, 0.2, 0.25, 0.3, 0.4, 0.5];
const EXPLORATION_SCORE_THRESHOLDS: [f64; 5] = [0.1, 0.2, 0.3, 0.4, 0.5];
const MISSED_OPPORTUNITY_SCORE_THRESHOLDS: [f64; 4] = [0.2, 0.3, 0.4, 0.5];
const MISSED_OPPORTUNITY_STRENGTH_THRESHOLDS: [f64; 5] = [0.25, 0.35, 0.4, 0.45, 0.5];

include!("engine/signal_contracts.rs");
include!("engine/signal_engine_types.rs");

impl Default for SignalEngine {
    fn default() -> Self {
        Self {
            prices: VecDeque::with_capacity(512),
            pending: VecDeque::with_capacity(256),
            trades: VecDeque::with_capacity(2_048),
            absorption_trades: VecDeque::with_capacity(512),
            book_imbalance: None,
            best_quote: None,
            book_bid_levels: Vec::with_capacity(16),
            book_ask_levels: Vec::with_capacity(16),
            book_history: VecDeque::with_capacity(64),
            volatility_1m: None,
            last_research_sample_at: None,
            last_live_snapshot_at: None,
            completed_scout_ids: BTreeSet::new(),
            completed_scout_order: VecDeque::new(),
        }
    }
}

fn breakout_watch(
    components: &SignalComponents,
    range_bounds: Option<(f64, f64)>,
    range_probability: Option<f64>,
) -> Option<(&'static str, f64)> {
    if range_probability? < 0.65 {
        return None;
    }
    let (lower, upper) = range_bounds?;
    let factors = [
        (
            components
                .trend_1m_percent
                .map(|value| (value / 0.25).clamp(-1.0, 1.0)),
            0.15,
        ),
        (
            components
                .momentum_10s_percent
                .map(|value| (value / 0.12).clamp(-1.0, 1.0)),
            0.20,
        ),
        (components.trade_flow_imbalance, 0.25),
        (components.book_imbalance, 0.25),
        (
            components.pressure_without_price_move.map(|value| -value),
            0.15,
        ),
    ];
    let (weighted_bias, available_weight) = factors
        .iter()
        .filter_map(|(value, weight)| value.map(|value| (value * weight, *weight)))
        .fold((0.0, 0.0), |(bias, weight_sum), (value, weight)| {
            (bias + value, weight_sum + weight)
        });
    if available_weight == 0.0 {
        return None;
    }
    let bias = weighted_bias / available_weight;
    if bias.abs() < 0.3 {
        return None;
    }

    let midpoint = lower + (upper - lower) * 0.5;
    let width_percent = if midpoint > 0.0 {
        (upper - lower) / midpoint * 100.0
    } else {
        f64::INFINITY
    };
    let expected_move = components.volatility_1m_percent.unwrap_or_default().abs() * 5.0_f64.sqrt();
    let compression = if expected_move > 0.0 {
        (1.0 - width_percent / (expected_move * 3.0)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let pressure = (bias.abs() * (0.7 + 0.3 * compression)).clamp(0.0, 1.0);
    (pressure >= 0.35).then_some((if bias > 0.0 { "UP" } else { "DOWN" }, pressure))
}

include!("engine/signal_logistic_models.rs");
include!("engine/signal_outcome_models.rs");

include!("engine/signal_engine_outcomes.rs");
include!("engine/signal_engine_research.rs");
include!("engine/signal_engine_market_inputs.rs");
include!("engine/signal_engine_snapshot.rs");
include!("engine/signal_snapshot_diagnostics.rs");
include!("engine/signal_engine_state.rs");

fn select_strength_threshold(training: &[SignalOutcome], evaluated_by: u64) -> Option<f64> {
    STRENGTH_THRESHOLDS
        .iter()
        .copied()
        .filter_map(|threshold| {
            let (net_total, selected_count) = training
                .iter()
                .filter(|sample| {
                    sample.evaluated_timestamp <= evaluated_by && sample.strength >= threshold
                })
                .fold((0.0, 0usize), |(total, count), sample| {
                    (
                        total + sample.net_return_percent.unwrap_or_default(),
                        count + 1,
                    )
                });
            if selected_count < WALK_FORWARD_MIN_THRESHOLD_SAMPLES {
                return None;
            }
            let mean = net_total / selected_count as f64;
            (mean > 0.0).then_some((threshold, mean))
        })
        .max_by(|left, right| left.1.total_cmp(&right.1).then(left.0.total_cmp(&right.0)))
        .map(|(threshold, _)| threshold)
}

include!("store/signal_store_types.rs");

include!("store/signal_record_codec.rs");

include!("store/signal_store_database.rs");
include!("store/signal_store_audit.rs");
include!("store/signal_store_execution_quality.rs");
const DECISION_QUALITY_MIN_COHORT_SAMPLES: usize = 10;
include!("store/signal_store_decision_quality.rs");
include!("store/signal_store_routes.rs");
include!("store/close_insights_api.rs");
include!("store/signal_store_market_data.rs");
include!("store/signal_store_dashboard.rs");
include!("store/signal_store_performance.rs");
include!("store/signal_store_missed_opportunities.rs");
include!("store/signal_walk_forward_samples.rs");
include!("store/signal_store_walk_forward_result.rs");
include!("store/signal_store_walk_forward_cache.rs");
include!("store/signal_store_walk_forward.rs");
include!("store/signal_store_record_writes.rs");

fn run_database_writer(mut connection: Connection, mut receiver: mpsc::Receiver<WriteRequest>) {
    const MAX_BATCH_REQUESTS: usize = 128;
    while let Some(first) = receiver.blocking_recv() {
        let mut batch = vec![first];
        while batch.len() < MAX_BATCH_REQUESTS {
            match receiver.try_recv() {
                Ok(request) => batch.push(request),
                Err(_) => break,
            }
        }
        let result = (|| -> Result<()> {
            // Do serialization and compression before taking SQLite's write lock.
            let mut signal_rows = Vec::new();
            let mut audit_rows = Vec::new();
            for request in &batch {
                match &request.operation {
                    WriteOperation::Records(records) => {
                        for record in records {
                            signal_rows.push((
                                record_timestamp(record) as i64,
                                record_kind(record),
                                compress_record(record)?,
                            ));
                        }
                    }
                    WriteOperation::Audit { decision_id, stage, timestamp, symbol, payload } => {
                        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
                        encoder.write_all(payload)?;
                        audit_rows.push((decision_id, stage, timestamp, symbol, encoder.finish()?));
                    }
                }
            }
            let transaction = connection.transaction()?;
            {
                let mut insert = transaction.prepare(
                    "INSERT INTO signal_records(timestamp, kind, payload) VALUES (?1, ?2, ?3)",
                )?;
                for (timestamp, kind, payload) in signal_rows {
                    insert.execute(params![timestamp, kind, payload])?;
                }
            }
            {
                let mut insert = transaction.prepare(
                    "INSERT INTO auto_trader_audit(decision_id, stage, timestamp, symbol, payload) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(decision_id, stage) DO UPDATE SET timestamp=excluded.timestamp, symbol=excluded.symbol, payload=excluded.payload",
                )?;
                for (decision_id, stage, timestamp, symbol, payload) in audit_rows {
                    insert.execute(params![decision_id, stage, timestamp, symbol, payload])?;
                }
            }
            transaction.commit()?;
            Ok(())
        })()
        .map_err(|error| error.to_string());
        for request in batch {
            let _ = request.ack.send(result.clone());
        }
    }
}

#[cfg(test)]
mod coverage_tests {
    use super::*;

    #[test]
    fn breakout_watch_requires_confidence_range_and_directional_pressure() {
        let bullish = SignalComponents {
            trend_1m_percent: Some(0.25),
            momentum_10s_percent: Some(0.12),
            trade_flow_imbalance: Some(1.0),
            book_imbalance: Some(1.0),
            volatility_1m_percent: Some(0.2),
            trend_5m_percent: None,
            trend_5m_volatility_units: None,
            mispricing_vs_vwap_bps: None,
            pressure_without_price_move: None,
            aggressor_cost_bps: None,
            ..SignalComponents::default()
        };
        assert_eq!(
            breakout_watch(&bullish, Some((99.0, 101.0)), Some(0.64)),
            None
        );
        assert_eq!(breakout_watch(&bullish, None, Some(0.9)), None);
        let (direction, pressure) =
            breakout_watch(&bullish, Some((99.0, 101.0)), Some(0.9)).unwrap();
        assert_eq!(direction, "UP");
        assert!(pressure >= 0.35);

        let bearish = SignalComponents {
            trend_1m_percent: None,
            momentum_10s_percent: None,
            trade_flow_imbalance: Some(-1.0),
            book_imbalance: Some(-1.0),
            volatility_1m_percent: None,
            trend_5m_percent: None,
            trend_5m_volatility_units: None,
            mispricing_vs_vwap_bps: None,
            pressure_without_price_move: None,
            aggressor_cost_bps: None,
            ..SignalComponents::default()
        };
        assert_eq!(
            breakout_watch(&bearish, Some((99.0, 101.0)), Some(0.9))
                .unwrap()
                .0,
            "DOWN"
        );
        assert_eq!(
            breakout_watch(
                &SignalComponents {
                    trend_1m_percent: None,
                    momentum_10s_percent: None,
                    trade_flow_imbalance: None,
                    book_imbalance: None,
                    volatility_1m_percent: None,
                    trend_5m_percent: None,
                    trend_5m_volatility_units: None,
                    mispricing_vs_vwap_bps: None,
                    pressure_without_price_move: None,
                    aggressor_cost_bps: None,
                    ..SignalComponents::default()
                },
                Some((99.0, 101.0)),
                Some(0.9)
            ),
            None
        );
    }

    #[test]
    fn strength_threshold_uses_only_available_positive_training_cohorts() {
        let training = (0..10)
            .map(|index| SignalOutcome {
                record_type: "outcome".into(),
                symbol: "BTCUSDT".into(),
                signal_timestamp: 1,
                evaluated_timestamp: 100 + index,
                horizon_seconds: 60,
                return_percent: 1.0,
                direction_match: true,
                direction: "long".into(),
                strength: 0.3,
                net_return_percent: Some(1.0),
                execution_cost_percent: None,
                trend_5m_volatility_units: None,
                trend_5m_direction_match: None,
                trend_5m_net_return_percent: None,
                features: None,
            })
            .collect::<Vec<_>>();
        assert_eq!(select_strength_threshold(&training, 105), None);
        assert_eq!(select_strength_threshold(&training, 200), Some(0.3));

        let losses = training
            .into_iter()
            .map(|mut sample| {
                sample.evaluated_timestamp = 1;
                sample.net_return_percent = Some(-1.0);
                sample
            })
            .collect::<Vec<_>>();
        assert_eq!(select_strength_threshold(&losses, 200), None);
    }
}

#[cfg(test)]
mod granular_live_feature_tests {
    use super::*;

    #[test]
    fn snapshots_publish_window_and_depth_variants_for_live_scouts() {
        let mut engine = SignalEngine::default();
        for index in 0..=120_u64 {
            let timestamp = 1_000 + index * 1_000;
            let price = 100.0 + index as f64 * 0.01;
            engine.on_price(timestamp, price, 0.0);
            if index % 5 == 0 {
                engine.on_book(
                    timestamp,
                    price - 0.01,
                    price + 0.01,
                    &[1.0, 2.0, 3.0, 4.0, 5.0],
                    &[5.0, 4.0, 3.0, 2.0, 1.0],
                    &[
                        (price - 0.01, 1.0),
                        (price - 0.02, 2.0),
                        (price - 0.03, 3.0),
                        (price - 0.04, 4.0),
                        (price - 0.05, 5.0),
                    ],
                    &[
                        (price + 0.01, 5.0),
                        (price + 0.02, 4.0),
                        (price + 0.03, 3.0),
                        (price + 0.04, 2.0),
                        (price + 0.05, 1.0),
                    ],
                );
                engine.on_trade(timestamp, price + 0.01, 1.0, true);
            }
        }

        let snapshot = engine.snapshot("ETH-USDT", 121_000).unwrap();
        let components = snapshot.components;
        assert!(
            components
                .momentum_by_window
                .iter()
                .any(|feature| { feature.window_seconds == 10 && feature.value.is_some() })
        );
        assert!(components.trade_flow_by_window.iter().any(|feature| {
            feature.window_seconds == 60 && feature.value.is_some() && feature.sample_count > 0
        }));
        assert!(
            components
                .book_imbalance_by_depth
                .iter()
                .any(|feature| { feature.depth_levels == 5 && feature.value.is_some() })
        );
        assert!(components.vwap_dislocation_by_window.iter().any(|feature| {
            feature.window_seconds == 60 && feature.value.is_some() && feature.sample_count >= 5
        }));
        assert!(
            components
                .absorption_pressure_by_window
                .iter()
                .any(|feature| {
                    feature.window_seconds == 30
                        && feature.replenishment_window_ms == 2_000
                        && feature.touch_tolerance_bps == 2.0
                })
        );
    }
}
