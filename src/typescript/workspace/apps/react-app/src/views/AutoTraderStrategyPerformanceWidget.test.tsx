import { after, before, describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer, type ViteDevServer } from 'vite';

let viteServer: ViteDevServer;
let StrategyPerformanceWidget: typeof import('./AutoTraderStrategyPerformanceWidget').AutoTraderStrategyPerformanceWidget;

before(async () => {
  viteServer = await createServer({
    configFile: './vite.config.ts',
    server: { middlewareMode: true },
    appType: 'custom',
  });
  const module = await viteServer.ssrLoadModule(
    '/src/views/AutoTraderStrategyPerformanceWidget.tsx',
  );
  StrategyPerformanceWidget =
    module.AutoTraderStrategyPerformanceWidget as typeof StrategyPerformanceWidget;
});

after(async () => {
  await viteServer?.close();
});

describe('Strategy Performance dashboard title', () => {
  it('keeps the source status while leaving the dashboard frame as the title', () => {
    const html = renderToStaticMarkup(
      createElement(StrategyPerformanceWidget, {
        widgetId: 'strategy-performance-title-fixture',
      }),
    );

    assert.match(html, /aria-label="Strategy performance"/);
    assert.match(html, /Refreshing outcomes|closed audit rows|Scout outcomes/);
    assert.doesNotMatch(html, /<strong>Strategy Performance<\/strong>/);
  });
});
