import { BehaviorSubject, Subscription } from 'rxjs';
import type {
  AutoTraderState,
  AutoTraderStrategyProfile,
  AutoTraderStrategyParameter,
  ExitPolicyMetricCoverageStatus,
  ExitPolicyMetricReport,
  LearnedExitPolicyAssessment,
  LearnedExitProtectionPolicy,
} from './AutoTraderModel';
import type {
  AutoTraderOutcome,
  AutoTraderOutcomesModel,
  ExecutablePricePathEvidence,
  InitialRiskEvidence,
} from './AutoTraderOutcomesModel';

type OutcomesState = AutoTraderOutcomesModel['state$']['value'];
type LifecycleSource<T> = {
  state$: BehaviorSubject<T>;
  start: () => void;
  stop: () => void;
};

export type ExitParameterKind =
  | 'stop'
  | 'target'
  | 'maximumHolding'
  | 'trailing'
  | 'profitProtection'
  | 'plannedRewardRisk'
  | 'otherExit';

export type ExitProfileParameter = {
  key: string;
  label: string;
  unit: string;
  kind: ExitParameterKind;
  rawServiceValue?: number;
  configuredValue?: number;
  defaultValue?: number;
  scoutVariantsAvailable: boolean;
  scoutVariants: Array<{
    candidateValue: number;
    sampleCount: number;
    promoted: boolean;
    rejected: boolean;
  }>;
};

export type ExitProfileRow = {
  id: string;
  symbol: string;
  strategy: string;
  learningSamples: number;
  positiveRate?: number;
  parameters: ExitProfileParameter[];
  profileId?: string;
  profileVersion?: number;
  exchange: undefined;
  supportedDirection: undefined;
  effectiveSampleSize?: number;
  oosFoldCount?: number;
  spaPValue?: number;
  lastEvidenceSampleTimestampMs?: number;
  regimeEvidenceKeys: string[];
  finalHoldoutStatus: 'validated' | 'failed' | 'not_validated' | 'unavailable';
  historyCoverage: 'complete' | 'partial' | 'unknown' | 'unavailable';
  costCoverage: 'complete' | 'partial' | 'unknown' | 'unavailable';
};

export type ExitPositionRow = {
  id: string;
  status: 'Open';
  symbol: string;
  strategy: string;
  decisionId: string;
  openedAt: number;
  entryProfileId?: string;
  entryProfileHash?: string;
  entryProfileVersion?: number;
  exitProfileId?: string;
  exitProfileVersion?: number;
  exitPolicyAsOfMs?: number;
  executionMode?: string;
  entryPolicy?: string;
  lifecycleSource?: string;
  signalDirection?: string;
  marketRegime?: string;
  selectedStopLossPercent?: number;
  selectedTakeProfitPercent?: number;
  selectedStopLossSource?: string;
  selectedTakeProfitSource?: string;
  selectedStopLossEvidenceSource?: string;
  selectedTakeProfitEvidenceSource?: string;
  fallbackStopLossPercent?: number;
  fallbackTakeProfitPercent?: number;
  currentEffectiveStopLossPercent?: number;
  currentEffectiveTakeProfitPercent?: number;
  currentThresholdCompatibility: boolean;
  maximumHoldingDurationMs?: number;
  maximumHoldingSource?: string;
  trailingActivationPercent?: number;
  trailingDistancePercent?: number;
  trailingSource?: string;
  breakevenActivationPercent?: number;
  breakevenSource?: string;
  profitProtectionActivationPercent?: number;
  profitProtectionGivebackPercent?: number;
  profitProtectionMinimumLockPercent?: number;
  profitProtectionSource?: string;
  currentPmBarrierId?: string;
  currentPmBarrierTriggerPrice?: number;
  protectionAdjustments: ExitPositionAdjustmentRow[];
  protectionAdjustmentsDropped?: number;
  stopLossActive?: boolean;
  takeProfitActive?: boolean;
  initialRiskEvidence?: InitialRiskEvidence;
  executablePricePathEvidence?: ExecutablePricePathEvidence;
  postExitMissedReturnPercent?: number;
  postExitBestReturnPercent?: number;
  postExitWorstReturnPercent?: number;
  postExitObservedAt?: number;
  postExitHorizonMs?: number;
};

export type ExitPositionAdjustmentRow = {
  id: string;
  reason?: string;
  previousTriggerPrice?: number;
  newTriggerPrice?: number;
  source?: string;
  timestampMs?: number;
  recordSource: 'Position Management' | 'AutoTrader recovered provenance';
};

export type ExitClosedEvidenceRow = {
  id: string;
  symbol: string;
  strategy: string;
  decisionId: string;
  selectedStopLossPercent?: number;
  selectedTakeProfitPercent?: number;
  entryProfileId?: string;
  entryProfileVersion?: number;
  exitProfileId?: string;
  exitProfileVersion?: number;
  initialRiskEvidence?: InitialRiskEvidence;
  executablePricePathEvidence?: ExecutablePricePathEvidence;
  postExitMissedReturnPercent?: number;
  postExitBestReturnPercent?: number;
  postExitWorstReturnPercent?: number;
  postExitObservedAt?: number;
  postExitHorizonMs?: number;
};

export type ExitAssessmentMetricRow = {
  key: string;
  value?: number;
  coverageStatus: ExitPolicyMetricCoverageStatus | 'missing';
  sampleCount?: number;
  sourceFields: string[];
  reason?: string;
};

export type ExitPolicyAssessmentRow = {
  id: string;
  symbol: string;
  strategy: string;
  profileId?: string;
  profileVersion?: number;
  stopLossPercent?: number;
  takeProfitPercent?: number;
  activationPercent?: number;
  givebackPercent?: number;
  minimumLockPercent?: number;
  trailingActivationPercent?: number;
  trailingDistancePercent?: number;
  trailingCoverageStatus?: 'complete' | 'partial' | 'unavailable';
  breakevenActivationPercent?: number;
  breakevenCoverageStatus?: 'available' | 'unavailable';
  maximumHoldingDuration?: {
    value?: number;
    coverageStatus: ExitPolicyMetricCoverageStatus | 'missing';
    sampleCount?: number;
    sourceFields: string[];
    reason?: string;
  };
  sampleCount?: number;
  trainingCount?: number;
  validationCount?: number;
  asOfMs?: number;
  promotionEligible?: boolean;
  promotionBlockers: string[];
  blockersAvailable: boolean;
  metrics: ExitAssessmentMetricRow[];
};

export type ExitLearningContext = {
  context: string;
  learningSamples: number;
  positiveRate?: number;
  scoutLearningSamples: number;
  scoutPositiveRate?: number;
  explorationAllowed: boolean;
  exitPolicyStatus: 'unavailable';
};

export type AutoTraderExitPolicyState = {
  active: boolean;
  settingsLoading: boolean;
  outcomesLoading: boolean;
  decisionParametersAvailable: boolean;
  decisionParametersRefreshedAt?: number;
  profileRows: ExitProfileRow[];
  positionRows: ExitPositionRow[];
  closedEvidenceRows: ExitClosedEvidenceRow[];
  contexts: ExitLearningContext[];
  view: 'profiles' | 'positions' | 'contexts' | 'drilldown' | 'assessments';
  strategyFilter: string;
  symbolFilter: string;
  strategyOptions: string[];
  symbolOptions: string[];
  filteredProfiles: ExitProfileRow[];
  filteredPositions: ExitPositionRow[];
  filteredClosedEvidence: ExitClosedEvidenceRow[];
  assessmentStatus: 'loading' | 'available' | 'unavailable';
  assessmentError?: string;
  assessments: ExitPolicyAssessmentRow[];
  publishedPoliciesAvailable: boolean;
  publishedPolicies: LearnedExitProtectionPolicy[];
  activeAutoTraderPolicyStatus: 'unavailable';
};

export type AutoTraderExitPolicySources = {
  settings: LifecycleSource<AutoTraderState> & {
    refreshDecisionParameters?: () => void | Promise<unknown>;
  };
  outcomes: LifecycleSource<OutcomesState>;
};

const EXIT_PARAMETER_WORDS = [
  'exit',
  'stop_loss',
  'stop-loss',
  'stop loss',
  'take_profit',
  'take-profit',
  'take profit',
  'holding_period',
  'holding period',
  'max_hold',
  'max hold',
  'hold_duration',
  'hold duration',
  'time_stop',
  'time stop',
  'trailing',
  'profit_protection',
  'profit protection',
  'breakeven',
  'reward_risk',
  'reward/risk',
  'risk_reward',
  'risk/reward',
  'r_multiple',
];

function exitParameterKind(
  parameter: AutoTraderStrategyParameter,
): ExitParameterKind | undefined {
  const identity = `${parameter.key} ${parameter.label}`.toLowerCase();
  if (/stop[_ -]?loss/.test(identity)) return 'stop';
  if (/take[_ -]?profit|profit[_ -]?target|target[_ -]?percent/.test(identity))
    return 'target';
  if (
    /max[_ -]?hold|holding[_ -]?(period|duration)|hold[_ -]?duration|time[_ -]?stop/.test(
      identity,
    )
  )
    return 'maximumHolding';
  if (/trailing/.test(identity)) return 'trailing';
  if (/profit[_ -]?protection|breakeven/.test(identity))
    return 'profitProtection';
  if (/reward[_ /]?risk|risk[_ /]?reward|r[_ -]?multiple/.test(identity))
    return 'plannedRewardRisk';
  if (EXIT_PARAMETER_WORDS.some((word) => identity.includes(word)))
    return 'otherExit';
  return undefined;
}

function finite(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value)
    ? value
    : undefined;
}

function optionalField(source: unknown, key: string): number | undefined {
  if (!source || typeof source !== 'object') return undefined;
  return finite((source as Record<string, unknown>)[key]);
}

function projectProfile(profile: AutoTraderStrategyProfile): ExitProfileRow {
  const evidence = profile.walk_forward_evidence;
  return {
    id: `${profile.symbol}:${profile.strategy}`,
    symbol: profile.symbol,
    strategy: profile.strategy,
    profileId:
      typeof profile.profile_id === 'string' && profile.profile_id.trim()
        ? profile.profile_id
        : undefined,
    profileVersion: finite(profile.profile_version),
    learningSamples: profile.learning_samples,
    positiveRate: finite(profile.positive_rate),
    parameters: profile.parameters.flatMap((parameter) => {
      const kind = exitParameterKind(parameter);
      return kind
        ? [
            {
              key: parameter.key,
              label: parameter.label,
              unit: parameter.unit,
              kind,
              rawServiceValue: finite(parameter.value),
              configuredValue: finite(parameter.configured_value),
              defaultValue: finite(parameter.default_value),
              scoutVariantsAvailable: Array.isArray(parameter.scout_variants),
              scoutVariants: Array.isArray(parameter.scout_variants)
                ? parameter.scout_variants.map((variant) => ({
                    candidateValue: variant.candidate_value,
                    sampleCount: variant.sample_count,
                    promoted: variant.promoted,
                    rejected: variant.rejected,
                  }))
                : [],
            },
          ]
        : [];
    }),
    exchange: undefined,
    supportedDirection: undefined,
    effectiveSampleSize: finite(evidence?.effectiveSampleSize),
    oosFoldCount: finite(evidence?.oosFoldCount),
    spaPValue: finite(evidence?.spaPValue),
    lastEvidenceSampleTimestampMs: finite(evidence?.lastSampleTimestampMs),
    regimeEvidenceKeys: Object.keys(
      profile.walk_forward_evidence_by_regime ?? {},
    ).sort(),
    finalHoldoutStatus: evidence?.finalHoldoutStatus ?? 'unavailable',
    historyCoverage: evidence?.historyCoverage ?? 'unavailable',
    costCoverage: evidence?.costCoverage ?? 'unavailable',
  };
}

function projectPosition(row: AutoTraderOutcome): ExitPositionRow {
  const provenance = row.exitParameterProvenance;
  const barrier = row.currentPmBarrier;
  const exactBarrierIdentity =
    typeof row.exitProfileId === 'string' &&
    row.exitProfileId.length > 0 &&
    typeof row.exitProfileVersion === 'number' &&
    Number.isSafeInteger(row.exitProfileVersion) &&
    barrier?.profile_id === row.exitProfileId &&
    barrier.profile_version === row.exitProfileVersion;
  const pmAdjustments = exactBarrierIdentity
    ? (barrier?.trigger_adjustments ?? []).filter(
        (adjustment) =>
          (adjustment.profile_id === undefined ||
            adjustment.profile_id === row.exitProfileId) &&
          (adjustment.profile_version === undefined ||
            adjustment.profile_version === row.exitProfileVersion),
      )
    : [];
  const pmRequestIds = new Set(
    pmAdjustments
      .map((adjustment) => adjustment.request_id)
      .filter((id): id is string => typeof id === 'string' && id.length > 0),
  );
  const adjustments: ExitPositionAdjustmentRow[] = [
    ...pmAdjustments.map((adjustment, index) => ({
      id:
        adjustment.request_id ??
        `pm:${index}:${adjustment.adjusted_at_ms ?? 0}`,
      reason: adjustment.reason ?? undefined,
      previousTriggerPrice: finite(adjustment.expected_trigger_price),
      newTriggerPrice: finite(adjustment.new_trigger_price),
      source: adjustment.policy_source ?? undefined,
      timestampMs: finite(adjustment.adjusted_at_ms),
      recordSource: 'Position Management' as const,
    })),
    ...(provenance?.protection_adjustments ?? [])
      .filter(
        (adjustment) =>
          !adjustment.request_id || !pmRequestIds.has(adjustment.request_id),
      )
      .map((adjustment, index) => ({
        id:
          adjustment.request_id ??
          `autotrader:${index}:${adjustment.updated_at_ms ?? 0}`,
        reason: adjustment.reason ?? undefined,
        previousTriggerPrice: finite(adjustment.previous_trigger_price),
        newTriggerPrice: finite(adjustment.trigger_price),
        source: adjustment.policy_source ?? undefined,
        timestampMs: finite(adjustment.updated_at_ms),
        recordSource: 'AutoTrader recovered provenance' as const,
      })),
  ].sort((left, right) => (left.timestampMs ?? 0) - (right.timestampMs ?? 0));
  return {
    id: row.rowId,
    status: 'Open',
    symbol: row.symbol,
    strategy: row.strategy,
    decisionId: row.decisionId,
    openedAt: row.openedAt,
    entryProfileId: row.entryProfileId,
    entryProfileHash: row.entryProfileHash,
    entryProfileVersion: row.entryProfileVersion,
    exitProfileId: row.exitProfileId,
    exitProfileVersion: row.exitProfileVersion,
    exitPolicyAsOfMs: row.exitPolicyAsOfMs,
    executionMode: row.executionMode,
    entryPolicy: row.entryPolicy,
    lifecycleSource: row.lifecycleSource,
    signalDirection: row.signalDirection,
    marketRegime: row.marketRegime,
    selectedStopLossPercent: finite(row.selectedStopLossPercent),
    selectedTakeProfitPercent: finite(row.selectedTakeProfitPercent),
    selectedStopLossSource: row.selectedStopLossSource,
    selectedTakeProfitSource: row.selectedTakeProfitSource,
    selectedStopLossEvidenceSource: row.selectedStopLossEvidenceSource,
    selectedTakeProfitEvidenceSource: row.selectedTakeProfitEvidenceSource,
    fallbackStopLossPercent: finite(row.fallbackStopLossPercent),
    fallbackTakeProfitPercent: finite(row.fallbackTakeProfitPercent),
    currentEffectiveStopLossPercent: finite(
      row.currentEffectiveStopLossPercent,
    ),
    currentEffectiveTakeProfitPercent: finite(
      row.currentEffectiveTakeProfitPercent,
    ),
    currentThresholdCompatibility: row.currentThresholdCompatibility ?? true,
    maximumHoldingDurationMs: finite(
      provenance?.maximum_holding_duration?.selected_ms,
    ),
    maximumHoldingSource:
      provenance?.maximum_holding_duration?.source ?? undefined,
    trailingActivationPercent: finite(
      provenance?.trailing_protection?.activation_percent,
    ),
    trailingDistancePercent: finite(
      provenance?.trailing_protection?.distance_percent,
    ),
    trailingSource: provenance?.trailing_protection?.source ?? undefined,
    breakevenActivationPercent: finite(
      provenance?.breakeven_protection?.activation_percent,
    ),
    breakevenSource: provenance?.breakeven_protection?.source ?? undefined,
    profitProtectionActivationPercent: finite(
      provenance?.profit_protection?.activation_percent,
    ),
    profitProtectionGivebackPercent: finite(
      provenance?.profit_protection?.giveback_percent,
    ),
    profitProtectionMinimumLockPercent: finite(
      provenance?.profit_protection?.minimum_lock_percent,
    ),
    profitProtectionSource: provenance?.profit_protection?.source ?? undefined,
    currentPmBarrierId: exactBarrierIdentity
      ? (barrier?.id ?? undefined)
      : undefined,
    currentPmBarrierTriggerPrice: exactBarrierIdentity
      ? finite(barrier?.trigger_price)
      : undefined,
    protectionAdjustments: adjustments,
    protectionAdjustmentsDropped: finite(
      provenance?.protection_adjustments_dropped,
    ),
    stopLossActive: row.stopLossActive,
    takeProfitActive: row.takeProfitActive,
    initialRiskEvidence: row.initialRiskEvidence,
    executablePricePathEvidence: row.executablePricePathEvidence,
    postExitMissedReturnPercent: finite(row.postExitMissedReturnPercent),
    postExitBestReturnPercent: finite(row.postExitBestReturnPercent),
    postExitWorstReturnPercent: finite(row.postExitWorstReturnPercent),
    postExitObservedAt: finite(row.postExitObservedAt),
    postExitHorizonMs: finite(row.postExitHorizonMs),
  };
}

function projectClosedEvidence(row: AutoTraderOutcome): ExitClosedEvidenceRow {
  return {
    id: row.rowId,
    symbol: row.symbol,
    strategy: row.strategy,
    decisionId: row.decisionId,
    selectedStopLossPercent: finite(row.selectedStopLossPercent),
    selectedTakeProfitPercent: finite(row.selectedTakeProfitPercent),
    entryProfileId: row.entryProfileId,
    entryProfileVersion: row.entryProfileVersion,
    exitProfileId: row.exitProfileId,
    exitProfileVersion: row.exitProfileVersion,
    initialRiskEvidence: row.initialRiskEvidence,
    executablePricePathEvidence: row.executablePricePathEvidence,
    postExitMissedReturnPercent: finite(row.postExitMissedReturnPercent),
    postExitBestReturnPercent: finite(row.postExitBestReturnPercent),
    postExitWorstReturnPercent: finite(row.postExitWorstReturnPercent),
    postExitObservedAt: finite(row.postExitObservedAt),
    postExitHorizonMs: finite(row.postExitHorizonMs),
  };
}

function projectAssessment(
  assessment: LearnedExitPolicyAssessment,
): ExitPolicyAssessmentRow {
  const report = assessment.metrics as ExitPolicyMetricReport | undefined;
  const values =
    report?.values && typeof report.values === 'object' ? report.values : {};
  const coverage =
    report?.coverage && typeof report.coverage === 'object'
      ? report.coverage
      : {};
  const keys = [
    ...new Set([...Object.keys(values), ...Object.keys(coverage)]),
  ].sort();
  const metrics = keys.map((key): ExitAssessmentMetricRow => {
    const rawCoverage = coverage[key];
    const status = rawCoverage?.status;
    const coverageStatus =
      status === 'complete' || status === 'partial' || status === 'unavailable'
        ? status
        : 'missing';
    const sourceFields = Array.isArray(rawCoverage?.source_fields)
      ? rawCoverage.source_fields.filter(
          (field): field is string => typeof field === 'string',
        )
      : [];
    let value = finite(values[key]);
    let reason =
      typeof rawCoverage?.reason === 'string'
        ? rawCoverage.reason
        : rawCoverage
          ? undefined
          : 'Metric has no coverage record in the assessment report.';
    if (key === 'expectancy_r' && value !== undefined) {
      const requiredRiskSources = [
        'entry_context.initial_risk_reference.schema_version',
        'entry_context.initial_risk_reference.status',
        'entry_context.initial_risk_reference.quote_amount',
        'entry_context.initial_risk_reference.quote_currency',
        'entry_context.initial_risk_reference.fill_ids',
        'replayed_net_return_percent',
      ];
      const riskCoverageComplete =
        coverageStatus === 'complete' &&
        requiredRiskSources.every((source) => sourceFields.includes(source));
      if (!riskCoverageComplete) {
        value = undefined;
        reason ??=
          'R value withheld because complete authoritative initial-risk and replay-return sources are not covered.';
      }
    }
    return {
      key,
      value,
      coverageStatus,
      sampleCount: finite(rawCoverage?.sample_count),
      sourceFields,
      reason,
    };
  });
  const blockersAvailable = Array.isArray(report?.promotion_blockers);
  const holdingMetric = metrics.find(
    (metric) => metric.key === 'maximum_holding_duration_ms',
  );
  const holdingCandidate = finite(assessment.maximum_holding_duration_ms);
  const maximumHoldingDuration = {
    value:
      holdingCandidate !== undefined &&
      holdingMetric &&
      (holdingMetric.coverageStatus === 'complete' ||
        holdingMetric.coverageStatus === 'partial') &&
      holdingMetric.sourceFields.length > 0
        ? holdingCandidate
        : undefined,
    coverageStatus: holdingMetric?.coverageStatus ?? ('missing' as const),
    sampleCount: holdingMetric?.sampleCount,
    sourceFields: holdingMetric?.sourceFields ?? [],
    reason:
      holdingMetric?.reason ??
      'No maximum_holding_duration_ms coverage record was returned.',
  };
  const hasExactProfileIdentity =
    typeof assessment.profile_id === 'string' &&
    assessment.profile_id.trim().length > 0 &&
    finite(assessment.profile_version) !== undefined;
  const trailingActivationCandidate = hasExactProfileIdentity
    ? optionalField(assessment, 'trailing_activation_percent')
    : undefined;
  const trailingDistanceCandidate = hasExactProfileIdentity
    ? optionalField(assessment, 'trailing_distance_percent')
    : undefined;
  const breakevenActivationCandidate = hasExactProfileIdentity
    ? optionalField(assessment, 'breakeven_activation_percent')
    : undefined;
  const trailingCoverageStatus =
    trailingActivationCandidate !== undefined &&
    trailingDistanceCandidate !== undefined
      ? 'complete'
      : trailingActivationCandidate !== undefined ||
          trailingDistanceCandidate !== undefined
        ? 'partial'
        : 'unavailable';
  return {
    id: `${assessment.symbol}:${assessment.strategy}:${assessment.as_of_ms}`,
    symbol: assessment.symbol,
    strategy: assessment.strategy,
    profileId:
      typeof assessment.profile_id === 'string' && assessment.profile_id.trim()
        ? assessment.profile_id
        : undefined,
    profileVersion: finite(assessment.profile_version),
    stopLossPercent: finite(assessment.stop_loss_percent),
    takeProfitPercent: finite(assessment.take_profit_percent),
    activationPercent: finite(assessment.activation_percent),
    givebackPercent: finite(assessment.giveback_percent),
    minimumLockPercent: finite(assessment.minimum_lock_percent),
    trailingActivationPercent: trailingActivationCandidate,
    trailingDistancePercent: trailingDistanceCandidate,
    trailingCoverageStatus,
    breakevenActivationPercent: breakevenActivationCandidate,
    breakevenCoverageStatus:
      breakevenActivationCandidate === undefined ? 'unavailable' : 'available',
    maximumHoldingDuration,
    sampleCount: finite(assessment.sample_count),
    trainingCount: finite(assessment.training_count),
    validationCount: finite(assessment.validation_count),
    asOfMs: finite(assessment.as_of_ms),
    promotionEligible:
      typeof report?.promotion_eligible === 'boolean'
        ? report.promotion_eligible
        : undefined,
    promotionBlockers: blockersAvailable
      ? report!.promotion_blockers.filter(
          (blocker): blocker is string => typeof blocker === 'string',
        )
      : [],
    blockersAvailable,
    metrics,
  };
}

function selected<T extends { strategy: string; symbol: string }>(
  rows: T[],
  strategyFilter: string,
  symbolFilter: string,
): T[] {
  return rows.filter(
    (row) =>
      (strategyFilter === 'all' || row.strategy === strategyFilter) &&
      (symbolFilter === 'all' || row.symbol === symbolFilter),
  );
}

export function projectExitPolicyState(
  settings: AutoTraderState,
  outcomes: OutcomesState,
  filters: { strategy: string; symbol: string },
  active: boolean,
  view: AutoTraderExitPolicyState['view'] = 'profiles',
): AutoTraderExitPolicyState {
  const parameters = settings.decisionParameters;
  const profiles = (parameters?.strategy_profiles ?? []).map(projectProfile);
  const positions = outcomes.rows
    .filter((row) => row.status === 'Open')
    .map(projectPosition);
  const closedEvidenceRows = outcomes.closedLedgerRows
    .filter(
      (row) =>
        row.status === 'Closed' && row.rowId.startsWith('ledger-summary:'),
    )
    .map(projectClosedEvidence);
  const contexts = (parameters?.contexts ?? []).map((context) => ({
    context: context.context,
    learningSamples: context.learning_samples,
    positiveRate: finite(context.positive_rate),
    scoutLearningSamples: context.scout_learning_samples,
    scoutPositiveRate: finite(context.scout_positive_rate),
    explorationAllowed: context.exploration_allowed,
    exitPolicyStatus: 'unavailable' as const,
  }));
  const strategyOptions = [
    ...new Set([
      ...profiles.map((row) => row.strategy),
      ...positions.map((row) => row.strategy),
      ...closedEvidenceRows.map((row) => row.strategy),
    ]),
  ].sort();
  const symbolOptions = [
    ...new Set([
      ...profiles.map((row) => row.symbol),
      ...positions.map((row) => row.symbol),
      ...closedEvidenceRows.map((row) => row.symbol),
    ]),
  ].sort();
  const assessmentContractAvailable = Array.isArray(
    parameters?.exit_policy_assessments,
  );
  const assessmentRows = assessmentContractAvailable
    ? parameters!.exit_policy_assessments!.map(projectAssessment)
    : [];
  const publishedPoliciesAvailable = Array.isArray(
    parameters?.exit_protection_policies,
  );
  const publishedPolicies = publishedPoliciesAvailable
    ? parameters!.exit_protection_policies!
    : [];

  return {
    active,
    settingsLoading: settings.loading,
    outcomesLoading: outcomes.loading,
    decisionParametersAvailable: parameters !== undefined,
    decisionParametersRefreshedAt: finite(parameters?.refreshed_at),
    profileRows: profiles,
    positionRows: positions,
    closedEvidenceRows,
    contexts,
    view,
    strategyFilter: filters.strategy,
    symbolFilter: filters.symbol,
    strategyOptions,
    symbolOptions,
    filteredProfiles: selected(profiles, filters.strategy, filters.symbol),
    filteredPositions: selected(positions, filters.strategy, filters.symbol),
    filteredClosedEvidence: selected(
      closedEvidenceRows,
      filters.strategy,
      filters.symbol,
    ),
    assessmentStatus: !parameters
      ? settings.loading
        ? 'loading'
        : 'unavailable'
      : assessmentContractAvailable
        ? 'available'
        : 'unavailable',
    assessmentError:
      settings.decisionParametersError ??
      (!parameters
        ? 'Decision-parameter contract unavailable.'
        : !assessmentContractAvailable
          ? 'AutoTrader decision-parameter response omits exit_policy_assessments.'
          : undefined),
    assessments: selected(assessmentRows, filters.strategy, filters.symbol),
    publishedPoliciesAvailable,
    publishedPolicies: selected(
      publishedPolicies,
      filters.strategy,
      filters.symbol,
    ),
    activeAutoTraderPolicyStatus: 'unavailable',
  };
}

export class AutoTraderExitPolicyModel {
  readonly state$: BehaviorSubject<AutoTraderExitPolicyState>;
  private readonly sources: AutoTraderExitPolicySources;
  private subscription?: Subscription;
  private started = false;
  private filters = { strategy: 'all', symbol: 'all' };
  private view: AutoTraderExitPolicyState['view'] = 'profiles';

  constructor(
    settings: AutoTraderExitPolicySources['settings'],
    outcomes: AutoTraderExitPolicySources['outcomes'],
  ) {
    this.sources = { settings, outcomes };
    this.state$ = new BehaviorSubject(
      projectExitPolicyState(
        settings.state$.value,
        outcomes.state$.value,
        this.filters,
        false,
        this.view,
      ),
    );
  }

  start() {
    if (this.started) return;
    this.started = true;
    this.sources.settings.start();
    this.sources.outcomes.start();
    void this.sources.settings.refreshDecisionParameters?.();
    this.subscription = new Subscription();
    this.subscription.add(
      this.sources.settings.state$.subscribe(() => this.publish()),
    );
    this.subscription.add(
      this.sources.outcomes.state$.subscribe(() => this.publish()),
    );
    this.publish();
  }

  stop() {
    if (!this.started) return;
    this.started = false;
    this.subscription?.unsubscribe();
    this.subscription = undefined;
    this.sources.settings.stop();
    this.sources.outcomes.stop();
    this.publish();
  }

  setStrategyFilter(strategy: string) {
    this.filters = { ...this.filters, strategy };
    this.publish();
  }

  setSymbolFilter(symbol: string) {
    this.filters = { ...this.filters, symbol };
    this.publish();
  }

  setView(view: AutoTraderExitPolicyState['view']) {
    this.view = view;
    this.publish();
  }

  private publish() {
    this.state$.next(
      projectExitPolicyState(
        this.sources.settings.state$.value,
        this.sources.outcomes.state$.value,
        this.filters,
        this.started,
        this.view,
      ),
    );
  }
}
