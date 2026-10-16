import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import type { Connection, ConnectionState, FeedSource } from '../connection';
import { SymbolPriceGrid } from './SymbolPriceGrid';
import { SymbolPriceTable } from './SymbolPriceTable';
import { PriceChangeChart } from './PriceChangeChart';
import { Users } from './Users';
import { Positions } from './Positions';
import { Fills } from './Fills';
import { Pnl } from './Pnl';
import { Account } from './Account';
import { PortfolioRisk } from './PortfolioRisk';
import { CloseInsights } from './CloseInsights';
import { Hedging } from './Hedging';
import { TradeTicketPopup } from './TradeTicketPopup';
import { OrderBookPopup } from './OrderBookPopup';
import { TradesView } from './TradesView';
import { MarketLiquidity } from './MarketLiquidity';
import { BestSignalsView, SignalHistoryView, SignalPerformanceView } from './SignalInfoViews';
import { PriceChangesModel } from '../price_change_data';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';

type FeedControlsProps = {
  connection: Connection;
  feedSource: FeedSource;
  connectionState: ConnectionState;
  onFeedSourceChange: (source: FeedSource) => void;
};

type DashboardWidgetType =
  | 'grid'
  | 'table'
  | 'chart'
  | 'book'
  | 'trades'
  | 'liquidity'
  | 'signals'
  | 'signal-history'
  | 'signal-performance'
  | 'positions'
  | 'fills'
  | 'pnl'
  | 'account'
  | 'risk'
  | 'close-insights'
  | 'hedging';
type DashboardWidget = {
  id: string;
  type: DashboardWidgetType;
  symbol: string;
  x: number;
  y: number;
  w: number;
  h: number;
};
type SavedDashboardLayout = {
  id: string;
  name: string;
  widgets: DashboardWidget[];
};

const DASHBOARD_WIDGET_TYPES: DashboardWidgetType[] = [
  'grid',
  'table',
  'chart',
  'book',
  'trades',
  'liquidity',
  'signals',
  'signal-history',
  'signal-performance',
  'positions',
  'fills',
  'pnl',
  'account',
  'risk',
  'close-insights',
  'hedging',
];
const DEFAULT_WIDGET_TYPES: DashboardWidgetType[] = [
  'grid',
  'table',
  'chart',
  'book',
  'risk',
];
const DASHBOARD_COLUMNS = 24;
const DEFAULT_WIDGET_WIDTH = DASHBOARD_COLUMNS / 2;
const DEFAULT_WIDGET_HEIGHT = 12;
const DASHBOARD_LAYOUT_KEY = 'market-dashboard-layout';
const DASHBOARD_LAYOUTS_KEY = 'market-dashboard-layouts';
const DASHBOARD_ACTIVE_LAYOUT_KEY = 'market-dashboard-active-layout';
const WIDGET_LABELS: Record<DashboardWidgetType, string> = {
  grid: 'Market overview',
  table: 'Pair volatility',
  chart: 'Price history',
  book: 'Order book',
  trades: 'Recent trades',
  liquidity: 'Market liquidity',
  signals: 'Best signals',
  'signal-history': 'Signal history',
  'signal-performance': 'Signal performance',
  positions: 'Orders',
  fills: 'Fills',
  pnl: 'P&L',
  account: 'Account',
  risk: 'Portfolio Risk',
  'close-insights': 'Close-out insights',
  hedging: 'Hedging',
};

function makeWidget(
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
  };
}

function defaultDashboardLayout(): DashboardWidget[] {
  return DEFAULT_WIDGET_TYPES.map((type, index) => ({
    id: 'default-' + type + '-' + index,
    type,
    symbol: '',
    x: index % 2 === 0 ? 0 : DEFAULT_WIDGET_WIDTH,
    y: Math.floor(index / 2) * DEFAULT_WIDGET_HEIGHT,
    w: DEFAULT_WIDGET_WIDTH,
    h: DEFAULT_WIDGET_HEIGHT,
  }));
}

function overlaps(a: DashboardWidget, b: DashboardWidget): boolean {
  return (
    a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y
  );
}

function firstOpenPosition(widgets: DashboardWidget[]): {
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
      };
      if (!widgets.some((widget) => overlaps(candidate, widget)))
        return { x, y };
    }
  }
}

function findFreeLanding(
  widget: DashboardWidget,
  x: number,
  y: number,
): { x: number; y: number } {
  return {
    x: Math.max(0, Math.min(DASHBOARD_COLUMNS - widget.w, x)),
    y: Math.max(0, y),
  };
}

function shiftWidgetsForAnchor(
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

function packDashboardWidgets(
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

function normalizeWidgets(items: unknown[], scale = 1): DashboardWidget[] {
  const widgets: DashboardWidget[] = [];
  items.forEach((item, index) => {
    if (
      typeof item === 'string' &&
      DASHBOARD_WIDGET_TYPES.includes(item as DashboardWidgetType)
    ) {
      widgets.push({
        id: 'legacy-' + item + '-' + index,
        type: item as DashboardWidgetType,
        symbol: '',
        x: (index % 2) * DEFAULT_WIDGET_WIDTH,
        y: Math.floor(index / 2) * DEFAULT_WIDGET_HEIGHT,
        w: DEFAULT_WIDGET_WIDTH,
        h: DEFAULT_WIDGET_HEIGHT,
      });
      return;
    }
    if (!item || typeof item !== 'object') return;
    const stored = item as {
      id?: unknown;
      type?: unknown;
      symbol?: unknown;
      x?: unknown;
      y?: unknown;
      w?: unknown;
      h?: unknown;
    };
    if (
      typeof stored.id !== 'string' ||
      !DASHBOARD_WIDGET_TYPES.includes(stored.type as DashboardWidgetType)
    )
      return;
    widgets.push({
      id: stored.id,
      type: stored.type as DashboardWidgetType,
      symbol: typeof stored.symbol === 'string' ? stored.symbol : '',
      x:
        typeof stored.x === 'number'
          ? Math.max(
              0,
              Math.min(DASHBOARD_COLUMNS - 1, Math.floor(stored.x * scale)),
            )
          : (index % 2) * DEFAULT_WIDGET_WIDTH,
      y:
        typeof stored.y === 'number'
          ? Math.max(0, Math.floor(stored.y * scale))
          : Math.floor(index / 2) * DEFAULT_WIDGET_HEIGHT,
      w:
        typeof stored.w === 'number'
          ? Math.max(
              1,
              Math.min(DASHBOARD_COLUMNS, Math.floor(stored.w * scale)),
            )
          : DEFAULT_WIDGET_WIDTH,
      h:
        typeof stored.h === 'number'
          ? Math.max(3, Math.floor(stored.h * scale))
          : DEFAULT_WIDGET_HEIGHT,
    });
  });
  const placed: DashboardWidget[] = [];
  [...widgets]
    .sort((a, b) => a.y - b.y || a.x - b.x)
    .forEach((widget) => {
      const width = Math.min(widget.w, DASHBOARD_COLUMNS);
      const candidate = {
        ...widget,
        x: Math.max(0, Math.min(DASHBOARD_COLUMNS - width, widget.x)),
        y: Math.max(0, widget.y),
        w: width,
      };
      while (placed.some((other) => overlaps(candidate, other)))
        candidate.y += 1;
      placed.push(candidate);
    });
  return widgets.map(
    (widget) => placed.find((item) => item.id === widget.id) ?? widget,
  );
}

function loadDashboardLayout(): DashboardWidget[] {
  if (typeof window === 'undefined') return defaultDashboardLayout();
  try {
    const saved = window.localStorage.getItem(DASHBOARD_LAYOUT_KEY);
    if (saved === null) return defaultDashboardLayout();
    const parsed: unknown = JSON.parse(saved);
    const versionTwo =
      parsed && typeof parsed === 'object' && !Array.isArray(parsed)
        ? (parsed as { version?: unknown; widgets?: unknown }).version === 2
        : false;
    const items = versionTwo
      ? (parsed as { widgets?: unknown }).widgets
      : parsed;
    if (!Array.isArray(items)) return defaultDashboardLayout();
    return normalizeWidgets(items, versionTwo ? 1 : 2);
  } catch {
    return defaultDashboardLayout();
  }
}

function loadSavedLayouts(): SavedDashboardLayout[] {
  if (typeof window !== 'undefined') {
    try {
      const saved = window.localStorage.getItem(DASHBOARD_LAYOUTS_KEY);
      if (saved !== null) {
        const parsed: unknown = JSON.parse(saved);
        if (Array.isArray(parsed)) {
          const layouts = parsed.flatMap((item) => {
            if (!item || typeof item !== 'object') return [];
            const stored = item as {
              id?: unknown;
              name?: unknown;
              widgets?: unknown;
            };
            if (
              typeof stored.id !== 'string' ||
              typeof stored.name !== 'string' ||
              !Array.isArray(stored.widgets)
            )
              return [];
            return [
              {
                id: stored.id,
                name: stored.name,
                widgets: normalizeWidgets(stored.widgets),
              },
            ];
          });
          if (layouts.length > 0) return layouts;
        }
      }
    } catch {
      // Fall through to the previous single-layout storage.
    }
  }
  return [{ id: 'default', name: 'Default', widgets: loadDashboardLayout() }];
}

function loadActiveLayoutId(): string {
  if (typeof window === 'undefined') return 'default';
  try {
    return (
      window.localStorage.getItem(DASHBOARD_ACTIVE_LAYOUT_KEY) ?? 'default'
    );
  } catch {
    return 'default';
  }
}

export function FeedControls({
  connection,
  feedSource,
  connectionState,
  onFeedSourceChange,
}: FeedControlsProps) {
  const [dataView, setDataView] = useState<'markets' | 'users'>('markets');
  const [tradeRequest, setTradeRequest] = useState<{
    symbol: string;
    side: 'buy' | 'sell';
  }>();
  const openTrade = useCallback((symbol: string, side: 'buy' | 'sell' = 'buy') =>
    setTradeRequest({ symbol, side }), []);
  const [savedLayouts, setSavedLayouts] =
    useState<SavedDashboardLayout[]>(loadSavedLayouts);
  const [activeLayoutId, setActiveLayoutId] = useState(() => {
    const storedId = loadActiveLayoutId();
    return savedLayouts.some((layout) => layout.id === storedId)
      ? storedId
      : (savedLayouts[0]?.id ?? 'default');
  });
  const [dashboardWidgets, setDashboardWidgets] = useState<DashboardWidget[]>(
    () =>
      savedLayouts.find((layout) => layout.id === activeLayoutId)?.widgets ??
      savedLayouts[0]?.widgets ??
      defaultDashboardLayout(),
  );
  const [widgetStoreOpen, setWidgetStoreOpen] = useState(false);
  const [draggingWidgetId, setDraggingWidgetId] = useState<string | null>(null);
  const [dropPreview, setDropPreview] = useState<{
    widgetId: string;
    x: number;
    y: number;
  } | null>(null);
  const [layoutNameInput, setLayoutNameInput] = useState('');
  const [layoutRenameOpen, setLayoutRenameOpen] = useState(false);
  const priceChangesModel = useMemo(
    () => new PriceChangesModel(connection),
    [connection],
  );
  const [spotSymbols, setSpotSymbols] = useState<string[]>([]);
  useEffect(() => {
    let active = true;
    const loadSpotSymbols = async () => {
      try {
        const response = await fetch('/instruments');
        if (!response.ok) throw new Error('Could not load spot instruments');
        const data = await response.json() as { symbols?: unknown };
        if (active && Array.isArray(data.symbols)) {
          setSpotSymbols(data.symbols.filter((symbol): symbol is string => typeof symbol === 'string'));
        }
      } catch (error) {
        console.warn('Could not load the full OKX spot instrument list', error);
      }
    };
    void loadSpotSymbols();
    const timer = setInterval(() => void loadSpotSymbols(), 15 * 60 * 1000);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, []);
  const priceChanges = useBehaviorSnapshot(priceChangesModel.state$);
  const [dragPriceChanges, setDragPriceChanges] = useState<typeof priceChanges | null>(null);
  const widgetPriceChanges = dragPriceChanges ?? priceChanges;
  useEffect(() => {
    priceChangesModel.start();
    return () => priceChangesModel.stop();
  }, [priceChangesModel]);
  const bookSymbols = useMemo(
    () => widgetPriceChanges.ticks.map((tick) => tick.symbol),
    [widgetPriceChanges.ticks],
  );
  const sortedBookSymbols = useMemo(
    () => [...bookSymbols].sort((left, right) => left.localeCompare(right)),
    [bookSymbols],
  );
  const selectionSymbols = useMemo(
    () => feedSource === 'okx' && spotSymbols.length > 0 ? spotSymbols : sortedBookSymbols,
    [feedSource, spotSymbols, sortedBookSymbols],
  );
  const selectedNewWidgetSymbol = bookSymbols[0] ?? selectionSymbols[0] ?? '';

  useEffect(() => {
    setSavedLayouts((current) => {
      const index = current.findIndex((layout) => layout.id === activeLayoutId);
      if (index < 0 || current[index].widgets === dashboardWidgets)
        return current;
      return current.map((layout) =>
        layout.id === activeLayoutId
          ? { ...layout, widgets: dashboardWidgets }
          : layout,
      );
    });
    try {
      window.localStorage.setItem(
        DASHBOARD_LAYOUT_KEY,
        JSON.stringify({ version: 2, widgets: dashboardWidgets }),
      );
      window.localStorage.setItem(DASHBOARD_ACTIVE_LAYOUT_KEY, activeLayoutId);
    } catch {
      // Keep the dashboard usable when browser storage is unavailable.
    }
  }, [dashboardWidgets, activeLayoutId]);

  useEffect(() => {
    try {
      window.localStorage.setItem(
        DASHBOARD_LAYOUTS_KEY,
        JSON.stringify(savedLayouts),
      );
    } catch {
      // Keep the dashboard usable when browser storage is unavailable.
    }
  }, [savedLayouts]);

  useEffect(() => {
    if (!selectedNewWidgetSymbol) return;
    setDashboardWidgets((current) => {
      if (
        !current.some(
          (widget) =>
            (widget.type === 'chart' ||
              widget.type === 'book' ||
              widget.type === 'trades') &&
            !widget.symbol,
        )
      )
        return current;
      return current.map((widget) =>
        (widget.type === 'chart' ||
          widget.type === 'book' ||
          widget.type === 'trades') &&
        !widget.symbol
          ? { ...widget, symbol: selectedNewWidgetSymbol }
          : widget,
      );
    });
  }, [selectedNewWidgetSymbol]);

  const chooseLayout = (id: string) => {
    const layout = savedLayouts.find((item) => item.id === id);
    if (!layout) return;
    setActiveLayoutId(id);
    setDashboardWidgets(layout.widgets);
  };
  const saveLayout = () => {
    const name = layoutNameInput.trim();
    if (!name) return;
    const existing = savedLayouts.find(
      (layout) => layout.name.toLocaleLowerCase() === name.toLocaleLowerCase(),
    );
    const id =
      existing?.id ??
      Date.now().toString(36) + '-' + Math.random().toString(36).slice(2);
    const snapshot = dashboardWidgets.map((widget) => ({ ...widget }));
    setSavedLayouts((current) =>
      existing
        ? current.map((layout) =>
            layout.id === id ? { ...layout, name, widgets: snapshot } : layout,
          )
        : [...current, { id, name, widgets: snapshot }],
    );
    setActiveLayoutId(id);
    setLayoutNameInput('');
  };
  const renameLayout = () => {
    const name = layoutNameInput.trim();
    if (!name) return;
    const duplicate = savedLayouts.some(
      (layout) =>
        layout.id !== activeLayoutId &&
        layout.name.toLocaleLowerCase() === name.toLocaleLowerCase(),
    );
    if (duplicate) return;
    setSavedLayouts((current) =>
      current.map((layout) =>
        layout.id === activeLayoutId ? { ...layout, name } : layout,
      ),
    );
    setLayoutNameInput('');
    setLayoutRenameOpen(false);
  };
  const addWidget = (type: DashboardWidgetType) => {
    setDashboardWidgets((current) => {
      const position = firstOpenPosition(current);
      return [
        ...current,
        makeWidget(
          type,
          type === 'chart' || type === 'book' || type === 'trades'
            ? selectedNewWidgetSymbol
            : '',
          position.x,
          position.y,
        ),
      ];
    });
  };
  const removeWidget = (id: string) => {
    setDashboardWidgets((current) =>
      current.filter((widget) => widget.id !== id),
    );
  };
  const updateWidgetSymbol = (id: string, symbol: string) => {
    setDashboardWidgets((current) =>
      current.map((widget) =>
        widget.id === id ? { ...widget, symbol } : widget,
      ),
    );
  };
  const getLandingPosition = (
    sourceId: string,
    clientX: number,
    clientY: number,
    widgets: DashboardWidget[],
    dashboard: HTMLElement,
  ) => {
    const source = widgets.find((widget) => widget.id === sourceId);
    if (!source) return null;
    const rect = dashboard.getBoundingClientRect();
    const styles = window.getComputedStyle(dashboard);
    const gap = parseFloat(styles.columnGap) || 12;
    const rowHeight = parseFloat(styles.gridAutoRows) || 22;
    const unitWidth =
      (rect.width - gap * (DASHBOARD_COLUMNS - 1)) / DASHBOARD_COLUMNS + gap;
    const unitHeight = rowHeight + gap;
    const x = Math.floor(
      (clientX - rect.left + dashboard.scrollLeft) / unitWidth,
    );
    const y = Math.floor(
      (clientY - rect.top + dashboard.scrollTop) / unitHeight,
    );
    return findFreeLanding(source, x, y);
  };
  const moveWidget = (sourceId: string, x: number, y: number) => {
    setDashboardWidgets((current) =>
      shiftWidgetsForAnchor(
        current.map((widget) =>
          widget.id === sourceId ? { ...widget, x, y } : widget,
        ),
        sourceId,
      ),
    );
  };
  const resizeWidget = (id: string, width: number, height: number) => {
    setDashboardWidgets((current) => {
      const widget = current.find((item) => item.id === id);
      if (!widget) return current;
      const nextWidth = Math.max(4, Math.min(DASHBOARD_COLUMNS, width));
      const nextHeight = Math.max(4, height);
      const nextX = Math.min(widget.x, DASHBOARD_COLUMNS - nextWidth);
      if (
        widget.w === nextWidth &&
        widget.h === nextHeight &&
        widget.x === nextX
      )
        return current;
      const candidate = { ...widget, x: nextX, w: nextWidth, h: nextHeight };
      return packDashboardWidgets(
        current.map((item) => (item.id === id ? candidate : item)),
        id,
      );
    });
  };

  const renderWidget = (
    widget: DashboardWidget,
    compact: boolean,
  ): ReactNode => {
    const symbol = widget.symbol || selectedNewWidgetSymbol;
    let content: ReactNode;
    switch (widget.type) {
      case 'grid':
        content = (
          <SymbolPriceGrid
            ticks={widgetPriceChanges.ticks}
            feedSource={feedSource}
            compact={compact}
            onSelectSymbol={openTrade}
          />
        );
        break;
      case 'table':
        content = (
          <SymbolPriceTable
            ticks={widgetPriceChanges.ticks}
            histories={widgetPriceChanges.histories}
            state={connectionState}
            onSelectSymbol={openTrade}
          />
        );
        break;
      case 'chart':
        content = (
          <PriceChangeChart
            changes={widgetPriceChanges}
            selectedSymbol={symbol}
            onSymbolChange={(value) => updateWidgetSymbol(widget.id, value)}
            showSymbolSelector
            onTrade={(side) => symbol && openTrade(symbol, side)}
          />
        );
        break;
      case 'positions':
        content = (
          <Positions connection={connection} symbols={selectionSymbols} paused={draggingWidgetId !== null} />
        );
        break;
      case 'fills':
        content = <Fills paused={draggingWidgetId !== null} />;
        break;
      case 'pnl':
        content = <Pnl connection={connection} paused={draggingWidgetId !== null} />;
        break;
      case 'account':
        content = <Account />;
        break;
      case 'risk':
        content = <PortfolioRisk connection={connection} paused={draggingWidgetId !== null} />;
        break;
      case 'close-insights':
        content = <CloseInsights histories={widgetPriceChanges.histories} paused={draggingWidgetId !== null} />;
        break;
      case 'hedging':
        content = <Hedging paused={draggingWidgetId !== null} />;
        break;
      case 'trades':
        content =
          feedSource === 'okx' ? (
            <TradesView
              streamKey={widget.id}
              symbol={symbol}
              symbols={selectionSymbols}
              onSymbolChange={(value) => updateWidgetSymbol(widget.id, value)}
              connection={connection}
              paused={draggingWidgetId !== null}
            />
          ) : (
            <section className="feed-section" aria-label="Recent trades">
              <div className="book-popup-message">
                Recent trade data is available with the OKX feed.
              </div>
            </section>
          );
        break;
      case 'liquidity':
        content = feedSource === 'okx' ? (
          <MarketLiquidity streamKey={widget.id} symbol={symbol} symbols={selectionSymbols} onSymbolChange={(value) => updateWidgetSymbol(widget.id, value)} connection={connection} paused={draggingWidgetId !== null} />
        ) : (
          <section className="feed-section" aria-label="Market liquidity"><div className="book-popup-message">Market liquidity indicators are available with the OKX feed.</div></section>
        );
        break;
      case 'signals':
        content = <BestSignalsView onTradeSymbol={openTrade} />;
        break;
      case 'signal-history':
        content = <SignalHistoryView paused={draggingWidgetId !== null} />;
        break;
      case 'signal-performance':
        content = <SignalPerformanceView paused={draggingWidgetId !== null} />;
        break;
      case 'book':
        content =
          feedSource === 'okx' ? (
            <OrderBookPopup
              streamKey={widget.id}
              mode="view"
              symbol={symbol}
              symbols={selectionSymbols}
              onSymbolChange={(value) => updateWidgetSymbol(widget.id, value)}
              showSymbolSelector
              onTrade={openTrade}
              connection={connection}
            />
          ) : (
            <section className="feed-section" aria-label="Order book">
              <div className="book-popup-message">
                Order book depth is available with the OKX feed.
              </div>
            </section>
          );
        break;
    }

    if (!compact) return content;
    const description =
      WIDGET_LABELS[widget.type] +
      (widget.type === 'chart' ||
      widget.type === 'book' ||
      widget.type === 'trades' ||
      widget.type === 'liquidity' ||
      widget.type === 'signals'
        ? ' for ' + symbol
        : '');
    return (
      <div
        className="dashboard-widget"
        key={widget.id}
        style={{
          gridColumn:
            widget.x +
            1 +
            ' / span ' +
            Math.min(widget.w, DASHBOARD_COLUMNS - widget.x),
          gridRow: widget.y + 1 + ' / span ' + widget.h,
        }}
        data-dragging={draggingWidgetId === widget.id || undefined}
      >
        <button
          className="dashboard-widget-drag"
          type="button"
          draggable
          aria-label={'Drag ' + description + ' widget'}
          title="Drag to rearrange"
          onDragStart={(event) => {
            event.dataTransfer.setData('text/widget-id', widget.id);
            event.dataTransfer.setData('text/plain', widget.id);
            event.dataTransfer.effectAllowed = 'move';
            const ghost = document.createElement('div');
            ghost.className = 'dashboard-drag-ghost';
            ghost.textContent = description;
            document.body.appendChild(ghost);
            event.dataTransfer.setDragImage(ghost, 18, 18);
            window.setTimeout(() => ghost.remove(), 0);
            setDragPriceChanges(priceChanges);
            setDraggingWidgetId(widget.id);
          }}
          onDragEnd={() => {
            setDragPriceChanges(null);
            setDraggingWidgetId(null);
            setDropPreview(null);
          }}
        >
          <span className="dashboard-widget-drag-icon" aria-hidden="true" />
        </button>
        <button
          className="dashboard-widget-remove"
          type="button"
          aria-label={'Remove ' + description + ' widget'}
          title="Remove widget"
          onClick={() => removeWidget(widget.id)}
        >
          <span className="dashboard-widget-remove-icon" aria-hidden="true" />
        </button>
        {content}
        <div
          className="dashboard-widget-resize"
          role="separator"
          aria-label={'Resize ' + description + ' widget'}
          title="Drag to resize"
          onPointerDown={(event) => {
            event.preventDefault();
            const startX = event.clientX;
            const startY = event.clientY;
            const startW = widget.w;
            const startH = widget.h;
            const dashboard = event.currentTarget.closest('.market-dashboard');
            if (!dashboard) return;
            const rect = dashboard.getBoundingClientRect();
            const styles = window.getComputedStyle(dashboard);
            const gap = parseFloat(styles.columnGap) || 12;
            const rowHeight = parseFloat(styles.gridAutoRows) || 22;
            const unitW =
              (rect.width - gap * (DASHBOARD_COLUMNS - 1)) / DASHBOARD_COLUMNS +
              gap;
            const unitH = rowHeight + gap;
            const handle = event.currentTarget;
            handle.setPointerCapture(event.pointerId);
            const move = (pointerEvent: PointerEvent) =>
              resizeWidget(
                widget.id,
                startW + Math.round((pointerEvent.clientX - startX) / unitW),
                startH + Math.round((pointerEvent.clientY - startY) / unitH),
              );
            const up = () => {
              handle.removeEventListener('pointermove', move);
              handle.removeEventListener('pointerup', up);
              handle.removeEventListener('pointercancel', up);
              if (handle.hasPointerCapture(event.pointerId))
                handle.releasePointerCapture(event.pointerId);
            };
            handle.addEventListener('pointermove', move);
            handle.addEventListener('pointerup', up);
            handle.addEventListener('pointercancel', up);
          }}
        />
      </div>
    );
  };

  return (
    <>
      <nav className="feed-controls" aria-label="Main navigation">
        <div className="view-switcher" role="group" aria-label="Main sections">
          <button
            type="button"
            aria-pressed={dataView === 'markets'}
            onClick={() => setDataView('markets')}
          >
            Markets
          </button>
          <button
            type="button"
            aria-pressed={dataView === 'users'}
            onClick={() => setDataView('users')}
          >
            Users
          </button>
        </div>
        {dataView === 'markets' && (
          <div
            className="view-switcher"
            role="group"
            aria-label="Choose price feed"
          >
            <button
              type="button"
              aria-pressed={feedSource === 'okx'}
              onClick={() => onFeedSourceChange('okx')}
            >
              OKX
            </button>
            <button
              type="button"
              aria-pressed={feedSource === 'synthetic'}
              onClick={() => onFeedSourceChange('synthetic')}
            >
              Synthetic · Fast
            </button>
          </div>
        )}
        {dataView === 'markets' && (
          <div className="workspace-controls">
            <label className="workspace-selector">
              <span>WORKSPACE</span>
              <select
                aria-label="Choose workspace layout"
                value={activeLayoutId}
                onChange={(event) => chooseLayout(event.target.value)}
              >
                {savedLayouts.map((layout) => (
                  <option key={layout.id} value={layout.id}>
                    {layout.name}
                  </option>
                ))}
              </select>
            </label>
            <button
              className="workspace-save-button"
              type="button"
              aria-label="Rename workspace layout"
              onClick={() => {
                const activeLayout = savedLayouts.find(
                  (layout) => layout.id === activeLayoutId,
                );
                if (!activeLayout) return;
                setLayoutNameInput(activeLayout.name);
                setLayoutRenameOpen(true);
              }}
            >
              Rename
            </button>
            <input
              className="workspace-name-input"
              aria-label="Workspace layout name"
              placeholder="Save layout as..."
              value={layoutNameInput}
              onChange={(event) => setLayoutNameInput(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') saveLayout();
              }}
            />
            <button
              className="workspace-save-button"
              type="button"
              onClick={saveLayout}
              disabled={!layoutNameInput.trim()}
            >
              Save
            </button>
          </div>
        )}
        {dataView === 'markets' && (
          <div className="widget-store-menu">
            <button
              className="widget-store-toggle"
              type="button"
              aria-label="Add widget"
              aria-expanded={widgetStoreOpen}
              aria-controls="widget-store-dropdown"
              onClick={() => setWidgetStoreOpen((open) => !open)}
            >
              +
            </button>
            {widgetStoreOpen && (
              <div
                className="widget-store-dropdown"
                id="widget-store-dropdown"
                aria-label="Add a dashboard widget"
              >
                {DASHBOARD_WIDGET_TYPES.map((type) => (
                  <button
                    type="button"
                    key={type}
                    onClick={() => {
                      addWidget(type);
                      setWidgetStoreOpen(false);
                    }}
                  >
                    Add {WIDGET_LABELS[type]}
                  </button>
                ))}
              </div>
            )}
          </div>
        )}
      </nav>

      {dataView === 'markets' ? (
        <div
          className="market-dashboard"
          onDragOver={(event) => {
            event.preventDefault();
            event.dataTransfer.dropEffect = 'move';
            const widgetId =
              event.dataTransfer.getData('text/widget-id') ||
              draggingWidgetId ||
              '';
            if (!widgetId) return;
            const landing = getLandingPosition(
              widgetId,
              event.clientX,
              event.clientY,
              dashboardWidgets,
              event.currentTarget,
            );
            if (landing)
              setDropPreview((current) =>
                current &&
                current.widgetId === widgetId &&
                current.x === landing.x &&
                current.y === landing.y
                  ? current
                  : { widgetId, x: landing.x, y: landing.y },
              );
          }}
          onDragLeave={(event) => {
            if (
              !event.currentTarget.contains(event.relatedTarget as Node | null)
            )
              setDropPreview(null);
          }}
          onDrop={(event) => {
            event.preventDefault();
            const widgetId =
              event.dataTransfer.getData('text/widget-id') ||
              draggingWidgetId ||
              '';
            const landing = getLandingPosition(
              widgetId,
              event.clientX,
              event.clientY,
              dashboardWidgets,
              event.currentTarget,
            );
            if (landing) moveWidget(widgetId, landing.x, landing.y);
            setDragPriceChanges(null);
            setDraggingWidgetId(null);
            setDropPreview(null);
          }}
        >
          {dashboardWidgets.map((widget) => renderWidget(widget, true))}
          {dropPreview &&
            dashboardWidgets.some(
              (widget) => widget.id === dropPreview.widgetId,
            ) && (
              <div
                className="dashboard-drop-preview"
                aria-hidden="true"
                style={{
                  gridColumn:
                    dropPreview.x +
                    1 +
                    ' / span ' +
                    (dashboardWidgets.find(
                      (widget) => widget.id === dropPreview.widgetId,
                    )?.w ?? DEFAULT_WIDGET_WIDTH),
                  gridRow:
                    dropPreview.y +
                    1 +
                    ' / span ' +
                    (dashboardWidgets.find(
                      (widget) => widget.id === dropPreview.widgetId,
                    )?.h ?? DEFAULT_WIDGET_HEIGHT),
                }}
              />
            )}
          {dashboardWidgets.length === 0 && (
            <div className="dashboard-empty-state">
              <strong>Your dashboard is empty</strong>
              <span>Open Widgets to add a panel.</span>
            </div>
          )}
        </div>
      ) : (
        <Users />
      )}
      {layoutRenameOpen && (
        <div
          className="workspace-rename-backdrop"
          role="presentation"
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) setLayoutRenameOpen(false);
          }}
        >
          <section
            className="workspace-rename-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="workspace-rename-title"
          >
            <h2 id="workspace-rename-title">Rename workspace layout</h2>
            <form
              onSubmit={(event) => {
                event.preventDefault();
                renameLayout();
              }}
            >
              <label htmlFor="workspace-rename-input">Layout name</label>
              <input
                id="workspace-rename-input"
                autoFocus
                value={layoutNameInput}
                onChange={(event) => setLayoutNameInput(event.target.value)}
              />
              <div className="workspace-rename-actions">
                <button type="button" onClick={() => setLayoutRenameOpen(false)}>
                  Cancel
                </button>
                <button type="submit" disabled={!layoutNameInput.trim()}>
                  Rename
                </button>
              </div>
            </form>
          </section>
        </div>
      )}
      {tradeRequest && (
        <TradeTicketPopup
          key={tradeRequest.symbol + tradeRequest.side}
          symbol={tradeRequest.symbol}
          initialSide={tradeRequest.side}
          connection={connection}
          onClose={() => setTradeRequest(undefined)}
        />
      )}
    </>
  );
}
