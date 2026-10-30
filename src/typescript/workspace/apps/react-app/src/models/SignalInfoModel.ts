import { BehaviorSubject } from 'rxjs';
import type { SignalSnapshot } from './SignalsModel';
import { currentInsightsSnapshot, subscribeInsightsStream } from '../insights_stream';

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
  private stopStream?: () => void;
  start() {
    if (this.stopStream) return;
    const apply = (snapshot: NonNullable<ReturnType<typeof currentInsightsSnapshot>>) => {
      const dashboard = snapshot.signal_dashboard as { best?: SignalSnapshot[]; history?: SignalSnapshot[]; performance?: SignalPerformanceRow[]; walkForward?: SignalWalkForwardRow[] } | undefined;
      const autoDashboard = snapshot.auto_trader_dashboard as { missedOpportunityBands?: MissedOpportunityBand[] } | undefined;
      this.state$.next({ ...this.state$.value, ...dashboard,
        missedOpportunityBands: autoDashboard?.missedOpportunityBands ?? this.state$.value.missedOpportunityBands,
        bestError: undefined, historyError: undefined, performanceError: undefined, walkForwardError: undefined, missedOpportunityBandsError: undefined });
    };
    const current = currentInsightsSnapshot();
    if (current) apply(current);
    this.stopStream = subscribeInsightsStream(apply);
  }
  stop() { this.stopStream?.(); this.stopStream = undefined; }
}
