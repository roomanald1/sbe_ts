import { BehaviorSubject } from 'rxjs';
import {
  currentPositionSnapshot,
  subscribePositionStream,
  waitForPositionSnapshot,
} from '../position_stream';
import {
  currentAutoTraderSnapshot,
  isAutoTraderSnapshotFresh,
  subscribeAutoTraderStream,
  waitForAutoTraderSnapshot,
  type AutoTraderUiSnapshot,
} from '../auto_trader_stream';
import { subscribeInsightsStream } from '../insights_stream';
import { autoTraderFetch, autoTraderRpc } from '../auto_trader_rsocket';
import { learningRpc } from '../learning_rsocket';
import { sharedGet } from '../shared_get';

const WORKER_DECISION_STATUS_POLL_MS = 2_000;

export type AutoTraderOrder = {
  timestamp: number;
  triggered_at?: number;
  symbol: string;
  side: string;
  order_type: string;
  purpose?: string;
  strategy?: string;
  decision_id?: string;
  quantity: number;
  filled_quantity: number;
  submitted_limit_price?: number;
  target_price?: number;
  average_price?: number;
  status: string;
  failure_reason?: string;
  realized_pnl: number;
  realized_pnl_usd?: number | null;
};
const defaultScoutStrategies: Strategy[] = [
  'live-momentum',
  'live-trade-flow',
  'live-order-book',
  'live-vwap-dislocation',
  'live-absorption-pressure',
  'live-range-mean-reversion',
  'donchian-55-btc200',
  'donchian-20-sma200',
  'donchian-20-btc200',
  'donchian-10-btc200',
  'donchian-20-btc200-hold3',
  'risk-adjusted-trend-portfolio',
  'trend-sleeves-btc200',
  'trend-50-200',
  'relative-strength-90-sma200',
  'risk-adjusted-monthly-momentum',
  'rsi2-oversold-rebound',
  'dated-futures-carry',
];
export type AutoTraderDecisionParameter = {
  section: string;
  name: string;
  value: number;
  unit: string;
};
export type AutoTraderContextParameters = {
  context: string;
  learning_samples: number;
  positive_rate?: number;
  scout_learning_samples: number;
  scout_positive_rate?: number;
  strength_floor: number;
  exploration_score_floor: number;
  exploration_allowed: boolean;
};
export type AutoTraderLatestExecutionParameters = {
  timestamp: number;
  symbol: string;
  context: string;
  baseline_order_type: string;
  selected_order_type: string;
  profile: number;
  vwap_duration_minutes: number;
  vwap_participation_percent: number;
  schedule_depth_threshold: number;
  proposed_depth_ratio: number;
  schedule_threshold_samples: number;
  limit_offset_bps: number;
  trigger_offset_bps: number;
  trailing_distance_percent: number;
};
export type AutoTraderStrategyScoutVariant = {
  candidate_value: number;
  sample_count: number;
  minimum_samples: number;
  average_net_improvement_percent: number;
  lower_90_one_sided_improvement_percent?: number | null;
  upper_90_one_sided_improvement_percent?: number | null;
  paired_baseline_increase_samples?: number;
  paired_baseline_decrease_samples?: number;
  paired_baseline_same_samples?: number;
  paired_baseline_unknown_samples?: number;
  paired_baseline_delta_mean?: number | null;
  paired_baseline_delta_samples?: number;
  promoted: boolean;
  rejected: boolean;
};
export type AutoTraderStrategyParameter = {
  key: string;
  label: string;
  value: number;
  learned_value: number;
  configured_value?: number | null;
  default_value: number;
  min: number;
  max: number;
  step: number;
  unit: string;
  overridden: boolean;
  scout_variants?: AutoTraderStrategyScoutVariant[] | null;
};
export type AutoTraderStrategyProfile = {
  symbol: string;
  strategy: Strategy;
  profile_id?: string | null;
  profile_version?: number | null;
  learning_samples: number;
  positive_rate?: number;
  realized_pnl: number;
  walk_forward_evidence?: WalkForwardEvidence | null;
  walk_forward_evidence_by_regime?: Record<string, WalkForwardEvidence>;
  parameters: AutoTraderStrategyParameter[];
};
export type WalkForwardEvidence = {
  evidenceContractVersion?: number;
  estimator?:
    | 'completed_outcome_clusters'
    | 'rolling_origin_portfolio'
    | 'unspecified';
  historyCoverage?: 'partial' | 'complete' | 'unknown';
  costCoverage?: 'partial' | 'complete' | 'unknown';
  searchFamilyCoverage?: 'partial' | 'complete' | 'unknown';
  pointInTimeCutoffTimestampMs?: number | null;
  finalHoldoutStatus?: 'validated' | 'failed' | 'not_validated';
  oosFoldCount?: number;
  spaPValue?: number | null;
  sampleCount: number;
  positiveNetRate?: number | null;
  averageNetReturnPercent?: number | null;
  expectancyCiLowPercent?: number | null;
  expectancyCiHighPercent?: number | null;
  effectiveSampleSize?: number | null;
  windowCount?: number;
  maxDrawdownPercent?: number | null;
  downsideLossPercent?: number | null;
  lastSampleTimestampMs: number;
};
export type AutoTraderPendingScoutParameter = {
  symbol: string;
  strategy: Strategy;
  parameter: string;
  candidateValue: number;
  openSamples: number;
};
export type AutoTraderDecisionParameters = {
  refreshed_at: number;
  thresholds: AutoTraderDecisionParameter[];
  contexts: AutoTraderContextParameters[];
  strategy_profiles: AutoTraderStrategyProfile[];
  exit_policy_assessments?: LearnedExitPolicyAssessment[];
  exit_protection_policies?: LearnedExitProtectionPolicy[];
  active_exit_policy: {
    stop_loss_percent: number;
    take_profit_percent: number;
  };
  latest_execution?: AutoTraderLatestExecutionParameters;
  catalog?: Record<
    string,
    Omit<
      AutoTraderStrategyParameter,
      | 'value'
      | 'learned_value'
      | 'configured_value'
      | 'overridden'
      | 'scout_variants'
    >[]
  >;
  version?: number;
};

export type ExitPolicyMetricCoverage = {
  status: ExitPolicyMetricCoverageStatus;
  sample_count: number;
  source_fields: string[];
  reason?: string | null;
};

export type ExitPolicyMetricCoverageStatus =
  | 'complete'
  | 'partial'
  | 'unavailable';

export type ExitPolicyMetricReport = {
  values: Record<string, number>;
  coverage: Record<string, ExitPolicyMetricCoverage>;
  promotion_eligible: boolean;
  promotion_blockers: string[];
};

/** A Learning candidate assessment; it is not an active or published policy. */
export type LearnedExitPolicyAssessment = {
  symbol: string;
  strategy: string;
  profile_id?: string | null;
  profile_version?: number | null;
  stop_loss_percent?: number | null;
  take_profit_percent?: number | null;
  activation_percent?: number | null;
  giveback_percent?: number | null;
  minimum_lock_percent?: number | null;
  sample_count: number;
  training_count: number;
  validation_count: number;
  as_of_ms: number;
  maximum_holding_duration_ms?: number | null;
  status: string;
  metrics: ExitPolicyMetricReport;
};

/** Learning-published candidates are still distinct from AutoTrader activation. */
export type LearnedExitProtectionPolicy = {
  symbol: string;
  strategy: string;
  profile_id?: string | null;
  profile_version?: number | null;
  stop_loss_percent?: number | null;
  take_profit_percent?: number | null;
  maximum_holding_duration_ms?: number | null;
  activation_percent: number;
  giveback_percent: number;
  minimum_lock_percent: number;
  sample_count: number;
  training_count: number;
  validation_count: number;
  baseline_validation_mean_percent: number;
  selected_validation_mean_percent: number;
  validation_gain_percent: number;
  validation_positive_rate: number;
  validation_negative_difference_rate: number;
  validation_tenth_percentile_difference_percent: number;
  validation_worst_difference_percent: number;
  as_of_ms: number;
};

export type Strategy =
  | 'live-momentum'
  | 'live-trade-flow'
  | 'live-order-book'
  | 'live-vwap-dislocation'
  | 'live-absorption-pressure'
  | 'live-range-mean-reversion'
  | 'donchian-55-btc200'
  | 'donchian-20-sma200'
  | 'donchian-20-btc200'
  | 'donchian-10-btc200'
  | 'donchian-20-btc200-hold3'
  | 'risk-adjusted-trend-portfolio'
  | 'trend-sleeves-btc200'
  | 'trend-50-200'
  | 'relative-strength-90-sma200'
  | 'risk-adjusted-monthly-momentum'
  | 'rsi2-oversold-rebound'
  | 'dated-futures-carry';
export type CapitalAllocationRow = {
  strategy: string;
  symbol: string;
  horizon: string;
  effectiveAllocation: number;
  reserved: number;
  used: number;
  utilization: number | null;
  evidenceEligible: boolean | null;
  state: string;
  reason: string;
};
export type CapitalAllocation = {
  version: 'v1';
  model: 'shared_pool';
  poolLimit: number;
  used: number;
  reserved: number;
  available: number;
  rows: CapitalAllocationRow[];
};
export type StrategyDiagnostic = {
  strategy: Strategy;
  as_of_ms: number;
  btc_close: number | null;
  btc_sma: number | null;
  btc_distance_percent: number | null;
  btc_window_days: number | null;
  btc_gate_required: boolean;
  btc_gate_passed: boolean | null;
  btc_source_close_time_ms?: number | null;
  btc_source_timeframe?:
    | 'daily'
    | 'monthly_rebalance_anchor_daily'
    | string
    | null;
  symbols: Array<{
    symbol: string;
    close: number;
    eligible: boolean;
    primary_rule: string;
    primary_level: number | null;
    primary_distance_percent: number | null;
    primary_distance_unit?: string | null;
    primary_metric_value?: number | null;
    primary_metric_unit?: string | null;
    primary_passed: boolean;
    secondary_rule?: string | null;
    secondary_level?: number | null;
    secondary_distance_percent?: number | null;
    secondary_passed?: boolean | null;
    evaluated?: boolean;
    signal_active: boolean;
    leader_gap_percent?: number | null;
    relative_strength_gap_pp?: number | null;
  }>;
};

export function projectStrategyDiagnostics(
  diagnostics: StrategyDiagnostic[] | undefined,
): StrategyDiagnostic[] {
  return (diagnostics ?? []).map((diagnostic) => ({
    ...diagnostic,
    btc_source_close_time_ms: diagnostic.btc_source_close_time_ms ?? null,
    btc_source_timeframe: diagnostic.btc_source_timeframe ?? null,
  }));
}
export type StrategyBlocker = {
  strategy: Strategy;
  symbol: string;
  reason: string;
  timestamp_ms: number;
};
export type SymbolEntryResult = {
  strategy: Strategy;
  symbol: string;
  profile_id?: string | null;
  event_id?: string | null;
  decision?: 'enter' | 'hold' | 'exit' | 'rebalance' | 'reject' | null;
  reason: string;
  timestamp_ms?: number | null;
};
export type LatestEntryDiagnostic = {
  cycleId: string;
  timestampMs: number;
  outcome:
    | 'no_signals'
    | 'no_eligible_candidates'
    | 'no_fresh_candidates'
    | 'candidates_rejected'
    | 'execution_blocked'
    | 'order_queued'
    | 'order_submitted'
    | 'waiting_for_strategy_signals'
    | 'global_risk_block'
    | 'cycle_error';
  rawSignalCount: number;
  allowedSignalCount: number;
  strategyCandidateCount: number;
  freshDirectionalCandidateCount?: number;
  staleCandidateCount?: number;
  nonDirectionalCandidateCount?: number;
  positionBlockedCandidateCount?: number;
  strategyRejectedCandidateCount?: number;
  evidencePresentCandidateCount?: number;
  eligibleCandidateCount: number;
  rejectedCandidateCount: number;
  selectedCandidateCount: number;
  orderAttemptCount: number;
  submittedOrderCount: number;
  fillConfirmed: boolean;
  primaryGateId?: string | null;
  primaryBlocker?: string | null;
  workerEventIds?: string[];
  candidateDecisionIds?: string[];
  gateCounts?: Array<{ gate: string; count: number }>;
  candidateGateCounts?: Array<{
    gateId: string;
    symbol: string;
    strategy?: string | null;
    count: number;
  }>;
  candidateDetails?: Array<{
    decisionId: string;
    workerEventId?: string | null;
    profileId?: string | null;
    symbol: string;
    strategy?: string | null;
    stage: string;
    observed: Record<string, unknown>;
    required: Record<string, unknown>;
    checks: Record<string, unknown>;
    evidenceContract: Record<string, unknown> | null;
    measurements?: Array<{
      gateId: string;
      observed: number;
      comparator: string;
      required: number;
      unit?: string | null;
      passed?: boolean | null;
    }>;
    reasons: string[];
  }>;
  detail: string;
};
export type AutoTraderState = {
  enabled: boolean;
  auditHealthy: boolean;
  auditFailureLatched: boolean;
  decisionAuditQueueDepth: number;
  executionAuditQueueDepth: number;
  auditCoalescedDecisions: number;
  auditDroppedEvents: number;
  paperLearningExplorationUntilMs: number;
  filledPaperOrdersSinceRecoveryReset: number;
  strategies: Strategy[];
  scoutStrategies: Strategy[];
  loading: boolean;
  busy: boolean;
  maxOrderNotional: string;
  dailyNotionalLimit: string;
  usedNotionalToday: number;
  availableNotional: number;
  serverDailyNotionalLimit: number;
  serverExplorationPercent: number;
  dailyRealizedLossLimit: string;
  explorationPercent: string;
  riskAppetite: string;
  maxOpenPositions: string;
  maxConcurrentScoutSamples: string;
  scoutMaxActivePositions: string;
  maxIntradayOpenPositions: string;
  maxDailyOpenPositions: string;
  capitalAllocation?: CapitalAllocation;
  maxEntriesPerPosition: string;
  stopLossPercent: number;
  takeProfitPercent: number;
  vwapDurationMinutes: string;
  vwapParticipationPercent: string;
  limitOffsetBps: string;
  triggerOffsetBps: string;
  trailingDistancePercent: string;
  closedTradeWindow: string;
  minimumScoutParameterSamples: string;
  lastDecision: string;
  lastDecisionAt: number;
  decisionsSinceTrade: number;
  gapReasons: Array<{ reason: string; count: number }>;
  gateHits: Array<{ gate: string; count: number }>;
  symbolGateHits: Array<{
    symbol: string;
    direction: string;
    gate: string;
    count: number;
    last_detail: string;
    last_seen_ms?: number;
    distance_to_pass: number | null;
  }>;
  latestSignalDiagnostic: string;
  latestEntryDiagnostic: LatestEntryDiagnostic | null;
  entryDiagnosticHistory: LatestEntryDiagnostic[];
  latestSymbolEntryResults: SymbolEntryResult[];
  strategyDiagnostics: StrategyDiagnostic[];
  latestStrategyBlockers: StrategyBlocker[];
  orders: AutoTraderOrder[];
  openPositionSymbols: string[];
  ledgerOpenPositionSymbols: string[];
  executionQuality: AutoTraderExecutionQuality;
  decisionQuality: AutoTraderDecisionQuality;
  scoutOpenPositions: number;
  pendingScoutParameters: AutoTraderPendingScoutParameter[];

  scoutLearningSamples: number;
  scoutLearningWinRate?: number;
  scoutLearningRealizedPnl: number;
  realizedPnl?: number | null;
  unrealizedPnl?: number | null;
  exitLearningSamples: number;
  closedTradeAverageReturnPercent?: number;
  closedTradeWins: number;
  closedTradeLosses: number;
  closedTradeFlat: number;
  closedTradeRealizedUsd?: number;
  closedTradeFeesUsd?: number;
  learningSamples: number;
  learningWinRate?: number;
  preferredOrderType?: string;
  parameterProfileSamples: number[];
  decisionParameters?: AutoTraderDecisionParameters;
  strategyParameterOverrides: Record<string, number>;
  decisionParametersError?: string;
  error?: string;
};
export type AutoTraderDecisionQuality = {
  decision_samples: number;
  filled_entry_samples: number;
  unresolved_trade_samples: number;
  unpriced_closed_trade_samples: number;
  completed_trade_samples: number;
  scout_filled_entry_samples?: number;
  scout_unresolved_trade_samples?: number;
  scout_unpriced_closed_trade_samples?: number;
  scout_completed_trade_samples?: number;
  history_truncated: boolean;
  cohorts: Array<{
    dimension: string;
    value: string;
    completed_trade_samples: number;
    average_net_return_percent: number;
    win_rate: number;
    total_net_pnl: number | null;
    pnl_quote_currency: string | null;
    preliminary: boolean;
  }>;
};
export type AutoTraderExecutionQuality = {
  order_samples: number;
  filled_order_samples: number;
  partial_fill_samples: number;
  unfilled_order_samples: number;
  measured_fill_samples: number;
  reference_price_coverage_percent?: number;
  average_fill_rate?: number;
  average_implementation_shortfall_bps?: number;
  average_fee_bps?: number;
  average_all_in_cost_bps?: number;
  average_submit_response_ms?: number;
  response_latency_samples: number;
  history_truncated: boolean;
  last_updated_at?: number;
};
const emptyExecutionQuality: AutoTraderExecutionQuality = {
  order_samples: 0,
  filled_order_samples: 0,
  partial_fill_samples: 0,
  unfilled_order_samples: 0,
  measured_fill_samples: 0,
  response_latency_samples: 0,
  history_truncated: false,
};
const emptyDecisionQuality: AutoTraderDecisionQuality = {
  decision_samples: 0,
  filled_entry_samples: 0,
  unresolved_trade_samples: 0,
  unpriced_closed_trade_samples: 0,
  completed_trade_samples: 0,
  history_truncated: false,
  cohorts: [],
};

type TraderStatus = {
  enabled: boolean;
  audit_healthy?: boolean;
  audit_failure_latched?: boolean;
  decision_audit_queue_depth?: number;
  execution_audit_queue_depth?: number;
  audit_coalesced_decisions?: number;
  audit_dropped_events?: number;
  paper_learning_exploration_until_ms?: number;
  strategy?: Strategy;
  strategies?: Strategy[];
  signal_strategies_enabled?: boolean;
  max_order_notional: number;
  daily_notional_limit: number;
  daily_realized_loss_limit: number;
  exploration_percent: number;
  risk_appetite?: number;
  max_open_positions: number;
  max_intraday_open_positions: number;
  max_daily_open_positions: number;
  capital_allocation?: CapitalAllocation;
  capitalAllocation?: CapitalAllocation;
  max_entries_per_auto_position: number;
  stop_loss_percent: number;
  take_profit_percent: number;
  vwap_duration_minutes: number;
  vwap_participation_percent: number;
  limit_offset_bps: number;
  trigger_offset_bps: number;
  trailing_distance_percent: number;
  closed_trade_window?: number;
  minimum_scout_parameter_samples?: number;
  minimum_live_scout_parameter_samples?: number;
  used_notional_today: number;
  available_notional: number;
  last_decision?: string;
  last_decision_at?: number;
  decisions_since_trade?: number;
  gap_reasons?: Array<{ reason: string; count: number }>;
  gate_hits?: Array<{ gate: string; count: number }>;
  symbol_gate_hits?: Array<{
    symbol: string;
    direction: string;
    gate: string;
    count: number;
    last_detail: string;
    last_seen_ms?: number;
    distance_to_pass?: number | null;
  }>;
  latest_signal_diagnostic?: string;
  latest_entry_diagnostic?: LatestEntryDiagnostic | null;
  entry_diagnostic_history?: LatestEntryDiagnostic[];
  latest_symbol_entry_results?: SymbolEntryResult[];
  strategy_diagnostics?: StrategyDiagnostic[];
  latest_strategy_blockers?: StrategyBlocker[];
};
type TraderActivity = {
  orders: AutoTraderOrder[];
  open_position_symbols?: string[];
  realized_pnl?: number | null;
  realized_pnl_by_quote?: Record<string, number> | null;
  unrealized_pnl?: number | null;
  unrealized_pnl_by_quote?: Record<string, number> | null;
  exit_learning_samples: number;
  closed_trade_average_return_percent?: number;
  closed_trade_wins?: number;
  closed_trade_losses?: number;
  closed_trade_flat?: number;
  closed_trade_realized_usd?: number;
  closed_trade_fees_usd?: number;
  learning_samples: number;
  learning_win_rate?: number;
  preferred_order_type?: string;
  parameter_profile_samples?: number[];
};

export class AutoTraderModel {
  readonly state$ = new BehaviorSubject<AutoTraderState>({
    enabled: false,
    auditHealthy: true,
    auditFailureLatched: false,
    decisionAuditQueueDepth: 0,
    executionAuditQueueDepth: 0,
    auditCoalescedDecisions: 0,
    auditDroppedEvents: 0,
    paperLearningExplorationUntilMs: 0,
    filledPaperOrdersSinceRecoveryReset: 0,
    strategies: [
      'donchian-55-btc200',
      'live-momentum',
      'live-trade-flow',
      'live-order-book',
      'live-vwap-dislocation',
      'live-absorption-pressure',
      'live-range-mean-reversion',
    ],
    scoutStrategies: defaultScoutStrategies,
    loading: true,
    busy: false,
    maxOrderNotional: '100',
    dailyNotionalLimit: '1000',
    usedNotionalToday: 0,
    availableNotional: 1000,
    serverDailyNotionalLimit: 1000,
    serverExplorationPercent: 10,
    lastDecision: '',
    lastDecisionAt: 0,
    decisionsSinceTrade: 0,
    gapReasons: [],
    gateHits: [],
    symbolGateHits: [],
    latestSignalDiagnostic: '',
    latestEntryDiagnostic: null,
    entryDiagnosticHistory: [],
    latestSymbolEntryResults: [],
    strategyDiagnostics: [],
    latestStrategyBlockers: [],
    orders: [],
    openPositionSymbols: [],
    ledgerOpenPositionSymbols: [],
    executionQuality: emptyExecutionQuality,
    decisionQuality: emptyDecisionQuality,
    scoutOpenPositions: 0,
    pendingScoutParameters: [],
    scoutLearningSamples: 0,
    scoutLearningRealizedPnl: 0,
    realizedPnl: 0,
    unrealizedPnl: 0,
    exitLearningSamples: 0,
    closedTradeWins: 0,
    closedTradeLosses: 0,
    closedTradeFlat: 0,
    dailyRealizedLossLimit: '10',
    explorationPercent: '2',
    riskAppetite: '50',
    maxOpenPositions: '20',
    maxConcurrentScoutSamples: '5000',
    scoutMaxActivePositions: '5000',
    maxIntradayOpenPositions: '10',
    maxDailyOpenPositions: '10',
    capitalAllocation: undefined,
    maxEntriesPerPosition: '1',
    stopLossPercent: 0.5,
    takeProfitPercent: 0.8,
    vwapDurationMinutes: '5',
    vwapParticipationPercent: '20',
    limitOffsetBps: '0',
    triggerOffsetBps: '20',
    trailingDistancePercent: '0.10',
    closedTradeWindow: '200',
    minimumScoutParameterSamples: '6',
    learningSamples: 0,
    parameterProfileSamples: [0, 0, 0],
    strategyParameterOverrides: {},
  });
  private timer?: ReturnType<typeof setInterval>;
  private workerDecisionStatusTimer?: ReturnType<typeof setInterval>;
  private startUsers = 0;
  private limitsDirty = false;
  private categoryLimitsDirty = false;
  private executionParametersDirty = false;
  private strategyParametersDirty = false;
  private refreshInFlight = false;
  private workerDecisionStatusRefreshInFlight = false;
  private usdRates: Record<string, number> = {};
  private stopPositionStream?: () => void;
  private stopAutoTraderStream?: () => void;
  private stopInsightsStream?: () => void;
  private streamRefreshTimer?: ReturnType<typeof setTimeout>;
  private lastStreamRefreshAt = 0;
  private decisionParametersRefreshInFlight = false;
  private decisionParametersAttemptAt = 0;
  private autoTraderSnapshot?: AutoTraderUiSnapshot;
  private autoTraderSnapshotAt = 0;
  start() {
    this.startUsers += 1;
    if (this.timer) return;
    this.usdRates = currentPositionSnapshot()?.usd_rates?.rates ?? {};
    this.stopPositionStream = subscribePositionStream((snapshot) => {
      const streamedRates = snapshot.usd_rates?.rates;
      if (streamedRates && Object.keys(streamedRates).length > 0) {
        this.usdRates = { ...this.usdRates, ...streamedRates };
      }
      if (this.autoTraderSnapshot) this.scheduleStreamRefresh();
    });
    this.autoTraderSnapshot = currentAutoTraderSnapshot();
    if (this.autoTraderSnapshot) this.autoTraderSnapshotAt = Date.now();
    this.stopAutoTraderStream = subscribeAutoTraderStream((snapshot) => {
      this.autoTraderSnapshot = snapshot;
      this.autoTraderSnapshotAt = Date.now();
      const decisionParameters = snapshot.parameters as
        | AutoTraderDecisionParameters
        | undefined;
      if (!decisionParameters) {
        void this.refreshDecisionParameters();
        this.scheduleStreamRefresh();
        return;
      }
      const savedOverrides = Object.fromEntries(
        decisionParameters.strategy_profiles.flatMap((profile) =>
          profile.parameters
            .filter(
              (parameter) =>
                parameter.configured_value !== undefined &&
                parameter.configured_value !== null,
            )
            .map(
              (parameter) =>
                [
                  profile.symbol + ':' + profile.strategy + ':' + parameter.key,
                  parameter.configured_value!,
                ] as const,
            ),
        ),
      );
      this.state$.next({
        ...this.state$.value,
        decisionParameters,
        strategyParameterOverrides: this.strategyParametersDirty
          ? this.state$.value.strategyParameterOverrides
          : savedOverrides,
        decisionParametersError: undefined,
      });
      this.scheduleStreamRefresh();
    });
    this.stopInsightsStream = subscribeInsightsStream((snapshot) => {
      this.state$.next({
        ...this.state$.value,
        executionQuality:
          (snapshot.execution_quality as AutoTraderExecutionQuality) ??
          this.state$.value.executionQuality,
        decisionQuality:
          (snapshot.decision_quality as AutoTraderDecisionQuality) ??
          this.state$.value.decisionQuality,
      });
    });
    void Promise.all([waitForAutoTraderSnapshot(), waitForPositionSnapshot()])
      .then(() => {
        if (this.timer) void this.refresh();
      })
      .catch(() => {
        if (this.timer) {
          void this.refreshDecisionParameters();
          void this.refresh();
        }
      });
    this.timer = setInterval(() => {
      if (!isAutoTraderSnapshotFresh(this.autoTraderSnapshotAt))
        void this.refresh();
    }, 30_000);
    void this.refreshWorkerDecisionStatus();
    this.workerDecisionStatusTimer = setInterval(() => {
      void this.refreshWorkerDecisionStatus();
    }, WORKER_DECISION_STATUS_POLL_MS);
  }
  stop() {
    this.startUsers = Math.max(0, this.startUsers - 1);
    if (this.startUsers > 0) return;
    if (this.timer) clearInterval(this.timer);
    this.timer = undefined;
    if (this.workerDecisionStatusTimer)
      clearInterval(this.workerDecisionStatusTimer);
    this.workerDecisionStatusTimer = undefined;
    if (this.streamRefreshTimer) clearTimeout(this.streamRefreshTimer);
    this.streamRefreshTimer = undefined;
    this.stopPositionStream?.();
    this.stopPositionStream = undefined;
    this.stopAutoTraderStream?.();
    this.stopAutoTraderStream = undefined;
    this.stopInsightsStream?.();
    this.stopInsightsStream = undefined;
  }

  setLimits(
    maxOrderNotional: string,
    dailyNotionalLimit: string,
    dailyRealizedLossLimit: string,
    explorationPercent: string,
    maxOpenPositions: string,
  ) {
    this.limitsDirty = true;
    this.state$.next({
      ...this.state$.value,
      maxOrderNotional,
      dailyNotionalLimit,
      dailyRealizedLossLimit,
      explorationPercent,
      maxOpenPositions,
    });
  }
  setCategoryLimits(
    values: Partial<
      Pick<
        AutoTraderState,
        'maxIntradayOpenPositions' | 'maxDailyOpenPositions'
      >
    >,
  ) {
    this.categoryLimitsDirty = true;
    this.state$.next({ ...this.state$.value, ...values });
  }

  setRiskAppetite(riskAppetite: string) {
    this.limitsDirty = true;
    this.state$.next({ ...this.state$.value, riskAppetite });
  }

  setMaxConcurrentScoutSamples(maxConcurrentScoutSamples: string) {
    this.limitsDirty = true;
    this.state$.next({ ...this.state$.value, maxConcurrentScoutSamples });
  }
  setScoutMaxActivePositions(scoutMaxActivePositions: string) {
    this.limitsDirty = true;
    this.state$.next({ ...this.state$.value, scoutMaxActivePositions });
  }

  setScoutStrategies(scoutStrategies: Strategy[]) {
    this.limitsDirty = true;
    this.state$.next({ ...this.state$.value, scoutStrategies });
  }

  setStrategies(strategies: Strategy[]) {
    this.limitsDirty = true;
    this.state$.next({ ...this.state$.value, strategies });
  }

  setClosedTradeWindow(closedTradeWindow: string) {
    this.limitsDirty = true;
    this.state$.next({ ...this.state$.value, closedTradeWindow });
  }

  setMinimumScoutParameterSamples(minimumScoutParameterSamples: string) {
    this.limitsDirty = true;
    this.state$.next({
      ...this.state$.value,
      minimumScoutParameterSamples,
    });
  }
  setMaxEntriesPerPosition(maxEntriesPerPosition: string) {
    this.limitsDirty = true;
    this.state$.next({ ...this.state$.value, maxEntriesPerPosition });
  }

  setExecutionParameters(
    values: Partial<
      Pick<
        AutoTraderState,
        | 'vwapDurationMinutes'
        | 'vwapParticipationPercent'
        | 'limitOffsetBps'
        | 'triggerOffsetBps'
        | 'trailingDistancePercent'
      >
    >,
  ) {
    this.executionParametersDirty = true;
    this.state$.next({ ...this.state$.value, ...values });
  }

  setStrategyParameter(key: string, value?: number) {
    const overrides = { ...this.state$.value.strategyParameterOverrides };
    if (value === undefined) delete overrides[key];
    else overrides[key] = value;
    this.strategyParametersDirty = true;
    this.state$.next({
      ...this.state$.value,
      strategyParameterOverrides: overrides,
    });
  }

  async refresh() {
    if (this.refreshInFlight) return;
    this.refreshInFlight = true;
    try {
      let status: TraderStatus;
      let activity: TraderActivity;
      let positions: { positions: Array<{ symbol: string; quantity: number }> };
      const autoSnapshot = this.autoTraderSnapshot;
      const useStreamSnapshot =
        !!autoSnapshot && isAutoTraderSnapshotFresh(this.autoTraderSnapshotAt);
      if (useStreamSnapshot && autoSnapshot) {
        status = autoSnapshot.status as TraderStatus;
        activity = autoSnapshot.activity as TraderActivity;
        const autoPositions =
          currentPositionSnapshot()?.auto_positions.positions ?? [];
        positions = {
          positions: autoPositions.map(({ symbol, quantity }) => ({
            symbol,
            quantity,
          })),
        };
      } else {
        const [statusResponse, activityResponse, positionsResponse] =
          await Promise.all([
            autoTraderRpc('GET', '/positions/auto-trader'),
            autoTraderRpc('GET', '/positions/auto-trader/activity'),
            sharedGet('/positions?source=auto'),
          ]);
        if (!statusResponse.ok)
          throw await this.responseError('status', statusResponse);
        if (!activityResponse.ok)
          throw await this.responseError('activity', activityResponse);
        if (!positionsResponse.ok)
          throw await this.responseError('positions', positionsResponse);
        status = (await statusResponse.json()) as TraderStatus;
        activity = (await activityResponse.json()) as TraderActivity;
        positions = (await positionsResponse.json()) as {
          positions: Array<{ symbol: string; quantity: number }>;
        };
      }
      let usdRates = this.usdRates;
      const quoteTotals = [
        activity.realized_pnl_by_quote,
        activity.unrealized_pnl_by_quote,
      ];
      const hasMissingRate = quoteTotals.some(
        (totals) =>
          totals &&
          Object.keys(totals).some(
            (quote) => quote !== 'USD' && !(usdRates[quote] > 0),
          ),
      );
      if (hasMissingRate) {
        try {
          const ratesResponse = await sharedGet('/positions/usd-rates', 30_000);
          if (ratesResponse.ok) {
            const ratesPayload = (await ratesResponse.json()) as {
              rates?: Record<string, number>;
            };
            if (ratesPayload.rates) {
              usdRates = { ...usdRates, ...ratesPayload.rates };
              this.usdRates = usdRates;
            }
          }
        } catch {
          /* Keep cached rates; missing currencies remain unavailable. */
        }
      }
      const scoutConfigResponse = await learningRpc(
        'GET',
        '/configuration',
      ).catch(() => undefined);
      const scoutConfiguration = scoutConfigResponse?.ok
        ? ((await scoutConfigResponse.json()) as {
            strategies: Strategy[];
            max_active_samples: number;
            max_active_positions: number;
          })
        : {
            strategies: this.state$.value.scoutStrategies,
            max_active_samples: Number(
              this.state$.value.maxConcurrentScoutSamples,
            ),
            max_active_positions: Number(
              this.state$.value.scoutMaxActivePositions,
            ),
          };
      const scoutStatusResponse = await learningRpc('GET', '/status').catch(
        () => undefined,
      );
      const scoutStatus = scoutStatusResponse?.ok
        ? ((await scoutStatusResponse.json()) as {
            active_samples: number;
            learning_samples: number;
            win_rate?: number;
            realized_pnl_usd: number;
          })
        : {
            active_samples: this.state$.value.scoutOpenPositions,
            learning_samples: this.state$.value.scoutLearningSamples,
            win_rate: this.state$.value.scoutLearningWinRate,
            realized_pnl_usd: this.state$.value.scoutLearningRealizedPnl,
          };
      const savedStrategies = status.strategies ?? [
        status.strategy ?? 'donchian-55-btc200',
      ];
      if (
        status.signal_strategies_enabled &&
        !savedStrategies.some((strategy) => strategy.startsWith('live-'))
      )
        savedStrategies.push(
          'live-momentum',
          'live-trade-flow',
          'live-order-book',
          'live-vwap-dislocation',
          'live-absorption-pressure',
          'live-range-mean-reversion',
        );
      const ledgerOpenPositionSymbols = positions.positions
        .filter((position) => Math.abs(position.quantity) > 1e-12)
        .map((position) => position.symbol);
      const rateForQuote = (quote: string) =>
        quote === 'USD' ? 1 : usdRates[quote];
      const convertQuoteTotals = (totals?: Record<string, number> | null) => {
        if (!totals) return null;
        let converted = 0;
        for (const [quote, amount] of Object.entries(totals)) {
          const rate = rateForQuote(quote);
          if (typeof rate !== 'number' || !Number.isFinite(rate)) return null;
          converted += amount * rate;
        }
        return converted;
      };
      const orders = activity.orders.map((order) => {
        const quote = order.symbol.split('-')[1];
        const rate = quote ? rateForQuote(quote) : undefined;
        return {
          ...order,
          realized_pnl_usd:
            typeof rate === 'number' && Number.isFinite(rate)
              ? order.realized_pnl * rate
              : null,
        };
      });
      const paperLearningExplorationUntilMs =
        status.paper_learning_exploration_until_ms ?? 0;
      const recoveryResetAtMs = paperLearningExplorationUntilMs
        ? paperLearningExplorationUntilMs - 3 * 60 * 60 * 1000
        : Number.POSITIVE_INFINITY;
      const filledPaperOrdersSinceRecoveryReset = orders.filter(
        (order) =>
          order.timestamp >= recoveryResetAtMs && order.filled_quantity > 0,
      ).length;
      this.state$.next({
        ...this.state$.value,
        enabled: status.enabled,
        auditHealthy: status.audit_healthy ?? true,
        auditFailureLatched: status.audit_failure_latched ?? false,
        decisionAuditQueueDepth: status.decision_audit_queue_depth ?? 0,
        executionAuditQueueDepth: status.execution_audit_queue_depth ?? 0,
        auditCoalescedDecisions: status.audit_coalesced_decisions ?? 0,
        auditDroppedEvents: status.audit_dropped_events ?? 0,
        paperLearningExplorationUntilMs,
        filledPaperOrdersSinceRecoveryReset,
        strategies: this.limitsDirty
          ? this.state$.value.strategies
          : savedStrategies,
        scoutStrategies: this.limitsDirty
          ? this.state$.value.scoutStrategies
          : scoutConfiguration.strategies,
        maxOrderNotional: this.limitsDirty
          ? this.state$.value.maxOrderNotional
          : String(status.max_order_notional),
        dailyNotionalLimit: this.limitsDirty
          ? this.state$.value.dailyNotionalLimit
          : String(status.daily_notional_limit),
        usedNotionalToday: status.used_notional_today,
        availableNotional: status.available_notional,
        serverDailyNotionalLimit: status.daily_notional_limit,
        serverExplorationPercent: status.exploration_percent,
        dailyRealizedLossLimit: this.limitsDirty
          ? this.state$.value.dailyRealizedLossLimit
          : String(status.daily_realized_loss_limit),
        explorationPercent: this.limitsDirty
          ? this.state$.value.explorationPercent
          : String(status.exploration_percent),
        riskAppetite: this.limitsDirty
          ? this.state$.value.riskAppetite
          : String(status.risk_appetite ?? 50),
        maxOpenPositions: this.limitsDirty
          ? this.state$.value.maxOpenPositions
          : String(status.max_open_positions),
        maxConcurrentScoutSamples: this.limitsDirty
          ? this.state$.value.maxConcurrentScoutSamples
          : String(scoutConfiguration.max_active_samples),
        scoutMaxActivePositions: this.limitsDirty
          ? this.state$.value.scoutMaxActivePositions
          : String(scoutConfiguration.max_active_positions),
        maxIntradayOpenPositions: this.categoryLimitsDirty
          ? this.state$.value.maxIntradayOpenPositions
          : String(status.max_intraday_open_positions),
        maxDailyOpenPositions: this.categoryLimitsDirty
          ? this.state$.value.maxDailyOpenPositions
          : String(status.max_daily_open_positions),
        capitalAllocation:
          status.capitalAllocation ?? status.capital_allocation,
        maxEntriesPerPosition: this.limitsDirty
          ? this.state$.value.maxEntriesPerPosition
          : String(status.max_entries_per_auto_position),
        stopLossPercent: status.stop_loss_percent,
        takeProfitPercent: status.take_profit_percent,
        vwapDurationMinutes: this.executionParametersDirty
          ? this.state$.value.vwapDurationMinutes
          : String(status.vwap_duration_minutes),
        vwapParticipationPercent: this.executionParametersDirty
          ? this.state$.value.vwapParticipationPercent
          : String(status.vwap_participation_percent),
        limitOffsetBps: this.executionParametersDirty
          ? this.state$.value.limitOffsetBps
          : String(status.limit_offset_bps),
        triggerOffsetBps: this.executionParametersDirty
          ? this.state$.value.triggerOffsetBps
          : String(status.trigger_offset_bps),
        trailingDistancePercent: this.executionParametersDirty
          ? this.state$.value.trailingDistancePercent
          : String(status.trailing_distance_percent),
        closedTradeWindow: this.limitsDirty
          ? this.state$.value.closedTradeWindow
          : String(status.closed_trade_window ?? 200),
        minimumScoutParameterSamples: this.limitsDirty
          ? this.state$.value.minimumScoutParameterSamples
          : String(
              status.minimum_scout_parameter_samples ??
                status.minimum_live_scout_parameter_samples ??
                6,
            ),
        lastDecision: status.last_decision ?? '',
        lastDecisionAt: status.last_decision_at ?? 0,
        decisionsSinceTrade: status.decisions_since_trade ?? 0,
        gapReasons: status.gap_reasons ?? [],
        gateHits: status.gate_hits ?? [],
        symbolGateHits:
          status.symbol_gate_hits?.map((hit) => ({
            ...hit,
            distance_to_pass: hit.distance_to_pass ?? null,
          })) ?? [],
        latestSignalDiagnostic: status.latest_signal_diagnostic ?? '',
        latestEntryDiagnostic: status.latest_entry_diagnostic ?? null,
        entryDiagnosticHistory: status.entry_diagnostic_history ?? [],
        latestSymbolEntryResults: status.latest_symbol_entry_results ?? [],
        strategyDiagnostics: projectStrategyDiagnostics(
          status.strategy_diagnostics,
        ),
        latestStrategyBlockers: status.latest_strategy_blockers ?? [],
        orders,
        openPositionSymbols: activity.open_position_symbols ?? [],
        ledgerOpenPositionSymbols,
        scoutOpenPositions: scoutStatus.active_samples,
        scoutLearningSamples: scoutStatus.learning_samples,
        scoutLearningWinRate: scoutStatus.win_rate,
        scoutLearningRealizedPnl: scoutStatus.realized_pnl_usd,
        realizedPnl: convertQuoteTotals(activity.realized_pnl_by_quote),
        unrealizedPnl: convertQuoteTotals(activity.unrealized_pnl_by_quote),
        exitLearningSamples: activity.exit_learning_samples,
        closedTradeAverageReturnPercent:
          activity.closed_trade_average_return_percent,
        closedTradeWins: activity.closed_trade_wins ?? 0,
        closedTradeLosses: activity.closed_trade_losses ?? 0,
        closedTradeFlat: activity.closed_trade_flat ?? 0,
        closedTradeRealizedUsd: activity.closed_trade_realized_usd,
        closedTradeFeesUsd: activity.closed_trade_fees_usd,
        learningSamples: activity.learning_samples,
        learningWinRate: activity.learning_win_rate,
        preferredOrderType: activity.preferred_order_type,
        parameterProfileSamples: activity.parameter_profile_samples ?? [
          0, 0, 0,
        ],
        loading: false,
        error: undefined,
      });
    } catch (error) {
      console.error('Auto Trader status/activity refresh failed:', error);
      this.state$.next({
        ...this.state$.value,
        loading: false,
        error:
          error instanceof Error
            ? error.message
            : 'Could not load Auto Trader status and activity.',
      });
    } finally {
      this.refreshInFlight = false;
    }
  }

  async refreshDecisionParameters() {
    if (this.decisionParametersRefreshInFlight) return;
    if (Date.now() - this.decisionParametersAttemptAt < 30_000) return;
    this.decisionParametersAttemptAt = Date.now();
    this.decisionParametersRefreshInFlight = true;
    try {
      const streamedParameters = this.autoTraderSnapshot?.parameters as
        | AutoTraderDecisionParameters
        | undefined;
      const [response, scoutResponse] = await Promise.all([
        streamedParameters
          ? Promise.resolve(undefined)
          : autoTraderRpc('GET', '/positions/auto-trader/parameters/v1'),
        this.state$.value.decisionParameters
          ? Promise.resolve(undefined)
          : autoTraderRpc(
              'GET',
              '/positions/auto-trader/parameters/scout-variants',
            ).catch(() => undefined),
      ]);
      if (!streamedParameters && !response?.ok) {
        if (response)
          throw await this.responseError('decision parameters', response);
        throw new Error('Could not load Auto Trader decision parameters.');
      }
      const receivedParameters =
        streamedParameters ??
        ((await response!.json()) as AutoTraderDecisionParameters);
      const decisionParameters =
        this.expandCompactParameters(receivedParameters);
      const previousScoutDetails =
        this.state$.value.decisionParameters?.strategy_profiles;
      if (scoutResponse?.ok) {
        const scoutDetails = (await scoutResponse.json()) as {
          scout_variants?: Record<string, AutoTraderStrategyScoutVariant[]>;
        };
        for (const profile of decisionParameters.strategy_profiles)
          for (const parameter of profile.parameters) {
            parameter.scout_variants =
              scoutDetails.scout_variants?.[
                `${profile.symbol}:${profile.strategy}:${parameter.key}`
              ] ?? [];
          }
      } else if (previousScoutDetails) {
        const previous = new Map(
          previousScoutDetails.flatMap((profile) =>
            profile.parameters.map(
              (parameter) =>
                [
                  `${profile.symbol}:${profile.strategy}:${parameter.key}`,
                  parameter.scout_variants,
                ] as const,
            ),
          ),
        );
        for (const profile of decisionParameters.strategy_profiles)
          for (const parameter of profile.parameters) {
            parameter.scout_variants =
              previous.get(
                `${profile.symbol}:${profile.strategy}:${parameter.key}`,
              ) ?? [];
          }
      }
      const pendingScoutParameters = this.state$.value.pendingScoutParameters;
      const savedOverrides = Object.fromEntries(
        decisionParameters.strategy_profiles.flatMap((profile) =>
          profile.parameters
            .filter(
              (parameter) =>
                parameter.configured_value !== undefined &&
                parameter.configured_value !== null,
            )
            .map(
              (parameter) =>
                [
                  profile.symbol + ':' + profile.strategy + ':' + parameter.key,
                  parameter.configured_value!,
                ] as const,
            ),
        ),
      );
      this.state$.next({
        ...this.state$.value,
        decisionParameters,
        pendingScoutParameters,
        decisionParametersError: undefined,
        strategyParameterOverrides: this.strategyParametersDirty
          ? this.state$.value.strategyParameterOverrides
          : savedOverrides,
      });
    } catch (error) {
      this.state$.next({
        ...this.state$.value,
        decisionParametersError:
          error instanceof Error
            ? error.message
            : 'Could not load Auto Trader decision parameters.',
      });
    } finally {
      this.decisionParametersRefreshInFlight = false;
    }
  }

  private expandCompactParameters(
    parameters: AutoTraderDecisionParameters,
  ): AutoTraderDecisionParameters {
    if (parameters.version !== 1 || !parameters.catalog) return parameters;
    return {
      ...parameters,
      strategy_profiles: parameters.strategy_profiles.map((profile) => {
        const schema = parameters.catalog?.[profile.strategy] ?? [];
        return {
          ...profile,
          parameters: Object.entries(
            profile.parameters as unknown as Record<
              string,
              {
                value: number;
                learned_value: number;
                configured_value?: number | null;
                overridden: boolean;
              }
            >,
          ).map(([key, value]) => {
            const entry = schema.find((item) => item.key === key);
            return {
              ...entry,
              ...value,
              key,
              label: entry?.label ?? key,
              default_value: entry?.default_value ?? value.value,
              min: entry?.min ?? value.value,
              max: entry?.max ?? value.value,
              step: entry?.step ?? 0,
              unit: entry?.unit ?? '',
              scout_variants: [],
            };
          }),
        };
      }),
    };
  }

  private scheduleStreamRefresh() {
    if (this.streamRefreshTimer) return;
    const delay = Math.max(
      250,
      10_000 - (Date.now() - this.lastStreamRefreshAt),
    );
    this.streamRefreshTimer = setTimeout(() => {
      this.streamRefreshTimer = undefined;
      this.lastStreamRefreshAt = Date.now();
      void this.refresh();
    }, delay);
  }

  private async refreshWorkerDecisionStatus() {
    if (this.workerDecisionStatusRefreshInFlight) return;
    this.workerDecisionStatusRefreshInFlight = true;
    try {
      const response = await autoTraderRpc('GET', '/positions/auto-trader');
      if (!response.ok) return;
      const status = (await response.json()) as Pick<
        TraderStatus,
        'latest_symbol_entry_results' | 'latest_strategy_blockers'
      >;
      const current = this.state$.value;
      const resultsChanged =
        status.latest_symbol_entry_results?.some((result, index) => {
          const previous = current.latestSymbolEntryResults[index];
          return (
            !previous ||
            result.strategy !== previous.strategy ||
            result.symbol !== previous.symbol ||
            result.profile_id !== previous.profile_id ||
            result.event_id !== previous.event_id ||
            result.decision !== previous.decision ||
            result.reason !== previous.reason ||
            result.timestamp_ms !== previous.timestamp_ms
          );
        }) ?? false;
      const resultCountChanged =
        status.latest_symbol_entry_results !== undefined &&
        status.latest_symbol_entry_results.length !==
          current.latestSymbolEntryResults.length;
      const blockersChanged =
        status.latest_strategy_blockers?.some((blocker, index) => {
          const previous = current.latestStrategyBlockers[index];
          return (
            !previous ||
            blocker.strategy !== previous.strategy ||
            blocker.symbol !== previous.symbol ||
            blocker.reason !== previous.reason ||
            blocker.timestamp_ms !== previous.timestamp_ms
          );
        }) ?? false;
      const blockerCountChanged =
        status.latest_strategy_blockers !== undefined &&
        status.latest_strategy_blockers.length !==
          current.latestStrategyBlockers.length;
      if (
        !resultsChanged &&
        !resultCountChanged &&
        !blockersChanged &&
        !blockerCountChanged
      )
        return;
      this.state$.next({
        ...current,
        latestSymbolEntryResults:
          status.latest_symbol_entry_results ??
          current.latestSymbolEntryResults,
        latestStrategyBlockers:
          status.latest_strategy_blockers ?? current.latestStrategyBlockers,
      });
    } catch {
      // The normal refresh path reports broader service errors; retry this
      // lightweight status request on the next two-second tick.
    } finally {
      this.workerDecisionStatusRefreshInFlight = false;
    }
  }

  private async responseError(
    endpoint:
      | 'status'
      | 'activity'
      | 'positions'
      | 'execution quality'
      | 'decision quality'
      | 'decision parameters',
    response: Response,
  ) {
    const detail = (await response.text()).trim();
    return new Error(
      'Could not load Auto Trader ' +
        endpoint +
        ' (' +
        response.status +
        ')' +
        (detail ? ': ' + detail : '.'),
    );
  }

  async setEnabled(enabled: boolean) {
    this.state$.next({ ...this.state$.value, busy: true, error: undefined });
    try {
      const response = await autoTraderFetch('/positions/auto-trader', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ enabled }),
      });
      if (!response.ok) throw new Error();
      await this.refresh();
      this.state$.next({ ...this.state$.value, busy: false });
    } catch {
      this.state$.next({
        ...this.state$.value,
        busy: false,
        error: 'Could not change Auto Trader state.',
      });
    }
  }

  async saveLimits(
    resetLearning = false,
    resetStrategyProfile?: { symbol: string; strategy: Strategy },
  ) {
    const current = this.state$.value;
    this.state$.next({ ...current, busy: true, error: undefined });
    try {
      const response = await autoTraderFetch(
        '/positions/auto-trader/settings',
        {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            strategies: current.strategies,
            signal_strategies_enabled: current.strategies.some((strategy) =>
              strategy.startsWith('live-'),
            ),
            max_order_notional: Number(current.maxOrderNotional),
            daily_notional_limit: Number(current.dailyNotionalLimit),
            daily_realized_loss_limit: Number(current.dailyRealizedLossLimit),
            exploration_percent: Number(current.explorationPercent),
            risk_appetite: Number(current.riskAppetite),
            max_open_positions: Number(current.maxOpenPositions),
            max_intraday_open_positions: Number(
              current.maxIntradayOpenPositions,
            ),
            max_daily_open_positions: Number(current.maxDailyOpenPositions),
            max_entries_per_auto_position: Number(
              current.maxEntriesPerPosition,
            ),
            vwap_duration_minutes: Number(current.vwapDurationMinutes),
            vwap_participation_percent: Number(
              current.vwapParticipationPercent,
            ),
            limit_offset_bps: Number(current.limitOffsetBps),
            trigger_offset_bps: Number(current.triggerOffsetBps),
            trailing_distance_percent: Number(current.trailingDistancePercent),
            closed_trade_window: Number(current.closedTradeWindow),
            minimum_scout_parameter_samples: Number(
              current.minimumScoutParameterSamples,
            ),
            ...(this.strategyParametersDirty
              ? {
                  strategy_parameter_overrides:
                    current.strategyParameterOverrides,
                }
              : {}),
            ...(resetStrategyProfile
              ? { reset_strategy_profile: resetStrategyProfile }
              : {}),
            reset_learning: resetLearning,
          }),
        },
      );
      if (!response.ok) throw new Error(await response.text());
      const scoutResponse = await learningRpc('POST', '/configuration', {
        strategies: current.scoutStrategies,
        max_active_samples: Number(current.maxConcurrentScoutSamples),
        max_active_positions: Number(current.scoutMaxActivePositions),
      });
      if (!scoutResponse.ok) throw new Error(await scoutResponse.text());
      // A refresh may already be in flight with a snapshot from before this PUT.
      // Let it finish while the dirty flags still protect the user's edits, then
      // fetch again so the saved values are the ones applied to the form.
      while (this.refreshInFlight)
        await new Promise((resolve) => setTimeout(resolve, 10));
      // The stream snapshot can predate the PUT, so force this refresh to read the server.
      this.autoTraderSnapshotAt = 0;
      await this.refresh();
      this.limitsDirty = false;
      this.categoryLimitsDirty = false;
      this.executionParametersDirty = false;
      this.strategyParametersDirty = false;
      this.state$.next({ ...this.state$.value, busy: false });
    } catch (error) {
      this.state$.next({
        ...this.state$.value,
        busy: false,
        error:
          error instanceof Error ? error.message : 'Could not save limits.',
      });
    }
  }

  async resetLearning() {
    await this.saveLimits(true);
  }

  async resetStrategyProfile(symbol: string, strategy: Strategy) {
    await this.saveLimits(false, { symbol, strategy });
  }
}

// Settings controls and strategy status must share one live settings snapshot.
export const sharedAutoTraderSettingsModel = new AutoTraderModel();
