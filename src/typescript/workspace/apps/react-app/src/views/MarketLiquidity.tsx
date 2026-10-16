import { memo, useEffect, useId, useMemo, useState } from 'react';
import { usePausedValue } from '../use_paused_value';
import { AgGridReact } from 'ag-grid-react';
import {
  ClientSideRowModelModule,
  ModuleRegistry,
  themeQuartz,
  type ColDef,
} from 'ag-grid-community';
import { FeedToolbar } from './FeedToolbar';
import type { Connection } from '../connection';
import { MarketLiquidityModel } from '../models/MarketLiquidityModel';
import type { LiquidityBand } from '../market_liquidity';
import { estimateSlippage, findLiquidityGaps } from '../market_liquidity';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';

type Props = {
  symbol: string;
  connection: Connection;
  symbols?: string[];
  onSymbolChange?: (symbol: string) => void;
  streamKey?: string;
  paused?: boolean;
};
type LiquidityGridRow = LiquidityBand & {
  bidQuote: number;
  askQuote: number;
  trend: string;
};

ModuleRegistry.registerModules([ClientSideRowModelModule]);

const number = new Intl.NumberFormat(undefined, { maximumFractionDigits: 6 });
const quote = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 });
const liquidityGridTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});
const defaultColDef: ColDef<LiquidityGridRow> = {
  sortable: true,
  resizable: true,
  flex: 1,
  minWidth: 85,
};

export function MarketLiquidityComponent({
  symbol,
  connection,
  symbols = [],
  onSymbolChange,
  streamKey,
  paused = false,
}: Props) {
  const id = useId();
  const key = streamKey ?? id;
  const model = useMemo(
    () => new MarketLiquidityModel({ connection, symbol, streamKey: key }),
    [],
  );
  const state = useBehaviorSnapshot(model.state$);
  const [side, setSide] = useState<'buy' | 'sell'>('buy');
  const [quantity, setQuantity] = useState('1');

  useEffect(() => {
    model.updateProps({ connection, symbol, streamKey: key });
  }, [model, connection, symbol, key]);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);

  const gaps = state.book
    ? findLiquidityGaps(state.book.bids, state.book.asks)
    : [];
  const slippage = state.book
    ? estimateSlippage(state.book, side, Number(quantity))
    : undefined;
  const rowData = useMemo<LiquidityGridRow[]>(
    () => state.bands.map((band, index) => ({
      ...band,
      bidQuote: band.bidNotional,
      askQuote: band.askNotional,
      trend: state.trends[index] ?? 'warming',
    })),
    [state.bands, state.trends],
  );
  const displayedRowData = usePausedValue(rowData, paused);
  const columnDefs = useMemo<ColDef<LiquidityGridRow>[]>(
    () => [
      {
        field: 'percent',
        headerName: 'Band',
        valueFormatter: ({ value }) => `${value}%`,
        minWidth: 70,
        maxWidth: 90,
      },
      {
        field: 'bidSize',
        headerName: 'Bid qty',
        valueFormatter: ({ value }) => number.format(value ?? 0),
      },
      {
        field: 'bidQuote',
        headerName: 'Bid quote',
        valueFormatter: ({ value }) => `${quote.format(value ?? 0)} quote`,
      },
      {
        field: 'askSize',
        headerName: 'Ask qty',
        valueFormatter: ({ value }) => number.format(value ?? 0),
      },
      {
        field: 'askQuote',
        headerName: 'Ask quote',
        valueFormatter: ({ value }) => `${quote.format(value ?? 0)} quote`,
      },
      {
        field: 'imbalance',
        headerName: 'Imbalance',
        cellClass: ({ value }) => value >= 0 ? 'liquidity-bid' : 'liquidity-ask',
        valueFormatter: ({ value }) => `${((value ?? 0) * 100).toFixed(1)}%`,
      },
      { field: 'trend', headerName: 'Trend (1m)' },
    ],
    [],
  );

  return (
    <section className="feed-section liquidity-view" aria-label="Market liquidity">
      <FeedToolbar title="Market liquidity">
        <label className="chart-symbol-control">
          <span>SYMBOL</span>
          <select
            aria-label="Liquidity symbol"
            value={symbol}
            onChange={(event) => onSymbolChange?.(event.target.value)}
          >
            {symbols.map((item) => <option key={item} value={item}>{item}</option>)}
          </select>
        </label>
      </FeedToolbar>
      {state.error && <div className="positions-error" role="alert">{state.error}</div>}
      {!state.book ? (
        <div className="book-popup-message">Waiting for live book data…</div>
      ) : (
        <>
          <div className="liquidity-mid">
            MID
            <strong>{number.format((state.book.bids[0].price + state.book.asks[0].price) / 2)}</strong>
            <span>Top {state.book.bids.length} levels per side</span>
          </div>
          <div className="data-grid-wrap pnl-grid-wrap liquidity-grid-wrap">
            <AgGridReact<LiquidityGridRow>
              rowData={displayedRowData}
              columnDefs={columnDefs}
              defaultColDef={defaultColDef}
              getRowId={({ data }) => String(data.percent)}
              theme={liquidityGridTheme}
              domLayout="autoHeight"
              rowHeight={34}
              headerHeight={38}
              suppressColumnMoveAnimation
              suppressHorizontalScroll={false}
              animateRows={false}
            />
          </div>
          <div className="liquidity-slippage">
            <strong>Proposed order slippage</strong>
            <label>
              Side
              <select value={side} onChange={(event) => setSide(event.target.value as 'buy' | 'sell')}>
                <option value="buy">Buy</option>
                <option value="sell">Sell</option>
              </select>
            </label>
            <label>
              Quantity
              <input type="number" min="0" step="any" value={quantity} onChange={(event) => setQuantity(event.target.value)} />
            </label>
            <span>
              Average {slippage?.averagePrice === undefined ? '—' : number.format(slippage.averagePrice)}
              {' · '}Slippage {slippage?.slippageBps?.toFixed(2) ?? '—'} bps
            </span>
            {(slippage?.unfilledQuantity ?? 0) > 0 && (
              <small>{number.format(slippage!.unfilledQuantity)} beyond displayed depth.</small>
            )}
            <small>Estimate walks the displayed book; deeper orders may have additional slippage.</small>
          </div>
          <div className="liquidity-lower">
            <div>
              <h3>Liquidity gaps ≥ 0.1%</h3>
              {gaps.length ? gaps.slice(0, 6).map((gap, index) => (
                <p key={index} className={gap.side === 'bid' ? 'liquidity-bid' : 'liquidity-ask'}>
                  {gap.side.toUpperCase()} {number.format(gap.from)}–{number.format(gap.to)} · {gap.gapPercent.toFixed(3)}%
                </p>
              )) : <p>No wide level gaps detected.</p>}
            </div>
            <div>
              <h3>Large or unusual trades</h3>
              {state.unusualTrades.length ? state.unusualTrades.slice(0, 6).map((trade) => (
                <p key={trade.tradeId} className={trade.side === 'buy' ? 'liquidity-bid' : 'liquidity-ask'}>
                  {trade.side.toUpperCase()} {number.format(trade.size)} · {quote.format(trade.price * trade.size)} quote
                </p>
              )) : <p>Waiting for trades ≥ 10,000 quote or 5× recent median.</p>}
            </div>
          </div>
        </>
      )}
    </section>
  );
}

export const MarketLiquidity = memo(MarketLiquidityComponent);
