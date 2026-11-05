import type { WorkspaceLayoutConfig } from '@workspace/ui-platform';

export const DASHBOARD_WIDGET_TYPES = [
  'grid',
  'table',
  'chart',
  'book',
  'trades',
  'liquidity',
  'signals',
  'news',
  'signal-history',
  'signal-performance',
  'walk-forward',
  'missed-opportunity-bands',
  'positions',
  'fills',
  'pnl',
  'account',
  'risk',
  'close-insights',
  'hedging',
  'auto-trader',
  'dated-carry',
  'auto-trader-diagnostics',
  'auto-trader-learning',
  'auto-trader-outcomes',
  'auto-trader-completed-positions',
  'auto-trader-live-positions',
  'auto-trader-scout-positions',
  'execution-fees',
] as const;

export type DashboardWidgetType = (typeof DASHBOARD_WIDGET_TYPES)[number];

export const DEFAULT_WIDGET_TYPES: readonly DashboardWidgetType[] = [
  'grid',
  'table',
  'chart',
  'book',
  'risk',
];

export const WIDGET_LABELS: Record<DashboardWidgetType, string> = {
  grid: 'Market overview',
  table: 'Pair volatility',
  chart: 'Price history',
  book: 'Order book',
  trades: 'Recent trades',
  liquidity: 'Market liquidity',
  signals: 'Best signals',
  news: 'Crypto news & signals',
  'signal-history': 'Signal history',
  'signal-performance': 'Signal performance',
  'walk-forward': 'Walk-forward evaluation',
  'missed-opportunity-bands': 'Missed opportunity bands',
  positions: 'Orders',
  fills: 'Fills',
  pnl: 'P&L',
  account: 'Account',
  risk: 'Portfolio Risk',
  'close-insights': 'Close-out insights',
  hedging: 'Hedging',
  'auto-trader': 'Auto Trader',
  'dated-carry': 'Dated futures carry',
  'auto-trader-diagnostics': 'Auto Trader diagnostics',
  'auto-trader-learning': 'Strategy learning impact',
  'auto-trader-outcomes': 'AutoTrader outcomes',
  'auto-trader-completed-positions': 'AutoTrader closed positions',
  'auto-trader-live-positions': 'AutoTrader live positions',
  'auto-trader-scout-positions': 'AutoTrader scout positions',
  'execution-fees': 'Execution fees',
};

export const DASHBOARD_STORAGE_KEYS = {
  layout: 'market-dashboard-layout',
  layouts: 'market-dashboard-layouts',
  activeLayout: 'market-dashboard-active-layout',
} as const;

export const WORKSPACE_LAYOUT_CONFIG: WorkspaceLayoutConfig<DashboardWidgetType> =
  {
    widgetTypes: DASHBOARD_WIDGET_TYPES,
    defaultWidgetTypes: DEFAULT_WIDGET_TYPES,
    symbolWidgetTypes: ['chart', 'book', 'trades'],
    storageKeys: DASHBOARD_STORAGE_KEYS,
  };
