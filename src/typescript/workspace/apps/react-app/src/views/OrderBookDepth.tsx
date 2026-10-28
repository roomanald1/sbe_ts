import { formatForSymbol } from '../instrument_precision';
import type { BookLevel } from '../types/symbol_price';

const money = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', maximumFractionDigits: 0 });

export function DepthRows({
  levels,
  side,
  maximumSideDepth,
  symbol,
  quoteDepth = false,
}: {
  levels: BookLevel[];
  side: 'bid' | 'ask';
  maximumSideDepth: number;
  symbol: string;
  quoteDepth?: boolean;
}) {
  if (levels.length === 0) {
    return <div className="book-empty">Waiting for {side} depth…</div>;
  }

  const orderedLevels = side === 'ask' ? [...levels].reverse() : levels;
  return (
    <div className={`book-levels book-levels-${side}`}>
      {orderedLevels.map((level) => (
        <div
          className={`book-level book-level-${side}`}
          key={`${level.price}-${level.size}`}
        >
          <span
            className={`book-depth-bar book-depth-bar-${side}`}
            style={{
              width: `${((quoteDepth ? (level.cumulativeNotional ?? level.cumulativeSize * level.price) : level.cumulativeSize) / maximumSideDepth) * 50}%`,
              ...(side === 'bid' ? { right: '50%' } : { left: '50%' }),
            }}
          />
          {side === 'bid' ? (
            <span className="book-level-quantity book-level-quantity-bid">
              <strong>{formatForSymbol(level.size, symbol, 'quantity')}</strong>
              <small>Σ {quoteDepth ? money.format(level.cumulativeNotional ?? level.cumulativeSize * level.price) : formatForSymbol(level.cumulativeSize, symbol, 'quantity')}</small>
            </span>
          ) : (
            <span className="book-level-empty" aria-hidden="true" />
          )}
          <span className={`book-level-price book-level-price-${side}`}>
            {formatForSymbol(level.price, symbol, 'price')}
          </span>
          {side === 'ask' ? (
            <span className="book-level-quantity book-level-quantity-ask">
              <strong>{formatForSymbol(level.size, symbol, 'quantity')}</strong>
              <small>Σ {quoteDepth ? money.format(level.cumulativeNotional ?? level.cumulativeSize * level.price) : formatForSymbol(level.cumulativeSize, symbol, 'quantity')}</small>
            </span>
          ) : (
            <span className="book-level-empty" aria-hidden="true" />
          )}
        </div>
      ))}
    </div>
  );
}

export function DepthBalance({
  bids,
  asks,
  label,
}: {
  bids: BookLevel[];
  asks: BookLevel[];
  label: string;
}) {
  const bidTotal = label === '50' ? (bids.at(-1)?.cumulativeNotional ?? 0) : (bids.at(-1)?.cumulativeSize ?? 0);
  const askTotal = label === '50' ? (asks.at(-1)?.cumulativeNotional ?? 0) : (asks.at(-1)?.cumulativeSize ?? 0);
  const total = bidTotal + askTotal;
  if (total <= 0) return null;

  const bidShare = (bidTotal / total) * 100;
  const askShare = 100 - bidShare;

  return (
    <div
      className="book-balance"
      aria-label={'Top-' + label + ' depth: bids ' + Math.round(bidShare) + '%, asks ' + Math.round(askShare) + '%'}
    >
      <div className="book-balance-labels">
        <span>TOP {label} DEPTH</span>
        <span>
          <strong className="book-balance-bid">
            {Math.round(bidShare)}% BID
          </strong>
          {' / '}
          <strong className="book-balance-ask">
            {Math.round(askShare)}% ASK
          </strong>
        </span>
      </div>
      <div className="book-balance-track" aria-hidden="true">
        <span
          className="book-balance-bid-fill"
          style={{ width: `${bidShare}%` }}
        />
        <span
          className="book-balance-ask-fill"
          style={{ width: `${askShare}%` }}
        />
      </div>
    </div>
  );
}

export function SweepSummary({ bids, asks, mid, symbol }: { bids: BookLevel[]; asks: BookLevel[]; mid: number; symbol: string }) {
  const estimate = (levels: BookLevel[], target: number) => {
    let remaining = target, quantity = 0, notional = 0;
    for (const level of levels) {
      const available = level.price * level.size;
      const taken = Math.min(remaining, available);
      quantity += taken / level.price;
      notional += taken;
      remaining -= taken;
      if (remaining <= 0) break;
    }
    if (quantity <= 0) return '—';
    const average = notional / quantity;
    const impactBps = Math.abs(average / mid - 1) * 10000;
    return remaining > 0 ? 'partial · ' + Math.round(notional).toLocaleString() + ' USD' : formatForSymbol(average, symbol, 'price') + ' · ' + impactBps.toFixed(2) + ' bps';
  };
  return <div className="book-sweep-summary">
    <strong>ESTIMATED MARKET SWEEP</strong>
    <span className="book-sweep-note">Static snapshot · before fees · displayed book only</span>
    {[250, 500, 1000].map((amount) => <div className="book-sweep-row" key={amount}>
      <span>USD {amount}</span><span>Buy: {estimate(asks, amount)}</span><span>Sell: {estimate(bids, amount)}</span>
    </div>)}
  </div>;
}

