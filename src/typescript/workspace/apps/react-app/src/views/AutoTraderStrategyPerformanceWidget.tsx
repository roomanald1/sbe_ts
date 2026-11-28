import { useEffect, useMemo, useRef } from 'react';
import { AgGridReact } from 'ag-grid-react';
import {
  ClientSideRowModelModule,
  ModuleRegistry,
  themeQuartz,
  type ColDef,
  type ColGroupDef,
  type GridApi,
  type ICellRendererParams,
} from 'ag-grid-community';
import {
  AutoTraderOutcomesModel,
  type AutoTraderOutcome,
  type StrictEntryGateSummary,
} from '../models/AutoTraderOutcomesModel';
import { autoTraderScoutModel } from '../models/AutoTraderScoutModel';
import {
  CompactTable,
  OperatorMetricStrip,
  OperatorStatus,
  OperatorTabBar,
  useBehaviorSnapshot,
} from '@workspace/ui-platform';
import {
  AutoTraderStrategyPerformanceModel,
  STRATEGY_PERFORMANCE_COLUMN_GROUPS,
  projectExitProfileSnapshot,
  projectStrategyCoverageByStrategy,
  projectStrategyCoverageHeatmap,
  projectStrategyOutcomeCoverage,
  projectStrategyOutcomeHistory,
  projectScoutOutcomeCounts,
  summarizeStrategyOutcomeEvidence,
} from '../models/AutoTraderStrategyPerformanceModel';
import {
  sharedAutoTraderSettingsModel,
  type AutoTraderStrategyProfile,
  type Strategy,
} from '../models/AutoTraderModel';
import { scoutableStrategies } from './AutoTraderStrategyParameters';
import { strategyLabel } from './AutoTraderOrderGrid';

ModuleRegistry.registerModules([ClientSideRowModelModule]);
const theme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});
const strategyGridDefaultColDef: ColDef<StrategyWinRateRow> = {
  sortable: true,
  resizable: true,
  flex: 0,
  minWidth: 48,
};
const strategyGridGetRowId = ({ data }: { data: StrategyWinRateRow }) =>
  data.rowId;
const pct = (v: unknown) =>
  typeof v === 'number' && Number.isFinite(v) ? v.toFixed(3) + '%' : '—';
const reasonAge = (timestamp: number | undefined) => {
  if (!timestamp || !Number.isFinite(timestamp)) return 'time unavailable';
  const ageSeconds = Math.max(0, Math.floor((Date.now() - timestamp) / 1000));
  if (ageSeconds < 60) return 'just now';
  if (ageSeconds < 3600) return Math.floor(ageSeconds / 60) + 'm ago';
  if (ageSeconds < 86400) return Math.floor(ageSeconds / 3600) + 'h ago';
  return 'stale · ' + Math.floor(ageSeconds / 86400) + 'd ago';
};
const entryEvaluationReason = (
  evaluation:
    | {
        reason?: string;
        timestamp?: number;
        decision: {
          btc_gate_required?: boolean;
          btc_gate_passed?: boolean | null;
          btc_window_days?: number | null;
          primary_passed?: boolean;
          primary_rule?: string;
          primary_metric_value?: number | null;
          primary_metric_unit?: string | null;
          primary_level?: number | null;
          primary_distance_percent?: number | null;
          secondary_passed?: boolean | null;
          secondary_rule?: string | null;
          secondary_distance_percent?: number | null;
        };
      }
    | undefined,
  fallbackReason?: string,
) => {
  if (!evaluation) return fallbackReason ?? 'No blocker recorded';
  const withAge = (reason: string) =>
    `${reason} · ${reasonAge(evaluation.timestamp)}`;
  if (evaluation.reason?.trim()) return withAge(evaluation.reason);
  const decision = evaluation.decision;
  if (decision.btc_gate_required && decision.btc_gate_passed === false)
    return withAge(
      `BTC trend gate not passed${decision.btc_window_days ? ` (SMA${decision.btc_window_days})` : ''}`,
    );
  if (decision.primary_passed === false && decision.primary_rule) {
    const isRelativeStrength = /return vs eligible assets/i.test(
      decision.primary_rule,
    );
    const observed =
      typeof decision.primary_metric_value === 'number'
        ? `observed ${decision.primary_metric_value.toFixed(2)}${decision.primary_metric_unit ? ` ${decision.primary_metric_unit}` : ''}`
        : 'observed value unavailable';
    const required =
      typeof decision.primary_level === 'number'
        ? isRelativeStrength
          ? `; leader return ${decision.primary_level.toFixed(2)}${decision.primary_metric_unit ? ` ${decision.primary_metric_unit}` : ''}`
          : `; required ${decision.primary_level.toFixed(2)}${decision.primary_metric_unit ? ` ${decision.primary_metric_unit}` : ''}`
        : '';
    const distance =
      typeof decision.primary_distance_percent === 'number'
        ? isRelativeStrength
          ? `; gap ${decision.primary_distance_percent.toFixed(2)} pp`
          : `; distance ${decision.primary_distance_percent.toFixed(2)}%`
        : '';
    return withAge(
      `${decision.primary_rule} not met (${observed}${required}${distance})`,
    );
  }
  if (decision.secondary_passed === false)
    return withAge(
      `${decision.secondary_rule ?? 'Secondary rule'} not met${typeof decision.secondary_distance_percent === 'number' ? ` (distance ${decision.secondary_distance_percent.toFixed(2)}%)` : ''}`,
    );
  return withAge('No rejection recorded in the latest symbol evaluation');
};
const usd = new Intl.NumberFormat(undefined, {
  style: 'currency',
  currency: 'USD',
  minimumFractionDigits: 3,
  maximumFractionDigits: 3,
});
const signed = {
  'pnl-positive': (p: { value: unknown }) =>
    typeof p.value === 'number' && p.value > 0,
  'pnl-negative': (p: { value: unknown }) =>
    typeof p.value === 'number' && p.value < 0,
};
const sharedOutcomesModel = new AutoTraderOutcomesModel();
const median = (values: number[]) => {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2
    ? sorted[middle]
    : (sorted[middle - 1] + sorted[middle]) / 2;
};
const durationLabel = (value: unknown) => {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0)
    return '—';
  const totalMinutes = Math.floor(value / 60_000);
  const days = Math.floor(totalMinutes / 1_440);
  const hours = Math.floor((totalMinutes % 1_440) / 60);
  const minutes = totalMinutes % 60;
  if (days > 0) return days + 'd ' + hours + 'h';
  if (hours > 0) return hours + 'h ' + minutes + 'm';
  if (totalMinutes > 0) return totalMinutes + 'm';
  return Math.floor(value / 1_000) + 's';
};
type StrategyWinRateRow = {
  rowId: string;
  strategy: string;
  enabled?: boolean;
  scoutEnabled?: boolean;
  scoutable?: boolean;
  runStatus: string;
  runReason: string;
  runReasonAt?: number;
  trades: number;
  strictTrades: number;
  exploratoryTrades: number;
  scoutTrades: number;
  paperCarryTrades: number;
  unknownSourceTrades: number;
  todayRealizedUsd?: number;
  medianHoldingMs?: number;
  averageHoldingMs?: number;
  periodStart: number;
  periodEnd: number;
  learningSamples?: number;
  learningPositiveRate?: number;
  learningAverageNet?: number;
  effectiveSizeMultiplier?: number;
  feedbackMultiplier?: number;
  scoutOutcomeSamples?: number;
  scoutOutcomeAverageNet?: number;
  pairedScoutSamples?: number;
  pairedScoutAverageLift?: number;
  strictEntryGate?: StrictEntryGateSummary;
  strictBaseStrengthFloor?: number;
  strictGateApplies?: boolean;
  rowType?: 'strategy' | 'symbol';
  symbol?: string;
  entryAverageNetReturn?: number | null;
  entryPositiveRate?: number | null;
  entrySampleCount?: number;
  entryRiskMultiplier?: number;
  entryRiskContext?: string;
  entryWhyNoEntry?: string;
};
const allStrategies: Strategy[] = [
  'live-momentum',
  'live-trade-flow',
  'live-order-book',
  'live-vwap-dislocation',
  'live-absorption-pressure',
  'live-range-mean-reversion',
  'donchian-55-btc200',
  'donchian-20-sma200',
  'donchian-20-btc200',
  'donchian-10-btc200',
  'donchian-20-btc200-hold3',
  'risk-adjusted-trend-portfolio',
  'trend-sleeves-btc200',
  'trend-50-200',
  'relative-strength-90-sma200',
  'risk-adjusted-monthly-momentum',
  'rsi2-oversold-rebound',
  'dated-futures-carry',
];
const strategyCellTints = {
  status: 'rgba(148, 157, 170, 0.06)',
  learning: 'rgba(74, 127, 151, 0.075)',
  sample: 'rgba(161, 139, 75, 0.075)',
  performance: 'rgba(77, 132, 91, 0.075)',
  profile: 'rgba(125, 100, 157, 0.075)',
  gate: 'rgba(178, 118, 72, 0.075)',
} as const;
const colorStrategyColumns = (
  columns: ColDef<StrategyWinRateRow>[],
  group: keyof typeof strategyCellTints,
) =>
  columns.map((column, index) => ({
    ...column,
    maxWidth: undefined,
    headerClass: [
      'strategy-column-' + group + '-header',
      index === 0 && 'strategy-group-start',
      index === columns.length - 1 && 'strategy-group-end',
    ]
      .filter(Boolean)
      .join(' '),
    cellClass: [
      'strategy-column-' + group + '-cell',
      index === 0 && 'strategy-group-start',
      index === columns.length - 1 && 'strategy-group-end',
    ]
      .filter(Boolean)
      .join(' '),
    cellStyle: { backgroundColor: strategyCellTints[group] },
  }));
const STRATEGY_COLUMN_GROUPS = STRATEGY_PERFORMANCE_COLUMN_GROUPS.map(
  (group) => ({
    ...group,
    label: (
      {
        performance: 'Performance',
        entryEvidence: 'Entry evidence',
        sample: 'Fully closed trades',
        status: 'Strategy status',
        profile: 'Trade profile',
        learning: 'Learning',
      } as const
    )[group.id],
  }),
);
function scrollPageFromStrategyGrid(event: globalThis.WheelEvent) {
  if (!event.deltaY) return;
  let pageScroller: HTMLElement | null = (event.currentTarget as HTMLElement)
    .parentElement;
  while (pageScroller) {
    const { overflowY } = window.getComputedStyle(pageScroller);
    if (
      (overflowY === 'auto' || overflowY === 'scroll') &&
      pageScroller.scrollHeight > pageScroller.clientHeight
    )
      break;
    pageScroller = pageScroller.parentElement;
  }
  const target = pageScroller ?? document.scrollingElement;
  if (!target) return;
  event.preventDefault();
  target.scrollTop += event.deltaY;
}

export function AutoTraderStrategyPerformanceWidget({
  widgetId = 'auto-trader-strategy-performance',
}: {
  widgetId?: string;
}) {
  const model = useMemo(
    () =>
      new AutoTraderStrategyPerformanceModel(
        {
          outcomes: {
            state$: sharedOutcomesModel.state$,
            start: () => sharedOutcomesModel.start(),
            stop: () => sharedOutcomesModel.stop(),
          },
          settings: sharedAutoTraderSettingsModel,
          scout: autoTraderScoutModel,
          refresh: async () => {
            await Promise.all([
              sharedOutcomesModel.refreshNow(),
              sharedAutoTraderSettingsModel.refresh(),
            ]);
            autoTraderScoutModel.retry();
          },
        },
        widgetId,
      ),
    [widgetId],
  );
  const viewState = useBehaviorSnapshot(model.state$);
  const state = viewState.outcomes;
  const settings = viewState.settings;
  const scoutState = viewState.scout;
  const expandedEntryStrategies = viewState.expandedEntryStrategies;
  const settingsModel = sharedAutoTraderSettingsModel;
  const strategyGridApi = useRef<GridApi<StrategyWinRateRow> | null>(null);
  const saveStrategyGridState = (api: GridApi<StrategyWinRateRow>) => {
    model.saveGridColumnState(JSON.stringify(api.getColumnState()));
  };
  const strategyGridWrapper = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  useEffect(() => {
    const wrapper = strategyGridWrapper.current;
    if (!wrapper) return;
    wrapper.addEventListener('wheel', scrollPageFromStrategyGrid, {
      passive: false,
    });
    return () =>
      wrapper.removeEventListener('wheel', scrollPageFromStrategyGrid);
  }, []);
  const hideDisabledStrategies = viewState.hideDisabledStrategies;
  const visibleColumnGroups = viewState.visibleColumnGroups;
  const updateColumnGroupVisibility = (
    id: (typeof STRATEGY_COLUMN_GROUPS)[number]['id'],
    visible: boolean,
  ) => {
    model.setColumnGroupVisible(id, visible);
    strategyGridApi.current?.setColumnsVisible(
      STRATEGY_COLUMN_GROUPS.find((group) => group.id === id)?.children ?? [],
      visible,
    );
  };
  useEffect(() => {
    const api = strategyGridApi.current;
    if (!api) return;
    for (const group of STRATEGY_COLUMN_GROUPS) {
      api.setColumnsVisible(group.children, visibleColumnGroups[group.id]);
    }
  }, [visibleColumnGroups]);
  const updateHideDisabledStrategies = (value: boolean) => {
    model.setHideDisabledStrategies(value);
  };
  const rows = state.rows;
  const closedRows = useMemo(
    () => rows.filter((row) => row.status === 'Closed'),
    [rows],
  );
  const evidenceSummary = useMemo(
    () => summarizeStrategyOutcomeEvidence(closedRows),
    [closedRows],
  );
  const coverageRows = useMemo(
    () => projectStrategyOutcomeCoverage(closedRows),
    [closedRows],
  );
  const coverageByStrategy = useMemo(
    () => projectStrategyCoverageByStrategy(coverageRows),
    [coverageRows],
  );
  const coverageHeatmap = useMemo(
    () => projectStrategyCoverageHeatmap(coverageRows),
    [coverageRows],
  );
  const closedHistory = useMemo(
    () => projectStrategyOutcomeHistory(closedRows),
    [closedRows],
  );
  const profileRows = useMemo(
    () =>
      projectExitProfileSnapshot(
        settings.decisionParameters?.strategy_profiles ?? [],
      ),
    [settings.decisionParameters?.strategy_profiles],
  );
  const scoutEvidenceRows = useMemo(
    () => projectScoutOutcomeCounts(scoutState.learningOutcomes),
    [scoutState.learningOutcomes],
  );
  const scoutOutcomeCount = scoutEvidenceRows.reduce(
    (total, row) => total + row.outcomes,
    0,
  );
  const scoutPositiveCount = scoutEvidenceRows.reduce(
    (total, row) => total + row.positiveOutcomes,
    0,
  );
  const openRows = useMemo(
    () => rows.filter((row) => row.status === 'Open'),
    [rows],
  );
  const scoutLearningByStrategy = useMemo(() => {
    const cohorts = new Map<
      string,
      { positive: number; count: number; returnTotal: number }
    >();
    for (const outcome of scoutState.learningOutcomes) {
      cohorts.set(outcome.strategy, {
        positive: outcome.positive,
        count: outcome.count,
        returnTotal: outcome.returnTotal,
      });
    }
    return new Map(
      [...cohorts].map(([strategy, cohort]) => [
        strategy,
        {
          count: cohort.count,
          positive: cohort.positive,
          positiveRate: cohort.positive / cohort.count,
          returnTotal: cohort.returnTotal,
        },
      ]),
    );
  }, [scoutState.learningOutcomes]);
  const strategyWinRates = useMemo<StrategyWinRateRow[]>(() => {
    const grouped = new Map<string, AutoTraderOutcome[]>();
    for (const row of closedRows) {
      grouped.set(row.strategy, [...(grouped.get(row.strategy) ?? []), row]);
    }
    const strategyNames = [
      ...new Set([
        ...allStrategies,
        ...grouped.keys(),
        ...Object.keys(state.todayStrategyRealizedUsd ?? {}),
        ...Object.keys(state.strategyLearning),
        ...scoutLearningByStrategy.keys(),
      ]),
    ];
    return strategyNames
      .map((strategy) => {
        const trades = grouped.get(strategy) ?? [];
        const enabled = settings.loading
          ? undefined
          : settings.strategies.includes(strategy as Strategy);
        const scoutEnabled = settings.scoutStrategies.includes(
          strategy as Strategy,
        );
        const scoutable = scoutableStrategies.some(
          (item) => item.strategy === strategy,
        );
        const latestBlocker = state.latestStrategyBlockers.find(
          (item) => item.strategy === strategy,
        );
        const openPosition =
          state.openStrategyPositions.find(
            (position) => position.strategy === strategy,
          ) ?? openRows.find((position) => position.strategy === strategy);
        const orders = state.strategyOrders
          .filter((order) => order.strategy === strategy)
          .sort((a, b) => b.timestamp - a.timestamp);
        const workingOrder = orders.find(
          (order) =>
            order.purpose !== 'stop-loss' &&
            [
              'submitting',
              'accepted',
              'resting',
              'partially_filled',
              'armed',
            ].includes(order.status ?? ''),
        );
        let runStatus: string;
        if (strategy === 'unattributed') {
          runStatus = 'Ledger adjustment';
        } else if (!state.autoTraderStatusAvailable) {
          runStatus = 'Unavailable';
        } else if (!enabled) {
          runStatus = 'Disabled';
        } else if (!state.autoTraderEnabled) {
          runStatus = 'Paused';
        } else if (
          strategy === 'dated-futures-carry' &&
          state.datedCarryDiagnostics
        ) {
          const carry = state.datedCarryDiagnostics;
          const carryPositions = (carry.positions ?? []).filter(
            (position) =>
              !['settled', 'closed', 'complete'].includes(
                position.status.toLowerCase(),
              ),
          );
          const eligibleCarry = (carry.candidates ?? []).filter(
            (candidate) => candidate.eligible,
          );
          const checkedAt = carry.checked_at_ms
            ? new Date(carry.checked_at_ms).toLocaleString()
            : undefined;
          if (carry.last_error) {
            runStatus = 'Scan error';
          } else if (carryPositions.length > 0) {
            runStatus = 'Position open';
          } else if (!checkedAt) {
            runStatus = 'Waiting for scan';
          } else if (eligibleCarry.length > 0) {
            runStatus = 'Signal active';
          } else {
            runStatus = 'No qualifying signal';
          }
        } else if (latestBlocker) {
          runStatus = 'Blocked';
        } else if (openPosition) {
          runStatus = 'Position open';
        } else if (workingOrder) {
          runStatus = 'Order working';
        } else {
          runStatus = 'No recent blocker';
        }
        const runReason = latestBlocker
          ? latestBlocker.symbol + ' · ' + latestBlocker.reason
          : '';
        const runReasonAt = latestBlocker?.timestamp_ms;
        const sourceCounts = {
          strict: 0,
          exploratory: 0,
          scout: 0,
          paperCarry: 0,
          unknown: 0,
        };
        for (const trade of trades) {
          if (trade.source === 'strict') sourceCounts.strict += 1;
          else if (trade.source === 'exploratory')
            sourceCounts.exploratory += 1;
          else if (trade.source === 'scout') sourceCounts.scout += 1;
          else if (trade.source === 'paper-carry') sourceCounts.paperCarry += 1;
          else sourceCounts.unknown += 1;
        }
        const holdingTimes = trades
          .map((row) => (row.closedAt ?? 0) - row.openedAt)
          .filter((value) => Number.isFinite(value) && value >= 0);
        const closedTimes = trades
          .map((row) => row.closedAt ?? row.openedAt)
          .filter((value) => value > 0 && Number.isFinite(value));
        const firstClosed = closedTimes.length ? Math.min(...closedTimes) : 0;
        const lastClosed = closedTimes.length ? Math.max(...closedTimes) : 0;
        const learning = state.learningSummary[strategy];
        return {
          rowId: `strategy:${strategy}`,
          strategy,
          trades: trades.length,
          strictTrades: sourceCounts.strict,
          exploratoryTrades: sourceCounts.exploratory,
          scoutTrades: sourceCounts.scout,
          paperCarryTrades: sourceCounts.paperCarry,
          unknownSourceTrades: sourceCounts.unknown,
          enabled,
          runStatus,
          runReason,
          runReasonAt,
          todayRealizedUsd:
            state.todayStrategyRealizedUsd?.[strategy] ??
            (state.todayStrategyRealizedUsd ? 0 : undefined),
          // The current outcomes contract does not expose independent
          // reconciliation or canonical strategy-level performance aggregates.
          // Do not derive profitability, returns, win rates, or cost ratios here.
          medianHoldingMs: holdingTimes.length
            ? median(holdingTimes)
            : undefined,
          averageHoldingMs: holdingTimes.length
            ? holdingTimes.reduce((sum, value) => sum + value, 0) /
              holdingTimes.length
            : undefined,
          periodStart: firstClosed,
          periodEnd: lastClosed,
          learningSamples: learning?.learning_samples,
          learningPositiveRate: learning?.positive_rate,
          learningAverageNet: learning?.average_net_return_percent,
          effectiveSizeMultiplier:
            state.strategyLearning[strategy]?.learning_multiplier,
          feedbackMultiplier:
            state.strategyLearning[strategy]?.learning_feedback_multiplier,
          scoutOutcomeSamples: learning?.scout_outcome_samples,
          scoutOutcomeAverageNet: learning?.scout_average_net_return_percent,
          pairedScoutSamples: learning?.paired_scout_samples,
          pairedScoutAverageLift: learning?.average_paired_scout_lift_percent,
          strictEntryGate: state.strictEntryGates[strategy],
          strictBaseStrengthFloor: state.strictBaseStrengthFloor,
          strictGateApplies: strategy.startsWith('live-'),
          scoutEnabled,
          scoutable,
        };
      })
      .sort((a, b) => a.strategy.localeCompare(b.strategy));
  }, [
    closedRows,
    state.learningSummary,
    state.strategyLearning,
    state.enabledStrategies,
    state.autoTraderEnabled,
    state.strategyDiagnostics,
    state.latestStrategyBlockers,
    state.latestStrategyEvaluations,
    state.strategyOrders,
    state.strictEntryGates,
    state.strictBaseStrengthFloor,
    state.todayStrategyRealizedUsd,
    state.openStrategyPositions,
    state.autoTraderStatusAvailable,
    state.datedCarryDiagnostics,
    openRows,
    scoutLearningByStrategy,
    settings.scoutStrategies,
    settings.strategies,
    settings.loading,
  ]);
  const visibleStrategyWinRates = useMemo(
    () =>
      hideDisabledStrategies
        ? strategyWinRates.filter((row) => row.enabled !== false)
        : strategyWinRates,
    [strategyWinRates, hideDisabledStrategies],
  );
  const entryReturnProfiles =
    settings.decisionParameters?.strategy_profiles ?? [];
  const entryReturnProfilesByStrategy = useMemo(() => {
    const groups = new Map<string, AutoTraderStrategyProfile[]>();
    const dailySymbols = new Set([
      'BTC-USDT',
      'ETH-USDT',
      'BNB-USDT',
      'XRP-USDT',
      'SOL-USDT',
      'ADA-USDT',
      'DOGE-USDT',
    ]);
    const portfolioSymbols = new Set([
      ...dailySymbols,
      'LTC-USDT',
      'TRX-USDT',
      'LINK-USDT',
      'AVAX-USDT',
    ]);
    const allowedSymbols = new Set(portfolioSymbols);
    for (const profile of entryReturnProfiles) {
      // Learning retains profiles beyond a strategy's evaluation universe.
      // Keep the expanded rows aligned with the symbol sets each strategy uses.
      const symbols =
        profile.strategy === 'trend-sleeves-btc200'
          ? new Set(['BTC-USDT', 'ETH-USDT', 'BNB-USDT'])
          : profile.strategy === 'risk-adjusted-trend-portfolio'
            ? portfolioSymbols
            : profile.strategy === 'dated-futures-carry'
              ? new Set(['BTC-USDT', 'ETH-USDT'])
              : [
                    'donchian-55-btc200',
                    'donchian-20-sma200',
                    'donchian-20-btc200',
                    'donchian-10-btc200',
                    'donchian-20-btc200-hold3',
                    'trend-50-200',
                    'relative-strength-90-sma200',
                    'risk-adjusted-monthly-momentum',
                    'rsi2-oversold-rebound',
                  ].includes(profile.strategy)
                ? dailySymbols
                : allowedSymbols;
      if (!symbols.has(profile.symbol)) continue;
      groups.set(profile.strategy, [
        ...(groups.get(profile.strategy) ?? []),
        profile,
      ]);
    }
    return groups;
  }, [entryReturnProfiles]);
  const latestProbeRiskByStrategySymbol = useMemo(() => {
    const latest = new Map<
      string,
      {
        openedAt: number;
        scale: number;
        sampleCount: number;
        direction?: string;
        regime?: string;
        regimeScoped: boolean;
      }
    >();
    for (const row of [...state.rows, ...state.closedLedgerRows]) {
      const risk = row.explorationRisk;
      if (row.source !== 'exploratory' || !risk || !Number.isFinite(risk.scale))
        continue;
      const key = `${row.strategy}:${row.symbol}`;
      const prior = latest.get(key);
      if (!prior || row.openedAt >= prior.openedAt)
        latest.set(key, {
          openedAt: row.openedAt,
          scale: risk.scale,
          sampleCount: risk.sampleCount,
          direction: risk.direction ?? row.direction,
          regime: risk.marketRegime,
          regimeScoped: risk.regimeScoped,
        });
    }
    return latest;
  }, [state.rows, state.closedLedgerRows]);
  // Keep column definitions stable across live settings and evidence updates.
  // AG Grid can otherwise reapply definition widths and visibly resize columns.
  const settingsRef = useRef(settings);
  settingsRef.current = settings;
  const entryProfilesByStrategyRef = useRef(entryReturnProfilesByStrategy);
  entryProfilesByStrategyRef.current = entryReturnProfilesByStrategy;
  const expandedEntryStrategiesRef = useRef(expandedEntryStrategies);
  expandedEntryStrategiesRef.current = expandedEntryStrategies;
  const computedStrategyGridRows = useMemo(
    () =>
      visibleStrategyWinRates.flatMap((row) => {
        const profiles = entryReturnProfilesByStrategy.get(row.strategy) ?? [];
        const evidenceProfiles = profiles.flatMap((profile) => {
          const evidence = profile.walk_forward_evidence;
          const samples = evidence?.sampleCount;
          return evidence && typeof samples === 'number' && samples > 0
            ? [{ evidence, samples }]
            : [];
        });
        const evidenceSampleCount = evidenceProfiles.reduce(
          (sum, item) => sum + item.samples,
          0,
        );
        const probeRisks = profiles.flatMap((profile) => {
          const risk = latestProbeRiskByStrategySymbol.get(
            `${row.strategy}:${profile.symbol}`,
          );
          return risk ? [risk] : [];
        });
        const master = {
          ...row,
          rowId: `strategy:${row.strategy}`,
          rowType: 'strategy' as const,
          entryAverageNetReturn:
            evidenceSampleCount > 0 &&
            evidenceProfiles.every(
              ({ evidence }) =>
                typeof evidence.averageNetReturnPercent === 'number',
            )
              ? evidenceProfiles.reduce(
                  (sum, { evidence, samples }) =>
                    sum + evidence.averageNetReturnPercent! * samples,
                  0,
                ) / evidenceSampleCount
              : undefined,
          entryPositiveRate:
            evidenceSampleCount > 0 &&
            evidenceProfiles.every(
              ({ evidence }) => typeof evidence.positiveNetRate === 'number',
            )
              ? evidenceProfiles.reduce(
                  (sum, { evidence, samples }) =>
                    sum + evidence.positiveNetRate! * samples,
                  0,
                ) / evidenceSampleCount
              : undefined,
          entrySampleCount: evidenceSampleCount || undefined,
          entryRiskMultiplier: probeRisks.length
            ? probeRisks.reduce((sum, value) => sum + value.scale, 0) /
              probeRisks.length
            : undefined,
          entryRiskContext: probeRisks.length
            ? `Average across ${probeRisks.length} symbols with a recent exploratory entry.`
            : undefined,
        };
        if (!expandedEntryStrategies.has(row.strategy) || profiles.length === 0)
          return [master];
        return [
          master,
          ...profiles.map((profile) => {
            const evaluation =
              state.latestStrategyEvaluationsByPosition[
                `${row.strategy}:${profile.symbol}`
              ];
            const strategyBlocker = settings.latestStrategyBlockers.find(
              (blocker) =>
                blocker.strategy === row.strategy &&
                blocker.symbol === profile.symbol,
            );
            const workerResult = settings.latestSymbolEntryResults
              .filter(
                (result) =>
                  result.strategy === row.strategy &&
                  result.symbol === profile.symbol,
              )
              .sort(
                (left, right) =>
                  (right.timestamp_ms ?? 0) - (left.timestamp_ms ?? 0),
              )[0];
            const symbolBlockers = settings.symbolGateHits
              .filter((hit) => hit.symbol === profile.symbol)
              .sort((a, b) => (b.last_seen_ms ?? 0) - (a.last_seen_ms ?? 0))
              .slice(0, 1);
            const workerReason = workerResult
              ? workerResult.reason ||
                (workerResult.decision
                  ? `Strategy: ${workerResult.decision}`
                  : 'No worker decision or skip report received for this symbol')
              : undefined;
            const workerDiagnosticAge =
              workerResult &&
              (workerResult.decision === undefined ||
                workerResult.decision === null) &&
              workerResult.timestamp_ms !== undefined &&
              workerResult.timestamp_ms !== null
                ? reasonAge(workerResult.timestamp_ms)
                : undefined;
            const fallbackReason = workerDiagnosticAge
              ? `${workerReason} · ${workerDiagnosticAge}`
              : strategyBlocker
                ? `${strategyBlocker.reason} · ${reasonAge(strategyBlocker.timestamp_ms)}`
                : symbolBlockers.length
                  ? symbolBlockers
                      .map((hit) => `${hit.direction}: ${hit.last_detail}`)
                      .join('; ')
                  : workerResult
                    ? workerResult.timestamp_ms
                      ? `${workerReason} · ${reasonAge(workerResult.timestamp_ms)}`
                      : workerReason
                    : 'No worker decision or skip report received for this symbol';
            const probeRisk = latestProbeRiskByStrategySymbol.get(
              `${row.strategy}:${profile.symbol}`,
            );
            return {
              rowId: `symbol:${row.strategy}:${profile.symbol}`,
              rowType: 'symbol' as const,
              strategy: row.strategy,
              symbol: profile.symbol,
              entryAverageNetReturn:
                profile.walk_forward_evidence?.averageNetReturnPercent,
              entryPositiveRate: profile.walk_forward_evidence?.positiveNetRate,
              entrySampleCount: profile.walk_forward_evidence?.sampleCount,
              entryRiskMultiplier: probeRisk?.scale,
              entryRiskContext: probeRisk
                ? `Latest probe: ${probeRisk.direction ?? 'direction unavailable'} · ${probeRisk.regime ?? 'regime unavailable'} regime · n=${probeRisk.sampleCount}${probeRisk.regimeScoped ? ' (regime scoped)' : ' (symbol fallback)'}.`
                : undefined,
              entryWhyNoEntry: workerDiagnosticAge
                ? fallbackReason
                : entryEvaluationReason(evaluation, fallbackReason),
            } as StrategyWinRateRow;
          }),
        ];
      }),
    [
      visibleStrategyWinRates,
      entryReturnProfilesByStrategy,
      latestProbeRiskByStrategySymbol,
      expandedEntryStrategies,
      state.latestStrategyEvaluationsByPosition,
      settings.latestStrategyBlockers,
      settings.latestSymbolEntryResults,
      settings.symbolGateHits,
    ],
  );
  // Preserve object identity for rows whose values did not change. AG Grid
  // uses getRowId plus row object identity to reconcile immutable rowData;
  // recreating every object here makes frequent snapshots look like updates
  // to the whole grid and can disrupt expanded rows.
  const previousStrategyRows = useRef(new Map<string, StrategyWinRateRow>());
  const strategyGridRows = useMemo(() => {
    const previous = previousStrategyRows.current;
    const next = new Map<string, StrategyWinRateRow>();
    const rows = computedStrategyGridRows.map((row) => {
      const prior = previous.get(row.rowId);
      const unchanged =
        prior &&
        Object.keys(row).length === Object.keys(prior).length &&
        Object.keys(row).every(
          (key) =>
            row[key as keyof StrategyWinRateRow] ===
            prior[key as keyof StrategyWinRateRow],
        );
      const stableRow = unchanged ? prior : row;
      next.set(row.rowId, stableRow);
      return stableRow;
    });
    previousStrategyRows.current = next;
    return rows;
  }, [computedStrategyGridRows]);
  const entryEvidenceGroup: ColGroupDef<StrategyWinRateRow> = {
    headerName: 'Entry evidence · strategy aggregate / per symbol',
    headerClass: 'strategy-group-learning',
    children: [
      {
        colId: 'entryAverageNetReturn',
        headerName: 'Net avg',
        headerTooltip:
          'Strategy rows show the sample-weighted aggregate across symbol profiles. Expanded symbol rows show Scout walk-forward average net return for that symbol and active parameter profile.',
        field: 'entryAverageNetReturn',
        cellClassRules: signed,
        valueFormatter: ({ value }) => pct(value),
        width: 72,
        minWidth: 72,
        maxWidth: 82,
        flex: 0,
      },
      {
        colId: 'entryPositiveRate',
        headerName: 'Positive%',
        headerTooltip:
          'Sample-weighted positive net rate across symbol profiles on strategy rows; per-symbol rate on expanded rows.',
        field: 'entryPositiveRate',
        valueFormatter: ({ value }) =>
          typeof value === 'number' && Number.isFinite(value)
            ? (value * 100).toFixed(1) + '%'
            : '—',
        width: 78,
        minWidth: 72,
        maxWidth: 86,
        flex: 0,
      },
      {
        colId: 'entrySampleCount',
        headerName: 'Samples',
        headerTooltip:
          'Total walk-forward samples across symbol profiles on strategy rows; per-symbol count on expanded rows.',
        field: 'entrySampleCount',
        valueFormatter: ({ value }) =>
          typeof value === 'number' && Number.isFinite(value)
            ? value.toLocaleString()
            : '—',
        width: 62,
        minWidth: 58,
        maxWidth: 68,
        flex: 0,
      },
      {
        colId: 'entryRiskMultiplier',
        headerName: 'Probe ×',
        headerTooltip:
          'Latest contextual exploratory size multiplier. Symbol rows show that strategy-symbol’s latest probe; strategy rows average the latest multiplier for each visible symbol. Strict entries remain governed by Scout evidence.',
        field: 'entryRiskMultiplier',
        tooltipField: 'entryRiskContext',
        valueFormatter: ({ value }) =>
          typeof value === 'number' && Number.isFinite(value)
            ? value.toFixed(2) + '×'
            : '—',
        width: 72,
        minWidth: 66,
        maxWidth: 78,
        flex: 0,
      },
      {
        colId: 'entryWhyNoEntry',
        headerName: 'Why no entry?',
        headerTooltip:
          'Latest blocker for this symbol. Uses its walk-forward evidence when no evaluation is recorded.',
        field: 'entryWhyNoEntry',
        tooltipField: 'entryWhyNoEntry',
        width: 300,
        minWidth: 240,
        flex: 0,
      },
    ],
  };
  const strategyColumns = useMemo<
    (ColDef<StrategyWinRateRow> | ColGroupDef<StrategyWinRateRow>)[]
  >(
    () => [
      {
        field: 'strategy',
        headerName: 'Strategy',
        valueGetter: ({ data }) => {
          if (!data) return '';
          if (data.rowType === 'symbol') return '↳ ' + (data.symbol ?? '');
          const hasEvidence = Boolean(
            entryProfilesByStrategyRef.current.get(data.strategy)?.length,
          );
          const expander = hasEvidence
            ? expandedEntryStrategiesRef.current.has(data.strategy)
              ? '▾ '
              : '▸ '
            : '';
          return expander + strategyLabel(data.strategy);
        },
        cellRenderer: ({ data }: ICellRendererParams<StrategyWinRateRow>) => {
          if (!data) return null;
          if (data.rowType === 'symbol') {
            return (
              <span className="auto-trader-entry-evidence-symbol-label">
                ↳ {data.symbol}
              </span>
            );
          }
          const label = strategyLabel(data.strategy);
          if (!entryProfilesByStrategyRef.current.get(data.strategy)?.length)
            return (
              <span className="auto-trader-strategy-label">
                <span aria-hidden="true" />
                {label}
              </span>
            );
          const expanded = expandedEntryStrategiesRef.current.has(
            data.strategy,
          );
          return (
            <button
              type="button"
              className="auto-trader-entry-evidence-expander-button"
              aria-expanded={expanded}
              aria-label={`${expanded ? 'Collapse' : 'Expand'} ${label} symbol entry evidence`}
            >
              <span aria-hidden="true">{expanded ? '▾' : '▸'}</span>
              {label}
            </button>
          );
        },
        headerClass: 'strategy-column-strategy-header strategy-group-end',
        cellClass: 'strategy-column-strategy-cell strategy-group-end',
        cellStyle: { backgroundColor: '#15181d' },
        width: 240,
        minWidth: 170,
        flex: 0,
      },
      {
        headerName: 'Source-reported today P&L',
        headerClass: 'strategy-group-performance',
        children: colorStrategyColumns(
          [
            {
              field: 'todayRealizedUsd',
              headerName: 'Today P&L',
              headerTooltip:
                'Today’s realized P&L for this strategy. Includes partial exits from positions that are still open; excludes unrealized P&L. Converted using the latest quote currency rates.',
              cellClassRules: signed,
              valueFormatter: ({ value }) =>
                typeof value === 'number' && Number.isFinite(value)
                  ? usd.format(value)
                  : '—',
              width: 78,
              minWidth: 78,
              maxWidth: 86,
              flex: 0,
            },
          ],
          'performance',
        ),
      },
      entryEvidenceGroup,
      {
        headerName: 'Closed audit outcome rows by source',
        headerClass: 'strategy-group-sample',
        children: colorStrategyColumns(
          [
            {
              field: 'trades',
              headerName: 'Trades',
              headerTooltip:
                'Raw count of fully closed outcomes in the AutoTrader audit-derived outcomes model. These rows are not independently reconciled performance samples.',
              width: 58,
              minWidth: 58,
              maxWidth: 64,
              flex: 0,
            },
            ...(
              [
                ['strictTrades', 'Strict'],
                ['exploratoryTrades', 'Exploratory'],
                ['scoutTrades', 'Scout'],
                ['paperCarryTrades', 'Carry'],
                ['unknownSourceTrades', 'Unknown'],
              ] as const
            ).map(([field, headerName]) => ({
              field,
              headerName,
              headerTooltip: `Raw closed outcome rows classified as ${headerName.toLowerCase()} by the existing outcomes source contract.`,
              width: 76,
              minWidth: 68,
              maxWidth: 84,
              flex: 0,
            })),
          ],
          'sample',
        ),
      },
      {
        headerName: 'Strategy status',
        headerClass: 'strategy-group-status',
        children: colorStrategyColumns(
          [
            {
              field: 'enabled',
              headerName: 'On',
              cellRenderer: ({ data }: { data?: StrategyWinRateRow }) =>
                data ? (
                  <input
                    type="checkbox"
                    aria-label={'Enable ' + strategyLabel(data.strategy)}
                    checked={data.enabled === true}
                    disabled={
                      settingsRef.current.loading || settingsRef.current.busy
                    }
                    onClick={(event) => event.stopPropagation()}
                    onChange={(event) => {
                      const currentSettings = settingsRef.current;
                      const strategies = event.target.checked
                        ? [
                            ...currentSettings.strategies,
                            data.strategy as Strategy,
                          ]
                        : currentSettings.strategies.filter(
                            (item) => item !== data.strategy,
                          );
                      settingsModel.setStrategies(strategies);
                      void settingsModel.saveLimits();
                    }}
                  />
                ) : null,
              width: 42,
              minWidth: 42,
              maxWidth: 48,
              flex: 0,
            },
            {
              field: 'scoutEnabled',
              headerName: 'Scout',
              headerTooltip:
                'Evaluate this strategy through Scout samples. Scout selection is independent of trading enablement.',
              cellRenderer: ({ data }: { data?: StrategyWinRateRow }) =>
                data ? (
                  <input
                    type="checkbox"
                    aria-label={
                      'Enable Scout for ' + strategyLabel(data.strategy)
                    }
                    checked={data.scoutEnabled === true}
                    disabled={
                      !data.scoutable ||
                      settingsRef.current.loading ||
                      settingsRef.current.busy
                    }
                    onClick={(event) => event.stopPropagation()}
                    onChange={(event) => {
                      const currentSettings = settingsRef.current;
                      const strategies = event.target.checked
                        ? [
                            ...currentSettings.scoutStrategies,
                            data.strategy as Strategy,
                          ]
                        : currentSettings.scoutStrategies.filter(
                            (item) => item !== data.strategy,
                          );
                      settingsModel.setScoutStrategies(strategies);
                      void settingsModel.saveLimits();
                    }}
                  />
                ) : null,
              width: 52,
              minWidth: 52,
              maxWidth: 58,
              flex: 0,
            },
          ],
          'status',
        ),
      },
      {
        headerName: 'Trade profile',
        headerClass: 'strategy-group-profile',
        children: colorStrategyColumns(
          [
            {
              field: 'medianHoldingMs',
              headerName: 'Hold M/A',
              headerTooltip:
                'Median and average time from entry to final exit.',
              valueFormatter: ({ value, data }) =>
                typeof value === 'number' &&
                typeof data?.averageHoldingMs === 'number'
                  ? durationLabel(value) +
                    ' / ' +
                    durationLabel(data.averageHoldingMs)
                  : '—',
              width: 82,
              minWidth: 82,
              maxWidth: 92,
              flex: 0,
            },
          ],
          'profile',
        ),
      },
      {
        headerName: 'Learning evidence · separate from paper outcomes',
        headerClass: 'strategy-group-learning',
        children: colorStrategyColumns(
          [
            {
              field: 'learningAverageNet',
              headerName: 'Trade net',
              headerTooltip:
                'Average net return from closed trades received by Learning in the last 7 days, pooled across symbols. Scout counterfactuals are reported separately.',
              cellClassRules: signed,
              valueFormatter: ({ value }) => pct(value),
              width: 72,
              minWidth: 72,
              maxWidth: 82,
              flex: 0,
            },
            {
              field: 'learningSamples',
              headerName: 'Trade n',
              headerTooltip:
                'Closed trade outcomes received by Learning in the last 7 days, pooled across symbols.',
              valueFormatter: ({ value }) =>
                typeof value === 'number' && Number.isFinite(value)
                  ? value.toLocaleString()
                  : '—',
              width: 60,
              minWidth: 60,
              maxWidth: 68,
              flex: 0,
            },
            {
              field: 'feedbackMultiplier',
              headerName: 'Feedback ×',
              headerTooltip:
                'Learning feedback multiplier from closed trade outcomes. Zero means new live entries are paused unless market-specific Scout evidence supports a capped probe.',
              valueFormatter: ({ value }) =>
                typeof value === 'number' && Number.isFinite(value)
                  ? '×' + value.toFixed(2)
                  : '—',
              width: 78,
              minWidth: 74,
              maxWidth: 84,
              flex: 0,
            },
            {
              field: 'effectiveSizeMultiplier',
              headerName: 'Size ×',
              headerTooltip:
                'Multiplier currently applied to this strategy’s effective budget. During recovery, a positive market-specific Scout profile may receive a capped probe size.',
              valueFormatter: ({ value }) =>
                typeof value === 'number' && Number.isFinite(value)
                  ? '×' + value.toFixed(2)
                  : '—',
              width: 70,
              minWidth: 66,
              maxWidth: 76,
              flex: 0,
            },
            {
              field: 'scoutOutcomeAverageNet',
              headerName: 'Scout net',
              headerTooltip:
                'Average Scout outcome return for today, including paired parameter comparisons. Paired comparison returns are also summarized separately.',
              cellClassRules: signed,
              valueFormatter: ({ value }) => pct(value),
              width: 72,
              minWidth: 72,
              maxWidth: 82,
              flex: 0,
            },
            {
              field: 'pairedScoutAverageLift',
              headerName: 'Paired lift',
              headerTooltip:
                'Average Scout variant-minus-baseline net return for paired parameter comparisons today.',
              cellClassRules: signed,
              valueFormatter: ({ value }) => pct(value),
              width: 76,
              minWidth: 76,
              maxWidth: 86,
              flex: 0,
            },
            {
              field: 'pairedScoutSamples',
              headerName: 'Pairs n',
              headerTooltip:
                'Paired Scout parameter comparison outcomes today, pooled across symbols.',
              valueFormatter: ({ value }) =>
                typeof value === 'number' && Number.isFinite(value)
                  ? value.toLocaleString()
                  : '—',
              width: 60,
              minWidth: 60,
              maxWidth: 68,
              flex: 0,
            },
          ],
          'learning',
        ),
      },
    ],
    [settingsModel],
  );
  return (
    <section
      className="feed-section auto-trader-outcomes-embedded v35-operator-card v35-strategy-performance"
      aria-label="Strategy performance"
    >
      <div className="v35-strategy-performance__heading">
        <OperatorStatus tone={state.loading ? 'warning' : 'neutral'}>
          {state.loading
            ? 'Refreshing outcomes'
            : viewState.sourceView === 'scouts'
              ? `${scoutOutcomeCount.toLocaleString()} raw Scout outcomes`
              : `${closedRows.length} closed audit rows`}
        </OperatorStatus>
      </div>
      <OperatorTabBar
        label="Performance evidence source"
        selectedId={viewState.sourceView}
        onSelect={(id) =>
          model.setSourceView(id as typeof viewState.sourceView)
        }
        tabs={[
          { id: 'autotrader', label: 'AutoTrader' },
          { id: 'scouts', label: 'Scouts' },
        ]}
      />
      <OperatorMetricStrip
        label="Strategy performance summary"
        metrics={
          viewState.sourceView === 'scouts'
            ? [
                {
                  label: 'Scout outcomes',
                  value: scoutOutcomeCount.toLocaleString(),
                },
                {
                  label: 'Positive outcomes',
                  value: scoutPositiveCount.toLocaleString(),
                },
                { label: 'Accepted gain', value: 'Unavailable' },
                { label: 'Effective N', value: 'Unavailable' },
                { label: 'Paper net P&L', value: 'Not Scout evidence' },
              ]
            : [
                {
                  label: 'Closed audit rows',
                  value: closedRows.length.toLocaleString(),
                },
                { label: 'Reconciled net P&L', value: 'Unavailable' },
                { label: 'Win rate', value: 'Unavailable' },
                { label: 'Profit factor', value: 'Unavailable' },
                { label: 'Expectancy (R)', value: 'Unavailable' },
              ]
        }
      />
      <OperatorTabBar
        label="Strategy performance views"
        selectedId={viewState.view}
        onSelect={(id) => model.setView(id as typeof viewState.view)}
        tabs={[
          { id: 'performance', label: 'Performance' },
          { id: 'heatmap', label: 'Heatmap' },
          { id: 'history', label: 'History' },
          { id: 'exit-policies', label: 'Learned exit policies' },
        ]}
      />
      {viewState.view === 'performance' &&
        (viewState.sourceView === 'scouts' ? (
          <>
            <CompactTable label="Raw Scout outcome counts by strategy">
              <thead>
                <tr>
                  <th scope="col">Strategy</th>
                  <th scope="col">Raw outcomes</th>
                  <th scope="col">Positive outcomes</th>
                  <th scope="col">Net P&amp;L</th>
                  <th scope="col">Effective N</th>
                </tr>
              </thead>
              <tbody>
                {scoutEvidenceRows.map((row) => (
                  <tr key={row.strategy}>
                    <th scope="row">
                      {strategyLabel(row.strategy as Strategy)}
                    </th>
                    <td>{row.outcomes}</td>
                    <td>{row.positiveOutcomes}</td>
                    <td>Unavailable · Scout evidence</td>
                    <td>Unavailable</td>
                  </tr>
                ))}
                {scoutEvidenceRows.length === 0 && (
                  <tr>
                    <td colSpan={5}>No Scout outcome rows returned.</td>
                  </tr>
                )}
              </tbody>
            </CompactTable>
            <p className="auto-trader-strategy-winrate-note">
              Scout observations are separate research evidence, not executed
              AutoTrader trades. Raw counts do not represent accepted
              independent gains, effective sample size or promotion eligibility.
            </p>
          </>
        ) : (
          <>
            <CompactTable label="Strategy financial performance evidence">
              <thead>
                <tr>
                  <th scope="col">Strategy</th>
                  <th scope="col">Closed audit rows</th>
                  <th scope="col">Net P&amp;L</th>
                  <th scope="col">Win %</th>
                  <th scope="col">PF</th>
                  <th scope="col">Payoff</th>
                  <th scope="col">Avg R</th>
                  <th scope="col">Effective N</th>
                </tr>
              </thead>
              <tbody>
                {coverageByStrategy.map(({ strategy, closedAuditRows }) => (
                  <tr key={strategy}>
                    <th scope="row">{strategyLabel(strategy as Strategy)}</th>
                    <td>{closedAuditRows.toLocaleString()}</td>
                    <td>Unavailable</td>
                    <td>Unavailable</td>
                    <td>Unavailable</td>
                    <td>Unavailable</td>
                    <td>Unavailable</td>
                    <td>Unavailable</td>
                  </tr>
                ))}
                {coverageByStrategy.length === 0 && (
                  <tr>
                    <td colSpan={8}>No closed audit outcome rows.</td>
                  </tr>
                )}
              </tbody>
            </CompactTable>
            <div className="v35-strategy-performance__visuals">
              <section aria-label="Winner and loser analysis">
                <strong>Net winners / losers</strong>
                <p>
                  Unavailable · no independently reconciled cost-complete
                  outcome distribution is exposed.
                </p>
              </section>
              <section aria-label="Cumulative net profit and loss">
                <strong>Cumulative net P&amp;L</strong>
                <p>
                  Unavailable · no continuous independently reconciled strategy
                  series is exposed.
                </p>
              </section>
            </div>
            <p className="auto-trader-strategy-winrate-note" role="status">
              Closed rows are audit-derived source records, not verified
              independent trades. Financial metrics, effective sample size, and
              performance ranking remain unavailable until authoritative
              cost-complete reconciliation is connected.
            </p>
          </>
        ))}
      {viewState.view === 'heatmap' &&
        (viewState.sourceView === 'scouts' ? (
          <p className="auto-trader-strategy-winrate-note">
            Symbol-level Scout outcome coverage is unavailable in the current
            Learning summary contract.
          </p>
        ) : (
          <div>
            <p className="auto-trader-strategy-winrate-note">
              Closed audit-row coverage only. Cell intensity represents returned
              row count; it does not represent financial performance,
              eligibility or independent sample size.
            </p>
            <CompactTable label="Closed audit row coverage heatmap by strategy and symbol">
              <thead>
                <tr>
                  <th scope="col">Strategy</th>
                  {coverageHeatmap.symbols.map((symbol) => (
                    <th scope="col" key={symbol}>
                      {symbol}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {coverageHeatmap.rows.map((row) => (
                  <tr key={row.strategy}>
                    <th scope="row">
                      {strategyLabel(row.strategy as Strategy)}
                    </th>
                    {row.cells.map((cell) => (
                      <td key={cell.symbol} data-intensity={cell.intensity}>
                        {cell.closedAuditRows || '—'}
                      </td>
                    ))}
                  </tr>
                ))}
                {coverageHeatmap.rows.length === 0 && (
                  <tr>
                    <td
                      colSpan={Math.max(1, coverageHeatmap.symbols.length + 1)}
                    >
                      No closed audit coverage.
                    </td>
                  </tr>
                )}
              </tbody>
            </CompactTable>
          </div>
        ))}
      {viewState.view === 'history' &&
        (viewState.sourceView === 'scouts' ? (
          <p className="auto-trader-strategy-winrate-note">
            Row-level Scout history is unavailable in the current aggregate
            Learning summary contract.
          </p>
        ) : (
          <CompactTable label="Recent closed audit outcome history">
            <thead>
              <tr>
                <th scope="col">Closed</th>
                <th scope="col">Symbol</th>
                <th scope="col">Strategy</th>
                <th scope="col">Source</th>
                <th scope="col">Profile</th>
                <th scope="col">Financial result</th>
              </tr>
            </thead>
            <tbody>
              {closedHistory.map((row) => (
                <tr key={row.rowId}>
                  <td>
                    {row.closedAt
                      ? new Date(row.closedAt).toLocaleString()
                      : 'Unavailable'}
                  </td>
                  <td>{row.symbol}</td>
                  <td>{strategyLabel(row.strategy as Strategy)}</td>
                  <td>{row.source}</td>
                  <td>
                    {row.entryProfileId
                      ? `${row.entryProfileId}${row.entryProfileVersion === null ? '' : ` · v${row.entryProfileVersion}`}`
                      : 'Unavailable'}
                  </td>
                  <td>Unavailable pending independent reconciliation</td>
                </tr>
              ))}
              {closedHistory.length === 0 && (
                <tr>
                  <td colSpan={6}>No closed audit history.</td>
                </tr>
              )}
            </tbody>
          </CompactTable>
        ))}
      {viewState.view === 'exit-policies' && (
        <CompactTable label="Strategy profile exit parameters and validation provenance">
          <thead>
            <tr>
              <th scope="col">Symbol</th>
              <th scope="col">Strategy</th>
              <th scope="col">Profile</th>
              <th scope="col">Exit parameters reported by profile</th>
              <th scope="col">Effective N</th>
              <th scope="col">Validation</th>
            </tr>
          </thead>
          <tbody>
            {profileRows.map((profile) => (
              <tr key={`${profile.symbol}:${profile.strategy}`}>
                <td>{profile.symbol}</td>
                <td>{strategyLabel(profile.strategy as Strategy)}</td>
                <td>{profile.profile ?? 'Unavailable'}</td>
                <td>
                  {profile.parameters.length
                    ? profile.parameters
                        .map(
                          (parameter) =>
                            `${parameter.label}: ${parameter.value === null ? 'Unavailable' : `${parameter.value} ${parameter.unit}`}`,
                        )
                        .join(' · ')
                    : 'No exit parameters reported'}
                </td>
                <td>{profile.effectiveSamples ?? 'Unavailable'}</td>
                <td>
                  {profile.validation === 'validated'
                    ? 'Validated by complete returned evidence'
                    : profile.validation === 'not-validated'
                      ? 'Not validated / incomplete evidence'
                      : 'Unavailable'}
                </td>
              </tr>
            ))}
            {profileRows.length === 0 && (
              <tr>
                <td colSpan={6}>No strategy profile snapshot is available.</td>
              </tr>
            )}
          </tbody>
        </CompactTable>
      )}
      <details className="v35-strategy-performance__diagnostics">
        <summary>Entry, status and Learning diagnostics</summary>
        <div className="auto-trader-strategy-winrate-heading">
          <strong className="auto-trader-strategy-winrate-title">
            Strategy performance evidence
          </strong>
          <label>
            <input
              type="checkbox"
              checked={hideDisabledStrategies}
              onChange={(event) =>
                updateHideDisabledStrategies(event.target.checked)
              }
            />{' '}
            Hide disabled strategies
          </label>
        </div>
        <div
          className="auto-trader-strategy-column-group-controls"
          aria-label="Visible column groups"
        >
          {STRATEGY_COLUMN_GROUPS.map(({ id, label }) => (
            <label key={id}>
              <input
                type="checkbox"
                checked={visibleColumnGroups[id]}
                onChange={(event) =>
                  updateColumnGroupVisibility(id, event.target.checked)
                }
              />{' '}
              {label}
            </label>
          ))}
        </div>
        <div
          className="auto-trader-strategy-winrate-note"
          role="status"
          aria-label="Strategy performance evidence status"
        >
          <strong>
            {state.loading
              ? 'Loading audit outcome evidence.'
              : evidenceSummary.auditClosedRows === 0
                ? 'No fully closed audit outcomes are available.'
                : `${evidenceSummary.auditClosedRows} fully closed audit outcome row${evidenceSummary.auditClosedRows === 1 ? '' : 's'} available.`}
          </strong>{' '}
          Source rows: strict {evidenceSummary.strictRows}, exploratory{' '}
          {evidenceSummary.exploratoryRows}, Scout {evidenceSummary.scoutRows},
          carry {evidenceSummary.paperCarryRows}, unknown{' '}
          {evidenceSummary.unknownSourceRows}. Independent financial
          reconciliation is not exposed by the current outcomes contract, so no
          validated profitability, expectancy, effective sample size, or
          strategy ranking is reported here. Learning and Scout evidence below
          remain separate from actual paper-trade outcome rows.
        </div>
        <p className="auto-trader-strategy-winrate-note">
          “Today P&amp;L” is the source-reported AutoTrader realized-P&amp;L
          field; it is not an independently reconciled figure. Raw closed counts
          include only closed audit-derived outcomes. Strict, exploratory,
          Scout, carry, and unknown source classifications are shown separately.
        </p>
        <div
          ref={strategyGridWrapper}
          className="data-grid-wrap signal-info-grid auto-trader-strategy-winrate-grid"
        >
          <AgGridReact<StrategyWinRateRow>
            rowData={strategyGridRows}
            columnDefs={strategyColumns}
            onRowClicked={({ data }) => {
              if (!data || data.rowType === 'symbol') return;
              if (!entryReturnProfilesByStrategy.get(data.strategy)?.length)
                return;
              model.toggleExpandedEntryStrategy(data.strategy);
            }}
            postSortRows={({ nodes }) => {
              const symbolRowsByStrategy = new Map<string, typeof nodes>();
              for (const node of nodes) {
                if (node.data?.rowType !== 'symbol') continue;
                const strategyRows =
                  symbolRowsByStrategy.get(node.data.strategy) ?? [];
                strategyRows.push(node);
                symbolRowsByStrategy.set(node.data.strategy, strategyRows);
              }
              const rowsInMasterDetailOrder = nodes
                .filter((node) => node.data?.rowType !== 'symbol')
                .flatMap((node) => [
                  node,
                  ...(node.data
                    ? (symbolRowsByStrategy.get(node.data.strategy) ?? [])
                    : []),
                ]);
              nodes.splice(0, nodes.length, ...rowsInMasterDetailOrder);
            }}
            rowClassRules={{
              'auto-trader-entry-evidence-master-row': ({ data }) =>
                data?.rowType === 'strategy' &&
                Boolean(
                  entryReturnProfilesByStrategy.get(data.strategy)?.length,
                ),
              'auto-trader-entry-evidence-symbol-row': ({ data }) =>
                data?.rowType === 'symbol',
            }}
            onGridReady={({ api }) => {
              strategyGridApi.current = api;
              try {
                const saved = viewState.gridColumnStateJson;
                if (saved) {
                  const parsed = JSON.parse(saved);
                  if (Array.isArray(parsed))
                    api.applyColumnState({
                      state: parsed.map((column) => ({ ...column, flex: 0 })),
                      applyOrder: true,
                    });
                }
              } catch {
                // Ignore invalid or unavailable saved grid state.
              }
              for (const group of STRATEGY_COLUMN_GROUPS) {
                api.setColumnsVisible(
                  group.children,
                  visibleColumnGroups[group.id],
                );
              }
            }}
            onColumnResized={({ api, finished }) => {
              if (finished) saveStrategyGridState(api);
            }}
            onColumnMoved={({ api, finished }) => {
              if (finished) saveStrategyGridState(api);
            }}
            onColumnVisible={({ api }) => {
              saveStrategyGridState(api);
            }}
            onColumnPinned={({ api }) => saveStrategyGridState(api)}
            onSortChanged={({ api }) => saveStrategyGridState(api)}
            getRowId={strategyGridGetRowId}
            defaultColDef={strategyGridDefaultColDef}
            maintainColumnOrder
            theme={theme}
            domLayout="normal"
            rowHeight={26}
            headerHeight={28}
          />
        </div>
        {visibleStrategyWinRates.length === 0 && !state.loading && (
          <p className="positions-empty">
            {hideDisabledStrategies
              ? 'No enabled strategies to show.'
              : 'No closed trades to calculate strategy win rates yet.'}
          </p>
        )}
      </details>
    </section>
  );
}
