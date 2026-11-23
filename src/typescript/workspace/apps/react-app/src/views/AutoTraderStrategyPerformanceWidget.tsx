import { useEffect, useMemo, useRef, useState } from 'react';
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
  sharedAutoTraderSettingsModel,
  type AutoTraderStrategyProfile,
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
  wins: number;
  losses: number;
  flat: number;
  feesUsd?: number;
  netPnlUsd?: number;
  todayRealizedUsd?: number;
  todayWinRate?: number;
  todayAverageReturn?: number;
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
const HIDE_DISABLED_STRATEGIES_KEY = 'autotrader-hide-disabled-strategies-v1';
const STRATEGY_COLUMN_GROUPS = [
  {
    id: 'performance',
    label: 'Performance',
    children: ['todayRealizedUsd', 'todayWinRate', 'todayAverageReturn'],
  },
  {
    id: 'entryEvidence',
    label: 'Entry evidence',
    children: [
      'entryAverageNetReturn',
      'entryPositiveRate',
      'entrySampleCount',
      'entryRiskMultiplier',
      'entryWhyNoEntry',
    ],
  },
  {
    id: 'sample',
    label: 'Fully closed trades',
    children: ['trades', 'wins', 'losses'],
  },
  {
    id: 'status',
    label: 'Strategy status',
    children: ['enabled', 'scoutEnabled'],
  },
  {
    id: 'profile',
    label: 'Trade profile',
    children: ['medianHoldingMs', 'averageEntrySizeUsd'],
  },
  {
    id: 'learning',
    label: 'Learning',
    children: [
      'learningAverageNet',
      'learningSamples',
      'feedbackMultiplier',
      'effectiveSizeMultiplier',
      'scoutOutcomeAverageNet',
      'pairedScoutAverageLift',
      'pairedScoutSamples',
    ],
  },
];
const STRATEGY_COLUMN_GROUPS_KEY = 'autotrader-strategy-column-groups-v1';
const STRATEGY_GRID_STATE_KEY = 'autotrader-strategy-grid-state-v1';
const readStrategyColumnGroups = (): Record<string, boolean> => {
  try {
    const stored = localStorage.getItem(STRATEGY_COLUMN_GROUPS_KEY);
    return stored ? JSON.parse(stored) : {};
  } catch {
    return {};
  }
};
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
  const saveStrategyGridState = (api: GridApi<StrategyWinRateRow>) => {
    try {
      localStorage.setItem(
        STRATEGY_GRID_STATE_KEY,
        JSON.stringify(api.getColumnState()),
      );
    } catch {
      // Keep the grid usable when browser storage is unavailable.
    }
  };
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
  const [hideDisabledStrategies, setHideDisabledStrategies] = useState(() => {
    try {
      return localStorage.getItem(HIDE_DISABLED_STRATEGIES_KEY) === 'true';
    } catch {
      return false;
    }
  });
  const [visibleColumnGroups, setVisibleColumnGroups] = useState(() => {
    const stored = readStrategyColumnGroups();
    return Object.fromEntries(
      STRATEGY_COLUMN_GROUPS.map(({ id }) => [id, stored[id] !== false]),
    ) as Record<(typeof STRATEGY_COLUMN_GROUPS)[number]['id'], boolean>;
  });
  const updateColumnGroupVisibility = (
    id: (typeof STRATEGY_COLUMN_GROUPS)[number]['id'],
    visible: boolean,
  ) => {
    setVisibleColumnGroups((previous) => {
      const next = { ...previous, [id]: visible };
      try {
        localStorage.setItem(STRATEGY_COLUMN_GROUPS_KEY, JSON.stringify(next));
      } catch {
        /* The grid still updates if browser storage is unavailable. */
      }
      strategyGridApi.current?.setColumnsVisible(
        STRATEGY_COLUMN_GROUPS.find((group) => group.id === id)?.children ?? [],
        visible,
      );
      return next;
    });
  };
  useEffect(() => {
    const api = strategyGridApi.current;
    if (!api) return;
    for (const group of STRATEGY_COLUMN_GROUPS) {
      api.setColumnsVisible(group.children, visibleColumnGroups[group.id]);
    }
  }, [visibleColumnGroups]);
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
        const now = new Date();
        const todayStart = new Date(
          now.getFullYear(),
          now.getMonth(),
          now.getDate(),
        ).getTime();
        const todayEnd = todayStart + 24 * 60 * 60 * 1000;
        const todayTrades = trades.filter(
          (trade) =>
            typeof trade.closedAt === 'number' &&
            trade.closedAt >= todayStart &&
            trade.closedAt < todayEnd,
        );
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
        const todayWins = todayTrades.filter((trade) => {
          const value =
            typeof trade.netPnlUsd === 'number' &&
            Number.isFinite(trade.netPnlUsd)
              ? trade.netPnlUsd
              : trade.pnl;
          return value > 0;
        }).length;
        const todayAverageReturn = todayTrades.length
          ? todayTrades.reduce((sum, trade) => {
              const value =
                typeof trade.netPnlUsd === 'number' &&
                Number.isFinite(trade.netPnlUsd)
                  ? trade.netPnlUsd
                  : trade.pnl * (trade.quoteUsdRate ?? 1);
              const capital =
                trade.entryNotionalUsd ??
                trade.quantity * trade.entryPrice * (trade.quoteUsdRate ?? 1);
              return sum + (capital > 0 ? (value / capital) * 100 : 0);
            }, 0) / todayTrades.length
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
          rowId: `strategy:${strategy}`,
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
          todayWinRate: todayTrades.length
            ? (todayWins / todayTrades.length) * 100
            : undefined,
          todayAverageReturn,
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
  const [expandedEntryStrategies, setExpandedEntryStrategies] = useState<
    Set<string>
  >(() => new Set());
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
        headerName: 'Performance',
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
            {
              field: 'todayWinRate',
              headerName: 'Today Win%',
              headerTooltip: 'Win rate for fully closed trades closed today.',
              valueFormatter: ({ value }) =>
                typeof value === 'number' ? value.toFixed(1) + '%' : '—',
              width: 55,
              minWidth: 55,
              maxWidth: 62,
              flex: 0,
            },
            {
              field: 'todayAverageReturn',
              headerName: 'Today Ret%',
              headerTooltip:
                'Average net return for fully closed trades closed today.',
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
      entryEvidenceGroup,
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
          rowData={strategyGridRows}
          columnDefs={strategyColumns}
          onRowClicked={({ data }) => {
            if (!data || data.rowType === 'symbol') return;
            if (!entryReturnProfilesByStrategy.get(data.strategy)?.length)
              return;
            setExpandedEntryStrategies((previous) => {
              const next = new Set(previous);
              if (next.has(data.strategy)) next.delete(data.strategy);
              else next.add(data.strategy);
              return next;
            });
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
              Boolean(entryReturnProfilesByStrategy.get(data.strategy)?.length),
            'auto-trader-entry-evidence-symbol-row': ({ data }) =>
              data?.rowType === 'symbol',
          }}
          onGridReady={({ api }) => {
            strategyGridApi.current = api;
            try {
              const saved = localStorage.getItem(STRATEGY_GRID_STATE_KEY);
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
    </section>
  );
}
