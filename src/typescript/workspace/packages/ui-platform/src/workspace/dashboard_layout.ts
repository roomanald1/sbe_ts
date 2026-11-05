export type DashboardWidgetType = string;
export type DashboardWidget<TType extends string = string> = {
  id: string;
  type: TType;
  symbol: string;
  x: number;
  y: number;
  w: number;
  h: number;
  collapsed: boolean;
};
export type SavedDashboardLayout<TType extends string = string> = {
  id: string;
  name: string;
  widgets: DashboardWidget<TType>[];
};
export type DashboardLayoutConfig<TType extends string> = {
  widgetTypes: readonly TType[];
  defaultWidgetTypes: readonly TType[];
  storageKeys: { layout: string; layouts: string; activeLayout: string };
};
export const DASHBOARD_COLUMNS = 24;
export const DEFAULT_WIDGET_WIDTH = DASHBOARD_COLUMNS / 2;
export const DEFAULT_WIDGET_HEIGHT = 12;

export function makeWidget<TType extends string>(
  type: TType,
  symbol = '',
  x = 0,
  y = 0,
): DashboardWidget<TType> {
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

export function defaultDashboardLayout<TType extends string>(
  types: readonly TType[],
): DashboardWidget<TType>[] {
  return types.map((type, index) => ({
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

export function overlaps<TType extends string>(
  a: DashboardWidget<TType>,
  b: DashboardWidget<TType>,
): boolean {
  return (
    a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y
  );
}

export function firstOpenPosition<TType extends string>(
  widgets: DashboardWidget<TType>[],
): {
  x: number;
  y: number;
} {
  for (let y = 0; ; y += 1) {
    for (let x = 0; x <= DASHBOARD_COLUMNS - DEFAULT_WIDGET_WIDTH; x += 1) {
      const candidate = {
        id: '',
        type: '' as TType,
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

export function findFreeLanding<TType extends string>(
  widget: DashboardWidget<TType>,
  x: number,
  y: number,
): { x: number; y: number } {
  return {
    x: Math.max(0, Math.min(DASHBOARD_COLUMNS - widget.w, x)),
    y: Math.max(0, y),
  };
}

export function shiftWidgetsForAnchor<TType extends string>(
  widgets: DashboardWidget<TType>[],
  anchorId: string,
): DashboardWidget<TType>[] {
  const anchor = widgets.find((widget) => widget.id === anchorId);
  if (!anchor) return widgets;
  const placed: DashboardWidget<TType>[] = [
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

export function packDashboardWidgets<TType extends string>(
  widgets: DashboardWidget<TType>[],
  anchorId?: string,
): DashboardWidget<TType>[] {
  const packed: DashboardWidget<TType>[] = [];
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
