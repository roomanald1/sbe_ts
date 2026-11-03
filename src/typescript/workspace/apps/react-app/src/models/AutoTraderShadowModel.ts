import { BehaviorSubject } from 'rxjs';

export type AutoTraderShadowPosition = {
  decisionId: string;
  symbol: string;
  strategy: string;
  source: 'Scout' | 'Exploratory';
  scoutLevelPercent?: number;
  scoutParameterKey?: string;
  scoutParameterValue?: number;
  scoutParameterEntryEligible?: boolean;
  exitScenario: 'strategy' | 'tight';
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
  returnBasis?: 'strategy exit' | 'ticker estimate';
};

type AuditRecord = {
  decision_id?: string;
  shadow_decision_id?: string;
  stage?: string;
  status?: string;
  timestamp?: number;
  signal_timestamp?: number;
  symbol?: string;
  direction?: string;
  strategy?: string;
  strength?: number;
  scout?: boolean;
  scout_level_percent?: number;
  scout_parameter_key?: string;
  scout_parameter_value?: number;
  scout_parameter_entry_eligible?: boolean;
  shadow_exit_scenario?: 'strategy' | 'tight';
  exploratory?: boolean | null;
  shadow_entry_bid?: number;
  shadow_entry_ask?: number;
  shadow_entry_price?: number;
  shadow_taker_fee_rate?: number;
  shadow_net_return_percent?: number;
  evaluated_timestamp?: number;
};

type AuditPage = { items: AuditRecord[]; next_cursor?: number | null };

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

const historyWindowMs = 7 * 24 * 60 * 60_000;
const repeatedScenarioCooldownMs = 60 * 60_000;

export class AutoTraderShadowModel {
  readonly state$ = new BehaviorSubject<{ rows: AutoTraderShadowPosition[]; learningOutcomes: Array<{ strategy: string; netReturnPercent: number; parameterKey?: string; parameterValue?: number }>; loading: boolean; error?: string }>({ rows: [], learningOutcomes: [], loading: true });
  private timer?: ReturnType<typeof setInterval>;
  private refreshing = false;
  private consumers = 0;

  start() {
    this.consumers += 1;
    if (this.consumers > 1) return;
    void this.refresh();
    this.timer = setInterval(() => void this.refresh(), 60_000);
  }

  stop() {
    this.consumers = Math.max(0, this.consumers - 1);
    if (this.consumers > 0) return;
    if (this.timer) clearInterval(this.timer);
    this.timer = undefined;
  }

  private async loadAuditHistory(since: number): Promise<AuditRecord[]> {
    const rows: AuditRecord[] = [];
    let cursor: number | undefined;
    do {
      const query = new URLSearchParams({ since: String(since), limit: '10000' });
      if (cursor !== undefined) query.set('cursor', String(cursor));
      const response = await fetch("/signals/auto-trader-audit?" + query.toString());
      if (!response.ok) throw new Error('Shadow outcome data unavailable');
      const page = await response.json() as AuditPage;
      rows.push(...page.items);
      cursor = typeof page.next_cursor === 'number' ? page.next_cursor : undefined;
    } while (cursor !== undefined);
    return rows;
  }

  private async refresh() {
    if (this.refreshing) return;
    this.refreshing = true;
    const since = Date.now() - historyWindowMs;
    try {
      const [auditRows, evaluationResponse] = await Promise.all([
        this.loadAuditHistory(since),
        fetch('/signals/evaluations?since=' + since + '&limit=150000'),
      ]);
      if (!evaluationResponse.ok) throw new Error('Shadow outcome data unavailable');
      const evaluations = await evaluationResponse.json() as SignalEvaluation[];
      const evaluationsBySignal = new Map<string, SignalEvaluation>();
      for (const evaluation of evaluations) {
        evaluationsBySignal.set(evaluation.symbol + '|' + evaluation.signal_timestamp, evaluation);
      }
      const outcomesByDecision = new Map<string, AuditRecord>();
      for (const audit of auditRows) {
        if (audit.stage === 'scout-outcome' && audit.shadow_decision_id) {
          outcomesByDecision.set(audit.shadow_decision_id, audit);
        }
      }
      const shadowDecisions = auditRows
        .filter((audit) => audit.stage === 'decision' && audit.status === 'shadowed' && audit.decision_id)
        .sort((a, b) => (a.timestamp ?? 0) - (b.timestamp ?? 0));
      const latestShadowDecisions = new Map<string, AuditRecord>();
      const lastSampleAt = new Map<string, number>();
      for (const audit of shadowDecisions) {
        const timestamp = audit.timestamp ?? audit.signal_timestamp ?? 0;
        const sampleKey = (audit.symbol ?? '') + '|' + (audit.strategy ?? '') + '|' + (audit.direction ?? '') + '|' + (audit.scout_level_percent ?? 'exploratory') + '|' + (audit.scout_parameter_key ?? '') + ':' + (audit.scout_parameter_value ?? '') + '|' + (audit.shadow_exit_scenario ?? 'strategy');
        const priorSampleAt = lastSampleAt.get(sampleKey);
        if (priorSampleAt !== undefined && timestamp - priorSampleAt < repeatedScenarioCooldownMs) continue;
        latestShadowDecisions.set(audit.decision_id!, audit);
        lastSampleAt.set(sampleKey, timestamp);
      }
      const rows = [...latestShadowDecisions.values()].flatMap((audit): AutoTraderShadowPosition[] => {
        const { decision_id: decisionId, signal_timestamp: signalTimestamp, symbol, direction } = audit;
        if (!decisionId || !signalTimestamp || !symbol || (direction !== 'bullish' && direction !== 'bearish')) return [];
        const outcome = outcomesByDecision.get(decisionId);
        const evaluation = evaluationsBySignal.get(symbol + '|' + signalTimestamp);
        const netReturn = typeof outcome?.shadow_net_return_percent === 'number'
          ? outcome.shadow_net_return_percent
          : evaluation
            ? direction === 'bullish' ? evaluation.long_net_percent : evaluation.short_net_percent
            : undefined;
        const entryPrice = direction === 'bullish'
          ? audit.shadow_entry_ask ?? audit.shadow_entry_price
          : audit.shadow_entry_bid ?? audit.shadow_entry_price;
        const missingLiveScoutEntry = audit.scout === true
          && audit.strategy?.startsWith('live-') === true
          && !(typeof entryPrice === 'number' && Number.isFinite(entryPrice) && entryPrice > 0);
        const status: AutoTraderShadowPosition['status'] = typeof netReturn === 'number' && Number.isFinite(netReturn)
          ? netReturn > 1e-12 ? 'Won' : netReturn < -1e-12 ? 'Lost' : 'Flat'
          : outcome || evaluation || missingLiveScoutEntry ? 'Outcome unavailable' : 'Open';
        return [{
          decisionId,
          symbol,
          strategy: audit.strategy ?? 'live-insights',
          source: audit.scout ? 'Scout' : 'Exploratory',
          scoutLevelPercent: typeof audit.scout_level_percent === 'number' ? audit.scout_level_percent : undefined,
          scoutParameterKey: audit.scout_parameter_key,
          scoutParameterValue: typeof audit.scout_parameter_value === 'number' ? audit.scout_parameter_value : undefined,
          scoutParameterEntryEligible: typeof audit.scout_parameter_entry_eligible === 'boolean' ? audit.scout_parameter_entry_eligible : undefined,
          exitScenario: audit.shadow_exit_scenario ?? 'strategy',
          direction,
          strength: audit.strength ?? 0,
          openedAt: audit.timestamp ?? signalTimestamp,
          closedAt: typeof outcome?.evaluated_timestamp === 'number'
            ? outcome.evaluated_timestamp
            : typeof evaluation?.evaluated_timestamp === 'number' ? evaluation.evaluated_timestamp : undefined,
          entryPrice: typeof entryPrice === 'number' && Number.isFinite(entryPrice) ? entryPrice : undefined,
          entryBid: typeof audit.shadow_entry_bid === 'number' && Number.isFinite(audit.shadow_entry_bid) ? audit.shadow_entry_bid : undefined,
          entryAsk: typeof audit.shadow_entry_ask === 'number' && Number.isFinite(audit.shadow_entry_ask) ? audit.shadow_entry_ask : undefined,
          takerFeeRate: outcome?.shadow_taker_fee_rate ?? evaluation?.taker_fee_rate ?? audit.shadow_taker_fee_rate ?? 0.001,
          status,
          netReturnPercent: typeof netReturn === 'number' && Number.isFinite(netReturn) ? netReturn : undefined,
          returnBasis: typeof netReturn === 'number' && Number.isFinite(netReturn) ? 'strategy exit' : undefined,
        }];
      }).sort((a, b) => b.openedAt - a.openedAt);
      const learningOutcomes = auditRows.flatMap((audit) =>
        audit.stage === 'scout-outcome' && typeof audit.strategy === 'string' &&
        typeof audit.shadow_net_return_percent === 'number' && Number.isFinite(audit.shadow_net_return_percent)
          ? [{
              strategy: audit.strategy,
              netReturnPercent: audit.shadow_net_return_percent,
              ...(typeof audit.scout_parameter_key === 'string' ? { parameterKey: audit.scout_parameter_key } : {}),
              ...(typeof audit.scout_parameter_value === 'number' && Number.isFinite(audit.scout_parameter_value) ? { parameterValue: audit.scout_parameter_value } : {}),
            }]
          : []);
      this.state$.next({ rows, learningOutcomes, loading: false });
    } catch (error) {
      this.state$.next({ ...this.state$.value, loading: false, error: error instanceof Error ? error.message : 'Could not load shadow outcomes' });
    } finally {
      this.refreshing = false;
    }
  }
}

export const autoTraderShadowModel = new AutoTraderShadowModel();
