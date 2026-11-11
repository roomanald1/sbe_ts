import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { BehaviorSubject } from 'rxjs';
import { AccountModel } from './AccountModel';
import { AutoTraderModel } from './AutoTraderModel';
import { AutoTraderOutcomesModel } from './AutoTraderOutcomesModel';
import { AutoTraderScoutModel } from './AutoTraderScoutModel';
import { CloseInsightsModel } from './CloseInsightsModel';
import { DatedCarryModel } from './DatedCarryModel';
import { ExecutionFeesModel } from './ExecutionFeesModel';
import { FillsModel } from './FillsModel';
import { MarketLiquidityModel } from './MarketLiquidityModel';
import { OrderBookModel } from './OrderBookModel';
import { OrderManagerBookFeeds } from './OrderManagerBookFeeds';
import { OrderManagerModel } from './OrderManagerModel';
import { PnlModel } from './PnlModel';
import { PriceChangeChartModel } from './PriceChangeChartModel';
import { SignalInfoModel } from './SignalInfoModel';
import { SignalsModel } from './SignalsModel';
import { TradeTicketModel } from './TradeTicketModel';
import { UsersModel } from './UsersModel';
import { deriveOrderManagerState } from './OrderManagerDerived';
import { orderSubmissionBody } from './OrderManagerSubmission';
import { fetchFeeRates } from './fee_rates';
import type { Connection } from '../connection';

const connection = {} as Connection;
function bookConnection(): Connection {
  return {
    openBook: (key: string, symbol: string) => ({
      key,
      symbol,
      data: new BehaviorSubject<unknown | undefined>(undefined),
      error: new BehaviorSubject<string | undefined>(undefined),
    }),
    closeBook: () => undefined,
  } as unknown as Connection;
}

describe('model initial state and simple actions', () => {
  it('initializes AccountModel and supports idempotent stop', () => {
    const model = new AccountModel();
    assert.equal(model.state$.value.loading, true);
    assert.equal(model.state$.value.max_leverage, 2);
    model.stop();
  });

  it('updates AutoTraderModel limit inputs in state', () => {
    const model = new AutoTraderModel();
    model.setLimits('250', '2000', '25', '15', '8');
    assert.equal(model.state$.value.maxOrderNotional, '250');
    assert.equal(model.state$.value.dailyNotionalLimit, '2000');
    assert.equal(model.state$.value.explorationPercent, '15');
    model.stop();
  });

  it('initializes outcomes and scout models in loading state', () => {
    const outcomes = new AutoTraderOutcomesModel();
    assert.equal(outcomes.state$.value.loading, true);
    assert.deepEqual(outcomes.state$.value.rows, []);
    const scouts = new AutoTraderScoutModel();
    assert.deepEqual(scouts.state$.value.rows, []);
    assert.equal(scouts.state$.value.loading, true);
    outcomes.stop();
    scouts.stop();
  });

  it('tracks close insight settings and resets confirmation on cancel', () => {
    const model = new CloseInsightsModel();
    model.setBarrierTargetUsd('25');
    assert.equal(model.state$.value.barrierTargetUsd, '25');
    model.cancelClose();
    assert.equal(model.state$.value.confirmingInsight, undefined);
    model.stop();
  });

  it('updates the dated carry notional draft', () => {
    const model = new DatedCarryModel();
    model.setNotional('1500');
    assert.equal(model.state$.value.notional, '1500');
    model.stop();
  });

  it('initializes execution fees and fills views empty', () => {
    const fees = new ExecutionFeesModel();
    assert.deepEqual(fees.state$.value.rows, []);
    assert.equal(fees.state$.value.loading, true);
    const fills = new FillsModel();
    assert.deepEqual(fills.state$.value.fills, []);
    assert.equal(fills.getSnapshot(), fills.state$.value);
    fees.stop();
    fills.stop();
  });

  it('initializes liquidity and order book models for the selected symbol', () => {
    const liquidity = new MarketLiquidityModel({
      symbol: 'BTC-USDT',
      connection,
      streamKey: 'test',
    });
    assert.equal(liquidity.state$.value.symbol, 'BTC-USDT');
    assert.deepEqual(liquidity.state$.value.bands, []);
    const book = new OrderBookModel({
      symbol: 'ETH-USDT',
      connection,
      streamKey: 'test',
    });
    assert.equal(book.getSnapshot().symbol, 'ETH-USDT');
    assert.equal(book.state$.value.frozen, false);
    book.stop();
    liquidity.stop();
  });

  it('constructs book feeds and order manager state from symbols', () => {
    const feeds = new OrderManagerBookFeeds(
      connection,
      () => undefined,
      () => undefined,
    );
    feeds.sync(new Set());
    feeds.close();
    const model = new OrderManagerModel({
      connection: bookConnection(),
      symbols: ['BTC-USDT', 'ETH-USDT'],
    });
    assert.equal(model.state$.value.selectedSymbol, 'BTC-USDT');
    assert.equal(model.state$.value.orderType, 'ioc');
    model.setSymbol('ETH-USDT');
    assert.equal(model.state$.value.selectedSymbol, 'ETH-USDT');
    model.stop();
  });

  it('initializes PnL and derives its connection snapshot', () => {
    const model = new PnlModel(connection);
    assert.deepEqual(model.getSnapshot().rows, []);
    assert.equal(model.props$.value.connection, connection);
    model.setConnection(connection);
    model.stop();
  });

  it('updates chart dimensions and selects symbols', () => {
    const model = new PriceChangeChartModel({
      changes: { ticks: [], histories: new Map() },
      initialSymbol: 'BTC-USDT',
    });
    assert.equal(model.getSnapshot().chartSize.width, 900);
    model.setChartSize(0, 300);
    assert.equal(model.getSnapshot().chartSize.width, 900);
    model.setChartSize(640, 320);
    assert.deepEqual(model.getSnapshot().chartSize, {
      width: 640,
      height: 320,
    });
    model.selectSymbol('ETH-USDT');
    assert.equal(model.props$.value.initialSymbol, 'BTC-USDT');
  });

  it('initializes signal information and connects signal model to its symbol', () => {
    const info = new SignalInfoModel();
    assert.deepEqual(info.state$.value.best, []);
    info.stop();
    const signals = new SignalsModel({
      connection,
      symbol: 'BTC-USDT',
      streamKey: 'test',
    });
    assert.deepEqual(signals.state$.value, { symbol: 'BTC-USDT' });
    signals.stop();
  });

  it('initializes trade ticket and updates order fields', () => {
    const model = new TradeTicketModel({
      symbol: 'BTC-USDT',
      initialSide: 'sell',
      connection,
    });
    assert.equal(model.state$.value.side, 'sell');
    assert.equal(model.state$.value.barrierDirection, 'above');
    model.setQuantity('2.5');
    model.setOrderType('limit');
    assert.equal(model.state$.value.quantity, '2.5');
    assert.equal(model.state$.value.orderType, 'limit');
    model.stop();
  });

  it('paginates the users view input and blocks invalid page numbers', async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = async () =>
      ({
        ok: true,
        json: async () => ({ data: [], has_more: false }),
      }) as Response;
    const model = new UsersModel();
    assert.equal(model.state$.value.rowData.length, 10);
    model.setPageInput('0');
    model.goToPage();
    assert.equal(model.state$.value.page, 0);
    model.setPageInput('3');
    model.goToPage();
    assert.equal(model.state$.value.page, 2);
    await model.refresh();
    assert.equal(model.state$.value.isError, false);
    model.stop();
    globalThis.fetch = originalFetch;
  });
});

describe('order manager derivation helpers', () => {
  it('returns no fee rates before a position snapshot is available', async () => {
    assert.equal(await fetchFeeRates(), undefined);
  });

  it('derives midpoint, fees, and order rows from state', () => {
    const state = deriveOrderManagerState({
      symbols: ['BTC-USDT'],
      books: {
        'BTC-USDT': {
          symbol: 'BTC-USDT',
          bids: [{ price: 99 }],
          asks: [{ price: 101 }],
        },
      },
      bookErrors: {},
      selectedSymbol: 'BTC-USDT',
      side: 'buy',
      quantity: '2',
      orderType: 'ioc',
      limitPrice: '',
      barrierPrice: '',
      barrierDirection: 'below',
      orders: [],
      barrierOrders: [],
      vwapRuns: [],
      rows: [],
      makerFeeRate: 0.001,
      takerFeeRate: 0.002,
      durationMinutes: '60',
      participationRate: '10',
      loading: false,
      submitting: false,
    } as never);
    assert.equal(state.selectedCurrentPrice, 100);
    assert.equal(state.estimatedFee, 0.4);
    assert.deepEqual(state.rows, []);
  });

  it('serializes VWAP and barrier submissions with their specific fields', () => {
    const state = {
      selectedSymbol: 'BTC-USDT',
      side: 'sell',
      orderType: 'vwap',
      barrierDirection: 'above',
    } as any;
    assert.deepEqual(
      orderSubmissionBody(state, {
        quantity: 2,
        limitPrice: 0,
        barrierPrice: 0,
        durationMinutes: 30,
        participationRate: 0.2,
      }),
      {
        symbol: 'BTC-USDT',
        side: 'sell',
        duration_minutes: 30,
        max_participation_rate: 0.2,
        quantity: 2,
      },
    );
    const barrier = orderSubmissionBody(
      { ...state, orderType: 'barrier' } as never,
      {
        quantity: 2,
        limitPrice: 0,
        barrierPrice: 105,
        durationMinutes: 30,
        participationRate: 0.2,
      },
    );
    assert.deepEqual(barrier, {
      symbol: 'BTC-USDT',
      side: 'sell',
      order_type: 'barrier',
      quantity: 2,
      trigger_price: 105,
      trigger_direction: 'above',
    });
  });
});
