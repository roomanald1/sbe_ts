import { BehaviorSubject } from 'rxjs';
import { learningRpc } from '../learning_rsocket';

export type LearningScoutActivity = {
  assignment_id: string;
  request_id: string;
  evaluated_at_ms: number;
  rank: number;
  symbol: string;
  strategy: string;
  decision: string;
  mode: string;
  candidate: Record<string, unknown>;
  walk_forward_evidence?: {
    sample_count: number;
    positive_net_rate?: number;
    average_net_return_percent?: number;
    expectancy_ci_low_percent?: number;
  } | null;
  evidence_sample_count: number;
  rank_basis: string;
};

export class LearningScoutActivityModel {
  readonly state$ = new BehaviorSubject<{
    rows: LearningScoutActivity[];
    loading: boolean;
    error?: string;
    updatedAt?: number;
  }>({ rows: [], loading: false });

  private timer?: ReturnType<typeof setInterval>;
  private stopped = true;

  start() {
    this.stopped = false;
    void this.refresh();
    this.timer = setInterval(() => void this.refresh(), 30_000);
  }

  stop() {
    this.stopped = true;
    if (this.timer) clearInterval(this.timer);
    this.timer = undefined;
  }

  async refresh() {
    if (this.stopped) return;
    this.state$.next({ ...this.state$.value, loading: true, error: undefined });
    try {
      const response = await learningRpc('GET', '/scout-activity');
      if (!response.ok) throw new Error(`Learning returned ${response.status}`);
      const payload = (await response.json()) as {
        items?: LearningScoutActivity[];
      };
      if (!this.stopped)
        this.state$.next({
          rows: payload.items ?? [],
          loading: false,
          updatedAt: Date.now(),
        });
    } catch (error) {
      if (!this.stopped)
        this.state$.next({
          ...this.state$.value,
          loading: false,
          error: error instanceof Error ? error.message : String(error),
        });
    }
  }
}
