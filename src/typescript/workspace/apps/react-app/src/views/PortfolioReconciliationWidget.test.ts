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
    assert.match(html, /Cash/);
    assert.match(html, /\$0\.00/);
    assert.match(html, /Equity[\s\S]*?Unavailable/);
    assert.match(html, /Independent reconciliation[\s\S]*?Not reconciled/);
    assert.match(html, /No interval evidence/);
    assert.match(html, /Fees paid are already reflected/);
    assert.match(html, /Reconciliation/);
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
    assert.match(
      html,
      /Independent comparison requires authoritative execution/,
    );
  });

  it('renders per-currency opening cash, separate contributions, and sourced opening/closing FX', () => {
    const state: PortfolioReconciliationState = {
      loading: false,
      stale: false,
      dataStatus: 'available',
      tab: 'balances',
      intervalFilter: 'all',
      snapshot: null,
      intervalCoverage: 'complete',
      intervals: [
        {
          raw: {
            opening_cash_balances_by_currency: { EUR: 10 },
            external_contributions_by_currency: { EUR: 2 },
            cash_balances_by_currency: { EUR: 12 },
            opening_cash_value_usd_by_currency: { EUR: 11 },
            closing_cash_value_usd_by_currency: { EUR: 13.2 },
            opening_fx_rates_usd_by_currency: { EUR: 1.1 },
            opening_fx_rates_updated_at_by_currency: {
              EUR: 1_800_000_000_000,
            },
            closing_fx_rates_usd_by_currency: { EUR: 1.1 },
            closing_fx_rates_updated_at_by_currency: {
              EUR: 1_800_000_000_000,
            },
            external_contribution_events: [{ contribution_id: 'topup-1' }],
            cash_movements_by_currency: { EUR: 2 },
          },
          evidenceId: 'interval-balance-1',
          endMs: 1_800_000_001_000,
          completeness: 'complete',
          independentStatus: 'not_evaluated',
          returnPercent: 0.1,
          netPnlUsd: 1,
          reasons: [],
        },
      ],
      truncated: false,
      allocation: null,
      refreshedAtMs: 1_800_000_001_000,
    };
    const html = renderToStaticMarkup(createElement(Panel, { state }));
    assert.match(html, /Opening cash/);
    assert.match(html, /External contributions/);
    assert.match(html, /Closing cash/);
    assert.match(html, /Opening USD value/);
    assert.match(html, /Closing USD value/);
    assert.match(html, /Opening FX rate/);
    assert.match(html, /Opening FX as of/);
    assert.match(html, /Closing FX rate/);
    assert.match(html, /FX age at interval end/);
    assert.match(html, /Contribution events[\s\S]*?1/);
  });
});
