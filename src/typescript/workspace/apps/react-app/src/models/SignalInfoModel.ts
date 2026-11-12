import { BehaviorSubject } from 'rxjs';
import type { SignalSnapshot } from './SignalsModel';
import {
  currentInsightsSnapshot,
  subscribeInsightsStream,
} from '../insights_stream';

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
export type MissedOpportunityBand = {
  gate: string;
  threshold: number;
  symbol?: string | null;
  direction?: string | null;
  strategy?: string | null;
  marketContext?: string | null;
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
  strategyExitSampleCount?: number;
  strategyExitAverageNetReturnPercent?: number | null;
  strategyExitPositiveNetRate?: number | null;
};
export type SkippedSignalAuditRow = {
  status: string;
  timestamp: number;
  symbol: string;
  direction: string;
  strategy?: string;
  score: number;
  strength: number;
  reason?: string;
  entry_gate?: {
    market_context?: string;
    observed?: { score?: number; strength?: number };
    thresholds?: {
      strength_floor?: number;
      exploration_score_absolute_floor?: number;
    };
    rejection_reasons?: string[];
  };
};
export type PostExitOpportunityRow = {
  timestamp: number;
  symbol: string;
  strategy?: string;
  side?: string;
  missedReturnPercent?: number;
  bestPostExitReturnPercent?: number;
  worstPostExitReturnPercent?: number;
  missedPnlQuote?: number;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function skippedSignalAuditRow(
  value: Record<string, unknown>,
): SkippedSignalAuditRow | undefined {
  const timestamp = value.timestamp;
  const symbol = value.symbol;
  const direction = value.direction;
  const score = value.score;
  const strength = value.strength;
  const status = value.status;
  if (
    typeof timestamp !== 'number' ||
    typeof symbol !== 'string' ||
    typeof direction !== 'string' ||
    typeof score !== 'number' ||
    typeof strength !== 'number' ||
    (status !== 'skipped' && status !== 'rejected')
  )
    return undefined;
  const gate = isRecord(value.entry_gate) ? value.entry_gate : undefined;
  const observed = gate && isRecord(gate.observed) ? gate.observed : undefined;
  const thresholds =
    gate && isRecord(gate.thresholds) ? gate.thresholds : undefined;
  return {
    status,
    timestamp,
    symbol,
    direction,
    strategy: typeof value.strategy === 'string' ? value.strategy : undefined,
    score,
    strength,
    reason: typeof value.reason === 'string' ? value.reason : undefined,
    entry_gate: gate
      ? {
          market_context:
            typeof gate.market_context === 'string'
              ? gate.market_context
              : undefined,
          observed: observed
            ? {
                score:
                  typeof observed.score === 'number'
                    ? observed.score
                    : undefined,
                strength:
                  typeof observed.strength === 'number'
                    ? observed.strength
                    : undefined,
              }
            : undefined,
          thresholds: thresholds
            ? {
                strength_floor:
                  typeof thresholds.strength_floor === 'number'
                    ? thresholds.strength_floor
                    : undefined,
                exploration_score_absolute_floor:
                  typeof thresholds.exploration_score_absolute_floor ===
                  'number'
                    ? thresholds.exploration_score_absolute_floor
                    : undefined,
              }
            : undefined,
          rejection_reasons: Array.isArray(gate.rejection_reasons)
            ? gate.rejection_reasons.filter(
                (reason): reason is string => typeof reason === 'string',
              )
            : undefined,
        }
      : undefined,
  };
}

function postExitOpportunityRow(
  value: Record<string, unknown>,
): PostExitOpportunityRow | undefined {
  const observation = isRecord(value.post_exit_observation)
    ? value.post_exit_observation
    : undefined;
  const timestamp = observation?.observed_at_ms;
  if (
    !observation ||
    typeof timestamp !== 'number' ||
    typeof value.symbol !== 'string'
  )
    return undefined;
  return {
    timestamp,
    symbol: value.symbol,
    strategy: typeof value.strategy === 'string' ? value.strategy : undefined,
    side: typeof observation.side === 'string' ? observation.side : undefined,
    missedReturnPercent:
      typeof observation.missed_return_percent === 'number'
        ? observation.missed_return_percent
        : undefined,
    bestPostExitReturnPercent:
      typeof observation.best_post_exit_return_percent === 'number'
        ? observation.best_post_exit_return_percent
        : undefined,
    worstPostExitReturnPercent:
      typeof observation.worst_post_exit_return_percent === 'number'
        ? observation.worst_post_exit_return_percent
        : undefined,
    missedPnlQuote:
      typeof observation.missed_pnl_quote === 'number'
        ? observation.missed_pnl_quote
        : undefined,
  };
}

export type SignalInfoState = {
  best: SignalSnapshot[];
  history: SignalSnapshot[];
  performance: SignalPerformanceRow[];
  missedOpportunityBands: MissedOpportunityBand[];
  skippedSignalAudit: SkippedSignalAuditRow[];
  postExitOpportunities: PostExitOpportunityRow[];
  missedOpportunityBandsError?: string;
  bestError?: string;
  historyError?: string;
  performanceError?: string;
};

export class SignalInfoModel {
  readonly state$ = new BehaviorSubject<SignalInfoState>({
    best: [],
    history: [],
    performance: [],
    missedOpportunityBands: [],
    skippedSignalAudit: [],
    postExitOpportunities: [],
  });
  private stopStream?: () => void;
  start() {
    if (this.stopStream) return;
    const apply = (
      snapshot: NonNullable<ReturnType<typeof currentInsightsSnapshot>>,
    ) => {
      const dashboard = snapshot.signal_dashboard as
        | {
            best?: SignalSnapshot[];
            history?: SignalSnapshot[];
            performance?: SignalPerformanceRow[];
          }
        | undefined;
      const autoDashboard = snapshot.auto_trader_dashboard as
        | { missedOpportunityBands?: MissedOpportunityBand[] }
        | undefined;
      const decisionAudit = snapshot.decision_audit as
        | { items?: SkippedSignalAuditRow[] }
        | undefined;
      const executionAudit = snapshot.execution_audit as
        | { items?: Record<string, unknown>[] }
        | undefined;
      this.state$.next({
        ...this.state$.value,
        ...dashboard,
        missedOpportunityBands:
          autoDashboard?.missedOpportunityBands ??
          this.state$.value.missedOpportunityBands,
        skippedSignalAudit:
          decisionAudit?.items
            ?.map(skippedSignalAuditRow)
            .filter(
              (item): item is SkippedSignalAuditRow => item !== undefined,
            ) ?? this.state$.value.skippedSignalAudit,
        postExitOpportunities:
          executionAudit?.items
            ?.map(postExitOpportunityRow)
            .filter(
              (item): item is PostExitOpportunityRow => item !== undefined,
            )
            .slice(-100)
            .reverse() ?? this.state$.value.postExitOpportunities,
        bestError: undefined,
        historyError: undefined,
        performanceError: undefined,
        missedOpportunityBandsError: undefined,
      });
    };
    const current = currentInsightsSnapshot();
    if (current) apply(current);
    this.stopStream = subscribeInsightsStream(apply);
  }
  stop() {
    this.stopStream?.();
    this.stopStream = undefined;
  }
}
