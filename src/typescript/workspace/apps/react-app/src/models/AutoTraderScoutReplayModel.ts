import { BehaviorSubject } from 'rxjs';
import { scoutWorkerRpc } from '../learning_rsocket';

export type ScoutReplayOutcome = {
  outcome_id: string;
  profile_id?: string | null;
  profile_symbol?: string;
  profile_symbol_source?: string;
  evidence_group_id?: string;
  evidence_group_source?: string;
  notional_usd?: number | null;
  portfolio_weight?: number | null;
  cost_breakdown?: {
    takerFeeRate?: number | null;
    grossReturnPercent?: number | null;
    markoutNetReturnPercent?: number | null;
    outcomeNetReturnPercent?: number | null;
    executionCostPercent?: number | null;
  } | null;
  eligibility?: {
    status: string;
    eligibleBeforeOrigin: boolean | null;
    profileIdentity: string;
    marketSymbol: string;
    profileSymbol: string;
    regime: string;
    cost: string;
    costBreakdown: string;
    evidenceGroup: string;
  };
  decision_id: string;
  learning_context: string;
  simulated_net_pnl_usd: number;
  net_return_percent: number;
  signal_timestamp_ms: number;
  evaluated_timestamp_ms: number;
  market_regime?: string | null;
  taker_fee_rate: number;
  symbol: string;
  strategy: string;
  strategy_parameters: Record<string, number>;
  exit_scenario: string;
  sample_group_id: string;
};

export type ScoutReplayCoverage = {
  status: 'complete' | 'incomplete';
  complete: boolean;
  totalDurableRecords: number;
  returnedRecords: number;
  hasMore: boolean;
  malformedRecords: number;
  firstEvaluatedTimestampMs: number | null;
  lastEvaluatedTimestampMs: number | null;
  runtimeHydrationLimit: number;
  originAtMs?: number | null;
  completedBeforeOriginRecords?: number | null;
  atOrAfterOriginRecords?: number | null;
  missingRegimeRecords?: number;
  feeRateAndNetReturnReportedRecords?: number;
  explicitCostBreakdownRecords?: number;
  explicitCostBreakdown?: boolean;
  openScoutInstances?: number;
  expiredInstancesWithoutOutcome?: number;
};

type ScoutReplayResponse = {
  datasetVersion: number;
  source: string;
  ordering: string;
  coverage: ScoutReplayCoverage;
  outcomes: ScoutReplayOutcome[];
  workerErrors?: Array<{ strategy: string; status: number; error: string }>;
};

type ScoutReplayState = ScoutReplayResponse & {
  loading: boolean;
  error?: string;
  updatedAt: number;
};

const emptyResponse: ScoutReplayResponse = {
  datasetVersion: 2,
  source: 'scout_records.outcomes',
  ordering: 'evaluated_timestamp_ms_asc_then_strategy_then_decision_id',
  coverage: {
    status: 'incomplete',
    complete: false,
    totalDurableRecords: 0,
    returnedRecords: 0,
    hasMore: false,
    malformedRecords: 0,
    firstEvaluatedTimestampMs: null,
    lastEvaluatedTimestampMs: null,
    runtimeHydrationLimit: 4096,
  },
  outcomes: [],
};

export class AutoTraderScoutReplayModel {
  readonly state$ = new BehaviorSubject<ScoutReplayState>({
    ...emptyResponse,
    loading: true,
    updatedAt: 0,
  });
  private timer?: ReturnType<typeof setTimeout>;
  private consumers = 0;
  private refreshing = false;
  private page = 0;

  start() {
    this.consumers += 1;
    if (this.consumers === 1) void this.refreshAndSchedule();
  }

  stop() {
    this.consumers = Math.max(0, this.consumers - 1);
    if (this.consumers > 0) return;
    if (this.timer) clearTimeout(this.timer);
    this.timer = undefined;
  }

  refreshNow() {
    if (this.refreshing) return;
    if (this.timer) clearTimeout(this.timer);
    this.timer = undefined;
    void this.refreshAndSchedule();
  }

  loadMore() {
    if (this.refreshing || !this.state$.value.coverage.hasMore) return;
    void this.refresh(false);
  }

  private async refreshAndSchedule() {
    await this.refresh(true);
    if (this.consumers === 0) return;
    this.timer = setTimeout(() => void this.refreshAndSchedule(), 60_000);
  }

  private async refresh(reset = true) {
    this.refreshing = true;
    this.state$.next({ ...this.state$.value, loading: true, error: undefined });
    const page = reset ? 0 : this.page + 1;
    try {
      const response = await scoutWorkerRpc(
        'GET',
        `/replay/outcomes?page=${page}&page_size=25`,
      );
      if (!response.ok)
        throw new Error(`Scout replay request failed (${response.status}).`);
      const data = (await response.json()) as ScoutReplayResponse;
      if (!Array.isArray(data.outcomes) || !data.coverage)
        throw new Error('Scout replay response is missing coverage data.');
      const previous = this.state$.value;
      const outcomes = reset
        ? data.outcomes
        : [...previous.outcomes, ...data.outcomes].sort(
            (left, right) =>
              left.evaluated_timestamp_ms - right.evaluated_timestamp_ms ||
              left.strategy.localeCompare(right.strategy) ||
              left.decision_id.localeCompare(right.decision_id),
          );
      const returnedRecords = reset
        ? data.coverage.returnedRecords
        : previous.coverage.returnedRecords + data.coverage.returnedRecords;
      const malformedRecords = reset
        ? data.coverage.malformedRecords
        : previous.coverage.malformedRecords + data.coverage.malformedRecords;
      const hasMore = data.coverage.hasMore;
      const complete =
        !hasMore &&
        returnedRecords + malformedRecords >=
          data.coverage.totalDurableRecords &&
        !data.workerErrors?.length;
      this.page = page;
      this.state$.next({
        ...data,
        outcomes,
        coverage: {
          ...data.coverage,
          returnedRecords,
          malformedRecords,
          hasMore,
          complete,
          status: complete ? 'complete' : 'incomplete',
        },
        loading: false,
        updatedAt: Date.now(),
      });
    } catch (error) {
      this.state$.next({
        ...this.state$.value,
        loading: false,
        error:
          error instanceof Error
            ? error.message
            : 'Could not load Scout replay history.',
      });
    } finally {
      this.refreshing = false;
    }
  }
}

export const autoTraderScoutReplayModel = new AutoTraderScoutReplayModel();
