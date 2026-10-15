import { useEffect, useId, useMemo } from 'react';
import { FeedToolbar } from './FeedToolbar';
import { Connection } from '../connection';
import { OrderBookModel } from '../models/OrderBookModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import type { BookLevel } from '../types/symbol_price';

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

const priceFormatter = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 2,
  maximumFractionDigits: 10,
});

const midFormatter = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
});

const sizeFormatter = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 4,
  maximumFractionDigits: 4,
});

function DepthRows({
  levels,
  side,
  maximumSideDepth,
}: {
  levels: BookLevel[];
  side: 'bid' | 'ask';
  maximumSideDepth: number;
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
              width: `${(level.cumulativeSize / maximumSideDepth) * 50}%`,
              ...(side === 'bid' ? { right: '50%' } : { left: '50%' }),
            }}
          />
          {side === 'bid' ? (
            <span className="book-level-quantity book-level-quantity-bid">
              <strong>{sizeFormatter.format(level.size)}</strong>
              <small>Σ {sizeFormatter.format(level.cumulativeSize)}</small>
            </span>
          ) : (
            <span className="book-level-empty" aria-hidden="true" />
          )}
          <span className={`book-level-price book-level-price-${side}`}>
            {priceFormatter.format(level.price)}
          </span>
          {side === 'ask' ? (
            <span className="book-level-quantity book-level-quantity-ask">
              <strong>{sizeFormatter.format(level.size)}</strong>
              <small>Σ {sizeFormatter.format(level.cumulativeSize)}</small>
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
}: {
  bids: BookLevel[];
  asks: BookLevel[];
}) {
  const bidTotal = bids.at(-1)?.cumulativeSize ?? 0;
  const askTotal = asks.at(-1)?.cumulativeSize ?? 0;
  const total = bidTotal + askTotal;
  if (total <= 0) return null;

  const bidShare = (bidTotal / total) * 100;
  const askShare = 100 - bidShare;

  return (
    <div
      className="book-balance"
      aria-label={`Top-seven depth: bids ${Math.round(bidShare)}%, asks ${Math.round(askShare)}%`}
    >
      <div className="book-balance-labels">
        <span>TOP 7 DEPTH</span>
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
  const visibleDepth = state.depth?.symbol === symbol ? state.depth : undefined;
  const maximumBidDepth = visibleDepth?.bids.at(-1)?.cumulativeSize ?? 0;
  const maximumAskDepth = visibleDepth?.asks.at(-1)?.cumulativeSize ?? 0;
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
          <span>BID SIZE</span>
          <span>PRICE</span>
          <span>ASK SIZE</span>
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
            />
            <div className="book-spread">
              <div className="book-market-metrics">
                <div className="book-market-metric">
                  <span>MID</span>
                  <strong>
                    {mid === undefined ? '—' : midFormatter.format(mid)}
                  </strong>
                </div>
                <div className="book-market-metric">
                  <span>SPREAD</span>
                  <strong>
                    {spread === undefined ? '—' : priceFormatter.format(spread)}
                  </strong>
                </div>
              </div>
              <time dateTime={new Date(visibleDepth.timestamp).toISOString()}>
                {new Date(visibleDepth.timestamp).toLocaleTimeString()}
              </time>
            </div>
            <DepthRows
              levels={visibleDepth.bids}
              side="bid"
              maximumSideDepth={maximumBidDepth || 1}
            />
            <DepthBalance bids={visibleDepth.bids} asks={visibleDepth.asks} />
          </>
        )}
        <footer className="book-popup-footer">
          {visibleDepth
            ? `Snapshot ${new Date(visibleDepth.timestamp).toLocaleTimeString()}${frozen ? ' · paused' : ''}`
            : 'Live updates begin after selection'}
        </footer>
      </section>
    </div>
  );
}
