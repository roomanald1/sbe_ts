import { BehaviorSubject } from 'rxjs';
import { currentPositionSnapshot, subscribePositionStream } from '../position_stream';
import { currentAutoTraderSnapshot, subscribeAutoTraderStream, type AutoTraderUiSnapshot } from '../auto_trader_stream';
import { subscribeInsightsStream } from '../insights_stream';

export type AutoTraderOrder = {
  timestamp: number;
  triggered_at?: number;
  symbol: string;
  side: string;
  order_type: string;
  purpose?: string;
  strategy?: string;
  decision_id?: string;
  quantity: number;
  filled_quantity: number;
  submitted_limit_price?: number;
  target_price?: number;
  average_price?: number;
  status: string;
  failure_reason?: string;
  realized_pnl: number;
  realized_pnl_usd?: number | null;
};
export type AutoTraderDecisionParameter = { section: string; name: string; value: number; unit: string };
export type AutoTraderContextParameters = { context: string; learning_samples: number; positive_rate?: number; strength_floor: number; exploration_score_floor: number; exploration_allowed: boolean };
export type AutoTraderLatestExecutionParameters = { timestamp: number; symbol: string; context: string; baseline_order_type: string; selected_order_type: string; profile: number; vwap_duration_minutes: number; vwap_participation_percent: number; schedule_depth_threshold: number; proposed_depth_ratio: number; schedule_threshold_samples: number; limit_offset_bps: number; trigger_offset_bps: number; trailing_distance_percent: number };
export type AutoTraderStrategyParameter = { key: string; label: string; value: number; learned_value: number; configured_value?: number | null; default_value: number; min: number; max: number; step: number; unit: string; overridden: boolean };
export type AutoTraderStrategyProfile = { symbol: string; strategy: Strategy; learning_samples: number; positive_rate?: number; parameters: AutoTraderStrategyParameter[] };
export type AutoTraderDecisionParameters = { refreshed_at: number; thresholds: AutoTraderDecisionParameter[]; contexts: AutoTraderContextParameters[]; strategy_profiles: AutoTraderStrategyProfile[]; active_exit_policy: { stop_loss_percent: number; take_profit_percent: number }; latest_execution?: AutoTraderLatestExecutionParameters };
export type Strategy = 'live-insights' | 'live-momentum' | 'live-trade-flow' | 'live-order-book' | 'live-vwap-dislocation' | 'live-absorption-pressure' | 'donchian-55-btc200' | 'donchian-20-sma200' | 'donchian-20-btc200' | 'trend-sleeves-btc200' | 'trend-50-200' | 'relative-strength-90-sma200' | 'risk-adjusted-monthly-momentum' | 'rsi2-oversold-rebound' | 'dated-futures-carry';
export type StrategyBudget = { strategy: Strategy; base_budget: number; effective_budget: number; used_notional: number; learning_multiplier: number; learning_samples: number; positive_rate?: number | null; realized_pnl: number };
export type ClosedStrategyOutcome = { trades: number; wins: number; losses: number; flat: number; winRate: number; averageReturn: number };
export type StrategyDiagnostic = {
  strategy: Strategy; as_of_ms: number; btc_close: number | null; btc_sma: number | null;
  btc_distance_percent: number | null; btc_window_days: number | null; btc_gate_required: boolean; btc_gate_passed: boolean | null;
  symbols: Array<{ symbol: string; close: number; eligible: boolean; primary_rule: string; primary_level: number; primary_distance_percent: number; primary_metric_value?: number | null; primary_metric_unit?: string | null; primary_passed: boolean; secondary_rule?: string | null; secondary_level?: number | null; secondary_distance_percent?: number | null; secondary_passed?: boolean | null; signal_active: boolean }>;
};
export type AutoTraderState = {
  enabled: boolean;
  strategies: Strategy[];
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
  maxConcurrentShadowSamples: string;
  maxIntradayOpenPositions: string; maxDailyOpenPositions: string;
  intradayNotionalLimit: string; dailyStrategyNotionalLimit: string;
  dailyStrategyBudgets: Partial<Record<Strategy, string>>;
  dailyStrategyLearning: StrategyBudget[];
  closedStrategyOutcomes: Record<string, ClosedStrategyOutcome>;
  intradayStrategyBudgets: Partial<Record<Strategy, string>>;
  intradayStrategyLearning: StrategyBudget[];
  intradayUsedNotional: number; dailyStrategyUsedNotional: number;
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
  strategyDiagnostics: StrategyDiagnostic[];
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
  strategyParameterOverrides: Record<string, number>;
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
  cohorts: Array<{ dimension: string; value: string; completed_trade_samples: number; average_net_return_percent: number; win_rate: number; total_net_pnl: number | null; pnl_quote_currency: string | null; preliminary: boolean }>;
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

type TraderStatus = { enabled: boolean; strategy?: Strategy; strategies?: Strategy[]; signal_strategies_enabled?: boolean; max_order_notional: number; daily_notional_limit: number; daily_realized_loss_limit: number; exploration_percent: number; risk_appetite?: number; max_open_positions: number; max_concurrent_shadow_samples?: number; max_intraday_open_positions: number; max_daily_open_positions: number; intraday_notional_limit: number; daily_strategy_notional_limit: number; daily_strategy_budgets?: StrategyBudget[]; intraday_strategy_budgets?: StrategyBudget[]; intraday_used_notional: number; daily_strategy_used_notional: number; max_entries_per_auto_position: number; stop_loss_percent: number; take_profit_percent: number; vwap_duration_minutes: number; vwap_participation_percent: number; limit_offset_bps: number; trigger_offset_bps: number; trailing_distance_percent: number; closed_trade_window?: number; used_notional_today: number; available_notional: number; last_decision?: string; last_decision_at?: number; decisions_since_trade?: number; gap_reasons?: Array<{ reason: string; count: number }>; gate_hits?: Array<{ gate: string; count: number }>; symbol_gate_hits?: Array<{ symbol: string; direction: string; gate: string; count: number; last_detail: string; distance_to_pass?: number | null;}>; latest_signal_diagnostic?: string; strategy_diagnostics?: StrategyDiagnostic[] };
type TraderActivity = { orders: AutoTraderOrder[]; open_position_symbols?: string[]; scout_open_positions?: number; scout_max_active_positions?: number; scout_learning_samples?: number; scout_learning_win_rate?: number; scout_learning_realized_pnl?: number; realized_pnl?: number | null; realized_pnl_by_quote?: Record<string, number> | null; unrealized_pnl?: number | null; unrealized_pnl_by_quote?: Record<string, number> | null; exit_learning_samples: number; closed_trade_average_return_percent?: number; closed_trade_wins?: number; closed_trade_losses?: number; closed_trade_flat?: number; closed_trade_realized_usd?: number; closed_trade_fees_usd?: number; learning_samples: number; learning_win_rate?: number; preferred_order_type?: string; parameter_profile_samples?: number[] };

export class AutoTraderModel {
  readonly state$ = new BehaviorSubject<AutoTraderState>({ enabled: false, strategies: ['donchian-55-btc200', 'live-momentum', 'live-trade-flow', 'live-order-book', 'live-vwap-dislocation', 'live-absorption-pressure'], loading: true, busy: false, maxOrderNotional: '100', dailyNotionalLimit: '1000', usedNotionalToday: 0, availableNotional: 1000, serverDailyNotionalLimit: 1000, serverExplorationPercent: 10, lastDecision: '', lastDecisionAt: 0, decisionsSinceTrade: 0, gapReasons: [], gateHits: [], symbolGateHits: [], latestSignalDiagnostic: '', strategyDiagnostics: [], orders: [], openPositionSymbols: [], ledgerOpenPositionSymbols: [], executionQuality: emptyExecutionQuality, decisionQuality: emptyDecisionQuality, scoutOpenPositions: 0, scoutMaxActivePositions: 5, scoutLearningSamples: 0, scoutLearningRealizedPnl: 0, realizedPnl: 0, unrealizedPnl: 0, exitLearningSamples: 0, closedTradeWins: 0, closedTradeLosses: 0, closedTradeFlat: 0, dailyRealizedLossLimit: '10', explorationPercent: '10', riskAppetite: '50', maxOpenPositions: '20', maxConcurrentShadowSamples: '5', maxIntradayOpenPositions: '10', maxDailyOpenPositions: '10', intradayNotionalLimit: '500', dailyStrategyNotionalLimit: '500', dailyStrategyBudgets: {}, dailyStrategyLearning: [], closedStrategyOutcomes: {}, intradayStrategyBudgets: {}, intradayStrategyLearning: [], intradayUsedNotional: 0, dailyStrategyUsedNotional: 0, maxEntriesPerPosition: '1', stopLossPercent: 0.5, takeProfitPercent: 0.8, vwapDurationMinutes: '5', vwapParticipationPercent: '20', limitOffsetBps: '0', triggerOffsetBps: '20', trailingDistancePercent: '0.10', closedTradeWindow: '200', learningSamples: 0, parameterProfileSamples: [0, 0, 0], strategyParameterOverrides: {} });
  private timer?: ReturnType<typeof setInterval>;
  private limitsDirty = false;
  private categoryLimitsDirty = false;
  private executionParametersDirty = false;
  private strategyParametersDirty = false;
  private refreshInFlight = false;
  private usdRates: Record<string, number> = {};
  private stopPositionStream?: () => void;
  private stopAutoTraderStream?: () => void;
  private stopInsightsStream?: () => void;
  private autoTraderSnapshot?: AutoTraderUiSnapshot;
  private autoTraderSnapshotAt = 0;
  start() {
    if (this.timer) return;
    this.usdRates = currentPositionSnapshot()?.usd_rates?.rates ?? {};
    this.stopPositionStream = subscribePositionStream((snapshot) => {
      this.usdRates = snapshot.usd_rates?.rates ?? this.usdRates;
      if (this.autoTraderSnapshot) void this.refresh();
    });
    this.autoTraderSnapshot = currentAutoTraderSnapshot();
    if (this.autoTraderSnapshot) this.autoTraderSnapshotAt = Date.now();
    this.stopAutoTraderStream = subscribeAutoTraderStream((snapshot) => {
      this.autoTraderSnapshot = snapshot;
      this.autoTraderSnapshotAt = Date.now();
      const decisionParameters = snapshot.parameters as AutoTraderDecisionParameters;
      const savedOverrides = Object.fromEntries(decisionParameters.strategy_profiles.flatMap((profile) => profile.parameters
        .filter((parameter) => parameter.configured_value !== undefined && parameter.configured_value !== null)
        .map((parameter) => [profile.symbol + ':' + profile.strategy + ':' + parameter.key, parameter.configured_value!] as const)));
      this.state$.next({ ...this.state$.value, decisionParameters, strategyParameterOverrides: this.strategyParametersDirty ? this.state$.value.strategyParameterOverrides : savedOverrides, decisionParametersError: undefined });
      void this.refresh();
    });
    this.stopInsightsStream = subscribeInsightsStream((snapshot) => {
      this.state$.next({ ...this.state$.value,
        executionQuality: snapshot.execution_quality as AutoTraderExecutionQuality ?? this.state$.value.executionQuality,
        decisionQuality: snapshot.decision_quality as AutoTraderDecisionQuality ?? this.state$.value.decisionQuality,
      });
    });
    void this.refresh();
    this.timer = setInterval(() => { if (Date.now() - this.autoTraderSnapshotAt > 45_000) void this.refresh(); }, 30_000);
  }
  stop() {
    if (this.timer) clearInterval(this.timer);
    this.timer = undefined;
    this.stopPositionStream?.();
    this.stopPositionStream = undefined;
    this.stopAutoTraderStream?.();
    this.stopAutoTraderStream = undefined;
    this.stopInsightsStream?.();
    this.stopInsightsStream = undefined;
  }
  setLimits(maxOrderNotional: string, dailyNotionalLimit: string, dailyRealizedLossLimit: string, explorationPercent: string, maxOpenPositions: string) { this.limitsDirty = true; this.state$.next({ ...this.state$.value, maxOrderNotional, dailyNotionalLimit, dailyRealizedLossLimit, explorationPercent, maxOpenPositions }); }
  setCategoryLimits(values: Partial<Pick<AutoTraderState, 'maxIntradayOpenPositions' | 'maxDailyOpenPositions' | 'intradayNotionalLimit' | 'dailyStrategyNotionalLimit'>>) { this.categoryLimitsDirty = true; this.state$.next({ ...this.state$.value, ...values }); }

  setDailyStrategyBudget(strategy: Strategy, value: string) {
    this.categoryLimitsDirty = true;
    this.state$.next({ ...this.state$.value, dailyStrategyBudgets: { ...this.state$.value.dailyStrategyBudgets, [strategy]: value } });
  }

  setIntradayStrategyBudget(strategy: Strategy, value: string) {
    this.categoryLimitsDirty = true;
    this.state$.next({ ...this.state$.value, intradayStrategyBudgets: { ...this.state$.value.intradayStrategyBudgets, [strategy]: value } });
  }

  setRiskAppetite(riskAppetite: string) { this.limitsDirty = true; this.state$.next({ ...this.state$.value, riskAppetite }); }

  setMaxConcurrentShadowSamples(maxConcurrentShadowSamples: string) { this.limitsDirty = true; this.state$.next({ ...this.state$.value, maxConcurrentShadowSamples }); }

  setStrategies(strategies: Strategy[]) { this.limitsDirty = true; this.state$.next({ ...this.state$.value, strategies }); }

  setClosedTradeWindow(closedTradeWindow: string) { this.limitsDirty = true; this.state$.next({ ...this.state$.value, closedTradeWindow }); }
  setMaxEntriesPerPosition(maxEntriesPerPosition: string) { this.limitsDirty = true; this.state$.next({ ...this.state$.value, maxEntriesPerPosition }); }

  setExecutionParameters(values: Partial<Pick<AutoTraderState, 'vwapDurationMinutes' | 'vwapParticipationPercent' | 'limitOffsetBps' | 'triggerOffsetBps' | 'trailingDistancePercent'>>) { this.executionParametersDirty = true; this.state$.next({ ...this.state$.value, ...values }); }

  setStrategyParameter(key: string, value?: number) {
    const overrides = { ...this.state$.value.strategyParameterOverrides };
    if (value === undefined) delete overrides[key];
    else overrides[key] = value;
    this.strategyParametersDirty = true;
    this.state$.next({ ...this.state$.value, strategyParameterOverrides: overrides });
  }

  async refresh() {
    if (this.refreshInFlight) return;
    this.refreshInFlight = true;
    try {
      let status: TraderStatus;
      let activity: TraderActivity;
      let positions: { positions: Array<{ symbol: string; quantity: number }> };
      let auditRows: Array<{ decision_id?: string; stage?: string; strategy?: string; symbol?: string; timestamp?: number; filled_quantity?: number; average_price?: number; exit_filled_quantity?: number; realized_pnl?: number; exit_order_type?: string; barrier_order_id?: string; exit_fills?: Array<{ parent_order_id?: string }>; entry_context?: { quantity?: number; average_price?: number } }>;
      const autoSnapshot = this.autoTraderSnapshot;
      const useStreamSnapshot = autoSnapshot && Date.now() - this.autoTraderSnapshotAt <= 45_000;
      if (useStreamSnapshot && autoSnapshot) {
        status = autoSnapshot.status as TraderStatus;
        activity = autoSnapshot.activity as TraderActivity;
        const autoPositions = currentPositionSnapshot()?.auto_positions.positions ?? [];
        positions = { positions: autoPositions.map(({ symbol, quantity }) => ({ symbol, quantity })) };
        auditRows = autoSnapshot.execution_audit.items as typeof auditRows;
      } else {
        const [statusResponse, activityResponse, positionsResponse, auditResponse] = await Promise.all([fetch('/positions/auto-trader'), fetch('/positions/auto-trader/activity'), fetch('/positions?source=auto'), fetch('/signals/auto-trader-execution-audit')]);
        if (!statusResponse.ok) throw await this.responseError('status', statusResponse);
        if (!activityResponse.ok) throw await this.responseError('activity', activityResponse);
        if (!positionsResponse.ok) throw await this.responseError('positions', positionsResponse);
        status = await statusResponse.json() as TraderStatus;
        activity = await activityResponse.json() as TraderActivity;
        positions = await positionsResponse.json() as { positions: Array<{ symbol: string; quantity: number }> };
        const auditPage = auditResponse.ok ? await auditResponse.json() as { items?: typeof auditRows } : undefined;
        auditRows = auditPage?.items ?? [];
      }
      const usdRates = this.usdRates;
      const savedStrategies = status.strategies ?? [status.strategy ?? 'donchian-55-btc200'];
      if (status.signal_strategies_enabled && savedStrategies.includes('live-insights')) { savedStrategies.splice(savedStrategies.indexOf('live-insights'), 1, 'live-momentum', 'live-trade-flow', 'live-order-book', 'live-vwap-dislocation', 'live-absorption-pressure'); }
      if (status.signal_strategies_enabled && !savedStrategies.some((strategy) => strategy.startsWith('live-'))) savedStrategies.push('live-momentum', 'live-trade-flow', 'live-order-book', 'live-vwap-dislocation', 'live-absorption-pressure');
      const ledgerOpenPositionSymbols = positions.positions.filter((position) => Math.abs(position.quantity) > 1e-12).map((position) => position.symbol);
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
      const auditTrades = new Map<string, { entry?: typeof auditRows[number]; exits: typeof auditRows }>();
      for (const row of auditRows) {
        if (!row.decision_id) continue;
        const trade = auditTrades.get(row.decision_id) ?? { exits: [] };
        if (row.stage === 'entry') trade.entry = row;
        else if (row.stage?.startsWith('exit')) trade.exits.push(row);
        auditTrades.set(row.decision_id, trade);
      }
      const closedStrategyGroups = new Map<string, Array<{ pnl: number; quantity: number; entryPrice: number }>>();
      for (const trade of auditTrades.values()) {
        const entry = trade.entry;
        const externalIds = new Set(trade.exits.filter((row) => row.exit_order_type === 'external-ledger-fill').flatMap((row) => row.exit_fills?.map((fill) => fill.parent_order_id).filter((id): id is string => Boolean(id)) ?? []));
        const exits = trade.exits.filter((row) => !(row.exit_order_type === 'StopMarket' && row.barrier_order_id && externalIds.has(row.barrier_order_id)));
        const exited = exits.reduce((sum, row) => sum + (row.exit_filled_quantity ?? 0), 0);
        const quantity = Math.max(entry?.filled_quantity ?? 0, ...exits.map((row) => row.entry_context?.quantity ?? 0));
        const entryPrice = entry?.average_price || exits.find((row) => row.entry_context?.average_price)?.entry_context?.average_price;
        if (!entry || !(quantity > 0 && typeof entryPrice === 'number' && entryPrice > 0) || exited + 1e-12 < quantity) continue;
        const strategy = entry.strategy ?? 'live-insights';
        const group = closedStrategyGroups.get(strategy) ?? [];
        group.push({ pnl: exits.reduce((sum, row) => sum + (row.realized_pnl ?? 0), 0), quantity, entryPrice });
        closedStrategyGroups.set(strategy, group);
      }
      const closedStrategyOutcomes = Object.fromEntries([...closedStrategyGroups].map(([strategy, trades]) => {
        const wins = trades.filter((trade) => trade.pnl > 0).length;
        const losses = trades.filter((trade) => trade.pnl < 0).length;
        return [strategy, { trades: trades.length, wins, losses, flat: trades.length - wins - losses, winRate: wins / trades.length * 100,
          averageReturn: trades.reduce((sum, trade) => sum + trade.pnl / (trade.quantity * trade.entryPrice) * 100, 0) / trades.length }];
      }));
      const savedDailyBudgets = Object.fromEntries((status.daily_strategy_budgets ?? []).map((budget) => [budget.strategy, String(budget.base_budget)])) as Partial<Record<Strategy, string>>;
      const savedIntradayBudgets = Object.fromEntries((status.intraday_strategy_budgets ?? []).map((budget) => [budget.strategy, String(budget.base_budget)])) as Partial<Record<Strategy, string>>;
      this.state$.next({ ...this.state$.value, enabled: status.enabled, strategies: this.limitsDirty ? this.state$.value.strategies : savedStrategies, maxOrderNotional: this.limitsDirty ? this.state$.value.maxOrderNotional : String(status.max_order_notional), dailyNotionalLimit: this.limitsDirty ? this.state$.value.dailyNotionalLimit : String(status.daily_notional_limit), usedNotionalToday: status.used_notional_today, availableNotional: status.available_notional, serverDailyNotionalLimit: status.daily_notional_limit, serverExplorationPercent: status.exploration_percent, dailyRealizedLossLimit: this.limitsDirty ? this.state$.value.dailyRealizedLossLimit : String(status.daily_realized_loss_limit), explorationPercent: this.limitsDirty ? this.state$.value.explorationPercent : String(status.exploration_percent), riskAppetite: this.limitsDirty ? this.state$.value.riskAppetite : String(status.risk_appetite ?? 50), maxOpenPositions: this.limitsDirty ? this.state$.value.maxOpenPositions : String(status.max_open_positions), maxConcurrentShadowSamples: this.limitsDirty ? this.state$.value.maxConcurrentShadowSamples : String(status.max_concurrent_shadow_samples ?? 5), maxIntradayOpenPositions: this.categoryLimitsDirty ? this.state$.value.maxIntradayOpenPositions : String(status.max_intraday_open_positions), maxDailyOpenPositions: this.categoryLimitsDirty ? this.state$.value.maxDailyOpenPositions : String(status.max_daily_open_positions), intradayNotionalLimit: this.categoryLimitsDirty ? this.state$.value.intradayNotionalLimit : String(status.intraday_notional_limit), dailyStrategyNotionalLimit: this.categoryLimitsDirty ? this.state$.value.dailyStrategyNotionalLimit : String(status.daily_strategy_notional_limit), dailyStrategyBudgets: this.categoryLimitsDirty ? this.state$.value.dailyStrategyBudgets : savedDailyBudgets, dailyStrategyLearning: status.daily_strategy_budgets ?? [], closedStrategyOutcomes, intradayStrategyBudgets: this.categoryLimitsDirty ? this.state$.value.intradayStrategyBudgets : savedIntradayBudgets, intradayStrategyLearning: status.intraday_strategy_budgets ?? [], intradayUsedNotional: status.intraday_used_notional, dailyStrategyUsedNotional: status.daily_strategy_used_notional, maxEntriesPerPosition: this.limitsDirty ? this.state$.value.maxEntriesPerPosition : String(status.max_entries_per_auto_position), stopLossPercent: status.stop_loss_percent, takeProfitPercent: status.take_profit_percent, vwapDurationMinutes: this.executionParametersDirty ? this.state$.value.vwapDurationMinutes : String(status.vwap_duration_minutes), vwapParticipationPercent: this.executionParametersDirty ? this.state$.value.vwapParticipationPercent : String(status.vwap_participation_percent), limitOffsetBps: this.executionParametersDirty ? this.state$.value.limitOffsetBps : String(status.limit_offset_bps), triggerOffsetBps: this.executionParametersDirty ? this.state$.value.triggerOffsetBps : String(status.trigger_offset_bps), trailingDistancePercent: this.executionParametersDirty ? this.state$.value.trailingDistancePercent : String(status.trailing_distance_percent), closedTradeWindow: this.limitsDirty ? this.state$.value.closedTradeWindow : String(status.closed_trade_window ?? 200), lastDecision: status.last_decision ?? '', lastDecisionAt: status.last_decision_at ?? 0, decisionsSinceTrade: status.decisions_since_trade ?? 0, gapReasons: status.gap_reasons ?? [], gateHits: status.gate_hits ?? [], symbolGateHits: status.symbol_gate_hits?.map((hit) => ({ ...hit, distance_to_pass: hit.distance_to_pass ?? null })) ?? [], latestSignalDiagnostic: status.latest_signal_diagnostic ?? '', strategyDiagnostics: status.strategy_diagnostics ?? [], orders, openPositionSymbols: activity.open_position_symbols ?? [], ledgerOpenPositionSymbols, scoutOpenPositions: activity.scout_open_positions ?? 0, scoutMaxActivePositions: activity.scout_max_active_positions ?? 5, scoutLearningSamples: activity.scout_learning_samples ?? 0, scoutLearningWinRate: activity.scout_learning_win_rate, scoutLearningRealizedPnl: activity.scout_learning_realized_pnl ?? 0, realizedPnl: convertQuoteTotals(activity.realized_pnl_by_quote), unrealizedPnl: convertQuoteTotals(activity.unrealized_pnl_by_quote), exitLearningSamples: activity.exit_learning_samples, closedTradeAverageReturnPercent: activity.closed_trade_average_return_percent, closedTradeWins: activity.closed_trade_wins ?? 0, closedTradeLosses: activity.closed_trade_losses ?? 0, closedTradeFlat: activity.closed_trade_flat ?? 0, closedTradeRealizedUsd: activity.closed_trade_realized_usd, closedTradeFeesUsd: activity.closed_trade_fees_usd, learningSamples: activity.learning_samples, learningWinRate: activity.learning_win_rate, preferredOrderType: activity.preferred_order_type, parameterProfileSamples: activity.parameter_profile_samples ?? [0, 0, 0], loading: false, error: undefined });
    } catch (error) {
      console.error('Auto Trader status/activity refresh failed:', error);
      this.state$.next({ ...this.state$.value, loading: false, error: error instanceof Error ? error.message : 'Could not load Auto Trader status and activity.' });
    } finally {
      this.refreshInFlight = false;
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

  async saveLimits(resetLearning = false, resetStrategyProfile?: { symbol: string; strategy: Strategy }) {
    const current = this.state$.value;
    this.state$.next({ ...current, busy: true, error: undefined });
    try {
      const response = await fetch('/positions/auto-trader/settings', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ strategies: current.strategies, signal_strategies_enabled: current.strategies.some((strategy) => strategy.startsWith('live-')), max_order_notional: Number(current.maxOrderNotional), daily_notional_limit: Number(current.dailyNotionalLimit), daily_realized_loss_limit: Number(current.dailyRealizedLossLimit), exploration_percent: Number(current.explorationPercent), risk_appetite: Number(current.riskAppetite), max_open_positions: Number(current.maxOpenPositions), max_concurrent_shadow_samples: Number(current.maxConcurrentShadowSamples), max_intraday_open_positions: Number(current.maxIntradayOpenPositions), max_daily_open_positions: Number(current.maxDailyOpenPositions), intraday_notional_limit: Number(current.intradayNotionalLimit), daily_strategy_notional_limit: Number(current.dailyStrategyNotionalLimit), daily_strategy_budgets: Object.fromEntries(current.strategies.filter((strategy) => !strategy.startsWith('live-') && strategy !== 'dated-futures-carry').map((strategy) => [strategy, Number(current.dailyStrategyBudgets[strategy] ?? 0)])), intraday_strategy_budgets: Object.fromEntries(current.strategies.filter((strategy) => strategy.startsWith('live-')).map((strategy) => [strategy, Number(current.intradayStrategyBudgets[strategy] ?? 0)])), max_entries_per_auto_position: Number(current.maxEntriesPerPosition), vwap_duration_minutes: Number(current.vwapDurationMinutes), vwap_participation_percent: Number(current.vwapParticipationPercent), limit_offset_bps: Number(current.limitOffsetBps), trigger_offset_bps: Number(current.triggerOffsetBps), trailing_distance_percent: Number(current.trailingDistancePercent), closed_trade_window: Number(current.closedTradeWindow), ...(this.strategyParametersDirty ? { strategy_parameter_overrides: current.strategyParameterOverrides } : {}), ...(resetStrategyProfile ? { reset_strategy_profile: resetStrategyProfile } : {}), reset_learning: resetLearning }) });
      if (!response.ok) throw new Error(await response.text());
      this.limitsDirty = false; this.categoryLimitsDirty = false; this.executionParametersDirty = false; this.strategyParametersDirty = false;
      await this.refresh();
      this.state$.next({ ...this.state$.value, busy: false });
    } catch (error) { this.state$.next({ ...this.state$.value, busy: false, error: error instanceof Error ? error.message : 'Could not save limits.' }); }
  }

  async resetLearning() {
    await this.saveLimits(true);
  }

  async resetStrategyProfile(symbol: string, strategy: Strategy) {
    await this.saveLimits(false, { symbol, strategy });
  }

}
