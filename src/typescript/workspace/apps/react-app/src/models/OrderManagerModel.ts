import { BehaviorSubject } from 'rxjs';
import type { Connection } from '../connection';
import { deriveOrderManagerState } from './OrderManagerDerived';
import { OrderManagerBookFeeds } from './OrderManagerBookFeeds';
import type {
  BarrierDirection, BarrierOrder, OrderManagerProps, OrderManagerState,
  OrderType, OrdersResponse, TradeSide, VwapRun,
} from './OrderManagerTypes';
import { orderSubmissionBody, validateOrderSubmission } from './OrderManagerSubmission';
import { currentPositionSnapshot, subscribePositionStream } from '../position_stream';

export class OrderManagerModel {
  readonly props$: BehaviorSubject<OrderManagerProps>;
  readonly state$: BehaviorSubject<OrderManagerState>;
  private bookFeeds!: OrderManagerBookFeeds;
  private timer?: ReturnType<typeof setInterval>;
  private stopPositionStream?: () => void;
  private lastPositionStreamAt = 0;
  private refreshingOrders = false;
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
      const vwapRuns = (snapshot.vwap_runs?.runs ?? []) as VwapRun[];
      const selectedRun = vwapRuns.find((run) => run.id === this.state$.value.vwapRunId)
        ?? [...vwapRuns].reverse().find((run) => run.symbol === this.state$.value.selectedSymbol && run.status === 'running');
      this.patch({
        orders: snapshot.orders.orders, barrierOrders: snapshot.orders.barrier_orders ?? [],
        vwapRuns, ...(snapshot.fee_rates ? { makerFeeRate: snapshot.fee_rates.maker, takerFeeRate: snapshot.fee_rates.taker } : {}),
        ...(selectedRun ? { vwapRunId: selectedRun.id, vwapStatus: selectedRun.status, vwapExecutedQuantity: selectedRun.executed_quantity } : {}),
        loading: false, error: undefined,
      });
      this.syncFeeds();
    });
    void this.loadFeeRates();
    if (!currentPositionSnapshot()) void this.refreshOrders();
    void this.loadActiveVwap(this.state$.value.selectedSymbol);
    this.timer = setInterval(() => {
      if (Date.now() - this.lastPositionStreamAt > 45_000) void this.refreshOrders();
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
    this.patch({ limitPrice, confirmedLimitPrice: undefined, error: undefined });
  }
  setBarrierPrice(barrierPrice: string): void {
    this.patch({ barrierPrice, error: undefined });
  }
  setBarrierDirection(barrierDirection: BarrierDirection): void {
    this.patch({ barrierDirection });
  }
  private async loadFeeRates(): Promise<void> {
    const rates = currentPositionSnapshot()?.fee_rates;
    if (rates && Number.isFinite(rates.maker) && Number.isFinite(rates.taker)) this.patch({ makerFeeRate: rates.maker, takerFeeRate: rates.taker });
  }
  async refreshOrders(): Promise<void> {
    if (this.refreshingOrders) return;
    this.refreshingOrders = true;
    try {
      const response = await fetch('/positions/orders');
      if (!response.ok)
        throw new Error(
          'Could not load resting orders (' + response.status + ')',
        );
      const data = (await response.json()) as OrdersResponse;
      if (!Array.isArray(data.orders))
        throw new Error('The orders API returned invalid data');
      this.patch({ orders: data.orders, barrierOrders: data.barrier_orders ?? [], loading: false, error: undefined });
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
  async submit(): Promise<void> {
    const state = this.state$.value;
    const validation = await validateOrderSubmission(state);
    if (validation && 'error' in validation) {
      this.patch({ error: validation.error });
      return;
    }
    if (!validation) return;
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
      const response = await fetch(isVwap ? '/positions/vwap' : '/positions/orders', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(orderSubmissionBody(state, validation.values)),
      });
      if (!response.ok) {
        const detail = await response.text();
        throw new Error(detail || 'Order rejected (' + response.status + ')');
      }
      const result = (await response.json()) as {
        filled_quantity?: number; remaining_quantity?: number; id?: string; status?: string; executed_quantity?: number;
        barrier_order?: BarrierOrder;
      };
      if (result.barrier_order) {
        await this.refreshOrders();
        this.patch({ quantity: '', barrierPrice: '', error: undefined });
        return;
      }
      if (isVwap && result.id) {
        this.patch({ vwapRunId: result.id, vwapStatus: result.status ?? 'running', vwapExecutedQuantity: result.executed_quantity ?? 0, vwapRuns: [...this.state$.value.vwapRuns.filter((run) => run.id !== result.id), result as VwapRun], error: undefined, quantity: '', orderType: 'ioc' });
        this.syncVwapFromStream(result.id);
        return;
      }
      await this.refreshOrders();
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
    const runs = currentPositionSnapshot()?.vwap_runs?.runs as VwapRun[] | undefined;
    if (!runs) return;
    this.patch({ vwapRuns: runs });
    const run = [...runs].reverse().find((item) => item.symbol === symbol && item.status === 'running');
    if (run) this.patch({ vwapRunId: run.id, vwapStatus: run.status, vwapExecutedQuantity: run.executed_quantity });
  }
  private syncVwapFromStream(runId: string): void {
    const run = (currentPositionSnapshot()?.vwap_runs?.runs as VwapRun[] | undefined)?.find((item) => item.id === runId);
    if (run) this.patch({ vwapStatus: run.status, vwapExecutedQuantity: run.executed_quantity });
  }
  async cancelVwap(runId = this.state$.value.vwapRunId): Promise<void> {
    if (!runId) return;
    try {
      const response = await fetch('/positions/vwap/' + encodeURIComponent(runId), { method: 'DELETE' });
      if (!response.ok) throw new Error((await response.text()) || 'Could not cancel VWAP execution');
      const vwapRuns = this.state$.value.vwapRuns.map((run) => run.id === runId ? { ...run, status: 'cancelled' } : run);
      this.patch({ vwapRuns, vwapStatus: 'cancelled', error: undefined });
    } catch (reason) {
      this.patch({ error: reason instanceof Error ? reason.message : 'Could not cancel VWAP execution' });
    }
  }
  async cancelOrder(id: string): Promise<void> {
    try {
      const response = await fetch(
        '/positions/orders/' + encodeURIComponent(id),
        { method: 'DELETE' },
      );
      if (!response.ok) throw new Error(await response.text());
      this.patch({
        orders: this.state$.value.orders.filter((order) => order.id !== id),
        barrierOrders: this.state$.value.barrierOrders.filter((order) => order.id !== id),
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
    const symbols = new Set([
      state.selectedSymbol,
      ...state.orders.map((order) => order.symbol),
      ...state.barrierOrders.filter((order) => order.status === 'armed').map((order) => order.symbol),
    ].filter(Boolean));
    this.bookFeeds.sync(symbols);
  }

  private closeFeeds(): void {
    this.bookFeeds.close();
  }

}
