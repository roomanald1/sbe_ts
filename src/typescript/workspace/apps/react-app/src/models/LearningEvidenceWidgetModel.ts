import { BehaviorSubject, type Subscription } from 'rxjs';
import {
  AutoTraderLearningWidgetModel,
  type AutoTraderLearningWidgetState,
} from './AutoTraderLearningWidgetModel';

export type LearningEvidenceRequestRow = {
  id: string;
  symbol: string;
  strategy: string;
  profile: string;
  regime: string;
  horizon: string;
  status: string;
  reason: string | null;
  attempts: number | null;
  createdAt: number | null;
  updatedAt: number | null;
  nextAttemptAt: number | null;
};

export type LearningEvidenceCohortRow = {
  id: string;
  symbol: string;
  strategy: string;
  profile: string;
  regime: string | null;
  horizon: string | null;
  freshness: string | null;
  gapReason: string | null;
  rawSamples: number | null;
  effectiveSamples: number | null;
  lastSampleAt: number | null;
  sampleAgeMs: number | null;
  coverageScope: string;
};

export type LearningEvidenceWidgetView = {
  profilesMonitored: number | null;
  strategiesMonitored: number | null;
  evidenceGaps: number | null;
  requests: LearningEvidenceRequestRow[] | null;
  requestTotal: number | null;
  omittedRequests: number | null;
  cohorts: LearningEvidenceCohortRow[] | null;
  cohortTotal: number | null;
  omittedCohorts: number | null;
  requestStates: Array<{ status: string; count: number }> | null;
  scoutEvidence: Array<{
    strategy: string;
    samples: number;
    positiveRate: number | null;
    averageReturnPercent: number | null;
  }> | null;
  actualTradeEvidence: Array<{
    strategy: string;
    activeSamples: number;
    activeAverageReturnPercent: number | null;
    otherSamples: number;
    otherAverageReturnPercent: number | null;
  }> | null;
  decisionBlockers: Array<{ strategy: string; symbol: string; reason: string }>;
  coverageError?: string;
  learningSummaryError?: string;
  profileError?: string;
};

export type LearningEvidenceTab =
  | 'evidence'
  | 'activity'
  | 'history'
  | 'promotion';
export type LearningEvidenceWidgetState = {
  view: LearningEvidenceWidgetView;
  tab: LearningEvidenceTab;
  filters: { symbol: string; strategy: string };
  symbols: string[];
  strategies: string[];
  visibleCohorts: LearningEvidenceCohortRow[] | null;
  visibleRequests: LearningEvidenceRequestRow[] | null;
  visibleScoutEvidence: LearningEvidenceWidgetView['scoutEvidence'];
  visibleActualTradeEvidence: LearningEvidenceWidgetView['actualTradeEvidence'];
  visibleDecisionBlockers: LearningEvidenceWidgetView['decisionBlockers'];
  refreshing: boolean;
  refreshError: string | null;
  lastRefreshAttemptAt: number | null;
  dataStatus: 'inactive' | 'loading' | 'available' | 'partial' | 'unavailable';
  stale: boolean;
};

const KNOWN_REQUEST_STATES = new Set([
  'pending',
  'running',
  'dispatched',
  'capacity_blocked',
  'waiting_for_market',
  'completed',
  'failed',
]);

function finiteNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

export function projectLearningEvidenceWidget(
  state: AutoTraderLearningWidgetState,
): LearningEvidenceWidgetView {
  const parameters = state.source.decisionParameters;
  const profiles = parameters?.strategy_profiles;
  const evidence = state.evidenceRequests;
  const requestRows =
    evidence?.requests.map(({ request }) => ({
      id: `${request.symbol}:${request.strategy}:${request.profile_id}:${request.regime}:${request.horizon}:${request.created_at_ms}`,
      symbol: request.symbol,
      strategy: request.strategy,
      profile: request.profile_id,
      regime: request.regime,
      horizon: request.horizon,
      status: request.status,
      reason: request.reason,
      attempts: finiteNumber(request.attempts),
      createdAt: finiteNumber(request.created_at_ms),
      updatedAt: finiteNumber(request.updated_at_ms),
      nextAttemptAt: finiteNumber(request.next_attempt_at_ms),
    })) ?? null;

  const cohorts =
    evidence?.coverage_cohorts.map((cohort) => ({
      id: `${cohort.symbol}:${cohort.strategy}:${cohort.profile_id}:${cohort.regime ?? ''}:${cohort.horizon ?? ''}`,
      symbol: cohort.symbol,
      strategy: cohort.strategy,
      profile: cohort.profile_id,
      regime: cohort.regime,
      horizon: cohort.horizon,
      freshness: cohort.coverage_status,
      gapReason: cohort.gap_reason,
      rawSamples: finiteNumber(cohort.sample_count),
      effectiveSamples: finiteNumber(cohort.effective_sample_size),
      lastSampleAt: finiteNumber(cohort.last_sample_timestamp_ms),
      sampleAgeMs: finiteNumber(cohort.last_sample_age_ms),
      coverageScope: cohort.coverage_scope,
    })) ?? null;

  const stateCounts = new Map<string, number>();
  for (const request of evidence?.requests ?? []) {
    const status = KNOWN_REQUEST_STATES.has(request.request.status)
      ? request.request.status
      : 'other_reported_state';
    stateCounts.set(status, (stateCounts.get(status) ?? 0) + 1);
  }

  return {
    profilesMonitored: profiles ? profiles.length : null,
    strategiesMonitored: profiles
      ? new Set(profiles.map((profile) => profile.strategy)).size
      : null,
    evidenceGaps: evidence
      ? evidence.coverage_cohorts.filter(
          (cohort) =>
            cohort.coverage_status === 'missing' ||
            cohort.coverage_status === 'stale' ||
            cohort.gap_reason != null,
        ).length
      : null,
    requests: requestRows,
    requestTotal: evidence?.total_count ?? null,
    omittedRequests: evidence?.omitted_count ?? null,
    cohorts,
    cohortTotal: evidence?.total_coverage_cohorts ?? null,
    omittedCohorts: evidence?.omitted_coverage_cohorts ?? null,
    requestStates: evidence
      ? [...stateCounts.entries()].map(([status, count]) => ({ status, count }))
      : null,
    scoutEvidence:
      state.summary?.flatMap((summary) =>
        typeof summary.scout_outcome_samples === 'number'
          ? [
              {
                strategy: summary.strategy,
                samples: summary.scout_outcome_samples,
                positiveRate: finiteNumber(summary.scout_positive_rate),
                averageReturnPercent: finiteNumber(
                  summary.scout_average_net_return_percent,
                ),
              },
            ]
          : [],
      ) ?? null,
    actualTradeEvidence:
      state.summary?.flatMap((summary) => {
        const activeSamples = finiteNumber(
          summary.active_profile_trade_samples,
        );
        const otherSamples = finiteNumber(summary.other_profile_trade_samples);
        if (activeSamples == null && otherSamples == null) return [];
        return [
          {
            strategy: summary.strategy,
            activeSamples: activeSamples ?? 0,
            activeAverageReturnPercent: finiteNumber(
              summary.active_profile_average_net_return_percent,
            ),
            otherSamples: otherSamples ?? 0,
            otherAverageReturnPercent: finiteNumber(
              summary.other_profile_average_net_return_percent,
            ),
          },
        ];
      }) ?? null,
    decisionBlockers: state.decisionCoverage.blockers.map((blocker) => ({
      strategy: blocker.strategy,
      symbol: blocker.symbol,
      reason: blocker.reason,
    })),
    coverageError: state.evidenceRequestError,
    learningSummaryError: state.summaryError,
    profileError: state.source.decisionParametersError,
  };
}

export class LearningEvidenceWidgetModel {
  readonly state$: BehaviorSubject<LearningEvidenceWidgetState>;
  private readonly createSourceModel: () => AutoTraderLearningWidgetModel;
  private sourceModel: AutoTraderLearningWidgetModel;
  private sourceSubscription?: Subscription;
  private sourceState: AutoTraderLearningWidgetState;
  private running = false;
  private generation = 0;
  private tab: LearningEvidenceTab = 'evidence';
  private filters = { symbol: '', strategy: '' };
  private refreshing = false;
  private refreshError: string | null = null;
  private lastRefreshAttemptAt: number | null = null;

  constructor(
    createSourceModel: () => AutoTraderLearningWidgetModel = () =>
      new AutoTraderLearningWidgetModel(),
  ) {
    this.createSourceModel = createSourceModel;
    this.sourceModel = createSourceModel();
    this.sourceState = this.sourceModel.state$.value;
    this.state$ = new BehaviorSubject(this.project());
  }

  start() {
    if (this.running) return;
    this.running = true;
    const generation = ++this.generation;
    this.sourceSubscription = this.sourceModel.state$.subscribe((state) => {
      if (!this.running || generation !== this.generation) return;
      this.sourceState = state;
      this.state$.next(this.project());
    });
    this.sourceModel.start();
  }

  stop() {
    if (!this.running) return;
    this.running = false;
    this.generation += 1;
    this.sourceSubscription?.unsubscribe();
    this.sourceSubscription = undefined;
    this.sourceModel.stop();
    // A stopped source may still settle requests. Drop its instance so a later
    // activation cannot subscribe to those responses.
    this.sourceModel = this.createSourceModel();
    this.sourceState = this.sourceModel.state$.value;
    this.refreshing = false;
    this.state$.next(this.project());
  }

  setTab(tab: LearningEvidenceTab) {
    this.tab = tab;
    this.state$.next(this.project());
  }

  setFilters(filters: Partial<LearningEvidenceWidgetState['filters']>) {
    this.filters = {
      symbol: filters.symbol ?? this.filters.symbol,
      strategy: filters.strategy ?? this.filters.strategy,
    };
    this.state$.next(this.project());
  }

  async refresh(): Promise<void> {
    if (!this.running) return;
    const generation = this.generation;
    this.refreshing = true;
    this.refreshError = null;
    this.state$.next(this.project());
    const sourceModel = this.sourceModel;
    try {
      await Promise.all([
        sourceModel.refreshSummary(),
        sourceModel.refreshDecisions(),
        sourceModel.refreshEvidenceRequests(),
      ]);
    } catch (error) {
      if (
        this.running &&
        generation === this.generation &&
        sourceModel === this.sourceModel
      )
        this.refreshError =
          error instanceof Error
            ? error.message
            : 'Learning evidence refresh failed';
    } finally {
      if (
        this.running &&
        generation === this.generation &&
        sourceModel === this.sourceModel
      ) {
        this.sourceState = sourceModel.state$.value;
        this.refreshing = false;
        this.lastRefreshAttemptAt = Date.now();
        this.state$.next(this.project());
      }
    }
  }

  private project(): LearningEvidenceWidgetState {
    const view = projectLearningEvidenceWidget(this.sourceState);
    const symbols = [
      ...new Set([
        ...(view.cohorts ?? []).map((row) => row.symbol),
        ...(view.requests ?? []).map((row) => row.symbol),
        ...view.decisionBlockers.map((row) => row.symbol),
      ]),
    ].sort();
    const strategies = [
      ...new Set([
        ...(view.cohorts ?? []).map((row) => row.strategy),
        ...(view.requests ?? []).map((row) => row.strategy),
        ...(view.scoutEvidence ?? []).map((row) => row.strategy),
        ...(view.actualTradeEvidence ?? []).map((row) => row.strategy),
        ...view.decisionBlockers.map((row) => row.strategy),
      ]),
    ].sort();
    const matches = (row: { symbol: string; strategy: string }) =>
      (!this.filters.symbol || row.symbol === this.filters.symbol) &&
      (!this.filters.strategy || row.strategy === this.filters.strategy);
    const hasData =
      view.profilesMonitored != null ||
      view.requests != null ||
      view.cohorts != null ||
      view.scoutEvidence != null ||
      view.actualTradeEvidence != null;
    const hasErrors = Boolean(
      view.coverageError || view.learningSummaryError || view.profileError,
    );
    return {
      view,
      tab: this.tab,
      filters: this.filters,
      symbols,
      strategies,
      visibleCohorts: view.cohorts?.filter(matches) ?? null,
      visibleRequests: view.requests?.filter(matches) ?? null,
      visibleScoutEvidence: this.filters.symbol
        ? []
        : (view.scoutEvidence?.filter(
            (row) =>
              !this.filters.strategy || row.strategy === this.filters.strategy,
          ) ?? null),
      visibleActualTradeEvidence: this.filters.symbol
        ? []
        : (view.actualTradeEvidence?.filter(
            (row) =>
              !this.filters.strategy || row.strategy === this.filters.strategy,
          ) ?? null),
      visibleDecisionBlockers: view.decisionBlockers.filter((row) =>
        matches(row),
      ),
      refreshing: this.refreshing,
      refreshError: this.refreshError,
      lastRefreshAttemptAt: this.lastRefreshAttemptAt,
      dataStatus: hasData
        ? hasErrors
          ? 'partial'
          : 'available'
        : hasErrors
          ? 'unavailable'
          : this.running
            ? 'loading'
            : 'inactive',
      stale: Boolean(
        view.cohorts?.some((cohort) => cohort.freshness === 'stale') ||
          this.sourceState.decisionCoverage.stalled,
      ),
    };
  }
}
