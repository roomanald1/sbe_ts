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
  lastDecision: string;
  lastDecisionAt: number;
  orders: AutoTraderOrder[];
  pnlBreakdown: AutoTraderPnlBreakdown[];
  realizedPnl: number;
  unrealizedPnl?: number | null;
  exitLearningSamples: number;
  learningSamples: number;
  learningWinRate?: number;
  preferredOrderType?: string;
  error?: string;
  lastResult?: string;
};
export type AutoTraderPnlBreakdown = { symbol: string; direction: string; purpose: string; realized_pnl: number; outcomes: number };
type TraderStatus = { enabled: boolean; max_order_notional: number; daily_notional_limit: number; daily_realized_loss_limit: number; exploration_percent: number; max_open_positions: number; stop_loss_percent: number; take_profit_percent: number; used_notional_today: number; last_decision?: string; last_decision_at?: number };

export class AutoTraderModel {
  readonly state$ = new BehaviorSubject<AutoTraderState>({ enabled: false, loading: true, busy: false, maxOrderNotional: '100', dailyNotionalLimit: '1000', usedNotionalToday: 0, lastDecision: '', lastDecisionAt: 0, orders: [], pnlBreakdown: [], realizedPnl: 0, unrealizedPnl: 0, exitLearningSamples: 0, dailyRealizedLossLimit: '10', explorationPercent: '10', maxOpenPositions: '20', stopLossPercent: 0.5, takeProfitPercent: 0.8, learningSamples: 0 });
  private timer?: ReturnType<typeof setInterval>;
  private limitsDirty = false;
  start() { if (!this.timer) { void this.refresh(); this.timer = setInterval(() => void this.refresh(), 5000); } }
  stop() { if (this.timer) clearInterval(this.timer); this.timer = undefined; }
  setLimits(maxOrderNotional: string, dailyNotionalLimit: string, dailyRealizedLossLimit: string, explorationPercent: string, maxOpenPositions: string) { this.limitsDirty = true; this.state$.next({ ...this.state$.value, maxOrderNotional, dailyNotionalLimit, dailyRealizedLossLimit, explorationPercent, maxOpenPositions }); }

  async refresh() {
    try {
      const [statusResponse, activityResponse] = await Promise.all([fetch('/positions/auto-trader'), fetch('/positions/auto-trader/activity')]);
      if (!statusResponse.ok) throw await this.responseError('status', statusResponse);
      if (!activityResponse.ok) throw await this.responseError('activity', activityResponse);
      const status = await statusResponse.json() as TraderStatus;
      const activity = await activityResponse.json() as { orders: AutoTraderOrder[]; pnl_breakdown: AutoTraderPnlBreakdown[]; realized_pnl: number; unrealized_pnl?: number | null; exit_learning_samples: number; learning_samples: number; learning_win_rate?: number; preferred_order_type?: string };
      this.state$.next({ ...this.state$.value, enabled: status.enabled, maxOrderNotional: this.limitsDirty ? this.state$.value.maxOrderNotional : String(status.max_order_notional), dailyNotionalLimit: this.limitsDirty ? this.state$.value.dailyNotionalLimit : String(status.daily_notional_limit), usedNotionalToday: status.used_notional_today, dailyRealizedLossLimit: this.limitsDirty ? this.state$.value.dailyRealizedLossLimit : String(status.daily_realized_loss_limit), explorationPercent: this.limitsDirty ? this.state$.value.explorationPercent : String(status.exploration_percent), maxOpenPositions: this.limitsDirty ? this.state$.value.maxOpenPositions : String(status.max_open_positions), stopLossPercent: status.stop_loss_percent, takeProfitPercent: status.take_profit_percent, lastDecision: status.last_decision ?? '', lastDecisionAt: status.last_decision_at ?? 0, orders: activity.orders, pnlBreakdown: activity.pnl_breakdown, realizedPnl: activity.realized_pnl, unrealizedPnl: activity.unrealized_pnl, exitLearningSamples: activity.exit_learning_samples, learningSamples: activity.learning_samples, learningWinRate: activity.learning_win_rate, preferredOrderType: activity.preferred_order_type, loading: false, error: undefined });
    } catch (error) {
      console.error('Auto Trader status/activity refresh failed:', error);
      this.state$.next({ ...this.state$.value, loading: false, error: error instanceof Error ? error.message : 'Could not load Auto Trader status and activity.' });
    }
  }

  private async responseError(endpoint: 'status' | 'activity', response: Response) {
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
      const response = await fetch('/positions/auto-trader/settings', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ max_order_notional: Number(current.maxOrderNotional), daily_notional_limit: Number(current.dailyNotionalLimit), daily_realized_loss_limit: Number(current.dailyRealizedLossLimit), exploration_percent: Number(current.explorationPercent), max_open_positions: Number(current.maxOpenPositions) }) });
      if (!response.ok) throw new Error(await response.text());
      this.limitsDirty = false;
      await this.refresh();
      this.state$.next({ ...this.state$.value, busy: false });
    } catch (error) { this.state$.next({ ...this.state$.value, busy: false, error: error instanceof Error ? error.message : 'Could not save limits.' }); }
  }

  async runOnce() {
    this.state$.next({ ...this.state$.value, busy: true, error: undefined, lastResult: undefined });
    try {
      const response = await fetch('/positions/auto-trader/run-once', { method: 'POST' });
      const result = await response.json() as { status?: string; reason?: string; symbol?: string; side?: string; quantity?: number; order_type?: string };
      if (!response.ok) throw new Error(result.reason ?? 'Auto Trader cycle failed');
      const message = result.status === 'order_submitted' || result.status === 'hedge_submitted' ? 'Submitted ' + result.order_type + ' ' + result.side?.toUpperCase() + ' ' + result.symbol : result.reason ?? 'No eligible signal.';
      await this.refresh();
      this.state$.next({ ...this.state$.value, busy: false, lastResult: message });
    } catch (error) { this.state$.next({ ...this.state$.value, busy: false, error: error instanceof Error ? error.message : 'Auto Trader cycle failed.' }); }
  }
}
