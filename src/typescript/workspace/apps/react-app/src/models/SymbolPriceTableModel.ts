import { BehaviorSubject } from 'rxjs';
import type { ConnectionState } from '../connection';
import type { SymbolPriceTick } from '../types/symbol_price';
import type { PriceHistorySample } from '../price_change_data';

export type SymbolPriceTableProps = {
  ticks: SymbolPriceTick[];
  histories: Map<string, PriceHistorySample[]>;
  state: ConnectionState;
  maxRows?: number;
  onSelectSymbol?: (symbol: string, side: 'buy' | 'sell') => void;
};
export type SymbolPriceTableState = SymbolPriceTableProps & {
  visibleTicks: SymbolPriceTick[];
  emptyMessage: string;
};
function toState(props: SymbolPriceTableProps): SymbolPriceTableState {
  return {
    ...props,
    visibleTicks:
      props.maxRows === undefined
        ? props.ticks
        : props.ticks.slice(0, props.maxRows),
    emptyMessage:
      props.state === 'connected'
        ? 'Waiting for the first SBE message…'
        : 'Connecting to the publisher…',
  };
}
export class SymbolPriceTableModel {
  readonly props$: BehaviorSubject<SymbolPriceTableProps>;
  readonly state$: BehaviorSubject<SymbolPriceTableState>;
  constructor(props: SymbolPriceTableProps) {
    this.props$ = new BehaviorSubject(props);
    this.state$ = new BehaviorSubject(toState(props));
  }
  readonly getSnapshot = (): SymbolPriceTableState => this.state$.value;
  readonly subscribe = (notify: () => void): (() => void) => {
    const sub = this.state$.subscribe(notify);
    return () => sub.unsubscribe();
  };
  setProps(props: SymbolPriceTableProps): void {
    this.props$.next(props);
    this.state$.next(toState(props));
  }
  selectSymbol(symbol: string, side: 'buy' | 'sell'): void {
    this.props$.value.onSelectSymbol?.(symbol, side);
  }
}
