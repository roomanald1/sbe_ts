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
      <div className="position-total-card"><span>Net liquidation equity</span><strong>{state.equity_available ? money.format(state.equity - state.estimated_exit_fees) + ' USD' : 'Unavailable'}</strong><small>Marked equity less estimated close fees</small></div>
      <div className="position-total-card"><span>Current cash balance</span><strong>{money.format(state.cash) + ' USD'}</strong><small>Buy/sell cash flows after fees already paid</small></div>
      <div className="position-total-card"><span>Initial contributed capital</span><strong>{money.format(state.initial_cash + state.topups.reduce((sum, topup) => sum + topup.amount, 0)) + ' USD'}</strong><small>Starting cash plus top-ups; unchanged by trading</small></div>
      <div className="position-total-card"><span>AutoTrader P&amp;L today + open</span><strong>{state.auto_trader_total == null ? 'Unavailable' : money.format(state.auto_trader_total) + ' USD'}</strong><small>Same scope as the AutoTrader summary</small></div>
      <div className="position-total-card"><span>Net liquidation P&amp;L</span><strong>{state.equity_available ? money.format(state.realized_pnl + state.unrealized_pnl) + ' USD' : 'Unavailable'}</strong><small>Realized plus open P&amp;L after estimated close fees</small></div>
      <div className="position-total-card"><span>All account realized P&amp;L</span><strong>{money.format(state.realized_pnl) + ' USD'}</strong><small>Closed positions, after allocated fees</small></div>
      <div className="position-total-card"><span>All account unrealized P&amp;L</span><strong>{state.equity_available ? money.format(state.unrealized_pnl) + ' USD' : 'Unavailable'}</strong><small>Open positions, estimated close fee included</small></div>
      <div className="position-total-card"><span>Estimated close fees</span><strong>{state.equity_available ? money.format(state.estimated_exit_fees) + ' USD' : 'Unavailable'}</strong><small>Deducted from open P&amp;L, not yet paid</small></div>
      <div className="position-total-card"><span>Fees paid</span><strong>{money.format(Math.abs(state.fees_paid)) + ' USD'}</strong><small>Already reflected in realized and open P&amp;L</small></div>
    </div>
    <p className="positions-empty">
      Cash includes proceeds from open short positions. Those proceeds do not count as realized profit:
      open positions are marked against cash when equity is calculated, and P&amp;L is realized when they close.
    </p>
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


