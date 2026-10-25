import { BehaviorSubject } from 'rxjs';

export type AutoTraderOrder = {
  timestamp: number;
  triggered_at?: number;
  symbol: string;
  side: string;
  order_type: string;
  purpose?: string;
  strategy?: string;
  quantity: number;
  filled_quantity: number;
  submitted_limit_price?: number;
  target_price?: number;
  average_price?: number;
  status: string;
  realized_pnl: number;
  realized_pnl_usd?: number | null;
};
export type AutoTraderDecisionParameter = { section: string; name: string; value: number; unit: string };
export type AutoTraderContextParameters = { context: string; learning_samples: number; positive_rate?: number; strength_floor: number; exploration_score_floor: number; exploration_allowed: boolean };
export type AutoTraderLatestExecutionParameters = { timestamp: number; symbol: string; context: string; baseline_order_type: string; selected_order_type: string; profile: number; vwap_duration_minutes: number; vwap_participation_percent: number; schedule_depth_threshold: number; proposed_depth_ratio: number; schedule_threshold_samples: number; limit_offset_bps: number; trigger_offset_bps: number; trailing_distance_percent: number };
export type AutoTraderDecisionParameters = { refreshed_at: number; thresholds: AutoTraderDecisionParameter[]; contexts: AutoTraderContextParameters[]; active_exit_policy: { stop_loss_percent: number; take_profit_percent: number }; latest_execution?: AutoTraderLatestExecutionParameters };
export type ResearchStrategy = 'donchian-55-btc200' | 'donchian-20-sma200' | 'trend-sleeves-btc200' | 'trend-50-200' | 'relative-strength-90-sma200' | 'rsi2-oversold-rebound';
export type ResearchStrategyDiagnostic = {
  strategy: ResearchStrategy; as_of_ms: number; btc_close: number; btc_sma200: number;
  btc_distance_percent: number; btc_gate_required: boolean; btc_gate_passed: boolean;
  symbols: Array<{ symbol: string; close: number; eligible: boolean; primary_rule: string; primary_level: number; primary_distance_percent: number; primary_passed: boolean; secondary_rule?: string | null; secondary_level?: number | null; secondary_distance_percent?: number | null; secondary_passed?: boolean | null; signal_active: boolean }>;
};
export type AutoTraderState = {
  enabled: boolean;
  researchStrategies: ResearchStrategy[];
  genericSignalsEnabled: boolean;
  loading: boolean;
  busy: boolean;
  maxOrderNotional: string;
  dailyNotionalLimit: string;
  usedNotionalToday: number;
  availableNotional: number;
  serverDailyNotionalLimit: number;
  serverExplorationPercent: number;
  dailyRealizedLossLimit: string;
  explorationPercent: string;
  riskAppetite: string;
  maxOpenPositions: string;
  maxEntriesPerPosition: string;
  stopLossPercent: number;
  takeProfitPercent: number;
  vwapDurationMinutes: string; vwapParticipationPercent: string; limitOffsetBps: string; triggerOffsetBps: string; trailingDistancePercent: string;
  closedTradeWindow: string;
  lastDecision: string;
  lastDecisionAt: number;
  decisionsSinceTrade: number;
  gapReasons: Array<{ reason: string; count: number }>;
  gateHits: Array<{ gate: string; count: number }>;
  symbolGateHits: Array<{ symbol: string; direction: string; gate: string; count: number; last_detail: string; distance_to_pass: number | null;}>;
  latestSignalDiagnostic: string;
  researchStrategyDiagnostic: ResearchStrategyDiagnostic | null;
  orders: AutoTraderOrder[];
  openPositionSymbols: string[];
  ledgerOpenPositionSymbols: string[];
  executionQuality: AutoTraderExecutionQuality;
  decisionQuality: AutoTraderDecisionQuality;
  scoutOpenPositions: number;
  scoutMaxActivePositions: number;
  scoutLearningSamples: number;
  scoutLearningWinRate?: number;
  scoutLearningRealizedPnl: number;
  realizedPnl?: number | null;
  unrealizedPnl?: number | null;
  exitLearningSamples: number;
  closedTradeAverageReturnPercent?: number;
  closedTradeWins: number;
  closedTradeLosses: number;
  closedTradeFlat: number;
  closedTradeRealizedUsd?: number;
  closedTradeFeesUsd?: number;
  learningSamples: number;
  learningWinRate?: number;
  preferredOrderType?: string;
  parameterProfileSamples: number[];
  decisionParameters?: AutoTraderDecisionParameters;
  decisionParametersError?: string;
  error?: string;
};
export type AutoTraderDecisionQuality = {
  decision_samples: number;
  filled_entry_samples: number;
  unresolved_trade_samples: number;
  unpriced_closed_trade_samples: number;
  completed_trade_samples: number;
  scout_filled_entry_samples?: number;
  scout_unresolved_trade_samples?: number;
  scout_unpriced_closed_trade_samples?: number;
  scout_completed_trade_samples?: number;
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

type TraderStatus = { enabled: boolean; research_strategy?: ResearchStrategy; research_strategies?: ResearchStrategy[]; generic_signals_enabled?: boolean; max_order_notional: number; daily_notional_limit: number; daily_realized_loss_limit: number; exploration_percent: number; risk_appetite?: number; max_open_positions: number; max_entries_per_auto_position: number; stop_loss_percent: number; take_profit_percent: number; vwap_duration_minutes: number; vwap_participation_percent: number; limit_offset_bps: number; trigger_offset_bps: number; trailing_distance_percent: number; closed_trade_window?: number; used_notional_today: number; available_notional: number; last_decision?: string; last_decision_at?: number; decisions_since_trade?: number; gap_reasons?: Array<{ reason: string; count: number }>; gate_hits?: Array<{ gate: string; count: number }>; symbol_gate_hits?: Array<{ symbol: string; direction: string; gate: string; count: number; last_detail: string; distance_to_pass?: number | null;}>; latest_signal_diagnostic?: string; research_strategy_diagnostic?: ResearchStrategyDiagnostic | null };

export class AutoTraderModel {
  readonly state$ = new BehaviorSubject<AutoTraderState>({ enabled: false, researchStrategies: ['donchian-55-btc200'], genericSignalsEnabled: true, loading: true, busy: false, maxOrderNotional: '100', dailyNotionalLimit: '1000', usedNotionalToday: 0, availableNotional: 1000, serverDailyNotionalLimit: 1000, serverExplorationPercent: 10, lastDecision: '', lastDecisionAt: 0, decisionsSinceTrade: 0, gapReasons: [], gateHits: [], symbolGateHits: [], latestSignalDiagnostic: '', researchStrategyDiagnostic: null, orders: [], openPositionSymbols: [], ledgerOpenPositionSymbols: [], executionQuality: emptyExecutionQuality, decisionQuality: emptyDecisionQuality, scoutOpenPositions: 0, scoutMaxActivePositions: 5, scoutLearningSamples: 0, scoutLearningRealizedPnl: 0, realizedPnl: 0, unrealizedPnl: 0, exitLearningSamples: 0, closedTradeWins: 0, closedTradeLosses: 0, closedTradeFlat: 0, dailyRealizedLossLimit: '10', explorationPercent: '10', riskAppetite: '50', maxOpenPositions: '20', maxEntriesPerPosition: '1', stopLossPercent: 0.5, takeProfitPercent: 0.8, vwapDurationMinutes: '5', vwapParticipationPercent: '20', limitOffsetBps: '0', triggerOffsetBps: '20', trailingDistancePercent: '0.10', closedTradeWindow: '200', learningSamples: 0, parameterProfileSamples: [0, 0, 0] });
  private timer?: ReturnType<typeof setInterval>;
  private limitsDirty = false;
  private executionParametersDirty = false;
  private refreshInFlight = false;
  private analyticsRefreshInFlight = false;
  private lastAnalyticsRefreshAt = 0;
  start() { if (!this.timer) { void this.refresh(); this.timer = setInterval(() => void this.refresh(), 5000); } }
  stop() { if (this.timer) clearInterval(this.timer); this.timer = undefined; }
  setLimits(maxOrderNotional: string, dailyNotionalLimit: string, dailyRealizedLossLimit: string, explorationPercent: string, maxOpenPositions: string) { this.limitsDirty = true; this.state$.next({ ...this.state$.value, maxOrderNotional, dailyNotionalLimit, dailyRealizedLossLimit, explorationPercent, maxOpenPositions }); }

  setRiskAppetite(riskAppetite: string) { this.limitsDirty = true; this.state$.next({ ...this.state$.value, riskAppetite }); }

  setResearchStrategies(researchStrategies: ResearchStrategy[]) { this.limitsDirty = true; this.state$.next({ ...this.state$.value, researchStrategies }); }
  setGenericSignalsEnabled(genericSignalsEnabled: boolean) { this.limitsDirty = true; this.state$.next({ ...this.state$.value, genericSignalsEnabled }); }

  setClosedTradeWindow(closedTradeWindow: string) { this.limitsDirty = true; this.state$.next({ ...this.state$.value, closedTradeWindow }); }
  setMaxEntriesPerPosition(maxEntriesPerPosition: string) { this.limitsDirty = true; this.state$.next({ ...this.state$.value, maxEntriesPerPosition }); }

  setExecutionParameters(values: Partial<Pick<AutoTraderState, 'vwapDurationMinutes' | 'vwapParticipationPercent' | 'limitOffsetBps' | 'triggerOffsetBps' | 'trailingDistancePercent'>>) { this.executionParametersDirty = true; this.state$.next({ ...this.state$.value, ...values }); }

  async refreshDecisionParameters() {
    try {
      const response = await fetch('/positions/auto-trader/parameters');
      if (!response.ok) throw await this.responseError('decision parameters', response);
      const decisionParameters = await response.json() as AutoTraderDecisionParameters;
      this.state$.next({ ...this.state$.value, decisionParameters, decisionParametersError: undefined });
    } catch (error) {
      this.state$.next({ ...this.state$.value, decisionParametersError: error instanceof Error ? error.message : 'Could not load live decision parameters.' });
    }
  }

  async refresh() {
    if (this.refreshInFlight) return;
    this.refreshInFlight = true;
    void this.refreshAnalyticsIfDue();
    try {
      const [statusResponse, activityResponse, positionsResponse, ratesResponse] = await Promise.all([fetch('/positions/auto-trader'), fetch('/positions/auto-trader/activity'), fetch('/positions'), fetch('/positions/usd-rates')]);
      if (!statusResponse.ok) throw await this.responseError('status', statusResponse);
      if (!activityResponse.ok) throw await this.responseError('activity', activityResponse);
      if (!positionsResponse.ok) throw await this.responseError('positions', positionsResponse);
      const usdRates = ratesResponse.ok ? (await ratesResponse.json() as { rates?: Record<string, number> }).rates ?? {} : {};
      const status = await statusResponse.json() as TraderStatus;
      const positions = await positionsResponse.json() as { positions: Array<{ symbol: string; quantity: number }> };
      const ledgerOpenPositionSymbols = positions.positions.filter((position) => Math.abs(position.quantity) > 1e-12).map((position) => position.symbol);
      const activity = await activityResponse.json() as { orders: AutoTraderOrder[]; open_position_symbols?: string[]; scout_open_positions?: number; scout_max_active_positions?: number; scout_learning_samples?: number; scout_learning_win_rate?: number; scout_learning_realized_pnl?: number; realized_pnl?: number | null; realized_pnl_by_quote?: Record<string, number> | null; unrealized_pnl?: number | null; unrealized_pnl_by_quote?: Record<string, number> | null; exit_learning_samples: number; closed_trade_average_return_percent?: number; closed_trade_wins?: number; closed_trade_losses?: number; closed_trade_flat?: number; closed_trade_realized_usd?: number; closed_trade_fees_usd?: number; learning_samples: number; learning_win_rate?: number; preferred_order_type?: string; parameter_profile_samples?: number[] };
      const rateForQuote = (quote: string) => quote === 'USD' ? 1 : usdRates[quote];
      const convertQuoteTotals = (totals?: Record<string, number> | null) => {
        if (!totals) return null;
        let converted = 0;
        for (const [quote, amount] of Object.entries(totals)) {
          const rate = rateForQuote(quote);
          if (typeof rate !== 'number' || !Number.isFinite(rate)) return null;
          converted += amount * rate;
        }
        return converted;
      };
      const orders = activity.orders.map((order) => {
        const quote = order.symbol.split('-')[1];
        const rate = quote ? rateForQuote(quote) : undefined;
        return { ...order, realized_pnl_usd: typeof rate === 'number' && Number.isFinite(rate) ? order.realized_pnl * rate : null };
      });
      this.state$.next({ ...this.state$.value, enabled: status.enabled, researchStrategies: this.limitsDirty ? this.state$.value.researchStrategies : status.research_strategies ?? [status.research_strategy ?? 'donchian-55-btc200'], genericSignalsEnabled: this.limitsDirty ? this.state$.value.genericSignalsEnabled : status.generic_signals_enabled ?? true, maxOrderNotional: this.limitsDirty ? this.state$.value.maxOrderNotional : String(status.max_order_notional), dailyNotionalLimit: this.limitsDirty ? this.state$.value.dailyNotionalLimit : String(status.daily_notional_limit), usedNotionalToday: status.used_notional_today, availableNotional: status.available_notional, serverDailyNotionalLimit: status.daily_notional_limit, serverExplorationPercent: status.exploration_percent, dailyRealizedLossLimit: this.limitsDirty ? this.state$.value.dailyRealizedLossLimit : String(status.daily_realized_loss_limit), explorationPercent: this.limitsDirty ? this.state$.value.explorationPercent : String(status.exploration_percent), riskAppetite: this.limitsDirty ? this.state$.value.riskAppetite : String(status.risk_appetite ?? 50), maxOpenPositions: this.limitsDirty ? this.state$.value.maxOpenPositions : String(status.max_open_positions), maxEntriesPerPosition: this.limitsDirty ? this.state$.value.maxEntriesPerPosition : String(status.max_entries_per_auto_position), stopLossPercent: status.stop_loss_percent, takeProfitPercent: status.take_profit_percent, vwapDurationMinutes: this.executionParametersDirty ? this.state$.value.vwapDurationMinutes : String(status.vwap_duration_minutes), vwapParticipationPercent: this.executionParametersDirty ? this.state$.value.vwapParticipationPercent : String(status.vwap_participation_percent), limitOffsetBps: this.executionParametersDirty ? this.state$.value.limitOffsetBps : String(status.limit_offset_bps), triggerOffsetBps: this.executionParametersDirty ? this.state$.value.triggerOffsetBps : String(status.trigger_offset_bps), trailingDistancePercent: this.executionParametersDirty ? this.state$.value.trailingDistancePercent : String(status.trailing_distance_percent), closedTradeWindow: this.limitsDirty ? this.state$.value.closedTradeWindow : String(status.closed_trade_window ?? 200), lastDecision: status.last_decision ?? '', lastDecisionAt: status.last_decision_at ?? 0, decisionsSinceTrade: status.decisions_since_trade ?? 0, gapReasons: status.gap_reasons ?? [], gateHits: status.gate_hits ?? [], symbolGateHits: status.symbol_gate_hits?.map((hit) => ({ ...hit, distance_to_pass: hit.distance_to_pass ?? null })) ?? [], latestSignalDiagnostic: status.latest_signal_diagnostic ?? '', researchStrategyDiagnostic: status.research_strategy_diagnostic ?? null, orders, openPositionSymbols: activity.open_position_symbols ?? [], ledgerOpenPositionSymbols, scoutOpenPositions: activity.scout_open_positions ?? 0, scoutMaxActivePositions: activity.scout_max_active_positions ?? 5, scoutLearningSamples: activity.scout_learning_samples ?? 0, scoutLearningWinRate: activity.scout_learning_win_rate, scoutLearningRealizedPnl: activity.scout_learning_realized_pnl ?? 0, realizedPnl: convertQuoteTotals(activity.realized_pnl_by_quote), unrealizedPnl: convertQuoteTotals(activity.unrealized_pnl_by_quote), exitLearningSamples: activity.exit_learning_samples, closedTradeAverageReturnPercent: activity.closed_trade_average_return_percent, closedTradeWins: activity.closed_trade_wins ?? 0, closedTradeLosses: activity.closed_trade_losses ?? 0, closedTradeFlat: activity.closed_trade_flat ?? 0, closedTradeRealizedUsd: activity.closed_trade_realized_usd, closedTradeFeesUsd: activity.closed_trade_fees_usd, learningSamples: activity.learning_samples, learningWinRate: activity.learning_win_rate, preferredOrderType: activity.preferred_order_type, parameterProfileSamples: activity.parameter_profile_samples ?? [0, 0, 0], loading: false, error: undefined });
    } catch (error) {
      console.error('Auto Trader status/activity refresh failed:', error);
      this.state$.next({ ...this.state$.value, loading: false, error: error instanceof Error ? error.message : 'Could not load Auto Trader status and activity.' });
    } finally {
      this.refreshInFlight = false;
    }
  }

  private async refreshAnalyticsIfDue() {
    const now = Date.now();
    if (this.analyticsRefreshInFlight || now - this.lastAnalyticsRefreshAt < 60_000) return;
    this.analyticsRefreshInFlight = true;
    this.lastAnalyticsRefreshAt = now;
    try {
      const now = new Date();
      const todayStartUtc = Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate());
      const since = 'since=' + todayStartUtc;
      const [executionResponse, decisionResponse] = await Promise.all([
        fetch('/signals/auto-trader-execution-quality?' + since),
        fetch('/signals/auto-trader-decision-quality?' + since),
      ]);
      if (!executionResponse.ok) throw await this.responseError('execution quality', executionResponse);
      if (!decisionResponse.ok) throw await this.responseError('decision quality', decisionResponse);
      const [executionQuality, decisionQuality] = await Promise.all([executionResponse.json() as Promise<AutoTraderExecutionQuality>, decisionResponse.json() as Promise<AutoTraderDecisionQuality>]);
      this.state$.next({ ...this.state$.value, executionQuality, decisionQuality });
    } catch (error) {
      console.error('Auto Trader analytics refresh failed:', error);
    } finally {
      this.analyticsRefreshInFlight = false;
    }
  }

  private async responseError(endpoint: 'status' | 'activity' | 'positions' | 'execution quality' | 'decision quality' | 'decision parameters', response: Response) {
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
      const response = await fetch('/positions/auto-trader/settings', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ research_strategies: current.researchStrategies, generic_signals_enabled: current.genericSignalsEnabled, max_order_notional: Number(current.maxOrderNotional), daily_notional_limit: Number(current.dailyNotionalLimit), daily_realized_loss_limit: Number(current.dailyRealizedLossLimit), exploration_percent: Number(current.explorationPercent), risk_appetite: Number(current.riskAppetite), max_open_positions: Number(current.maxOpenPositions), max_entries_per_auto_position: Number(current.maxEntriesPerPosition), vwap_duration_minutes: Number(current.vwapDurationMinutes), vwap_participation_percent: Number(current.vwapParticipationPercent), limit_offset_bps: Number(current.limitOffsetBps), trigger_offset_bps: Number(current.triggerOffsetBps), trailing_distance_percent: Number(current.trailingDistancePercent), closed_trade_window: Number(current.closedTradeWindow) }) });
      if (!response.ok) throw new Error(await response.text());
      this.limitsDirty = false; this.executionParametersDirty = false;
      await this.refresh();
      this.state$.next({ ...this.state$.value, busy: false });
    } catch (error) { this.state$.next({ ...this.state$.value, busy: false, error: error instanceof Error ? error.message : 'Could not save limits.' }); }
  }

}
