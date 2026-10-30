import { BehaviorSubject } from 'rxjs';

export type AutoTraderShadowPosition = {
  decisionId: string;
  symbol: string;
  strategy: string;
  source: 'Scout' | 'Exploratory';
  direction: 'bullish' | 'bearish';
  strength: number;
  openedAt: number;
  closedAt?: number;
  entryPrice?: number;
  entryBid?: number;
  entryAsk?: number;
  takerFeeRate: number;
  status: 'Open' | 'Won' | 'Lost' | 'Flat' | 'Outcome unavailable';
  netReturnPercent?: number;
  returnBasis?: 'five-minute bid/ask evaluation' | 'ticker estimate';
};

type AuditRecord = {
  decision_id?: string;
  stage?: string;
  status?: string;
  timestamp?: number;
  signal_timestamp?: number;
  symbol?: string;
  direction?: string;
  strategy?: string;
  strength?: number;
  scout?: boolean;
  exploratory?: boolean | null;
  shadow_entry_bid?: number;
  shadow_entry_ask?: number;
  shadow_taker_fee_rate?: number;
};

type SignalEvaluation = {
  symbol: string;
  signal_timestamp: number;
  evaluated_timestamp: number;
  horizon_seconds: number;
  signal_direction: string;
  long_net_percent?: number | null;
  short_net_percent?: number | null;
  taker_fee_rate: number;
};

const shadowHorizonSeconds = 300;
const historyWindowMs = 15 * 60_000;
const rowKey = (symbol: string, timestamp: number, direction: string) => `${symbol}|${timestamp}|${direction}`;

export class AutoTraderShadowModel {
  readonly state$ = new BehaviorSubject<{ rows: AutoTraderShadowPosition[]; loading: boolean; error?: string }>({ rows: [], loading: true });
  private timer?: ReturnType<typeof setInterval>;
  private refreshing = false;
  private consumers = 0;

  start() {
    this.consumers += 1;
    if (this.consumers > 1) return;
    void this.refresh();
    this.timer = setInterval(() => void this.refresh(), 15_000);
  }

  stop() {
    this.consumers = Math.max(0, this.consumers - 1);
    if (this.consumers > 0) return;
    if (this.timer) clearInterval(this.timer);
    this.timer = undefined;
  }

  private async refresh() {
    if (this.refreshing) return;
    this.refreshing = true;
    const since = Date.now() - historyWindowMs;
    try {
      const [auditResponse, evaluationResponse] = await Promise.all([
        fetch(`/signals/auto-trader-audit?since=${since}&limit=10000`),
        fetch(`/signals/evaluations?since=${since}&limit=50000`),
      ]);
      if (!auditResponse.ok || !evaluationResponse.ok) throw new Error('Shadow outcome data unavailable');
      const [auditRows, evaluations] = await Promise.all([
        auditResponse.json() as Promise<AuditRecord[]>,
        evaluationResponse.json() as Promise<SignalEvaluation[]>,
      ]);
      const evaluationBySignal = new Map<string, SignalEvaluation>();
      for (const evaluation of evaluations) {
        if (evaluation.horizon_seconds === shadowHorizonSeconds) {
          evaluationBySignal.set(rowKey(evaluation.symbol, evaluation.signal_timestamp, evaluation.signal_direction), evaluation);
        }
      }
      const shadowDecisions = auditRows
        .filter((audit) => audit.stage === 'decision' && audit.status === 'shadowed' && audit.decision_id)
        .sort((a, b) => (a.timestamp ?? 0) - (b.timestamp ?? 0));
      const latestShadowDecisions = new Map<string, AuditRecord>();
      const lastSampleAt = new Map<string, number>();
      for (const audit of shadowDecisions) {
        const timestamp = audit.timestamp ?? audit.signal_timestamp ?? 0;
        const sampleKey = `${audit.symbol ?? ''}|${audit.strategy ?? ''}|${audit.direction ?? ''}`;
        const priorSampleAt = lastSampleAt.get(sampleKey);
        if (priorSampleAt !== undefined && timestamp - priorSampleAt < shadowHorizonSeconds * 1_000) continue;
        latestShadowDecisions.set(audit.decision_id!, audit);
        lastSampleAt.set(sampleKey, timestamp);
      }
      const now = Date.now();
      const rows = [...latestShadowDecisions.values()].flatMap((audit): AutoTraderShadowPosition[] => {
        const { decision_id: decisionId, signal_timestamp: signalTimestamp, symbol, direction } = audit;
        if (!decisionId || !signalTimestamp || !symbol || (direction !== 'bullish' && direction !== 'bearish')) return [];
        const evaluation = evaluationBySignal.get(rowKey(symbol, signalTimestamp, direction));
        const netReturn = evaluation
          ? direction === 'bullish' ? evaluation.long_net_percent : evaluation.short_net_percent
          : undefined;
        const status: AutoTraderShadowPosition['status'] = typeof netReturn === 'number' && Number.isFinite(netReturn)
          ? netReturn > 1e-12 ? 'Won' : netReturn < -1e-12 ? 'Lost' : 'Flat'
          : now < (audit.timestamp ?? signalTimestamp) + shadowHorizonSeconds * 1_000 ? 'Open' : 'Outcome unavailable';
        const entryPrice = direction === 'bullish' ? audit.shadow_entry_ask : audit.shadow_entry_bid;
        return [{
          decisionId,
          symbol,
          strategy: audit.strategy ?? 'live-insights',
          source: audit.scout ? 'Scout' : 'Exploratory',
          direction,
          strength: audit.strength ?? 0,
          openedAt: audit.timestamp ?? signalTimestamp,
          closedAt: typeof evaluation?.evaluated_timestamp === 'number' ? evaluation.evaluated_timestamp : undefined,
          entryPrice: typeof entryPrice === 'number' && Number.isFinite(entryPrice) ? entryPrice : undefined,
          entryBid: typeof audit.shadow_entry_bid === 'number' && Number.isFinite(audit.shadow_entry_bid) ? audit.shadow_entry_bid : undefined,
          entryAsk: typeof audit.shadow_entry_ask === 'number' && Number.isFinite(audit.shadow_entry_ask) ? audit.shadow_entry_ask : undefined,
          takerFeeRate: evaluation?.taker_fee_rate ?? audit.shadow_taker_fee_rate ?? 0.001,
          status,
          netReturnPercent: typeof netReturn === 'number' && Number.isFinite(netReturn) ? netReturn : undefined,
          returnBasis: typeof netReturn === 'number' && Number.isFinite(netReturn) ? 'five-minute bid/ask evaluation' : undefined,
        }];
      }).sort((a, b) => b.openedAt - a.openedAt);
      this.state$.next({ rows, loading: false });
    } catch (error) {
      this.state$.next({ ...this.state$.value, loading: false, error: error instanceof Error ? error.message : 'Could not load shadow outcomes' });
    } finally {
      this.refreshing = false;
    }
  }
}

export const autoTraderShadowModel = new AutoTraderShadowModel();
