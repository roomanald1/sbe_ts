import {
  DASHBOARD_ACTIVE_LAYOUT_KEY,
  DASHBOARD_WIDGET_TYPES,
  DASHBOARD_LAYOUT_KEY,
  DASHBOARD_LAYOUTS_KEY,
  DASHBOARD_COLUMNS,
  DEFAULT_WIDGET_HEIGHT,
  DEFAULT_WIDGET_WIDTH,
  type DashboardWidget,
  type DashboardWidgetType,
  type SavedDashboardLayout,
  defaultDashboardLayout,
  overlaps,
} from './dashboard_layout';

export function normalizeWidgets(items: unknown[], scale = 1): DashboardWidget[] {
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
        collapsed: false,
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
      collapsed?: unknown;
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
      collapsed: stored.collapsed === true,
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

export function loadDashboardLayout(): DashboardWidget[] {
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

export function loadSavedLayouts(): SavedDashboardLayout[] {
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

export function loadActiveLayoutId(): string {
  if (typeof window === 'undefined') return 'default';
  try {
    return (
      window.localStorage.getItem(DASHBOARD_ACTIVE_LAYOUT_KEY) ?? 'default'
    );
  } catch {
    return 'default';
  }
}

