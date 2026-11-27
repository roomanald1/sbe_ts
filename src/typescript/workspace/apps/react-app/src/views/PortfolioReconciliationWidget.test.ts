import { after, before, describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer, type ViteDevServer } from 'vite';
import type { PortfolioReconciliationState } from '../models/PortfolioReconciliationWidgetModel';

let viteServer: ViteDevServer;
let Panel: typeof import('./PortfolioReconciliationWidget').PortfolioReconciliationPanel;

before(async () => {
  viteServer = await createServer({
    configFile: './vite.config.ts',
    server: { middlewareMode: true },
    appType: 'custom',
  });
  const module = await viteServer.ssrLoadModule(
    '/src/views/PortfolioReconciliationWidget.tsx',
  );
  Panel = module.PortfolioReconciliationPanel as typeof Panel;
});

after(async () => {
  await viteServer?.close();
});

describe('PortfolioReconciliationPanel', () => {
  it('renders unavailable values, cash zero, and independent evidence limitations honestly', () => {
    const state: PortfolioReconciliationState = {
      loading: false,
      stale: false,
      dataStatus: 'available',
      tab: 'summary',
      intervalFilter: 'all',
      snapshot: {
        snapshot_id: 'snapshot-1',
        captured_at_ms: 1_800_000_000_000,
        account: {
          currency: 'USD',
          cash: 0,
          equity_available: false,
          fees_paid: 0,
        },
        ledger_coverage: {
          ledger_evidence_epoch: 'pm-ledger-evidence:v1:epoch-1',
          ledger_evidence_epoch_status: 'initialized_empty',
          independent_execution_ledger_reconciled: false,
        },
      },
      intervalCoverage: 'no_durable_checkpoint',
      intervals: [],
      truncated: false,
      allocation: null,
      refreshedAtMs: 1_800_000_000_000,
    };
    const html = renderToStaticMarkup(createElement(Panel, { state }));
    assert.match(html, /Current cash/);
    assert.match(html, /\$0\.00/);
    assert.match(html, /Current equity[\s\S]*?Unavailable/);
    assert.match(
      html,
      /Independent execution comparison[\s\S]*?Not reconciled/,
    );
    assert.match(html, /Match probes are not settlements or fills/);
    assert.match(html, /Fees paid are already included/);
    assert.match(html, /Intervals \(0\)/);
  });

  it('shows complete returns only when the PM interval declares return completeness', () => {
    const state: PortfolioReconciliationState = {
      loading: false,
      stale: false,
      dataStatus: 'available',
      tab: 'intervals',
      intervalFilter: 'all',
      snapshot: null,
      intervalCoverage: 'pm_internal_complete',
      intervals: [1, 2].map((index) => ({
        raw: { evidence_id: `complete-${index}` },
        evidenceId: `complete-${index}`,
        endMs: 1_800_000_000_000 + index,
        completeness: 'complete' as const,
        independentStatus: 'not_evaluated' as const,
        returnPercent: 0.25,
        netPnlUsd: 2.5,
        reasons: [],
      })),
      truncated: false,
      allocation: null,
      refreshedAtMs: 1_800_000_000_000,
    };
    const html = renderToStaticMarkup(createElement(Panel, { state }));
    assert.match(html, /complete-1/);
    assert.match(html, /Authoritative complete interval returns/);
    assert.match(html, /0\.2500%/);
    assert.match(html, /Independent evidence requires authoritative execution/);
  });
});
