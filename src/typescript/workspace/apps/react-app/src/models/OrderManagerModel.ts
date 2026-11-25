import { BehaviorSubject } from 'rxjs';
import type { Connection } from '../connection';
import { deriveOrderManagerState } from './OrderManagerDerived';
import { OrderManagerBookFeeds } from './OrderManagerBookFeeds';
import type {
  BarrierDirection,
  BarrierOrder,
  OrderManagerProps,
  OrderManagerState,
  OrderType,
  OrdersHistoryResponse,
  RestingOrder,
  TradeSide,
  VwapRun,
} from './OrderManagerTypes';
import {
  orderSubmissionBody,
  validateOrderSubmission,
} from './OrderManagerSubmission';
import {
  currentPositionSnapshot,
  subscribePositionStream,
} from '../position_stream';
import { sharedGet } from '../shared_get';
import { paperExchangeFetch } from '../paper_exchange_api';

const ORDERS_HISTORY_PAGE_SIZE = 50;

function mergeById<T extends { id: string }>(current: T[], updates: T[]): T[] {
  const merged = new Map(current.map((item) => [item.id, item]));
  for (const item of updates) merged.set(item.id, item);
  return [...merged.values()];
}

function activeRestingOrders(orders: RestingOrder[]): RestingOrder[] {
  return orders.filter((order) => order.remaining_quantity > 0);
}

function activeBarrierOrders(orders: BarrierOrder[]): BarrierOrder[] {
  return orders.filter((order) =>
    ['armed', 'triggering'].includes(order.status),
  );
}

export class OrderManagerModel {
  readonly props$: BehaviorSubject<OrderManagerProps>;
  readonly state$: BehaviorSubject<OrderManagerState>;
  private bookFeeds!: OrderManagerBookFeeds;
  private timer?: ReturnType<typeof setInterval>;
  private stopPositionStream?: () => void;
  private lastPositionStreamAt = 0;
  private refreshingOrders = false;
  private historyOrders: RestingOrder[] = [];
  private historyBarrierOrders: BarrierOrder[] = [];
  private activeOrders: RestingOrder[] = [];
  private activeBarrierOrders: BarrierOrder[] = [];
  private nextOrdersCursor?: string;
  private nextBarrierOrdersCursor?: string;
  private loadingMoreHistory = false;
  constructor(props: OrderManagerProps) {
    this.props$ = new BehaviorSubject(props);
    this.state$ = new BehaviorSubject<OrderManagerState>({
      symbols: props.symbols,
      books: {},
      bookErrors: {},
      selectedSymbol: props.symbols[0] ?? '',
      side: 'buy',
      quantity: '',
      orderType: 'ioc',
      limitPrice: '',
      barrierPrice: '',
      barrierDirection: 'below',
      durationMinutes: '60',
      participationRate: '10',
      makerFeeRate: 0.0008,
      takerFeeRate: 0.001,
      orders: [],
      barrierOrders: [],
      loadingMoreHistory: false,
      hasMoreOrdersHistory: false,
      hasMoreBarrierHistory: false,
      vwapRuns: [],
      rows: [],
      loading: true,
      submitting: false,
    });
    this.bookFeeds = new OrderManagerBookFeeds(
      props.connection,
      (symbol, depth) => {
        const books = { ...this.state$.value.books };
        if (depth) books[symbol] = depth;
        else delete books[symbol];
        const bookErrors = { ...this.state$.value.bookErrors };
        if (depth) delete bookErrors[symbol];
        this.patch({ books, bookErrors });
      },
      (symbol, error) => {
        const bookErrors = { ...this.state$.value.bookErrors };
        if (error) bookErrors[symbol] = error;
        else delete bookErrors[symbol];
        this.patch({ bookErrors });
      },
    );
  }
  private get connection(): Connection {
    return this.props$.value.connection;
  }
  readonly getSnapshot = (): OrderManagerState => this.state$.value;
  readonly subscribe = (notify: () => void): (() => void) => {
    const sub = this.state$.subscribe(notify);
    return () => sub.unsubscribe();
  };
  updateProps(props: OrderManagerProps): void {
    const oldConnection = this.connection;
    this.props$.next(props);
    const selectedSymbol = props.symbols.includes(
      this.state$.value.selectedSymbol,
    )
      ? this.state$.value.selectedSymbol
      : (props.symbols[0] ?? '');
    this.patch({ symbols: props.symbols, selectedSymbol });
    if (oldConnection !== props.connection) {
      this.bookFeeds.updateConnection(props.connection);
      this.syncFeeds();
    }
  }
  start(): void {
    if (this.timer) return;
    this.syncFeeds();
    this.stopPositionStream = subscribePositionStream((snapshot) => {
      this.lastPositionStreamAt = Date.now();
      this.applyStreamOrders(
        snapshot.orders.orders ?? [],
        snapshot.orders.barrier_orders ?? [],
      );
      const vwapRuns = (snapshot.vwap_runs?.runs ?? []) as VwapRun[];
      const selectedRun =
        vwapRuns.find((run) => run.id === this.state$.value.vwapRunId) ??
        [...vwapRuns]
          .reverse()
          .find(
            (run) =>
              run.symbol === this.state$.value.selectedSymbol &&
              run.status === 'running',
          );
      this.patch({
        vwapRuns,
        ...(snapshot.fee_rates
          ? {
              makerFeeRate: snapshot.fee_rates.maker,
              takerFeeRate: snapshot.fee_rates.taker,
            }
          : {}),
        ...(selectedRun
          ? {
              vwapRunId: selectedRun.id,
              vwapStatus: selectedRun.status,
              vwapExecutedQuantity: selectedRun.executed_quantity,
            }
          : {}),
        loading: false,
        error: undefined,
      });
    });
    void this.loadFeeRates();
    void this.refreshOrders();
    void this.loadActiveVwap(this.state$.value.selectedSymbol);
    this.timer = setInterval(() => {
      if (Date.now() - this.lastPositionStreamAt > 45_000)
        void this.refreshOrders(false);
    }, 30_000);
  }
  stop(): void {
    this.stopPositionStream?.();
    this.stopPositionStream = undefined;
    if (this.timer) clearInterval(this.timer);
    this.timer = undefined;
    this.closeFeeds();
  }
  setSymbol(selectedSymbol: string): void {
    this.patch({ selectedSymbol, confirmedLimitPrice: undefined });
    this.syncFeeds();
  }
  setSide(side: TradeSide): void {
    this.patch({ side, barrierDirection: side === 'sell' ? 'above' : 'below' });
  }
  setQuantity(quantity: string): void {
    this.patch({ quantity });
  }
  setOrderType(orderType: OrderType): void {
    this.patch({ orderType, confirmedLimitPrice: undefined });
  }
  setDurationMinutes(durationMinutes: string): void {
    this.patch({ durationMinutes });
  }
  setParticipationRate(participationRate: string): void {
    this.patch({ participationRate });
  }
  setLimitPrice(limitPrice: string): void {
    this.patch({
      limitPrice,
      confirmedLimitPrice: undefined,
      error: undefined,
    });
  }
  setBarrierPrice(barrierPrice: string): void {
    this.patch({ barrierPrice, error: undefined });
  }
  setBarrierDirection(barrierDirection: BarrierDirection): void {
    this.patch({ barrierDirection });
  }
  private async loadFeeRates(): Promise<void> {
    const rates = currentPositionSnapshot()?.fee_rates;
    if (rates && Number.isFinite(rates.maker) && Number.isFinite(rates.taker))
      this.patch({ makerFeeRate: rates.maker, takerFeeRate: rates.taker });
  }
  async refreshOrders(resetHistory = true): Promise<void> {
    if (this.refreshingOrders) return;
    this.refreshingOrders = true;
    try {
      const response = await sharedGet(
        '/positions/orders/history?limit=' + ORDERS_HISTORY_PAGE_SIZE,
      );
      if (!response.ok)
        throw new Error(
          'Could not load resting orders (' + response.status + ')',
        );
      const data = (await response.json()) as OrdersHistoryResponse;
      if (!Array.isArray(data.orders))
        throw new Error('The orders API returned invalid data');
      if (
        (data.has_more_orders && !data.next_orders_cursor) ||
        (data.has_more_barrier_orders && !data.next_barrier_orders_cursor)
      )
        throw new Error('The orders API returned an invalid history cursor');
      const pageBarrierOrders = data.barrier_orders ?? [];
      const startFreshHistory =
        resetHistory ||
        (this.historyOrders.length === 0 &&
          this.historyBarrierOrders.length === 0);
      if (startFreshHistory) {
        this.historyOrders = data.orders;
        this.historyBarrierOrders = pageBarrierOrders;
        this.nextOrdersCursor = data.next_orders_cursor;
        this.nextBarrierOrdersCursor = data.next_barrier_orders_cursor;
      } else {
        const orderPageIds = new Set(data.orders.map((order) => order.id));
        const barrierPageIds = new Set(
          pageBarrierOrders.map((order) => order.id),
        );
        this.historyOrders = mergeById(
          data.orders,
          this.historyOrders.filter((order) => !orderPageIds.has(order.id)),
        );
        this.historyBarrierOrders = mergeById(
          pageBarrierOrders,
          this.historyBarrierOrders.filter(
            (order) => !barrierPageIds.has(order.id),
          ),
        );
      }
      this.activeOrders = Array.isArray(data.active_orders)
        ? data.active_orders
        : activeRestingOrders(data.orders);
      this.activeBarrierOrders = Array.isArray(data.active_barrier_orders)
        ? data.active_barrier_orders
        : activeBarrierOrders(data.barrier_orders ?? []);
      this.patch({
        orders: mergeById(this.historyOrders, this.activeOrders),
        barrierOrders: mergeById(
          this.historyBarrierOrders,
          this.activeBarrierOrders,
        ),
        hasMoreOrdersHistory: startFreshHistory
          ? data.has_more_orders === true
          : this.state$.value.hasMoreOrdersHistory,
        hasMoreBarrierHistory: startFreshHistory
          ? data.has_more_barrier_orders === true
          : this.state$.value.hasMoreBarrierHistory,
        loading: false,
        error: undefined,
      });
      this.syncFeeds();
    } catch (reason) {
      this.patch({
        loading: false,
        error:
          reason instanceof Error
            ? reason.message
            : 'Could not load resting orders',
      });
    } finally {
      this.refreshingOrders = false;
    }
  }
  async loadMoreHistory(): Promise<void> {
    const state = this.state$.value;
    if (
      this.loadingMoreHistory ||
      (!state.hasMoreOrdersHistory && !state.hasMoreBarrierHistory)
    )
      return;
    this.loadingMoreHistory = true;
    this.patch({ loadingMoreHistory: true });
    try {
      const query = new URLSearchParams({
        limit: String(ORDERS_HISTORY_PAGE_SIZE),
      });
      if (state.hasMoreOrdersHistory && this.nextOrdersCursor)
        query.set('orders_cursor', this.nextOrdersCursor);
      if (state.hasMoreBarrierHistory && this.nextBarrierOrdersCursor)
        query.set('barrier_orders_cursor', this.nextBarrierOrdersCursor);
      const response = await sharedGet(
        '/positions/orders/history?' + query.toString(),
      );
      if (!response.ok)
        throw new Error(
          'Could not load older orders (' + response.status + ')',
        );
      const data = (await response.json()) as OrdersHistoryResponse;
      if (!Array.isArray(data.orders) || !Array.isArray(data.barrier_orders))
        throw new Error('The orders history API returned invalid data');
      if (
        (data.has_more_orders && !data.next_orders_cursor) ||
        (data.has_more_barrier_orders && !data.next_barrier_orders_cursor)
      )
        throw new Error('The orders history API returned an invalid cursor');
      this.historyOrders = mergeById(this.historyOrders, data.orders);
      this.historyBarrierOrders = mergeById(
        this.historyBarrierOrders,
        data.barrier_orders,
      );
      if (Array.isArray(data.active_orders))
        this.activeOrders = data.active_orders;
      if (Array.isArray(data.active_barrier_orders))
        this.activeBarrierOrders = data.active_barrier_orders;
      this.nextOrdersCursor = data.next_orders_cursor;
      this.nextBarrierOrdersCursor = data.next_barrier_orders_cursor;
      this.patch({
        orders: mergeById(this.historyOrders, this.activeOrders),
        barrierOrders: mergeById(
          this.historyBarrierOrders,
          this.activeBarrierOrders,
        ),
        hasMoreOrdersHistory: data.has_more_orders === true,
        hasMoreBarrierHistory: data.has_more_barrier_orders === true,
        error: undefined,
      });
      this.syncFeeds();
    } catch (reason) {
      this.patch({
        error:
          reason instanceof Error
            ? reason.message
            : 'Could not load older orders',
      });
    } finally {
      this.loadingMoreHistory = false;
      this.patch({ loadingMoreHistory: false });
    }
  }

  private applyStreamOrders(
    orders: RestingOrder[],
    barrierOrders: BarrierOrder[],
  ): void {
    const previousActiveOrders = this.activeOrders;
    const previousActiveBarrierOrders = this.activeBarrierOrders;
    const orderUpdates = new Map(orders.map((order) => [order.id, order]));
    const barrierUpdates = new Map(
      barrierOrders.map((order) => [order.id, order]),
    );
    this.activeOrders = activeRestingOrders(orders);
    this.activeBarrierOrders = activeBarrierOrders(barrierOrders);
    const activeOrderIds = new Set(this.activeOrders.map((order) => order.id));
    const activeBarrierIds = new Set(
      this.activeBarrierOrders.map((order) => order.id),
    );
    this.historyOrders = mergeById(
      this.historyOrders.map((order) => orderUpdates.get(order.id) ?? order),
      previousActiveOrders.flatMap((order) => {
        const update = orderUpdates.get(order.id);
        return update && !activeOrderIds.has(order.id) ? [update] : [];
      }),
    );
    this.historyBarrierOrders = mergeById(
      this.historyBarrierOrders.map(
        (order) => barrierUpdates.get(order.id) ?? order,
      ),
      previousActiveBarrierOrders.flatMap((order) => {
        const update = barrierUpdates.get(order.id);
        return update && !activeBarrierIds.has(order.id) ? [update] : [];
      }),
    );
    this.patch({
      orders: mergeById(this.historyOrders, this.activeOrders),
      barrierOrders: mergeById(
        this.historyBarrierOrders,
        this.activeBarrierOrders,
      ),
    });
    this.syncFeeds();
  }
  async submit(): Promise<void> {
    const state = this.state$.value;
    const validation = await validateOrderSubmission(state);
    if (!validation) return;
    if ('error' in validation) {
      this.patch({ error: validation.error });
      return;
    }
    if (
      state.orderType === 'limit' &&
      state.limitPriceWarning &&
      state.confirmedLimitPrice !== state.limitPrice
    ) {
      this.patch({
        confirmedLimitPrice: state.limitPrice,
        error: state.limitPriceWarning + ' Submit again to confirm this price.',
      });
      return;
    }
    this.patch({ submitting: true, error: undefined });
    try {
      const isVwap = state.orderType === 'vwap';
      const response = await paperExchangeFetch(
        isVwap ? '/positions/vwap' : '/positions/orders',
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(orderSubmissionBody(state, validation.values)),
        },
      );
      if (!response.ok) {
        const detail = await response.text();
        throw new Error(detail || 'Order rejected (' + response.status + ')');
      }
      const result = (await response.json()) as {
        filled_quantity?: number;
        remaining_quantity?: number;
        id?: string;
        status?: string;
        executed_quantity?: number;
        barrier_order?: BarrierOrder;
      };
      if (result.barrier_order) {
        await this.refreshOrders(false);
        this.patch({ quantity: '', barrierPrice: '', error: undefined });
        return;
      }
      if (isVwap && result.id) {
        this.patch({
          vwapRunId: result.id,
          vwapStatus: result.status ?? 'running',
          vwapExecutedQuantity: result.executed_quantity ?? 0,
          vwapRuns: [
            ...this.state$.value.vwapRuns.filter((run) => run.id !== result.id),
            result as VwapRun,
          ],
          error: undefined,
          quantity: '',
          orderType: 'ioc',
        });
        this.syncVwapFromStream(result.id);
        return;
      }
      await this.refreshOrders(false);
      this.patch({
        error: undefined,
        quantity: '',
        confirmedLimitPrice: undefined,
      });
    } catch (reason) {
      this.patch({
        error: reason instanceof Error ? reason.message : 'Order failed',
      });
    } finally {
      this.patch({ submitting: false });
    }
  }
  private async loadActiveVwap(symbol: string): Promise<void> {
    if (!symbol || this.state$.value.selectedSymbol !== symbol) return;
    const runs = currentPositionSnapshot()?.vwap_runs?.runs as
      | VwapRun[]
      | undefined;
    if (!runs) return;
    this.patch({ vwapRuns: runs });
    const run = [...runs]
      .reverse()
      .find((item) => item.symbol === symbol && item.status === 'running');
    if (run)
      this.patch({
        vwapRunId: run.id,
        vwapStatus: run.status,
        vwapExecutedQuantity: run.executed_quantity,
      });
  }
  private syncVwapFromStream(runId: string): void {
    const run = (
      currentPositionSnapshot()?.vwap_runs?.runs as VwapRun[] | undefined
    )?.find((item) => item.id === runId);
    if (run)
      this.patch({
        vwapStatus: run.status,
        vwapExecutedQuantity: run.executed_quantity,
      });
  }
  async cancelVwap(runId = this.state$.value.vwapRunId): Promise<void> {
    if (!runId) return;
    try {
      const response = await paperExchangeFetch(
        '/positions/vwap/' + encodeURIComponent(runId),
        { method: 'DELETE' },
      );
      if (!response.ok)
        throw new Error(
          (await response.text()) || 'Could not cancel VWAP execution',
        );
      const vwapRuns = this.state$.value.vwapRuns.map((run) =>
        run.id === runId ? { ...run, status: 'cancelled' } : run,
      );
      this.patch({ vwapRuns, vwapStatus: 'cancelled', error: undefined });
    } catch (reason) {
      this.patch({
        error:
          reason instanceof Error
            ? reason.message
            : 'Could not cancel VWAP execution',
      });
    }
  }
  async cancelOrder(id: string): Promise<void> {
    try {
      const response = await paperExchangeFetch(
        '/positions/orders/' + encodeURIComponent(id),
        { method: 'DELETE' },
      );
      if (!response.ok) throw new Error(await response.text());
      this.historyOrders = this.historyOrders.filter(
        (order) => order.id !== id,
      );
      this.historyBarrierOrders = this.historyBarrierOrders.filter(
        (order) => order.id !== id,
      );
      this.activeOrders = this.activeOrders.filter((order) => order.id !== id);
      this.activeBarrierOrders = this.activeBarrierOrders.filter(
        (order) => order.id !== id,
      );
      this.patch({
        orders: this.state$.value.orders.filter((order) => order.id !== id),
        barrierOrders: this.state$.value.barrierOrders.filter(
          (order) => order.id !== id,
        ),
      });
      this.syncFeeds();
    } catch (reason) {
      this.patch({
        error:
          reason instanceof Error ? reason.message : 'Could not cancel order',
      });
    }
  }
  private patch(patch: Partial<OrderManagerState>): void {
    this.state$.next(this.derive({ ...this.state$.value, ...patch }));
  }
  private derive(state: OrderManagerState): OrderManagerState {
    return deriveOrderManagerState(state);
  }
  private syncFeeds(): void {
    const state = this.state$.value;
    const symbols = new Set(
      [
        state.selectedSymbol,
        ...state.orders.map((order) => order.symbol),
        ...state.barrierOrders
          .filter((order) => order.status === 'armed')
          .map((order) => order.symbol),
      ].filter(Boolean),
    );
    this.bookFeeds.sync(symbols);
  }

  private closeFeeds(): void {
    this.bookFeeds.close();
  }
}
