import { after, before, describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { AutoTraderModel } from '../models/AutoTraderModel';
import {
  CapitalAllocationWidgetModel,
  type CapitalAllocationGroup,
  type CapitalAllocationTab,
  type CapitalAllocationWidgetState,
} from '../models/CapitalAllocationWidgetModel';
import {
  DASHBOARD_WIDGET_GROUPS,
  DASHBOARD_WIDGET_TYPES,
  WIDGET_LABELS,
} from '../dashboard/widget_catalog';
import type { ViteDevServer } from 'vite';

let viteServer: ViteDevServer;
let CapitalAllocationPanel: typeof import('./CapitalAllocationWidget').CapitalAllocationPanel;

before(async () => {
  viteServer = await (
    await import('vite')
  ).createServer({
    configFile: './vite.config.ts',
    server: { middlewareMode: true },
    appType: 'custom',
  });
  const module = await viteServer.ssrLoadModule(
    '/src/views/CapitalAllocationWidget.tsx',
  );
  CapitalAllocationPanel =
    module.CapitalAllocationPanel as typeof CapitalAllocationPanel;
});

after(async () => {
  await viteServer?.close();
});

function state(
  overrides: {
    capitalAllocation?: CapitalAllocationWidgetState['capitalAllocation'];
    availableNotional?: number;
    loading?: boolean;
    tab?: CapitalAllocationTab;
    groupBy?: CapitalAllocationGroup;
  } = {},
): CapitalAllocationWidgetState {
  const upstream = new AutoTraderModel();
  upstream.state$.next({
    ...upstream.state$.value,
    loading: overrides.loading ?? false,
    capitalAllocation: overrides.capitalAllocation,
    availableNotional: overrides.availableNotional ?? 500,
  });
  const model = new CapitalAllocationWidgetModel('render-fixture', {
    state$: upstream.state$,
    start: () => undefined,
    stop: () => undefined,
  });
  if (overrides.tab) model.setTab(overrides.tab);
  if (overrides.groupBy) model.setGroupBy(overrides.groupBy);
  return { ...model.state$.value, active: true };
}

function panel(widgetState: CapitalAllocationWidgetState) {
  return createElement(CapitalAllocationPanel, {
    state: widgetState,
    onSetTab: () => undefined,
    onSetGroupBy: () => undefined,
  });
}

const carryAllocation: NonNullable<
  CapitalAllocationWidgetState['capitalAllocation']
> = {
  version: 'v1',
  model: 'shared_pool',
  poolLimit: 500,
  used: 75,
  reserved: 25,
  available: 400,
  rows: [
    {
      strategy: 'dated-futures-carry',
      symbol: 'BTC-USD-20261225',
      horizon: 'carry',
      effectiveAllocation: 100,
      reserved: 25,
      used: 75,
      utilization: 1,
      evidenceEligible: true,
      state: 'committed_carry',
      reason: 'Committed dated carry exposure',
    },
  ],
};

describe('Capital Allocation dashboard widget', () => {
  it('is independently available in the widget store and Auto Trader group', () => {
    assert.ok(DASHBOARD_WIDGET_TYPES.includes('capital-allocation'));
    assert.ok(
      DASHBOARD_WIDGET_GROUPS.some((group) =>
        group.types.includes('capital-allocation'),
      ),
    );
    assert.equal(WIDGET_LABELS['capital-allocation'], 'Capital allocation');
  });

  it('renders compact shared-pool totals and source-backed candidate detail', () => {
    const markup = renderToStaticMarkup(
      panel(
        state({
          capitalAllocation: carryAllocation,
          availableNotional: 400,
        }),
      ),
    );
    assert.match(markup, /Autonomous/);
    assert.doesNotMatch(markup, /<h2[^>]*>Autonomous capital allocation/);
    assert.match(
      markup,
      /role="img" aria-label="Shared capital pool composition/,
    );
    assert.match(markup, /<dt[^>]*>Pool<\/dt><dd[^>]*>\$500\.00<\/dd>/);
    assert.match(markup, /<dt[^>]*>Deployed<\/dt><dd[^>]*>\$75\.00<\/dd>/);
    assert.match(markup, /<dt[^>]*>Reserved<\/dt><dd[^>]*>\$25\.00<\/dd>/);
    assert.match(markup, /<dt[^>]*>Available<\/dt><dd[^>]*>\$400\.00<\/dd>/);
    assert.match(markup, /<dt[^>]*>Utilisation<\/dt><dd[^>]*>20\.00%<\/dd>/);
    assert.match(markup, /dated-futures-carry/);
    assert.match(markup, /committed_carry/);
    assert.match(markup, /Reported allocation reasons/);
    assert.match(markup, /Remaining limits/);
    assert.match(markup, /Shared capital pool composition/);
    assert.doesNotMatch(markup, /do not reconcile/);

    const candidateMarkup = renderToStaticMarkup(
      panel(
        state({
          capitalAllocation: carryAllocation,
          availableNotional: 400,
          tab: 'candidates',
        }),
      ),
    );
    assert.match(candidateMarkup, /BTC-USD-20261225/);
    assert.match(
      candidateMarkup,
      /Expected edge, confidence and opportunity-specific proposal are unavailable/,
    );
    assert.doesNotMatch(
      markup,
      /strategy budget|horizon budget|edit allocation/i,
    );
  });

  it('keeps cold-start zero data separate from a missing v1 contract', () => {
    const coldStart = renderToStaticMarkup(
      panel(
        state({
          capitalAllocation: {
            version: 'v1',
            model: 'shared_pool',
            poolLimit: 500,
            used: 0,
            reserved: 0,
            available: 500,
            rows: [
              {
                strategy: 'trend-50-200',
                symbol: 'BTC-USDT',
                horizon: 'daily',
                effectiveAllocation: 0,
                reserved: 0,
                used: 0,
                utilization: 0,
                evidenceEligible: null,
                state: 'cold_start',
                reason: 'Evidence collection is pending',
              },
            ],
          },
          availableNotional: 500,
        }),
      ),
    );
    assert.match(coldStart, /\$0\.00/);
    assert.match(coldStart, /Unknown/);
    assert.match(coldStart, /cold_start/);

    const missing = renderToStaticMarkup(
      panel(state({ capitalAllocation: undefined, availableNotional: 500 })),
    );
    assert.match(
      missing,
      /Allocation details are unavailable from the current status contract/,
    );
    assert.match(missing, /does not indicate zero allocation/);
  });

  it('renders backend signed zero as zero without changing nonzero amounts', () => {
    const markup = renderToStaticMarkup(
      panel(
        state({
          capitalAllocation: {
            version: 'v1',
            model: 'shared_pool',
            poolLimit: 500,
            used: 0,
            reserved: -0,
            available: 500,
            rows: [
              {
                strategy: 'trend-50-200',
                symbol: 'BTC-USDT',
                horizon: 'daily',
                effectiveAllocation: -0,
                reserved: -0,
                used: 0,
                utilization: null,
                evidenceEligible: null,
                state: 'unavailable',
                reason: 'No fresh eligible directional candidate',
              },
            ],
          },
          availableNotional: 500,
        }),
      ),
    );
    assert.match(markup, /<dt[^>]*>Reserved<\/dt><dd[^>]*>\$0\.00<\/dd>/);
    assert.match(markup, /\$0\.00/);
    assert.doesNotMatch(markup, /-0\.00/);
  });

  it('flags pool composition discrepancies without rewriting sourced amounts', () => {
    const mismatch = {
      ...carryAllocation,
      available: 399,
    };
    const markup = renderToStaticMarkup(
      panel(state({ capitalAllocation: mismatch })),
    );
    assert.match(markup, /do not sum to the pool limit/);
    assert.match(markup, /\$399\.00/);
    assert.match(markup, /\$75\.00/);
    assert.match(markup, /\$25\.00/);
  });

  it('keeps unavailable history explicit rather than treating current rows as a historical series', () => {
    const markup = renderToStaticMarkup(
      panel(state({ capitalAllocation: undefined, tab: 'history' })),
    );
    assert.match(markup, /Allocation history is unavailable/);
    assert.match(markup, /current v1 status contract exposes a snapshot/);
  });
});
