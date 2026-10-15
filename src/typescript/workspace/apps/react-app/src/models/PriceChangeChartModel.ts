import { BehaviorSubject } from 'rxjs';
import {
  createPriceChartData,
  type PriceChanges,
  type PriceChartData,
} from '../price_change_data';
type Props = {
  changes: PriceChanges;
  initialSymbol?: string;
  selectedSymbol?: string;
  onSymbolChange?: (symbol: string) => void;
};
export type PriceChangeChartState = {
  data: PriceChartData;
  chartSize: { width: number; height: number };
};
export class PriceChangeChartModel {
  readonly props$: BehaviorSubject<Props>;
  readonly state$: BehaviorSubject<PriceChangeChartState>;
  private localSymbol: string;
  constructor(props: Props) {
    this.props$ = new BehaviorSubject(props);
    this.localSymbol = props.initialSymbol ?? '';
    this.state$ = new BehaviorSubject({
      data: this.createData(props),
      chartSize: { width: 900, height: 340 },
    });
  }
  getSnapshot = (): PriceChangeChartState => this.state$.value;
  subscribe = (notify: () => void): (() => void) => {
    const sub = this.state$.subscribe(notify);
    return () => sub.unsubscribe();
  };
  updateProps(props: Props): void {
    this.props$.next(props);
    if (!props.selectedSymbol && !this.localSymbol)
      this.localSymbol = props.initialSymbol ?? '';
    this.publish();
  }
  selectSymbol(symbol: string): void {
    const props = this.props$.value;
    if (props.onSymbolChange) props.onSymbolChange(symbol);
    else this.localSymbol = symbol;
    this.publish();
  }
  setChartSize(width: number, height: number): void {
    if (width <= 0 || height <= 0) return;
    const current = this.state$.value.chartSize;
    if (current.width === width && current.height === height) return;
    this.state$.next({ ...this.state$.value, chartSize: { width, height } });
  }
  private createData(props: Props): PriceChartData {
    return createPriceChartData(
      props.changes.ticks,
      props.changes.histories,
      props.selectedSymbol ?? this.localSymbol,
    );
  }
  private publish(): void {
    this.state$.next({
      ...this.state$.value,
      data: this.createData(this.props$.value),
    });
  }
}
