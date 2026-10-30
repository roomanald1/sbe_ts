import { BehaviorSubject } from 'rxjs';
import type { Position } from './PositionTypes';
import { currentPositionSnapshot, subscribePositionStream } from '../position_stream';
import { currentInsightsSnapshot, subscribeInsightsStream } from '../insights_stream';

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
  close_depth_quantity: number;
  profitable_close_probability_5m?: number;
  probability_samples: number;
};
type BarrierOrder = { id: string; symbol: string; quantity: number; remaining_quantity: number; trigger_price: number; trigger_direction: 'above' | 'below'; status: string };
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
  closeDepthQuantity: number;
  profitableCloseProbability5m?: number;
  probabilitySamples: number;
};
export type CloseInsightsState = {
  insights: CloseInsight[];
  loading: boolean;
  closingSymbol?: string;
  barrierTargetUsd: string;
  pendingBarrierSymbols: string[];
  barriers: BarrierOrder[];
  confirmingInsight?: CloseInsight;
  error?: string;
};

export class CloseInsightsModel {
  readonly state$ = new BehaviorSubject<CloseInsightsState>({ insights: [], loading: true, barrierTargetUsd: '1', barriers: [], pendingBarrierSymbols: [] });
  private livePositions?: Position[];
  private liveInsights?: BackendInsight[];
  private liveBarriers: BarrierOrder[] = [];
  private usdRates: Record<string, number> = {};
  private stopPositionStream?: () => void;
  private stopInsightsStream?: () => void;

  start(): void {
    if (this.stopPositionStream) return;
    this.usdRates = currentPositionSnapshot()?.usd_rates?.rates ?? {};
    this.stopPositionStream = subscribePositionStream((snapshot) => {
      this.usdRates = snapshot.usd_rates?.rates ?? this.usdRates;
      this.livePositions = snapshot.positions.positions;
      this.liveBarriers = snapshot.orders.barrier_orders ?? [];
      void this.refresh();
    });
    this.liveInsights = currentInsightsSnapshot()?.close_insights?.insights as BackendInsight[] | undefined;
    this.stopInsightsStream = subscribeInsightsStream((snapshot) => {
      this.liveInsights = snapshot.close_insights?.insights as BackendInsight[] | undefined;
      void this.refresh();
    });
    void this.refresh();
  }

  stop(): void {
    this.stopPositionStream?.();
    this.stopPositionStream = undefined;
    this.stopInsightsStream?.();
    this.stopInsightsStream = undefined;
  }

  async refresh(): Promise<void> {
    const positions = this.livePositions ?? currentPositionSnapshot()?.positions.positions;
    const insights = this.liveInsights ?? currentInsightsSnapshot()?.close_insights?.insights as BackendInsight[] | undefined;
    if (!positions || !insights) return;
      const positionsBySymbol = new Map(positions.map((position) => [position.symbol, position]));
      const rows = insights.flatMap((insight) => {
        const position = positionsBySymbol.get(insight.symbol);
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
              depthCoversPosition: insight.depth_covers_position,
              closeDepthQuantity: insight.close_depth_quantity,
              profitableCloseProbability5m: insight.profitable_close_probability_5m,
              probabilitySamples: insight.probability_samples,
            }]
          : [];
      });
      const rates = this.usdRates;
      this.state$.next({
        ...this.state$.value,
        loading: false,
        error: undefined,
        barriers: this.liveBarriers,
        insights: rows.map((row) => {
          const quote = row.position.symbol.split('-')[1] ?? '';
          const rate = rates[quote];
          return {
            ...row,
            closeFeeUsd: row.closeFee !== undefined && rate !== undefined ? row.closeFee * rate : undefined,
            closePnlUsd: row.closePnl !== undefined && rate !== undefined ? row.closePnl * rate : undefined,
          };
        }),
      });
  }

  setBarrierTargetUsd(barrierTargetUsd: string): void {
    this.state$.next({ ...this.state$.value, barrierTargetUsd, error: undefined });
  }

  async createProfitBarrier(insight: CloseInsight): Promise<void> {
    const current = this.state$.value;
    const symbol = insight.position.symbol;
    if (current.pendingBarrierSymbols.includes(symbol) || current.barriers.some((barrier) => barrier.symbol === symbol && barrier.status === 'armed')) return;
    const target = Number(current.barrierTargetUsd);
    if (!Number.isFinite(target) || target <= 0) {
      this.state$.next({ ...current, error: 'Enter a USD profit target greater than zero' });
      return;
    }
    this.state$.next({ ...current, pendingBarrierSymbols: [...current.pendingBarrierSymbols, symbol], error: undefined });
    try {
      const response = await fetch('/positions/barriers/profit-target', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ symbol, target_profit_usd: target }),
      });
      if (!response.ok) {
        const detail = (await response.text()).trim();
        throw new Error(detail || ('Could not create profit barrier (HTTP ' + response.status + ')'));
      }
      await this.refresh();
    } catch (reason) {
      this.state$.next({ ...this.state$.value, error: reason instanceof Error ? reason.message : 'Could not create profit barrier' });
    } finally {
      this.state$.next({ ...this.state$.value, pendingBarrierSymbols: this.state$.value.pendingBarrierSymbols.filter((pendingSymbol) => pendingSymbol !== symbol) });
    }
  }

  async cancelBarrier(barrier: BarrierOrder): Promise<void> {
    const current = this.state$.value;
    if (barrier.status !== 'armed' || current.pendingBarrierSymbols.includes(barrier.symbol)) return;
    this.state$.next({ ...current, pendingBarrierSymbols: [...current.pendingBarrierSymbols, barrier.symbol], error: undefined });
    try {
      const response = await fetch('/positions/orders/' + encodeURIComponent(barrier.id), { method: 'DELETE' });
      if (!response.ok) {
        const detail = (await response.text()).trim();
        throw new Error(detail || ('Could not cancel barrier (HTTP ' + response.status + ')'));
      }
      await this.refresh();
    } catch (reason) {
      this.state$.next({ ...this.state$.value, error: reason instanceof Error ? reason.message : 'Could not cancel barrier' });
    } finally {
      this.state$.next({ ...this.state$.value, pendingBarrierSymbols: this.state$.value.pendingBarrierSymbols.filter((symbol) => symbol !== barrier.symbol) });
    }
  }

  requestClose(insight: CloseInsight): void {
    if (insight.closeDepthQuantity <= 0 || this.state$.value.closingSymbol) return;
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
    this.state$.next({ ...current, confirmingInsight: undefined });
    await this.submitClose(insight);
  }

  private async submitClose(insight: CloseInsight): Promise<void> {
    const current = this.state$.value;
    if (current.closingSymbol) return;
    this.state$.next({ ...current, confirmingInsight: undefined, closingSymbol: insight.position.symbol, error: undefined });
    try {
      const response = await fetch('/positions/close-profitable', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ symbol: insight.position.symbol }),
      });
      if (!response.ok) {
        const detail = (await response.text()).trim();
        throw new Error(detail || ('Close IOC failed (HTTP ' + response.status + (response.statusText ? ' ' + response.statusText : '') + ')'));
      }
      await this.refresh();
    } catch (reason) {
      this.state$.next({ ...this.state$.value, error: reason instanceof Error ? reason.message : 'Could not close position' });
    } finally {
      this.state$.next({ ...this.state$.value, closingSymbol: undefined });
    }
  }
}
