import { BehaviorSubject, type Subscription } from 'rxjs';
import {
  projectExecutableThresholdCrossings,
  type AutoTraderOutcome,
  type AutoTraderOutcomesModel,
  type ExecutableThresholdCrossing,
} from './AutoTraderOutcomesModel';
import { autoTraderRpc } from '../auto_trader_rsocket';

export type ExcursionView =
  | 'overview'
  | 'continuation'
  | 'transitions'
  | 'sensitivity'
  | 'replay'
  | 'what-if';
export type ExcursionSource = 'all' | 'autotrader' | 'scouts';
export type ExcursionRange = '30d' | '90d' | 'all';
export const EXCURSION_PATH_PAGE_SIZE = 250;
export const WHAT_IF_CANDIDATE_POLICY_ID = 'what-if-draft';
export const WHAT_IF_CANDIDATE_POLICY_VERSION = 'draft-v1';

export type WhatIfParameterName =
  | 'stop_loss_percent'
  | 'take_profit_percent'
  | 'activation_percent'
  | 'giveback_percent'
  | 'minimum_lock_percent'
  | 'trailing_activation_percent'
  | 'trailing_distance_percent'
  | 'breakeven_activation_percent';

export type WhatIfParameters = Record<WhatIfParameterName, number | null>;

export type WhatIfDraft = {
  tradeId?: string;
  candidatePolicyId: string;
  candidatePolicyVersion: string;
  parameters: WhatIfParameters;
  maximumHoldDays: number | null;
  maximumHoldSpec?: {
    minimum: number;
    maximum: number;
    default: number;
    step: number;
    unit: 'days';
  };
  maximumHoldReason: string;
};

export type WhatIfReplayResult = {
  tradeId: string;
  scope: string;
  learningEligible: false;
  baselineNetReturnPercent: number;
  candidateNetReturnPercent: number;
  deltaNetReturnPercent: number;
  durablePathSampleCount: number;
  baselineMaximumHoldingDurationDays?: number;
  baselineMaximumHoldingDurationMs?: number;
  candidateMaximumHoldingDurationDays?: number;
  candidateMaximumHoldingDurationMs?: number;
  durationUnit: 'days';
  candidateExit: {
    timestampMs: number;
    price: number;
    reason: string;
    quoteSourceSystem?: string;
    quoteSourceEpoch?: string;
    quoteSourceCursor?: number;
    quoteSourceRecordId?: string;
  };
  feeScheduleVersions: unknown[];
};

export type WhatIfReplayStatus =
  | 'idle'
  | 'loading'
  | 'complete'
  | 'unavailable'
  | 'not_computed'
  | 'error'
  | 'cancelled';

export type WhatIfReplayState = {
  status: WhatIfReplayStatus;
  tradeId?: string;
  error?: string;
  reason?: string;
  result?: WhatIfReplayResult;
  processedSampleCount?: number;
  totalSampleCount?: number;
  afterTimestampMs?: number;
  afterCursor?: number;
};

export type TradePricePathSample = {
  cursor: number;
  pathEpoch: string;
  sequence: number;
  timestampMs: number;
  bid?: number;
  ask?: number;
  sourceUpdatedAtMs?: number;
  sourceFresh?: boolean;
  quoteSourceSystem?: string;
  quoteSourceEpoch?: string;
  quoteSourceCursor?: number;
  quoteSourceRecordId?: string;
};

export type TradePricePathSegment = {
  pathEpoch: string;
  firstTimestampMs: number;
  lastTimestampMs: number;
  sampleCount: number;
  maximumGapMs: number;
  complete: boolean;
  missingReasons: string[];
};

export type TradePathCapturePolicy = {
  fineIntervalMs: number;
  fineWindowMs: number;
  coarseIntervalMs: number;
  coarseAfterMs: number;
  eventDrivenExtrema: boolean;
  eventDrivenThresholdCrossings: boolean;
};

export type LifetimeExcursionEvidence = {
  grossMfePercent?: number;
  grossMaePercent?: number;
  eventSampleCount: number;
  eventMaximumGapMs?: number;
  complete: boolean;
  thresholdCrossings: ExecutableThresholdCrossing[];
  missingReasons: string[];
};

export type TradePathReplayEligibility = {
  status:
    | 'eligible'
    | 'ineligible_coarse_sampling'
    | 'incomplete'
    | 'unavailable';
  requiredMaximumGapMs: number;
  maximumObservedGapMs?: number;
  reason?: string;
};

export type TradePathHorizonCoverage = {
  horizon: 'fine_0_30m' | 'coarse_after_30m';
  offsetStartMs?: number;
  offsetEndMs?: number;
  targetIntervalMs: number;
  sampleCount: number;
  maximumObservedGapMs?: number;
  complete: boolean;
  horizonReached: boolean;
  missingReasons: string[];
};

export type TradePricePathPage = {
  tradeId: string;
  coverageStatus: string;
  historyComplete: boolean;
  totalSampleCount: number;
  samples: TradePricePathSample[];
  segments: TradePricePathSegment[];
  missingReasons: string[];
  nextTimestampMs?: number;
  nextCursor?: number;
  hasMore: boolean;
  capturePolicy?: TradePathCapturePolicy;
  lifetimeExcursionEvidence?: LifetimeExcursionEvidence;
  replayEligibility?: TradePathReplayEligibility;
  horizonCoverage?: TradePathHorizonCoverage[];
};

export type ExcursionPathFetcher = (path: string) => Promise<Response>;
export type ExcursionWhatIfFetcher = (
  path: string,
  body: unknown,
) => Promise<Response>;

function capturedPercent(value: unknown): number | null {
  return finite(value) ?? null;
}

function holdDaysOnGrid(
  value: number,
  spec: NonNullable<WhatIfDraft['maximumHoldSpec']>,
): boolean {
  if (
    !Number.isFinite(value) ||
    value < spec.minimum ||
    value > spec.maximum ||
    spec.step <= 0
  )
    return false;
  const stepIndex = (value - spec.minimum) / spec.step;
  return Math.abs(stepIndex - Math.round(stepIndex)) <= 1e-7;
}

function maximumHoldInputs(row?: ExcursionTrade): {
  maximumHoldDays: number | null;
  maximumHoldSpec?: WhatIfDraft['maximumHoldSpec'];
  maximumHoldReason: string;
} {
  const provenance = row?.trade.exitParameterProvenance as
    | (Record<string, unknown> & {
        maximum_holding_duration?: unknown;
      })
    | undefined;
  const specification = record(provenance?.strategy_parameter_specification);
  const rawSpec = record(specification?.maximum_hold_days);
  if (
    specification?.schema_version !== 1 ||
    rawSpec?.key !== 'maximum_hold_days'
  )
    return {
      maximumHoldDays: null,
      maximumHoldReason:
        'Disabled: captured schema-1 maximum_hold_days bounds are unavailable.',
    };
  const minimum = finite(rawSpec.min);
  const maximum = finite(rawSpec.max);
  const defaultValue = finite(rawSpec.default);
  const step = finite(rawSpec.step);
  if (
    minimum === undefined ||
    maximum === undefined ||
    defaultValue === undefined ||
    step === undefined ||
    maximum < minimum ||
    step <= 0 ||
    rawSpec.unit !== 'days'
  )
    return {
      maximumHoldDays: null,
      maximumHoldReason:
        'Disabled: captured maximum-hold bounds have an unsupported unit or invalid range.',
    };
  const maximumHoldSpec = {
    minimum,
    maximum,
    default: defaultValue,
    step,
    unit: 'days' as const,
  };
  if (!holdDaysOnGrid(defaultValue, maximumHoldSpec))
    return {
      maximumHoldDays: null,
      maximumHoldReason:
        'Disabled: captured maximum-hold default is outside the reported bounds or step grid.',
    };
  const selected = record(provenance?.maximum_holding_duration);
  const capturedDays = finite(selected?.selected_days);
  if (capturedDays === undefined)
    return {
      maximumHoldDays: null,
      maximumHoldSpec,
      maximumHoldReason:
        'No captured baseline duration was reported; choose a candidate on the captured grid or leave blank to preserve baseline.',
    };
  if (!holdDaysOnGrid(capturedDays, maximumHoldSpec))
    return {
      maximumHoldDays: null,
      maximumHoldSpec: undefined,
      maximumHoldReason:
        'Captured baseline duration is outside the reported bounds or step grid; candidate duration left blank.',
    };
  return {
    maximumHoldDays: capturedDays,
    maximumHoldSpec,
    maximumHoldReason:
      'Seeded from captured trade-time policy; editable within captured bounds.',
  };
}

function whatIfDraftForTrade(row?: ExcursionTrade): WhatIfDraft {
  const provenance = row?.trade.exitParameterProvenance;
  const holdInputs = maximumHoldInputs(row);
  return {
    tradeId: row?.stableTradeId,
    candidatePolicyId: WHAT_IF_CANDIDATE_POLICY_ID,
    candidatePolicyVersion: WHAT_IF_CANDIDATE_POLICY_VERSION,
    parameters: {
      stop_loss_percent: capturedPercent(
        provenance?.stop_loss?.selected_percent,
      ),
      take_profit_percent: capturedPercent(
        provenance?.take_profit?.selected_percent,
      ),
      activation_percent: capturedPercent(
        provenance?.profit_protection?.activation_percent,
      ),
      giveback_percent: capturedPercent(
        provenance?.profit_protection?.giveback_percent,
      ),
      minimum_lock_percent: capturedPercent(
        provenance?.profit_protection?.minimum_lock_percent,
      ),
      trailing_activation_percent: capturedPercent(
        provenance?.trailing_protection?.activation_percent,
      ),
      trailing_distance_percent: capturedPercent(
        provenance?.trailing_protection?.distance_percent,
      ),
      breakeven_activation_percent: capturedPercent(
        provenance?.breakeven_protection?.activation_percent,
      ),
    },
    ...holdInputs,
  };
}

const requiredWhatIfParameters: WhatIfParameterName[] = [
  'stop_loss_percent',
  'take_profit_percent',
  'activation_percent',
  'giveback_percent',
  'minimum_lock_percent',
];

function readyWhatIfRequest(
  draft: WhatIfDraft,
  selected?: ExcursionTrade,
  pathPage?: TradePricePathPage,
):
  | { ready: true; body: Record<string, unknown> }
  | { ready: false; reason: string } {
  if (!selected?.stableTradeId)
    return { ready: false, reason: 'A stable canonical trade ID is required.' };
  const capturedProfile = selected.trade.exitParameterProvenance;
  if (
    !text(capturedProfile?.profile_id) ||
    finite(capturedProfile?.profile_version) === undefined
  )
    return {
      ready: false,
      reason: 'Captured policy lineage is unavailable for this trade.',
    };
  if (draft.tradeId !== selected.stableTradeId)
    return {
      ready: false,
      reason: 'Draft policy belongs to a different trade.',
    };
  if (pathPage?.tradeId !== selected.stableTradeId)
    return {
      ready: false,
      reason:
        'Load this trade’s durable path before checking replay eligibility.',
    };
  if (!pathPage.replayEligibility)
    return {
      ready: false,
      reason:
        'Fine-resolution replay eligibility is unavailable from the path source.',
    };
  if (pathPage.replayEligibility.status !== 'eligible')
    return {
      ready: false,
      reason:
        pathPage.replayEligibility.reason ??
        (pathPage.replayEligibility.status === 'ineligible_coarse_sampling'
          ? 'Coarse lifetime sampling is not eligible for finer-resolution policy replay.'
          : pathPage.replayEligibility.status === 'incomplete'
            ? 'Path coverage is incomplete for policy replay.'
            : 'Replay eligibility is unavailable from the path source.'),
    };
  if (
    pathPage.replayEligibility.maximumObservedGapMs === undefined ||
    pathPage.replayEligibility.maximumObservedGapMs >
      pathPage.replayEligibility.requiredMaximumGapMs
  )
    return {
      ready: false,
      reason:
        'Replay eligibility conflicts with the observed maximum gap; finer-resolution replay is blocked.',
    };
  const missing = requiredWhatIfParameters.filter((key) => {
    const value = draft.parameters[key];
    return typeof value !== 'number' || !Number.isFinite(value);
  });
  if (missing.length)
    return {
      ready: false,
      reason: `Captured lineage or draft values are missing: ${missing.join(', ')}.`,
    };
  if (
    draft.maximumHoldDays !== null &&
    (!draft.maximumHoldSpec ||
      !holdDaysOnGrid(draft.maximumHoldDays, draft.maximumHoldSpec))
  )
    return {
      ready: false,
      reason:
        'Candidate maximum hold must match the captured strategy bounds and step.',
    };
  return {
    ready: true,
    body: {
      trade_id: selected.stableTradeId,
      candidate_policy_id: draft.candidatePolicyId,
      candidate_policy_version: draft.candidatePolicyVersion,
      candidate_parameters: Object.fromEntries(
        Object.entries(draft.parameters).filter(
          ([, value]) => typeof value === 'number' && Number.isFinite(value),
        ),
      ),
      ...(draft.maximumHoldDays === null
        ? {}
        : {
            candidate_maximum_holding_duration_days: draft.maximumHoldDays,
          }),
    },
  };
}

export function parseWhatIfReplayResult(
  payload: unknown,
  expectedTradeId: string,
): WhatIfReplayState | null {
  const root = record(payload);
  if (
    !root ||
    root.schema_version !== 1 ||
    root.trade_id !== expectedTradeId ||
    typeof root.status !== 'string'
  )
    return null;
  if (root.status === 'unavailable')
    return {
      status: 'unavailable',
      tradeId: expectedTradeId,
      reason: text(root.reason) ?? 'Replay evidence is unavailable.',
    };
  if (root.status === 'not_computed')
    return {
      status: 'not_computed',
      tradeId: expectedTradeId,
      reason: text(root.reason) ?? 'Replay exceeded its bounded time budget.',
      processedSampleCount: finite(root.processed_sample_count),
      totalSampleCount: finite(root.total_sample_count),
      afterTimestampMs: finite(root.after_timestamp_ms),
      afterCursor: finite(root.after_cursor),
    };
  if (root.status !== 'complete' || root.learning_eligible !== false)
    return null;
  if (root.duration_unit !== 'days') return null;
  const candidateExit = record(root.candidate_exit);
  const baselinePolicy = record(root.baseline_policy);
  const candidatePolicy = record(root.candidate_policy);
  const baseline = finite(root.baseline_replay_net_return_percent);
  const candidate = finite(root.candidate_replay_net_return_percent);
  const delta = finite(root.candidate_minus_baseline_net_return_percent);
  const sampleCount = finite(root.durable_path_sample_count);
  const timestamp = finite(candidateExit?.timestamp_ms);
  const price = finite(candidateExit?.price);
  const reason = text(candidateExit?.reason);
  if (
    root.scope !== 'research_only_historical_what_if' ||
    baseline === undefined ||
    candidate === undefined ||
    delta === undefined ||
    sampleCount === undefined ||
    !candidateExit ||
    timestamp === undefined ||
    price === undefined ||
    reason === undefined ||
    !Array.isArray(root.fee_schedule_versions)
  )
    return null;
  return {
    status: 'complete',
    tradeId: expectedTradeId,
    result: {
      tradeId: expectedTradeId,
      scope: root.scope,
      learningEligible: false,
      baselineNetReturnPercent: baseline,
      candidateNetReturnPercent: candidate,
      deltaNetReturnPercent: delta,
      durablePathSampleCount: sampleCount,
      baselineMaximumHoldingDurationDays: finite(
        baselinePolicy?.maximum_holding_duration_days,
      ),
      baselineMaximumHoldingDurationMs: finite(
        baselinePolicy?.maximum_holding_duration_ms,
      ),
      candidateMaximumHoldingDurationDays: finite(
        candidatePolicy?.maximum_holding_duration_days,
      ),
      candidateMaximumHoldingDurationMs: finite(
        candidatePolicy?.maximum_holding_duration_ms,
      ),
      durationUnit: 'days',
      candidateExit: {
        timestampMs: timestamp,
        price,
        reason,
        quoteSourceSystem: text(candidateExit.quote_source_system),
        quoteSourceEpoch: text(candidateExit.quote_source_epoch),
        quoteSourceCursor: finite(candidateExit.quote_source_cursor),
        quoteSourceRecordId: text(candidateExit.quote_source_record_id),
      },
      feeScheduleVersions: root.fee_schedule_versions,
    },
  };
}

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function finite(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value)
    ? value
    : undefined;
}

function text(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value : undefined;
}

function textArray(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === 'string')
    : [];
}

export function parseTradePricePathPage(
  payload: unknown,
  expectedTradeId: string,
): TradePricePathPage | null {
  const root = record(payload);
  if (
    !root ||
    root.schema_version !== 1 ||
    root.trade_id !== expectedTradeId ||
    !Array.isArray(root.samples) ||
    !Array.isArray(root.segments)
  )
    return null;
  const samples = root.samples.flatMap((item): TradePricePathSample[] => {
    const row = record(item);
    const quote = record(row?.sample);
    const cursor = finite(row?.cursor);
    const sequence = finite(row?.sequence);
    const timestampMs = finite(row?.timestamp_ms);
    const pathEpoch = text(row?.path_epoch);
    if (
      !quote ||
      cursor === undefined ||
      sequence === undefined ||
      timestampMs === undefined ||
      pathEpoch === undefined
    )
      return [];
    return [
      {
        cursor,
        sequence,
        timestampMs,
        pathEpoch,
        bid: finite(quote.bid),
        ask: finite(quote.ask),
        sourceUpdatedAtMs: finite(quote.source_updated_at_ms),
        sourceFresh:
          typeof quote.source_fresh === 'boolean'
            ? quote.source_fresh
            : undefined,
        quoteSourceSystem: text(quote.quote_source_system),
        quoteSourceEpoch: text(quote.quote_source_epoch),
        quoteSourceCursor: finite(quote.quote_source_cursor),
        quoteSourceRecordId: text(quote.quote_source_record_id),
      },
    ];
  });
  const segments = root.segments.flatMap((item): TradePricePathSegment[] => {
    const segment = record(item);
    const pathEpoch = text(segment?.path_epoch);
    const firstTimestampMs = finite(segment?.first_timestamp_ms);
    const lastTimestampMs = finite(segment?.last_timestamp_ms);
    const sampleCount = finite(segment?.sample_count);
    const maximumGapMs = finite(segment?.maximum_gap_ms);
    if (
      pathEpoch === undefined ||
      firstTimestampMs === undefined ||
      lastTimestampMs === undefined ||
      sampleCount === undefined ||
      maximumGapMs === undefined
    )
      return [];
    return [
      {
        pathEpoch,
        firstTimestampMs,
        lastTimestampMs,
        sampleCount,
        maximumGapMs,
        complete: segment?.complete === true,
        missingReasons: textArray(segment?.missing_reasons),
      },
    ];
  });
  const tradeId = text(root.trade_id);
  const coverageStatus = text(root.coverage_status);
  const totalSampleCount = finite(root.total_sample_count);
  const rawCapturePolicy = record(root.capture_policy);
  const capturePolicy =
    rawCapturePolicy &&
    finite(rawCapturePolicy.fine_interval_ms) !== undefined &&
    finite(rawCapturePolicy.fine_window_ms) !== undefined &&
    finite(rawCapturePolicy.coarse_interval_ms) !== undefined &&
    finite(rawCapturePolicy.coarse_after_ms) !== undefined &&
    typeof rawCapturePolicy.event_driven_extrema === 'boolean' &&
    typeof rawCapturePolicy.event_driven_threshold_crossings === 'boolean'
      ? {
          fineIntervalMs: finite(rawCapturePolicy.fine_interval_ms) as number,
          fineWindowMs: finite(rawCapturePolicy.fine_window_ms) as number,
          coarseIntervalMs: finite(
            rawCapturePolicy.coarse_interval_ms,
          ) as number,
          coarseAfterMs: finite(rawCapturePolicy.coarse_after_ms) as number,
          eventDrivenExtrema: rawCapturePolicy.event_driven_extrema,
          eventDrivenThresholdCrossings:
            rawCapturePolicy.event_driven_threshold_crossings,
        }
      : undefined;
  const rawLifetimeEvidence = record(root.lifetime_excursion_evidence);
  const lifetimeSampleCount = finite(rawLifetimeEvidence?.event_sample_count);
  const lifetimeMaximumGap = finite(rawLifetimeEvidence?.event_maximum_gap_ms);
  const lifetimeMissingReasons = textArray(
    rawLifetimeEvidence?.missing_reasons,
  );
  const lifetimeExcursionEvidence =
    rawLifetimeEvidence &&
    lifetimeSampleCount !== undefined &&
    typeof rawLifetimeEvidence.complete === 'boolean'
      ? {
          grossMfePercent: finite(rawLifetimeEvidence.gross_mfe_percent),
          grossMaePercent: finite(rawLifetimeEvidence.gross_mae_percent),
          eventSampleCount: lifetimeSampleCount,
          eventMaximumGapMs: lifetimeMaximumGap,
          complete:
            rawLifetimeEvidence.complete && lifetimeMissingReasons.length === 0,
          thresholdCrossings: projectExecutableThresholdCrossings(
            rawLifetimeEvidence.threshold_crossings,
          ).sort((left, right) => left.timestampMs - right.timestampMs),
          missingReasons: lifetimeMissingReasons,
        }
      : undefined;
  const rawReplayEligibility = record(root.replay_eligibility);
  const replayStatus = rawReplayEligibility?.status;
  const requiredMaximumGapMs = finite(
    rawReplayEligibility?.required_maximum_gap_ms,
  );
  const maximumObservedGapMs = finite(
    rawReplayEligibility?.maximum_observed_gap_ms,
  );
  const replayEligibility =
    (replayStatus === 'eligible' ||
      replayStatus === 'ineligible_coarse_sampling' ||
      replayStatus === 'incomplete' ||
      replayStatus === 'unavailable') &&
    requiredMaximumGapMs !== undefined
      ? {
          status: replayStatus as TradePathReplayEligibility['status'],
          requiredMaximumGapMs,
          maximumObservedGapMs,
          reason: text(rawReplayEligibility?.reason),
        }
      : undefined;
  const rawHorizons = root.horizon_coverage;
  const horizonCoverage = Array.isArray(rawHorizons)
    ? rawHorizons.flatMap((item): TradePathHorizonCoverage[] => {
        const horizon = record(item);
        const id = horizon?.horizon;
        const targetIntervalMs = finite(horizon?.target_interval_ms);
        const sampleCount = finite(horizon?.sample_count);
        const maximumObservedGapMs = finite(horizon?.maximum_observed_gap_ms);
        if (
          (id !== 'fine_0_30m' && id !== 'coarse_after_30m') ||
          targetIntervalMs === undefined ||
          sampleCount === undefined ||
          typeof horizon?.complete !== 'boolean' ||
          typeof horizon.horizon_reached !== 'boolean'
        )
          return [];
        const missingReasons = textArray(horizon.missing_reasons);
        const entryOffsetKnown = !missingReasons.includes(
          'entry_timestamp_unavailable',
        );
        return [
          {
            horizon: id,
            ...(entryOffsetKnown
              ? {
                  offsetStartMs: finite(horizon.offset_start_ms),
                  offsetEndMs: finite(horizon.offset_end_ms),
                }
              : {}),
            targetIntervalMs,
            sampleCount,
            maximumObservedGapMs,
            complete: horizon.complete && missingReasons.length === 0,
            horizonReached: horizon.horizon_reached === true,
            missingReasons,
          },
        ];
      })
    : undefined;
  if (
    tradeId !== expectedTradeId ||
    coverageStatus === undefined ||
    totalSampleCount === undefined ||
    samples.length !== root.samples.length ||
    segments.length !== root.segments.length
  )
    return null;
  return {
    tradeId,
    coverageStatus,
    historyComplete: root.history_complete === true,
    totalSampleCount,
    samples,
    segments,
    missingReasons: textArray(root.missing_reasons),
    nextTimestampMs: finite(root.next_timestamp_ms),
    nextCursor: finite(root.next_cursor),
    hasMore: root.has_more === true,
    capturePolicy,
    lifetimeExcursionEvidence,
    replayEligibility,
    horizonCoverage:
      Array.isArray(rawHorizons) &&
      horizonCoverage?.length === rawHorizons.length
        ? horizonCoverage
        : undefined,
  };
}

export const EXCURSION_EVIDENCE_NOTICE =
  'Surfaced records are non-empirical; do not use them for natural-lifecycle, profitability, Learning, or promotion claims.';

export function excursionPanelStatus(
  view: ExcursionView,
  recordCount: number,
  completePathCount: number,
  postExitCount: number,
) {
  switch (view) {
    case 'what-if':
      return 'Research-only historical replay. Results are never Learning eligible.';
    case 'continuation':
      return postExitCount > 0
        ? `${postExitCount} records include post-exit observations; horizon coverage and cohort outcomes are not provided.`
        : 'Unavailable: no post-exit observation evidence is available.';
    case 'transitions':
      return 'Unavailable: original-cohort terminal transition counts and complete-window denominators are not provided.';
    case 'sensitivity':
      return 'Unavailable: no replay-backed policy sensitivity results are provided.';
    case 'replay':
      return completePathCount > 0
        ? 'Unavailable: aggregate complete-path evidence exists, but timestamped executable path points are not provided.'
        : 'Unavailable: no complete executable paths or timestamped path points are provided.';
    case 'overview':
      return recordCount === 0 ? 'No records match the selected filters.' : '';
  }
}

export type ExcursionTrade = {
  id: string;
  stableTradeId?: string;
  trade: AutoTraderOutcome;
  executablePathComplete: boolean;
  incompletePathReason?: string;
  grossMfePercent?: number;
  grossMaePercent?: number;
  thresholdCrossings: ExecutableThresholdCrossing[];
  thresholdCrossingEvidenceStatus?: string;
  initialRiskEligible: boolean;
  originalRiskQuote?: number;
  postExitAvailable: boolean;
  profileProvenanceAvailable: boolean;
};

export type AutoTraderExcursionsState = {
  active: boolean;
  loading: boolean;
  error?: string;
  view: ExcursionView;
  source: ExcursionSource;
  range: ExcursionRange;
  strategy: string;
  strategyOptions: string[];
  trades: ExcursionTrade[];
  selectedTradeId?: string;
  pathLoading: boolean;
  pathError?: string;
  pathTradeId?: string;
  pathPage?: TradePricePathPage;
  whatIfDraft: WhatIfDraft;
  whatIfReplay: WhatIfReplayState;
  sourceCount: number;
  eligiblePathCount: number;
  incompletePathCount: number;
  whatIfAvailable: true;
};

/** Model-owned filtering and selection for the read-only excursion research view. */
export class AutoTraderExcursionsModel {
  readonly state$ = new BehaviorSubject<AutoTraderExcursionsState>({
    active: false,
    loading: true,
    view: 'overview',
    source: 'all',
    range: '30d',
    strategy: 'all',
    strategyOptions: [],
    trades: [],
    whatIfDraft: whatIfDraftForTrade(),
    whatIfReplay: { status: 'idle' },
    sourceCount: 0,
    eligiblePathCount: 0,
    incompletePathCount: 0,
    pathLoading: false,
    whatIfAvailable: true,
  });
  private subscription?: Subscription;
  private sourceRows: AutoTraderOutcome[] = [];
  private sourceLoading = true;
  private sourceError?: string;
  private readonly outcomes: AutoTraderOutcomesModel;
  private readonly fetchPath: ExcursionPathFetcher;
  private readonly fetchWhatIf: ExcursionWhatIfFetcher;
  private pathRequestGeneration = 0;
  private whatIfRequestGeneration = 0;

  constructor(
    outcomes: AutoTraderOutcomesModel,
    fetchPath: ExcursionPathFetcher = (path) => autoTraderRpc('GET', path),
    fetchWhatIf: ExcursionWhatIfFetcher = (path, body) =>
      autoTraderRpc('POST', path, body),
  ) {
    this.outcomes = outcomes;
    this.fetchPath = fetchPath;
    this.fetchWhatIf = fetchWhatIf;
  }

  start() {
    if (this.subscription) return;
    this.outcomes.start();
    this.subscription = this.outcomes.state$.subscribe((state) => {
      this.sourceLoading = state.loading;
      this.sourceError = state.error;
      this.sourceRows = [...state.rows, ...state.closedLedgerRows];
      this.project();
    });
    this.state$.next({ ...this.state$.value, active: true });
    const state = this.state$.value;
    if (state.view === 'replay' && state.selectedTradeId && !state.pathPage)
      void this.loadSelectedPath(false);
  }

  stop() {
    if (!this.subscription && !this.state$.value.active) return;
    this.subscription?.unsubscribe();
    this.subscription = undefined;
    this.outcomes.stop();
    this.pathRequestGeneration++;
    this.cancelWhatIf();
    this.state$.next({
      ...this.state$.value,
      active: false,
      pathLoading: false,
    });
  }

  setView(view: ExcursionView) {
    const current = this.state$.value;
    if (current.view === 'what-if' && view !== 'what-if') this.cancelWhatIf();
    const state = this.state$.value;
    this.state$.next({ ...state, view });
    const selected = state.trades.find(
      (row) => row.id === state.selectedTradeId,
    );
    if (
      (view === 'replay' || view === 'what-if') &&
      selected &&
      !state.pathLoading &&
      (!selected.stableTradeId ||
        state.pathError ||
        state.pathPage?.tradeId !== selected.stableTradeId ||
        (view === 'what-if' && !state.pathPage.replayEligibility))
    )
      void this.loadSelectedPath(false);
  }

  showTradePath(id: string) {
    const alreadyOnReplay = this.state$.value.view === 'replay';
    if (!this.state$.value.trades.some((row) => row.id === id)) return;
    this.selectTrade(id);
    if (!alreadyOnReplay) this.setView('replay');
  }

  setSource(source: ExcursionSource) {
    this.cancelWhatIf();
    this.invalidatePath();
    this.state$.next({
      ...this.state$.value,
      source,
      selectedTradeId: undefined,
      whatIfDraft: whatIfDraftForTrade(),
      whatIfReplay: { status: 'idle' },
    });
    this.project();
  }

  setRange(range: ExcursionRange) {
    this.cancelWhatIf();
    this.invalidatePath();
    this.state$.next({
      ...this.state$.value,
      range,
      selectedTradeId: undefined,
      whatIfDraft: whatIfDraftForTrade(),
      whatIfReplay: { status: 'idle' },
    });
    this.project();
  }

  setStrategy(strategy: string) {
    this.cancelWhatIf();
    this.invalidatePath();
    this.state$.next({
      ...this.state$.value,
      strategy,
      selectedTradeId: undefined,
      whatIfDraft: whatIfDraftForTrade(),
      whatIfReplay: { status: 'idle' },
    });
    this.project();
  }

  selectTrade(id?: string) {
    const current = this.state$.value;
    const exists =
      id === undefined || current.trades.some((row) => row.id === id);
    if (!exists) return;
    const selected = current.trades.find((row) => row.id === id);
    if (current.selectedTradeId !== id) this.cancelWhatIf();
    this.invalidatePath();
    this.state$.next({
      ...this.state$.value,
      selectedTradeId: id,
      pathTradeId: id,
      pathLoading: false,
      pathError: undefined,
      pathPage: undefined,
      whatIfDraft:
        current.selectedTradeId === id
          ? current.whatIfDraft
          : whatIfDraftForTrade(selected),
      whatIfReplay:
        current.selectedTradeId === id
          ? current.whatIfReplay
          : { status: 'idle' },
    });
    if (
      id &&
      (this.state$.value.view === 'replay' ||
        this.state$.value.view === 'what-if')
    )
      void this.loadSelectedPath(false);
  }

  async loadMorePath() {
    await this.loadSelectedPath(true);
  }

  setWhatIfParameter(name: WhatIfParameterName, value: number | null) {
    this.cancelWhatIf();
    const state = this.state$.value;
    this.state$.next({
      ...state,
      whatIfDraft: {
        ...state.whatIfDraft,
        parameters: { ...state.whatIfDraft.parameters, [name]: value },
      },
      whatIfReplay: { status: 'idle' },
    });
  }

  setWhatIfMaximumHoldDays(value: number | null) {
    this.cancelWhatIf();
    const state = this.state$.value;
    this.state$.next({
      ...state,
      whatIfDraft: { ...state.whatIfDraft, maximumHoldDays: value },
      whatIfReplay: { status: 'idle' },
    });
  }

  canRunWhatIf(): boolean {
    const state = this.state$.value;
    const selected = state.trades.find(
      (row) => row.id === state.selectedTradeId,
    );
    return readyWhatIfRequest(state.whatIfDraft, selected, state.pathPage)
      .ready;
  }

  whatIfReadinessReason(): string | undefined {
    const state = this.state$.value;
    const selected = state.trades.find(
      (row) => row.id === state.selectedTradeId,
    );
    const readiness = readyWhatIfRequest(
      state.whatIfDraft,
      selected,
      state.pathPage,
    );
    return readiness.ready ? undefined : readiness.reason;
  }

  cancelWhatIf() {
    this.whatIfRequestGeneration++;
    const current = this.state$.value;
    if (current.whatIfReplay.status === 'loading')
      this.state$.next({
        ...current,
        whatIfReplay: {
          status: 'cancelled',
          tradeId: current.whatIfReplay.tradeId,
          reason: 'Replay request cancelled; no result was applied.',
        },
      });
  }

  async runWhatIf() {
    const state = this.state$.value;
    const selected = state.trades.find(
      (row) => row.id === state.selectedTradeId,
    );
    const readiness = readyWhatIfRequest(
      state.whatIfDraft,
      selected,
      state.pathPage,
    );
    if (!readiness.ready || !selected) {
      this.state$.next({
        ...state,
        whatIfReplay: {
          status: 'unavailable',
          tradeId: selected?.stableTradeId,
          reason: readiness.ready
            ? 'Select a trade before running replay.'
            : readiness.reason,
        },
      });
      return;
    }
    const tradeId = selected.stableTradeId;
    if (!tradeId) return;
    const generation = ++this.whatIfRequestGeneration;
    this.state$.next({
      ...state,
      whatIfReplay: { status: 'loading', tradeId },
    });
    try {
      const response = await this.fetchWhatIf(
        '/positions/auto-trader/trade-price-path/what-if',
        readiness.body,
      );
      if (!response.ok)
        throw new Error(
          `Historical What-if replay unavailable (HTTP ${response.status})`,
        );
      const parsed = parseWhatIfReplayResult(await response.json(), tradeId);
      if (!parsed)
        throw new Error('Historical What-if response contract unavailable');
      if (
        generation !== this.whatIfRequestGeneration ||
        this.state$.value.selectedTradeId !== selected.id ||
        this.state$.value.whatIfDraft.tradeId !== tradeId
      )
        return;
      this.state$.next({ ...this.state$.value, whatIfReplay: parsed });
    } catch (error) {
      if (
        generation !== this.whatIfRequestGeneration ||
        this.state$.value.selectedTradeId !== selected.id
      )
        return;
      this.state$.next({
        ...this.state$.value,
        whatIfReplay: {
          status: 'error',
          tradeId,
          error:
            error instanceof Error
              ? error.message
              : 'Historical What-if replay failed.',
        },
      });
    }
  }

  private invalidatePath() {
    this.pathRequestGeneration++;
  }

  private async loadSelectedPath(append: boolean) {
    const state = this.state$.value;
    const selected = state.trades.find(
      (row) => row.id === state.selectedTradeId,
    );
    if (!selected) return;
    const tradeId = selected.stableTradeId;
    if (!tradeId) {
      this.state$.next({
        ...state,
        pathTradeId: selected.id,
        pathLoading: false,
        pathError: 'No stable canonical trade ID is available for path lookup.',
        pathPage: undefined,
      });
      return;
    }
    const prior =
      state.pathPage?.tradeId === tradeId ? state.pathPage : undefined;
    if (
      append &&
      (!prior?.hasMore ||
        prior.nextTimestampMs === undefined ||
        prior.nextCursor === undefined)
    )
      return;
    const generation = ++this.pathRequestGeneration;
    const query = new URLSearchParams({
      trade_id: tradeId,
      limit: String(EXCURSION_PATH_PAGE_SIZE),
      ...(append && prior
        ? {
            after_timestamp_ms: String(prior.nextTimestampMs),
            after_cursor: String(prior.nextCursor),
          }
        : {}),
    });
    this.state$.next({
      ...state,
      pathTradeId: selected.id,
      pathLoading: true,
      pathError: undefined,
    });
    try {
      const response = await this.fetchPath(
        `/positions/auto-trader/trade-price-path?${query.toString()}`,
      );
      if (!response.ok)
        throw new Error(
          `Trade price path unavailable (HTTP ${response.status})`,
        );
      const page = parseTradePricePathPage(await response.json(), tradeId);
      if (!page) throw new Error('Trade price path contract unavailable');
      if (
        generation !== this.pathRequestGeneration ||
        this.state$.value.selectedTradeId !== selected.id
      )
        return;
      this.state$.next({
        ...this.state$.value,
        pathLoading: false,
        pathError: undefined,
        pathTradeId: selected.id,
        pathPage:
          append && prior
            ? {
                ...page,
                samples: [...prior.samples, ...page.samples],
              }
            : page,
      });
    } catch (error) {
      if (
        generation !== this.pathRequestGeneration ||
        this.state$.value.selectedTradeId !== selected.id
      )
        return;
      this.state$.next({
        ...this.state$.value,
        pathLoading: false,
        pathError:
          error instanceof Error
            ? error.message
            : 'Trade price path unavailable',
      });
    }
  }

  private project() {
    const state = this.state$.value;
    const since =
      state.range === 'all'
        ? undefined
        : Date.now() - (state.range === '30d' ? 30 : 90) * 86_400_000;
    const unique = new Map<string, AutoTraderOutcome>();
    for (const row of this.sourceRows)
      unique.set(row.tradeId ?? row.rowId, row);
    const sourceAndRange = [...unique.values()].filter((row) => {
      const isScout = row.scout || row.source === 'scout';
      return (
        (state.source === 'all' ||
          (state.source === 'scouts' ? isScout : !isScout)) &&
        (since === undefined ||
          row.closedAt === undefined ||
          row.closedAt >= since)
      );
    });
    const strategyOptions = [
      ...new Set(sourceAndRange.map((row) => row.strategy).filter(Boolean)),
    ].sort((left, right) => left.localeCompare(right));
    const selected = sourceAndRange.filter(
      (row) => state.strategy === 'all' || row.strategy === state.strategy,
    );
    const trades = selected.map((trade) => ({
      id: trade.tradeId ?? trade.rowId,
      stableTradeId:
        trade.tradeId ??
        (typeof trade.ledgerTrade?.lifecycle?.trade_id === 'string'
          ? trade.ledgerTrade.lifecycle.trade_id
          : undefined),
      trade,
      executablePathComplete:
        trade.executablePricePathEvidence?.complete === true,
      incompletePathReason:
        trade.executablePricePathEvidence?.complete === true
          ? undefined
          : trade.executablePricePathEvidence?.missingCoverage?.length
            ? trade.executablePricePathEvidence.missingCoverage.join(', ')
            : trade.executablePricePathEvidence
              ? 'Source reports an incomplete path without a specific reason.'
              : 'No executable path evidence was returned for this record.',
      grossMfePercent:
        trade.executablePricePathEvidence?.complete === true
          ? trade.executablePricePathEvidence.grossExecutableMfePercent
          : undefined,
      grossMaePercent:
        trade.executablePricePathEvidence?.complete === true
          ? trade.executablePricePathEvidence.grossExecutableMaePercent
          : undefined,
      thresholdCrossings: [
        ...(trade.executablePricePathEvidence?.thresholdCrossings ?? []),
      ].sort((left, right) => left.timestampMs - right.timestampMs),
      thresholdCrossingEvidenceStatus:
        trade.executablePricePathEvidence?.thresholdCrossingEvidenceStatus,
      initialRiskEligible:
        trade.initialRiskEvidence?.eligibleForRMultiple === true,
      originalRiskQuote: trade.initialRiskEvidence?.eligibleForRMultiple
        ? trade.initialRiskEvidence.quoteAmount
        : undefined,
      postExitAvailable: trade.postExitObservedAt !== undefined,
      profileProvenanceAvailable: Boolean(
        trade.exitProfileId && trade.exitProfileVersion !== undefined,
      ),
    }));
    const selectedTradeId = trades.some(
      (row) => row.id === state.selectedTradeId,
    )
      ? state.selectedTradeId
      : undefined;
    const pathStillSelected = trades.some(
      (row) => row.id === selectedTradeId && row.id === state.pathTradeId,
    );
    const selectedStillPresent = trades.find(
      (row) => row.id === selectedTradeId,
    );
    const whatIfDraftStillSelected =
      selectedTradeId !== undefined &&
      state.whatIfDraft.tradeId === selectedStillPresent?.stableTradeId;
    if (!pathStillSelected && state.pathTradeId !== undefined)
      this.invalidatePath();
    if (
      selectedTradeId !== state.selectedTradeId ||
      (!whatIfDraftStillSelected && state.whatIfDraft.tradeId !== undefined)
    )
      this.cancelWhatIf();
    this.state$.next({
      ...state,
      loading: this.sourceLoading,
      error: this.sourceError,
      trades,
      strategyOptions,
      selectedTradeId,
      whatIfDraft: whatIfDraftStillSelected
        ? state.whatIfDraft
        : whatIfDraftForTrade(selectedStillPresent),
      whatIfReplay: whatIfDraftStillSelected
        ? state.whatIfReplay
        : { status: 'idle' },
      ...(pathStillSelected
        ? {}
        : {
            pathLoading: false,
            pathError: undefined,
            pathTradeId: undefined,
            pathPage: undefined,
          }),
      sourceCount: selected.length,
      eligiblePathCount: trades.filter((row) => row.executablePathComplete)
        .length,
      incompletePathCount: trades.filter((row) => !row.executablePathComplete)
        .length,
      whatIfAvailable: true,
    });
  }
}
