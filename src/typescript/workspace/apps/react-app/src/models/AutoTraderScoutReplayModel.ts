import { BehaviorSubject } from 'rxjs';
import { learningRpc } from '../learning_rsocket';

export type ScoutReplayOutcome = {
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
  malformedRecords: number;
  firstEvaluatedTimestampMs: number | null;
  lastEvaluatedTimestampMs: number | null;
  runtimeHydrationLimit: number;
};

type ScoutReplayResponse = {
  datasetVersion: number;
  source: string;
  ordering: string;
  coverage: ScoutReplayCoverage;
  outcomes: ScoutReplayOutcome[];
};

type ScoutReplayState = ScoutReplayResponse & {
  loading: boolean;
  error?: string;
  updatedAt: number;
};

const emptyResponse: ScoutReplayResponse = {
  datasetVersion: 1,
  source: 'scout_records.outcomes',
  ordering: 'evaluated_timestamp_ms_asc_then_record_key_asc',
  coverage: {
    status: 'incomplete',
    complete: false,
    totalDurableRecords: 0,
    returnedRecords: 0,
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

  private async refreshAndSchedule() {
    await this.refresh();
    if (this.consumers === 0) return;
    this.timer = setTimeout(() => void this.refreshAndSchedule(), 60_000);
  }

  private async refresh() {
    this.refreshing = true;
    this.state$.next({ ...this.state$.value, loading: true, error: undefined });
    try {
      const response = await learningRpc('GET', '/replay/outcomes');
      if (!response.ok)
        throw new Error(`Scout replay request failed (${response.status}).`);
      const data = (await response.json()) as ScoutReplayResponse;
      if (!Array.isArray(data.outcomes) || !data.coverage)
        throw new Error('Scout replay response is missing coverage data.');
      this.state$.next({
        ...data,
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
