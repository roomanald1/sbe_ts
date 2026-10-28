import { BehaviorSubject } from 'rxjs';
import type { SymbolPriceTick } from '../types/symbol_price';

export type SymbolGridCell = SymbolPriceTick | undefined;
export type SymbolGridRow = { rowIndex: number; cells: SymbolGridCell[] };
export type SymbolPriceGridProps = {
  ticks: SymbolPriceTick[];
  feedSource: 'okx' | 'synthetic';
  compact: boolean;
  onSelectSymbol?: (symbol: string, side: 'buy' | 'sell') => void;
  onTradeSymbol?: (symbol: string, side: 'buy' | 'sell') => void;
};
export type SymbolPriceGridState = SymbolPriceGridProps & {
  rows: SymbolGridRow[];
};

function createRows(ticks: SymbolPriceTick[], previousRows: SymbolGridRow[] = []): SymbolGridRow[] {
  const nextRows = Array.from({ length: 10 }, (_, rowIndex) => {
    const prior = previousRows[rowIndex];
    const cells = Array.from({ length: 10 }, (_, columnIndex) =>
      ticks[rowIndex * 10 + columnIndex],
    );
    const unchanged = prior?.cells.every((cell, index) => {
      const next = cells[index];
      return cell?.symbol === next?.symbol
        && cell?.price === next?.price
        && cell?.volatility1m === next?.volatility1m
        && cell?.volatility5m === next?.volatility5m
        && cell?.direction === next?.direction;
    });
    return unchanged ? prior : { rowIndex, cells };
  });
  return nextRows;
}

export class SymbolPriceGridModel {
  readonly props$: BehaviorSubject<SymbolPriceGridProps>;
  readonly state$: BehaviorSubject<SymbolPriceGridState>;
  constructor(props: SymbolPriceGridProps) {
    this.props$ = new BehaviorSubject(props);
    this.state$ = new BehaviorSubject({
      ...props,
      rows: createRows(props.ticks),
    });
  }
  readonly getSnapshot = (): SymbolPriceGridState => this.state$.value;
  readonly subscribe = (notify: () => void): (() => void) => {
    const sub = this.state$.subscribe(notify);
    return () => sub.unsubscribe();
  };
  setProps(props: SymbolPriceGridProps): void {
    this.props$.next(props);
    this.state$.next({ ...props, rows: createRows(props.ticks, this.state$.value.rows) });
  }
  selectSymbol(symbol: string, side: 'buy' | 'sell'): void {
    this.props$.value.onSelectSymbol?.(symbol, side);
  }
  tradeSymbol(symbol: string, side: 'buy' | 'sell'): void {
    this.props$.value.onTradeSymbol?.(symbol, side);
  }
}
