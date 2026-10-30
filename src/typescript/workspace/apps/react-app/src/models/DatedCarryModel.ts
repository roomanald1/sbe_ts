import { BehaviorSubject } from 'rxjs';
import { currentAutoTraderSnapshot, subscribeAutoTraderStream } from '../auto_trader_stream';

export type DatedCarryCandidate = {
  spot_symbol: string; future_symbol: string; expiry_ms: number; days_to_expiry: number;
  quantity: number; spot_ask: number; future_bid: number; basis_percent: number;
  net_after_budget_percent: number; eligible: boolean; reason: string;
};
export type DatedCarryPosition = {
  spot_symbol: string; future_symbol: string; expiry_ms: number; entry_ms: number;
  quantity: number; spot_entry_price: number; future_entry_price: number;
  entry_fees: number; status: string; spot_exit_twap?: number | null;
  settlement_price?: number | null; settlement_source?: string | null; net_pnl_usdt?: number | null;
};
export type DatedCarryBook = {
  enabled: boolean; max_spot_notional_usdt: number; candidates: DatedCarryCandidate[];
  positions: DatedCarryPosition[]; checked_at_ms: number; last_error?: string | null;
};
export type DatedCarryViewState = {
  loading: boolean; saving: boolean; book?: DatedCarryBook; notional: string; error?: string;
};

export class DatedCarryModel {
  readonly state$ = new BehaviorSubject<DatedCarryViewState>({
    loading: true, saving: false, notional: '1000',
  });
  private timer?: ReturnType<typeof setInterval>;
  private stopped = false;
  private stopStream?: () => void;

  start() {
    this.stopped = false;
    const update = (book: DatedCarryBook) => {
      const current = this.state$.value;
      this.state$.next({ ...current, loading: false, book, error: undefined, notional: current.loading ? String(book.max_spot_notional_usdt) : current.notional });
    };
    const snapshot = currentAutoTraderSnapshot();
    if (snapshot?.dated_carry) update(snapshot.dated_carry as DatedCarryBook);
    this.stopStream = subscribeAutoTraderStream((next) => { if (next.dated_carry) update(next.dated_carry as DatedCarryBook); });
    if (!snapshot) void this.refresh();
  }
  stop() {
    this.stopped = true;
    if (this.timer) clearInterval(this.timer);
    this.timer = undefined;
    this.stopStream?.();
    this.stopStream = undefined;
  }
  setNotional(value: string) {
    this.state$.next({ ...this.state$.value, notional: value });
  }
  async refresh() {
    try {
      const streamed = currentAutoTraderSnapshot()?.dated_carry as DatedCarryBook | undefined;
      if (streamed) {
        if (!this.stopped) this.state$.next({ ...this.state$.value, loading: false, book: streamed, error: undefined });
        return;
      }
      const response = await fetch('/positions/auto-trader/dated-carry');
      if (!response.ok) throw new Error(await response.text());
      const book = await response.json() as DatedCarryBook;
      if (this.stopped) return;
      const current = this.state$.value;
      this.state$.next({
        ...current, loading: false, book, error: undefined,
        notional: current.loading ? String(book.max_spot_notional_usdt) : current.notional,
      });
    } catch (error) {
      if (!this.stopped) this.state$.next({
        ...this.state$.value, loading: false, error: error instanceof Error ? error.message : String(error),
      });
    }
  }
  async saveSize() {
    const current = this.state$.value;
    const notional = Number(current.notional);
    if (!Number.isFinite(notional) || notional < 100 || notional > 10_000) {
      this.state$.next({ ...current, error: 'Paper spot notional must be between 100 and 10,000 USDT.' });
      return;
    }
    this.state$.next({ ...current, saving: true, error: undefined });
    try {
      const response = await fetch('/positions/auto-trader/dated-carry', {
        method: 'PUT', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ max_spot_notional_usdt: notional }),
      });
      if (!response.ok) throw new Error(await response.text());
      const book = await response.json() as DatedCarryBook;
      if (!this.stopped) this.state$.next({ ...this.state$.value, saving: false, book, error: undefined });
    } catch (error) {
      if (!this.stopped) this.state$.next({
        ...this.state$.value, saving: false, error: error instanceof Error ? error.message : String(error),
      });
    }
  }
}
