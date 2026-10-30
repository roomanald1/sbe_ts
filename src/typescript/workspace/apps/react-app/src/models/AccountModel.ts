import { BehaviorSubject } from 'rxjs';
import { currentPositionSnapshot, subscribePositionStream } from '../position_stream';
import { currentAutoTraderSnapshot, subscribeAutoTraderStream } from '../auto_trader_stream';
export type AccountState = { cash: number; equity: number; realized_pnl: number; unrealized_pnl: number; fees_paid: number; estimated_exit_fees: number; equity_available: boolean; initial_cash: number; topups: Array<{ amount: number; timestamp: number }>; max_leverage: number; auto_trader_total?: number | null; loading: boolean; submitting: boolean; error?: string };
type AccountResponse = Omit<AccountState, 'loading' | 'submitting' | 'error'>;
export class AccountModel {
  readonly state$ = new BehaviorSubject<AccountState>({ cash: 0, equity: 0, realized_pnl: 0, unrealized_pnl: 0, fees_paid: 0, estimated_exit_fees: 0, equity_available: false, initial_cash: 0, topups: [], max_leverage: 2, auto_trader_total: null, loading: true, submitting: false });
  private timer?: ReturnType<typeof setInterval>;
  private stopPositionStream?: () => void;
  private stopAutoTraderStream?: () => void;
  private usdRates: Record<string, number> = {};
  start() {
    if (this.timer) return;
    const current = currentPositionSnapshot();
    this.usdRates = current?.usd_rates?.rates ?? {};
    this.stopPositionStream = subscribePositionStream((snapshot) => {
      this.usdRates = snapshot.usd_rates?.rates ?? this.usdRates;
      if (snapshot.account) this.state$.next({ ...this.state$.value, ...snapshot.account as AccountResponse, loading: false });
    });
    const updateAutoTotal = (activity: Record<string, unknown>) => {
      const totals = activity as { realized_pnl_by_quote?: Record<string, number> | null; unrealized_pnl_by_quote?: Record<string, number> | null };
      const convert = (values?: Record<string, number> | null) => {
        if (!values) return null;
        let total = 0;
        for (const [quote, amount] of Object.entries(values)) {
          const rate = quote === 'USD' ? 1 : this.usdRates[quote];
          if (typeof rate !== 'number' || !Number.isFinite(rate)) return null;
          total += amount * rate;
        }
        return total;
      };
      const realized = convert(totals.realized_pnl_by_quote);
      const unrealized = convert(totals.unrealized_pnl_by_quote);
      this.state$.next({ ...this.state$.value, auto_trader_total: realized !== null && unrealized !== null ? realized + unrealized : null });
    };
    const autoSnapshot = currentAutoTraderSnapshot();
    if (autoSnapshot) updateAutoTotal(autoSnapshot.activity);
    this.stopAutoTraderStream = subscribeAutoTraderStream((snapshot) => updateAutoTotal(snapshot.activity));
    void this.refresh();
    this.timer = setInterval(() => void this.refresh(), 30_000);
  }
  stop() { if (this.timer) clearInterval(this.timer); this.timer = undefined; this.stopPositionStream?.(); this.stopPositionStream = undefined; this.stopAutoTraderStream?.(); this.stopAutoTraderStream = undefined; }
  async refresh() {
    try {
      const snapshot = currentPositionSnapshot();
      const autoSnapshot = currentAutoTraderSnapshot();
      const [response, activityResponse] = await Promise.all([
        snapshot?.account ? Promise.resolve(undefined) : fetch('/positions/account'),
        autoSnapshot ? Promise.resolve(undefined) : fetch('/positions/auto-trader/activity'),
      ]);
      if (!snapshot?.account && !response?.ok) throw new Error('Could not load account (' + response?.status + ')');
      const data = snapshot?.account as AccountResponse | undefined ?? await response!.json() as AccountResponse;
      let autoTraderTotal: number | null = null;
      if (autoSnapshot || activityResponse?.ok) {
        const activity = autoSnapshot?.activity as { realized_pnl_by_quote?: Record<string, number> | null; unrealized_pnl_by_quote?: Record<string, number> | null } ?? await activityResponse!.json() as { realized_pnl_by_quote?: Record<string, number> | null; unrealized_pnl_by_quote?: Record<string, number> | null };
        const rates = this.usdRates;
        const convert = (totals?: Record<string, number> | null) => {
          if (!totals) return null;
          let value = 0;
          for (const [quote, amount] of Object.entries(totals)) {
            const rate = quote === 'USD' ? 1 : rates[quote];
            if (typeof rate !== 'number' || !Number.isFinite(rate)) return null;
            value += amount * rate;
          }
          return value;
        };
        const realized = convert(activity.realized_pnl_by_quote);
        const unrealized = convert(activity.unrealized_pnl_by_quote);
        if (realized !== null && unrealized !== null) autoTraderTotal = realized + unrealized;
      }
      this.state$.next({ ...this.state$.value, ...data, auto_trader_total: autoTraderTotal, loading: false, error: undefined });
    } catch (reason) { this.state$.next({ ...this.state$.value, loading: false, error: reason instanceof Error ? reason.message : 'Could not load account' }); }
  }
  async topUp(amount: number) {
    this.state$.next({ ...this.state$.value, submitting: true, error: undefined });
    try { const response = await fetch('/positions/account/topups', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ amount }) }); if (!response.ok) { const message = await response.text(); throw new Error(message || 'Top-up failed (' + response.status + ')'); } await this.refresh(); }
    catch (reason) { this.state$.next({ ...this.state$.value, error: reason instanceof Error ? reason.message : 'Top-up failed' }); }
    finally { this.state$.next({ ...this.state$.value, submitting: false }); }
  }
  async setMaxLeverage(max_leverage: number) {
    this.state$.next({ ...this.state$.value, submitting: true, error: undefined });
    try { const response = await fetch('/positions/account/leverage', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ max_leverage }) }); if (!response.ok) { const message = await response.text(); throw new Error(message || 'Could not update maximum leverage (' + response.status + ')'); } const data = await response.json() as AccountResponse; this.state$.next({ ...this.state$.value, ...data, loading: false, error: undefined }); }
    catch (reason) { this.state$.next({ ...this.state$.value, error: reason instanceof Error ? reason.message : 'Could not update maximum leverage' }); }
    finally { this.state$.next({ ...this.state$.value, submitting: false }); }
  }
}
