import { BehaviorSubject } from 'rxjs';
import type { SignalSnapshot } from './SignalsModel';

export type SignalPerformanceRow = {
  symbol: string;
  horizon_seconds: number;
  sample_count: number;
  directional_sample_count: number;
  neutral_sample_count: number;
  bullish_match_rate?: number | null;
  bearish_match_rate?: number | null;
  direction_match_rate?: number | null;
  average_return_percent: number;
  net_sample_count: number;
  average_net_return_percent?: number | null;
};
export type SignalWalkForwardRow = {
  symbol: string;
  horizon_seconds: number;
  training_samples: number;
  evaluated_signals: number;
  selected_signals: number;
  selected_strength_threshold?: number | null;
  direction_match_rate?: number | null;
  average_net_return_percent?: number | null;
  positive_net_rate?: number | null;
  baseline_direction_match_rate?: number | null;
  baseline_average_net_return_percent?: number | null;
  five_minute_trend_sample_count: number;
  five_minute_trend_match_rate?: number | null;
  five_minute_trend_average_net_return_percent?: number | null;
};
export type SignalInfoState = {
  best: SignalSnapshot[];
  history: SignalSnapshot[];
  performance: SignalPerformanceRow[];
  walkForward: SignalWalkForwardRow[];
  bestError?: string;
  historyError?: string;
  performanceError?: string;
  walkForwardError?: string;
};

export class SignalInfoModel {
  readonly state$ = new BehaviorSubject<SignalInfoState>({ best: [], history: [], performance: [], walkForward: [] });
  private timer?: ReturnType<typeof setInterval>;
  async start() {
    if (this.timer) return;
    await this.refresh();
    this.timer = setInterval(() => void this.refresh(), 15_000);
  }
  stop() {
    if (this.timer) clearInterval(this.timer);
    this.timer = undefined;
  }
  private async refresh() {
    const [best, history, performance, walkForward] = await Promise.allSettled([
      fetch('/signals/best').then(async (response) => {
        if (!response.ok) throw new Error('Could not load best signals');
        return await response.json() as SignalSnapshot[];
      }),
      fetch('/signals/history').then(async (response) => {
        if (!response.ok) throw new Error('Could not load signal history');
        return await response.json() as SignalSnapshot[];
      }),
      fetch('/signals/performance').then(async (response) => {
        if (!response.ok) throw new Error('Could not load signal performance');
        return await response.json() as SignalPerformanceRow[];
      }),
      fetch('/signals/walk-forward').then(async (response) => {
        if (!response.ok) throw new Error('Could not load walk-forward evaluation');
        return await response.json() as SignalWalkForwardRow[];
      }),
    ]);
    const previous = this.state$.value;
    this.state$.next({
      best: best.status === 'fulfilled' ? best.value : previous.best,
      history: history.status === 'fulfilled' ? history.value : previous.history,
      performance: performance.status === 'fulfilled' ? performance.value : previous.performance,
      walkForward: walkForward.status === 'fulfilled' ? walkForward.value : previous.walkForward,
      bestError: best.status === 'rejected' ? 'Best signals are unavailable.' : undefined,
      historyError: history.status === 'rejected' ? 'Signal history is unavailable.' : undefined,
      performanceError: performance.status === 'rejected' ? 'Signal performance is unavailable.' : undefined,
      walkForwardError: walkForward.status === 'rejected' ? 'Walk-forward evaluation is unavailable.' : undefined,
    });
  }
}
