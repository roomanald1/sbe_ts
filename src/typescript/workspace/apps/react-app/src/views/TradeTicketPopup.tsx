import { formatInstrumentValue, getInstrumentPrecision } from '../instrument_precision';
﻿import { useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import type { Connection } from '../connection';
import { TradeTicketModel } from '../models/TradeTicketModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import { orderTypeDescriptions } from '../order_type_descriptions';

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
  const [precision, setPrecision] = useState<import('../instrument_precision').InstrumentPrecision>();
  useEffect(() => { let active = true; void getInstrumentPrecision(symbol).then((value) => { if (active) setPrecision(value); }); return () => { active = false; }; }, [symbol]);
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
              title={orderTypeDescriptions[state.orderType]}
              onChange={(event) =>
                model.setOrderType(event.target.value as 'market' | 'ioc' | 'limit' | 'vwap' | 'twap' | 'barrier' | 'stop-market' | 'stop-limit' | 'trailing-stop' | 'post-only-limit' | 'fok' | 'gtd')
              }
            >
              <option value="market" title={orderTypeDescriptions['market']}>Market</option>
              <option value="ioc" title={orderTypeDescriptions['ioc']}>Immediate or cancel</option>
              <option value="limit" title={orderTypeDescriptions['limit']}>Limit</option>
              <option value="post-only-limit" title={orderTypeDescriptions['post-only-limit']}>Post-only limit</option>
              <option value="fok" title={orderTypeDescriptions['fok']}>Fill or kill</option>
              <option value="gtd" title={orderTypeDescriptions['gtd']}>Good till date (limit)</option>
              <option value="stop-market" title={orderTypeDescriptions['stop-market']}>Stop-market</option>
              <option value="stop-limit" title={orderTypeDescriptions['stop-limit']}>Stop-limit</option>
              <option value="trailing-stop" title={orderTypeDescriptions['trailing-stop']}>Trailing stop</option>
              <option value="barrier" title={orderTypeDescriptions['barrier']}>Barrier order</option>
              <option value="twap" title={orderTypeDescriptions['twap']}>TWAP execution</option>
              <option value="vwap" title={orderTypeDescriptions['vwap']}>VWAP execution</option>
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
          {(state.orderType === 'vwap' || state.orderType === 'twap') && (
            <>
              <label>
                DURATION (MINUTES)
                <input type="number" min="5" max="390" step="1" required value={state.durationMinutes} onChange={(event) => model.setDurationMinutes(event.target.value)} />
              </label>
              {state.orderType === 'vwap' && <label>
                MAX MARKET PARTICIPATION (%)
                <input type="number" min="1" max="100" step="1" required value={state.participationRate} onChange={(event) => model.setParticipationRate(event.target.value)} />
              </label>}
            </>
          )}
          {['limit', 'stop-limit', 'post-only-limit', 'gtd'].includes(state.orderType) && (
            <label>
              <span className="limit-price-label">
                <span>LIMIT PRICE</span>
                <small className="current-instrument-price">
                  MARK{' '}
                  {state.currentPrice === undefined
                    ? state.bookError
                      ? 'Unavailable'
                      : 'Loadingâ€¦'
                    : formatInstrumentValue(state.currentPrice, 'price', precision) +
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
          {state.orderType === 'gtd' && <label>EXPIRES AT<input type="datetime-local" required value={state.expiresAtLocal} onChange={(event) => model.setExpiresAtLocal(event.target.value)} /></label>}
          {(state.orderType === 'barrier' || state.orderType === 'stop-market' || state.orderType === 'stop-limit') && (
            <>
              <label>
                BARRIER PRICE
                <input type="number" min="0.00000001" step="any" required value={state.barrierPrice} onChange={(event) => model.setBarrierPrice(event.target.value)} />
              </label>
              <label>
                TRIGGER WHEN PRICE MOVES
                <select value={state.barrierDirection} onChange={(event) => model.setBarrierDirection(event.target.value as 'above' | 'below')}>
                  <option value="above">Above barrier</option>
                  <option value="below">Below barrier</option>
                </select>
              </label>
            </>
          )}
          {state.orderType === 'trailing-stop' && <>
            <label>TRAILING DISTANCE<select value={state.trailingMode} onChange={(event) => model.setTrailingMode(event.target.value as 'amount' | 'percent')}><option value="percent">Percent</option><option value="amount">Price amount</option></select></label>
            <label>{state.trailingMode === 'percent' ? 'TRAIL (%)' : 'TRAIL PRICE AMOUNT'}<input type="number" min="0.00000001" step="any" required value={state.trailingValue} onChange={(event) => model.setTrailingValue(event.target.value)} /></label>
          </>}
          {['barrier', 'stop-market', 'stop-limit', 'trailing-stop'].includes(state.orderType) && (
            <p className="barrier-order-note">The server watches live prices. Triggered market stops send an IOC; stop-limits activate a resting limit order.</p>
          )}
          {state.orderType === 'market' && <p className="barrier-order-note">Executes immediately against the best available book prices; any amount beyond displayed liquidity may remain unfilled.</p>}
          {state.orderType === 'fok' && <p className="barrier-order-note">The full quantity must fill immediately. If any quantity would remain, the whole order is canceled.</p>}
          {state.orderType === 'post-only-limit' && <p className="barrier-order-note">Adds liquidity only. The server rejects it if it would execute immediately and cancels it if it later becomes marketable.</p>}
          {state.orderType === 'gtd' && <p className="barrier-order-note">The limit order is automatically canceled at the selected local date and time.</p>}
          {state.orderType === 'twap' && <p className="barrier-order-note">Splits the quantity into evenly sized child orders over the selected duration.</p>}
          <div className="trade-preview" aria-live="polite">
            <div className="trade-preview-item">
              <span>BEST BID / ASK</span>
              <strong>
                {state.bestBid !== undefined && state.bestAsk !== undefined
                  ? `${formatInstrumentValue(state.bestBid, 'price', precision)} / ${formatInstrumentValue(state.bestAsk, 'price', precision)}`
                  : state.bookError || 'Waiting for book'}
              </strong>
            </div>
            <div className="trade-preview-item">
              <span>SPREAD</span>
              <strong>
                {state.spread !== undefined
                  ? `${formatInstrumentValue(state.spread, 'price', precision)} (${state.currentPrice ? `${(state.spread / state.currentPrice * 100).toFixed(3)}%` : ''})`
                  : '—'}
              </strong>
            </div>
            <div className="trade-preview-item">
              <span>ESTIMATED NOTIONAL · MID</span>
              <strong>
                {state.estimatedNotional !== undefined
                  ? `${state.estimatedNotional.toLocaleString(undefined, { maximumFractionDigits: 2 })} ${state.symbol.split('-')[1] ?? ''}`
                  : 'Enter quantity'}
              </strong>
            </div>
            <div className="trade-preview-item">
              <span>ESTIMATED FEE ({((state.orderType === "limit" ? state.makerFeeRate : state.takerFeeRate) * 100).toFixed(3)}%)</span>
              <strong>{state.estimatedFee === undefined ? "Unavailable" : state.estimatedFee.toLocaleString(undefined, { maximumFractionDigits: 2 }) + " " + (state.symbol.split("-")[1] ?? "")}</strong>
            </div>
            <div className="trade-preview-item trade-preview-book">
              <span>VISIBLE BOOK ESTIMATE</span>
              {state.estimatedFillQuantity !== undefined ? (
                <strong>
                  {formatInstrumentValue(state.estimatedFillQuantity, 'quantity', precision)} of {state.quantity}
                  {' · '}avg {formatInstrumentValue(state.estimatedFillAverage, 'price', precision)}
                  {state.estimatedUnfilledQuantity
                    ? <small>{formatInstrumentValue(state.estimatedUnfilledQuantity, 'quantity', precision)} beyond displayed depth</small>
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
              (['limit', 'stop-limit', 'post-only-limit', 'gtd'].includes(state.orderType) && !state.limitPrice) ||
              (['barrier', 'stop-market', 'stop-limit'].includes(state.orderType) && !state.barrierPrice) ||
              (['vwap', 'twap'].includes(state.orderType) && !!state.vwapRunId && state.vwapStatus === 'running')
            }
          >
            {['vwap', 'twap'].includes(state.orderType) && state.vwapStatus === 'running'
              ? (state.orderType === 'twap' ? 'TWAP running' : 'VWAP running')
              : state.busy
                ? 'Submittingâ€¦'
              : state.limitPriceWarning &&
                  state.confirmedLimitPrice === state.limitPrice
                ? 'Confirm price and ' + state.side
                : 'Confirm ' + state.side}
          </button>
          {['vwap', 'twap'].includes(state.orderType) && state.vwapStatus === 'running' && (
            <button type="button" onClick={() => void model.cancelVwap()}>Cancel {state.orderType === 'twap' ? 'TWAP' : 'VWAP'}</button>
          )}
        </form>
        {state.message && <p role="status">{state.message}</p>}
      </section>
    </div>
  ), document.body);
}



