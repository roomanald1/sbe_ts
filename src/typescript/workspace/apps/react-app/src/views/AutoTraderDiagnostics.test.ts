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
  it('renders a compact per-cycle funnel with explicit denominator and attribution limits', () => {
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
    assert.match(markup, /Decision funnel/);
    assert.match(
      markup,
      /1 retained cycle reports; server history completeness is unknown/,
    );
    assert.match(markup, /partial; 2 retained of 3 observed; truncated/);
    assert.match(markup, /Raw signals[\s\S]*?2/);
    assert.match(markup, /Eligible[\s\S]*?2/);
    assert.match(markup, /Conversion[\s\S]*?denominator not defined/);
    assert.match(markup, /Cycle-level blocker[\s\S]*?capital unavailable/);
    assert.doesNotMatch(markup, /<th[^>]*>cycle-42<\/th>/);
    assert.match(markup, /Cycle identity<\/dt><dd>cycle-42/);
    assert.match(markup, /BTC-USDT · trend-50-200/);
    assert.match(markup, /observed 3 bps &gt;= 5 bps · failed/);
    assert.match(markup, /Reported reasons \(unordered\)/);
    assert.match(markup, /Filter by cycle/);
    assert.match(markup, /Group candidates by/);
    assert.match(markup, /Failed measurement · net_edge/);
    assert.match(markup, /One candidate may contribute multiple records/);
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
    assert.match(zeroRows, /No candidate detail rows for these filters/);
    assert.match(zeroRows, /Raw signals[\s\S]*?2/);
    assert.match(unavailable, /No entry-cycle report is available/);
    assert.match(unavailable, /Directional[\s\S]*?—/);
    assert.match(unavailable, /server history completeness is unknown/);
  });

  it('shows strict or exploratory classification coverage without treating unknowns as failures', () => {
    const markup = renderToStaticMarkup(
      createElement(DecisionFunnelPanel, {
        initialTab: 'heatmap',
        nowMs: 1_800_000_000_000,
        cycles: [
          cycle([
            {
              decisionId: 'decision-a',
              workerEventId: 'event-a',
              symbol: 'BTC-USDT',
              strategy: 'momentum',
              stage: 'selected',
              observed: {},
              required: {},
              checks: {},
              evidenceContract: null,
              reasons: [],
              selectionMode: 'strict',
              strictEligibility: true,
              exploratoryEligibility: false,
            },
            {
              decisionId: 'decision-b',
              symbol: 'BTC-USDT',
              strategy: 'momentum',
              stage: 'rejected',
              observed: {},
              required: {},
              checks: {},
              evidenceContract: null,
              reasons: [],
              selectionMode: null,
              strictEligibility: false,
              exploratoryEligibility: null,
            },
            {
              decisionId: 'decision-c',
              symbol: 'ETH-USDT',
              strategy: 'momentum',
              stage: 'rejected',
              observed: {},
              required: {},
              checks: {},
              evidenceContract: null,
              reasons: [],
              selectionMode: null,
              strictEligibility: null,
              exploratoryEligibility: null,
            },
          ]),
        ],
      }),
    );
    assert.match(
      markup,
      /aria-label="Symbol and strategy eligibility heatmap"/,
    );
    assert.match(markup, /50% · 1\/2/);
    assert.match(markup, /data-eligibility="unknown"/);
    assert.match(markup, /Unknown classifications are excluded/);
    assert.match(markup, /Exploratory/);
    assert.doesNotMatch(markup, /100% · 1\/1/);
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
