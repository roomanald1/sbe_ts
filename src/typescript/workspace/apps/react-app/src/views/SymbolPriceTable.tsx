import { memo, useEffect, useMemo } from 'react';
import {
  SymbolPriceTableModel,
  type SymbolPriceTableProps,
} from '../models/SymbolPriceTableModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import { FeedToolbar } from './FeedToolbar';
import type { SymbolPriceTick } from '../types/symbol_price';

const priceFormatter = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 2,
  maximumFractionDigits: 10,
});

const timeFormatter = new Intl.DateTimeFormat(undefined, {
  hour: 'numeric',
  minute: '2-digit',
  second: '2-digit',
});

const SymbolPriceRow = memo(function SymbolPriceRow({
  tick,
  model,
}: {
  tick: SymbolPriceTick;
  model: SymbolPriceTableModel;
}) {
  return (
    <tr>
      <td>
        <button
          className="symbol-tag symbol-book-button"
          type="button"
          onClick={() => model.selectSymbol(tick.symbol)}
          disabled={!model.state$.value.onSelectSymbol}
          aria-label={`Show ${tick.symbol} order book`}
        >
          {tick.symbol}
        </button>
      </td>
      <td className={`price-cell price-${tick.direction}`}>
        {priceFormatter.format(tick.price)}
      </td>
      <td className="time-cell">{timeFormatter.format(tick.receivedAt)}</td>
    </tr>
  );
});

export function SymbolPriceTable(props: SymbolPriceTableProps) {
  const model = useMemo(() => new SymbolPriceTableModel(props), []);
  useEffect(() => {
    model.setProps(props);
  }, [model, props.ticks, props.state, props.onSelectSymbol, props.maxRows]);
  const view = useBehaviorSnapshot(model.state$);
  const { visibleTicks, maxRows } = view;
  return (
    <section
      className={
        maxRows === undefined
          ? 'feed-section'
          : 'feed-section compact-market-panel'
      }
      aria-label="Live symbol price events"
    >
      <FeedToolbar title="Incoming events">
        <span className="toolbar-meta">LATEST TICKS</span>
      </FeedToolbar>
      <div className="table-scroll">
        <table className="tick-table">
          <colgroup>
            <col className="tick-symbol-column" />
            <col className="tick-price-column" />
            <col className="tick-time-column" />
          </colgroup>
          <thead>
            <tr>
              <th scope="col">SYMBOL</th>
              <th scope="col">PRICE</th>
              <th scope="col">RECEIVED AT</th>
            </tr>
          </thead>
          <tbody>
            {visibleTicks.length === 0 ? (
              <tr>
                <td className="empty-state" colSpan={3}>
                  {view.emptyMessage}
                </td>
              </tr>
            ) : (
              visibleTicks.map((tick) => (
                <SymbolPriceRow key={tick.symbol} tick={tick} model={model} />
              ))
            )}
          </tbody>
        </table>
      </div>
    </section>
  );
}
