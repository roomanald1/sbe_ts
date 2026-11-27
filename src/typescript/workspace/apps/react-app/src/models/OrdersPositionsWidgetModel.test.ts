import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import type { AutoTraderOutcome } from './AutoTraderOutcomesModel';
import {
  barriersForPosition,
  buildActivityOnlyTerminalOrders,
  groupClosedPositionRows,
  linkActivityOrder,
  OrdersPositionsWidgetModel,
  parseLifecycleHistory,
  sortClosedPositions,
  sortOpenPositions,
  terminalPathForOrder,
  type LifecycleOrder,
} from './OrdersPositionsWidgetModel';

function outcome(
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

describe('OrdersPositionsWidgetModel', () => {
  it('groups summary and fill rows only by a unique canonical trade ID', () => {
    const summary = outcome({
      rowId: 'ledger-summary:trade-1',
      status: 'Closed',
      tradeId: 'trade-1',
      exitReason: 'stop-loss',
      netPnlUsd: -0.01,
    });
    const entryFill = outcome({
      rowId: 'ledger-trade:entry-fill',
      status: 'Closed',
      ledgerTrade: {
        id: 'entry-fill',
        lifecycle: { trade_id: 'trade-1' },
      } as unknown as AutoTraderOutcome['ledgerTrade'],
    });
    const exitFill = outcome({
      rowId: 'ledger-trade:exit-fill',
      status: 'Closed',
      ledgerTrade: {
        id: 'exit-fill',
        lifecycle: { trade_id: 'trade-1' },
      } as unknown as AutoTraderOutcome['ledgerTrade'],
    });
    const sameSymbolOtherTrade = outcome({
      rowId: 'ledger-trade:other-trade',
      status: 'Closed',
      symbol: 'BTC-USDT',
      ledgerTrade: {
        id: 'other-trade',
        lifecycle: { trade_id: 'trade-2' },
      } as unknown as AutoTraderOutcome['ledgerTrade'],
    });
    const legacy = outcome({
      rowId: 'ledger-trade:legacy-fill',
      status: 'Closed',
    });

    const grouped = groupClosedPositionRows([
      summary,
      entryFill,
      exitFill,
      sameSymbolOtherTrade,
      legacy,
    ]);
    assert.equal(grouped.length, 3);
    assert.deepEqual(
      grouped[0].fillDetails.map((fill) => fill.ledgerTrade?.id),
      ['entry-fill', 'exit-fill'],
    );
    assert.equal(grouped[0].exitReason, 'stop-loss');
    assert.equal(grouped[1].rowId, 'ledger-trade:other-trade');
    assert.equal(grouped[1].fillDetails.length, 0);
    assert.equal(grouped[2].rowId, 'ledger-trade:legacy-fill');
  });

  it('parses PM history without defaulting missing contracts or numeric fields', () => {
    assert.equal(parseLifecycleHistory({ orders: [] }), null);
    const parsed = parseLifecycleHistory({
      orders: [{ id: 'order-1', quantity: 1, remaining_quantity: 0 }],
      barrier_orders: [],
      active_orders: [],
      active_barrier_orders: [],
      has_more_orders: false,
      has_more_barrier_orders: false,
      execution_paths: [],
      execution_paths_truncated: true,
    });
    assert.ok(parsed);
    assert.equal(parsed.orders[0].remaining_quantity, 0);
    assert.equal(parsed.hasMoreOrders, false);
    assert.equal(parsed.executionPathsTruncated, true);
    assert.equal(parsed.nextOrdersCursor, null);
  });

  it('links PM order activity by exact stable IDs and never by symbol or time', () => {
    const activity = [
      {
        execution_order_id: 'pm-order-1',
        decision_id: 'decision-1',
        symbol: 'ETH-USDT',
      },
      {
        execution_order_id: 'different-order',
        decision_id: 'decision-2',
        symbol: 'BTC-USDT',
      },
    ];
    const pmOrder: LifecycleOrder = {
      id: 'pm-order-1',
      symbol: 'BTC-USDT',
      timestamp: 1_000,
    };
    assert.equal(
      linkActivityOrder(pmOrder, activity)?.decision_id,
      'decision-1',
    );
    assert.equal(
      linkActivityOrder({ symbol: 'BTC-USDT', timestamp: 1_000 }, activity),
      null,
    );
  });

  it('preserves cancelled or expired terminal reason only for a unique stable order link', () => {
    const order: LifecycleOrder = { id: 'pm-order-1' };
    const paths = [
      {
        order: { id: 'pm-order-1' },
        terminal_reason: 'expired_without_fill',
        terminal_at_ms: 5_000,
      },
      {
        order: { id: 'pm-order-2' },
        terminal_reason: 'cancelled',
        terminal_at_ms: 5_001,
      },
    ];
    assert.equal(
      terminalPathForOrder(order, paths)?.terminal_reason,
      'expired_without_fill',
    );
    assert.equal(terminalPathForOrder({ symbol: 'BTC-USDT' }, paths), null);
    assert.equal(terminalPathForOrder(order, [...paths, paths[0]]), null);
  });

  it('links protective barriers through stable lifecycle or parent-order decision lineage only', () => {
    const position = outcome();
    const activityOrders = [
      { execution_order_id: 'entry-order', decision_id: 'decision-1' },
    ];
    const linked = {
      id: 'barrier-1',
      symbol: 'ETH-USDT',
      parent_order_id: 'entry-order',
      status: 'armed',
    };
    const unrelated = {
      id: 'barrier-2',
      symbol: 'BTC-USDT',
      parent_order_id: 'different-order',
      status: 'armed',
    };
    assert.deepEqual(
      barriersForPosition(position, [linked, unrelated], activityOrders),
      [linked],
    );
    assert.deepEqual(
      barriersForPosition(
        outcome({ decisionId: 'ledger:BTC-USDT:unknown' }),
        [linked],
        activityOrders,
      ),
      [],
    );
    assert.deepEqual(
      barriersForPosition(
        outcome({ tradeId: 'trade-1' }),
        [{ id: 'barrier-3', lifecycle: { trade_id: 'trade-1' } }],
        [],
      ),
      [{ id: 'barrier-3', lifecycle: { trade_id: 'trade-1' } }],
    );
  });

  it('loads the existing bounded PM history contract and records fetch time without synthesizing rows', async () => {
    const requested: string[] = [];
    const model = new OrdersPositionsWidgetModel(
      undefined,
      async (path) => {
        requested.push(path);
        return new Response(
          JSON.stringify({
            orders: [],
            barrier_orders: [],
            active_orders: [],
            active_barrier_orders: [],
            execution_paths: [],
            execution_paths_truncated: false,
            has_more_orders: false,
            has_more_barrier_orders: false,
          }),
          { status: 200 },
        );
      },
      () => 42,
    );
    await model.refreshHistory();
    assert.deepEqual(requested, ['/positions/orders/history?limit=200']);
    assert.equal(model.state$.value.pmAvailable, true);
    assert.equal(model.state$.value.history.activeOrders.length, 0);
    assert.equal(model.state$.value.positions.length, 0);
    assert.equal(model.state$.value.fetchedAtMs, 42);
  });

  it('shows bounded AutoTrader terminal submissions when no exact PM order record is linked', () => {
    const source = Array.from({ length: 3 }, (_, index) => ({
      timestamp: 1_000 + index,
      symbol: 'BTC-USDT',
      side: 'buy',
      execution_order_id: `execution-${index}`,
      decision_id: `decision-${index}`,
      status: index === 2 ? 'resting' : 'expired',
      quantity: 1,
      filled_quantity: 0,
    }));
    const result = buildActivityOnlyTerminalOrders(source, [], 1);
    assert.equal(result.sourceCount, 2);
    assert.equal(result.rows.length, 1);
    assert.equal(result.rows[0].activity.execution_order_id, 'execution-1');
    assert.equal(result.rows[0].activity.filled_quantity, 0);
    assert.equal(result.truncated, true);
  });

  it('deduplicates exact execution/client IDs but does not deduplicate by symbol or time', () => {
    const source = [
      {
        timestamp: 1_000,
        symbol: 'BTC-USDT',
        side: 'buy',
        execution_order_id: 'pm-1',
        status: 'expired',
      },
      {
        timestamp: 1_000,
        symbol: 'BTC-USDT',
        side: 'buy',
        execution_order_id: 'activity-only',
        status: 'cancelled',
      },
      {
        timestamp: 1_000,
        symbol: 'BTC-USDT',
        side: 'buy',
        client_order_id: 'client-match',
        status: 'rejected',
      },
    ];
    const pmOrders: LifecycleOrder[] = [
      { id: 'pm-1', symbol: 'ETH-USDT', timestamp: 7_000 },
      { id: 'pm-3', client_order_id: 'client-match', symbol: 'SOL-USDT' },
    ];
    const result = buildActivityOnlyTerminalOrders(source, pmOrders);
    assert.deepEqual(
      result.rows.map(
        (row) =>
          row.activity.execution_order_id ?? row.activity.client_order_id,
      ),
      ['activity-only'],
    );
  });

  it('does not hide activity records behind an ambiguous shared parent ID', () => {
    const result = buildActivityOnlyTerminalOrders(
      [
        {
          parent_order_id: 'parent',
          status: 'cancelled',
          decision_id: 'decision-1',
        },
      ],
      [
        { id: 'order-a', parent_order_id: 'parent' },
        { id: 'order-b', parent_order_id: 'parent' },
      ],
    );
    assert.equal(result.rows.length, 1);
  });

  it('sorts open and closed rows stably by selected table keys while keeping missing values last', () => {
    const open = [
      outcome({
        rowId: 'b',
        symbol: 'ETH-USDT',
        openedAt: 2_000,
        entryNotionalUsd: 20,
      }),
      outcome({
        rowId: 'a',
        symbol: 'BTC-USDT',
        openedAt: 1_000,
        entryNotionalUsd: 10,
      }),
      outcome({ rowId: 'c', symbol: 'SOL-USDT', openedAt: 3_000 }),
    ];
    assert.deepEqual(
      sortOpenPositions(open, { key: 'symbol', direction: 'asc' }).map(
        (row) => row.rowId,
      ),
      ['a', 'b', 'c'],
    );
    assert.deepEqual(
      sortOpenPositions(open, { key: 'notional', direction: 'desc' }).map(
        (row) => row.rowId,
      ),
      ['b', 'a', 'c'],
    );
    const closed = [
      {
        ...open[0],
        status: 'Closed' as const,
        closedAt: 4_000,
        totalFees: 2,
        fillDetails: [],
      },
      {
        ...open[1],
        status: 'Closed' as const,
        closedAt: 3_000,
        totalFees: 1,
        fillDetails: [],
      },
    ];
    assert.deepEqual(
      sortClosedPositions(closed, { key: 'closedAt', direction: 'desc' }).map(
        (row) => row.rowId,
      ),
      ['b', 'a'],
    );
  });

  it('keeps sort commands in the per-widget model state', () => {
    const model = new OrdersPositionsWidgetModel();
    model.sortOpenBy('strategy');
    assert.deepEqual(model.state$.value.openSort, {
      key: 'strategy',
      direction: 'asc',
    });
    model.sortOpenBy('strategy');
    assert.deepEqual(model.state$.value.openSort, {
      key: 'strategy',
      direction: 'desc',
    });
    model.sortClosedBy('net');
    assert.deepEqual(model.state$.value.closedSort, {
      key: 'net',
      direction: 'asc',
    });
    model.stop();
  });
});
