import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { BehaviorSubject } from 'rxjs';
import type {
  AutoTraderExecutionQuality,
  AutoTraderModel,
} from './AutoTraderModel';
import {
  AutoTraderExecutionQualityModel,
  buildExecutionOrderRows,
  buildExecutionQualitySnapshot,
  decisionPriceDistanceLabel,
  executionFreshnessLabel,
  joinMarketPathsToOrders,
  parseExecutionPathData,
  type ExecutionOrderEvidence,
} from './AutoTraderExecutionQualityModel';

function order(
  overrides: Partial<ExecutionOrderEvidence> = {},
): ExecutionOrderEvidence {
  return {
    timestamp: 1_000,
    symbol: 'BTC-USDT',
    side: 'buy',
    order_type: 'PostOnlyLimit · P2',
    purpose: 'signal',
    strategy: 'trend-50-200',
    decision_id: 'decision-1',
    execution_order_id: 'execution-1',
    quantity: 1,
    filled_quantity: 0,
    submitted_limit_price: 100.02,
    decision_bid: 100,
    decision_ask: 101,
    reserved_notional: 100.02,
    expires_at: 61_000,
    status: 'expired',
    failure_reason: 'passive order expired',
    realized_pnl: 0,
    fees_paid: 0,
    ...overrides,
  };
}

const emptyQuality: AutoTraderExecutionQuality = {
  order_samples: 0,
  filled_order_samples: 0,
  partial_fill_samples: 0,
  unfilled_order_samples: 0,
  measured_fill_samples: 0,
  response_latency_samples: 0,
  history_truncated: false,
};

function sourceFixture(orders: ExecutionOrderEvidence[] = []) {
  const state$ = new BehaviorSubject({
    orders,
    executionQuality: emptyQuality,
    loading: false,
  });
  let starts = 0;
  let stops = 0;
  return {
    source: {
      state$,
      start: () => {
        starts += 1;
      },
      stop: () => {
        stops += 1;
      },
    } as unknown as AutoTraderModel,
    get starts() {
      return starts;
    },
    get stops() {
      return stops;
    },
  };
}

function response(payload: unknown): Response {
  return { ok: true, status: 200, json: async () => payload } as Response;
}

const activePayload = { orders: [] };
const historyPayload = {
  execution_paths: [],
  execution_paths_truncated: false,
  has_more_orders: false,
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((accept) => {
    resolve = accept;
  });
  return { promise, resolve };
}

describe('AutoTraderExecutionQualityModel', () => {
  it('distinguishes post-only crossing rejection from passive expiry', () => {
    const [crossing, expiry] = buildExecutionOrderRows([
      order({
        status: 'rejected',
        failure_reason: 'Post-only order would cross the opposite quote',
      }),
      order({
        execution_order_id: 'execution-2',
        status: 'expired',
        failure_reason: 'Passive order expired without a fill',
      }),
    ]);
    assert.equal(crossing.outcome, 'post_only_cross_rejection');
    assert.equal(crossing.outcomeLabel, 'Post-only crossing rejection');
    assert.equal(expiry.outcome, 'passive_expiry');
    assert.equal(expiry.outcomeLabel, 'Passive order expired');
    assert.equal(expiry.order.filled_quantity, 0);
  });

  it('uses decision-time quotes only and preserves missing execution path as unavailable', () => {
    const [row] = buildExecutionOrderRows([order()]);
    assert.ok(Math.abs((row.decisionTouchDistanceBps ?? 0) - 2) < 1e-9);
    assert.equal(row.configuredLifetimeMs, 60_000);
    assert.equal(row.stableOrderId, 'execution-1');
    assert.equal(row.decisionId, 'decision-1');
    assert.equal(decisionPriceDistanceLabel(null), 'Unavailable');
  });

  it('caps rows, reports omitted records and preserves aggregate denominators', () => {
    const orders = Array.from({ length: 125 }, (_, index) =>
      order({ timestamp: index, execution_order_id: `execution-${index}` }),
    );
    const snapshot = buildExecutionQualitySnapshot(orders, emptyQuality, false);
    assert.equal(snapshot.availableOrderCount, 125);
    assert.equal(snapshot.displayedOrderCount, 100);
    assert.equal(snapshot.rows[0].order.timestamp, 124);
    assert.equal(snapshot.summary.order_samples, 0);
  });

  it('derives strategy/outcome summaries only from returned evidence records', () => {
    const orders = [
      order({ status: 'expired', strategy: 'trend-50-200', fees_paid: 0 }),
      order({
        status: 'filled',
        strategy: 'trend-50-200',
        execution_order_id: 'execution-filled',
        filled_quantity: 1,
        failure_reason: undefined,
        fees_paid: 0.02,
      }),
      order({
        status: 'cancelled',
        strategy: undefined,
        execution_order_id: 'execution-unattributed',
        failure_reason: undefined,
        fees_paid: undefined,
      }),
    ];
    const snapshot = buildExecutionQualitySnapshot(orders, emptyQuality, false);
    assert.equal(snapshot.displayedOrderCount, 3);
    assert.equal(snapshot.outcomeCounts.passive_expiry, 1);
    assert.equal(snapshot.outcomeCounts.filled, 1);
    assert.equal(snapshot.outcomeCounts.cancelled, 1);
    assert.deepEqual(snapshot.strategyRows, [
      {
        strategy: 'trend-50-200',
        orderCount: 2,
        filledCount: 1,
        feeEvidenceCount: 2,
        terminalAgeSamples: 0,
        averageTerminalLifetimeMs: null,
        fillRatePercent: 50,
      },
      {
        strategy: 'Attribution unavailable',
        orderCount: 1,
        filledCount: 0,
        feeEvidenceCount: 0,
        terminalAgeSamples: 0,
        averageTerminalLifetimeMs: null,
        fillRatePercent: 0,
      },
    ]);
  });

  it('computes average strategy age only from exact joined terminal timestamps', () => {
    const orders = [
      order({ execution_order_id: 'matched', timestamp: 1_000 }),
      order({ execution_order_id: 'unmatched', timestamp: 1_500 }),
    ];
    const evidence = parseExecutionPathData(
      { orders: [] },
      {
        execution_paths: [
          {
            order: { id: 'matched', remaining_quantity: 0 },
            terminal_reason: 'expired',
            terminal_at_ms: 3_000,
          },
        ],
        execution_paths_truncated: false,
        has_more_orders: false,
      },
    );
    const snapshot = buildExecutionQualitySnapshot(
      orders,
      emptyQuality,
      false,
      evidence,
    );
    assert.equal(snapshot.strategyRows[0].orderCount, 2);
    assert.equal(snapshot.strategyRows[0].terminalAgeSamples, 1);
    assert.equal(snapshot.strategyRows[0].averageTerminalLifetimeMs, 2_000);
  });

  it('marks missing and stale aggregate timestamps explicitly', () => {
    assert.deepEqual(executionFreshnessLabel(undefined, 10_000), {
      label: 'Update timestamp unavailable',
      stale: true,
    });
    assert.equal(executionFreshnessLabel(1, 70_002).stale, true);
    assert.equal(executionFreshnessLabel(69_000, 70_000).stale, false);
  });

  it('joins active and restart-retained terminal paths only by exact stable identifiers', () => {
    const orders = [
      order({ execution_order_id: 'pm-order-active', symbol: 'BTC-USDT' }),
      order({
        execution_order_id: 'pm-order-expired',
        symbol: 'ETH-USDT',
        timestamp: 500,
      }),
    ];
    const data = parseExecutionPathData(
      {
        orders: [
          {
            id: 'pm-order-active',
            remaining_quantity: 1,
            market_path_completeness: 'observed_only',
            market_path: [
              {
                source_system: 'paper_exchange',
                source_epoch: 'pe-1',
                source_cursor: 7,
                book_updated_at_ms: 900,
                best_bid: 99,
                best_ask: 101,
                reference_price: 100,
              },
            ],
          },
        ],
      },
      {
        execution_paths: [
          {
            order: {
              id: 'pm-order-expired',
              remaining_quantity: 0,
              market_path_completeness: 'truncated',
              market_path_dropped_samples: 2,
              market_path: [
                {
                  source_system: 'paper_exchange',
                  source_epoch: 'pe-1',
                  source_cursor: 8,
                  book_updated_at_ms: 800,
                  source_capture_complete: false,
                },
              ],
            },
            terminal_reason: 'expired_without_full_fill',
            terminal_at_ms: 1_100,
          },
        ],
        execution_paths_truncated: false,
        has_more_orders: false,
      },
    );
    const joined = joinMarketPathsToOrders(orders, data);
    assert.equal(joined.rows[0].marketPaths[0].source, 'active_order');
    assert.equal(joined.rows[1].marketPaths[0].source, 'terminal_history');
    assert.equal(
      joined.rows[1].marketPaths[0].terminalReason,
      'expired_without_full_fill',
    );
    assert.equal(
      joined.rows[1].marketPaths[0].order.market_path_dropped_samples,
      2,
    );
    assert.equal(joined.rows[1].order.filled_quantity, 0);
    assert.equal(joined.unlinked.length, 0);
  });

  it('does not attribute paths by symbol or timestamps and retains unmatched paths visibly', () => {
    const data = parseExecutionPathData(
      { orders: [] },
      {
        execution_paths: [
          {
            order: {
              id: 'pm-order-other',
              symbol: 'BTC-USDT',
              timestamp: 1_000,
              market_path: [{ book_updated_at_ms: 1_001 }],
            },
            terminal_reason: 'cancelled',
            terminal_at_ms: 2_000,
          },
        ],
        execution_paths_truncated: false,
        has_more_orders: false,
      },
    );
    const joined = joinMarketPathsToOrders(
      [order({ symbol: 'BTC-USDT', timestamp: 1_000 })],
      data,
    );
    assert.equal(joined.rows[0].marketPaths.length, 0);
    assert.equal(joined.unlinked[0].reason, 'no_activity_order_match');
  });

  it('supports stable client IDs and leaves non-unique parent joins unattributed', () => {
    const data = parseExecutionPathData(
      {
        orders: [
          {
            id: 'different-pm-id',
            client_order_id: 'client-a',
            remaining_quantity: 0.5,
            market_path: [{ book_updated_at_ms: 10 }],
            market_path_completeness: 'observed_only',
          },
        ],
      },
      {
        execution_paths: [
          {
            order: {
              id: 'terminal-x',
              parent_order_id: 'shared-parent',
              market_path: [{ book_updated_at_ms: 11 }],
            },
            terminal_reason: 'cancelled',
            terminal_at_ms: 20,
          },
        ],
        execution_paths_truncated: false,
        has_more_orders: false,
      },
    );
    const joined = joinMarketPathsToOrders(
      [
        order({
          execution_order_id: 'activity-a',
          client_order_id: 'client-a',
        }),
        order({
          execution_order_id: 'activity-b',
          parent_order_id: 'shared-parent',
        }),
        order({
          execution_order_id: 'activity-c',
          parent_order_id: 'shared-parent',
        }),
      ],
      data,
    );
    assert.equal(joined.rows[0].marketPaths[0].order.id, 'different-pm-id');
    assert.equal(joined.rows[1].marketPaths.length, 0);
    assert.equal(joined.rows[2].marketPaths.length, 0);
    assert.equal(joined.unlinked[0].reason, 'ambiguous_activity_order_match');
  });

  it('retains a cancelled terminal order when old data has no market-path samples', () => {
    const data = parseExecutionPathData(
      { orders: [] },
      {
        execution_paths: [
          {
            order: {
              id: 'pm-cancelled',
              market_path_completeness: 'unavailable',
            },
            terminal_reason: 'cancelled_by_operator',
            terminal_at_ms: 1_500,
          },
        ],
        execution_paths_truncated: false,
        has_more_orders: false,
      },
    );
    const joined = joinMarketPathsToOrders(
      [order({ execution_order_id: 'pm-cancelled' })],
      data,
    );
    assert.equal(joined.rows[0].marketPaths.length, 1);
    assert.equal(
      joined.rows[0].marketPaths[0].order.market_path?.length,
      undefined,
    );
    assert.equal(
      joined.rows[0].marketPaths[0].terminalReason,
      'cancelled_by_operator',
    );
  });

  it('keeps missing legacy contracts unavailable and reports history truncation as partial', () => {
    const legacy = parseExecutionPathData({ orders: [] }, { orders: [] });
    assert.equal(legacy.historyContractAvailable, false);
    assert.equal(
      buildExecutionQualitySnapshot([], emptyQuality, false, legacy)
        .pathEvidenceStatus,
      'partial',
    );
    const truncated = parseExecutionPathData(
      { orders: [] },
      {
        execution_paths: [],
        execution_paths_truncated: true,
        has_more_orders: false,
      },
    );
    assert.equal(
      buildExecutionQualitySnapshot([], emptyQuality, false, truncated)
        .pathEvidenceStatus,
      'partial',
    );
    const moreOrdersOnly = parseExecutionPathData(
      { orders: [] },
      {
        execution_paths: [],
        execution_paths_truncated: false,
        has_more_orders: true,
      },
    );
    assert.equal(
      buildExecutionQualitySnapshot([], emptyQuality, false, moreOrdersOnly)
        .pathEvidenceStatus,
      'available',
    );
    assert.equal(moreOrdersOnly.historyHasMoreOrders, true);
    const missing = parseExecutionPathData(null, null);
    assert.equal(
      buildExecutionQualitySnapshot([], emptyQuality, false, missing)
        .pathEvidenceStatus,
      'unavailable',
    );
  });

  it('keeps filter state and row projection in the model', () => {
    const fixture = sourceFixture([
      order({ symbol: 'BTC-USDT', status: 'expired' }),
      order({
        symbol: 'ETH-USDT',
        status: 'filled',
        failure_reason: undefined,
        filled_quantity: 1,
        execution_order_id: 'execution-eth',
      }),
    ]);
    const model = new AutoTraderExecutionQualityModel(
      fixture.source,
      async () => response(null),
    );

    assert.equal(model.state$.value.visibleRows.length, 2);
    model.setFilters({ symbol: 'ETH-USDT', outcome: 'filled' });
    assert.deepEqual(model.state$.value.filters, {
      symbol: 'ETH-USDT',
      outcome: 'filled',
    });
    assert.deepEqual(
      model.state$.value.visibleRows.map(({ order: item }) => item.symbol),
      ['ETH-USDT'],
    );
    model.setFilters({ symbol: '' });
    assert.equal(model.state$.value.visibleRows.length, 1);
    model.setView('orders');
    assert.equal(model.state$.value.view, 'orders');
    model.selectOrder('execution-eth');
    assert.equal(model.state$.value.selectedOrder?.order.symbol, 'ETH-USDT');
    model.setSort('symbol');
    assert.deepEqual(model.state$.value.sort, {
      field: 'symbol',
      direction: 'asc',
    });
    model.setSort('symbol');
    assert.equal(model.state$.value.sort.direction, 'desc');
    model.setFilters({ outcome: 'passive_expiry' });
    assert.equal(model.state$.value.selectedOrder, null);
    model.stop();
  });

  it('makes activation idempotent and fences late path responses after deactivation', async () => {
    const fixture = sourceFixture();
    const requests: Array<ReturnType<typeof deferred<Response>>> = [];
    const model = new AutoTraderExecutionQualityModel(fixture.source, () => {
      const request = deferred<Response>();
      requests.push(request);
      return request.promise;
    });
    let emissions = 0;
    const subscription = model.state$.subscribe(() => {
      emissions += 1;
    });

    model.start();
    model.start();
    assert.equal(fixture.starts, 1);
    assert.equal(requests.length, 2);
    model.stop();
    model.stop();
    assert.equal(fixture.stops, 1);
    const emissionsAtStop = emissions;
    assert.equal(model.state$.value.pathEvidenceStatus, 'loading');
    assert.equal(model.state$.value.pathEvidenceLoading, false);

    model.start();
    assert.equal(fixture.starts, 2);
    assert.equal(requests.length, 4);
    requests[2].resolve(response({ orders: [{ id: 'fresh-after-restart' }] }));
    requests[3].resolve(response(historyPayload));
    await model.refresh();
    assert.equal(model.state$.value.pathEvidenceStatus, 'available');
    requests[0].resolve(response({ orders: [{ id: 'late-before-restart' }] }));
    requests[1].resolve(response(historyPayload));
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.equal(
      model.state$.value.pathEvidence.activeOrders[0].id,
      'fresh-after-restart',
    );
    assert.equal(emissions, emissionsAtStop + 3);
    model.stop();
    subscription.unsubscribe();
  });

  it('preserves last successful path data as stale on failure and clears the error on recovery', async () => {
    const fixture = sourceFixture();
    let fail = false;
    const model = new AutoTraderExecutionQualityModel(
      fixture.source,
      async (path) => {
        if (fail) throw new Error('connection lost');
        return path.includes('/history')
          ? response(historyPayload)
          : response(activePayload);
      },
    );
    model.start();
    await model.refresh();
    assert.equal(model.state$.value.pathEvidenceStatus, 'available');

    fail = true;
    await model.refresh();
    assert.equal(
      model.state$.value.pathEvidenceError,
      'Active order paths: connection lost; Terminal path history: connection lost',
    );
    assert.equal(model.state$.value.pathEvidenceStale, true);
    assert.equal(model.state$.value.pathEvidenceStatus, 'available');

    fail = false;
    await model.refresh();
    assert.equal(model.state$.value.pathEvidenceError, null);
    assert.equal(model.state$.value.pathEvidenceStale, false);
    model.stop();
  });

  it('keeps independent widget instances isolated', () => {
    const first = sourceFixture([order({ symbol: 'BTC-USDT' })]);
    const second = sourceFixture([order({ symbol: 'ETH-USDT' })]);
    const firstModel = new AutoTraderExecutionQualityModel(
      first.source,
      async () => response(null),
    );
    const secondModel = new AutoTraderExecutionQualityModel(
      second.source,
      async () => response(null),
    );

    firstModel.setFilters({ symbol: 'BTC-USDT' });
    secondModel.setFilters({ symbol: 'ETH-USDT' });
    assert.deepEqual(
      firstModel.state$.value.visibleRows.map(({ order: item }) => item.symbol),
      ['BTC-USDT'],
    );
    assert.deepEqual(
      secondModel.state$.value.visibleRows.map(
        ({ order: item }) => item.symbol,
      ),
      ['ETH-USDT'],
    );
    firstModel.stop();
    secondModel.stop();
  });
});
