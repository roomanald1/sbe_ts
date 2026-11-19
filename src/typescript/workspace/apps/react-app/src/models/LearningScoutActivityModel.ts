import { BehaviorSubject } from 'rxjs';
import { learningRpc } from '../learning_rsocket';
import { autoTraderRpc } from '../auto_trader_rsocket';
import type { AutoTraderStrategyProfile } from './AutoTraderModel';

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
    profiles: AutoTraderStrategyProfile[];
    loading: boolean;
    error?: string;
    profilesError?: string;
    updatedAt?: number;
  }>({ rows: [], profiles: [], loading: false });

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
    const [activityResult, profilesResult] = await Promise.allSettled([
      learningRpc('GET', '/scout-activity'),
      autoTraderRpc('GET', '/positions/auto-trader/parameters'),
    ]);
    if (this.stopped) return;

    const current = this.state$.value;
    let rows = current.rows;
    let error: string | undefined;
    if (activityResult.status === 'rejected') {
      error =
        activityResult.reason instanceof Error
          ? activityResult.reason.message
          : String(activityResult.reason);
    } else if (!activityResult.value.ok) {
      error = `Learning returned ${activityResult.value.status}`;
    } else {
      const payload = (await activityResult.value.json()) as {
        items?: LearningScoutActivity[];
      };
      rows = payload.items ?? [];
    }

    let profiles = current.profiles;
    let profilesError: string | undefined;
    if (profilesResult.status === 'rejected') {
      profilesError =
        profilesResult.reason instanceof Error
          ? profilesResult.reason.message
          : String(profilesResult.reason);
    } else if (!profilesResult.value.ok) {
      profilesError = `Auto Trader returned ${profilesResult.value.status}`;
    } else {
      const payload = (await profilesResult.value.json()) as {
        strategy_profiles?: AutoTraderStrategyProfile[];
      };
      profiles = payload.strategy_profiles ?? [];
    }
    this.state$.next({
      rows,
      profiles,
      loading: false,
      error,
      profilesError,
      updatedAt: Date.now(),
    });
  }
}
