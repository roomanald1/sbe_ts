import { useEffect, useMemo, useRef, useState } from 'react';
import { AgGridReact } from 'ag-grid-react';
import {
  ClientSideRowModelModule,
  ModuleRegistry,
  themeQuartz,
  type ColDef,
  type ColGroupDef,
  type GridApi,
} from 'ag-grid-community';
import {
  AutoTraderOutcomesModel,
  type AutoTraderOutcome,
  type StrictEntryGateSummary,
} from '../models/AutoTraderOutcomesModel';
import { autoTraderScoutModel } from '../models/AutoTraderScoutModel';
import {
  sharedAutoTraderSettingsModel,
  type Strategy,
} from '../models/AutoTraderModel';
import { useBehaviorSnapshot } from '@workspace/ui-platform';
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
  data.strategy;
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
const reasonTimestamp = (timestamp: number | undefined) =>
  timestamp && Number.isFinite(timestamp)
    ? new Date(timestamp).toLocaleString()
    : 'Timestamp unavailable';
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
  strategy: string;
  enabled?: boolean;
  scoutEnabled?: boolean;
  scoutable?: boolean;
  runStatus: string;
  runReason: string;
  runReasonAt?: number;
  trades: number;
  wins: number;
  losses: number;
  flat: number;
  feesUsd?: number;
  netPnlUsd?: number;
  todayRealizedUsd?: number;
  winRate?: number;
  averageReturn?: number;
  averageNetWinUsd?: number;
  averageNetWinPercent?: number;
  averageNetLossUsd?: number;
  averageNetLossPercent?: number;
  profitFactor?: number;
  medianHoldingMs?: number;
  averageHoldingMs?: number;
  periodStart: number;
  periodEnd: number;
  averageEntrySizeUsd?: number;
  lowSample: boolean;
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
};
const allStrategies: Strategy[] = [
  'live-momentum',
  'live-trade-flow',
  'live-order-book',
  'live-vwap-dislocation',
  'live-absorption-pressure',
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
const HIDE_DISABLED_STRATEGIES_KEY = 'autotrader-hide-disabled-strategies-v1';
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

export function AutoTraderStrategyPerformanceWidget() {
  const model = useMemo(() => sharedOutcomesModel, []);
  const state = useBehaviorSnapshot(model.state$);
  const settingsModel = useMemo(() => sharedAutoTraderSettingsModel, []);
  const settings = useBehaviorSnapshot(settingsModel.state$);
  const scoutState = useBehaviorSnapshot(autoTraderScoutModel.state$);
  const strategyGridApi = useRef<GridApi<StrategyWinRateRow> | null>(null);
  const strategyGridWrapper = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    void model.start();
    return () => model.stop();
  }, [model]);
  useEffect(() => {
    settingsModel.start();
    return () => settingsModel.stop();
  }, [settingsModel]);
  useEffect(() => {
    autoTraderScoutModel.start();
    return () => autoTraderScoutModel.stop();
  }, []);
  useEffect(() => {
    const wrapper = strategyGridWrapper.current;
    if (!wrapper) return;
    wrapper.addEventListener('wheel', scrollPageFromStrategyGrid, {
      passive: false,
    });
    return () =>
      wrapper.removeEventListener('wheel', scrollPageFromStrategyGrid);
  }, []);
  useEffect(() => {
    const timer = setInterval(
      () =>
        strategyGridApi.current?.refreshCells({
          columns: ['runReason'],
          force: true,
        }),
      15_000,
    );
    return () => clearInterval(timer);
  }, []);
  const [hideDisabledStrategies, setHideDisabledStrategies] = useState(() => {
    try {
      return localStorage.getItem(HIDE_DISABLED_STRATEGIES_KEY) === 'true';
    } catch {
      return false;
    }
  });
  const updateHideDisabledStrategies = (value: boolean) => {
    setHideDisabledStrategies(value);
    try {
      localStorage.setItem(HIDE_DISABLED_STRATEGIES_KEY, String(value));
    } catch {
      /* The filter still works if browser storage is unavailable. */
    }
  };
  const rows = state.rows;
  const closedRows = useMemo(
    () => rows.filter((row) => row.status === 'Closed'),
    [rows],
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
      const cohort = cohorts.get(outcome.strategy) ?? {
        positive: 0,
        count: 0,
        returnTotal: 0,
      };
      cohort.count += 1;
      cohort.returnTotal += outcome.netReturnPercent;
      if (outcome.netReturnPercent > 0) cohort.positive += 1;
      cohorts.set(outcome.strategy, cohort);
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
        const fees = trades.map((row) => row.feesUsd);
        const netValues = trades.map((row) => row.netPnlUsd);
        const grossValues = trades.map((row) =>
          typeof row.quoteUsdRate === 'number' &&
          Number.isFinite(row.quoteUsdRate) &&
          typeof row.feesUsd === 'number' &&
          Number.isFinite(row.feesUsd)
            ? row.pnl * row.quoteUsdRate + row.feesUsd
            : undefined,
        );
        const netComplete =
          trades.length > 0 &&
          netValues.every(
            (value): value is number =>
              typeof value === 'number' && Number.isFinite(value),
          );
        const grossComplete =
          trades.length > 0 &&
          grossValues.every(
            (value): value is number =>
              typeof value === 'number' && Number.isFinite(value),
          );
        const outcomes = netComplete ? netValues : trades.map((row) => row.pnl);
        const wins = outcomes.filter((value) => value > 0).length;
        const losses = outcomes.filter((value) => value < 0).length;
        const netSamples = netComplete
          ? trades.flatMap((trade, index) => {
              const value = netValues[index];
              const capital =
                trade.entryNotionalUsd ??
                trade.quantity * trade.entryPrice * (trade.quoteUsdRate ?? 1);
              return typeof value === 'number' &&
                Number.isFinite(capital) &&
                capital > 0
                ? [{ value, returnPercent: (value / capital) * 100 }]
                : [];
            })
          : [];
        const netWins = netSamples.filter((sample) => sample.value > 0);
        const netLosses = netSamples.filter((sample) => sample.value < 0);
        const profitTotal = grossComplete
          ? grossValues
              .filter((value) => value > 0)
              .reduce((sum, value) => sum + value, 0)
          : undefined;
        const lossTotal = grossComplete
          ? Math.abs(
              grossValues
                .filter((value) => value < 0)
                .reduce((sum, value) => sum + value, 0),
            )
          : undefined;
        const holdingTimes = trades
          .map((row) => (row.closedAt ?? 0) - row.openedAt)
          .filter((value) => Number.isFinite(value) && value >= 0);
        const entrySizes = trades.map((row) =>
          typeof row.quoteUsdRate === 'number' &&
          Number.isFinite(row.quoteUsdRate)
            ? Math.abs(row.quantity * row.entryPrice * row.quoteUsdRate)
            : undefined,
        );
        const validEntrySizes = entrySizes.filter(
          (value): value is number =>
            typeof value === 'number' && Number.isFinite(value),
        );
        const averageReturn = trades.length
          ? netComplete
            ? netSamples.reduce(
                (sum, sample) => sum + sample.returnPercent,
                0,
              ) / trades.length
            : trades.reduce(
                (sum, row) =>
                  sum + (row.pnl / (row.quantity * row.entryPrice)) * 100,
                0,
              ) / trades.length
          : undefined;
        const closedTimes = trades
          .map((row) => row.closedAt ?? row.openedAt)
          .filter((value) => value > 0 && Number.isFinite(value));
        const firstClosed = closedTimes.length ? Math.min(...closedTimes) : 0;
        const lastClosed = closedTimes.length ? Math.max(...closedTimes) : 0;
        const totalNet = netComplete
          ? netValues.reduce((sum, value) => sum + value, 0)
          : undefined;
        const mean = (values: number[]) =>
          values.length
            ? values.reduce((sum, value) => sum + value, 0) / values.length
            : undefined;
        const learning = state.learningSummary[strategy];
        return {
          strategy,
          trades: trades.length,
          wins,
          losses,
          flat: trades.length - wins - losses,
          enabled,
          runStatus,
          runReason,
          runReasonAt,
          todayRealizedUsd:
            state.todayStrategyRealizedUsd?.[strategy] ??
            (state.todayStrategyRealizedUsd ? 0 : undefined),
          feesUsd:
            trades.length > 0 &&
            fees.every(
              (value): value is number =>
                typeof value === 'number' && Number.isFinite(value),
            )
              ? fees.reduce((sum, value) => sum + value, 0)
              : undefined,
          netPnlUsd: totalNet,
          winRate: trades.length ? (wins / trades.length) * 100 : undefined,
          averageReturn,
          averageNetWinUsd: netComplete
            ? mean(netWins.map((sample) => sample.value))
            : undefined,
          averageNetWinPercent: netComplete
            ? mean(netWins.map((sample) => sample.returnPercent))
            : undefined,
          averageNetLossUsd: netComplete
            ? mean(netLosses.map((sample) => sample.value))
            : undefined,
          averageNetLossPercent: netComplete
            ? mean(netLosses.map((sample) => sample.returnPercent))
            : undefined,
          profitFactor:
            profitTotal !== undefined && lossTotal !== undefined
              ? lossTotal > 0
                ? profitTotal / lossTotal
                : profitTotal > 0
                  ? Number.POSITIVE_INFINITY
                  : undefined
              : undefined,
          medianHoldingMs: holdingTimes.length
            ? median(holdingTimes)
            : undefined,
          averageHoldingMs: mean(holdingTimes),
          periodStart: firstClosed,
          periodEnd: lastClosed,
          averageEntrySizeUsd:
            validEntrySizes.length === trades.length
              ? mean(validEntrySizes)
              : undefined,
          lowSample: trades.length > 0 && trades.length < 30,
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
      .sort(
        (a, b) => b.trades - a.trades || a.strategy.localeCompare(b.strategy),
      );
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
  const strategyColumns = useMemo<
    (ColDef<StrategyWinRateRow> | ColGroupDef<StrategyWinRateRow>)[]
  >(
    () => [
      {
        field: 'strategy',
        headerName: 'Strategy',
        valueFormatter: ({ value }) => strategyLabel(value),
        headerClass: 'strategy-column-strategy-header strategy-group-end',
        cellClass: 'strategy-column-strategy-cell strategy-group-end',
        cellStyle: { backgroundColor: '#15181d' },
        minWidth: 170,
      },
      {
        headerName: 'Performance',
        headerClass: 'strategy-group-performance',
        children: colorStrategyColumns(
          [
            {
              field: 'feesUsd',
              headerName: 'Fees',
              headerTooltip:
                'Entry and exit fees for fully closed trades, converted using the latest quote currency rate.',
              cellClassRules: {
                'pnl-negative': (p) =>
                  typeof p.value === 'number' && p.value > 0,
              },
              valueFormatter: ({ value }) =>
                typeof value === 'number' && Number.isFinite(value)
                  ? usd.format(value)
                  : '—',
              width: 64,
              minWidth: 64,
              maxWidth: 72,
              flex: 0,
            },
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
            {
              field: 'netPnlUsd',
              headerName: 'Closed net',
              headerTooltip:
                'Realized P&L after fees for fully closed, audited trades in the available strategy sample. Partial exits from open trades are excluded.',
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
            {
              field: 'winRate',
              headerName: 'Win%',
              sort: 'desc',
              valueFormatter: ({ value }) =>
                typeof value === 'number' ? value.toFixed(1) + '%' : '—',
              width: 55,
              minWidth: 55,
              maxWidth: 62,
              flex: 0,
            },
            {
              field: 'averageReturn',
              headerName: 'Ret%',
              cellClassRules: signed,
              valueFormatter: ({ value }) => pct(value),
              width: 58,
              minWidth: 58,
              maxWidth: 65,
              flex: 0,
            },
          ],
          'performance',
        ),
      },
      {
        headerName: 'Fully closed trades',
        headerClass: 'strategy-group-sample',
        children: colorStrategyColumns(
          [
            {
              field: 'trades',
              headerName: 'Trades',
              headerTooltip:
                'Counts only fully closed, audited trades. Partial exits from positions that remain open do not count.',
              width: 58,
              minWidth: 58,
              maxWidth: 64,
              flex: 0,
            },
            {
              field: 'wins',
              headerName: 'Wins',
              width: 46,
              minWidth: 46,
              maxWidth: 52,
              flex: 0,
            },
            {
              field: 'losses',
              headerName: 'Losses',
              width: 50,
              minWidth: 50,
              maxWidth: 56,
              flex: 0,
            },
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
                    disabled={settings.loading || settings.busy}
                    onClick={(event) => event.stopPropagation()}
                    onChange={(event) => {
                      const strategies = event.target.checked
                        ? [...settings.strategies, data.strategy as Strategy]
                        : settings.strategies.filter(
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
                      !data.scoutable || settings.loading || settings.busy
                    }
                    onClick={(event) => event.stopPropagation()}
                    onChange={(event) => {
                      const strategies = event.target.checked
                        ? [
                            ...settings.scoutStrategies,
                            data.strategy as Strategy,
                          ]
                        : settings.scoutStrategies.filter(
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
            {
              field: 'runReason',
              headerName: 'Why no entry?',
              headerTooltip:
                'The latest rejection or entry evaluation explanation reported for this strategy.',
              valueFormatter: ({ value, data }) =>
                value
                  ? data?.runReasonAt
                    ? value + ' · ' + reasonAge(data.runReasonAt)
                    : value
                  : '—',
              tooltipValueGetter: ({ data }) =>
                data
                  ? data.runReasonAt
                    ? data.runReason +
                      ' · recorded ' +
                      reasonTimestamp(data.runReasonAt) +
                      ' (' +
                      reasonAge(data.runReasonAt) +
                      ')'
                    : data.runReason
                  : '',
              width: 300,
              minWidth: 240,
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
            {
              field: 'averageEntrySizeUsd',
              headerName: 'Entry USD',
              headerTooltip:
                'Mean USD notional at entry for fully closed trades; uses entry price and the latest quote currency conversion rate.',
              valueFormatter: ({ value }) =>
                typeof value === 'number' && Number.isFinite(value)
                  ? usd.format(value)
                  : '—',
              width: 72,
              minWidth: 72,
              maxWidth: 82,
              flex: 0,
            },
          ],
          'profile',
        ),
      },
      {
        headerName: 'Learning · all symbols · recent',
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
                'Average Scout outcome return in the last 7 days, including paired parameter comparisons. Paired comparison returns are also summarized separately.',
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
                'Average Scout variant-minus-baseline net return for paired parameter comparisons in the last 7 days.',
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
                'Paired Scout parameter comparison outcomes in the last 7 days, pooled across symbols.',
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
    [
      settings.busy,
      settings.loading,
      settings.scoutStrategies,
      settings.strategies,
      settingsModel,
    ],
  );
  return (
    <section
      className="feed-section auto-trader-outcomes-embedded"
      aria-label="Strategy performance"
    >
      <div className="auto-trader-strategy-winrate-heading">
        <strong className="auto-trader-strategy-winrate-title">
          Strategy performance and today’s realized P&amp;L
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
      <p className="auto-trader-strategy-winrate-note">
        Today P&amp;L includes all realized exits today, including partial exits
        on positions still open. Wins, losses, and Closed net use fully closed
        audited trades only. Unrealized P&amp;L is shown with open positions and
        is not included in Today P&amp;L.
      </p>
      <div
        ref={strategyGridWrapper}
        className="data-grid-wrap signal-info-grid auto-trader-strategy-winrate-grid"
      >
        <AgGridReact<StrategyWinRateRow>
          rowData={visibleStrategyWinRates}
          columnDefs={strategyColumns}
          onGridReady={({ api }) => {
            strategyGridApi.current = api;
          }}
          getRowId={strategyGridGetRowId}
          defaultColDef={strategyGridDefaultColDef}
          maintainColumnOrder
          theme={theme}
          domLayout="autoHeight"
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
    </section>
  );
}
