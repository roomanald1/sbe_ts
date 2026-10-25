import { BehaviorSubject } from 'rxjs';
export type AccountState = { cash: number; equity: number; realized_pnl: number; unrealized_pnl: number; fees_paid: number; estimated_exit_fees: number; equity_available: boolean; initial_cash: number; topups: Array<{ amount: number; timestamp: number }>; max_leverage: number; auto_trader_total?: number | null; loading: boolean; submitting: boolean; error?: string };
type AccountResponse = Omit<AccountState, 'loading' | 'submitting' | 'error'>;
export class AccountModel {
  readonly state$ = new BehaviorSubject<AccountState>({ cash: 0, equity: 0, realized_pnl: 0, unrealized_pnl: 0, fees_paid: 0, estimated_exit_fees: 0, equity_available: false, initial_cash: 0, topups: [], max_leverage: 2, auto_trader_total: null, loading: true, submitting: false });
  private timer?: ReturnType<typeof setInterval>;
  start() { if (this.timer) return; void this.refresh(); this.timer = setInterval(() => void this.refresh(), 5000); }
  stop() { if (this.timer) clearInterval(this.timer); this.timer = undefined; }
  async refresh() {
    try {
      const [response, activityResponse, ratesResponse] = await Promise.all([
        fetch('/positions/account'),
        fetch('/positions/auto-trader/activity'),
        fetch('/positions/usd-rates'),
      ]);
      if (!response.ok) throw new Error('Could not load account (' + response.status + ')');
      const data = await response.json() as AccountResponse;
      let autoTraderTotal: number | null = null;
      if (activityResponse.ok && ratesResponse.ok) {
        const activity = await activityResponse.json() as { realized_pnl_by_quote?: Record<string, number> | null; unrealized_pnl_by_quote?: Record<string, number> | null };
        const rates = (await ratesResponse.json() as { rates?: Record<string, number> }).rates ?? {};
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