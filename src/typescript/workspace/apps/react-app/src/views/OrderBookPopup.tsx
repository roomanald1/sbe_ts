import { formatForSymbol } from '../instrument_precision';
import { useEffect, useId, useMemo, useState } from 'react';
import { FeedToolbar } from './FeedToolbar';
import { Connection } from '../connection';
import { OrderBookModel } from '../models/OrderBookModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import type { BookLevel } from '../types/symbol_price';

const money = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', maximumFractionDigits: 0 });

type OrderBookPopupProps = {
  symbol: string;
  connection: Connection;
  onClose?: () => void;
  symbols?: string[];
  onSymbolChange?: (symbol: string) => void;
  mode?: 'popup' | 'view';
  showSymbolSelector?: boolean;
  streamKey?: string;
  onTrade?: (symbol: string, side: 'buy' | 'sell') => void;
};




function DepthRows({
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

function DepthBalance({
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

function SweepSummary({ bids, asks, mid, symbol }: { bids: BookLevel[]; asks: BookLevel[]; mid: number; symbol: string }) {
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

export function OrderBookPopup({
  symbol,
  connection,
  onClose,
  symbols = [],
  onSymbolChange,
  mode = 'popup',
  showSymbolSelector = true,
  streamKey,
  onTrade,
}: OrderBookPopupProps) {
  const [depthMode, setDepthMode] = useState<'7' | '50'>('7');
  const generatedStreamKey = useId();
  const bookKey = streamKey ?? generatedStreamKey;
  const model = useMemo(
    () => new OrderBookModel({ connection, symbol, streamKey: bookKey }),
    [],
  );
  const state = useBehaviorSnapshot(model.state$);
  useEffect(() => {
    model.updateProps({ connection, symbol, streamKey: bookKey });
  }, [model, connection, symbol, bookKey]);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && mode === 'popup') onClose?.();
    };
    window.addEventListener('keydown', closeOnEscape);
    return () => window.removeEventListener('keydown', closeOnEscape);
  }, [mode, onClose]);
  const frozen = state.frozen;
  const error = state.error;
  const selectedDepth = depthMode === '50' ? state.deepDepth : state.depth;
  const visibleDepth = selectedDepth?.symbol === symbol ? selectedDepth : undefined;
  const maximumBidDepth = depthMode === '50' ? (visibleDepth?.bids.at(-1)?.cumulativeNotional ?? 0) : (visibleDepth?.bids.at(-1)?.cumulativeSize ?? 0);
  const maximumAskDepth = depthMode === '50' ? (visibleDepth?.asks.at(-1)?.cumulativeNotional ?? 0) : (visibleDepth?.asks.at(-1)?.cumulativeSize ?? 0);
  const spread =
    visibleDepth?.bids[0] && visibleDepth.asks[0]
      ? visibleDepth.asks[0].price - visibleDepth.bids[0].price
      : undefined;
  const mid =
    visibleDepth?.bids[0] && visibleDepth.asks[0]
      ? (visibleDepth.asks[0].price + visibleDepth.bids[0].price) / 2
      : undefined;

  return (
    <div
      className={
        mode === 'popup' ? 'book-popup-backdrop' : 'feed-section book-view-wrap'
      }
      role={mode === 'view' ? 'region' : undefined}
      aria-label={mode === 'view' ? 'Order book' : undefined}
      onMouseDown={(event) => {
        if (mode === 'popup' && event.target === event.currentTarget)
          onClose?.();
      }}
    >
      {mode === 'view' && (
        <FeedToolbar title="Order book">
          {showSymbolSelector && (
            <label className="chart-symbol-control">
              <span>SYMBOL</span>
              <select
                aria-label="Book symbol"
                value={symbol}
                onChange={(event) => onSymbolChange?.(event.target.value)}
              >
                {symbols.map((item) => (
                  <option key={item} value={item}>
                    {item}
                  </option>
                ))}
              </select>
            </label>
          )}
        </FeedToolbar>
      )}
      <section
        className={mode === 'view' ? 'book-popup book-view' : 'book-popup'}
        role={mode === 'popup' ? 'dialog' : 'region'}
        aria-modal={mode === 'popup' ? true : undefined}
        aria-labelledby="book-popup-title"
      >
        <header className="book-popup-header">
          <div>
            <span className="book-popup-eyebrow">OKX · BOOK DEPTH</span>
            <h2 id="book-popup-title">{symbol}</h2>
          </div>
          <div className="book-popup-header-actions">
            {onTrade && mode === 'view' && (
              <>
                <button
                  className="book-trade-button is-buy"
                  type="button"
                  onClick={() => onTrade(symbol, 'buy')}
                >
                  Buy
                </button>
                <button
                  className="book-trade-button is-sell"
                  type="button"
                  onClick={() => onTrade(symbol, 'sell')}
                >
                  Sell
                </button>
              </>
            )}
            <div className="book-depth-mode" role="group" aria-label="Displayed order book depth">
              <button type="button" aria-pressed={depthMode === '7'} onClick={() => setDepthMode('7')}>7 levels</button>
              <button type="button" aria-pressed={depthMode === '50'} onClick={() => setDepthMode('50')}>50 levels</button>
            </div>
            <span className={`book-stream-status${frozen ? ' is-frozen' : ''}`}>
              {frozen ? 'FROZEN' : 'LIVE'}
            </span>
            <button
              className="book-freeze-button"
              type="button"
              aria-pressed={frozen}
              onClick={() => model.toggleFrozen()}
            >
              {frozen ? 'Resume' : 'Freeze'}
            </button>
            {mode === 'popup' && (
              <button
                className="book-popup-close"
                type="button"
                onClick={onClose}
                aria-label="Close order book"
              >
                ×
              </button>
            )}
          </div>
        </header>
        <div className="book-popup-columns" aria-hidden="true">
          <span>{depthMode === '50' ? 'CUMULATIVE QUOTE DEPTH' : 'BID SIZE'}</span>
          <span>PRICE</span>
          <span>{depthMode === '50' ? 'CUMULATIVE QUOTE DEPTH' : 'ASK SIZE'}</span>
        </div>
        {error ? (
          <div className="book-popup-message book-popup-error">{error}</div>
        ) : !visibleDepth ? (
          <div className="book-popup-message">
            <span className="book-loading-dot" />
            Waiting for the first book snapshot…
          </div>
        ) : (
          <>
            <DepthRows
              levels={visibleDepth.asks}
              side="ask"
              maximumSideDepth={maximumAskDepth || 1}
              symbol={symbol}
              quoteDepth={depthMode === '50'}
            />
            <div className="book-spread">
              <div className="book-market-metrics">
                <div className="book-market-metric">
                  <span>MID</span>
                  <strong>
                    {mid === undefined ? '—' : formatForSymbol(mid, symbol, 'price')}
                  </strong>
                </div>
                <div className="book-market-metric">
                  <span>SPREAD</span>
                  <strong>
                    {spread === undefined ? '—' : formatForSymbol(spread, symbol, 'price')}
                  </strong>
                </div>
              </div>
              <time dateTime={new Date(visibleDepth.timestamp).toISOString()}>
                {new Date(visibleDepth.timestamp).toLocaleTimeString()}
              </time>
            </div>
            {mid !== undefined && depthMode === '50' && (symbol.endsWith('-USDT') || symbol.endsWith('-USDC')) && <SweepSummary bids={visibleDepth.bids} asks={visibleDepth.asks} mid={mid} symbol={symbol} />}
            <DepthRows
              levels={visibleDepth.bids}
              side="bid"
              maximumSideDepth={maximumBidDepth || 1}
              symbol={symbol}
              quoteDepth={depthMode === '50'}
            />
            <DepthBalance bids={visibleDepth.bids} asks={visibleDepth.asks} label={depthMode} />
          </>
        )}
        <footer className="book-popup-footer">
          {visibleDepth
            ? 'Snapshot ' + new Date(visibleDepth.timestamp).toLocaleTimeString() + (depthMode === '50' ? ' · 50 levels from OKX REST' : ' · 7 live levels') + (frozen ? ' · paused' : '')
            : (depthMode === '50' ? 'Waiting for the 50-level OKX snapshot…' : 'Live updates begin after selection')}
        </footer>
      </section>
    </div>
  );
}
