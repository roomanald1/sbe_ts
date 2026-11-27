import { after, before, describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { AutoTraderModel } from '../models/AutoTraderModel';
import type { CapitalAllocationWidgetState } from '../models/CapitalAllocationWidgetModel';
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
  overrides: Partial<CapitalAllocationWidgetState> = {},
): CapitalAllocationWidgetState {
  const source = new AutoTraderModel().state$.value;
  return {
    active: true,
    capitalAllocation: source.capitalAllocation,
    availableNotional: source.availableNotional,
    ...overrides,
  };
}

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

  it('renders current shared-pool data, committed dated carry and controls guidance', () => {
    const markup = renderToStaticMarkup(
      createElement(CapitalAllocationPanel, {
        state: state({
          capitalAllocation: {
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
          },
          availableNotional: 400,
        }),
      }),
    );
    assert.match(
      markup,
      /Autonomous pool across eligible strategies, symbols and horizons/,
    );
    assert.match(markup, /<dt>Capital<\/dt><dd>USD 500\.00<\/dd>/);
    assert.match(markup, /<dt>Deployed<\/dt><dd>USD 75\.00<\/dd>/);
    assert.match(markup, /<dt>Reserved<\/dt><dd>USD 25\.00<\/dd>/);
    assert.match(markup, /<dt>Available<\/dt><dd>USD 400\.00<\/dd>/);
    assert.match(markup, /<dt>Utilisation<\/dt><dd>20\.0%<\/dd>/);
    assert.match(markup, /dated-futures-carry · BTC-USD-20261225 · carry/);
    assert.match(markup, /committed_carry/);
    assert.match(
      markup,
      /Global order, daily loss and position limits remain in Auto Trader Controls/,
    );
    assert.doesNotMatch(
      markup,
      /strategy budget|horizon budget|edit allocation/i,
    );
  });

  it('keeps cold-start zero data separate from a missing v1 contract', () => {
    const coldStart = renderToStaticMarkup(
      createElement(CapitalAllocationPanel, {
        state: state({
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
      }),
    );
    assert.match(coldStart, /Effective USD 0\.00/);
    assert.match(coldStart, /Evidence eligibility: unknown/);
    assert.match(coldStart, /State: cold_start/);

    const missing = renderToStaticMarkup(
      createElement(CapitalAllocationPanel, {
        state: state({ capitalAllocation: undefined, availableNotional: 500 }),
      }),
    );
    assert.match(
      missing,
      /details are unavailable from the current AutoTrader status contract/,
    );
    assert.match(missing, /does not indicate zero allocation/);
  });

  it('renders backend signed zero as zero without changing nonzero amounts', () => {
    const markup = renderToStaticMarkup(
      createElement(CapitalAllocationPanel, {
        state: state({
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
      }),
    );
    assert.match(markup, /<dt>Reserved<\/dt><dd>USD 0\.00<\/dd>/);
    assert.match(
      markup,
      /Effective USD 0\.00 · reserved USD 0\.00 · used USD 0\.00/,
    );
    assert.doesNotMatch(markup, /-0\.00/);
  });
});
