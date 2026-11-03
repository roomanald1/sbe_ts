import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { AutoTraderOutcomesModel } from './AutoTraderOutcomesModel';
import { buildLegacyLedgerOutcomes } from './legacy_ledger_outcomes';
import type { PositionTrade } from './PositionTypes';

describe('AutoTraderOutcomesModel', () => {
  it('combines legacy entry and exit fills into realized position cycles', () => {
    const trades: PositionTrade[] = [
      { id: 'buy-win', symbol: 'BTC-USDT', side: 'buy', quantity: 1, execution_price: 100, timestamp: 1, parent_order_id: 'entry-win' },
      { id: 'sell-win', symbol: 'BTC-USDT', side: 'sell', quantity: 1, execution_price: 110, timestamp: 2, parent_order_id: 'exit-win' },
      { id: 'buy-loss', symbol: 'BTC-USDT', side: 'buy', quantity: 1, execution_price: 100, timestamp: 3, parent_order_id: 'entry-loss' },
      { id: 'sell-loss', symbol: 'BTC-USDT', side: 'sell', quantity: 1, execution_price: 90, timestamp: 4, parent_order_id: 'exit-loss' },
    ];
    const rows = buildLegacyLedgerOutcomes('BTC-USDT', trades, 'USDT', 1, { 'exit-win': 10, 'exit-loss': -10 }, new Map());
    const summaries = rows.filter((row) => row.rowId.startsWith('ledger-summary:'));
    assert.equal(summaries.length, 2);
    assert.deepEqual(summaries.map((row) => row.netPnlUsd), [10, -10]);
    assert.deepEqual(summaries.map((row) => row.side), ['buy', 'buy']);
    assert.equal(rows.filter((row) => row.rowId.startsWith('ledger-trade:')).length, 4);
  });

  it('tracks close confirmation state', () => {
    const model = new AutoTraderOutcomesModel();
    assert.equal(model.state$.value.loading, true);
    model.requestClose('BTC-USDT');
    assert.equal(model.state$.value.confirmingCloseSymbols.has('BTC-USDT'), true);
    model.cancelClose('BTC-USDT');
    assert.equal(model.state$.value.confirmingCloseSymbols.has('BTC-USDT'), false);
    model.stop();
  });
});
