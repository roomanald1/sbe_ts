import { BehaviorSubject } from 'rxjs';

export type AlternativeConfig = {
  enabled: boolean; max_trade_usd: number; daily_loss_limit_usd: number;
  spot_fee_bps: number; short_borrow_apr: number; min_edge_bps: number;
};
export type AlternativePosition = { kind: string } & Record<string, unknown>;
export type AlternativeEvent = {
  timestamp_ms: number; observation_id: string; action: string; detail: string;
  cash_usd: number; equity_usd: number;
};
export type AlternativeSleeve = {
  feed?: { ready: boolean; signal: string; source_timestamp_ms: number; computed_at_ms: number;
    summary: string; features: Record<string, unknown> };
  id: string; config: AlternativeConfig; initial_capital_usd: number; cash_usd: number;
  equity_usd: number; max_drawdown_percent: number; realized_pnl_usd: number; fees_usd: number;
  last_observation_ms: number; status: string; positions: Record<string, AlternativePosition>;
  events: AlternativeEvent[];
};
type Book = { sleeves: Record<string, AlternativeSleeve> };
type Snapshot = { mode: string; auto_trader_enabled: boolean; load_error?: string | null; book: Book };
type ViewState = {
  loading: boolean; saving: boolean; snapshot?: Snapshot;
  drafts: Record<string, AlternativeConfig>; error?: string;
};
export const alternativeLabels: Record<string, string> = {
  'residual-momentum': 'Market-adjusted momentum',
  'filtered-covered-call': 'Filtered BTC covered calls',
  'staking-redemption-arbitrage': 'Staking mint / redemption arbitrage',
  'liquidation-keeper': 'DeFi liquidation keeper',
  'adaptive-stablecoin-liquidity': 'Adaptive stablecoin liquidity',
};
export const alternativeDataNeeds: Record<string, string> = {
  'residual-momentum': 'Insights supplies closed daily candles and depth quotes; short borrowing is simulated.',
  'filtered-covered-call': 'Deribit BTC/USDC option books, lot sizes, DVOL regime and official settlements via FeedHandlers → Insights.',
  'staking-redemption-arbitrage': 'Lido mint → Curve sale, simulated at a pinned Ethereum block; Insights evaluates net gas-adjusted edge.',
  'liquidation-keeper': 'Aave V3 borrower watchlist; successful liquidation and collateral-sale simulation required.',
  'adaptive-stablecoin-liquidity': 'Uniswap V3 USDC/USDT state; Insights computes adaptive ticks, inside-range fees and gas costs.',
};

export class AlternativeStrategiesModel {
  readonly state$ = new BehaviorSubject<ViewState>({ loading: true, saving: false, drafts: {} });
  private timer?: ReturnType<typeof setInterval>;
  private stopped = true;
  private refreshing = false;
  private revision = 0;
  private requests = new Set<AbortController>();

  start() {
    if (!this.stopped) return;
    this.stopped = false;
    void this.refresh();
    this.timer = setInterval(() => void this.refresh(), 5000);
  }
  stop() {
    this.stopped = true;
    this.revision += 1;
    if (this.timer) clearInterval(this.timer);
    this.timer = undefined;
    for (const request of this.requests) request.abort();
    this.requests.clear();
  }
  edit(id: string, patch: Partial<AlternativeConfig>) {
    const state = this.state$.value;
    const existing = state.drafts[id] ?? state.snapshot?.book.sleeves[id]?.config;
    if (existing) this.state$.next({ ...state, drafts: { ...state.drafts, [id]: { ...existing, ...patch } } });
  }
  private async request(path: string, init?: RequestInit) {
    const controller = new AbortController();
    this.requests.add(controller);
    try {
      const response = await fetch(path, { ...init, signal: controller.signal });
      if (!response.ok) throw new Error(await response.text());
      return await response.json();
    } finally { this.requests.delete(controller); }
  }
  async refresh() {
    if (this.stopped || this.refreshing || this.state$.value.saving) return;
    this.refreshing = true;
    const revision = this.revision;
    try {
      const snapshot = await this.request('/positions/auto-trader/alternatives') as Snapshot;
      if (!this.stopped && !this.state$.value.saving && revision === this.revision) {
        this.state$.next({ ...this.state$.value, loading: false, snapshot, error: undefined });
      }
    } catch (error) {
      if (!this.stopped && revision === this.revision) this.state$.next({
        ...this.state$.value, loading: false, error: error instanceof Error ? error.message : String(error),
      });
    } finally { this.refreshing = false; }
  }
  async save(id: string) {
    const state = this.state$.value;
    const config = state.drafts[id] ?? state.snapshot?.book.sleeves[id]?.config;
    if (!config || state.saving || this.stopped) return;
    if (!Object.values(config).every((value) => typeof value === 'boolean' || Number.isFinite(value))
      || config.max_trade_usd <= 0 || config.max_trade_usd > 1000
      || config.daily_loss_limit_usd <= 0 || config.daily_loss_limit_usd > 1000
      || config.spot_fee_bps < 0 || config.spot_fee_bps > 100
      || config.short_borrow_apr < 0 || config.short_borrow_apr > 5
      || config.min_edge_bps < 0 || config.min_edge_bps > 1000) {
      this.state$.next({ ...state, error: 'Check the paper size, loss limit, fees and edge buffer.' });
      return;
    }
    this.revision += 1;
    this.state$.next({ ...state, saving: true, error: undefined });
    try {
      const book = await this.request('/positions/auto-trader/alternatives/' + encodeURIComponent(id), {
        method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(config),
      }) as Book;
      if (this.stopped) return;
      const next = this.state$.value;
      const drafts = { ...next.drafts };
      delete drafts[id];
      this.state$.next({ ...next, saving: false, drafts,
        snapshot: next.snapshot ? { ...next.snapshot, book } : undefined });
    } catch (error) {
      if (!this.stopped) this.state$.next({ ...this.state$.value, saving: false,
        error: error instanceof Error ? error.message : String(error) });
    }
  }
  feedReady(id: string) {
    const feed = this.state$.value.snapshot?.book.sleeves[id]?.feed;
    const now = Date.now();
    return !!feed?.ready && feed.computed_at_ms <= now && now - feed.computed_at_ms <= 120_000
      && feed.source_timestamp_ms <= now && now - feed.source_timestamp_ms <= 120_000;
  }
  rows() {
    return Object.values(this.state$.value.snapshot?.book.sleeves ?? {}).map((s) => ({
      ...s, name: alternativeLabels[s.id] ?? s.id,
      open_positions: Object.keys(s.positions).length,
      data_status: this.feedReady(s.id) ? 'Ready · ' + s.feed?.signal : 'Unavailable / stale',
      insight: s.feed?.summary ?? 'Awaiting native Insights',
    }));
  }
  events() {
    return Object.values(this.state$.value.snapshot?.book.sleeves ?? {})
      .flatMap((s) => s.events.map((event, index) => ({ ...event, strategy: alternativeLabels[s.id] ?? s.id,
        key: s.id + ':' + event.observation_id + ':' + index })))
      .sort((a, b) => b.timestamp_ms - a.timestamp_ms).slice(0, 200);
  }
  positions() {
    return Object.values(this.state$.value.snapshot?.book.sleeves ?? {}).flatMap((s) =>
      Object.entries(s.positions).map(([id, p]) => ({ id: s.id + ':' + id, strategy: alternativeLabels[s.id] ?? s.id,
        instrument: id, kind: p.kind, quantity: p.quantity ?? p.liquidity ?? null,
        detail: JSON.stringify(p) })));
  }
}
