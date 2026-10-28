import { formatForSymbol } from '../instrument_precision';
import type { AutoTraderOrder } from '../models/AutoTraderModel';
import { getOrderTypeDescription } from '../order_type_descriptions';
import {
  themeQuartz,
  type ColDef,
  type ICellRendererParams,
  type ITooltipParams,
} from 'ag-grid-community';

export const money = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 });
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
  { field: 'strategy', headerName: 'Strategy', minWidth: 154, cellRenderer: (params: ICellRendererParams<AutoTraderOrderRow>) => <span>{params.data?.protection_order ? '↳ ' : ''}{strategyLabel(params.value)}</span> },
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

export function strategyLabel(strategy?: string): string {
  const labels: Record<string, string> = {
    'donchian-55-btc200': 'Donchian 55 + BTC SMA200',
    'donchian-20-sma200': 'Donchian 20 + asset SMA200',
    'trend-sleeves-btc200': 'BTC/ETH/BNB trend sleeves',
    'trend-50-200': 'SMA 50/200 trend',
    'relative-strength-90-sma200': '90-day momentum rotation',
    'rsi2-oversold-rebound': 'RSI(2) rebound',
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
