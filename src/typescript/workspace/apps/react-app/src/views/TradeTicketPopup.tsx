import { useEffect, useMemo } from 'react';
import { createPortal } from 'react-dom';
import type { Connection } from '../connection';
import { TradeTicketModel } from '../models/TradeTicketModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';

export function TradeTicketPopup({
  symbol,
  initialSide = 'buy',
  connection,
  onClose,
}: {
  symbol: string;
  initialSide?: 'buy' | 'sell';
  connection: Connection;
  onClose: () => void;
}) {
  const model = useMemo(
    () => new TradeTicketModel({ symbol, initialSide, connection }),
    [],
  );
  const state = useBehaviorSnapshot(model.state$);
  useEffect(() => {
    model.updateProps({ symbol, initialSide, connection });
  }, [model, symbol, initialSide, connection]);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  return createPortal((
    <div
      className="trade-popup-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <section
        className="trade-popup"
        role="dialog"
        aria-modal="true"
        aria-label={'Order for ' + state.symbol}
      >
        <header>
          <div>
            <span>ORDER</span>
            <h2>{state.symbol}</h2>
          </div>
          <button
            type="button"
            aria-label="Close trade popup"
            onClick={onClose}
          >
            {'×'}
          </button>
        </header>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            void model.submit();
          }}
        >
          <label>
            SIDE
            <select
              value={state.side}
              onChange={(event) =>
                model.setSide(event.target.value as 'buy' | 'sell')
              }
            >
              <option value="buy">Buy</option>
              <option value="sell">Sell</option>
            </select>
          </label>
          <label>
            ORDER TYPE
            <select
              value={state.orderType}
              onChange={(event) =>
                model.setOrderType(event.target.value as 'ioc' | 'limit' | 'vwap')
              }
            >
              <option value="ioc">Immediate or cancel</option>
              <option value="limit">Limit</option>
              <option value="vwap">VWAP execution</option>
            </select>
          </label>
          <label>
            QUANTITY
            <input
              type="number"
              min="0.00000001"
              step="any"
              required
              value={state.quantity}
              onChange={(event) => model.setQuantity(event.target.value)}
            />
          </label>
          {state.orderType === 'vwap' && (
            <>
              <label>
                DURATION (MINUTES)
                <input type="number" min="5" max="390" step="1" required value={state.durationMinutes} onChange={(event) => model.setDurationMinutes(event.target.value)} />
              </label>
              <label>
                MAX MARKET PARTICIPATION (%)
                <input type="number" min="1" max="100" step="1" required value={state.participationRate} onChange={(event) => model.setParticipationRate(event.target.value)} />
              </label>
            </>
          )}
          {state.orderType === 'limit' && (
            <label>
              <span className="limit-price-label">
                <span>LIMIT PRICE</span>
                <small className="current-instrument-price">
                  MARK{' '}
                  {state.currentPrice === undefined
                    ? state.bookError
                      ? 'Unavailable'
                      : 'Loadingâ€¦'
                    : state.currentPrice.toPrecision(8) +
                      ' ' +
                      (state.symbol.split('-')[1] ?? '')}
                </small>
              </span>
              <input
                type="number"
                min="0.00000001"
                step="any"
                required
                value={state.limitPrice}
                onChange={(event) => model.setLimitPrice(event.target.value)}
              />
              {state.limitPriceWarning && (
                <small className="limit-price-warning" role="alert">
                  {state.limitPriceWarning}
                </small>
              )}
            </label>
          )}
          <div className="trade-preview" aria-live="polite">
            <div className="trade-preview-item">
              <span>BEST BID / ASK</span>
              <strong>
                {state.bestBid !== undefined && state.bestAsk !== undefined
                  ? `${state.bestBid.toPrecision(8)} / ${state.bestAsk.toPrecision(8)}`
                  : state.bookError || 'Waiting for book'}
              </strong>
            </div>
            <div className="trade-preview-item">
              <span>SPREAD</span>
              <strong>
                {state.spread !== undefined
                  ? `${state.spread.toPrecision(6)} (${state.currentPrice ? `${(state.spread / state.currentPrice * 100).toFixed(3)}%` : ''})`
                  : '—'}
              </strong>
            </div>
            <div className="trade-preview-item">
              <span>ESTIMATED NOTIONAL · MID</span>
              <strong>
                {state.estimatedNotional !== undefined
                  ? `${state.estimatedNotional.toPrecision(8)} ${state.symbol.split('-')[1] ?? ''}`
                  : 'Enter quantity'}
              </strong>
            </div>
            <div className="trade-preview-item">
              <span>ESTIMATED FEE ({((state.orderType === "limit" ? state.makerFeeRate : state.takerFeeRate) * 100).toFixed(3)}%)</span>
              <strong>{state.estimatedFee === undefined ? "Unavailable" : state.estimatedFee.toPrecision(8) + " " + (state.symbol.split("-")[1] ?? "")}</strong>
            </div>
            <div className="trade-preview-item trade-preview-book">
              <span>VISIBLE BOOK ESTIMATE</span>
              {state.estimatedFillQuantity !== undefined ? (
                <strong>
                  {state.estimatedFillQuantity.toPrecision(8)} of {state.quantity}
                  {' · '}avg {state.estimatedFillAverage?.toPrecision(8)}
                  {state.estimatedUnfilledQuantity
                    ? <small>{state.estimatedUnfilledQuantity.toPrecision(8)} beyond displayed depth</small>
                    : null}
                </strong>
              ) : (
                <strong>Unavailable</strong>
              )}
            </div>
          </div>

          <button
            className={'paper-trade-submit is-' + state.side}
            type="submit"
            disabled={
              state.busy ||
              !state.quantity ||
              (state.orderType === 'limit' && !state.limitPrice) ||
              (state.orderType === 'vwap' && !!state.vwapRunId && state.vwapStatus === 'running')
            }
          >
            {state.orderType === 'vwap' && state.vwapStatus === 'running'
              ? 'VWAP running'
              : state.busy
                ? 'Submittingâ€¦'
              : state.limitPriceWarning &&
                  state.confirmedLimitPrice === state.limitPrice
                ? 'Confirm price and ' + state.side
                : 'Confirm ' + state.side}
          </button>
          {state.orderType === 'vwap' && state.vwapStatus === 'running' && (
            <button type="button" onClick={() => void model.cancelVwap()}>Cancel VWAP</button>
          )}
        </form>
        {state.message && <p role="status">{state.message}</p>}
      </section>
    </div>
  ), document.body);
}



