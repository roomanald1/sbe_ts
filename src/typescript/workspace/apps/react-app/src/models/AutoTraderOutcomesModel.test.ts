import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  applyInstanceTodayPnl,
  AutoTraderOutcomesModel,
  mergeLatestStrategyDecisions,
  parseLearningSummary,
  projectLedgerLifecycleIdentity,
  resolveLedgerOrder,
  strategyInstanceForClose,
} from './AutoTraderOutcomesModel';
import type { AutoTraderOutcome } from './AutoTraderOutcomesModel';
import { buildLegacyLedgerOutcomes } from './legacy_ledger_outcomes';
import type { PositionTrade } from './PositionTypes';

describe('AutoTraderOutcomesModel', () => {
  it('projects only explicit ledger lifecycle IDs onto the outcome row', () => {
    const legacy = {
      rowId: 'ledger-trade:legacy-fill',
      status: 'Closed' as const,
      decisionId: 'decision-legacy',
      symbol: 'BTC-USDT',
      side: 'buy',
      openedAt: 10,
      quantity: 1,
      signedQuantity: 1,
      entryPrice: 100,
      pnl: 2,
      entryFees: 0,
      estimatedExitFeeRate: 0,
      estimatedExitSlippageBps: 0,
      pnlQuoteCurrency: 'USDT',
      strategy: 'unattributed',
      direction: '—',
      strength: null,
      scout: false,
      source: 'unknown' as const,
      components: {},
      ledgerTrade: {
        id: 'ledger-fill-id',
        symbol: 'BTC-USDT',
        side: 'buy' as const,
        quantity: 1,
        execution_price: 100,
        timestamp: 10,
      },
    };
    assert.equal(projectLedgerLifecycleIdentity(legacy), legacy);

    const linked = {
      ...legacy,
      ledgerTrade: {
        ...legacy.ledgerTrade,
        lifecycle: {
          opportunity_id: 'opp-7',
          episode_id: 'episode-3',
          trade_id: 'trade-9',
        },
      },
    } as unknown as AutoTraderOutcome;
    const projected = projectLedgerLifecycleIdentity(linked);
    assert.equal(projected.opportunityId, 'opp-7');
    assert.equal(projected.episodeId, 'episode-3');
    assert.equal(projected.tradeId, 'trade-9');
    assert.equal(projected.rowId, legacy.rowId);
    assert.equal(projected.pnl, legacy.pnl);
  });

  it('leaves missing lifecycle links absent without deriving historical IDs', () => {
    const row = {
      rowId: 'ledger-trade:old-fill',
      status: 'Closed' as const,
      decisionId: 'decision-old',
      symbol: 'BTC-USDT',
      side: 'buy',
      openedAt: 1_234,
      quantity: 1,
      signedQuantity: 1,
      entryPrice: 100,
      pnl: 0,
      entryFees: 0,
      estimatedExitFeeRate: 0,
      estimatedExitSlippageBps: 0,
      pnlQuoteCurrency: 'USDT',
      strategy: 'unattributed',
      direction: '—',
      strength: null,
      scout: false,
      source: 'unknown' as const,
      components: {},
      ledgerTrade: {
        id: 'ledger-fill-id',
        symbol: 'BTC-USDT',
        side: 'buy' as const,
        quantity: 1,
        execution_price: 100,
        timestamp: 1_234,
        lifecycle: {
          opportunity_id: null,
          episode_id: '',
          trade_id: undefined,
        },
      },
    } as unknown as Parameters<typeof projectLedgerLifecycleIdentity>[0];
    const projected = projectLedgerLifecycleIdentity(row);
    assert.equal('opportunityId' in projected, false);
    assert.equal('episodeId' in projected, false);
    assert.equal('tradeId' in projected, false);
  });

  it('distinguishes unavailable Learning summaries from empty recent outcomes', () => {
    assert.deepEqual(parseLearningSummary(undefined), {
      rows: [],
      status: 'unavailable',
    });
    assert.deepEqual(parseLearningSummary([]), { rows: [], status: 'empty' });
    assert.deepEqual(
      parseLearningSummary([
        { strategy: 'live-momentum', learning_samples: 2 },
      ]),
      {
        rows: [{ strategy: 'live-momentum', learning_samples: 2 }],
        status: 'available',
      },
    );
  });

  it('uses the newest decision audit instead of an older strategy reason', () => {
    const latest = mergeLatestStrategyDecisions(
      {
        'live-momentum': {
          timestamp: 1_000,
          symbols: ['ETH-USDT'],
          reason: 'older order reason',
        },
      },
      [
        {
          stage: 'decision',
          timestamp: 2_000,
          status: 'skipped',
          strategy: 'live-momentum',
          symbol: 'BTC-USDT',
          reason: 'entry gate rejected',
        },
        {
          stage: 'decision',
          timestamp: 3_000,
          status: 'selected',
          strategy: 'live-momentum',
          symbol: 'XRP-USDT',
          reason: 'candidate selected',
        },
        {
          stage: 'entry',
          timestamp: 4_000,
          strategy: 'live-momentum',
          symbol: 'SOL-USDT',
          reason: 'not a strategy decision',
        },
      ],
    );
    assert.deepEqual(latest['live-momentum'], {
      timestamp: 2_000,
      symbols: ['BTC-USDT'],
      reason: 'entry gate rejected',
    });
  });

  it('keeps total net P&L and exposes today P&L separately', () => {
    const row = {
      rowId: 'ledger-summary:1',
      status: 'Closed' as const,
      decisionId: 'instance-1',
      symbol: 'CC-USDT',
      side: 'sell',
      openedAt: 1,
      quantity: 2,
      signedQuantity: -2,
      entryPrice: 1,
      pnl: 2,
      entryFees: 0,
      estimatedExitFeeRate: 0,
      estimatedExitSlippageBps: 0,
      pnlQuoteCurrency: 'USDT',
      strategy: 'live-momentum',
      direction: 'BEARISH',
      strength: null,
      scout: false,
      source: 'unknown' as const,
      components: {},
    };
    const [updated] = applyInstanceTodayPnl(
      [row],
      { 'instance-1': 4.25 },
      { USDT: 1 },
    );
    assert.equal(updated.netPnlUsd, 2);
    assert.equal(updated.todayRealizedUsd, 4.25);
  });

  it('keeps per-fill net P&L when applying the instance total to closed rows', () => {
    const row = {
      rowId: 'ledger-trade:fill-1',
      status: 'Closed' as const,
      decisionId: 'instance-1',
      symbol: 'CC-USDT',
      side: 'sell',
      openedAt: 1,
      quantity: 2,
      signedQuantity: -2,
      entryPrice: 1,
      pnl: 0,
      entryFees: 0,
      estimatedExitFeeRate: 0,
      estimatedExitSlippageBps: 0,
      pnlQuoteCurrency: 'USDT',
      strategy: 'live-momentum',
      direction: 'BEARISH',
      strength: null,
      scout: false,
      source: 'unknown' as const,
      components: {},
      netPnlUsd: -0.12,
      ledgerTrade: {
        id: 'fill-1',
        symbol: 'CC-USDT',
        side: 'sell' as const,
        quantity: 2,
        execution_price: 1,
        timestamp: 1,
      },
    };
    const [updated] = applyInstanceTodayPnl(
      [row],
      { 'instance-1': -0.4 },
      { USDT: 1 },
    );
    assert.equal(updated.netPnlUsd, -0.12);
  });

  it('resolves a protective barrier fill to its strategy entry instance', () => {
    const stop = {
      timestamp: 200,
      symbol: 'PEPE-USDT',
      side: 'buy',
      execution_order_id: 'barrier-200-1',
      purpose: 'stop-loss',
      strategy: 'live-vwap-dislocation',
      filled_quantity: 1,
    };
    const entry = {
      timestamp: 100,
      symbol: 'PEPE-USDT',
      side: 'sell',
      purpose: 'signal',
      strategy: 'live-vwap-dislocation',
      decision_id: 'entry-decision',
      filled_quantity: 1,
    };
    const trade: PositionTrade = {
      id: 'fill-1',
      symbol: 'PEPE-USDT',
      side: 'buy',
      quantity: 1,
      execution_price: 1,
      timestamp: 250,
      parent_order_id: 'barrier-200-1',
    };
    const resolved = resolveLedgerOrder(
      trade,
      new Map([[String(stop.timestamp), stop]]),
      [entry, stop],
    );
    assert.equal(resolved?.strategy, 'live-vwap-dislocation');
    assert.equal(resolved?.decision_id, 'entry-decision');
  });
  it('combines legacy entry and exit fills into realized position cycles', () => {
    const trades: PositionTrade[] = [
      {
        id: 'buy-win',
        symbol: 'BTC-USDT',
        side: 'buy',
        quantity: 1,
        execution_price: 100,
        timestamp: 1,
        parent_order_id: 'entry-win',
      },
      {
        id: 'sell-win',
        symbol: 'BTC-USDT',
        side: 'sell',
        quantity: 1,
        execution_price: 110,
        timestamp: 2,
        parent_order_id: 'exit-win',
      },
      {
        id: 'buy-loss',
        symbol: 'BTC-USDT',
        side: 'buy',
        quantity: 1,
        execution_price: 100,
        timestamp: 3,
        parent_order_id: 'entry-loss',
      },
      {
        id: 'sell-loss',
        symbol: 'BTC-USDT',
        side: 'sell',
        quantity: 1,
        execution_price: 90,
        timestamp: 4,
        parent_order_id: 'exit-loss',
      },
    ];
    const rows = buildLegacyLedgerOutcomes(
      'BTC-USDT',
      trades,
      'USDT',
      1,
      { 'exit-win': 10, 'exit-loss': -10 },
      { 'exit-win': 10 },
      new Map(),
    );
    const summaries = rows.filter((row) =>
      row.rowId.startsWith('ledger-summary:'),
    );
    assert.equal(summaries.length, 2);
    assert.deepEqual(
      summaries.map((row) => row.side),
      ['buy', 'buy'],
    );
    assert.deepEqual(
      summaries.map((row) => row.todayRealizedUsd),
      [10, 0],
    );
    assert.deepEqual(
      summaries.map((row) => row.netPnlUsd),
      [10, 0],
      'the closed ledger summary net uses the today-only window',
    );
    assert.equal(
      rows.filter((row) => row.rowId.startsWith('ledger-trade:')).length,
      4,
    );
  });

  it('keeps realized exit P&L when the entry fill is on an older unloaded page', () => {
    const rows = buildLegacyLedgerOutcomes(
      'BTC-USDT',
      [
        {
          id: 'exit-only',
          symbol: 'BTC-USDT',
          side: 'sell',
          quantity: 1,
          execution_price: 110,
          timestamp: 2,
          parent_order_id: 'exit',
        },
      ],
      'USDT',
      1,
      { exit: 8 },
      { exit: 8 },
      new Map(),
    );
    const summary = rows.find((row) => row.rowId.startsWith('ledger-summary:'));
    assert.equal(summary?.netPnlUsd, 8);
    assert.equal(summary?.todayRealizedUsd, 8);
  });

  it('keeps partially attributed closed fills visible without inventing P&L', () => {
    const rows = buildLegacyLedgerOutcomes(
      'BTC-USDT',
      [
        {
          id: 'unattributed-entry',
          symbol: 'BTC-USDT',
          side: 'buy',
          quantity: 1,
          execution_price: 100,
          timestamp: 1,
          parent_order_id: 'entry-order',
        },
        {
          id: 'unattributed-exit',
          symbol: 'BTC-USDT',
          side: 'sell',
          quantity: 1,
          execution_price: 110,
          timestamp: 2,
          parent_order_id: 'exit-order',
        },
      ],
      'USDT',
      1,
      {},
      {},
      new Map(),
    );
    const summary = rows.find((row) => row.rowId.startsWith('ledger-summary:'));
    assert.equal(summary?.status, 'Closed');
    assert.equal(summary?.netPnlUsd, undefined);
    assert.equal(summary?.todayRealizedUsd, undefined);
    assert.equal(
      rows.filter((row) => row.rowId.startsWith('ledger-trade:')).length,
      2,
    );
    const exit = rows.find(
      (row) => row.rowId === 'ledger-trade:unattributed-exit',
    );
    assert.equal(exit?.netPnlUsd, undefined);
    assert.equal(exit?.pnl, 0);
  });

  it('tracks close confirmation state', () => {
    const model = new AutoTraderOutcomesModel();
    assert.equal(model.state$.value.loading, true);
    model.requestClose('BTC-USDT');
    assert.equal(
      model.state$.value.confirmingCloseSymbols.has('BTC-USDT'),
      true,
    );
    model.cancelClose('BTC-USDT');
    assert.equal(
      model.state$.value.confirmingCloseSymbols.has('BTC-USDT'),
      false,
    );
    model.stop();
  });

  it('sends a strategy instance only when a symbol has one open strategy lot', () => {
    const rows = [
      { symbol: 'LINK-USDT', status: 'Open' as const, decisionId: 'daily-1' },
      { symbol: 'BTC-USDT', status: 'Closed' as const, decisionId: 'old-1' },
    ];
    assert.equal(strategyInstanceForClose(rows, 'LINK-USDT'), 'daily-1');
    assert.equal(
      strategyInstanceForClose(
        [
          ...rows,
          { symbol: 'LINK-USDT', status: 'Open', decisionId: 'daily-2' },
        ],
        'LINK-USDT',
      ),
      undefined,
    );
  });
});
