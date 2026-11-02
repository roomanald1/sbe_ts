import { BehaviorSubject } from 'rxjs';

export type ExecutionFeeRow = {
  source: string;
  liquidity: string;
  fee_currency: string;
  fills: number;
  traded_notional: number;
  fees_paid: number;
  effective_fee_bps?: number | null;
  first_fill_ms: number;
  last_fill_ms: number;
};
export type ExecutionFeesState = {
  fills: number;
  rows: ExecutionFeeRow[];
  first_fill_ms?: number | null;
  last_fill_ms?: number | null;
  loading: boolean;
  updatedAt?: number;
  error?: string;
};
type FeeSummaryResponse = Pick<ExecutionFeesState, 'fills' | 'rows' | 'first_fill_ms' | 'last_fill_ms'>;

export class ExecutionFeesModel {
  readonly state$ = new BehaviorSubject<ExecutionFeesState>({ fills: 0, rows: [], loading: true });
  private timer?: ReturnType<typeof setInterval>;
  private inFlight = false;

  start(): void {
    if (this.timer) return;
    void this.refresh();
    this.timer = setInterval(() => void this.refresh(), 30_000);
  }

  stop(): void {
    if (this.timer) clearInterval(this.timer);
    this.timer = undefined;
  }

  async refresh(): Promise<void> {
    if (this.inFlight) return;
    this.inFlight = true;
    try {
      const response = await fetch('/positions/fee-summary');
      if (!response.ok) throw new Error('Could not load ledger fee summary (' + response.status + ')');
      const summary = await response.json() as FeeSummaryResponse;
      if (!Array.isArray(summary.rows) || typeof summary.fills !== 'number') {
        throw new Error('The ledger fee summary returned invalid data');
      }
      this.state$.next({ ...summary, loading: false, updatedAt: Date.now() });
    } catch (reason) {
      this.state$.next({
        ...this.state$.value,
        loading: false,
        error: reason instanceof Error ? reason.message : 'Could not load ledger fee summary',
      });
    } finally {
      this.inFlight = false;
    }
  }
}
