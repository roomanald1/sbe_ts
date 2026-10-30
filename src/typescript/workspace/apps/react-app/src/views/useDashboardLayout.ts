import { useCallback, useEffect, useState } from 'react';
import {
  DASHBOARD_COLUMNS, DASHBOARD_LAYOUT_KEY, DASHBOARD_LAYOUTS_KEY,
  DASHBOARD_ACTIVE_LAYOUT_KEY, type DashboardWidget, type DashboardWidgetType,
  type SavedDashboardLayout, defaultDashboardLayout, firstOpenPosition,
  findFreeLanding, makeWidget, packDashboardWidgets, shiftWidgetsForAnchor,
} from './dashboard_layout';
import { loadActiveLayoutId, loadSavedLayouts } from './dashboard_layout_storage';

type ResizePreview = { id: string; x: number; y: number; w: number; h: number };

export function useDashboardLayout(selectedSymbol: string) {
  const [savedLayouts, setSavedLayouts] = useState<SavedDashboardLayout[]>(loadSavedLayouts);
  const [activeLayoutId, setActiveLayoutId] = useState(() => {
    const savedId = loadActiveLayoutId();
    return savedLayouts.some(({ id }) => id === savedId) ? savedId : savedLayouts[0]?.id ?? 'default';
  });
  const [widgets, setWidgets] = useState<DashboardWidget[]>(() =>
    savedLayouts.find(({ id }) => id === activeLayoutId)?.widgets ?? savedLayouts[0]?.widgets ?? defaultDashboardLayout());
  const [widgetStoreOpen, setWidgetStoreOpen] = useState(false);
  const [draggingWidgetId, setDraggingWidgetId] = useState<string | null>(null);
  const [resizePreview, setResizePreview] = useState<ResizePreview | null>(null);
  const [layoutNameInput, setLayoutNameInput] = useState('');
  const [layoutRenameOpen, setLayoutRenameOpen] = useState(false);

  useEffect(() => {
    setSavedLayouts((current) => current.map((layout) => layout.id === activeLayoutId ? { ...layout, widgets } : layout));
    try {
      window.localStorage.setItem(DASHBOARD_LAYOUT_KEY, JSON.stringify({ version: 2, widgets }));
      window.localStorage.setItem(DASHBOARD_ACTIVE_LAYOUT_KEY, activeLayoutId);
    } catch { /* Browser storage is optional. */ }
  }, [widgets, activeLayoutId]);
  useEffect(() => {
    try { window.localStorage.setItem(DASHBOARD_LAYOUTS_KEY, JSON.stringify(savedLayouts)); }
    catch { /* Browser storage is optional. */ }
  }, [savedLayouts]);
  useEffect(() => {
    if (!selectedSymbol) return;
    setWidgets((current) => current.map((widget) =>
      ['chart', 'book', 'trades'].includes(widget.type) && !widget.symbol
        ? { ...widget, symbol: selectedSymbol } : widget));
  }, [selectedSymbol]);

  const chooseLayout = (id: string) => {
    const layout = savedLayouts.find((item) => item.id === id);
    if (!layout) return;
    setActiveLayoutId(id);
    setWidgets(layout.widgets);
  };
  const saveLayout = () => {
    const name = layoutNameInput.trim();
    if (!name) return;
    const existing = savedLayouts.find((layout) => layout.name.toLocaleLowerCase() === name.toLocaleLowerCase());
    const id = existing?.id ?? Date.now().toString(36) + '-' + Math.random().toString(36).slice(2);
    const snapshot = widgets.map((widget) => ({ ...widget }));
    setSavedLayouts((current) => existing
      ? current.map((layout) => layout.id === id ? { ...layout, name, widgets: snapshot } : layout)
      : [...current, { id, name, widgets: snapshot }]);
    setActiveLayoutId(id);
    setLayoutNameInput('');
  };
  const renameLayout = () => {
    const name = layoutNameInput.trim();
    if (!name || savedLayouts.some((layout) => layout.id !== activeLayoutId && layout.name.toLocaleLowerCase() === name.toLocaleLowerCase())) return;
    setSavedLayouts((current) => current.map((layout) => layout.id === activeLayoutId ? { ...layout, name } : layout));
    setLayoutNameInput('');
    setLayoutRenameOpen(false);
  };
  const addWidget = (type: DashboardWidgetType) => setWidgets((current) => {
    const position = firstOpenPosition(current);
    const symbol = ['chart', 'book', 'trades'].includes(type) ? selectedSymbol : '';
    return [...current, makeWidget(type, symbol, position.x, position.y)];
  });
  const removeWidget = (id: string) => setWidgets((current) => current.filter((widget) => widget.id !== id));
  const updateWidgetSymbol = useCallback((id: string, symbol: string) => setWidgets((current) => current.map((widget) => widget.id === id ? { ...widget, symbol } : widget)), []);
  const moveWidget = (id: string, x: number, y: number) => setWidgets((current) => shiftWidgetsForAnchor(current.map((widget) => widget.id === id ? { ...widget, x, y } : widget), id));
  const resizeWidget = (id: string, width: number, height: number) => setWidgets((current) => {
    const widget = current.find((item) => item.id === id);
    if (!widget) return current;
    const w = Math.max(4, Math.min(DASHBOARD_COLUMNS, width));
    const h = Math.max(4, height);
    const x = Math.min(widget.x, DASHBOARD_COLUMNS - w);
    if (widget.w === w && widget.h === h && widget.x === x) return current;
    return packDashboardWidgets(current.map((item) => item.id === id ? { ...item, x, y: widget.y, w, h } : item), id);
  });
  const getLandingPosition = (id: string, clientX: number, clientY: number, dashboard: HTMLElement) => {
    const widget = widgets.find((item) => item.id === id);
    if (!widget) return null;
    const rect = dashboard.getBoundingClientRect();
    const styles = window.getComputedStyle(dashboard);
    const gap = parseFloat(styles.columnGap) || 12;
    const rowHeight = parseFloat(styles.gridAutoRows) || 22;
    const unitWidth = (rect.width - gap * (DASHBOARD_COLUMNS - 1)) / DASHBOARD_COLUMNS + gap;
    const unitHeight = rowHeight + gap;
    return findFreeLanding(widget,
      Math.floor((clientX - rect.left + dashboard.scrollLeft) / unitWidth),
      Math.floor((clientY - rect.top + dashboard.scrollTop) / unitHeight));
  };
  const displayWidgets = widgets.some(({ collapsed }) => collapsed)
    ? packDashboardWidgets(widgets.map((widget) => ({ ...widget, h: widget.collapsed ? 2 : widget.h })))
    : widgets;

  return {
    savedLayouts, activeLayoutId, widgets, setWidgets, widgetStoreOpen, setWidgetStoreOpen,
    draggingWidgetId, setDraggingWidgetId, resizePreview, setResizePreview,
    layoutNameInput, setLayoutNameInput, layoutRenameOpen, setLayoutRenameOpen,
    displayWidgets, chooseLayout, saveLayout, renameLayout, addWidget, removeWidget,
    updateWidgetSymbol, moveWidget, resizeWidget, getLandingPosition,
  };
}
