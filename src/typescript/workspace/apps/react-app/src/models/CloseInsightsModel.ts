import { BehaviorSubject } from 'rxjs';
import type { Position } from './PositionTypes';

type PositionsResponse = { positions: Position[] };
type BackendInsight = {
  symbol: string;
  close_side: 'buy' | 'sell';
  close_quantity: number;
  close_price?: number;
  close_pnl?: number;
  depth_covers_position: boolean;
  profitable_close_probability_5m?: number;
  probability_samples: number;
};
type InsightsResponse = { insights: BackendInsight[] };
export type CloseInsight = {
  position: Position;
  closeSide: 'buy' | 'sell';
  closeQuantity: number;
  closePrice?: number;
  closePnl?: number;
  depthCoversPosition: boolean;
  profitableCloseProbability5m?: number;
  probabilitySamples: number;
};
export type CloseInsightsState = {
  insights: CloseInsight[];
  loading: boolean;
  closingSymbol?: string;
  message?: string;
  error?: string;
};

export class CloseInsightsModel {
  readonly state$ = new BehaviorSubject<CloseInsightsState>({ insights: [], loading: true });
  private timer?: ReturnType<typeof setInterval>;

  start(): void {
    if (this.timer) return;
    void this.refresh();
    this.timer = setInterval(() => void this.refresh(), 5000);
  }

  stop(): void {
    if (this.timer) clearInterval(this.timer);
    this.timer = undefined;
  }

  async refresh(): Promise<void> {
    try {
      const [positionsResponse, insightsResponse] = await Promise.all([
        fetch('/positions'),
        fetch('/positions/insights'),
      ]);
      if (!positionsResponse.ok || !insightsResponse.ok) {
        throw new Error('Could not load close-out insights');
      }
      const positions = (await positionsResponse.json() as PositionsResponse).positions;
      const insights = (await insightsResponse.json() as InsightsResponse).insights;
      const positionsBySymbol = new Map(positions.map((position) => [position.symbol, position]));
      this.state$.next({
        ...this.state$.value,
        loading: false,
        error: undefined,
        insights: insights.flatMap((insight) => {
          const position = positionsBySymbol.get(insight.symbol);
          return position && Math.abs(position.quantity) > 1e-12
            ? [{
                position,
                closeSide: insight.close_side,
                closeQuantity: insight.close_quantity,
                closePrice: insight.close_price,
                closePnl: insight.close_pnl,
                depthCoversPosition: insight.depth_covers_position,
                profitableCloseProbability5m: insight.profitable_close_probability_5m,
                probabilitySamples: insight.probability_samples,
              }]
            : [];
        }),
      });
    } catch (reason) {
      this.state$.next({
        ...this.state$.value,
        loading: false,
        error: reason instanceof Error ? reason.message : 'Could not load close-out insights',
      });
    }
  }

  async closePosition(insight: CloseInsight): Promise<void> {
    if (insight.closePrice === undefined || insight.closePnl === undefined || insight.closePnl <= 0) return;
    const current = this.state$.value;
    if (current.closingSymbol) return;
    const quote = insight.position.symbol.split('-')[1] ?? '';
    if (!window.confirm(
      'Close ' + insight.closeQuantity + ' ' + insight.position.symbol +
      ' for an estimated ' + insight.closePnl.toFixed(4) + ' ' + quote +
      ' profit? An IOC order can fill at a different price.',
    )) return;
    this.state$.next({ ...current, closingSymbol: insight.position.symbol, message: undefined, error: undefined });
    try {
      const response = await fetch('/positions/close-profitable', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ symbol: insight.position.symbol }),
      });
      if (!response.ok) throw new Error((await response.text()).trim() || 'Close order rejected');
      const result = await response.json() as {
        filled_quantity?: number;
        remaining_quantity?: number;
        average_execution_price?: number;
      };
      const remaining = result.remaining_quantity ?? Math.max(insight.closeQuantity - (result.filled_quantity ?? 0), 0);
      this.state$.next({
        ...this.state$.value,
        message: insight.position.symbol + ': IOC filled ' + (result.filled_quantity ?? 0) +
          (result.average_execution_price === undefined ? '' : ' at ' + result.average_execution_price) +
          '; ' + remaining + ' remains.',
      });
      await this.refresh();
    } catch (reason) {
      this.state$.next({ ...this.state$.value, error: reason instanceof Error ? reason.message : 'Could not close position' });
    } finally {
      this.state$.next({ ...this.state$.value, closingSymbol: undefined });
    }
  }
}
