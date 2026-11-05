import {
  DASHBOARD_COLUMNS,
  type DashboardLayoutConfig,
  type DashboardWidget,
  type SavedDashboardLayout,
  defaultDashboardLayout,
  overlaps,
} from './dashboard_layout.js';

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

/** Accepts only the current widget object shape, then clamps and packs its layout. */
export function validateWidgets<TType extends string>(
  items: unknown[],
  config: DashboardLayoutConfig<TType>,
): DashboardWidget<TType>[] {
  const widgets: DashboardWidget<TType>[] = [];
  items.forEach((item) => {
    if (!isRecord(item)) return;
    const { id, type, symbol, x, y, w, h, collapsed } = item;
    if (
      typeof id !== 'string' ||
      typeof type !== 'string' ||
      !config.widgetTypes.includes(type as TType) ||
      typeof symbol !== 'string' ||
      typeof x !== 'number' ||
      !Number.isFinite(x) ||
      typeof y !== 'number' ||
      !Number.isFinite(y) ||
      typeof w !== 'number' ||
      !Number.isFinite(w) ||
      typeof h !== 'number' ||
      !Number.isFinite(h) ||
      typeof collapsed !== 'boolean'
    )
      return;

    const width = Math.max(1, Math.min(DASHBOARD_COLUMNS, Math.floor(w)));
    widgets.push({
      id,
      type: type as TType,
      symbol,
      x: Math.max(0, Math.min(DASHBOARD_COLUMNS - width, Math.floor(x))),
      y: Math.max(0, Math.floor(y)),
      w: width,
      h: Math.max(3, Math.floor(h)),
      collapsed,
    });
  });

  const placed: DashboardWidget<TType>[] = [];
  [...widgets]
    .sort((a, b) => a.y - b.y || a.x - b.x)
    .forEach((widget) => {
      const candidate = { ...widget };
      while (placed.some((other) => overlaps(candidate, other)))
        candidate.y += 1;
      placed.push(candidate);
    });
  return widgets.map(
    (widget) => placed.find((item) => item.id === widget.id) ?? widget,
  );
}

function readVersionTwoWidgets<TType extends string>(
  value: unknown,
  config: DashboardLayoutConfig<TType>,
): DashboardWidget<TType>[] | null {
  if (!isRecord(value) || value.version !== 2 || !Array.isArray(value.widgets))
    return null;
  return validateWidgets(value.widgets, config);
}

export function loadDashboardLayout<TType extends string>(
  config: DashboardLayoutConfig<TType>,
): DashboardWidget<TType>[] {
  if (typeof window === 'undefined')
    return defaultDashboardLayout(config.defaultWidgetTypes);
  try {
    const saved = window.localStorage.getItem(config.storageKeys.layout);
    if (saved === null)
      return defaultDashboardLayout(config.defaultWidgetTypes);
    return (
      readVersionTwoWidgets(JSON.parse(saved), config) ??
      defaultDashboardLayout(config.defaultWidgetTypes)
    );
  } catch {
    return defaultDashboardLayout(config.defaultWidgetTypes);
  }
}

export function loadSavedLayouts<TType extends string>(
  config: DashboardLayoutConfig<TType>,
): SavedDashboardLayout<TType>[] {
  if (typeof window !== 'undefined') {
    try {
      const saved = window.localStorage.getItem(config.storageKeys.layouts);
      if (saved !== null) {
        const parsed: unknown = JSON.parse(saved);
        if (Array.isArray(parsed)) {
          const layouts: SavedDashboardLayout<TType>[] = parsed.flatMap(
            (item) => {
              if (
                !isRecord(item) ||
                typeof item.id !== 'string' ||
                typeof item.name !== 'string'
              )
                return [];
              const widgets = validateWidgets(
                Array.isArray(item.widgets) ? item.widgets : [],
                config,
              );
              return [{ id: item.id, name: item.name, widgets }];
            },
          );
          if (layouts.length > 0) return layouts;
        }
      }
    } catch {
      // Invalid current-format data is ignored.
    }
  }
  return [
    { id: 'default', name: 'Default', widgets: loadDashboardLayout(config) },
  ];
}

export function loadActiveLayoutId<TType extends string>(
  config: DashboardLayoutConfig<TType>,
): string {
  if (typeof window === 'undefined') return 'default';
  try {
    return (
      window.localStorage.getItem(config.storageKeys.activeLayout) ?? 'default'
    );
  } catch {
    return 'default';
  }
}
