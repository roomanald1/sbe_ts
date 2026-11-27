import { BehaviorSubject, type Subscription } from 'rxjs';
import type {
  AutoTraderOutcome,
  AutoTraderOutcomesModel,
} from './AutoTraderOutcomesModel';

export type ExcursionView =
  | 'overview'
  | 'continuation'
  | 'transitions'
  | 'sensitivity'
  | 'replay'
  | 'what-if';
export type ExcursionSource = 'all' | 'autotrader' | 'scouts';
export type ExcursionRange = '30d' | '90d' | 'all';

export const EXCURSION_EVIDENCE_NOTICE =
  'Surfaced records are non-empirical; do not use them for natural-lifecycle, profitability, Learning, or promotion claims.';

export function excursionPanelStatus(
  view: ExcursionView,
  recordCount: number,
  completePathCount: number,
  postExitCount: number,
) {
  switch (view) {
    case 'what-if':
      return 'Unavailable: no authoritative deterministic replay results are provided.';
    case 'continuation':
      return postExitCount > 0
        ? `${postExitCount} records include post-exit observations; horizon coverage and cohort outcomes are not provided.`
        : 'Unavailable: no post-exit observation evidence is available.';
    case 'transitions':
      return 'Unavailable: original-cohort terminal transition counts and complete-window denominators are not provided.';
    case 'sensitivity':
      return 'Unavailable: no replay-backed policy sensitivity results are provided.';
    case 'replay':
      return completePathCount > 0
        ? 'Unavailable: aggregate complete-path evidence exists, but timestamped executable path points are not provided.'
        : 'Unavailable: no complete executable paths or timestamped path points are provided.';
    case 'overview':
      return recordCount === 0 ? 'No records match the selected filters.' : '';
  }
}

export type ExcursionTrade = {
  id: string;
  trade: AutoTraderOutcome;
  executablePathComplete: boolean;
  incompletePathReason?: string;
  grossMfePercent?: number;
  grossMaePercent?: number;
  initialRiskEligible: boolean;
  originalRiskQuote?: number;
  postExitAvailable: boolean;
  profileProvenanceAvailable: boolean;
};

export type AutoTraderExcursionsState = {
  active: boolean;
  loading: boolean;
  error?: string;
  view: ExcursionView;
  source: ExcursionSource;
  range: ExcursionRange;
  strategy: string;
  trades: ExcursionTrade[];
  selectedTradeId?: string;
  sourceCount: number;
  eligiblePathCount: number;
  incompletePathCount: number;
  whatIfAvailable: false;
};

/** Model-owned filtering and selection for the read-only excursion research view. */
export class AutoTraderExcursionsModel {
  readonly state$ = new BehaviorSubject<AutoTraderExcursionsState>({
    active: false,
    loading: true,
    view: 'overview',
    source: 'all',
    range: '30d',
    strategy: 'all',
    trades: [],
    sourceCount: 0,
    eligiblePathCount: 0,
    incompletePathCount: 0,
    whatIfAvailable: false,
  });
  private subscription?: Subscription;
  private sourceRows: AutoTraderOutcome[] = [];
  private sourceLoading = true;
  private sourceError?: string;
  private readonly outcomes: AutoTraderOutcomesModel;

  constructor(outcomes: AutoTraderOutcomesModel) {
    this.outcomes = outcomes;
  }

  start() {
    if (this.subscription) return;
    this.outcomes.start();
    this.subscription = this.outcomes.state$.subscribe((state) => {
      this.sourceLoading = state.loading;
      this.sourceError = state.error;
      this.sourceRows = [...state.rows, ...state.closedLedgerRows];
      this.project();
    });
    this.state$.next({ ...this.state$.value, active: true });
  }

  stop() {
    if (!this.subscription && !this.state$.value.active) return;
    this.subscription?.unsubscribe();
    this.subscription = undefined;
    this.outcomes.stop();
    this.state$.next({ ...this.state$.value, active: false });
  }

  setView(view: ExcursionView) {
    this.state$.next({ ...this.state$.value, view });
  }

  setSource(source: ExcursionSource) {
    this.state$.next({
      ...this.state$.value,
      source,
      selectedTradeId: undefined,
    });
    this.project();
  }

  setRange(range: ExcursionRange) {
    this.state$.next({
      ...this.state$.value,
      range,
      selectedTradeId: undefined,
    });
    this.project();
  }

  setStrategy(strategy: string) {
    this.state$.next({
      ...this.state$.value,
      strategy,
      selectedTradeId: undefined,
    });
    this.project();
  }

  selectTrade(id?: string) {
    const exists =
      id === undefined || this.state$.value.trades.some((row) => row.id === id);
    if (exists) this.state$.next({ ...this.state$.value, selectedTradeId: id });
  }

  private project() {
    const state = this.state$.value;
    const since =
      state.range === 'all'
        ? undefined
        : Date.now() - (state.range === '30d' ? 30 : 90) * 86_400_000;
    const unique = new Map<string, AutoTraderOutcome>();
    for (const row of this.sourceRows)
      unique.set(row.tradeId ?? row.rowId, row);
    const selected = [...unique.values()].filter((row) => {
      const isScout = row.scout || row.source === 'scout';
      return (
        (state.source === 'all' ||
          (state.source === 'scouts' ? isScout : !isScout)) &&
        (state.strategy === 'all' || row.strategy === state.strategy) &&
        (since === undefined ||
          row.closedAt === undefined ||
          row.closedAt >= since)
      );
    });
    const trades = selected.map((trade) => ({
      id: trade.tradeId ?? trade.rowId,
      trade,
      executablePathComplete:
        trade.executablePricePathEvidence?.complete === true,
      incompletePathReason:
        trade.executablePricePathEvidence?.complete === true
          ? undefined
          : trade.executablePricePathEvidence?.missingCoverage?.length
            ? trade.executablePricePathEvidence.missingCoverage.join(', ')
            : trade.executablePricePathEvidence
              ? 'Source reports an incomplete path without a specific reason.'
              : 'No executable path evidence was returned for this record.',
      grossMfePercent:
        trade.executablePricePathEvidence?.complete === true
          ? trade.executablePricePathEvidence.grossExecutableMfePercent
          : undefined,
      grossMaePercent:
        trade.executablePricePathEvidence?.complete === true
          ? trade.executablePricePathEvidence.grossExecutableMaePercent
          : undefined,
      initialRiskEligible:
        trade.initialRiskEvidence?.eligibleForRMultiple === true,
      originalRiskQuote: trade.initialRiskEvidence?.eligibleForRMultiple
        ? trade.initialRiskEvidence.quoteAmount
        : undefined,
      postExitAvailable: trade.postExitObservedAt !== undefined,
      profileProvenanceAvailable: Boolean(
        trade.exitProfileId && trade.exitProfileVersion !== undefined,
      ),
    }));
    const selectedTradeId = trades.some(
      (row) => row.id === state.selectedTradeId,
    )
      ? state.selectedTradeId
      : undefined;
    this.state$.next({
      ...state,
      loading: this.sourceLoading,
      error: this.sourceError,
      trades,
      selectedTradeId,
      sourceCount: selected.length,
      eligiblePathCount: trades.filter((row) => row.executablePathComplete)
        .length,
      incompletePathCount: trades.filter((row) => !row.executablePathComplete)
        .length,
      whatIfAvailable: false,
    });
  }
}
