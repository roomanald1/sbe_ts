import { BehaviorSubject } from 'rxjs';
import type { Position, PositionTrade } from './PositionTypes';

export type AutoTraderOutcome = {
  rowId: string; status: 'Open' | 'Closed'; decisionId: string; symbol: string; side: string; openedAt: number; closedAt?: number;
  quantity: number; signedQuantity: number; entryPrice: number; pnl: number; entryFees: number; estimatedExitFeeRate: number; estimatedExitSlippageBps: number;
  pnlQuoteCurrency: string; strategy: string; direction: string; strength: number | null;
  stopLossPercent?: number; takeProfitPercent?: number;
  scout: boolean; components: Record<string, number | null>;
};

type AuditRecord = { decision_id?: string; stage?: string; symbol?: string; timestamp?: number; side?: string; direction?: string; strategy?: string; exit_order_type?: string; barrier_order_id?: string; exit_fills?: Array<{ parent_order_id?: string }>; filled_quantity?: number; average_price?: number; exit_filled_quantity?: number; realized_pnl?: number; entry_context?: { quantity?: number; average_price?: number }; strength?: number; scout?: boolean; signal_components?: Record<string, number | null> };
type PositionsResponse = { positions?: Position[] };
type TradesResponse = { trades?: PositionTrade[] };
type ActivityResponse = {
  orders?: Array<{ timestamp: number; symbol: string; side: string; purpose?: string; strategy?: string; decision_id?: string; filled_quantity: number }>;
  position_exit_policies?: Array<{ symbol: string; stop_loss_percent: number; take_profit_percent: number }>;
};
type TradeAudit = { entry?: AuditRecord; exits: AuditRecord[] };
const quoteCurrency = (symbol: string) => symbol.includes('-') ? symbol.slice(symbol.lastIndexOf('-') + 1) : 'Unknown';

export class AutoTraderOutcomesModel {
  readonly state$ = new BehaviorSubject<{ rows: AutoTraderOutcome[]; loading: boolean; error?: string; confirmingCloseSymbols: Set<string>; closingSymbols: Set<string> }>({ rows: [], loading: true, confirmingCloseSymbols: new Set(), closingSymbols: new Set() });
  private timer?: ReturnType<typeof setInterval>;
  async start() { if (this.timer) return; await this.refresh(); this.timer = setInterval(() => void this.refresh(), 30_000); }
  stop() { if (this.timer) clearInterval(this.timer); this.timer = undefined; }
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
      await this.refresh();
    } catch (reason) {
      this.state$.next({ ...this.state$.value, error: reason instanceof Error ? reason.message : 'Could not close position' });
    } finally {
      const latest = this.state$.value;
      const closingSymbols = new Set(latest.closingSymbols);
      closingSymbols.delete(symbol);
      this.state$.next({ ...latest, closingSymbols });
    }
  }
  private async refresh() {
    try {
      const [auditResponse, positionsResponse, tradesResponse, activityResponse] = await Promise.all([
        fetch('/signals/auto-trader-execution-audit'),
        fetch('/positions?source=auto'),
        fetch('/positions/trades?source=auto'),
        fetch('/positions/auto-trader/activity'),
      ]);
      if (!auditResponse.ok) throw new Error('Audit unavailable');
      if (!positionsResponse.ok) throw new Error('AutoTrader positions unavailable');
      if (!tradesResponse.ok) throw new Error('AutoTrader fills unavailable');
      const records = await auditResponse.json() as AuditRecord[];
      const positionsData = await positionsResponse.json() as PositionsResponse;
      const tradesData = await tradesResponse.json() as TradesResponse;
      const activityData = activityResponse.ok ? await activityResponse.json() as ActivityResponse : {};
      const exitPolicies = new Map((activityData.position_exit_policies ?? []).map((policy) => [policy.symbol, policy]));
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
      const rows: AutoTraderOutcome[] = [...trades.entries()].flatMap(([decisionId, trade]) => {
        const entry = trade.entry;
        if (!entry || !trade.exits.length) return [];
        const canonicalExits = this.canonicalExits(trade.exits);
        const exited = canonicalExits.reduce((sum, row) => sum + (row.exit_filled_quantity ?? 0), 0);
        const entryQuantity = Math.max(entry.filled_quantity ?? 0, ...canonicalExits.map((row) => row.entry_context?.quantity ?? 0));
        const entryPrice = entry.average_price || canonicalExits.find((row) => row.entry_context?.average_price)?.entry_context?.average_price;
        if (!(entryQuantity > 0 && typeof entryPrice === 'number' && entryPrice > 0) || exited + 1e-12 < entryQuantity) return [];
        const symbol = entry.symbol ?? canonicalExits.find((row) => row.symbol)?.symbol ?? '—';
        return [{ rowId: 'closed:' + decisionId, status: 'Closed', decisionId, symbol, side: entry.side ?? '—', openedAt: entry.timestamp ?? 0,
          closedAt: Math.max(...canonicalExits.map((row) => row.timestamp ?? 0)), quantity: entryQuantity, signedQuantity: entry.side === 'sell' ? -entryQuantity : entryQuantity,
          entryPrice, pnl: canonicalExits.reduce((sum, row) => sum + (row.realized_pnl ?? 0), 0), entryFees: 0, estimatedExitFeeRate: 0, estimatedExitSlippageBps: 0,
          pnlQuoteCurrency: quoteCurrency(symbol), strategy: entry.strategy ?? 'live-insights', direction: entry.direction ?? '—', strength: entry.strength ?? null,
          scout: entry.scout ?? false, components: entry.signal_components ?? {} }];
      });
      const openPositions = (positionsData.positions ?? []).filter((position) => Math.abs(position.quantity) > 1e-12);
      for (const position of openPositions) {
        const side = position.quantity > 0 ? 'buy' : 'sell';
        const activeEntries = [...trades.values()].flatMap((trade) => {
          const entry = trade.entry;
          if (!entry || entry.symbol !== position.symbol || !entry.filled_quantity || !entry.average_price) return [];
          const exited = this.canonicalExits(trade.exits).reduce((sum, row) => sum + (row.exit_filled_quantity ?? 0), 0);
          const remaining = entry.filled_quantity - exited;
          const sameSide = (position.quantity > 0 && entry.side === 'buy') || (position.quantity < 0 && entry.side === 'sell');
          return sameSide && remaining > 1e-12 ? [entry] : [];
        }).sort((a, b) => (b.timestamp ?? 0) - (a.timestamp ?? 0));
        const latestFilledOrder = (activityData.orders ?? [])
          .filter((order) => order.symbol === position.symbol
            && order.side === side
            && (order.purpose === 'signal' || order.purpose === 'scout')
            && order.filled_quantity > 1e-12)
          .sort((a, b) => b.timestamp - a.timestamp)[0];
        const entry = activeEntries[0]
          ?? (latestFilledOrder?.decision_id ? trades.get(latestFilledOrder.decision_id)?.entry : undefined);
        const symbol = position.symbol;
        const adverseSlippage = adverseSlippageBySymbol.get(symbol) ?? [];
        const estimatedExitSlippageBps = adverseSlippage.length ? adverseSlippage.reduce((sum, value) => sum + value, 0) / adverseSlippage.length : 0;
        const decisionId = entry?.decision_id ?? latestFilledOrder?.decision_id ?? 'ledger:' + symbol;
        rows.push({ rowId: 'open:' + symbol, status: 'Open', decisionId, symbol, side, openedAt: entry?.timestamp ?? latestFilledOrder?.timestamp ?? position.last_trade_timestamp,
          quantity: Math.abs(position.quantity), signedQuantity: position.quantity, entryPrice: position.average_entry_price, pnl: 0,
          entryFees: position.entry_fees ?? 0, estimatedExitFeeRate: position.estimated_exit_fee_rate ?? 0, estimatedExitSlippageBps,
          pnlQuoteCurrency: quoteCurrency(symbol), strategy: entry?.strategy ?? latestFilledOrder?.strategy ?? 'Unknown', direction: entry?.direction ?? (side === 'buy' ? 'bullish' : 'bearish'),
          stopLossPercent: exitPolicies.get(symbol)?.stop_loss_percent, takeProfitPercent: exitPolicies.get(symbol)?.take_profit_percent,
          strength: entry?.strength ?? null, scout: entry?.scout ?? false, components: entry?.signal_components ?? {} });
      }
      rows.sort((a, b) => (a.status === b.status ? b.openedAt - a.openedAt : a.status === 'Open' ? -1 : 1));
      this.state$.next({ ...this.state$.value, rows, loading: false });
    } catch { this.state$.next({ ...this.state$.value, loading: false, error: 'AutoTrader positions or closed trade audit are unavailable.' }); }
  }
  private canonicalExits(exits: AuditRecord[]) {
    const externalFillIds = new Set(exits
      .filter((row) => row.exit_order_type === 'external-ledger-fill')
      .flatMap((row) => row.exit_fills?.map((fill) => fill.parent_order_id).filter((id): id is string => Boolean(id)) ?? []));
    return exits.filter((row) => !(row.exit_order_type === 'StopMarket' && row.barrier_order_id && externalFillIds.has(row.barrier_order_id)));
  }
}
