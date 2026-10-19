import { BehaviorSubject } from 'rxjs';
export type AccountState = { cash: number; equity: number; realized_pnl: number; fees_paid: number; equity_available: boolean; initial_cash: number; topups: Array<{ amount: number; timestamp: number }>; max_leverage: number; loading: boolean; submitting: boolean; error?: string };
type AccountResponse = Omit<AccountState, 'loading' | 'submitting' | 'error'>;
export class AccountModel {
  readonly state$ = new BehaviorSubject<AccountState>({ cash: 0, equity: 0, realized_pnl: 0, fees_paid: 0, equity_available: false, initial_cash: 0, topups: [], max_leverage: 2, loading: true, submitting: false });
  private timer?: ReturnType<typeof setInterval>;
  start() { if (this.timer) return; void this.refresh(); this.timer = setInterval(() => void this.refresh(), 5000); }
  stop() { if (this.timer) clearInterval(this.timer); this.timer = undefined; }
  async refresh() {
    try { const response = await fetch('/positions/account'); if (!response.ok) throw new Error('Could not load account (' + response.status + ')'); const data = await response.json() as AccountResponse; this.state$.next({ ...this.state$.value, ...data, loading: false, error: undefined }); }
    catch (reason) { this.state$.next({ ...this.state$.value, loading: false, error: reason instanceof Error ? reason.message : 'Could not load account' }); }
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