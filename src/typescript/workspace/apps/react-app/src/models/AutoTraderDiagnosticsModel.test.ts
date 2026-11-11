import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { AutoTraderModel } from './AutoTraderModel';
import {
  AutoTraderDiagnosticsModel,
  buildAutoTraderDiagnosticRows,
} from './AutoTraderDiagnosticsModel';

describe('buildAutoTraderDiagnosticRows', () => {
  it('groups gate hits, computes readiness and expands details', () => {
    const rows = buildAutoTraderDiagnosticRows(
      [
        {
          symbol: 'ETH-USDT',
          direction: 'long',
          gate: 'Spread',
          count: 2,
          last_detail: 'wide',
          distance_to_pass: 0.5,
        },
        {
          symbol: 'ETH-USDT',
          direction: 'long',
          gate: 'Volume',
          count: 1,
          last_detail: 'low',
          distance_to_pass: 0.25,
        },
        {
          symbol: 'BTC-USDT',
          direction: 'short',
          gate: 'Readiness',
          count: 1,
          last_detail: 'ready',
          distance_to_pass: 0,
        },
      ],
      new Set(['ETH-USDT:long']),
    );

    assert.deepEqual(
      rows.map((row) => row.kind),
      ['symbol', 'symbol', 'gate', 'gate'],
    );
    const btc = rows[0];
    assert.equal(btc.kind, 'symbol');
    if (btc.kind !== 'symbol') return;
    assert.equal(btc.readinessScore, 100);
    assert.equal(btc.expanded, false);
    const eth = rows[1];
    assert.equal(eth.kind, 'symbol');
    if (eth.kind !== 'symbol') return;
    assert.equal(eth.distance, 0.5);
    assert.equal(eth.readinessScore, 57);
    assert.equal(eth.expanded, true);
  });

  it('treats missing distances as a full readiness penalty', () => {
    const [row] = buildAutoTraderDiagnosticRows(
      [
        {
          symbol: 'SOL-USDT',
          direction: 'long',
          gate: 'Liquidity',
          count: 1,
          last_detail: 'missing',
          distance_to_pass: null,
        },
      ],
      new Set(),
    );
    assert.equal(row.kind, 'symbol');
    if (row.kind === 'symbol') {
      assert.equal(row.distance, null);
      assert.equal(row.readinessScore, 50);
    }
  });
});

describe('AutoTraderDiagnosticsModel', () => {
  it('owns group expansion and publishes updated rows when toggled', () => {
    const source = new AutoTraderModel();
    source.state$.next({
      ...source.state$.value,
      symbolGateHits: [
        {
          symbol: 'BTC-USDT',
          direction: 'long',
          gate: 'Liquidity',
          count: 1,
          last_detail: 'thin book',
          distance_to_pass: 0.2,
        },
      ],
    });
    const model = new AutoTraderDiagnosticsModel(source);
    const observedCounts: number[] = [];
    model.state$.subscribe((state) =>
      observedCounts.push(state.diagnosticRows.length),
    );

    assert.equal(model.state$.value.symbolGroupCount, 1);
    assert.equal(model.state$.value.diagnosticRows.length, 1);
    model.toggleGroup('BTC-USDT', 'long');
    assert.equal(model.state$.value.diagnosticRows.length, 2);
    assert.equal(model.state$.value.diagnosticRows[0].kind, 'symbol');
    assert.equal(model.state$.value.diagnosticRows[0].expanded, true);
    assert.deepEqual(observedCounts, [1, 2]);

    model.toggleGroup('BTC-USDT', 'long');
    assert.equal(model.state$.value.diagnosticRows.length, 1);
  });
});
