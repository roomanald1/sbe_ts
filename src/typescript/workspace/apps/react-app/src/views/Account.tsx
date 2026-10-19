import { useEffect, useMemo, useState } from 'react';
import { AccountModel } from '../models/AccountModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import { FeedToolbar } from './FeedToolbar';
const money = new Intl.NumberFormat(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 });
export function Account() {
  const model = useMemo(() => new AccountModel(), []);
  const state = useBehaviorSnapshot(model.state$);
  const [amount, setAmount] = useState('');
  const [maxLeverage, setMaxLeverage] = useState('2');
  useEffect(() => { if (!state.loading) setMaxLeverage(String(state.max_leverage)); }, [state.loading, state.max_leverage]);
  useEffect(() => { model.start(); return () => model.stop(); }, [model]);
  return <section className="feed-section" aria-label="Paper account"><FeedToolbar title="Account" />
    {state.error && <div className="positions-error" role="alert">{state.error}</div>}
    <div className="positions-totals">
      <div className="position-total-card"><span>Realized balance</span><strong>{money.format(state.initial_cash + state.topups.reduce((sum, topup) => sum + topup.amount, 0) + state.realized_pnl) + ' USD'}</strong></div>
      <div className="position-total-card"><span>Realized P&amp;L</span><strong>{money.format(state.realized_pnl) + ' USD'}</strong></div>
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
    <form className="paper-trade-form account-topup-form" onSubmit={(event) => { event.preventDefault(); const value = Number(maxLeverage); if (Number.isFinite(value) && value >= 1 && value <= 10) void model.setMaxLeverage(value); }}>
      <label>
        <span>MAX LEVERAGE</span>
        <select value={maxLeverage} onChange={(event) => setMaxLeverage(event.target.value)}>
          {Array.from({ length: 10 }, (_, index) => index + 1).map((value) => <option key={value} value={value}>{value}x</option>)}
        </select>
      </label>
      <button className="paper-trade-submit" type="submit" disabled={state.submitting || state.loading || Number(maxLeverage) === state.max_leverage}>
        {state.submitting ? 'Saving...' : 'Save leverage'}
      </button>
    </form>
    {state.loading && <p className="positions-empty">Loading account�</p>}
  </section>;
}


