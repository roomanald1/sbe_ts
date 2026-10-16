import { useEffect, useMemo, useState } from 'react';
import { AccountModel } from '../models/AccountModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import { FeedToolbar } from './FeedToolbar';
const money = new Intl.NumberFormat(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 });
export function Account() {
  const model = useMemo(() => new AccountModel(), []);
  const state = useBehaviorSnapshot(model.state$);
  const [amount, setAmount] = useState('');
  useEffect(() => { model.start(); return () => model.stop(); }, [model]);
  return <section className="feed-section" aria-label="Paper account"><FeedToolbar title="Account" />
    {state.error && <div className="positions-error" role="alert">{state.error}</div>}
    <div className="positions-totals">
      <div className="position-total-card"><span>Equity</span><strong>{state.equity_available ? money.format(state.equity) + ' USD' : 'Unavailable'}</strong></div>
      <div className="position-total-card"><span>Cash</span><strong>{state.equity_available ? money.format(state.cash) + ' USD' : 'Unavailable'}</strong></div>
      <div className="position-total-card"><span>Fees paid</span><strong>{state.equity_available ? money.format(state.fees_paid) + ' USD' : 'Unavailable'}</strong></div>
    </div>
    <form className="paper-trade-form account-topup-form" onSubmit={(event) => { event.preventDefault(); const value = Number(amount); if (Number.isFinite(value) && value > 0) void model.topUp(value); }}>
      <label>
        <span>TOP UP USD</span>
        <input type="number" min="0.01" step="0.01" value={amount} onChange={(event) => setAmount(event.target.value)} placeholder="Amount" />
      </label>
      <button className="paper-trade-submit" type="submit" disabled={state.submitting || !amount}>
        {state.submitting ? 'Adding...' : 'Top up'}
      </button>
    </form>
    {state.loading && <p className="positions-empty">Loading account…</p>}
  </section>;
}


