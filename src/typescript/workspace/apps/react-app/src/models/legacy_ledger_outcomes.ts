import type { AutoTraderOutcome } from './AutoTraderOutcomesModel';
import type { PositionTrade } from './PositionTypes';

type LegacyCycle = { trades: PositionTrade[]; openedAt: number; entryQuantity: number; entryPrice: number; positionQuantity: number; direction: 'buy' | 'sell'; realizedQuote: number; realizedParents: Set<string> };

export function buildLegacyLedgerOutcomes(symbol: string, trades: PositionTrade[], quote: string, quoteUsdRate: number | undefined, realizedByParent: Record<string, number>, orderTypes: Map<string, string>): AutoTraderOutcome[] {
  const sorted = [...trades].sort((a, b) => a.timestamp - b.timestamp || a.id.localeCompare(b.id));
  const rows: AutoTraderOutcome[] = [];
  let cycle: LegacyCycle | undefined;
  let cycleNumber = 0;
  const startCycle = (trade: PositionTrade, signedQuantity: number) => {
    cycle = { trades: [], openedAt: trade.timestamp, entryQuantity: 0, entryPrice: trade.execution_price, positionQuantity: 0, direction: signedQuantity > 0 ? 'buy' : 'sell', realizedQuote: 0, realizedParents: new Set() };
  };
  const flushCycle = (closedAt?: number) => {
    if (!cycle || cycle.trades.length === 0) return;
    const cycleId = 'legacy:' + symbol + ':' + cycle.trades[0].id + ':' + cycleNumber++;
    const remaining = Math.abs(cycle.positionQuantity);
    const status = remaining > 1e-12 ? 'Partially closed' : 'Closed';
    const realizedUsd = typeof quoteUsdRate === 'number' && Number.isFinite(quoteUsdRate) ? cycle.realizedQuote * quoteUsdRate : undefined;
    rows.push({ rowId: 'ledger-summary:' + cycleId, status, decisionId: cycleId, strategyInstanceId: cycleId, symbol, side: cycle.direction, openedAt: cycle.openedAt, closedAt: status === 'Closed' ? closedAt : undefined, quantity: cycle.entryQuantity, remainingQuantity: status === 'Partially closed' ? remaining : undefined, signedQuantity: cycle.direction === 'buy' ? remaining : -remaining, entryPrice: cycle.entryPrice, pnl: 0, entryFees: 0, estimatedExitFeeRate: 0, estimatedExitSlippageBps: 0, netPnlUsd: realizedUsd, pnlQuoteCurrency: quote, quoteUsdRate, strategy: 'legacy-unknown', direction: cycle.direction.toUpperCase(), strength: null, scout: false, source: 'unknown', components: {} });
    for (const trade of cycle.trades) {
      const parentId = trade.parent_order_id ?? '';
      rows.push({ rowId: 'ledger-trade:' + trade.id, status, decisionId: cycleId, strategyInstanceId: cycleId, ledgerTrade: trade, ledgerSource: trade.source ?? 'auto', ledgerOrderType: orderTypes.get(parentId), symbol, side: trade.side, openedAt: trade.timestamp, closedAt: trade.timestamp, quantity: trade.quantity, signedQuantity: trade.side === 'buy' ? trade.quantity : -trade.quantity, entryPrice: trade.execution_price, pnl: 0, entryFees: trade.fee ?? 0, estimatedExitFeeRate: trade.fee_rate ?? 0, estimatedExitSlippageBps: 0, pnlQuoteCurrency: quote, quoteUsdRate, strategy: 'legacy-unknown', direction: trade.side.toUpperCase(), strength: null, scout: false, source: 'unknown', components: {} });
    }
  };
  for (const trade of sorted) {
    const signed = trade.side === 'buy' ? trade.quantity : -trade.quantity;
    if (!cycle) startCycle(trade, signed);
    const active = cycle!;
    const oldQuantity = active.positionQuantity;
    const closes = oldQuantity !== 0 && Math.sign(oldQuantity) !== Math.sign(signed);
    if (closes && trade.parent_order_id && !active.realizedParents.has(trade.parent_order_id)) {
      active.realizedParents.add(trade.parent_order_id);
      active.realizedQuote += realizedByParent[trade.parent_order_id] ?? 0;
    }
    active.trades.push(trade);
    if (oldQuantity === 0 || Math.sign(oldQuantity) === Math.sign(signed)) {
      const nextAbs = Math.abs(oldQuantity) + Math.abs(signed);
      active.entryPrice = nextAbs > 0 ? (Math.abs(oldQuantity) * active.entryPrice + Math.abs(signed) * trade.execution_price) / nextAbs : trade.execution_price;
      active.entryQuantity += Math.abs(signed);
      active.positionQuantity = oldQuantity + signed;
      continue;
    }
    active.positionQuantity = oldQuantity + signed;
    if (Math.abs(active.positionQuantity) <= 1e-12) {
      active.positionQuantity = 0;
      flushCycle(trade.timestamp);
      cycle = undefined;
    } else if (Math.sign(active.positionQuantity) !== Math.sign(oldQuantity)) {
      const remainder = active.positionQuantity;
      flushCycle(trade.timestamp);
      cycle = undefined;
      startCycle(trade, remainder);
      cycle!.trades.push(trade);
      cycle!.entryQuantity = Math.abs(remainder);
      cycle!.positionQuantity = remainder;
      cycle!.entryPrice = trade.execution_price;
    }
  }
  if (cycle) flushCycle();
  return rows;
}

