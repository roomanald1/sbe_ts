//! Versioned request/response contract for stateless live strategy evaluation.
//! Scout assignments intentionally use their separate contracts.

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
};

const LEARNING_SNAPSHOT_GZIP_MAGIC: &[u8] = b"SBE-LS-GZIP-1\0";
const MAX_DECOMPRESSED_LEARNING_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;

/// Encode a Learning snapshot as a compact private RSocket payload.
pub fn encode_learning_snapshot_payload(
    snapshot: &crate::LearnedProfileSnapshot,
) -> anyhow::Result<Vec<u8>> {
    use flate2::{Compression, write::GzEncoder};

    let json = serde_json::to_vec(snapshot)?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(&json)?;
    let compressed = encoder.finish()?;
    let mut payload = Vec::with_capacity(LEARNING_SNAPSHOT_GZIP_MAGIC.len() + compressed.len());
    payload.extend_from_slice(LEARNING_SNAPSHOT_GZIP_MAGIC);
    payload.extend_from_slice(&compressed);
    Ok(payload)
}

/// Decode the compressed snapshot payload; return `None` for legacy JSON messages.
pub fn decode_learning_snapshot_payload(
    payload: &[u8],
) -> anyhow::Result<Option<crate::LearnedProfileSnapshot>> {
    use flate2::read::GzDecoder;

    let Some(compressed) = payload.strip_prefix(LEARNING_SNAPSHOT_GZIP_MAGIC) else {
        return Ok(None);
    };
    let decoder = GzDecoder::new(compressed);
    let mut json = Vec::new();
    decoder
        .take(MAX_DECOMPRESSED_LEARNING_SNAPSHOT_BYTES + 1)
        .read_to_end(&mut json)?;
    anyhow::ensure!(
        json.len() as u64 <= MAX_DECOMPRESSED_LEARNING_SNAPSHOT_BYTES,
        "decompressed Learning snapshot exceeds the 64 MiB limit"
    );
    Ok(Some(serde_json::from_slice(&json)?))
}

use crate::{
    DailyStrategySignalFeatures, DatedCarryCandidate, DatedCarrySettlementSource, Strategy,
    StrategyMarketSignal, StrategyWalkForwardEvidence,
};

pub const LIVE_EVALUATION_CONTRACT_VERSION: u16 = 1;
pub const STRATEGY_INSIGHT_UPDATE_VERSION: u16 = 1;
pub const LIVE_STRATEGY_EVENT_ID_VERSION: u16 = 1;

/// Deterministic identity assigned to one upstream strategy insight update.
/// Event identity follows the source update; episode identity follows the
/// strategy thesis and intentionally excludes event timestamp/payload.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct LiveStrategyEventIdentity {
    pub schema_version: u16,
    pub event_id: String,
    /// Stable across market updates while the canonical thesis is unchanged.
    /// If upstream state provides no authoritative reset, the same thesis
    /// conservatively reuses this ID, including after worker restarts.
    pub episode_id: String,
    /// Durable AutoTrader reset token for a completed, fully filled stop.
    /// Zero preserves the original v1 thesis hash byte-for-byte.
    pub episode_reset_generation: u64,
}

impl LiveStrategyEventIdentity {
    pub fn from_source_event_id(source_event_id: &str) -> Self {
        let key = format!(
            "strategy-insight:v{}:{source_event_id}",
            LIVE_STRATEGY_EVENT_ID_VERSION
        );
        Self {
            schema_version: LIVE_STRATEGY_EVENT_ID_VERSION,
            event_id: key,
            // Source-only consumers (daily and carry) have no authoritative
            // thesis/reset state. Keep their episode best-effort and stable
            // for retries without pretending a reset occurred.
            episode_id: format!(
                "strategy-episode:v{}:source:{source_event_id}",
                LIVE_STRATEGY_EVENT_ID_VERSION
            ),
            episode_reset_generation: 0,
        }
    }

    /// Build a deterministic episode identity from durable thesis dimensions.
    /// Source event changes never alter the episode ID. A new event ID still
    /// results for every distinct source key. Missing reset evidence therefore
    /// conservatively reuses the ID for an identical thesis.
    pub fn for_thesis(
        source_event_id: &str,
        strategy: crate::Strategy,
        symbol: &str,
        profile_id: &str,
        direction: &str,
        active_market_state_key: Option<&str>,
    ) -> Self {
        Self::for_thesis_with_reset_generation(
            source_event_id,
            strategy,
            symbol,
            profile_id,
            direction,
            active_market_state_key,
            0,
        )
    }

    /// Versioned episode continuity with an optional authoritative reset
    /// token. Reset generation zero retains the v1 encoding exactly; a
    /// completed stop advances the token durably in AutoTrader and causes a
    /// new episode even when all thesis and market-state dimensions match.
    pub fn for_thesis_with_reset_generation(
        source_event_id: &str,
        strategy: crate::Strategy,
        symbol: &str,
        profile_id: &str,
        direction: &str,
        active_market_state_key: Option<&str>,
        episode_reset_generation: u64,
    ) -> Self {
        use sha2::{Digest, Sha256};

        let mut identity = Self::from_source_event_id(source_event_id);
        let canonical_thesis = if episode_reset_generation == 0 {
            // Preserve deployed v1 episode IDs when no authoritative reset
            // has occurred; source event changes still do not affect them.
            serde_json::to_vec(&(
                "strategy-thesis",
                LIVE_STRATEGY_EVENT_ID_VERSION,
                strategy.as_str(),
                symbol,
                profile_id,
                direction,
                active_market_state_key,
            ))
        } else {
            serde_json::to_vec(&(
                "strategy-thesis",
                LIVE_STRATEGY_EVENT_ID_VERSION,
                strategy.as_str(),
                symbol,
                profile_id,
                direction,
                active_market_state_key,
                "completed-stop-reset",
                episode_reset_generation,
            ))
        }
        .expect("serializing thesis dimensions cannot fail");
        let digest = Sha256::digest(canonical_thesis);
        let mut hex = String::with_capacity(digest.len() * 2);
        for byte in digest {
            use std::fmt::Write as _;
            write!(&mut hex, "{byte:02x}").expect("writing to String cannot fail");
        }
        identity.episode_id = format!(
            "strategy-episode:v{}:sha256:{hex}",
            LIVE_STRATEGY_EVENT_ID_VERSION
        );
        identity.episode_reset_generation = episode_reset_generation;
        identity
    }
}

/// A derived, bounded Insights update delivered to strategy workers.
/// The event timestamp is the deduplication key within a symbol stream.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StrategyInsightUpdate {
    pub contract_version: u16,
    pub event_id: String,
    pub event_time_ms: u64,
    #[serde(default)]
    pub signal: Option<crate::StrategyMarketSignal>,
    #[serde(default)]
    pub market: Option<serde_json::Value>,
    #[serde(default)]
    pub taker_fee_rate: Option<f64>,
    #[serde(default)]
    pub daily_snapshot: Option<crate::daily::DailyStrategySnapshot>,
    #[serde(default)]
    pub dated_carry_scan: Option<DatedCarryScanUpdate>,
    #[serde(default)]
    pub dated_carry_candidate: Option<crate::DatedCarryCandidate>,
    #[serde(default)]
    pub dated_carry_settlement: Option<DatedCarrySettlementUpdate>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DatedCarryScanUpdate {
    pub emitted_at_ms: u64,
    pub candidates: Vec<crate::DatedCarryCandidate>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DatedCarrySettlementUpdate {
    pub emitted_at_ms: u64,
    pub spot_symbol: String,
    pub future_symbol: String,
    pub expiry_ms: u64,
    pub spot_exit_twap: f64,
    pub settlement_price: f64,
    pub settlement_source: crate::DatedCarrySettlementSource,
}

/// Coordinator-owned live configuration installed atomically by AutoTrader.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveStrategyProfileSet {
    pub contract_version: u16,
    pub set_version: u64,
    pub strategy: Strategy,
    pub enabled: bool,
    pub profiles: Vec<LiveStrategyProfile>,
    #[serde(default)]
    pub open_positions: Vec<LiveStrategyOpenPosition>,
    /// Persisted reset generations for a thesis after a fully filled closing
    /// stop. Missing entries are legacy generation zero.
    #[serde(default)]
    pub episode_reset_generations: Vec<LiveStrategyEpisodeResetGeneration>,
}

impl LiveStrategyProfileSet {
    pub fn episode_reset_generation(&self, symbol: &str, profile_id: &str) -> u64 {
        self.episode_reset_generations
            .iter()
            .find(|reset| reset.symbol == symbol && reset.profile_id == profile_id)
            .map(|reset| reset.generation)
            .unwrap_or(0)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct LiveStrategyEpisodeResetGeneration {
    pub symbol: String,
    pub profile_id: String,
    pub generation: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveStrategyOpenPosition {
    pub symbol: String,
    pub profile_id: String,
    pub profile_symbol: String,
}

/// A strategy decision returned to AutoTrader; it is never an executable order.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveStrategyDecisionReport {
    pub contract_version: u16,
    pub event_id: String,
    /// Version of the deterministic event/episode key encoding. Missing on
    /// legacy reports, where consumers should treat identity as unversioned.
    #[serde(default)]
    pub event_identity_version: u16,
    /// Stable identity for the unchanged trading thesis; source events can
    /// advance while this episode continues.
    #[serde(default)]
    pub episode_id: String,
    pub strategy: Strategy,
    pub symbol: String,
    pub profile_id: String,
    pub profile_version: u64,
    pub parameter_hash: String,
    pub event_time_ms: u64,
    pub decision: LiveStrategyDecision,
    pub reason: Option<String>,
    pub walk_forward_evidence: Option<crate::StrategyWalkForwardEvidence>,
    pub signal: crate::StrategyMarketSignal,
    #[serde(default)]
    pub dated_carry_scan: Option<DatedCarryScanUpdate>,
    #[serde(default)]
    pub dated_carry_candidate: Option<crate::DatedCarryCandidate>,
    #[serde(default)]
    pub dated_carry_settlement: Option<DatedCarrySettlementUpdate>,
}

/// A per-symbol worker skip or preparation failure that explains why no
/// decision report was produced for an Insights update.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveStrategyWorkerDiagnostic {
    pub contract_version: u16,
    pub event_id: String,
    pub strategy: Strategy,
    pub symbol: String,
    #[serde(default)]
    pub profile_id: Option<String>,
    pub event_time_ms: u64,
    pub reason: String,
}

/// Typed AutoTrader/worker RSocket operations. Profile sets flow from
/// AutoTrader to workers; decisions flow from workers back to AutoTrader.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "operation", content = "payload", rename_all = "snake_case")]
pub enum LiveStrategySocketRequest {
    UpdateProfiles(LiveStrategyProfileSet),
    UpdateLearningSnapshot(crate::LearnedProfileSnapshot),
    ReportDecision(LiveStrategyDecisionReport),
    ReportDiagnostic(LiveStrategyWorkerDiagnostic),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "operation", content = "payload", rename_all = "snake_case")]
pub enum LiveStrategySocketResponse {
    ProfilesApplied {
        strategy: Strategy,
        set_version: u64,
    },
    LearningSnapshotApplied,
    DecisionAccepted {
        status: u16,
        detail: Option<String>,
    },
    Rejected {
        status: u16,
        detail: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveStrategyProfile {
    pub profile_id: String,
    pub profile_symbol: String,
    pub version: u64,
    /// Optional live routing label. `None` is the backward-compatible fallback profile.
    #[serde(default)]
    pub market_regime: Option<String>,
    /// Versioned direction/structure plus volatility routing key.
    #[serde(default)]
    pub market_state_key: Option<String>,
    pub parameters: BTreeMap<String, f64>,
    /// Updated by Learning and delivered through AutoTrader's profile push.
    #[serde(default)]
    pub walk_forward_evidence: Option<StrategyWalkForwardEvidence>,
    /// Regime-conditioned Scout evidence. The live worker selects the bucket
    /// matching the incoming signal and otherwise uses the unconditioned value.
    #[serde(default)]
    pub walk_forward_evidence_by_regime:
        std::collections::BTreeMap<String, StrategyWalkForwardEvidence>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct LivePositionContext {
    pub is_open: bool,
    pub strategy_enabled: bool,
    pub signal_active: Option<bool>,
    pub rebalance_required: bool,
    pub daily_features: Option<DailyStrategySignalFeatures>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "stage", content = "input", rename_all = "snake_case")]
pub enum LiveEvaluationInput {
    Entry {
        signal: StrategyMarketSignal,
        daily_features: Option<DailyStrategySignalFeatures>,
        walk_forward_evidence: Option<StrategyWalkForwardEvidence>,
    },
    OpenPosition {
        position: LivePositionContext,
    },
    DatedCarryEntry {
        emitted_at_ms: u64,
        evaluated_at_ms: u64,
        candidate: DatedCarryCandidate,
    },
    Settlement {
        emitted_at_ms: u64,
        evaluated_at_ms: u64,
        spot_symbol: String,
        future_symbol: String,
        expiry_ms: u64,
        spot_exit_twap: f64,
        settlement_price: f64,
        settlement_source: Option<DatedCarrySettlementSource>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveEvaluationRequest {
    pub contract_version: u16,
    pub request_id: String,
    pub strategy: Strategy,
    pub symbol: String,
    pub profile: LiveStrategyProfile,
    pub input: LiveEvaluationInput,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveStrategyDecision {
    Enter,
    Hold,
    Exit,
    Rebalance,
    Reject,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveEvaluationErrorCode {
    InvalidRequest,
    UnsupportedVersion,
    WrongStrategy,
    InvalidSymbol,
    InvalidProfile,
    InvalidParameters,
    EvaluationFailed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LiveEvaluationError {
    pub code: LiveEvaluationErrorCode,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveEvaluationResponse {
    pub contract_version: u16,
    pub request_id: String,
    pub strategy: Strategy,
    pub symbol: String,
    pub profile_id: String,
    pub profile_symbol: String,
    pub profile_version: u64,
    pub decision: Option<LiveStrategyDecision>,
    pub error: Option<LiveEvaluationError>,
}

#[cfg(test)]
mod socket_contract_tests {
    use super::*;

    #[test]
    fn compressed_learning_snapshot_round_trips_and_is_smaller() {
        let context_returns = (0..1_000)
            .map(|index| {
                (
                    format!("profile-{index}|long:trend:BTC-USDT:live-momentum"),
                    vec![(1_700_000_000_000_u64, -0.0123); 5],
                )
            })
            .collect::<BTreeMap<_, _>>();
        let snapshot: crate::LearnedProfileSnapshot = serde_json::from_value(serde_json::json!({
            "profiles": [],
            "strategy_size_multipliers": {},
            "recent_profile_context_returns": context_returns,
        }))
        .unwrap();
        let plain = serde_json::to_vec(&snapshot).unwrap();

        let compressed = encode_learning_snapshot_payload(&snapshot).unwrap();
        let decoded = decode_learning_snapshot_payload(&compressed)
            .unwrap()
            .unwrap();

        assert!(compressed.len() < plain.len() / 5);
        assert_eq!(
            serde_json::to_value(decoded).unwrap(),
            serde_json::to_value(snapshot).unwrap()
        );
        assert!(decode_learning_snapshot_payload(&plain).unwrap().is_none());
    }

    #[test]
    fn profile_push_and_decision_report_use_distinct_typed_socket_operations() {
        let profile_set = LiveStrategyProfileSet {
            contract_version: LIVE_EVALUATION_CONTRACT_VERSION,
            set_version: 42,
            strategy: Strategy::LiveMomentum,
            enabled: true,
            profiles: Vec::new(),
            open_positions: Vec::new(),
            episode_reset_generations: Vec::new(),
        };
        let profile_request = LiveStrategySocketRequest::UpdateProfiles(profile_set);
        let profile_wire = serde_json::to_value(&profile_request).unwrap();
        assert_eq!(profile_wire["operation"], "update_profiles");
        assert!(profile_wire.get("path").is_none());
        assert_eq!(
            serde_json::to_value(
                serde_json::from_value::<LiveStrategySocketRequest>(profile_wire).unwrap()
            )
            .unwrap(),
            serde_json::to_value(profile_request).unwrap()
        );

        let decision_request =
            LiveStrategySocketRequest::ReportDecision(LiveStrategyDecisionReport {
                contract_version: LIVE_EVALUATION_CONTRACT_VERSION,
                event_id: "event-1".into(),
                event_identity_version: LIVE_STRATEGY_EVENT_ID_VERSION,
                episode_id: "episode-1".into(),
                strategy: Strategy::LiveMomentum,
                symbol: "BTC-USDT".into(),
                profile_id: "profile-1".into(),
                profile_version: 1,
                parameter_hash: "profile-1".into(),
                event_time_ms: 1_700_000_000_000,
                decision: LiveStrategyDecision::Reject,
                reason: Some("neutral signal".into()),
                walk_forward_evidence: None,
                signal: crate::StrategyMarketSignal::default(),
                dated_carry_scan: None,
                dated_carry_candidate: None,
                dated_carry_settlement: None,
            });
        let decision_wire = serde_json::to_value(&decision_request).unwrap();
        assert_eq!(decision_wire["operation"], "report_decision");
        assert!(decision_wire.get("path").is_none());
        assert_eq!(
            serde_json::to_value(
                serde_json::from_value::<LiveStrategySocketRequest>(decision_wire).unwrap()
            )
            .unwrap(),
            serde_json::to_value(decision_request).unwrap()
        );
    }

    #[test]
    fn source_identity_is_versioned_deterministic_and_thesis_identity_ignores_source_key() {
        let retry = LiveStrategyEventIdentity::from_source_event_id("BTC-USDT:123");
        let replay = LiveStrategyEventIdentity::from_source_event_id("BTC-USDT:123");
        let thesis_a = LiveStrategyEventIdentity::for_thesis(
            "BTC-USDT:123",
            crate::Strategy::LiveMomentum,
            "BTC-USDT",
            "profile-a",
            "bullish",
            Some("v2:trend_up:normal"),
        );
        let thesis_b = LiveStrategyEventIdentity::for_thesis(
            "BTC-USDT:124",
            crate::Strategy::LiveMomentum,
            "BTC-USDT",
            "profile-a",
            "bullish",
            Some("v2:trend_up:normal"),
        );

        assert_eq!(retry, replay);
        assert_eq!(retry.schema_version, LIVE_STRATEGY_EVENT_ID_VERSION);
        assert!(retry.event_id.starts_with("strategy-insight:v1:"));
        assert_ne!(thesis_a.event_id, thesis_b.event_id);
        assert_eq!(thesis_a.episode_id, thesis_b.episode_id);
        assert!(
            thesis_a
                .episode_id
                .starts_with("strategy-episode:v1:sha256:")
        );
    }

    #[test]
    fn thesis_identity_resets_on_supported_thesis_dimensions_only() {
        let baseline = || {
            LiveStrategyEventIdentity::for_thesis(
                "event-1",
                crate::Strategy::LiveMomentum,
                "BTC-USDT",
                "profile-a",
                "bullish",
                Some("v2:trend_up:normal"),
            )
        };
        let base = baseline();
        let variants = [
            LiveStrategyEventIdentity::for_thesis(
                "event-1",
                crate::Strategy::LiveTradeFlow,
                "BTC-USDT",
                "profile-a",
                "bullish",
                Some("v2:trend_up:normal"),
            ),
            LiveStrategyEventIdentity::for_thesis(
                "event-1",
                crate::Strategy::LiveMomentum,
                "ETH-USDT",
                "profile-a",
                "bullish",
                Some("v2:trend_up:normal"),
            ),
            LiveStrategyEventIdentity::for_thesis(
                "event-1",
                crate::Strategy::LiveMomentum,
                "BTC-USDT",
                "profile-b",
                "bullish",
                Some("v2:trend_up:normal"),
            ),
            LiveStrategyEventIdentity::for_thesis(
                "event-1",
                crate::Strategy::LiveMomentum,
                "BTC-USDT",
                "profile-a",
                "bearish",
                Some("v2:trend_up:normal"),
            ),
            LiveStrategyEventIdentity::for_thesis(
                "event-1",
                crate::Strategy::LiveMomentum,
                "BTC-USDT",
                "profile-a",
                "bullish",
                Some("v2:range:normal"),
            ),
        ];
        assert!(
            variants
                .iter()
                .all(|variant| variant.episode_id != base.episode_id)
        );
        let no_state_a = LiveStrategyEventIdentity::for_thesis(
            "event-1",
            crate::Strategy::LiveMomentum,
            "BTC-USDT",
            "profile-a",
            "bullish",
            None,
        );
        let no_state_b = LiveStrategyEventIdentity::for_thesis(
            "event-2",
            crate::Strategy::LiveMomentum,
            "BTC-USDT",
            "profile-a",
            "bullish",
            None,
        );
        assert_eq!(no_state_a.episode_id, no_state_b.episode_id);
    }

    #[test]
    fn completed_stop_reset_generation_changes_episode_but_not_event_identity() {
        let baseline = LiveStrategyEventIdentity::for_thesis(
            "event-1",
            crate::Strategy::LiveMomentum,
            "BTC-USDT",
            "profile-a",
            "bullish",
            Some("v2:trend_up:normal"),
        );
        let explicit_zero = LiveStrategyEventIdentity::for_thesis_with_reset_generation(
            "event-1",
            crate::Strategy::LiveMomentum,
            "BTC-USDT",
            "profile-a",
            "bullish",
            Some("v2:trend_up:normal"),
            0,
        );
        let reset_one = LiveStrategyEventIdentity::for_thesis_with_reset_generation(
            "event-1",
            crate::Strategy::LiveMomentum,
            "BTC-USDT",
            "profile-a",
            "bullish",
            Some("v2:trend_up:normal"),
            1,
        );
        let reset_one_after_update = LiveStrategyEventIdentity::for_thesis_with_reset_generation(
            "event-2",
            crate::Strategy::LiveMomentum,
            "BTC-USDT",
            "profile-a",
            "bullish",
            Some("v2:trend_up:normal"),
            1,
        );
        assert_eq!(baseline.episode_id, explicit_zero.episode_id);
        assert_ne!(baseline.episode_id, reset_one.episode_id);
        assert_eq!(reset_one.episode_id, reset_one_after_update.episode_id);
        assert_ne!(reset_one.event_id, reset_one_after_update.event_id);
        assert_eq!(reset_one.episode_reset_generation, 1);
    }
}
