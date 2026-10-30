import { BehaviorSubject } from 'rxjs';
import type { Position, PositionTrade } from './PositionTypes';
import { currentPositionSnapshot, subscribePositionStream, type PositionUiSnapshot } from '../position_stream';
import { currentAutoTraderSnapshot, subscribeAutoTraderStream, type AutoTraderUiSnapshot } from '../auto_trader_stream';

export type AutoTraderOutcome = {
  rowId: string; status: 'Open' | 'Partially closed' | 'Closed'; decisionId: string; symbol: string; side: string; openedAt: number; closedAt?: number;
  quantity: number; remainingQuantity?: number; signedQuantity: number; entryPrice: number; pnl: number; entryFees: number; estimatedExitFeeRate: number; estimatedExitSlippageBps: number;
  feesUsd?: number; netPnlUsd?: number;
  entryNotionalUsd?: number;
  pnlQuoteCurrency: string; quoteUsdRate?: number; strategy: string; direction: string; strength: number | null;
  stopLossPercent?: number; takeProfitPercent?: number;
  scout: boolean; source: 'scout' | 'exploratory' | 'strict' | 'unknown'; components: Record<string, number | null>;
};

type AuditRecord = { decision_id?: string; stage?: string; symbol?: string; timestamp?: number; side?: string; direction?: string; strategy?: string; exit_order_type?: string; barrier_order_id?: string; exit_fills?: Array<{ parent_order_id?: string }>; filled_quantity?: number; average_price?: number; exit_filled_quantity?: number; realized_pnl?: number; fees?: number; entry_fees_paid?: number; exit_fees?: number; entry_context?: { quantity?: number; average_price?: number; fees?: number; initial_entry_notional?: number; total_entry_fees_paid?: number; total_fees_paid?: number }; strength?: number; score?: number; scout?: boolean; exploratory?: boolean | null; signal_components?: Record<string, number | null>; signal_timestamp?: number; entry_gate?: { checks?: Record<string, boolean>; observed?: { strength?: number; score?: number }; thresholds?: { strength_floor?: number }; rejection_reasons?: string[] } };
export type StrictEntryGateSummary = { symbol: string; direction: string; strength: number; floor: number; gap: number; score?: number; observedAt: number; strictWalkForwardPassed?: boolean; rejectionReasons: string[] };
type DecisionParametersResponse = { thresholds?: Array<{ section?: string; name?: string; value?: number }> };
type PositionsResponse = { positions?: Position[] };
type TradesResponse = { trades?: PositionTrade[] };
type StrategyLearning = { strategy: string; learning_multiplier: number; learning_samples: number; positive_rate?: number | null };
type StrategySymbolDiagnostic = { symbol: string; eligible: boolean; primary_rule: string; primary_distance_percent: number; primary_metric_value?: number | null; primary_metric_unit?: string | null; primary_passed: boolean; secondary_rule?: string | null; secondary_distance_percent?: number | null; secondary_passed?: boolean | null; signal_active: boolean };
type StrategyDiagnostic = { strategy: string; as_of_ms: number; btc_distance_percent: number | null; btc_window_days: number | null; btc_gate_required: boolean; btc_gate_passed: boolean | null; symbols: StrategySymbolDiagnostic[] };
type StrategyOrder = { timestamp: number; symbol: string; side: string; purpose?: string; strategy?: string; decision_id?: string; filled_quantity: number; average_price?: number | null; status?: string; failure_reason?: string | null; order_type?: string };
type OpenStrategyPosition = { symbol: string; strategy: string; decision_id?: string | null; quantity: number; average_entry_price: number; entry_fees: number; entry_timestamp_ms: number; stop_loss_percent?: number | null; take_profit_percent?: number | null; horizon_take_profit_percent?: number | null };
type OpenPositionLot = { symbol: string; quantity: number; average_entry_price: number; entry_fees: number; estimated_exit_fee_rate: number; last_trade_timestamp: number; strategy?: string; decision_id?: string | null; stop_loss_percent?: number | null; take_profit_percent?: number | null };
type AutoTraderLearningResponse = { enabled?: boolean; strategies?: string[]; strategy_diagnostics?: StrategyDiagnostic[]; daily_strategy_budgets?: StrategyLearning[]; intraday_strategy_budgets?: StrategyLearning[] };
type ActivityResponse = {
  orders?: StrategyOrder[];
  pnl_breakdown?: Array<{ symbol: string; strategy: string; realized_pnl: number }>;
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
  positions?: Array<{ future_symbol: string; status: string; expiry_ms: number; net_pnl_usdt?: number | null }>;
};
type TradeAudit = { entry?: AuditRecord; exits: AuditRecord[] };
const quoteCurrency = (symbol: string) => symbol.includes('-') ? symbol.slice(symbol.lastIndexOf('-') + 1) : 'Unknown';

export class AutoTraderOutcomesModel {
  readonly state$ = new BehaviorSubject<{ rows: AutoTraderOutcome[]; strategyLearning: Record<string, StrategyLearning>; autoTraderEnabled: boolean; autoTraderStatusAvailable: boolean; enabledStrategies: string[]; strategyDiagnostics: StrategyDiagnostic[]; strategyOrders: StrategyOrder[]; strictEntryGates: Record<string, StrictEntryGateSummary>; strictBaseStrengthFloor?: number; todayStrategyRealizedUsd?: Record<string, number | null>; datedCarryDiagnostics?: DatedCarryDiagnostics; openStrategyPositions: Array<{ symbol: string; strategy?: string }>; loading: boolean; error?: string; confirmingCloseSymbols: Set<string>; closingSymbols: Set<string> }>({ rows: [], strategyLearning: {}, autoTraderEnabled: false, autoTraderStatusAvailable: false, enabledStrategies: [], strategyDiagnostics: [], strategyOrders: [], strictEntryGates: {}, strictBaseStrengthFloor: undefined, todayStrategyRealizedUsd: undefined, datedCarryDiagnostics: undefined, openStrategyPositions: [], loading: true, confirmingCloseSymbols: new Set(), closingSymbols: new Set() });
  private timer?: ReturnType<typeof setInterval>;
  private stopPositionStream?: () => void;
  private stopAutoTraderStream?: () => void;
  private refreshTimer?: ReturnType<typeof setTimeout>;
  private positionSnapshot?: PositionUiSnapshot;
  private autoTraderSnapshot?: AutoTraderUiSnapshot;
  private refreshing = false;
  private consumers = 0;
  private gateAuditCache: { records: AuditRecord[]; refreshedAt: number } = { records: [], refreshedAt: 0 };
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
    if (this.refreshing) return;
    this.refreshing = true;
    try {
      const streamSnapshot = forceHttpPositionSnapshot ? undefined : this.positionSnapshot;
      const autoSnapshot = this.autoTraderSnapshot;
      const refreshGateAudit = Date.now() - this.gateAuditCache.refreshedAt >= 30_000;
      const [auditResponse, activityResponse, learningResponse, datedCarryResponse, positionsResponse, tradesResponse, gateAuditResponse, parametersResponse] = await Promise.all([
        autoSnapshot ? Promise.resolve(undefined) : fetch('/signals/auto-trader-execution-audit'),
        autoSnapshot ? Promise.resolve(undefined) : fetch('/positions/auto-trader/activity'),
        autoSnapshot ? Promise.resolve(undefined) : fetch('/positions/auto-trader'),
        autoSnapshot ? Promise.resolve(undefined) : fetch('/positions/auto-trader/dated-carry'),
        streamSnapshot ? Promise.resolve(undefined) : fetch('/positions?source=auto'),
        streamSnapshot ? Promise.resolve(undefined) : fetch('/positions/trades?source=auto'),
        refreshGateAudit ? fetch('/signals/auto-trader-audit?limit=100').catch(() => undefined) : Promise.resolve(undefined),
        autoSnapshot ? Promise.resolve(undefined) : fetch('/positions/auto-trader/parameters').catch(() => undefined),
      ]);
      if (!autoSnapshot && !auditResponse?.ok) throw new Error('Audit unavailable');
      if (!streamSnapshot && !positionsResponse?.ok) throw new Error('AutoTrader positions unavailable');
      if (!streamSnapshot && !tradesResponse?.ok) throw new Error('AutoTrader fills unavailable');
      const records = autoSnapshot?.execution_audit as AuditRecord[] ?? await auditResponse!.json() as AuditRecord[];
      if (gateAuditResponse?.ok) {
        try {
          const gateRecords = await gateAuditResponse.json() as AuditRecord[];
          if (Array.isArray(gateRecords)) this.gateAuditCache = { records: gateRecords, refreshedAt: Date.now() };
        } catch { /* Keep the most recent successful gate audit. */ }
      }
      const positionsData = streamSnapshot?.auto_positions ?? await positionsResponse!.json() as PositionsResponse;
      const tradesData = streamSnapshot?.auto_trades ?? await tradesResponse!.json() as TradesResponse;
      const activityData = autoSnapshot?.activity as ActivityResponse ?? (activityResponse?.ok ? await activityResponse.json() as ActivityResponse : {});
      const usdRatesData = streamSnapshot?.usd_rates ?? {};
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
      const adverseSlippageBySymbol = new Map<string, number[]>();
      for (const trade of tradesData.trades ?? []) {
        if (typeof trade.slippage_bps !== 'number' || !Number.isFinite(trade.slippage_bps)) continue;
        const values = adverseSlippageBySymbol.get(trade.symbol) ?? [];
        values.push(Math.max(0, -trade.slippage_bps));
        adverseSlippageBySymbol.set(trade.symbol, values);
      }
      const trades = new Map<string, TradeAudit>();
      for (const record of records) {
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
          entryPrice, pnl, entryFees: 0, estimatedExitFeeRate: 0, estimatedExitSlippageBps: 0, feesUsd, netPnlUsd, entryNotionalUsd,
          pnlQuoteCurrency: quote, quoteUsdRate, strategy: entry.strategy ?? 'live-insights', direction: entry.direction ?? '—', strength: entry.strength ?? null,
          scout: scoutEntry || entry.scout === true,
          source: scoutEntry || entry.scout ? 'scout' : entry.exploratory === true ? 'exploratory' : entry.exploratory === false ? 'strict' : 'unknown',
          components: entry.signal_components ?? {} }];
      });
      const aggregatePositions = new Map((positionsData.positions ?? []).map((position) => [position.symbol, position]));
      const strategyLots = (activityData.strategy_positions ?? []).filter((position) => Math.abs(position.quantity) > 1e-12);
      const openPositions: OpenPositionLot[] = strategyLots.length
        ? strategyLots.map((position) => ({
            symbol: position.symbol,
            quantity: position.quantity,
            average_entry_price: position.average_entry_price,
            entry_fees: position.entry_fees,
            estimated_exit_fee_rate: aggregatePositions.get(position.symbol)?.estimated_exit_fee_rate ?? 0,
            last_trade_timestamp: position.entry_timestamp_ms,
            strategy: position.strategy,
            decision_id: position.decision_id,
            stop_loss_percent: position.stop_loss_percent,
            take_profit_percent: position.take_profit_percent ?? position.horizon_take_profit_percent,
          }))
        : (positionsData.positions ?? [])
            .filter((position) => Math.abs(position.quantity) > 1e-12)
            .map((position) => ({
              ...position,
              strategy: openPositionAttributions.get(position.symbol)?.strategy,
              decision_id: openPositionAttributions.get(position.symbol)?.decision_id,
            }));
      for (const position of openPositions) {
        const side = position.quantity > 0 ? 'buy' : 'sell';
        const entry = position.decision_id ? trades.get(position.decision_id)?.entry : undefined;
        const latestFilledOrder = (activityData.orders ?? [])
          .filter((order) => order.symbol === position.symbol
            && order.side === side
            && (!position.strategy || order.strategy === position.strategy)
            && (order.purpose === 'signal' || order.purpose === 'scout')
            && order.filled_quantity > 1e-12)
          .sort((a, b) => b.timestamp - a.timestamp)[0];
        const symbol = position.symbol;
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
          quantity: Math.abs(position.quantity), signedQuantity: position.quantity, entryPrice: position.average_entry_price, pnl: 0,
          entryFees: position.entry_fees, estimatedExitFeeRate: position.estimated_exit_fee_rate, estimatedExitSlippageBps,
          pnlQuoteCurrency: quoteCurrency(symbol), quoteUsdRate: quoteCurrency(symbol) === 'USD' ? 1 : usdRates[quoteCurrency(symbol)], strategy: strategyLabel, direction: entry?.direction ?? (side === 'buy' ? 'bullish' : 'bearish'),
          stopLossPercent: position.stop_loss_percent ?? exitPolicy?.stop_loss_percent, takeProfitPercent: position.take_profit_percent ?? exitPolicy?.take_profit_percent,
          strength: entry?.strength ?? null, scout: isScout,
          source: isScout ? 'scout' : entry?.exploratory === true ? 'exploratory' : entry?.exploratory === false ? 'strict' : 'unknown',
          components: entry?.signal_components ?? {} });
      }
      rows.sort((a, b) => (a.status === b.status ? b.openedAt - a.openedAt : a.status === 'Open' ? -1 : 1));
      this.state$.next({ ...this.state$.value, rows, strategyLearning, autoTraderEnabled: learningData.enabled ?? false, autoTraderStatusAvailable: Boolean(autoSnapshot || learningResponse?.ok), enabledStrategies: learningData.strategies ?? [], strategyDiagnostics: learningData.strategy_diagnostics ?? [], strategyOrders: activityData.orders ?? [], strictEntryGates, strictBaseStrengthFloor, todayStrategyRealizedUsd, datedCarryDiagnostics, openStrategyPositions: strategyLots.length ? strategyLots.map(({ symbol, strategy }) => ({ symbol, strategy })) : activityData.open_positions ?? [], loading: false });
    } catch { this.state$.next({ ...this.state$.value, loading: false, error: 'AutoTrader positions or closed trade audit are unavailable.' }); }
    finally { this.refreshing = false; }
  }
  private canonicalExits(exits: AuditRecord[]) {
    const externalFillIds = new Set(exits
      .filter((row) => row.exit_order_type === 'external-ledger-fill')
      .flatMap((row) => row.exit_fills?.map((fill) => fill.parent_order_id).filter((id): id is string => Boolean(id)) ?? []));
    return exits.filter((row) => !(row.exit_order_type === 'StopMarket' && row.barrier_order_id && externalFillIds.has(row.barrier_order_id)));
  }
}
