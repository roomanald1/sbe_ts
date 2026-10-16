import { BehaviorSubject } from 'rxjs';
import type { Position } from './PositionTypes';

type PositionsResponse = { positions: Position[] };
type BackendInsight = {
  symbol: string;
  close_side: 'buy' | 'sell';
  close_quantity: number;
  close_price?: number;
  current_price?: number;
  breakeven_price?: number;
  close_pnl?: number;
  close_fee?: number;
  depth_covers_position: boolean;
  profitable_close_probability_5m?: number;
  probability_samples: number;
};
type InsightsResponse = { insights: BackendInsight[] };
type UsdRatesResponse = { rates: Record<string, number> };
export type CloseInsight = {
  position: Position;
  closeSide: 'buy' | 'sell';
  closeQuantity: number;
  closePrice?: number;
  currentPrice?: number;
  breakevenPrice?: number;
  closePnl?: number;
  closeFee?: number;
  closeFeeUsd?: number;
  closePnlUsd?: number;
  depthCoversPosition: boolean;
  profitableCloseProbability5m?: number;
  probabilitySamples: number;
};
export type CloseInsightsState = {
  insights: CloseInsight[];
  loading: boolean;
  closingSymbol?: string;
  confirmingInsight?: CloseInsight;
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
      const [positionsResponse, insightsResponse, ratesResponse] = await Promise.all([
        fetch('/positions'),
        fetch('/positions/insights'),
        fetch('/positions/usd-rates'),
      ]);
      if (!positionsResponse.ok || !insightsResponse.ok) throw new Error('Could not load close-out insights');
      const positions = (await positionsResponse.json() as PositionsResponse).positions;
      const insights = (await insightsResponse.json() as InsightsResponse).insights;
      const rates = ratesResponse.ok ? (await ratesResponse.json() as UsdRatesResponse).rates : {};
      const positionsBySymbol = new Map(positions.map((position) => [position.symbol, position]));
      this.state$.next({
        ...this.state$.value,
        loading: false,
        error: undefined,
        insights: insights.flatMap((insight) => {
          const position = positionsBySymbol.get(insight.symbol);
          const quote = insight.symbol.split('-')[1] ?? '';
          const rate = rates[quote];
          return position && Math.abs(position.quantity) > 1e-12
            ? [{
                position,
                closeSide: insight.close_side,
                closeQuantity: insight.close_quantity,
                closePrice: insight.close_price,
                currentPrice: insight.current_price,
                breakevenPrice: insight.breakeven_price,
                closePnl: insight.close_pnl,
                closeFee: insight.close_fee,
                closeFeeUsd: insight.close_fee !== undefined && rate !== undefined ? insight.close_fee * rate : undefined,
                closePnlUsd: insight.close_pnl !== undefined && rate !== undefined ? insight.close_pnl * rate : undefined,
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

  requestClose(insight: CloseInsight): void {
    if (!insight.depthCoversPosition || insight.closePrice === undefined ||
        !Number.isFinite(insight.closePrice) || insight.closePrice <= 0 ||
        this.state$.value.closingSymbol) return;
    this.state$.next({ ...this.state$.value, confirmingInsight: insight, error: undefined });
  }

  cancelClose(): void {
    if (this.state$.value.closingSymbol) return;
    this.state$.next({ ...this.state$.value, confirmingInsight: undefined });
  }

  async confirmClose(): Promise<void> {
    const current = this.state$.value;
    const insight = current.confirmingInsight;
    if (!insight || current.closingSymbol) return;
    this.state$.next({ ...current, confirmingInsight: undefined, closingSymbol: insight.position.symbol, error: undefined });
    try {
      const response = await fetch('/positions/close-profitable', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ symbol: insight.position.symbol }),
      });
      if (!response.ok) throw new Error((await response.text()).trim() || 'Close order rejected');
      await this.refresh();
    } catch (reason) {
      this.state$.next({ ...this.state$.value, error: reason instanceof Error ? reason.message : 'Could not close position' });
    } finally {
      this.state$.next({ ...this.state$.value, closingSymbol: undefined });
    }
  }
}
