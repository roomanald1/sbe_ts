import { after, before, describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { AutoTraderModel } from '../models/AutoTraderModel';
import type { CapitalAllocation } from '../models/AutoTraderModel';
import { CapitalAllocationWidgetModel } from '../models/CapitalAllocationWidgetModel';
import { createServer, type ViteDevServer } from 'vite';

let viteServer: ViteDevServer;
let CapitalAllocationView: typeof import('./AutoTraderAllocationView').CapitalAllocationView;

before(async () => {
  viteServer = await createServer({
    configFile: './vite.config.ts',
    server: { middlewareMode: true },
    appType: 'custom',
  });
  const module = await viteServer.ssrLoadModule(
    '/src/views/AutoTraderAllocationView.tsx',
  );
  CapitalAllocationView =
    module.CapitalAllocationView as typeof CapitalAllocationView;
});

after(async () => {
  await viteServer?.close();
});

function viewState(allocation?: CapitalAllocation, availableNotional = 500) {
  const upstream = new AutoTraderModel();
  upstream.state$.next({
    ...upstream.state$.value,
    loading: false,
    capitalAllocation: allocation,
    availableNotional,
  });
  return new CapitalAllocationWidgetModel('allocation-view-test', {
    state$: upstream.state$,
    start: () => undefined,
    stop: () => undefined,
  }).state$.value;
}

describe('CapitalAllocationView', () => {
  it('renders a zero cold-start allocation and unknown evidence distinctly', () => {
    const allocation: CapitalAllocation = {
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
    };
    const state = viewState(allocation);
    const markup = renderToStaticMarkup(
      createElement(CapitalAllocationView, {
        state: { ...state, tab: 'candidates' },
        onSetTab: () => undefined,
        onSetGroupBy: () => undefined,
      }),
    );
    assert.match(markup, /trend-50-200/);
    assert.match(markup, /BTC-USDT/);
    assert.match(markup, /Effective/);
    assert.match(markup, /Unknown/);
    assert.match(markup, /cold_start/);
    assert.match(markup, /Evidence collection is pending/);
    assert.doesNotMatch(markup, /profit|return|win rate/i);
  });

  it('shows missing contract as unavailable rather than zero allocation', () => {
    const markup = renderToStaticMarkup(
      createElement(CapitalAllocationView, {
        state: viewState(undefined, 500),
        onSetTab: () => undefined,
        onSetGroupBy: () => undefined,
      }),
    );
    assert.match(markup, /Allocation details are unavailable/);
    assert.match(markup, /\$500\.00/);
    assert.match(markup, /does not indicate zero allocation/);
  });
});
