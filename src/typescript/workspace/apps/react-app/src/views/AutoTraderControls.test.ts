import { after, before, describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer, type ViteDevServer } from 'vite';
import { AutoTraderModel } from '../models/AutoTraderModel';

let viteServer: ViteDevServer;
let ControlsSummary: typeof import('./AutoTraderGlobalControlsSummary').AutoTraderGlobalControlsSummary;
let AutoTraderView: typeof import('./AutoTrader').AutoTrader;

before(async () => {
  viteServer = await createServer({
    configFile: './vite.config.ts',
    server: { middlewareMode: true },
    appType: 'custom',
  });
  const summary = await viteServer.ssrLoadModule(
    '/src/views/AutoTraderGlobalControlsSummary.tsx',
  );
  const view = await viteServer.ssrLoadModule('/src/views/AutoTrader.tsx');
  ControlsSummary =
    summary.AutoTraderGlobalControlsSummary as typeof ControlsSummary;
  AutoTraderView = view.AutoTrader as typeof AutoTraderView;
});

after(async () => {
  await viteServer?.close();
});

describe('AutoTrader Controls & Safety', () => {
  it('renders source-backed global constraints and profile-owned safety without universal exit defaults', () => {
    const base = new AutoTraderModel().state$.value;
    const state = {
      ...base,
      loading: false,
      availableNotional: 996.5,
      dailyNotionalLimit: '1250',
      serverDailyNotionalLimit: 1000,
      maxOrderNotional: '100',
      dailyRealizedLossLimit: '10',
      maxOpenPositions: '20',
      serverExplorationPercent: 2,
      auditHealthy: true,
      auditFailureLatched: false,
    };
    const before = { ...state };
    const html = renderToStaticMarkup(
      createElement(ControlsSummary, {
        state,
        learnedPoliciesOpen: false,
        onViewLearnedPolicies: () => undefined,
      }),
    );
    assert.match(html, /Global risk limits/);
    assert.match(html, /Available capital[\s\S]*?\$996\.50/);
    assert.match(html, /Daily notional limit[\s\S]*?\$1,000(?!\.\d)/);
    assert.match(html, /Max order[\s\S]*?\$100(?!\.\d)/);
    assert.match(html, /Daily loss limit[\s\S]*?\$10(?!\.\d)/);
    assert.match(html, /Max positions[\s\S]*?20/);
    assert.match(html, /Exploration[\s\S]*?2%/);
    assert.match(html, /Exit policy[\s\S]*?Profile-owned/);
    assert.match(html, /View learned policies/);
    assert.match(html, /Audit[\s\S]*?OK/);
    assert.match(html, /Safety latch[\s\S]*?Clear/);
    assert.doesNotMatch(html, /stop loss|take profit/i);
    assert.doesNotMatch(html, /per.strategy budget/i);
    assert.deepEqual(
      state,
      before,
      'rendering controls must not mutate settings',
    );
  });

  it('uses warning states when audit persistence is degraded or the safety latch is set', () => {
    const state = {
      ...new AutoTraderModel().state$.value,
      auditHealthy: false,
      auditFailureLatched: true,
    };
    const html = renderToStaticMarkup(
      createElement(ControlsSummary, {
        state,
        learnedPoliciesOpen: false,
        onViewLearnedPolicies: () => undefined,
      }),
    );
    assert.match(html, /Audit[\s\S]*?Degraded/);
    assert.match(html, /Safety latch[\s\S]*?Set/);
  });

  it('does not present bootstrap state as effective runtime limits while loading', () => {
    const html = renderToStaticMarkup(
      createElement(ControlsSummary, {
        state: new AutoTraderModel().state$.value,
        onViewLearnedPolicies: () => undefined,
      }),
    );
    assert.match(html, /Available capital[\s\S]*?Unavailable/);
    assert.match(html, /Daily notional limit[\s\S]*?Unavailable/);
    assert.doesNotMatch(html, /\$1,000/);
  });

  it('labels the active mode Paper and never presents a Go live control', () => {
    const html = renderToStaticMarkup(createElement(AutoTraderView));
    assert.match(html, /Paper execution/);
    assert.match(html, /Connecting/);
    assert.match(html, /Resume/);
    assert.doesNotMatch(html, />LIVE</);
    assert.doesNotMatch(html, /Go live/);
  });
});
