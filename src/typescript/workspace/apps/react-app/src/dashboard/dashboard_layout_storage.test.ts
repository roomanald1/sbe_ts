import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import {
  loadDashboardLayout,
  loadSavedLayouts,
} from '../../../../packages/ui-platform/src/workspace/dashboard_layout_storage.ts';
import type { DashboardLayoutConfig } from '../../../../packages/ui-platform/src/workspace/dashboard_layout.ts';
import { UnavailableDashboardWidget } from '../../../../packages/ui-platform/src/components/UnavailableDashboardWidget.tsx';

const config: DashboardLayoutConfig<'grid'> = {
  widgetTypes: ['grid'],
  defaultWidgetTypes: ['grid'],
  storageKeys: {
    layout: 'layout',
    layouts: 'layouts',
    activeLayout: 'active',
  },
};

function withLocalStorage<T>(values: Record<string, string>, run: () => T): T {
  const prior = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const writes: Array<[string, string]> = [];
  const localStorage = {
    getItem: (key: string) => values[key] ?? null,
    setItem: (key: string, value: string) => {
      writes.push([key, value]);
    },
    removeItem: (key: string) => {
      delete values[key];
    },
    clear: () => {
      for (const key of Object.keys(values)) delete values[key];
    },
    key: (index: number) => Object.keys(values)[index] ?? null,
    get length() {
      return Object.keys(values).length;
    },
  };
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: { localStorage },
  });
  try {
    const result = run();
    assert.deepEqual(writes, [], 'loading must not rewrite saved layouts');
    return result;
  } finally {
    if (prior) Object.defineProperty(globalThis, 'window', prior);
    else Reflect.deleteProperty(globalThis, 'window');
  }
}

describe('saved dashboard layout compatibility', () => {
  it('preserves unknown widget types, extra configuration and exact geometry', () => {
    const widget = {
      id: 'retired-widget-1',
      type: 'legacy-outcomes',
      symbol: 'BTC-USDT',
      x: 17,
      y: 6,
      w: 7,
      h: 11,
      collapsed: true,
      filters: { strategy: 'risk-adjusted-monthly-momentum' },
      customConfig: { selectedTab: 'closed', density: 'compact' },
    };
    const saved = {
      id: 'saved-view-1',
      name: 'Legacy desk',
      description: 'Retained for operator review',
      widgets: [widget],
    };

    const layouts = withLocalStorage({ layouts: JSON.stringify([saved]) }, () =>
      loadSavedLayouts(config),
    );

    assert.equal(layouts.length, 1);
    assert.deepEqual(layouts[0], saved);
  });

  it('preserves unsupported widgets from the active version-two layout', () => {
    const widget = {
      id: 'retired-active-widget',
      type: 'legacy-fees',
      symbol: '',
      x: 12,
      y: 9,
      w: 6,
      h: 8,
      collapsed: false,
      filters: { currency: 'USDT' },
    };
    const layout = withLocalStorage(
      { layout: JSON.stringify({ version: 2, widgets: [widget] }) },
      () => loadDashboardLayout(config),
    );

    assert.deepEqual(layout, [widget]);
  });

  it('renders a manual-replacement placeholder for a retained unknown type', () => {
    const html = renderToStaticMarkup(
      createElement(UnavailableDashboardWidget, { type: 'legacy-fees' }),
    );

    assert.match(html, /Saved widget unavailable/);
    assert.match(html, /legacy-fees/);
    assert.match(html, /Preserved\. Replace or remove this widget manually\./);
  });
});
