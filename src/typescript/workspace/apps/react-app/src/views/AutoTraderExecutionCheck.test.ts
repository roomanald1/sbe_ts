import { after, before, describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer, type ViteDevServer } from 'vite';
import type { AutoTraderExecutionQuality } from '../models/AutoTraderModel';
import {
  buildExecutionQualitySnapshot,
  parseExecutionPathData,
  type ExecutionOrderEvidence,
} from '../models/AutoTraderExecutionQualityModel';

let viteServer: ViteDevServer;
let ExecutionQualityPanel: typeof import('./AutoTraderExecutionCheck').ExecutionQualityPanel;

before(async () => {
  viteServer = await createServer({
    configFile: './vite.config.ts',
    server: { middlewareMode: true },
    appType: 'custom',
  });
  const module = await viteServer.ssrLoadModule(
    '/src/views/AutoTraderExecutionCheck.tsx',
  );
  ExecutionQualityPanel =
    module.ExecutionQualityPanel as typeof ExecutionQualityPanel;
});

after(async () => {
  await viteServer?.close();
});

const quality: AutoTraderExecutionQuality = {
  order_samples: 2,
  filled_order_samples: 0,
  partial_fill_samples: 0,
  unfilled_order_samples: 2,
  measured_fill_samples: 0,
  response_latency_samples: 2,
  average_submit_response_ms: 4.5,
  history_truncated: false,
  last_updated_at: 1_800_000_000_000,
};

function order(
  overrides: Partial<ExecutionOrderEvidence> = {},
): ExecutionOrderEvidence {
  return {
    timestamp: 1_800_000_000_000,
    symbol: 'BTC-USDT',
    side: 'buy',
    order_type: 'PostOnlyLimit · P2',
    purpose: 'signal',
    strategy: 'trend-50-200',
    decision_id: 'decision-cross',
    execution_order_id: 'order-cross',
    quantity: 1,
    filled_quantity: 0,
    submitted_limit_price: 100.02,
    decision_bid: 100,
    decision_ask: 101,
    reserved_notional: 100.02,
    expires_at: 1_800_000_300_000,
    status: 'rejected',
    failure_reason: 'Post-only order would cross the opposite quote',
    realized_pnl: 0,
    fees_paid: 0,
    ...overrides,
  };
}

describe('ExecutionQualityPanel', () => {
  it('renders price evidence, explicit denominators and distinct terminal outcomes', () => {
    const rows = [
      order(),
      order({
        timestamp: 1_800_000_000_100,
        symbol: 'ETH-USDT',
        decision_id: 'decision-expiry',
        execution_order_id: 'order-expiry',
        status: 'expired',
        failure_reason: 'Passive order expired without a fill',
      }),
    ];
    const state = buildExecutionQualitySnapshot(rows, quality, false);
    const markup = renderToStaticMarkup(
      createElement(ExecutionQualityPanel, {
        state,
        nowMs: 1_800_000_000_200,
      }),
    );
    assert.match(markup, /Aggregate execution quality/);
    assert.match(markup, /Filled orders/);
    assert.match(markup, /0 \/ 2/);
    assert.match(markup, /Post-only crossing rejection/);
    assert.match(markup, /Passive order expired/);
    assert.match(markup, /Decision bid \/ ask/);
    assert.match(markup, /Submitted limit/);
    assert.match(markup, /Configured lifetime to expiry/);
    assert.match(markup, /reservation ID and timestamp unavailable/);
    assert.match(markup, /Post-submit executable market path, queue position/);
    assert.match(markup, /counterfactual fillability are unavailable/);
    assert.match(markup, /Filter execution orders by symbol/);
    assert.match(markup, /Filter execution orders by outcome/);
  });

  it('renders empty returned orders and zero summary fills without claiming complete history', () => {
    const state = buildExecutionQualitySnapshot([], quality, false);
    const markup = renderToStaticMarkup(
      createElement(ExecutionQualityPanel, {
        state,
        nowMs: 1_800_000_000_200,
      }),
    );
    assert.match(markup, /No AutoTrader order records were returned/);
    assert.match(markup, /0 fills from 2 orders/);
    assert.match(
      markup,
      /zero orders in this response do not establish complete history/,
    );
  });

  it('keeps missing legacy matcher-path fields unavailable instead of showing zero retained paths', () => {
    const legacyPaths = parseExecutionPathData({ orders: [] }, { orders: [] });
    const state = buildExecutionQualitySnapshot(
      [],
      quality,
      false,
      legacyPaths,
    );
    const markup = renderToStaticMarkup(
      createElement(ExecutionQualityPanel, {
        state,
        nowMs: 1_800_000_000_200,
      }),
    );
    assert.match(markup, /Terminal paths retained<\/dt><dd>Unavailable/);
    assert.match(markup, /Unlinked path records<\/dt><dd>Unavailable/);
    assert.match(
      markup,
      /history contract does not expose durable matcher-path records/,
    );
  });

  it('renders source path records, terminal retention, and observed-only limits without claiming fills', () => {
    const sourceOrder = order({
      execution_order_id: 'pm-expired-order',
      status: 'expired',
      filled_quantity: 0,
      failure_reason: 'Expired without full fill',
    });
    const pathEvidence = parseExecutionPathData(
      { orders: [] },
      {
        execution_paths: [
          {
            order: {
              id: 'pm-expired-order',
              remaining_quantity: 1,
              market_path_completeness: 'source_incomplete',
              market_path_dropped_samples: 3,
              market_path: [
                {
                  source_system: 'paper_exchange',
                  source_epoch: 'paper-epoch-4',
                  source_cursor: 45,
                  source_cursor_kind: 'event_sequence',
                  source_capture_complete: false,
                  request_fingerprint: 'sha256:sample',
                  attempt_order_id: 'attempt-9',
                  parent_order_id: 'pm-parent',
                  recorded_at_utc_ms: 1_800_000_000_001,
                  book_updated_at_ms: 1_800_000_000_000,
                  best_bid: 100,
                  best_ask: 101,
                  reference_price: 100.5,
                },
              ],
            },
            terminal_reason: 'expired_without_full_fill',
            terminal_at_ms: 1_800_000_000_100,
          },
        ],
        execution_paths_truncated: true,
        has_more_orders: false,
      },
    );
    const state = buildExecutionQualitySnapshot(
      [sourceOrder],
      quality,
      false,
      pathEvidence,
    );
    const markup = renderToStaticMarkup(
      createElement(ExecutionQualityPanel, {
        state,
        nowMs: 1_800_000_000_200,
      }),
    );
    assert.match(markup, /Terminal resting order/);
    assert.match(markup, /source_incomplete/);
    assert.match(markup, /Global terminal-path history truncated/);
    assert.match(markup, /45 · event_sequence/);
    assert.match(markup, /100 \/ 101/);
    assert.match(markup, /expired_without_full_fill/);
    assert.match(markup, /Dropped samples/);
    assert.match(
      markup,
      /do not establish continuous quote coverage, queue priority, or fillability/,
    );
    assert.match(markup, /Filled \/ requested quantity[\s\S]*?0 \/ 1/);
    assert.match(
      markup,
      /Matcher probes and resting-limit observations are not settled fills/,
    );
  });
});
