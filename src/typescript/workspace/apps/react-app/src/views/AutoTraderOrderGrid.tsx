import { formatForSymbol } from '../instrument_precision';
import type { AutoTraderOrder } from '../models/AutoTraderModel';
import { getOrderTypeDescription } from '../order_type_descriptions';
import {
  themeQuartz,
  type ColDef,
  type ICellRendererParams,
  type ITooltipParams,
} from 'ag-grid-community';

export const money = new Intl.NumberFormat(undefined, { minimumFractionDigits: 3, maximumFractionDigits: 3 });
export const timestamp = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' });

export function explainAutoTraderReason(reason: string): string {
  const lower = reason.toLowerCase();
  if (lower.startsWith('no tradeable bias')) {
    const fresh = reason.match(/fresh directional (\d+)\/(\d+)/i);
    const strength = reason.match(/strength floors passed (\d+)/i);
    const exploration = reason.match(/raw-score floors passed (\d+)/i);
    const usable = fresh ? Number(fresh[1]) : 0;
    const total = fresh ? Number(fresh[2]) : 0;
    if (total === 0) return 'No insights available';
    if (usable === 0) {
      const filters = reason.match(/filtered: (\d+) stale, (\d+) non-directional, (\d+) blocked by existing positions/i);
      const causes = filters
        ? Number(filters[1]) + ' stale · ' + Number(filters[2]) + ' no direction · ' + Number(filters[3]) + ' position blocked'
        : 'filtered before strength or history checks';
      return 'No usable buy or sell signal\n' + total + ' insight checked · ' + causes;
    }
    if (strength?.[1] === '0' && exploration?.[1] === '0') return 'Signal too weak\n' + usable + ' usable · 0 passed normal strength · 0 passed trial score';
    const history = reason.match(/(\d+) candidate\(s\) blocked by negative walk-forward history/i);
    const blocked = history ? Number(history[1]) : 0;
    if (blocked > 0) return 'Past-performance check blocked ' + blocked + ' candidate' + (blocked === 1 ? '' : 's');
    return 'No signal passed all entry checks\n' + usable + ' usable signals checked';
  }
  if (lower.includes('no insights available')) return 'No insights available';
  if (lower.includes('no usable buy or sell signal') || lower.includes('no usable directional signal') || lower.includes('no fresh directional signal')) return 'No usable buy or sell signal';
  if (lower.includes('signals were stale')) return 'Insights were stale';
  if (lower.includes('no directional signal')) return 'Insights had no clear direction';
  if (lower.includes('position blocked')) return 'Existing position blocked another entry';
  if (lower.includes('signal too weak') || lower.includes('strength and score floors')) return 'Signal too weak for entry';
  if (lower.includes('daily realized-loss limit')) return 'Daily loss limit reached';
  if (lower.includes('budget') || lower.includes('capacity')) return 'Not enough budget or position capacity';
  if (lower.includes('walk-forward') || lower.includes('historical')) return 'Past-performance check blocked the trade';
  if (lower.includes('stale local order book')) return 'Market data was out of date';
  if (lower.includes('no cached local order book')) return 'Market data was unavailable';
  if (lower.includes('position') || lower.includes('cooldown')) return 'Existing position or wait period';
  if (lower.includes('cycle error')) return 'Decision cycle error';
  return reason;
}
export const autoTraderGridTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});

export type AutoTraderOrderRow = AutoTraderOrder & { current_price?: number; position_status?: string; protection_order?: boolean; strategy_group?: string; group_header?: boolean; group_label?: string };

export const orderColumnDefs: ColDef<AutoTraderOrderRow>[] = [
  { field: 'timestamp', headerName: 'Time / Trigger', minWidth: 102, colSpan: ({ data }) => data?.group_header ? orderColumnDefs.length : 1, cellRenderer: ({ data, value }: ICellRendererParams<AutoTraderOrderRow>) => data?.group_header ? <strong>{data.group_label}</strong> : value ? timestamp.format(value) : '', valueGetter: ({ data }) => data?.group_header ? undefined : data?.triggered_at ?? data?.timestamp },
  { field: 'symbol', headerName: 'Symbol', minWidth: 78 },
  { field: 'side', headerName: 'Side', minWidth: 56, valueFormatter: ({ value }) => String(value ?? '').toUpperCase() },
  { field: 'order_type', headerName: 'Type', minWidth: 70, tooltipValueGetter: (params: ITooltipParams<AutoTraderOrderRow>) => getOrderTypeDescription(params.value), cellRenderer: (params: ICellRendererParams<AutoTraderOrderRow>) => <span title={getOrderTypeDescription(params.value)}>{String(params.value ?? '')}</span> },
  { field: 'strategy', headerName: 'Strategy', minWidth: 154, cellRenderer: (params: ICellRendererParams<AutoTraderOrderRow>) => <span title={strategyProfile(params.value)}>{params.data?.protection_order ? '↳ ' : ''}{strategyLabel(params.value)}</span> },
  { field: 'purpose', headerName: 'Purpose', minWidth: 76, valueFormatter: ({ value }) => value === 'portfolio-hedge' ? 'HEDGE' : String(value ?? '').toUpperCase() },
  { field: 'status', headerName: 'Order Status', minWidth: 82, filter: 'agTextColumnFilter' },
  { field: 'failure_reason', headerName: 'Failure reason', minWidth: 210, cellRenderer: (params: ICellRendererParams<AutoTraderOrderRow>) => { const reason = String(params.value ?? ''); return reason ? <span title={reason}>{reason}</span> : ''; } },
  { field: 'position_status', headerName: 'Position', minWidth: 75 },
  { field: 'filled_quantity', headerName: 'Filled / Qty', minWidth: 96, valueFormatter: ({ data }) => data ? formatForSymbol(data.filled_quantity, data.symbol, 'quantity') + '/' + formatForSymbol(data.quantity, data.symbol, 'quantity') : '' },
  { field: 'submitted_limit_price', headerName: 'Limit / Stop Price', minWidth: 112, valueGetter: ({ data }) => data?.submitted_limit_price ?? data?.target_price, valueFormatter: ({ value, data }) => value == null || !data ? '' : formatForSymbol(value, data.symbol, 'price') },
  { field: 'current_price', headerName: 'Current Price', minWidth: 92, valueFormatter: ({ value, data }) => value == null || !data ? '' : formatForSymbol(value, data.symbol, 'price') },
  { field: 'average_price', headerName: 'Avg Price', minWidth: 78, valueFormatter: ({ value }) => value == null ? '' : money.format(value) },
  { field: 'realized_pnl_usd', headerName: 'Realized USD', minWidth: 92, cellClass: ({ value }) => value >= 0 ? 'pnl-positive' : 'pnl-negative', valueFormatter: ({ value }) => value == null || Math.abs(value) < 1e-9 ? '—' : money.format(value) },
];

export function strategyProfile(strategy?: string): string {
  const profiles: Record<string, string> = {
    'donchian-55-btc200': 'Entry: close breaks above the prior 55-day high while BTC is above its SMA200. Exit: protective stop or daily breakout/BTC-regime signal exit; no fixed take-profit. Long-only; the BTC regime gates eligible assets.',
    'donchian-20-sma200': 'Entry: close breaks above the prior 20-day high and the asset is above its own SMA200. Exit: protective stop or daily signal when breakout/trend eligibility ends; no fixed take-profit. Long-only.',
    'donchian-20-btc200': 'Entry: close breaks above the prior 20-day high while BTC is above its SMA200. Exit: daily signal when breakout or BTC regime becomes inactive. No fixed protective stop or take-profit; losses can exceed a nominal risk threshold before the next signal-driven close.',
    'donchian-10-btc200': 'Entry: close breaks above the prior 10-day high while BTC is above its SMA200. Exit: daily signal when breakout or BTC regime becomes inactive. No fixed protective stop or take-profit; losses can exceed a nominal risk threshold before the next signal-driven close.',
    'donchian-20-btc200-hold3': 'Entry: close breaks above the prior 20-day high while BTC is above its SMA200. Exit: daily signal after a minimum 72-hour hold. No fixed protective stop or take-profit; the minimum hold can delay a signal exit during adverse moves.',
    'risk-adjusted-trend-portfolio': 'Entry/rebalance: long-only 16-pair portfolio when BTC is above SMA200 and average 10/20/40/80/160-day returns are positive; inverse-volatility weights, capped at 25% per asset, rebalance daily when drift exceeds 5%. Exit: daily signal/rebalance. No fixed stop or take-profit; may hold cash when the regime or asset criteria fail.',
    'trend-sleeves-btc200': 'Entry: BTC and ETH above their own SMA50, or BNB above its SMA100, with BTC above SMA200. Exit: protective stop, take-profit, or when the sleeve trend/BTC regime signal becomes inactive. Long-only BTC/ETH/BNB; other symbols are ineligible.',
    'trend-50-200': 'Entry: asset close above both its SMA50 and SMA200. Exit: protective stop, take-profit, or daily trend signal when either moving-average condition fails. Long-only.',
    'relative-strength-90-sma200': 'Entry/rotation: strongest eligible 90-day performer, provided its close is above its own SMA200. Exit: protective stop, take-profit, or daily signal when it loses eligibility / another leader takes over. Long-only; only the current leader is active.',
    'risk-adjusted-monthly-momentum': 'Entry/rotation: at the monthly anchor, choose the eligible positive 90-day-return leader by 90-day return / 60-day realized volatility while BTC is above SMA200. Exit: monthly signal rotation or BTC-regime/eligibility change. No fixed stop or take-profit; long-only, one asset or cash, and volatility targeting scales size rather than limiting the loss.',
    'rsi2-oversold-rebound': 'Entry: two-day RSI(2) at or below 10 while the asset is above its SMA200. Exit: protective stop, take-profit, RSI(2) reaching 70, SMA200 gate failure, or five-day maximum hold. Long-only; daily candles mean intraday moves can pass between signal checks.',
    'dated-futures-carry': 'Entry: Insights carry signal opens a paired spot-long / dated-future-short paper position. Exit: settlement/expiry-window lifecycle; no directional TP/SL. Both legs matter: missing/stale leg marks can delay valuation, and settlement/fees affect realized carry.',
    'live-momentum': 'Entry: fresh directional momentum signal passing configured strength/risk gates. Exit: protective stop, take-profit, or strategy signal/position-management rules. Execution type and fill price depend on the configured order type; fees and slippage can move net P&L beyond the displayed threshold.',
    'live-trade-flow': 'Entry: fresh aligned trade-flow imbalance signal passing configured gates. Exit: protective stop, take-profit, or strategy signal/position-management rules. Execution type and fill price depend on the configured order type; fees and slippage affect net P&L.',
    'live-order-book': 'Entry: fresh aligned order-book imbalance signal passing configured gates. Exit: protective stop, take-profit, or strategy signal/position-management rules. Stale or unavailable book data can prevent entry; execution and slippage affect net P&L.',
    'live-vwap-dislocation': 'Entry: fresh aligned VWAP-dislocation signal passing configured gates. Exit: protective stop, take-profit, or strategy signal/position-management rules. Data freshness, order type, fees, and slippage affect realized results.',
    'live-absorption-pressure': 'Entry: fresh aligned absorption/pressure signal passing configured gates. Exit: protective stop, take-profit, or strategy signal/position-management rules. Signal expiry and execution quality can affect entry and exit timing.',
    'live-insights': 'Entry: fresh directional Insights signal that passes configured risk and strength gates. Exit: protective stop, take-profit, or signal/position-management rules. Fees and execution slippage mean net P&L can differ from the price threshold.',
    'generic-insight-signals': 'Legacy Insights-driven entry. Exit behavior depends on the stored position policy and available signal state; verify the position row for active stop/target values.',
    'portfolio-hedge': 'Portfolio-level hedge order, not a standalone directional strategy. Its purpose and lifecycle are controlled by portfolio risk management.',
  };
  return strategy ? profiles[strategy] ?? 'Strategy profile unavailable for this legacy or unknown strategy. Check the position row and order purpose for applicable exits.' : 'Legacy / unknown strategy; entry and exit rules are unavailable.';
}

export function strategyLabel(strategy?: string): string {
  const labels: Record<string, string> = {
    'donchian-55-btc200': 'Donchian 55 + BTC SMA200',
    'donchian-20-sma200': 'Donchian 20 + asset SMA200',
    'donchian-20-btc200': 'Donchian 20 + BTC SMA200',
    'donchian-10-btc200': 'Donchian 10 + BTC SMA200',
    'donchian-20-btc200-hold3': 'Donchian 20 + BTC SMA200 + 3-day hold',
    'risk-adjusted-trend-portfolio': 'Risk-adjusted trend portfolio',
    'trend-sleeves-btc200': 'BTC/ETH/BNB trend sleeves',
    'trend-50-200': 'SMA 50/200 trend',
    'relative-strength-90-sma200': '90-day momentum rotation',
    'risk-adjusted-monthly-momentum': 'Risk-adjusted monthly momentum',
    'rsi2-oversold-rebound': 'RSI(2) rebound',
    'dated-futures-carry': 'Dated futures carry',
    'live-momentum': 'Live momentum',
    'live-trade-flow': 'Live trade flow',
    'live-order-book': 'Live order book',
    'live-vwap-dislocation': 'Live VWAP dislocation',
    'live-absorption-pressure': 'Live absorption',
    'live-insights': 'Live Insights',
    'generic-insight-signals': 'Legacy Live Insights',
    'portfolio-hedge': 'Portfolio hedge',
  };
  return strategy ? labels[strategy] ?? strategy : 'Legacy / unknown';
}

export const defaultOrderColDef: ColDef<AutoTraderOrderRow> = {
  sortable: true,
  resizable: true,
  filter: true,
  flex: 1,
  minWidth: 68,
};
