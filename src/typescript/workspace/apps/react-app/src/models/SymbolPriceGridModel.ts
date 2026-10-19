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

function createRows(ticks: SymbolPriceTick[]): SymbolGridRow[] {
  const rows = Array.from({ length: 10 }, (_, rowIndex) => ({
    rowIndex,
    cells: Array<SymbolGridCell>(10).fill(undefined),
  }));
  ticks.slice(0, 100).forEach((tick, index) => {
    rows[Math.floor(index / 10)].cells[index % 10] = tick;
  });
  return rows;
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
    this.state$.next({ ...props, rows: createRows(props.ticks) });
  }
  selectSymbol(symbol: string, side: 'buy' | 'sell'): void {
    this.props$.value.onSelectSymbol?.(symbol, side);
  }
  tradeSymbol(symbol: string, side: 'buy' | 'sell'): void {
    this.props$.value.onTradeSymbol?.(symbol, side);
  }
}
