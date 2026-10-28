import { formatForSymbol } from '../instrument_precision';
import { createPortal } from 'react-dom';
import { useEffect, useId, useMemo, useState } from 'react';
import { FeedToolbar } from './FeedToolbar';
import { Connection } from '../connection';
import { OrderBookModel } from '../models/OrderBookModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';

import { DepthBalance, DepthRows, SweepSummary } from './OrderBookDepth';

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

  const popup = (
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
  return mode === 'popup' ? createPortal(popup, document.body) : popup;
}
