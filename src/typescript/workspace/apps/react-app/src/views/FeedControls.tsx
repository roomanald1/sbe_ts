import { getInstrumentList } from '../instrument_precision';
import { createPortal } from 'react-dom';
import { Fragment, useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import type { Connection, ConnectionState, FeedSource } from '../connection';
import { Users } from './Users';
import { TradeTicketPopup } from './TradeTicketPopup';
import { RangeOrderPopup } from './RangeOrderPopup';
import { PriceChangesModel } from '../price_change_data';
import { DashboardWidgetContent } from './DashboardWidgetContent';
import { DashboardWidgetFrame } from './DashboardWidgetFrame';
import { DashboardToolbar } from './DashboardToolbar';
import { useDashboardLayout } from './useDashboardLayout';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import {
  DEFAULT_WIDGET_HEIGHT,
  DEFAULT_WIDGET_WIDTH,
  type DashboardWidget,
  WIDGET_LABELS,
} from './dashboard_layout';
type FeedControlsProps = {
  connection: Connection;
  feedSource: FeedSource;
  connectionState: ConnectionState;
  onFeedSourceChange: (source: FeedSource) => void;
};

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
  const [rangeOrderRequest, setRangeOrderRequest] = useState<{ symbol: string; lower: number; upper: number }>();
  const openTrade = useCallback((symbol: string, side: 'buy' | 'sell' = 'buy') =>
    setTradeRequest({ symbol, side }), []);
  const dropPreviewRef = useRef<HTMLDivElement>(null);
  const priceChangesModel = useMemo(
    () => new PriceChangesModel(connection),
    [connection],
  );
  const [spotSymbols, setSpotSymbols] = useState<string[]>([]);
  useEffect(() => {
    let active = true;
    const loadSpotSymbols = async () => {
      try {
        const data = await getInstrumentList();
        if (active) setSpotSymbols(data.symbols);
      } catch (error) {
        console.warn('Could not load the full OKX spot instrument list', error);
      }
    };
    void loadSpotSymbols();
    return () => {
      active = false;
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

  const layout = useDashboardLayout(selectedNewWidgetSymbol);
  const {
    savedLayouts, activeLayoutId, widgets: dashboardWidgets, setWidgets: setDashboardWidgets,
    widgetStoreOpen, setWidgetStoreOpen, draggingWidgetId, setDraggingWidgetId,
    resizePreview, setResizePreview, layoutNameInput, setLayoutNameInput,
    layoutRenameOpen, setLayoutRenameOpen, displayWidgets: displayWidgetLayout,
    chooseLayout, saveLayout, renameLayout, addWidget, removeWidget,
    updateWidgetSymbol, moveWidget, resizeWidget, getLandingPosition,
  } = layout;

  const renderWidget = (
    widget: DashboardWidget,
    compact: boolean,
    layoutWidget: DashboardWidget = widget,
  ): ReactNode => {
    const symbol = widget.symbol || selectedNewWidgetSymbol;
    const content = <DashboardWidgetContent
      widget={widget}
      symbol={symbol}
      prices={widgetPriceChanges}
      connection={connection}
      feedSource={feedSource}
      connectionState={connectionState}
      selectionSymbols={selectionSymbols}
      paused={draggingWidgetId !== null}
      onTrade={openTrade}
      onRangeOrders={setRangeOrderRequest}
      onSymbolChange={updateWidgetSymbol}
    />;
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
    return <DashboardWidgetFrame
      widget={widget}
      layout={layoutWidget}
      description={description}
      dragging={draggingWidgetId === widget.id}
      connected={connectionState === 'connected'}
      onDragStart={(dragged) => { setDragPriceChanges(priceChanges); setDraggingWidgetId(dragged.id); }}
      onDragEnd={() => { setDragPriceChanges(null); setDraggingWidgetId(null); if (dropPreviewRef.current) dropPreviewRef.current.style.display = 'none'; }}
      onToggle={(id) => setDashboardWidgets((current) => current.map((item) => item.id === id ? { ...item, collapsed: !item.collapsed } : item))}
      onRemove={removeWidget}
      onResize={resizeWidget}
      onResizePreview={setResizePreview}
    >{content}</DashboardWidgetFrame>;
  };
  return (
    <>
      <DashboardToolbar
        dataView={dataView}
        onDataViewChange={setDataView}
        feedSource={feedSource}
        onFeedSourceChange={onFeedSourceChange}
        layouts={savedLayouts}
        activeLayoutId={activeLayoutId}
        onChooseLayout={chooseLayout}
        name={layoutNameInput}
        onNameChange={setLayoutNameInput}
        onSave={saveLayout}
        onRenameStart={() => {
          const current = savedLayouts.find((layout) => layout.id === activeLayoutId);
          if (!current) return;
          setLayoutNameInput(current.name);
          setLayoutRenameOpen(true);
        }}
        widgetStoreOpen={widgetStoreOpen}
        onWidgetStoreToggle={() => setWidgetStoreOpen((open) => !open)}
        onAddWidget={(type) => { addWidget(type); setWidgetStoreOpen(false); }}
      />

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
              event.currentTarget,
            );
            const preview = dropPreviewRef.current;
            if (preview) {
              if (!landing) {
                preview.style.display = 'none';
              } else {
                preview.style.display = 'block';
                const draggedWidget = dashboardWidgets.find((widget) => widget.id === widgetId);
                preview.dataset.collapsed = String(draggedWidget?.collapsed === true);
                preview.style.gridColumn = landing.x + 1 + ' / span ' + (draggedWidget?.w ?? DEFAULT_WIDGET_WIDTH);
                preview.style.gridRow = landing.y + 1 + ' / span ' + (draggedWidget?.collapsed ? 2 : draggedWidget?.h ?? DEFAULT_WIDGET_HEIGHT);
              }
            }
          }}
          onDragLeave={(event) => {
            if (
              !event.currentTarget.contains(event.relatedTarget as Node | null)
            )
              if (dropPreviewRef.current) dropPreviewRef.current.style.display = 'none';
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
              event.currentTarget,
            );
            if (landing) moveWidget(widgetId, landing.x, landing.y);
            setDragPriceChanges(null);
            setDraggingWidgetId(null);
            if (dropPreviewRef.current) dropPreviewRef.current.style.display = 'none';
          }}
        >
          {dashboardWidgets.map((widget) =>
            <Fragment key={widget.id}>{renderWidget(
              widget,
              true,
              displayWidgetLayout.find((item) => item.id === widget.id) ?? widget,
            )}</Fragment>,
          )}
          {resizePreview && (
            <div
              className="dashboard-resize-preview"
              aria-hidden="true"
              style={{
                gridColumn: resizePreview.x + 1 + ' / span ' + resizePreview.w,
                gridRow: resizePreview.y + 1 + ' / span ' + resizePreview.h,
              }}
            />
          )}
          <div ref={dropPreviewRef} className="dashboard-drop-preview" aria-hidden="true" style={{ display: 'none' }} />
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
      {layoutRenameOpen && createPortal(
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
        </div>,
        document.body,
      )}
      {rangeOrderRequest && <RangeOrderPopup request={rangeOrderRequest} onClose={() => setRangeOrderRequest(undefined)} />}
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
