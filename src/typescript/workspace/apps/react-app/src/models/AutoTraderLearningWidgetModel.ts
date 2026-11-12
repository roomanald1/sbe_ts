import { BehaviorSubject, type Subscription } from 'rxjs';
import { AutoTraderModel, type AutoTraderState } from './AutoTraderModel';
import type { LearningSummary } from './AutoTraderOutcomesModel';
import {
  summarizeLearningEffectiveness,
  type LearningEffectivenessSummary,
} from './LearningEffectivenessModel';
import { autoTraderRpc } from '../auto_trader_rsocket';

export type LearningRow = {
  id: string;
  symbol: string;
  strategy: string;
  parameter: string;
  unit: string;
  step: number;
  base: number;
  candidate: number;
  effective: number;
  pairedDelta: number | null;
  pairedDeltaSamples: number;
  pairedUnknownSamples: number;
  pairedDeltaEstimated: boolean;
  samples: number;
  minimumSamples: number;
  averageLift: number | null;
  lower95: number | null;
  upper95: number | null;
  promoted: boolean;
  rejected: boolean;
  status: string;
};

export type TradeEffectivenessRow = {
  strategy: string;
  activeSamples: number;
  activePositiveRate: number | null;
  activeAverage: number | null;
  otherSamples: number;
  otherPositiveRate: number | null;
  otherAverage: number | null;
  averageDifference: number | null;
};

export type AutoTraderLearningWidgetState = {
  source: AutoTraderState;
  summary: LearningSummary[] | null;
  summaryError?: string;
  rows: LearningRow[];
  visibleRows: LearningRow[];
  promotedOnly: boolean;
  effectiveness: LearningEffectivenessSummary;
  actualTradeRows: TradeEffectivenessRow[];
  profileReturnDifference: number | null;
  activeCandidates: number;
  rejectedCandidates: number;
  qualifiedCandidates: number;
  collectingCandidates: number;
};

type SourceModel = Pick<AutoTraderModel, 'state$' | 'start' | 'stop'>;
type GetSummary = () => Promise<LearningSummary[]>;

export function projectLearningWidgetState(
  source: AutoTraderState,
  summary: LearningSummary[] | null,
  summaryError: string | undefined,
  promotedOnly: boolean,
): AutoTraderLearningWidgetState {
  const profiles = source.decisionParameters?.strategy_profiles ?? [];
  const rows = profiles.flatMap((profile) =>
    profile.parameters.flatMap((parameter) =>
      (parameter.scout_variants ?? []).map((variant): LearningRow => {
        const base = parameter.configured_value ?? parameter.default_value;
        const pairedCount = variant.paired_baseline_delta_samples ?? 0;
        const unknownCount = variant.paired_baseline_unknown_samples ?? 0;
        return {
          id: `${profile.symbol}:${profile.strategy}:${parameter.key}:${variant.candidate_value.toFixed(8)}`,
          symbol: profile.symbol,
          strategy: profile.strategy,
          parameter: parameter.label,
          unit: parameter.unit,
          step: parameter.step,
          base,
          candidate: variant.candidate_value,
          effective: parameter.value,
          pairedDelta:
            pairedCount > 0
              ? (variant.paired_baseline_delta_mean ?? null)
              : unknownCount > 0
                ? variant.candidate_value - base
                : null,
          pairedDeltaSamples: pairedCount > 0 ? pairedCount : unknownCount,
          pairedUnknownSamples: pairedCount > 0 ? unknownCount : 0,
          pairedDeltaEstimated: pairedCount === 0 && unknownCount > 0,
          samples: variant.sample_count,
          minimumSamples: variant.minimum_samples,
          averageLift: variant.average_net_improvement_percent,
          lower95: variant.lower_90_one_sided_improvement_percent ?? null,
          upper95: variant.upper_90_one_sided_improvement_percent ?? null,
          promoted: variant.promoted,
          rejected: variant.rejected,
          status: variant.promoted
            ? 'Active'
            : variant.rejected
              ? 'Avoid'
              : variant.sample_count < variant.minimum_samples
                ? 'Collecting'
                : (variant.lower_90_one_sided_improvement_percent ?? 0) > 0
                  ? 'Qualified · not active'
                  : 'Not qualified',
        };
      }),
    ),
  );

  const rowsById = new Map(rows.map((row) => [row.id, row]));
  for (const pending of source.pendingScoutParameters) {
    const profile = profiles.find(
      (candidate) =>
        candidate.symbol === pending.symbol &&
        candidate.strategy === pending.strategy,
    );
    const parameter = profile?.parameters.find(
      (candidate) => candidate.key === pending.parameter,
    );
    if (!profile || !parameter) continue;
    const id = `${profile.symbol}:${profile.strategy}:${parameter.key}:${pending.candidateValue.toFixed(8)}`;
    const existing = rowsById.get(id);
    if (existing) {
      if (!existing.promoted && existing.samples < existing.minimumSamples) {
        existing.status = `Collecting · ${pending.openSamples} open`;
      }
      continue;
    }
    const row: LearningRow = {
      id,
      symbol: profile.symbol,
      strategy: profile.strategy,
      parameter: parameter.label,
      unit: parameter.unit,
      step: parameter.step,
      base: parameter.configured_value ?? parameter.default_value,
      candidate: pending.candidateValue,
      effective: parameter.value,
      pairedDelta: null,
      pairedDeltaSamples: 0,
      pairedUnknownSamples: 0,
      pairedDeltaEstimated: false,
      samples: 0,
      minimumSamples: Number(source.minimumScoutParameterSamples),
      averageLift: null,
      lower95: null,
      upper95: null,
      promoted: false,
      rejected: false,
      status: `Collecting · ${pending.openSamples} open`,
    };
    rows.push(row);
    rowsById.set(id, row);
  }

  const effectiveness = summarizeLearningEffectiveness(summary ?? []);
  const actualTradeRows = (summary ?? [])
    .filter(
      (item) =>
        (item.active_profile_trade_samples ?? 0) > 0 ||
        (item.other_profile_trade_samples ?? 0) > 0,
    )
    .map((item): TradeEffectivenessRow => {
      const activeAverage =
        item.active_profile_average_net_return_percent ?? null;
      const otherAverage =
        item.other_profile_average_net_return_percent ?? null;
      return {
        strategy: item.strategy,
        activeSamples: item.active_profile_trade_samples ?? 0,
        activePositiveRate: item.active_profile_trade_positive_rate ?? null,
        activeAverage,
        otherSamples: item.other_profile_trade_samples ?? 0,
        otherPositiveRate: item.other_profile_trade_positive_rate ?? null,
        otherAverage,
        averageDifference:
          activeAverage != null && otherAverage != null
            ? activeAverage - otherAverage
            : null,
      };
    });
  const visibleRows = promotedOnly ? rows.filter((row) => row.promoted) : rows;

  return {
    source,
    summary,
    summaryError,
    rows,
    visibleRows,
    promotedOnly,
    effectiveness,
    actualTradeRows,
    profileReturnDifference:
      effectiveness.activeProfileReturn != null &&
      effectiveness.otherProfileReturn != null
        ? effectiveness.activeProfileReturn - effectiveness.otherProfileReturn
        : null,
    activeCandidates: rows.filter((row) => row.promoted).length,
    rejectedCandidates: rows.filter((row) => row.rejected).length,
    qualifiedCandidates: rows.filter(
      (row) => row.status === 'Qualified · not active',
    ).length,
    collectingCandidates: rows.filter((row) =>
      row.status.startsWith('Collecting'),
    ).length,
  };
}

const requestLearningSummary: GetSummary = async () => {
  const response = await autoTraderRpc(
    'GET',
    '/positions/auto-trader/learning-summary',
  );
  if (!response.ok) {
    throw new Error(`Learning summary request failed (${response.status}).`);
  }
  return (await response.json()) as LearningSummary[];
};

export class AutoTraderLearningWidgetModel {
  readonly state$: BehaviorSubject<AutoTraderLearningWidgetState>;
  private sourceSubscription?: Subscription;
  private refreshTimer?: ReturnType<typeof setInterval>;
  private promotedOnly = false;
  private summary: LearningSummary[] | null = null;
  private summaryError?: string;
  private running = false;
  private readonly source: SourceModel;
  private readonly getSummary: GetSummary;
  private readonly refreshIntervalMs: number;

  constructor(
    source: SourceModel = new AutoTraderModel(),
    getSummary: GetSummary = requestLearningSummary,
    refreshIntervalMs = 30_000,
  ) {
    this.source = source;
    this.getSummary = getSummary;
    this.refreshIntervalMs = refreshIntervalMs;
    this.state$ = new BehaviorSubject(
      projectLearningWidgetState(source.state$.value, null, undefined, false),
    );
  }

  start() {
    if (this.running) return;
    this.running = true;
    this.source.start();
    this.sourceSubscription = this.source.state$.subscribe((source) =>
      this.publish(source),
    );
    void this.refreshSummary();
    this.refreshTimer = setInterval(
      () => void this.refreshSummary(),
      this.refreshIntervalMs,
    );
  }

  stop() {
    if (!this.running) return;
    this.running = false;
    this.sourceSubscription?.unsubscribe();
    this.sourceSubscription = undefined;
    if (this.refreshTimer) clearInterval(this.refreshTimer);
    this.refreshTimer = undefined;
    this.source.stop();
  }

  setPromotedOnly(promotedOnly: boolean) {
    this.promotedOnly = promotedOnly;
    this.publish(this.source.state$.value);
  }

  async refreshSummary() {
    try {
      this.summary = await this.getSummary();
      this.summaryError = undefined;
    } catch (error) {
      this.summaryError =
        error instanceof Error
          ? error.message
          : 'Could not load Learning summary.';
    }
    this.publish(this.source.state$.value);
  }

  private publish(source: AutoTraderState) {
    this.state$.next(
      projectLearningWidgetState(
        source,
        this.summary,
        this.summaryError,
        this.promotedOnly,
      ),
    );
  }
}
