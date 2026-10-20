import { BehaviorSubject } from 'rxjs';

export type AutoTraderOrder = {
  timestamp: number;
  triggered_at?: number;
  symbol: string;
  side: string;
  order_type: string;
  purpose?: string;
  quantity: number;
  filled_quantity: number;
  average_price?: number;
  status: string;
  realized_pnl: number;
};
export type AutoTraderState = {
  enabled: boolean;
  loading: boolean;
  busy: boolean;
  maxOrderNotional: string;
  dailyNotionalLimit: string;
  usedNotionalToday: number;
  dailyRealizedLossLimit: string;
  explorationPercent: string;
  maxOpenPositions: string;
  stopLossPercent: number;
  takeProfitPercent: number;
  vwapDurationMinutes: string; vwapParticipationPercent: string; limitOffsetBps: string; triggerOffsetBps: string; trailingDistancePercent: string;
  lastDecision: string;
  lastDecisionAt: number;
  orders: AutoTraderOrder[];
  pnlBreakdown: AutoTraderPnlBreakdown[];
  executionQuality: AutoTraderExecutionQuality;
  decisionQuality: AutoTraderDecisionQuality;
  realizedPnl: number;
  unrealizedPnl?: number | null;
  exitLearningSamples: number;
  closedTradeAverageReturnPercent?: number;
  closedTradeWins: number;
  closedTradeLosses: number;
  closedTradeFlat: number;
  learningSamples: number;
  learningWinRate?: number;
  preferredOrderType?: string;
  parameterProfileSamples: number[];
  error?: string;
};
export type AutoTraderPnlBreakdown = { symbol: string; direction: string; purpose: string; realized_pnl: number; outcomes: number };
export type AutoTraderDecisionQuality = {
  decision_samples: number;
  filled_entry_samples: number;
  unresolved_trade_samples: number;
  unpriced_closed_trade_samples: number;
  completed_trade_samples: number;
  history_truncated: boolean;
  cohorts: Array<{ dimension: string; value: string; completed_trade_samples: number; average_net_return_percent: number; win_rate: number; total_net_pnl: number; preliminary: boolean }>;
};
export type AutoTraderExecutionQuality = {
  order_samples: number;
  filled_order_samples: number;
  partial_fill_samples: number;
  unfilled_order_samples: number;
  measured_fill_samples: number;
  reference_price_coverage_percent?: number;
  average_fill_rate?: number;
  average_implementation_shortfall_bps?: number;
  average_fee_bps?: number;
  average_all_in_cost_bps?: number;
  average_submit_response_ms?: number;
  response_latency_samples: number;
  history_truncated: boolean;
  last_updated_at?: number;
};
const emptyExecutionQuality: AutoTraderExecutionQuality = { order_samples: 0, filled_order_samples: 0, partial_fill_samples: 0, unfilled_order_samples: 0, measured_fill_samples: 0, response_latency_samples: 0, history_truncated: false };
const emptyDecisionQuality: AutoTraderDecisionQuality = { decision_samples: 0, filled_entry_samples: 0, unresolved_trade_samples: 0, unpriced_closed_trade_samples: 0, completed_trade_samples: 0, history_truncated: false, cohorts: [] };

type TraderStatus = { enabled: boolean; max_order_notional: number; daily_notional_limit: number; daily_realized_loss_limit: number; exploration_percent: number; max_open_positions: number; stop_loss_percent: number; take_profit_percent: number; vwap_duration_minutes: number; vwap_participation_percent: number; limit_offset_bps: number; trigger_offset_bps: number; trailing_distance_percent: number; used_notional_today: number; last_decision?: string; last_decision_at?: number };

export class AutoTraderModel {
  readonly state$ = new BehaviorSubject<AutoTraderState>({ enabled: false, loading: true, busy: false, maxOrderNotional: '100', dailyNotionalLimit: '1000', usedNotionalToday: 0, lastDecision: '', lastDecisionAt: 0, orders: [], pnlBreakdown: [], executionQuality: emptyExecutionQuality, decisionQuality: emptyDecisionQuality, realizedPnl: 0, unrealizedPnl: 0, exitLearningSamples: 0, closedTradeWins: 0, closedTradeLosses: 0, closedTradeFlat: 0, dailyRealizedLossLimit: '10', explorationPercent: '10', maxOpenPositions: '20', stopLossPercent: 0.5, takeProfitPercent: 0.8, vwapDurationMinutes: '5', vwapParticipationPercent: '20', limitOffsetBps: '0', triggerOffsetBps: '20', trailingDistancePercent: '0.10', learningSamples: 0, parameterProfileSamples: [0, 0, 0] });
  private timer?: ReturnType<typeof setInterval>;
  private limitsDirty = false;
  private executionParametersDirty = false;
  start() { if (!this.timer) { void this.refresh(); this.timer = setInterval(() => void this.refresh(), 5000); } }
  stop() { if (this.timer) clearInterval(this.timer); this.timer = undefined; }
  setLimits(maxOrderNotional: string, dailyNotionalLimit: string, dailyRealizedLossLimit: string, explorationPercent: string, maxOpenPositions: string) { this.limitsDirty = true; this.state$.next({ ...this.state$.value, maxOrderNotional, dailyNotionalLimit, dailyRealizedLossLimit, explorationPercent, maxOpenPositions }); }

  setExecutionParameters(values: Partial<Pick<AutoTraderState, 'vwapDurationMinutes' | 'vwapParticipationPercent' | 'limitOffsetBps' | 'triggerOffsetBps' | 'trailingDistancePercent'>>) { this.executionParametersDirty = true; this.state$.next({ ...this.state$.value, ...values }); }

  async refresh() {
    try {
      const [statusResponse, activityResponse, executionQualityResponse, decisionQualityResponse] = await Promise.all([fetch('/positions/auto-trader'), fetch('/positions/auto-trader/activity'), fetch('/signals/auto-trader-execution-quality'), fetch('/signals/auto-trader-decision-quality')]);
      if (!statusResponse.ok) throw await this.responseError('status', statusResponse);
      if (!activityResponse.ok) throw await this.responseError('activity', activityResponse);
      if (!executionQualityResponse.ok) throw await this.responseError('execution quality', executionQualityResponse);
      if (!decisionQualityResponse.ok) throw await this.responseError('decision quality', decisionQualityResponse);
      const status = await statusResponse.json() as TraderStatus;
      const activity = await activityResponse.json() as { orders: AutoTraderOrder[]; pnl_breakdown: AutoTraderPnlBreakdown[]; realized_pnl: number; unrealized_pnl?: number | null; exit_learning_samples: number; closed_trade_average_return_percent?: number; closed_trade_wins?: number; closed_trade_losses?: number; closed_trade_flat?: number; learning_samples: number; learning_win_rate?: number; preferred_order_type?: string; parameter_profile_samples?: number[] };
      const executionQuality = await executionQualityResponse.json() as AutoTraderExecutionQuality;
      const decisionQuality = await decisionQualityResponse.json() as AutoTraderDecisionQuality;
      this.state$.next({ ...this.state$.value, enabled: status.enabled, maxOrderNotional: this.limitsDirty ? this.state$.value.maxOrderNotional : String(status.max_order_notional), dailyNotionalLimit: this.limitsDirty ? this.state$.value.dailyNotionalLimit : String(status.daily_notional_limit), usedNotionalToday: status.used_notional_today, dailyRealizedLossLimit: this.limitsDirty ? this.state$.value.dailyRealizedLossLimit : String(status.daily_realized_loss_limit), explorationPercent: this.limitsDirty ? this.state$.value.explorationPercent : String(status.exploration_percent), maxOpenPositions: this.limitsDirty ? this.state$.value.maxOpenPositions : String(status.max_open_positions), stopLossPercent: status.stop_loss_percent, takeProfitPercent: status.take_profit_percent, vwapDurationMinutes: this.executionParametersDirty ? this.state$.value.vwapDurationMinutes : String(status.vwap_duration_minutes), vwapParticipationPercent: this.executionParametersDirty ? this.state$.value.vwapParticipationPercent : String(status.vwap_participation_percent), limitOffsetBps: this.executionParametersDirty ? this.state$.value.limitOffsetBps : String(status.limit_offset_bps), triggerOffsetBps: this.executionParametersDirty ? this.state$.value.triggerOffsetBps : String(status.trigger_offset_bps), trailingDistancePercent: this.executionParametersDirty ? this.state$.value.trailingDistancePercent : String(status.trailing_distance_percent), lastDecision: status.last_decision ?? '', lastDecisionAt: status.last_decision_at ?? 0, orders: activity.orders, pnlBreakdown: activity.pnl_breakdown, executionQuality, decisionQuality, realizedPnl: activity.realized_pnl, unrealizedPnl: activity.unrealized_pnl, exitLearningSamples: activity.exit_learning_samples, closedTradeAverageReturnPercent: activity.closed_trade_average_return_percent, closedTradeWins: activity.closed_trade_wins ?? 0, closedTradeLosses: activity.closed_trade_losses ?? 0, closedTradeFlat: activity.closed_trade_flat ?? 0, learningSamples: activity.learning_samples, learningWinRate: activity.learning_win_rate, preferredOrderType: activity.preferred_order_type, parameterProfileSamples: activity.parameter_profile_samples ?? [0, 0, 0], loading: false, error: undefined });
    } catch (error) {
      console.error('Auto Trader status/activity refresh failed:', error);
      this.state$.next({ ...this.state$.value, loading: false, error: error instanceof Error ? error.message : 'Could not load Auto Trader status and activity.' });
    }
  }

  private async responseError(endpoint: 'status' | 'activity' | 'execution quality' | 'decision quality', response: Response) {
    const detail = (await response.text()).trim();
    return new Error('Could not load Auto Trader ' + endpoint + ' (' + response.status + ')' + (detail ? ': ' + detail : '.'));
  }

  async setEnabled(enabled: boolean) {
    this.state$.next({ ...this.state$.value, busy: true, error: undefined });
    try {
      const response = await fetch('/positions/auto-trader', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ enabled }) });
      if (!response.ok) throw new Error();
      await this.refresh();
      this.state$.next({ ...this.state$.value, busy: false });
    } catch { this.state$.next({ ...this.state$.value, busy: false, error: 'Could not change Auto Trader state.' }); }
  }

  async saveLimits() {
    const current = this.state$.value;
    this.state$.next({ ...current, busy: true, error: undefined });
    try {
      const response = await fetch('/positions/auto-trader/settings', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ max_order_notional: Number(current.maxOrderNotional), daily_notional_limit: Number(current.dailyNotionalLimit), daily_realized_loss_limit: Number(current.dailyRealizedLossLimit), exploration_percent: Number(current.explorationPercent), max_open_positions: Number(current.maxOpenPositions), vwap_duration_minutes: Number(current.vwapDurationMinutes), vwap_participation_percent: Number(current.vwapParticipationPercent), limit_offset_bps: Number(current.limitOffsetBps), trigger_offset_bps: Number(current.triggerOffsetBps), trailing_distance_percent: Number(current.trailingDistancePercent) }) });
      if (!response.ok) throw new Error(await response.text());
      this.limitsDirty = false; this.executionParametersDirty = false;
      await this.refresh();
      this.state$.next({ ...this.state$.value, busy: false });
    } catch (error) { this.state$.next({ ...this.state$.value, busy: false, error: error instanceof Error ? error.message : 'Could not save limits.' }); }
  }

}
