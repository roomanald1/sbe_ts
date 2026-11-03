import { BehaviorSubject } from 'rxjs';
import type { Position, PositionTrade } from './PositionTypes';
import { buildLegacyLedgerOutcomes } from './legacy_ledger_outcomes';
import { currentPositionSnapshot, subscribePositionStream, type PositionUiSnapshot } from '../position_stream';
import { currentAutoTraderSnapshot, subscribeAutoTraderStream, type AutoTraderUiSnapshot } from '../auto_trader_stream';

export type AutoTraderOutcome = {
  rowId: string; status: 'Open' | 'Partially closed' | 'Closed'; decisionId: string; symbol: string; side: string; openedAt: number; closedAt?: number;
  quantity: number; remainingQuantity?: number; signedQuantity: number; entryPrice: number; pnl: number; entryFees: number; estimatedExitFeeRate: number; estimatedExitSlippageBps: number;
  feesUsd?: number; netPnlUsd?: number;
  strategyInstanceId?: string; ledgerSource?: string; ledgerOrderType?: string; ledgerTrade?: PositionTrade;
  entryNotionalUsd?: number; feePercent?: number;
  pnlQuoteCurrency: string; quoteUsdRate?: number; strategy: string; direction: string; strength: number | null;
  stopLossPercent?: number; takeProfitPercent?: number;
  stopLossActive?: boolean; takeProfitActive?: boolean;
  carryDetails?: DatedCarryPosition;
  entryOrderType?: string;
  entryOrderTypes?: string[];
  scout: boolean; source: 'scout' | 'exploratory' | 'strict' | 'paper-carry' | 'unknown'; components: Record<string, number | null>;
};

type AuditRecord = { decision_id?: string; stage?: string; symbol?: string; timestamp?: number; side?: string; direction?: string; strategy?: string; exit_order_type?: string; barrier_order_id?: string; exit_fills?: Array<{ parent_order_id?: string }>; filled_quantity?: number; average_price?: number; exit_filled_quantity?: number; realized_pnl?: number; fees?: number; entry_fees_paid?: number; exit_fees?: number; entry_context?: { quantity?: number; average_price?: number; fees?: number; initial_entry_notional?: number; total_entry_fees_paid?: number; total_fees_paid?: number }; strength?: number; score?: number; scout?: boolean; exploratory?: boolean | null; signal_components?: Record<string, number | null>; signal_timestamp?: number; entry_gate?: { checks?: Record<string, boolean>; observed?: { strength?: number; score?: number }; thresholds?: { strength_floor?: number }; rejection_reasons?: string[] } };
export type StrictEntryGateSummary = { symbol: string; direction: string; strength: number; floor: number; gap: number; score?: number; observedAt: number; strictWalkForwardPassed?: boolean; rejectionReasons: string[] };
type DecisionParametersResponse = { thresholds?: Array<{ section?: string; name?: string; value?: number }> };
type PositionsResponse = { positions?: Position[] };
type TradesResponse = { trades?: PositionTrade[]; total?: number; next_offset?: number | null; has_older?: boolean };
type LedgerRealizedResponse = { realized_pnl_by_parent_order?: Record<string, number> };
type UsdRatesResponse = { rates?: Record<string, number> };
type LedgerHistoryCache = { dayStart?: number; trades?: PositionTrade[]; recentTrades?: PositionTrade[]; todayTradeOffset?: number; todayLoadedOffset?: number; olderTradeOffset?: number; olderHistoryExhausted?: boolean };
type StrategyLearning = { strategy: string; learning_multiplier: number; learning_samples: number; positive_rate?: number | null; realized_pnl?: number };
type StrategySymbolDiagnostic = { symbol: string; close: number; trend_return_windows?: Array<{ days: number; reference_close: number; return_percent: number }>; eligible: boolean; primary_rule: string; primary_level: number; primary_distance_percent: number; primary_metric_value?: number | null; primary_metric_unit?: string | null; primary_passed: boolean; secondary_rule?: string | null; secondary_level?: number | null; secondary_distance_percent?: number | null; secondary_passed?: boolean | null; signal_active: boolean; leader_gap_percent?: number | null; relative_strength_gap_pp?: number | null; rsi_exit_level?: number | null; maximum_hold_days?: number | null };
type StrategyDiagnostic = { strategy: string; as_of_ms: number; btc_close?: number | null; btc_sma?: number | null; btc_distance_percent: number | null; btc_window_days: number | null; btc_gate_required: boolean; btc_gate_passed: boolean | null; symbols: StrategySymbolDiagnostic[] };
type StrategyOrder = { timestamp: number; symbol: string; side: string; execution_order_id?: string; purpose?: string; strategy?: string; decision_id?: string; filled_quantity: number; average_price?: number | null; status?: string; failure_reason?: string | null; order_type?: string };
type DatedCarryPosition = { spot_symbol: string; future_symbol: string; expiry_ms: number; entry_ms: number; quantity: number; spot_entry_price: number; future_entry_price: number; entry_fees: number; status: string; spot_exit_twap?: number | null; settlement_price?: number | null; net_pnl_usdt?: number | null; settlement_source?: string | null; spot_mark_bid?: number | null; future_mark_ask?: number | null; spot_leg_pnl_usdt?: number | null; future_leg_pnl_usdt?: number | null; estimated_exit_fees_usdt?: number | null; estimated_exit_slippage_usdt?: number | null; marked_net_pnl_usdt?: number | null; mark_checked_at_ms?: number | null };
type OpenStrategyPosition = { symbol: string; strategy: string; decision_id?: string | null; quantity: number; average_entry_price: number; entry_fees: number; entry_timestamp_ms: number; entry_order_type?: string; entry_order_types?: string[]; stop_loss_percent?: number | null; take_profit_percent?: number | null; horizon_take_profit_percent?: number | null; stop_loss_active?: boolean; take_profit_active?: boolean; carry_details?: DatedCarryPosition };
type OpenPositionLot = { symbol: string; quantity: number; average_entry_price: number; entry_fees: number; estimated_exit_fee_rate: number; last_trade_timestamp: number; strategy?: string; decision_id?: string | null; entry_order_type?: string; entry_order_types?: string[]; stop_loss_percent?: number | null; take_profit_percent?: number | null; stop_loss_active?: boolean; take_profit_active?: boolean; carry_details?: DatedCarryPosition };
type AutoTraderLearningResponse = { enabled?: boolean; strategies?: string[]; strategy_diagnostics?: StrategyDiagnostic[]; daily_strategy_budgets?: StrategyLearning[]; intraday_strategy_budgets?: StrategyLearning[] };
type ActivityResponse = {
  orders?: StrategyOrder[];
  pnl_breakdown?: Array<{ symbol: string; strategy: string; decision_id?: string | null; direction: string; purpose: string; realized_pnl: number; outcomes: number }>;
  realized_pnl_by_quote?: Record<string, number> | null;
  open_positions?: Array<{ symbol: string; strategy?: string; decision_id?: string }>;
  strategy_positions?: OpenStrategyPosition[];
  position_exit_policies?: Array<{ symbol: string; stop_loss_percent: number; take_profit_percent: number }>;
};
type DatedCarryDiagnostics = {
  enabled?: boolean;
  checked_at_ms?: number;
  last_error?: string | null;
  candidates?: Array<{ spot_symbol: string; future_symbol: string; eligible: boolean; reason: string; basis_percent: number; net_after_budget_percent: number; days_to_expiry: number }>;
  positions?: DatedCarryPosition[];
};
type TradeAudit = { entry?: AuditRecord; exits: AuditRecord[] };
const quoteCurrency = (symbol: string) => symbol.includes('-') ? symbol.slice(symbol.lastIndexOf('-') + 1) : 'Unknown';
export class AutoTraderOutcomesModel {
  readonly state$ = new BehaviorSubject<{ rows: AutoTraderOutcome[]; closedLedgerRows: AutoTraderOutcome[]; strategyLearning: Record<string, StrategyLearning>; autoTraderEnabled: boolean; autoTraderStatusAvailable: boolean; enabledStrategies: string[]; strategyDiagnostics: StrategyDiagnostic[]; strategyOrders: StrategyOrder[]; strictEntryGates: Record<string, StrictEntryGateSummary>; strictBaseStrengthFloor?: number; todayStrategyRealizedUsd?: Record<string, number | null>; datedCarryDiagnostics?: DatedCarryDiagnostics; openStrategyPositions: Array<{ symbol: string; strategy?: string }>; loading: boolean; error?: string; confirmingCloseSymbols: Set<string>; closingSymbols: Set<string>; auditCursor?: number | null; loadingOlderAudit: boolean; hasOlderLedgerTrades: boolean; loadingOlderLedgerTrades: boolean }>({ rows: [], closedLedgerRows: [], strategyLearning: {}, autoTraderEnabled: false, autoTraderStatusAvailable: false, enabledStrategies: [], strategyDiagnostics: [], strategyOrders: [], strictEntryGates: {}, strictBaseStrengthFloor: undefined, todayStrategyRealizedUsd: undefined, datedCarryDiagnostics: undefined, openStrategyPositions: [], loading: true, confirmingCloseSymbols: new Set(), closingSymbols: new Set(), auditCursor: null, loadingOlderAudit: false, hasOlderLedgerTrades: false, loadingOlderLedgerTrades: false });
  private timer?: ReturnType<typeof setInterval>;
  private stopPositionStream?: () => void;
  private stopAutoTraderStream?: () => void;
  private refreshTimer?: ReturnType<typeof setTimeout>;
  private positionSnapshot?: PositionUiSnapshot;
  private autoTraderSnapshot?: AutoTraderUiSnapshot;
  private refreshing = false;
  private consumers = 0;
  private gateAuditCache: { records: AuditRecord[]; refreshedAt: number } = { records: [], refreshedAt: 0 };
  private auditRows: AuditRecord[] = [];
  private ledgerRowsCache: AutoTraderOutcome[] = [];
  private ledgerTradeSignature = '';
  private ledgerRowsBuiltAt = 0;
  private slippageTradeSignature = '';
  private slippageBuiltAt = 0;
  private adverseSlippageBySymbol = new Map<string, number[]>();
  private recentLedgerTrades: PositionTrade[] = [];
  private olderLedgerTrades: PositionTrade[] = [];
  private recentLedgerLoadedAt = 0;
  private recentLedgerLoaded = false;
  private ledgerDayStart = 0;
  private todayTradeOffset?: number;
  private todayLoadedOffset = 0;
  private olderTradeOffset?: number;
  private olderHistoryExhausted = false;
  private historyCacheLoaded = false;
  private auditCursor?: number | null;
  start() {
    this.consumers += 1;
    if (this.consumers > 1) return;
    this.positionSnapshot = currentPositionSnapshot();
    this.stopPositionStream = subscribePositionStream((snapshot) => { this.positionSnapshot = snapshot; this.scheduleRefresh(); });
    this.autoTraderSnapshot = currentAutoTraderSnapshot();
    this.stopAutoTraderStream = subscribeAutoTraderStream((snapshot) => { this.autoTraderSnapshot = snapshot; this.scheduleRefresh(); });
    void this.refresh();
    this.timer = setInterval(() => void this.refresh(), 30_000);
  }
  stop() {
    this.consumers = Math.max(0, this.consumers - 1);
    if (this.consumers > 0) return;
    if (this.timer) clearInterval(this.timer);
    this.timer = undefined;
    this.stopPositionStream?.();
    this.stopPositionStream = undefined;
    this.stopAutoTraderStream?.();
    this.stopAutoTraderStream = undefined;
    if (this.refreshTimer) clearTimeout(this.refreshTimer);
    this.refreshTimer = undefined;
  }
  private scheduleRefresh() {
    if (this.refreshTimer) clearTimeout(this.refreshTimer);
    this.refreshTimer = setTimeout(() => { this.refreshTimer = undefined; void this.refresh(); }, 100);
  }
  requestClose(symbol: string) {
    const current = this.state$.value;
    if (current.closingSymbols.has(symbol)) return;
    this.state$.next({ ...current, confirmingCloseSymbols: new Set(current.confirmingCloseSymbols).add(symbol), error: undefined });
  }
  cancelClose(symbol: string) {
    const current = this.state$.value;
    if (!current.confirmingCloseSymbols.has(symbol)) return;
    const confirmingCloseSymbols = new Set(current.confirmingCloseSymbols);
    confirmingCloseSymbols.delete(symbol);
    this.state$.next({ ...current, confirmingCloseSymbols });
  }
  async confirmClose(symbol: string) {
    const current = this.state$.value;
    if (current.closingSymbols.has(symbol) || !current.confirmingCloseSymbols.has(symbol)) return;
    const confirmingCloseSymbols = new Set(current.confirmingCloseSymbols);
    confirmingCloseSymbols.delete(symbol);
    this.state$.next({ ...current, confirmingCloseSymbols, closingSymbols: new Set(current.closingSymbols).add(symbol), error: undefined });
    try {
      const response = await fetch('/positions/close-profitable', {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ symbol }),
      });
      if (!response.ok) {
        const detail = (await response.text()).trim();
        throw new Error(detail || ('Close IOC failed (HTTP ' + response.status + (response.statusText ? ' ' + response.statusText : '') + ')'));
      }
      await this.refresh(true);
    } catch (reason) {
      this.state$.next({ ...this.state$.value, error: reason instanceof Error ? reason.message : 'Could not close position' });
    } finally {
      const latest = this.state$.value;
      const closingSymbols = new Set(latest.closingSymbols);
      closingSymbols.delete(symbol);
      this.state$.next({ ...latest, closingSymbols });
    }
  }
  private async refresh(forceHttpPositionSnapshot = false) {
    if (this.refreshing) { this.scheduleRefresh(); return; }
    this.refreshing = true;
    try {
      const streamSnapshot = forceHttpPositionSnapshot ? undefined : this.positionSnapshot;
      const autoSnapshot = this.autoTraderSnapshot;
      const refreshGateAudit = Date.now() - this.gateAuditCache.refreshedAt >= 30_000;
      const now = new Date();
      const todayStart = Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate());
      const dayChanged = this.ledgerDayStart !== 0 && this.ledgerDayStart !== todayStart;
      if (dayChanged) { this.todayTradeOffset = undefined; this.todayLoadedOffset = 0; this.olderTradeOffset = undefined; this.recentLedgerLoaded = false; this.historyCacheLoaded = false; }
      this.ledgerDayStart = todayStart;
      if (!this.historyCacheLoaded) { this.restoreLedgerHistory(todayStart); this.historyCacheLoaded = true; }
      const refreshRecentTrades = !this.recentLedgerLoaded || Date.now() - this.recentLedgerLoadedAt >= 30_000;
      const [auditResponse, activityResponse, learningResponse, datedCarryResponse, positionsResponse, tradesResponse, gateAuditResponse, parametersResponse, ledgerRealizedResponse, usdRatesResponse] = await Promise.all([
        autoSnapshot ? Promise.resolve(undefined) : fetch('/signals/auto-trader-execution-audit?limit=500'),
        autoSnapshot ? Promise.resolve(undefined) : fetch('/positions/auto-trader/activity'),
        autoSnapshot ? Promise.resolve(undefined) : fetch('/positions/auto-trader'),
        autoSnapshot ? Promise.resolve(undefined) : fetch('/positions/auto-trader/dated-carry'),
        streamSnapshot ? Promise.resolve(undefined) : fetch('/positions?source=auto'),
        refreshRecentTrades ? fetch('/positions/trades?source=auto&since_ms=' + todayStart + '&limit=500') : Promise.resolve(undefined),
        refreshGateAudit ? fetch('/signals/auto-trader-audit?limit=100').catch(() => undefined) : Promise.resolve(undefined),
        autoSnapshot ? Promise.resolve(undefined) : fetch('/positions/auto-trader/parameters').catch(() => undefined),
        fetch('/positions/realized-pnl?source=auto&since_ms=0').catch(() => undefined),
        fetch('/positions/usd-rates').catch(() => undefined),
      ]);
      if (!autoSnapshot && !auditResponse?.ok) throw new Error('Audit unavailable');
      if (!streamSnapshot && !positionsResponse?.ok) throw new Error('AutoTrader positions unavailable');
      if (refreshRecentTrades && !tradesResponse?.ok) throw new Error('AutoTrader fills unavailable');
      const auditPage = autoSnapshot?.execution_audit ?? await auditResponse!.json() as { items?: AuditRecord[]; nextCursor?: number | null };
      const isInitialAuditPage = this.auditRows.length === 0;
      if (isInitialAuditPage) this.auditCursor = auditPage.nextCursor ?? null;
      const mergedAuditRows = new Map<string, AuditRecord>();
      for (const record of [...this.auditRows, ...(auditPage.items ?? [])]) {
        mergedAuditRows.set(`${record.decision_id ?? ''}:${record.stage ?? ''}`, record);
      }
      this.auditRows = [...mergedAuditRows.values()].sort((left, right) => (left.timestamp ?? 0) - (right.timestamp ?? 0));
      if (gateAuditResponse?.ok) {
        try {
          const gateRecords = await gateAuditResponse.json() as AuditRecord[];
          if (Array.isArray(gateRecords)) this.gateAuditCache = { records: gateRecords, refreshedAt: Date.now() };
        } catch { /* Keep the most recent successful gate audit. */ }
      }
      const positionsData = streamSnapshot?.auto_positions ?? await positionsResponse!.json() as PositionsResponse;
      const ledgerRealizedData = ledgerRealizedResponse?.ok ? await ledgerRealizedResponse.json() as LedgerRealizedResponse : undefined;
      if (tradesResponse?.ok) {
        const page = await tradesResponse.json() as TradesResponse;
        this.recentLedgerTrades = page.trades ?? [];
        this.recentLedgerLoadedAt = Date.now();
        this.recentLedgerLoaded = true;
        if (this.todayLoadedOffset === 0) this.todayTradeOffset = page.next_offset ?? undefined;
        this.state$.next({ ...this.state$.value, hasOlderLedgerTrades: this.todayTradeOffset !== undefined || (!this.olderHistoryExhausted && page.has_older === true) });
        this.persistLedgerHistory();
      }
      const ledgerTradeMap = new Map<string, PositionTrade>();
      for (const trade of [...this.olderLedgerTrades, ...this.recentLedgerTrades]) ledgerTradeMap.set(trade.id, trade);
      const tradesData: TradesResponse = { trades: [...ledgerTradeMap.values()], has_older: this.state$.value.hasOlderLedgerTrades };
      const activityData = autoSnapshot?.activity as ActivityResponse ?? (activityResponse?.ok ? await activityResponse.json() as ActivityResponse : {});
      const streamedRates = streamSnapshot?.usd_rates?.rates;
      const usdRatesData = streamedRates && Object.keys(streamedRates).length > 0
        ? streamSnapshot!.usd_rates!
        : (usdRatesResponse?.ok ? await usdRatesResponse.json() as UsdRatesResponse : {});
      const learningData = autoSnapshot?.status as AutoTraderLearningResponse ?? (learningResponse?.ok ? await learningResponse.json() as AutoTraderLearningResponse : {});
      const parametersData = autoSnapshot?.parameters as DecisionParametersResponse ?? (parametersResponse?.ok ? await parametersResponse.json() as DecisionParametersResponse : {});
      const strictBaseStrengthFloor = parametersData.thresholds?.find((parameter) => parameter.section === 'Signal qualification' && parameter.name === 'Base strict strength floor')?.value;
      const strictEntryGates: Record<string, StrictEntryGateSummary> = {};
      const recentGateCutoff = Date.now() - 10 * 60_000;
      for (const record of this.gateAuditCache.records) {
        const gate = record.entry_gate;
        const strategy = record.strategy;
        const strength = gate?.observed?.strength ?? record.strength;
        const floor = gate?.thresholds?.strength_floor;
        const observedAt = record.signal_timestamp ?? record.timestamp ?? 0;
        if (record.stage !== 'decision' || !strategy || !gate || !Number.isFinite(strength) || !Number.isFinite(floor) || observedAt < recentGateCutoff) continue;
        const candidate: StrictEntryGateSummary = {
          symbol: record.symbol ?? '—',
          direction: record.direction ?? '—',
          strength: strength!,
          floor: floor!,
          gap: Math.max(0, floor! - strength!),
          score: gate.observed?.score ?? record.score,
          observedAt,
          strictWalkForwardPassed: gate.checks?.strict_walk_forward_pass,
          rejectionReasons: gate.rejection_reasons ?? [],
        };
        const previous = strictEntryGates[strategy];
        const closeness = floor! > 0 ? strength! / floor! : strength!;
        const previousCloseness = previous && previous.floor > 0 ? previous.strength / previous.floor : previous?.strength ?? -1;
        if (!previous || closeness > previousCloseness || (closeness === previousCloseness && observedAt > previous.observedAt)) {
          strictEntryGates[strategy] = candidate;
        }
      }
      const datedCarryDiagnostics = autoSnapshot?.dated_carry as DatedCarryDiagnostics ?? (datedCarryResponse?.ok ? await datedCarryResponse.json() as DatedCarryDiagnostics : undefined);
      const strategyLearning = Object.fromEntries([...(learningData.daily_strategy_budgets ?? []), ...(learningData.intraday_strategy_budgets ?? [])].map((item) => [item.strategy, item]));
      const usdRates = usdRatesData.rates ?? {};
      const todayStrategyRealizedUsd: Record<string, number | null> | undefined = Array.isArray(activityData.pnl_breakdown)
        ? Object.create(null) as Record<string, number | null>
        : undefined;
      for (const item of activityData.pnl_breakdown ?? []) {
        const quote = quoteCurrency(item.symbol);
        const rate = quote === 'USD' ? 1 : usdRates[quote];
        const prior = todayStrategyRealizedUsd && Object.prototype.hasOwnProperty.call(todayStrategyRealizedUsd, item.strategy)
          ? todayStrategyRealizedUsd[item.strategy]
          : 0;
        if (todayStrategyRealizedUsd && (typeof rate !== 'number' || !Number.isFinite(rate))) todayStrategyRealizedUsd[item.strategy] = null;
        else if (todayStrategyRealizedUsd && prior !== null) todayStrategyRealizedUsd[item.strategy] = prior + item.realized_pnl * rate!;
      }
      if (todayStrategyRealizedUsd && activityData.realized_pnl_by_quote) {
        const ledgerTotalUsd = Object.entries(activityData.realized_pnl_by_quote).reduce<number | undefined>((sum, [quote, amount]) => {
          const rate = quote === 'USD' ? 1 : usdRates[quote];
          return sum === undefined || typeof rate !== 'number' || !Number.isFinite(rate) ? undefined : sum + amount * rate;
        }, 0);
        if (ledgerTotalUsd !== undefined) {
          const attributedTotalUsd = Object.values(todayStrategyRealizedUsd).reduce<number>((sum, amount) => sum + (amount ?? 0), 0);
          const unattributedUsd = ledgerTotalUsd - attributedTotalUsd;
          if (Math.abs(unattributedUsd) >= 0.0005) todayStrategyRealizedUsd['unattributed'] = unattributedUsd;
        }
      }
      const exitPolicies = new Map((activityData.position_exit_policies ?? []).map((policy) => [policy.symbol, policy]));
      const openPositionAttributions = new Map((activityData.open_positions ?? []).map((position) => [position.symbol, position]));
      const slippageTrades = tradesData.trades ?? [];
      const slippageSignature = slippageTrades.length + ':' + (slippageTrades[0]?.id ?? '') + ':' + (slippageTrades[slippageTrades.length - 1]?.id ?? '');
      if (slippageSignature !== this.slippageTradeSignature && (!this.slippageTradeSignature || Date.now() - this.slippageBuiltAt >= 15_000)) {
        this.adverseSlippageBySymbol.clear();
        for (const trade of slippageTrades) {
          if (typeof trade.slippage_bps !== 'number' || !Number.isFinite(trade.slippage_bps)) continue;
          const values = this.adverseSlippageBySymbol.get(trade.symbol) ?? [];
          values.push(Math.max(0, -trade.slippage_bps));
          this.adverseSlippageBySymbol.set(trade.symbol, values);
        }
        this.slippageTradeSignature = slippageSignature;
        this.slippageBuiltAt = Date.now();
      }
      const adverseSlippageBySymbol = this.adverseSlippageBySymbol;
      const trades = new Map<string, TradeAudit>();
      for (const record of this.auditRows) {
        if (!record.decision_id) continue;
        const trade = trades.get(record.decision_id) ?? { exits: [] };
        if (record.stage === 'entry') trade.entry = record;
        else if (record.stage?.startsWith('exit')) trade.exits.push(record);
        trades.set(record.decision_id, trade);
      }
      const openDecisionIds = new Set([
        ...(activityData.strategy_positions ?? [])
          .filter((position) => Math.abs(position.quantity) > 1e-12)
          .map((position) => position.decision_id)
          .filter((decisionId): decisionId is string => typeof decisionId === 'string' && decisionId.length > 0),
        ...(activityData.open_positions ?? [])
          .map((position) => position.decision_id)
          .filter((decisionId): decisionId is string => typeof decisionId === 'string' && decisionId.length > 0),
      ]);
      const rows: AutoTraderOutcome[] = [...trades.entries()].flatMap(([decisionId, trade]) => {
        const entry = trade.entry;
        if (!entry || !trade.exits.length) return [];
        const canonicalExits = this.canonicalExits(trade.exits);
        const exited = canonicalExits.reduce((sum, row) => sum + (row.exit_filled_quantity ?? 0), 0);
        const entryOrderFilledQuantity = (activityData.orders ?? [])
          .filter((order) => order.decision_id === decisionId && ['signal', 'scout'].includes(order.purpose ?? ''))
          .reduce((sum, order) => sum + Math.max(0, order.filled_quantity ?? 0), 0);
        const latestEntryContext = [...canonicalExits].reverse().find((row) => row.entry_context);
        const entryQuantity = Math.max(entryOrderFilledQuantity, entry.filled_quantity ?? 0, ...canonicalExits.map((row) => row.entry_context?.quantity ?? 0));
        const entryPrice = latestEntryContext?.entry_context?.average_price || entry.average_price;
        if (!(entryQuantity > 0 && typeof entryPrice === 'number' && entryPrice > 0) || exited <= 1e-12) return [];
        const stillOpen = openDecisionIds.has(decisionId);
        const fullyClosed = !stillOpen && exited + 1e-12 >= entryQuantity;
        if (!fullyClosed && !stillOpen) return [];
        const symbol = entry.symbol ?? canonicalExits.find((row) => row.symbol)?.symbol ?? '—';
        const entryOrderTypes = [...new Set((activityData.orders ?? [])
          .filter((order) => order.decision_id === decisionId && ['signal', 'scout'].includes(order.purpose ?? '') && order.filled_quantity > 1e-12)
          .map((order) => (order.order_type ?? '').split(' · ')[0]).filter(Boolean))];
        const scoutEntry = (activityData.orders ?? []).some((order) => order.decision_id === decisionId && order.purpose === 'scout');
        const pnl = canonicalExits.reduce((sum, row) => sum + (row.realized_pnl ?? 0), 0);
        const auditedEntryFees = entry.fees ?? entry.entry_fees_paid;
        const contextEntryFees = canonicalExits.reduce((sum, row) => {
          const contextQuantity = row.entry_context?.quantity ?? 0;
          const exitedQuantity = row.exit_filled_quantity ?? 0;
          const contextFees = row.entry_context?.fees ?? 0;
          return contextQuantity > 0
            ? sum + Math.abs(contextFees) * Math.min(exitedQuantity, contextQuantity) / contextQuantity
            : sum;
        }, 0);
        const cumulativeFees = latestEntryContext?.entry_context?.total_fees_paid;
        const cumulativeEntryFees = latestEntryContext?.entry_context?.total_entry_fees_paid;
        const entryFees = (cumulativeEntryFees ?? auditedEntryFees ?? 0) > 0
          ? (cumulativeEntryFees ?? auditedEntryFees!)
          : contextEntryFees;
        const exitFees = canonicalExits.map((row) => row.exit_fees);
        const fees = typeof cumulativeFees === 'number' && Number.isFinite(cumulativeFees)
          ? cumulativeFees
          : typeof entryFees === 'number' && Number.isFinite(entryFees)
          && exitFees.every((value): value is number => typeof value === 'number' && Number.isFinite(value))
          ? Math.abs(entryFees) + exitFees.reduce((sum, value) => sum + Math.abs(value), 0)
          : undefined;
        const quote = quoteCurrency(symbol);
        const quoteUsdRate = quote === 'USD' ? 1 : usdRates[quote];
        const pnlUsd = typeof quoteUsdRate === 'number' && Number.isFinite(quoteUsdRate) ? pnl * quoteUsdRate : undefined;
        const feesUsd = fees !== undefined && typeof quoteUsdRate === 'number' && Number.isFinite(quoteUsdRate) ? fees * quoteUsdRate : undefined;
        const initialEntryNotional = latestEntryContext?.entry_context?.initial_entry_notional;
        const entryNotionalUsd = typeof quoteUsdRate === 'number' && Number.isFinite(quoteUsdRate)
          ? (typeof initialEntryNotional === 'number' && initialEntryNotional > 0
            ? initialEntryNotional * quoteUsdRate
            : entryQuantity * (entryPrice ?? 0) * quoteUsdRate)
          : undefined;
        const netPnlUsd = pnlUsd;
        return [{ rowId: (fullyClosed ? 'closed:' : 'partial:') + decisionId, status: fullyClosed ? 'Closed' : 'Partially closed', decisionId, symbol, side: entry.side ?? '—', openedAt: entry.timestamp ?? 0,
          closedAt: fullyClosed ? Math.max(...canonicalExits.map((row) => row.timestamp ?? 0)) : undefined,
          quantity: fullyClosed ? entryQuantity : exited,
          remainingQuantity: fullyClosed ? undefined : Math.max(0, entryQuantity - exited),
          signedQuantity: entry.side === 'sell' ? -(fullyClosed ? entryQuantity : exited) : (fullyClosed ? entryQuantity : exited),
          entryPrice, pnl, entryFees: 0, estimatedExitFeeRate: 0, estimatedExitSlippageBps: 0, feesUsd, netPnlUsd, entryNotionalUsd, entryOrderTypes,
          pnlQuoteCurrency: quote, quoteUsdRate, strategy: entry.strategy ?? 'live-insights', direction: entry.direction ?? '—', strength: entry.strength ?? null,
          scout: scoutEntry || entry.scout === true,
          source: scoutEntry || entry.scout ? 'scout' : entry.exploratory === true ? 'exploratory' : entry.exploratory === false ? 'strict' : 'unknown',
          components: entry.signal_components ?? {} }];
      });
      const ledgerTrades = tradesData.trades ?? [];
      const ordersForLedger = activityData.orders ?? [];
      const ledgerSignature = ledgerTrades.length + ':' + (ledgerTrades[0]?.id ?? '') + ':' + (ledgerTrades[ledgerTrades.length - 1]?.id ?? '') + ':' + this.auditRows.length + ':' + ordersForLedger.length + ':' + (ordersForLedger[0]?.timestamp ?? 0);
      if (ledgerSignature !== this.ledgerTradeSignature && (!this.ledgerTradeSignature || Date.now() - this.ledgerRowsBuiltAt >= 15_000)) {
        // Build this large history once per ledger/order change.
      const orderByLedgerId = new Map<string, StrategyOrder>();
      for (const order of ordersForLedger) {
        orderByLedgerId.set(String(order.timestamp), order);
        if (order.execution_order_id) orderByLedgerId.set(order.execution_order_id, order);
      }
      for (const record of this.auditRows) {
        if (!record.decision_id || !record.strategy) continue;
        const auditOrder: StrategyOrder = { timestamp: record.timestamp ?? 0, symbol: record.symbol ?? '', side: '', order_type: record.exit_order_type, purpose: 'stop-loss', strategy: record.strategy, decision_id: record.decision_id, filled_quantity: 0 };
        if (record.barrier_order_id && !orderByLedgerId.has(record.barrier_order_id)) orderByLedgerId.set(record.barrier_order_id, auditOrder);
        for (const fill of record.exit_fills ?? []) if (fill.parent_order_id) orderByLedgerId.set(fill.parent_order_id, auditOrder);
      }
      const instanceGroups = new Map<string, { strategy: string; instanceId: string; symbol: string; trades: PositionTrade[]; quoteRate?: number }>();
      const legacyTradesBySymbol = new Map<string, PositionTrade[]>();
      for (const trade of ledgerTrades) {
        if (trade.source && trade.source !== 'auto') continue;
        const order = trade.parent_order_id ? orderByLedgerId.get(trade.parent_order_id) : undefined;
        if (!order?.decision_id) {
          const symbolTrades = legacyTradesBySymbol.get(trade.symbol) ?? [];
          symbolTrades.push(trade);
          legacyTradesBySymbol.set(trade.symbol, symbolTrades);
          continue;
        }
        const strategy = order.strategy ?? 'legacy-unknown';
        const instanceId = order?.decision_id ?? ('ledger:' + (trade.parent_order_id ?? trade.id));
        const instanceKey = strategy + ':' + instanceId;
        const groupKey = instanceKey + ':' + trade.symbol;
        const group = instanceGroups.get(groupKey) ?? { strategy, instanceId, symbol: trade.symbol, trades: [], quoteRate: quoteCurrency(trade.symbol) === 'USD' ? 1 : usdRates[quoteCurrency(trade.symbol)] };
        group.trades.push(trade);
        instanceGroups.set(groupKey, group);
      }
      const openPositionKeys = new Set((activityData.strategy_positions ?? []).filter((position) => Math.abs(position.quantity) > 1e-12).map((position) => (position.strategy + ':' + (position.decision_id ?? '') + ':' + position.symbol)));
      const ledgerOutcomeRows: AutoTraderOutcome[] = [];
      for (const [symbol, symbolTrades] of legacyTradesBySymbol) {
        const quote = quoteCurrency(symbol);
        const rate = quote === 'USD' ? 1 : usdRates[quote];
        ledgerOutcomeRows.push(...buildLegacyLedgerOutcomes(symbol, symbolTrades, quote, rate, ledgerRealizedData?.realized_pnl_by_parent_order ?? {}, new Map([...orderByLedgerId].map(([id, order]) => [id, order.order_type ?? '']))));
      }
      for (const [groupKey, group] of instanceGroups) {
        const status = openPositionKeys.has(group.strategy + ':' + group.instanceId + ':' + group.symbol) ? 'Partially closed' : 'Closed';
        const orderedTrades = [...group.trades].sort((a, b) => a.timestamp - b.timestamp);
        const realizedParents = new Set<string>();
        let realizedQuote = 0;
        for (const trade of orderedTrades) {
          const parentId = trade.parent_order_id;
          if (!parentId || realizedParents.has(parentId)) continue;
          realizedParents.add(parentId);
          realizedQuote += ledgerRealizedData?.realized_pnl_by_parent_order?.[parentId] ?? 0;
        }
        const firstTrade = orderedTrades[0];
        const lastTrade = orderedTrades[orderedTrades.length - 1];
        const netQuantity = orderedTrades.reduce((sum, trade) => sum + (trade.side === 'buy' ? trade.quantity : -trade.quantity), 0);
        const tradedNotional = orderedTrades.reduce((sum, trade) => sum + Math.abs(trade.quantity * trade.execution_price), 0);
        const feesPaid = orderedTrades.reduce((sum, trade) => sum + Math.abs(trade.fee ?? 0), 0);
        const entryAudit = trades.get(group.instanceId)?.entry;
        const positionSource = entryAudit?.scout ? 'scout' : entryAudit?.exploratory === true ? 'exploratory' : entryAudit?.exploratory === false ? 'strict' : 'unknown';
        ledgerOutcomeRows.push({ rowId: 'ledger-summary:' + groupKey, status, decisionId: group.instanceId, strategyInstanceId: group.instanceId,
          symbol: group.symbol, side: netQuantity >= 0 ? 'buy' : 'sell', openedAt: firstTrade?.timestamp ?? 0, closedAt: status === 'Closed' ? lastTrade?.timestamp : undefined,
          quantity: Math.abs(netQuantity), remainingQuantity: status === 'Partially closed' ? Math.abs(netQuantity) : undefined, signedQuantity: netQuantity,
          entryPrice: firstTrade?.execution_price ?? 0, pnl: 0, entryFees: 0, estimatedExitFeeRate: 0, estimatedExitSlippageBps: 0,
          // A round trip has two traded legs; normalize the combined fees to
          // one average leg's notional so maker + taker costs are both shown.
          feePercent: tradedNotional > 0 ? feesPaid / (tradedNotional / 2) * 100 : undefined,
          netPnlUsd: ledgerRealizedData && typeof group.quoteRate === 'number' ? realizedQuote * group.quoteRate : undefined,
          pnlQuoteCurrency: quoteCurrency(group.symbol), quoteUsdRate: group.quoteRate, strategy: group.strategy, direction: netQuantity >= 0 ? 'LONG' : 'SHORT', strength: null,
          scout: positionSource === 'scout', source: positionSource, components: entryAudit?.signal_components ?? {} });
        for (const trade of orderedTrades) {
          const order = trade.parent_order_id ? orderByLedgerId.get(trade.parent_order_id) : undefined;
          ledgerOutcomeRows.push({ rowId: 'ledger-trade:' + trade.id, status, decisionId: group.instanceId, strategyInstanceId: group.instanceId,
            ledgerTrade: trade, ledgerSource: trade.source ?? 'auto', ledgerOrderType: order?.order_type, symbol: trade.symbol, side: trade.side, openedAt: trade.timestamp, closedAt: trade.timestamp,
            quantity: trade.quantity, signedQuantity: trade.side === 'buy' ? trade.quantity : -trade.quantity, entryPrice: trade.execution_price,
            pnl: 0, entryFees: trade.fee ?? 0, estimatedExitFeeRate: trade.fee_rate ?? 0, estimatedExitSlippageBps: 0,
            feePercent: Math.abs(trade.quantity * trade.execution_price) > 0 ? Math.abs(trade.fee ?? 0) / Math.abs(trade.quantity * trade.execution_price) * 100 : trade.fee_rate !== undefined ? trade.fee_rate * 100 : undefined,
            pnlQuoteCurrency: quoteCurrency(trade.symbol), quoteUsdRate: group.quoteRate, strategy: group.strategy, direction: trade.side.toUpperCase(), strength: null,
            scout: order?.purpose === 'scout' || entryAudit?.scout === true, source: positionSource, components: entryAudit?.signal_components ?? {} });
        }
      }
      const groupTimestamp = new Map([...instanceGroups].map(([key, group]) => [key, Math.min(...group.trades.map((trade) => trade.timestamp))]));
      const ledgerGroupKey = (row: AutoTraderOutcome) => row.rowId.startsWith('ledger-summary:')
        ? row.rowId.slice('ledger-summary:'.length)
        : row.strategy + ':' + (row.strategyInstanceId ?? '') + ':' + row.symbol;
      ledgerOutcomeRows.sort((a, b) => (groupTimestamp.get(ledgerGroupKey(b)) ?? b.openedAt) - (groupTimestamp.get(ledgerGroupKey(a)) ?? a.openedAt)
        || Number(!a.rowId.startsWith('ledger-summary:')) - Number(!b.rowId.startsWith('ledger-summary:'))
        || a.openedAt - b.openedAt);
      this.ledgerRowsCache = ledgerOutcomeRows;
      this.ledgerTradeSignature = ledgerSignature;
      this.ledgerRowsBuiltAt = Date.now();
      }
      const aggregatePositions = new Map((positionsData.positions ?? []).map((position) => [position.symbol, position]));
      const strategyLots = (activityData.strategy_positions ?? []).filter((position) => Math.abs(position.quantity) > 1e-12);
      const strategyPositionSymbols = new Set(strategyLots.map((position) => position.symbol));
      const openPositions: OpenPositionLot[] = [
        ...strategyLots.map((position) => ({
            symbol: position.symbol,
            quantity: position.quantity,
            average_entry_price: position.average_entry_price,
            entry_fees: position.entry_fees,
            estimated_exit_fee_rate: aggregatePositions.get(position.symbol)?.estimated_exit_fee_rate ?? 0,
            last_trade_timestamp: position.entry_timestamp_ms,
            strategy: position.strategy,
            decision_id: position.decision_id,
            entry_order_type: position.entry_order_type,
            entry_order_types: position.entry_order_types,
            stop_loss_percent: position.stop_loss_percent,
            take_profit_percent: position.take_profit_percent ?? position.horizon_take_profit_percent,
            stop_loss_active: position.stop_loss_active,
            take_profit_active: position.take_profit_active,
            carry_details: position.carry_details,
          })),
        ...(positionsData.positions ?? [])
            .filter((position) => Math.abs(position.quantity) > 1e-12 && !strategyPositionSymbols.has(position.symbol))
            .map((position) => ({
              ...position,
              strategy: openPositionAttributions.get(position.symbol)?.strategy,
              decision_id: openPositionAttributions.get(position.symbol)?.decision_id,
              entry_order_type: undefined,
            })),
      ];
      for (const position of openPositions) {
        const carryDetails = position.carry_details;
        const side = carryDetails ? 'paired' : position.quantity > 0 ? 'buy' : 'sell';
        const entry = position.decision_id ? trades.get(position.decision_id)?.entry : undefined;
        const latestFilledOrder = (activityData.orders ?? [])
          .filter((order) => order.symbol === position.symbol
            && order.side === side
            && (!position.strategy || order.strategy === position.strategy)
            && order.timestamp >= position.last_trade_timestamp
            && (order.purpose === 'signal' || order.purpose === 'scout')
            && order.filled_quantity > 1e-12)
          .sort((a, b) => b.timestamp - a.timestamp)[0];
        const filledEntryOrderTypes = (activityData.orders ?? [])
          .filter((order) => order.symbol === position.symbol
            && order.side === side
            && (!position.strategy || order.strategy === position.strategy)
            && order.timestamp >= position.last_trade_timestamp
            && (order.purpose === 'signal' || order.purpose === 'scout')
            && order.filled_quantity > 1e-12)
          .map((order) => (order.order_type ?? '').split(' · ')[0])
          .filter(Boolean);
        const symbol = carryDetails
          ? carryDetails.spot_symbol.replace(/(USDT|USDC)$/, '-$1')
          : position.symbol;
        const adverseSlippage = adverseSlippageBySymbol.get(symbol) ?? [];
        const estimatedExitSlippageBps = adverseSlippage.length ? adverseSlippage.reduce((sum, value) => sum + value, 0) / adverseSlippage.length : 0;
        const entryOrder = position.decision_id
          ? (activityData.orders ?? []).find((order) => order.decision_id === position.decision_id)
          : latestFilledOrder;
        const isScout = entryOrder?.purpose === 'scout' || latestFilledOrder?.purpose === 'scout' || entry?.scout === true;
        const decisionId = position.decision_id ?? latestFilledOrder?.decision_id ?? 'ledger:' + symbol + ':' + (position.strategy ?? 'unknown');
        const strategyLabel = position.strategy ?? entry?.strategy ?? entryOrder?.strategy ?? 'legacy-unknown';
        const exitPolicy = exitPolicies.get(symbol);
        const rowId = position.strategy ? 'open:' + symbol + ':' + position.strategy : 'open:' + symbol;
        rows.push({ rowId, status: 'Open', decisionId, symbol, side, openedAt: entry?.timestamp ?? latestFilledOrder?.timestamp ?? position.last_trade_timestamp,
          quantity: Math.abs(position.quantity), signedQuantity: position.quantity, entryPrice: carryDetails?.spot_entry_price ?? position.average_entry_price, pnl: 0,
          entryFees: position.entry_fees, estimatedExitFeeRate: position.estimated_exit_fee_rate, estimatedExitSlippageBps,
          entryOrderType: position.entry_order_type ?? latestFilledOrder?.order_type,
          entryOrderTypes: [...new Set([
            ...(position.entry_order_types ?? []),
            ...(!position.entry_order_types?.length && position.entry_order_type ? [position.entry_order_type.split(' · ')[0]] : []),
            ...filledEntryOrderTypes,
          ])],
          pnlQuoteCurrency: carryDetails ? 'USDT' : quoteCurrency(symbol), quoteUsdRate: carryDetails ? usdRates.USDT ?? 1 : quoteCurrency(symbol) === 'USD' ? 1 : usdRates[quoteCurrency(symbol)], strategy: strategyLabel, direction: carryDetails ? 'Spot long / future short' : entry?.direction ?? (side === 'buy' ? 'bullish' : 'bearish'),
          stopLossPercent: position.stop_loss_active === false ? undefined : position.stop_loss_percent ?? exitPolicy?.stop_loss_percent, takeProfitPercent: position.take_profit_active === false ? undefined : position.take_profit_percent ?? exitPolicy?.take_profit_percent,
          stopLossActive: position.stop_loss_active, takeProfitActive: position.take_profit_active,
          strength: entry?.strength ?? null, scout: isScout,
          source: carryDetails ? 'paper-carry' : isScout ? 'scout' : entry?.exploratory === true ? 'exploratory' : entry?.exploratory === false ? 'strict' : 'unknown',
          carryDetails,
          components: entry?.signal_components ?? {} });
      }
      rows.sort((a, b) => (a.status === b.status ? b.openedAt - a.openedAt : a.status === 'Open' ? -1 : 1));
      this.state$.next({ ...this.state$.value, rows, closedLedgerRows: this.ledgerRowsCache, strategyLearning, autoTraderEnabled: learningData.enabled ?? false, autoTraderStatusAvailable: Boolean(autoSnapshot || learningResponse?.ok), enabledStrategies: learningData.strategies ?? [], strategyDiagnostics: learningData.strategy_diagnostics ?? [], strategyOrders: activityData.orders ?? [], strictEntryGates, strictBaseStrengthFloor, todayStrategyRealizedUsd, datedCarryDiagnostics, openStrategyPositions: strategyLots.length ? strategyLots.map(({ symbol, strategy }) => ({ symbol, strategy })) : activityData.open_positions ?? [], loading: false, auditCursor: this.auditCursor, hasOlderLedgerTrades: this.state$.value.hasOlderLedgerTrades });
    } catch { this.state$.next({ ...this.state$.value, loading: false, error: 'AutoTrader positions or closed trade audit are unavailable.' }); }
    finally { this.refreshing = false; }
  }

  async loadOlderLedgerTrades() {
    const current = this.state$.value;
    if (current.loadingOlderLedgerTrades || !current.hasOlderLedgerTrades) return;
    this.state$.next({ ...current, loadingOlderLedgerTrades: true, error: undefined });
    try {
      let url: string;
      let pageKind: 'today' | 'older';
      if (this.todayTradeOffset !== undefined) {
        pageKind = 'today';
        url = '/positions/trades?source=auto&since_ms=' + this.ledgerDayStart + '&limit=500&offset=' + this.todayTradeOffset;
      } else {
        pageKind = 'older';
        url = '/positions/trades?source=auto&before_ms=' + this.ledgerDayStart + '&limit=500&offset=' + (this.olderTradeOffset ?? 0);
      }
      const [response, auditResponse] = await Promise.all([
        fetch(url),
        this.auditCursor == null ? Promise.resolve(undefined) : fetch('/signals/auto-trader-execution-audit?cursor=' + this.auditCursor + '&limit=500').catch(() => undefined),
      ]);
      if (!response.ok) throw new Error('Could not load older position-ledger trades');
      const page = await response.json() as TradesResponse;
      if (auditResponse?.ok) {
        const auditPage = await auditResponse.json() as { items?: AuditRecord[]; nextCursor?: number | null };
        const auditRows = new Map(this.auditRows.map((record) => [record.decision_id + ':' + record.stage, record]));
        for (const record of auditPage.items ?? []) auditRows.set(record.decision_id + ':' + record.stage, record);
        this.auditRows = [...auditRows.values()].sort((left, right) => (left.timestamp ?? 0) - (right.timestamp ?? 0));
        this.auditCursor = auditPage.nextCursor ?? null;
      }
      const fetched = page.trades ?? [];
      this.olderLedgerTrades = [...this.olderLedgerTrades, ...fetched];
      if (pageKind === 'today') {
        this.todayLoadedOffset += fetched.length;
        this.todayTradeOffset = page.next_offset ?? undefined;
      } else {
        this.olderTradeOffset = page.next_offset ?? undefined;
        if (page.next_offset == null) this.olderHistoryExhausted = true;
      }
      const hasMore = this.todayTradeOffset !== undefined || (!this.olderHistoryExhausted && (pageKind === 'today' ? page.has_older === true : page.next_offset != null));
      this.persistLedgerHistory();
      this.state$.next({ ...this.state$.value, loadingOlderLedgerTrades: false, hasOlderLedgerTrades: hasMore });
      this.ledgerTradeSignature = '';
      this.slippageTradeSignature = '';
      this.scheduleRefresh();
    } catch (reason) {
      this.state$.next({ ...this.state$.value, loadingOlderLedgerTrades: false, error: reason instanceof Error ? reason.message : 'Could not load older position-ledger trades' });
    }
  }
  private ledgerHistoryStorageKey() { return 'autotrader-ledger-history-v1'; }
  private restoreLedgerHistory(dayStart: number) {
    try {
      const stored = localStorage.getItem(this.ledgerHistoryStorageKey());
      if (!stored) return;
      const cache = JSON.parse(stored) as LedgerHistoryCache;
      this.olderLedgerTrades = Array.isArray(cache.trades) ? cache.trades : [];
      const sameDay = cache.dayStart === dayStart;
      this.todayTradeOffset = sameDay ? cache.todayTradeOffset : undefined;
      this.todayLoadedOffset = sameDay ? cache.todayLoadedOffset ?? 0 : 0;
      if (!sameDay) {
        const combined = new Map<string, PositionTrade>();
        for (const trade of [...(cache.recentTrades ?? []), ...this.olderLedgerTrades]) combined.set(trade.id, trade);
        this.olderLedgerTrades = [...combined.values()].sort((a, b) => b.timestamp - a.timestamp);
      }
      this.olderTradeOffset = sameDay ? cache.olderTradeOffset : this.olderLedgerTrades.length;
      this.olderHistoryExhausted = sameDay ? cache.olderHistoryExhausted ?? false : false;
    } catch { /* Ignore unavailable or invalid browser storage. */ }
  }
  private persistLedgerHistory() {
    try {
      const cache: LedgerHistoryCache = {
        dayStart: this.ledgerDayStart,
        trades: this.olderLedgerTrades,
        recentTrades: this.recentLedgerTrades,
        todayTradeOffset: this.todayTradeOffset,
        todayLoadedOffset: this.todayLoadedOffset,
        olderTradeOffset: this.olderTradeOffset,
        olderHistoryExhausted: this.olderHistoryExhausted,
      };
      localStorage.setItem(this.ledgerHistoryStorageKey(), JSON.stringify(cache));
    } catch { /* Keep pagination working if browser storage is full or unavailable. */ }
  }
  async loadOlderAudit() {
    const cursor = this.state$.value.auditCursor;
    if (cursor == null || this.state$.value.loadingOlderAudit) return;
    this.state$.next({ ...this.state$.value, loadingOlderAudit: true });
    try {
      const response = await fetch(`/signals/auto-trader-execution-audit?cursor=${cursor}&limit=500`);
      if (!response.ok) throw new Error('Could not load older execution history');
      const page = await response.json() as { items?: AuditRecord[]; nextCursor?: number | null };
      const rows = new Map(this.auditRows.map((record) => [`${record.decision_id ?? ''}:${record.stage ?? ''}`, record]));
      for (const record of page.items ?? []) rows.set(`${record.decision_id ?? ''}:${record.stage ?? ''}`, record);
      this.auditRows = [...rows.values()].sort((left, right) => (left.timestamp ?? 0) - (right.timestamp ?? 0));
      this.auditCursor = page.nextCursor ?? null;
      this.state$.next({ ...this.state$.value, loadingOlderAudit: false });
      await this.refresh();
    } catch (error) {
      this.state$.next({ ...this.state$.value, loadingOlderAudit: false, error: error instanceof Error ? error.message : 'Could not load older execution history' });
    }
  }
  private canonicalExits(exits: AuditRecord[]) {
    const externalFillIds = new Set(exits
      .filter((row) => row.exit_order_type === 'external-ledger-fill')
      .flatMap((row) => row.exit_fills?.map((fill) => fill.parent_order_id).filter((id): id is string => Boolean(id)) ?? []));
    return exits.filter((row) => !(row.exit_order_type === 'StopMarket' && row.barrier_order_id && externalFillIds.has(row.barrier_order_id)));
  }
}
