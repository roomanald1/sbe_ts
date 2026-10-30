export type DashboardWidgetType =
  | 'grid'
  | 'table'
  | 'chart'
  | 'book'
  | 'trades'
  | 'liquidity'
  | 'signals'
  | 'news'
  | 'signal-history'
  | 'signal-performance'
  | 'walk-forward'
  | 'missed-opportunity-bands'
  | 'positions'
  | 'fills'
  | 'pnl'
  | 'account'
  | 'risk'
  | 'close-insights'
  | 'hedging'
  | 'auto-trader'
  | 'dated-carry'
  | 'auto-trader-diagnostics'
  | 'auto-trader-outcomes'
  | 'auto-trader-completed-positions'
  | 'auto-trader-live-positions'
  | 'auto-trader-shadow-positions';
export type DashboardWidget = {
  id: string;
  type: DashboardWidgetType;
  symbol: string;
  x: number;
  y: number;
  w: number;
  h: number;
  collapsed: boolean;
};
export type SavedDashboardLayout = {
  id: string;
  name: string;
  widgets: DashboardWidget[];
};

export const DASHBOARD_WIDGET_TYPES: DashboardWidgetType[] = [
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
  'auto-trader-outcomes',
  'auto-trader-completed-positions',
  'auto-trader-live-positions',
  'auto-trader-shadow-positions',
];
export const DEFAULT_WIDGET_TYPES: DashboardWidgetType[] = [
  'grid',
  'table',
  'chart',
  'book',
  'risk',
];
export const DASHBOARD_COLUMNS = 24;
export const DEFAULT_WIDGET_WIDTH = DASHBOARD_COLUMNS / 2;
export const DEFAULT_WIDGET_HEIGHT = 12;
export const DASHBOARD_LAYOUT_KEY = 'market-dashboard-layout';
export const DASHBOARD_LAYOUTS_KEY = 'market-dashboard-layouts';
export const DASHBOARD_ACTIVE_LAYOUT_KEY = 'market-dashboard-active-layout';
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
  'auto-trader-outcomes': 'AutoTrader outcomes',
  'auto-trader-completed-positions': 'AutoTrader closed & partial positions',
  'auto-trader-live-positions': 'AutoTrader live positions',
  'auto-trader-shadow-positions': 'AutoTrader shadow positions',
};

export function makeWidget(
  type: DashboardWidgetType,
  symbol = '',
  x = 0,
  y = 0,
): DashboardWidget {
  return {
    id: Date.now().toString(36) + '-' + Math.random().toString(36).slice(2),
    type,
    symbol,
    x,
    y,
    w: DEFAULT_WIDGET_WIDTH,
    h: DEFAULT_WIDGET_HEIGHT,
    collapsed: false,
  };
}

export function defaultDashboardLayout(): DashboardWidget[] {
  return DEFAULT_WIDGET_TYPES.map((type, index) => ({
    id: 'default-' + type + '-' + index,
    type,
    symbol: '',
    x: index % 2 === 0 ? 0 : DEFAULT_WIDGET_WIDTH,
    y: Math.floor(index / 2) * DEFAULT_WIDGET_HEIGHT,
    w: DEFAULT_WIDGET_WIDTH,
    h: DEFAULT_WIDGET_HEIGHT,
    collapsed: false,
  }));
}

export function overlaps(a: DashboardWidget, b: DashboardWidget): boolean {
  return (
    a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y
  );
}

export function firstOpenPosition(widgets: DashboardWidget[]): {
  x: number;
  y: number;
} {
  for (let y = 0; ; y += 1) {
    for (let x = 0; x <= DASHBOARD_COLUMNS - DEFAULT_WIDGET_WIDTH; x += 1) {
      const candidate = {
        id: '',
        type: 'grid' as DashboardWidgetType,
        symbol: '',
        x,
        y,
        w: DEFAULT_WIDGET_WIDTH,
        h: DEFAULT_WIDGET_HEIGHT,
        collapsed: false,
      };
      if (!widgets.some((widget) => overlaps(candidate, widget)))
        return { x, y };
    }
  }
}

export function findFreeLanding(
  widget: DashboardWidget,
  x: number,
  y: number,
): { x: number; y: number } {
  return {
    x: Math.max(0, Math.min(DASHBOARD_COLUMNS - widget.w, x)),
    y: Math.max(0, y),
  };
}

export function shiftWidgetsForAnchor(
  widgets: DashboardWidget[],
  anchorId: string,
): DashboardWidget[] {
  const anchor = widgets.find((widget) => widget.id === anchorId);
  if (!anchor) return widgets;
  const placed: DashboardWidget[] = [
    {
      ...anchor,
      x: Math.max(0, Math.min(DASHBOARD_COLUMNS - anchor.w, anchor.x)),
      y: Math.max(0, anchor.y),
    },
  ];
  const remaining = widgets
    .filter((widget) => widget.id !== anchorId)
    .sort((a, b) => a.y - b.y || a.x - b.x);
  remaining.forEach((widget) => {
    const candidate = {
      ...widget,
      x: Math.max(0, Math.min(DASHBOARD_COLUMNS - widget.w, widget.x)),
    };
    while (placed.some((other) => overlaps(candidate, other))) candidate.y += 1;
    placed.push(candidate);
  });
  return widgets.map(
    (widget) => placed.find((item) => item.id === widget.id) ?? widget,
  );
}

export function packDashboardWidgets(
  widgets: DashboardWidget[],
  anchorId?: string,
): DashboardWidget[] {
  const packed: DashboardWidget[] = [];
  const anchor = widgets.find((widget) => widget.id === anchorId);
  if (anchor)
    packed.push({
      ...anchor,
      x: Math.max(0, Math.min(DASHBOARD_COLUMNS - anchor.w, anchor.x)),
      y: Math.max(0, anchor.y),
      w: Math.min(anchor.w, DASHBOARD_COLUMNS),
    });
  const ordered = [...widgets]
    .filter((widget) => widget.id !== anchorId)
    .sort((a, b) => a.y - b.y || a.x - b.x);
  ordered.forEach((widget) => {
    const width = Math.min(widget.w, DASHBOARD_COLUMNS);
    for (let y = 0; ; y += 1) {
      let placed = false;
      for (let x = 0; x <= DASHBOARD_COLUMNS - width; x += 1) {
        const candidate = { ...widget, x, y, w: width };
        if (!packed.some((other) => overlaps(candidate, other))) {
          packed.push(candidate);
          placed = true;
          break;
        }
      }
      if (placed) break;
    }
  });
  return widgets.map(
    (widget) => packed.find((item) => item.id === widget.id) ?? widget,
  );
}
