import { BehaviorSubject } from 'rxjs';
import type { PositionTrade } from './PositionTypes';

type FillsResponse = { trades: PositionTrade[] };
export type FillsState = {
  fills: PositionTrade[];
  loading: boolean;
  error?: string;
};
export class FillsModel {
  readonly state$ = new BehaviorSubject<FillsState>({
    fills: [],
    loading: true,
  });
  private timer?: ReturnType<typeof setInterval>;
  getSnapshot = (): FillsState => this.state$.value;
  subscribe = (notify: () => void): (() => void) => {
    const sub = this.state$.subscribe(notify);
    return () => sub.unsubscribe();
  };
  start(): void {
    if (this.timer) return;
    void this.refresh();
    this.timer = setInterval(() => void this.refresh(), 5000);
  }
  stop(): void {
    if (this.timer) clearInterval(this.timer);
    this.timer = undefined;
  }
  async refresh(): Promise<void> {
    try {
      const response = await fetch('/positions/trades');
      if (!response.ok)
        throw new Error('Could not load fills (' + response.status + ')');
      const payload = (await response.json()) as FillsResponse;
      if (!Array.isArray(payload.trades))
        throw new Error('The fills API returned invalid data');
      this.state$.next({
        fills: [...payload.trades].reverse(),
        loading: false,
      });
    } catch (reason) {
      this.state$.next({
        ...this.state$.value,
        loading: false,
        error:
          reason instanceof Error ? reason.message : 'Could not load fills',
      });
    }
  }
}
