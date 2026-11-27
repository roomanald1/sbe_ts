import { after, before, describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer, type ViteDevServer } from 'vite';
import type { AutoTraderOutcome } from '../models/AutoTraderOutcomesModel';
import type { OrdersPositionsState } from '../models/OrdersPositionsWidgetModel';

let viteServer: ViteDevServer;
let Panel: typeof import('./OrdersPositionsWidget').OrdersPositionsPanel;

before(async () => {
  viteServer = await createServer({
    configFile: './vite.config.ts',
    server: { middlewareMode: true },
    appType: 'custom',
  });
  const module = await viteServer.ssrLoadModule(
    '/src/views/OrdersPositionsWidget.tsx',
  );
  Panel = module.OrdersPositionsPanel as typeof Panel;
});
after(async () => {
  await viteServer?.close();
});

const emptyState: OrdersPositionsState = {
  loading: false,
  pmAvailable: false,
  outcomesLoading: false,
  operationalSummary: null,
  positions: [],
  closedPositions: [],
  activityOrders: [],
  history: {
    orders: [],
    barrierOrders: [],
    activeOrders: [],
    activeBarrierOrders: [],
    terminalPaths: [],
    hasMoreOrders: null,
    hasMoreBarrierOrders: null,
    nextOrdersCursor: null,
    nextBarrierOrdersCursor: null,
    executionPathsTruncated: null,
  },
  fetchedAtMs: null,
};

function position(
  overrides: Partial<AutoTraderOutcome> = {},
): AutoTraderOutcome {
  return {
    rowId: 'open:BTC-USDT:trend',
    status: 'Open',
    decisionId: 'decision-1',
    symbol: 'BTC-USDT',
    side: 'buy',
    openedAt: 1_000,
    quantity: 1,
    signedQuantity: 1,
    entryPrice: 100,
    pnl: 0,
    entryFees: 0,
    estimatedExitFeeRate: 0,
    estimatedExitSlippageBps: 0,
    pnlQuoteCurrency: 'USDT',
    strategy: 'trend-50-200',
    direction: 'bullish',
    strength: null,
    scout: false,
    source: 'unknown',
    components: {},
    ...overrides,
  };
}

function render(
  state: OrdersPositionsState,
  initialTab?: 'positions' | 'active' | 'protection' | 'history' | 'closed',
) {
  return renderToStaticMarkup(
    createElement(Panel, { state, nowMs: 2_000, initialTab }),
  );
}

describe('OrdersPositionsPanel compact operator redesign', () => {
  it('shows the compact summary, open/closed navigation and explicit empty state', () => {
    const html = render({
      ...emptyState,
      pmAvailable: true,
      fetchedAtMs: 1_000,
    });
    assert.match(html, /Orders &amp; positions/);
    assert.doesNotMatch(html, /<h2[^>]*>Orders &amp; positions<\/h2>/);
    assert.match(html, /Paper ledger ·/);
    assert.match(html, /Orders and positions summary/);
    assert.match(html, /No open positions reported/);
    assert.doesNotMatch(html, /Gross \/ net unrealised P&amp;L/);
  });

  it('renders compact sortable open rows with unavailable numbers and expandable identifiers', () => {
    const html = render({
      ...emptyState,
      positions: [
        position({
          entryPolicy: 'exploratory',
          entryProfileId: 'btc-trend-v2',
          entryProfileVersion: 2,
        }),
      ],
    });
    assert.match(html, /aria-label="Open positions"/);
    assert.match(html, /Symbol/);
    assert.match(html, /Strategy/);
    assert.match(html, /aria-sort="ascending"/);
    assert.match(html, /BTC-USDT/);
    assert.match(html, /trend-50-200/);
    assert.match(html, /Entry value/);
    assert.match(html, /Mode/);
    assert.match(html, /Explore/);
    assert.doesNotMatch(html, /—\s*<small> entry<\/small>/);
    assert.match(html, /class="operator-net-pnl"/);
    assert.match(html, /net return unavailable/);
    assert.match(html, /Net breakeven unavailable/);
    assert.match(html, /Net unrealised/);
    assert.match(html, /Protection exceptions/);
    assert.match(html, /<summary>Details<\/summary>/);
    assert.match(html, /Entry notional/);
    assert.match(
      html,
      /No unique canonical trade ID match; symbol-only match withheld/,
    );
    assert.match(html, /Holding duration/);
    assert.match(html, /Symbol \/ strategy[\s\S]*?BTC-USDT \/ trend-50-200/);
    assert.match(html, /decision-1/);
    assert.match(html, /btc-trend-v2 \/ — \/ exploratory/);
    assert.doesNotMatch(html, /Position context for BTC-USDT/);
    assert.match(html, /Executable mark[\s\S]*?—/);
  });

  it('plots only selected protection and exact PM trigger while withholding a missing executable close mark', () => {
    const row = position({
      selectedStopLossPercent: 0.4,
      selectedTakeProfitPercent: 0.9,
      selectedStopLossSource: 'validated_exit_policy',
      selectedTakeProfitSource: 'validated_exit_policy',
      entryPrice: 100,
      entryProfileId: 'profile-a',
      entryProfileVersion: 4,
      currentPmBarrier: {
        id: 'barrier-a',
        status: 'armed',
        trigger_price: 99.58,
        profile_id: 'profile-a',
        profile_version: 4,
        trigger_adjustments: [
          {
            reason: 'trailing-stop',
            expected_trigger_price: 99.5,
            new_trigger_price: 99.58,
            adjusted_at_ms: 1_500,
          },
        ],
      },
      operationalPosition: {
        symbol: 'BTC-USDT',
        quantity: 0.35,
        average_entry_price: 100,
        realized_pnl: 0,
        entry_fees: 0.02,
        fees_paid: 0.02,
        estimated_exit_fee_rate: 0.001,
        last_trade_timestamp: 1_000,
        canonical_trade_ids: ['trade-btc'],
        quote_currency: 'USDT',
        entry_value_usd: 35,
        entry_value_quote: 35,
        executable_close_side: 'bid',
        executable_mark: 100.32,
        mark_age_ms: 250,
        mark_source_record_id: 'mark-123',
        gross_unrealized_quote: 0.112,
        net_unrealized_quote: 0.08,
        net_unrealized_usd: 0.08,
        gross_exposure_usd: 35.112,
        signed_exposure_usd: 35.112,
        net_edge_vs_breakeven_percent: 0.32,
        valuation_status: 'complete',
        protection_status: 'active',
        protection_exception: false,
        protection_markers: [
          {
            barrier_id: 'barrier-a',
            order_kind: 'trailing-stop',
            status: 'armed',
            trigger_price: 99.58,
            projected_net_return_percent: -0.42,
          },
        ],
      },
    });
    const html = render({ ...emptyState, positions: [row] });
    assert.match(html, /\$35\.00<small> entry<\/small>/);
    assert.match(html, /PM trailing-stop/);
    assert.match(html, /Close now \+0\.32% · \$0\.08/);
    assert.match(html, /Net breakeven 0\.000%/);
    assert.match(
      html,
      /Executable mark[\s\S]*?bid 100\.32 USDT · 250 ms old · mark-123/,
    );
    assert.match(
      html,
      /Gross \/ net unrealised[\s\S]*?0\.112 USDT \/ 0\.08 USDT \(\$0\.08\)/,
    );
    assert.match(html, /Protection exception[\s\S]*?active/);
    assert.match(html, /barrier-a · armed · trigger 99\.58/);
  });

  it('renders only backend operational aggregates in the summary strip', () => {
    const html = render({
      ...emptyState,
      operationalSummary: {
        capturedAtMs: 2_000,
        grossExposureUsd: 35.112,
        netUnrealizedUsd: 0.08,
        protectionExceptionCount: 1,
        valuationStatus: 'complete',
        valuationReasons: [],
      },
    });
    assert.match(html, /Exposure[\s\S]*?\$35\.11/);
    assert.match(html, /Net unrealised[\s\S]*?\$0\.08/);
    assert.match(html, /Protection exceptions[\s\S]*?>1<\/dd>/);
  });

  it('labels signal-managed exits without inventing a fixed protective threshold', () => {
    const html = render({
      ...emptyState,
      positions: [
        position({
          exitControlMode: 'signal_managed',
          stopLossActive: false,
          takeProfitActive: false,
          selectedStopLossPercent: 0.5,
          selectedTakeProfitPercent: 0.75,
          selectedStopLossSource: 'global_fallback',
          selectedTakeProfitSource: 'global_fallback',
          currentPmBarrier: {
            id: 'signal-managed-barrier',
            status: 'armed',
            trigger_price: 99.6,
          },
        }),
      ],
    });
    assert.match(
      html,
      /Signal-managed strategy exits; a fixed PM stop\/target is not required/,
    );
    assert.match(
      html,
      /Selected stop \/ target[\s\S]*?Inactive · signal-managed exit contract \/ Inactive · signal-managed exit contract/,
    );
    assert.doesNotMatch(html, /SL -0\.50%|TP \+0\.75%/);
    assert.doesNotMatch(html, /PM trigger entry-price move/);
    assert.match(html, /signal-managed-barrier/);
  });

  it('renders a sortable closed-position ledger table with fees, net P&L and detailed attribution', () => {
    const closed = {
      ...position({
        rowId: 'ledger-summary:trade-1',
        status: 'Closed',
        closedAt: 1_500,
        decisionId: 'decision-closed',
        tradeId: 'trade-1',
        opportunityId: 'opp-1',
        episodeId: 'episode-1',
        netPnlUsd: 1.25,
        totalFees: 0.03,
        entryNotionalUsd: 100,
        executablePricePathEvidence: {
          complete: false,
          retainedSampleCount: 2,
          totalSampleCount: 4,
        },
      }),
      fillDetails: [],
    };
    const html = render({ ...emptyState, closedPositions: [closed] }, 'closed');
    assert.doesNotMatch(html, /<h2[^>]*>Orders &amp; positions · closed<\/h2>/);
    assert.match(html, /Closed positions/);
    assert.match(html, /Net P&amp;L/);
    assert.match(html, /\$1\.25/);
    assert.match(html, /0\.03 USDT/);
    assert.match(html, /Net %/);
    assert.match(html, /R/);
    assert.match(html, /MFE/);
    assert.match(html, /MAE/);
    assert.match(html, /Exit/);
    assert.match(html, /Evidence/);
    assert.match(html, /Net realised/);
    assert.match(html, /aria-sort="descending"/);
    assert.match(html, /<summary>Details<\/summary>/);
    assert.match(
      html,
      /Net return[\s\S]*?Unavailable in this position contract/,
    );
    assert.match(html, /Realised R multiple/);
    assert.match(html, /Executable path evidence/);
    assert.match(html, /Symbol \/ strategy[\s\S]*?BTC-USDT \/ trend-50-200/);
    assert.match(html, /trade-1/);
    assert.match(html, /Incomplete executable path/);
  });

  it('shows complete gross executable excursion and risk eligibility only from complete sourced evidence', () => {
    const closed = {
      ...position({
        rowId: 'ledger-summary:trade-complete',
        status: 'Closed',
        closedAt: 1_500,
        netPnlUsd: -0.09,
        initialRiskEvidence: {
          eligibilityStatus: 'eligible_for_r_multiple',
          eligibleForRMultiple: true,
          quoteCurrency: 'USDT',
          quoteAmount: 0.1,
          fillIdCount: 2,
        },
        executablePricePathEvidence: {
          complete: true,
          grossExecutableMfePercent: 0.11,
          grossExecutableMaePercent: 0.55,
          retainedSampleCount: 12,
          totalSampleCount: 12,
          freshSampleCount: 12,
        },
      }),
      fillDetails: [],
    };
    const html = render({ ...emptyState, closedPositions: [closed] }, 'closed');
    assert.match(html, /0\.11%/);
    assert.match(html, /0\.55%/);
    assert.match(html, /Path complete · R eligible/);
    assert.doesNotMatch(html, /Accepted/);
  });

  it('keeps absent closed financial values unavailable instead of projecting zero', () => {
    const closed = {
      ...position({
        status: 'Closed',
        rowId: 'closed-no-net',
        closedAt: 1_500,
      }),
      fillDetails: [],
    };
    const html = render({ ...emptyState, closedPositions: [closed] }, 'closed');
    assert.match(html, /Net P&amp;L[\s\S]*?—/);
    assert.match(html, /Fees[\s\S]*?—/);
  });

  it('keeps terminal and stable order identifiers expandable in the compact order table', () => {
    const order = {
      id: 'pm-order',
      symbol: 'ETH-USDT',
      side: 'buy',
      quantity: 2,
      timestamp: 1_000,
      lifecycle: { profile_id: 'profile-eth' },
    };
    const html = render(
      {
        ...emptyState,
        pmAvailable: true,
        history: {
          ...emptyState.history,
          orders: [order],
          activeOrders: [order],
          terminalPaths: [
            { order, terminal_reason: 'cancelled', terminal_at_ms: 1_500 },
          ],
        },
      },
      'history',
    );
    assert.match(html, /Order history/);
    assert.match(html, /Order details/);
    assert.match(html, /pm-order \/ — \/ —/);
    assert.match(html, /cancelled/);
  });

  it('shows exact protective barrier rows with recovery detail instead of joining by symbol', () => {
    const barrier = {
      id: 'barrier-1',
      symbol: 'ETH-USDT',
      side: 'sell',
      order_kind: 'stop_market',
      trigger_price: 99.5,
      quantity: 2,
      status: 'armed',
      error: 'none',
    };
    const html = render(
      {
        ...emptyState,
        history: { ...emptyState.history, activeBarrierOrders: [barrier] },
      },
      'protection',
    );
    assert.match(html, /Protective barriers/);
    assert.match(html, /99\.5/);
    assert.match(html, /Barrier details/);
    assert.match(html, /barrier-1 \/ — \/ —/);
  });

  it('keeps AutoTrader-only terminal submissions separate from the PM order ledger', () => {
    const html = render(
      {
        ...emptyState,
        activityOrders: [
          {
            timestamp: 1_000,
            symbol: 'BTC-USDT',
            side: 'buy',
            execution_order_id: 'activity-1',
            decision_id: 'decision-1',
            strategy: 'trend-50-200',
            status: 'expired',
            failure_reason: 'expired_without_full_fill',
          },
        ],
      },
      'history',
    );
    assert.match(html, /AutoTrader-only terminal submissions · 1/);
    assert.match(html, /execution activity-1/);
    assert.match(html, /expired_without_full_fill/);
  });

  it('does not duplicate activity-only submissions when a PM order has the exact execution ID', () => {
    const state: OrdersPositionsState = {
      ...emptyState,
      pmAvailable: true,
      activityOrders: [
        {
          timestamp: 1_000,
          symbol: 'ETH-USDT',
          side: 'buy',
          execution_order_id: 'pm-order',
          decision_id: 'decision-1',
          status: 'cancelled',
        },
      ],
      history: {
        ...emptyState.history,
        orders: [{ id: 'pm-order', symbol: 'ETH-USDT', timestamp: 1_000 }],
        activeOrders: [],
      },
    };
    const html = render(state, 'history');
    assert.equal((html.match(/pm-order/g) ?? []).length, 1);
    assert.doesNotMatch(html, /AutoTrader-only terminal submissions/);
  });
});
