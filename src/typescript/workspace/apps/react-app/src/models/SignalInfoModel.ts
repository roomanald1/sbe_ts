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
  logistic_direction_match_rate?: number | null;
  logistic_average_net_return_percent?: number | null;
  logistic_positive_net_rate?: number | null;
  logistic_selected_signals: number;
  signal_long: SignalWalkForwardSideMetrics;
  signal_short: SignalWalkForwardSideMetrics;
  logistic_long: SignalWalkForwardSideMetrics;
  logistic_short: SignalWalkForwardSideMetrics;
};
export type SignalWalkForwardSideMetrics = {
  sample_count: number;
  direction_match_rate?: number | null;
  average_net_return_percent?: number | null;
  positive_net_rate?: number | null;
  positive_net_rate_lower_95?: number | null;
};
export type MissedOpportunityBand = {
  gate: string;
  threshold: number;
  horizonSeconds: number;
  sampleCount: number;
  averageNetReturnPercent?: number | null;
  positiveNetRate?: number | null;
  timeToProfitSampleCount: number;
  timeToProfitRate?: number | null;
  averageTimeToProfitSeconds?: number | null;
  medianTimeToProfitSeconds?: number | null;
  stopTargetSampleCount: number;
  stopTargetAverageNetReturnPercent?: number | null;
  stopTargetPositiveNetRate?: number | null;
};
export type SignalInfoState = {
  best: SignalSnapshot[];
  history: SignalSnapshot[];
  performance: SignalPerformanceRow[];
  walkForward: SignalWalkForwardRow[];
  missedOpportunityBands: MissedOpportunityBand[];
  missedOpportunityBandsError?: string;
  bestError?: string;
  historyError?: string;
  performanceError?: string;
  walkForwardError?: string;
};

export class SignalInfoModel {
  readonly state$ = new BehaviorSubject<SignalInfoState>({ best: [], history: [], performance: [], walkForward: [], missedOpportunityBands: [] });
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
    try {
      const bandsResponse = await fetch('/signals/auto-trader-dashboard');
      if (!bandsResponse.ok) throw new Error('Could not load missed opportunity bands');
      const bandsDashboard = await bandsResponse.json() as { missedOpportunityBands?: MissedOpportunityBand[] };
      const previousBands = this.state$.value;
      this.state$.next({ ...previousBands, missedOpportunityBands: bandsDashboard.missedOpportunityBands ?? [], missedOpportunityBandsError: undefined });
    } catch {
      this.state$.next({ ...this.state$.value, missedOpportunityBandsError: 'Missed opportunity bands are unavailable.' });
    }
    try {
      const response = await fetch('/signals/dashboard');
      if (!response.ok) throw new Error('Could not load signal dashboard');
      const dashboard = await response.json() as {
        best: SignalSnapshot[];
        history: SignalSnapshot[];
        performance: SignalPerformanceRow[];
        walkForward: SignalWalkForwardRow[];
      };
      this.state$.next({ ...this.state$.value, ...dashboard });
    } catch {
      const previous = this.state$.value;
      this.state$.next({
        ...previous,
        bestError: 'Best signals are unavailable.',
        historyError: 'Signal history is unavailable.',
        performanceError: 'Signal performance is unavailable.',
        walkForwardError: 'Walk-forward evaluation is unavailable.',
      });
    }
  }
}
