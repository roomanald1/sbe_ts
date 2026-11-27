import { after, before, describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer, type ViteDevServer } from 'vite';
import type { FunnelCycle } from '../models/AutoTraderDiagnosticsModel';

let viteServer: ViteDevServer;
let DecisionFunnelPanel: typeof import('./AutoTraderDiagnostics').DecisionFunnelPanel;
let BtcRegimeSourceLabel: typeof import('./AutoTraderDiagnostics').BtcRegimeSourceLabel;

before(async () => {
  viteServer = await createServer({
    configFile: './vite.config.ts',
    server: { middlewareMode: true },
    appType: 'custom',
  });
  const module = await viteServer.ssrLoadModule(
    '/src/views/AutoTraderDiagnostics.tsx',
  );
  DecisionFunnelPanel =
    module.DecisionFunnelPanel as typeof DecisionFunnelPanel;
  BtcRegimeSourceLabel =
    module.BtcRegimeSourceLabel as typeof BtcRegimeSourceLabel;
});

after(async () => {
  await viteServer?.close();
});

function cycle(candidateDetails: FunnelCycle['candidateDetails']): FunnelCycle {
  return {
    cycleId: 'cycle-42',
    timestampMs: 1_800_000_000_000,
    outcome: 'candidates_rejected',
    rawSignalCount: 2,
    allowedSignalCount: 2,
    strategyCandidateCount: 2,
    eligibleCandidateCount: 2,
    rejectedCandidateCount: 1,
    selectedCandidateCount: 1,
    orderAttemptCount: 1,
    submittedOrderCount: 1,
    fillConfirmed: false,
    primaryBlocker: 'capital unavailable',
    detail: '',
    candidateCoverage: {
      state: 'partial',
      retained: 2,
      observed: 3,
      truncated: true,
    },
    candidateDetails,
  };
}

describe('DecisionFunnelPanel', () => {
  it('renders explicit classification, measured gate values and stable identities', () => {
    const markup = renderToStaticMarkup(
      createElement(DecisionFunnelPanel, {
        nowMs: 1_800_000_000_000,
        cycles: [
          cycle([
            {
              decisionId: 'decision-7',
              workerEventId: 'event-9',
              profileId: 'profile-v2',
              symbol: 'BTC-USDT',
              strategy: 'trend-50-200',
              stage: 'selected',
              observed: { edge_bps: 3 },
              required: { edge_bps: 5 },
              checks: { edge: false },
              evidenceContract: null,
              measurements: [
                {
                  gateId: 'net_edge',
                  observed: 3,
                  comparator: '>=',
                  required: 5,
                  unit: 'bps',
                  passed: false,
                },
              ],
              reasons: ['net edge below required threshold'],
              selectionMode: 'exploratory',
              strictEligibility: false,
              exploratoryEligibility: true,
            },
            {
              decisionId: 'decision-8',
              symbol: 'ETH-USDT',
              strategy: 'range-breakout',
              stage: 'rejected',
              observed: {},
              required: {},
              checks: {},
              evidenceContract: null,
              reasons: ['market data stale', 'spread unavailable'],
              selectionMode: null,
              strictEligibility: null,
              exploratoryEligibility: null,
            },
          ]),
        ],
      }),
    );
    assert.match(markup, /Decision funnel diagnostics/);
    assert.match(
      markup,
      /Showing 1 of 1 cycle reports returned by API \(view cap 35\); server history completeness is unknown/,
    );
    assert.match(markup, /candidate detail records: 2/);
    assert.match(markup, /partial; 2 retained of 3 observed; truncated/);
    assert.match(markup, /BTC-USDT \/ trend-50-200/);
    assert.match(markup, /exploratory: 1/);
    assert.match(markup, /Strict eligibility/);
    assert.match(markup, /Fresh directional before final gates/);
    assert.match(markup, /decision decision-7 · cycle cycle-42/);
    assert.match(markup, /observed 3 bps &gt;= 5 bps · failed/);
    assert.match(markup, /Reported reasons \(unordered\)/);
    assert.match(markup, /Unknown/);
    assert.match(markup, /Filter by cycle/);
    assert.match(markup, /Group candidates by/);
    assert.doesNotMatch(markup, /strict eligibility: Eligible/);
  });

  it('distinguishes no reports from a report with zero candidate details', () => {
    const zeroRows = renderToStaticMarkup(
      createElement(DecisionFunnelPanel, {
        cycles: [cycle([])],
        nowMs: 1_800_000_000_000,
      }),
    );
    const unavailable = renderToStaticMarkup(
      createElement(DecisionFunnelPanel, { cycles: [] }),
    );
    assert.match(zeroRows, /No candidate detail records are available/);
    assert.match(unavailable, /no entry-cycle reports were received/);
    assert.match(
      zeroRows,
      /Showing 1 of 1 cycle reports returned by API \(view cap 35\); server history completeness is unknown/,
    );
    assert.match(
      unavailable,
      /Showing 0 of 0 cycle reports returned by API \(view cap 35\); server history completeness is unknown/,
    );
  });

  it('keeps repeated worker-event candidate rows renderable without duplicate React keys', () => {
    const duplicateCandidate: NonNullable<
      FunnelCycle['candidateDetails']
    >[number] = {
      decisionId:
        'strategy-worker-strategy-insight:v1:insights-event:v1:sha256:repeat',
      workerEventId: 'insights-event:v1:sha256:repeat',
      symbol: 'BTC-USDT',
      strategy: 'live-momentum',
      stage: 'rejected',
      observed: {},
      required: {},
      checks: {},
      evidenceContract: null,
      reasons: ['same worker event appears in overlapping retained reports'],
      selectionMode: null,
      strictEligibility: null,
      exploratoryEligibility: null,
    };
    const repeatedCycle = cycle([duplicateCandidate, duplicateCandidate]);
    const previousError = console.error;
    const errors: string[] = [];
    console.error = (...args: unknown[]) => errors.push(args.join(' '));
    try {
      renderToStaticMarkup(
        createElement(DecisionFunnelPanel, {
          nowMs: repeatedCycle.timestampMs,
          cycles: [repeatedCycle, repeatedCycle],
        }),
      );
    } finally {
      console.error = previousError;
    }
    assert.equal(
      errors.filter((error) => error.includes('same key')).length,
      0,
      errors.join('\n'),
    );
  });
});

describe('BtcRegimeSourceLabel', () => {
  it('renders the monthly rebalance anchor close timestamp', () => {
    const markup = renderToStaticMarkup(
      createElement(BtcRegimeSourceLabel, {
        diagnostic: {
          strategy: 'risk-adjusted-monthly-momentum',
          as_of_ms: 1_800_000_000_000,
          btc_close: 100,
          btc_sma: 90,
          btc_distance_percent: 11.1,
          btc_window_days: 200,
          btc_gate_required: true,
          btc_gate_passed: true,
          btc_source_close_time_ms: Date.UTC(2026, 11, 1),
          btc_source_timeframe: 'monthly_rebalance_anchor_daily',
          symbols: [],
        },
      }),
    );
    assert.match(
      markup,
      /prior completed monthly rebalance anchor \(daily close\)/,
    );
    assert.match(markup, /2026-12-01T00:00:00\.000Z/);
    assert.doesNotMatch(markup, /latest completed daily close/);
  });

  it('renders the latest completed daily close timestamp for ordinary gates', () => {
    const markup = renderToStaticMarkup(
      createElement(BtcRegimeSourceLabel, {
        diagnostic: {
          strategy: 'donchian-55-btc200',
          as_of_ms: 1_800_000_000_000,
          btc_close: 100,
          btc_sma: 90,
          btc_distance_percent: 11.1,
          btc_window_days: 200,
          btc_gate_required: true,
          btc_gate_passed: true,
          btc_source_close_time_ms: Date.UTC(2027, 0, 15, 8),
          btc_source_timeframe: 'daily',
          symbols: [],
        },
      }),
    );
    assert.match(markup, /latest completed daily close/);
    assert.match(markup, /2027-01-15T08:00:00\.000Z/);
  });
});
