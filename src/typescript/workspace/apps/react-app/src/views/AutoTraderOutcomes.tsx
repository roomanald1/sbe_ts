import { memo, useEffect, useMemo, useRef, useState } from 'react';
import { AgGridReact } from 'ag-grid-react';
import {
  ClientSideRowModelModule,
  ModuleRegistry,
  themeQuartz,
  type ColDef,
  type ColGroupDef,
  type GridApi,
  type IRowNode,
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
import type { SymbolPriceTick } from '../types/symbol_price';
import { formatForSymbol } from '../instrument_precision';
import { useBehaviorSnapshot } from '@workspace/ui-platform';
import { FeedToolbar } from './FeedToolbar';
import { scoutableStrategies } from './AutoTraderStrategyParameters';
import { money, strategyLabel } from './AutoTraderOrderGrid';

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
const entryOrderTypeLabel = (value: string) => {
  const type = value.split(' · ')[0];
  if (['dated-carry', 'ScheduledExecution', 'Twap', 'Vwap'].includes(type))
    return '';
  return (
    {
      Ioc: 'IOC',
      PostOnlyLimit: 'Post-only limit',
      Fok: 'FOK',
      Gtd: 'GTD',
      Market: 'Market',
      Limit: 'Limit',
    }[type] ?? type.replace(/([a-z])([A-Z])/g, '$1 $2')
  );
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
type OutcomeGridRow = AutoTraderOutcome & { markPrice?: number };
const outcomeGridDefaultColDef = {
  sortable: true,
  resizable: true,
  flex: 1,
  minWidth: 80,
};
const outcomeGridGetRowId = ({ data }: { data: OutcomeGridRow }) => data.rowId;
const CLOSED_GRID_SORT_KEY = 'autotrader-closed-position-grid-sort-v1';
const HIDE_DISABLED_STRATEGIES_KEY = 'autotrader-hide-disabled-strategies-v1';
function restoreClosedGridSort(api: GridApi<OutcomeGridRow>) {
  try {
    const stored = localStorage.getItem(CLOSED_GRID_SORT_KEY);
    const saved = stored
      ? (JSON.parse(stored) as Array<{
          colId: string;
          sort: 'asc' | 'desc';
          sortIndex?: number;
        }>)
      : [];
    const state = saved.length
      ? saved
      : [{ colId: 'openedAt', sort: 'desc' as const, sortIndex: 0 }];
    api.applyColumnState({ state, defaultState: { sort: null } });
  } catch {
    api.applyColumnState({
      state: [{ colId: 'openedAt', sort: 'desc', sortIndex: 0 }],
    });
  }
}
function persistClosedGridSort(api: GridApi<OutcomeGridRow>) {
  try {
    const state = api
      .getColumnState()
      .filter((column) => column.sort === 'asc' || column.sort === 'desc')
      .map(({ colId, sort, sortIndex }) => ({ colId, sort, sortIndex }));
    localStorage.setItem(CLOSED_GRID_SORT_KEY, JSON.stringify(state));
  } catch {
    /* Sorting still works if browser storage is unavailable. */
  }
}
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

function keepLedgerGroupsTogether(nodes: IRowNode<OutcomeGridRow>[]) {
  const groups = new Map<string, IRowNode<OutcomeGridRow>[]>();
  for (const node of nodes) {
    const row = node.data;
    if (!row) continue;
    const key = row.rowId.startsWith('ledger-summary:')
      ? row.rowId.slice('ledger-summary:'.length)
      : row.strategy + ':' + (row.strategyInstanceId ?? '') + ':' + row.symbol;
    const group = groups.get(key) ?? [];
    group.push(node);
    groups.set(key, group);
  }
  const originalPosition = new Map(nodes.map((node, index) => [node, index]));
  const ordered = [...groups.values()]
    .sort((left, right) => {
      const leftSummary =
        left.find((node) => node.data?.rowId.startsWith('ledger-summary:')) ??
        left[0];
      const rightSummary =
        right.find((node) => node.data?.rowId.startsWith('ledger-summary:')) ??
        right[0];
      return (
        (originalPosition.get(leftSummary) ?? 0) -
        (originalPosition.get(rightSummary) ?? 0)
      );
    })
    .flatMap((group) => [
      ...group.filter((node) => node.data?.rowId.startsWith('ledger-summary:')),
      ...group.filter(
        (node) => !node.data?.rowId.startsWith('ledger-summary:'),
      ),
    ]);
  nodes.splice(0, nodes.length, ...ordered);
}
const sharedOutcomesModel = new AutoTraderOutcomesModel();
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
const dateLabel = (value: number) =>
  new Intl.DateTimeFormat(undefined, {
    year: '2-digit',
    month: 'short',
    day: 'numeric',
  }).format(value);
const utcDateLabel = (value: number) =>
  new Intl.DateTimeFormat('en-GB', {
    day: '2-digit',
    month: 'short',
    year: '2-digit',
    timeZone: 'UTC',
  })
    .format(value)
    .replace(' ', '-')
    .replace(' ', '-');
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
const openPnl = (row?: OutcomeGridRow) => {
  if (!row || row.status !== 'Open') return undefined;
  if (row.carryDetails) {
    const value = row.carryDetails.marked_net_pnl_usdt;
    return typeof value === 'number' && Number.isFinite(value)
      ? value
      : undefined;
  }
  if (row.markPrice === undefined) return undefined;
  const exitNotional = Math.abs(row.signedQuantity * row.markPrice);
  const grossPnl = (row.markPrice - row.entryPrice) * row.signedQuantity;
  const estimatedExitFee = exitNotional * row.estimatedExitFeeRate;
  const estimatedExitSlippage =
    (exitNotional * row.estimatedExitSlippageBps) / 10_000;
  return grossPnl + row.entryFees - estimatedExitFee - estimatedExitSlippage;
};
function AutoTraderOutcomesComponent({
  embedded = false,
  ticks = [],
  liveOnly = false,
  closedOnly = false,
  strategyRatesOnly = false,
}: {
  embedded?: boolean;
  ticks?: SymbolPriceTick[];
  liveOnly?: boolean;
  closedOnly?: boolean;
  strategyRatesOnly?: boolean;
}) {
  const model = useMemo(() => sharedOutcomesModel, []);
  const state = useBehaviorSnapshot(model.state$);
  const settingsModel = useMemo(() => sharedAutoTraderSettingsModel, []);
  const settings = useBehaviorSnapshot(settingsModel.state$);
  const scoutState = useBehaviorSnapshot(autoTraderScoutModel.state$);
  const outcomesGridApi = useRef<GridApi<OutcomeGridRow> | null>(null);
  const strategyGridApi = useRef<GridApi<StrategyWinRateRow> | null>(null);
  const strategyGridWrapper = useRef<HTMLDivElement | null>(null);
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
  const updateHideDisabledStrategies = (value: boolean) => {
    setHideDisabledStrategies(value);
    try {
      localStorage.setItem(HIDE_DISABLED_STRATEGIES_KEY, String(value));
    } catch {
      /* The filter still works if browser storage is unavailable. */
    }
  };
  const [expandedLedgerGroups, setExpandedLedgerGroups] = useState<Set<string>>(
    () => new Set(),
  );
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
    outcomesGridApi.current?.resetRowHeights();
  }, [state.confirmingCloseSymbols, state.closingSymbols]);
  const [now, setNow] = useState(Date.now);
  const [gatePosition, setGatePosition] = useState<OutcomeGridRow | null>(null);
  useEffect(() => {
    const timer = setInterval(() => {
      setNow(Date.now());
      strategyGridApi.current?.refreshCells({
        columns: ['runReason'],
        force: true,
      });
    }, 15_000);
    return () => clearInterval(timer);
  }, []);
  const marks = useMemo(
    () =>
      new Map(
        ticks
          .filter(
            (tick) =>
              Number.isFinite(tick.price) && now - tick.receivedAt < 60_000,
          )
          .map((tick) => [tick.symbol, tick.price]),
      ),
    [ticks, now],
  );
  const rowMarks = closedOnly ? undefined : marks;
  const rowSource = closedOnly ? state.closedLedgerRows : state.rows;
  const rows = useMemo<OutcomeGridRow[]>(() => {
    if (!closedOnly)
      return rowSource
        .filter((row) =>
          liveOnly
            ? row.status === 'Open'
            : !row.rowId.startsWith('ledger-summary:') &&
              !row.rowId.startsWith('ledger-trade:'),
        )
        .map((row) => ({
          ...row,
          markPrice:
            row.status === 'Open' && !row.carryDetails
              ? rowMarks?.get(row.symbol)
              : undefined,
        }));
    const visible: OutcomeGridRow[] = [];
    const groupByTradeKey = new Map(
      rowSource
        .filter((row) => row.rowId.startsWith('ledger-summary:'))
        .map((row) => [
          (row.strategyInstanceId ?? '') + ':' + row.symbol,
          row.rowId,
        ]),
    );
    for (const row of rowSource) {
      if (row.rowId.startsWith('ledger-summary:')) visible.push(row);
      else if (row.rowId.startsWith('ledger-trade:')) {
        const groupId = groupByTradeKey.get(
          (row.strategyInstanceId ?? '') + ':' + row.symbol,
        );
        if (groupId && expandedLedgerGroups.has(groupId)) visible.push(row);
      }
    }
    return visible;
  }, [rowSource, rowMarks, liveOnly, closedOnly, expandedLedgerGroups]);
  const loadedOutcomesTodayUsd = useMemo(
    () =>
      rowSource
        .filter((row) => row.rowId.startsWith('ledger-summary:'))
        .reduce((sum, row) => sum + (row.todayRealizedUsd ?? 0), 0),
    [rowSource],
  );
  const closedRows = useMemo(
    () => (closedOnly ? [] : rows.filter((row) => row.status === 'Closed')),
    [rows, closedOnly],
  );
  const openRows = useMemo(
    () => (closedOnly ? [] : rows.filter((row) => row.status === 'Open')),
    [rows, closedOnly],
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
  }, [scoutState.rows]);
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
          : 'No blocker reported by AutoTrader';
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
    state.latestStrategyEvaluations,
    state.strategyOrders,
    state.strictEntryGates,
    state.strictBaseStrengthFloor,
    state.todayStrategyRealizedUsd,
    state.openStrategyPositions,
    state.autoTraderStatusAvailable,
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
              headerName: 'Latest blocker',
              headerTooltip:
                'The latest blocker reported by AutoTrader. Fills and cancellations are not blockers. Events older than 24 hours are marked stale.',
              valueFormatter: ({ value, data }) =>
                value
                  ? data?.runReasonAt
                    ? value + ' · ' + reasonAge(data.runReasonAt)
                    : value
                  : 'No blocker reported by AutoTrader',
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
    [],
  );
  const summary = useMemo(
    () => ({
      wins: closedRows.filter((row) => row.pnl > 0).length,
      losses: closedRows.filter((row) => row.pnl < 0).length,
      avg: closedRows.length
        ? closedRows.reduce(
            (sum, row) =>
              sum + (row.pnl / (row.quantity * row.entryPrice)) * 100,
            0,
          ) / closedRows.length
        : 0,
      openPnlByQuote: openRows.reduce<Record<string, number>>((totals, row) => {
        const pnl = openPnl(row);
        if (pnl !== undefined)
          totals[row.pnlQuoteCurrency] =
            (totals[row.pnlQuoteCurrency] ?? 0) + pnl;
        return totals;
      }, {}),
      markedOpenCount: openRows.filter((row) => openPnl(row) !== undefined)
        .length,
    }),
    [closedRows, openRows],
  );
  const columns = useMemo<ColDef<OutcomeGridRow>[]>(
    () => [
      {
        colId: 'net-vs-breakeven',
        headerName: 'Net vs breakeven',
        cellRenderer: ({ data }: { data?: OutcomeGridRow }) => {
          if (!data || data.status !== 'Open') return data?.status ?? '—';
          if (data.carryDetails) {
            const carry = data.carryDetails;
            const net = carry.marked_net_pnl_usdt;
            if (typeof net !== 'number' || !Number.isFinite(net))
              return (
                <div
                  className="auto-trader-carry-settlement"
                  title="At expiry, the spot leg is sold using TWAP over the settlement window and the future is cash-settled."
                >
                  {durationLabel(Math.max(0, carry.expiry_ms - now))} ·{' '}
                  {utcDateLabel(carry.expiry_ms)} · NEXT SETTLEMENT
                </div>
              );
            const carryNotional = carry.quantity * carry.spot_entry_price;
            const carryReturnPct =
              carryNotional > 0 ? (net / carryNotional) * 100 : 0;
            const carryPosition = Math.max(
              10,
              Math.min(90, 50 + (carryReturnPct / 5) * 40),
            );
            const carryFillLeft = Math.min(50, carryPosition);
            const carryFillWidth = Math.abs(carryPosition - 50);
            const carryTone =
              net > 0 ? 'is-positive' : net < 0 ? 'is-negative' : 'is-flat';
            const spotMovePct =
              typeof carry.spot_mark_bid === 'number' &&
              carry.spot_entry_price > 0
                ? ((carry.spot_mark_bid - carry.spot_entry_price) /
                    carry.spot_entry_price) *
                  100
                : undefined;
            const futureMovePct =
              typeof carry.future_mark_ask === 'number' &&
              carry.future_entry_price > 0
                ? ((carry.future_mark_ask - carry.future_entry_price) /
                    carry.future_entry_price) *
                  100
                : undefined;
            const signedPct = (value: number | undefined) =>
              value === undefined
                ? '—'
                : (value >= 0 ? '+' : '−') + Math.abs(value).toFixed(2) + '%';
            const signedPp = (value: number | undefined) =>
              value === undefined
                ? '—'
                : (value >= 0 ? '+' : '−') + Math.abs(value).toFixed(2) + ' pp';
            const relativeMovePct =
              spotMovePct !== undefined && futureMovePct !== undefined
                ? spotMovePct - futureMovePct
                : undefined;
            const carryResultLabel =
              net > 0 ? 'Profit' : net < 0 ? 'Loss' : 'Breakeven';
            const carrySignalExit =
              durationLabel(Math.max(0, carry.expiry_ms - now)) +
              ' · ' +
              utcDateLabel(carry.expiry_ms) +
              ' · NEXT SETTLEMENT';
            return (
              <div
                className="auto-trader-pnl-bar-cell auto-trader-carry-pnl"
                title="Estimated net P&L if both legs were closed now at the current spot bid and futures ask. Includes entry fees, estimated exit fees and slippage. Center is breakeven."
              >
                <div
                  className="auto-trader-exit-summary auto-trader-carry-summary"
                  aria-label="Settlement and carry details"
                >
                  <span
                    className="is-rebalance"
                    title="At expiry, the spot leg is sold using TWAP over the settlement window and the future is cash-settled."
                  >
                    <b>{carrySignalExit}</b>
                  </span>
                  <span>
                    <b>Spot</b> {signedPct(spotMovePct)} − <b>Future</b>{' '}
                    {signedPct(futureMovePct)} = {signedPp(relativeMovePct)}
                  </span>
                </div>
                <div
                  className="auto-trader-carry-bar-wrap"
                  aria-label={
                    'Net return ' +
                    carryReturnPct.toFixed(2) +
                    '%; red is loss, green is profit, center is breakeven'
                  }
                >
                  <div className="auto-trader-pnl-bar-track">
                    <span
                      className={'auto-trader-pnl-bar-fill ' + carryTone}
                      style={{
                        left: carryFillLeft + '%',
                        width: carryFillWidth + '%',
                      }}
                    />
                    <span
                      className="auto-trader-pnl-marker is-breakeven"
                      style={{ left: '50%' }}
                      title="Net breakeven"
                    />
                    <span
                      className="auto-trader-pnl-marker is-current"
                      style={{ left: carryPosition + '%' }}
                      title="Estimated close-now net P&L"
                    />
                  </div>
                </div>
                <div className="auto-trader-pnl-bar-label">
                  <strong className={carryTone}>
                    Close now · {carryResultLabel} ·{' '}
                    {(net >= 0 ? '+$' : '−$') + Math.abs(net).toFixed(3)} ·{' '}
                    {carryReturnPct >= 0 ? '+' : ''}
                    {carryReturnPct.toFixed(2)}%
                  </strong>
                </div>
              </div>
            );
          }
          const netQuote = openPnl(data);
          if (netQuote === undefined)
            return (
              <span className="auto-trader-pnl-bar-unavailable">
                Mark unavailable
              </span>
            );
          if (
            typeof data.quoteUsdRate !== 'number' ||
            !Number.isFinite(data.quoteUsdRate)
          )
            return (
              <span className="auto-trader-pnl-bar-unavailable">
                USD conversion unavailable
              </span>
            );
          const netUsd = netQuote * data.quoteUsdRate;
          const notional = data.quantity * data.entryPrice;
          const returnPct = notional > 0 ? (netQuote / notional) * 100 : 0;
          const positive = netUsd > 0;
          const hasStop =
            data.stopLossActive !== false &&
            typeof data.stopLossPercent === 'number' &&
            Number.isFinite(data.stopLossPercent) &&
            data.stopLossPercent > 0;
          const hasBtcRegimeExit = [
            'risk-adjusted-monthly-momentum',
            'risk-adjusted-trend-portfolio',
          ].includes(data.strategy);
          const btcSignal = hasBtcRegimeExit
            ? state.strategyDiagnostics.find(
                (diagnostic) =>
                  diagnostic.strategy === data.strategy &&
                  typeof diagnostic.btc_sma === 'number',
              )
            : undefined;
          const trendSignal =
            data.strategy === 'risk-adjusted-trend-portfolio'
              ? state.strategyDiagnostics
                  .find((diagnostic) => diagnostic.strategy === data.strategy)
                  ?.symbols.find((item) => item.symbol === data.symbol)
              : undefined;
          const trendWindows = trendSignal?.trend_return_windows ?? [];
          const trendMeanReturn = trendWindows.length
            ? trendWindows.reduce(
                (sum, window) => sum + window.return_percent,
                0,
              ) / trendWindows.length
            : undefined;
          const trendCurrentPrice = data.markPrice ?? trendSignal?.close;
          const trendPriceLabel =
            data.markPrice !== undefined ? 'Now ' : 'Latest close ';
          const trendReferencePrices = trendWindows
            .map(
              (window) =>
                ' · ' +
                window.days +
                'd ago ' +
                formatForSymbol(window.reference_close, data.symbol, 'price'),
            )
            .join('');
          const btcNow =
            marks.get('BTC-USDT') ??
            marks.get('BTCUSDT') ??
            btcSignal?.btc_close ??
            undefined;
          const isMonthlyMomentum =
            data.strategy === 'risk-adjusted-monthly-momentum';
          const monthlyLeaderDiagnostic = isMonthlyMomentum
            ? btcSignal?.symbols.find((item) => item.signal_active)
            : undefined;
          const monthlyLeader = monthlyLeaderDiagnostic?.symbol;
          const isRiskAdjustedTrend =
            data.strategy === 'risk-adjusted-trend-portfolio';
          const resultLabel =
            netUsd > 0 ? 'Profit' : netUsd < 0 ? 'Loss' : 'Breakeven';
          const pnlLabel =
            'Close now · ' +
            resultLabel +
            ' · ' +
            (netUsd >= 0 ? '+' : '−') +
            'USD ' +
            Math.abs(netUsd).toFixed(3) +
            ' · ' +
            (returnPct >= 0 ? '+' : '') +
            returnPct.toFixed(2) +
            '%';
          const trendDetail =
            'Asset ' +
            (trendCurrentPrice !== undefined
              ? trendPriceLabel.toLowerCase() +
                formatForSymbol(trendCurrentPrice, data.symbol, 'price')
              : 'price unavailable') +
            trendReferencePrices +
            (trendMeanReturn !== undefined
              ? ' · mean return ' +
                (trendMeanReturn >= 0 ? '+' : '') +
                trendMeanReturn.toFixed(2) +
                '%'
              : '') +
            ' · exit if BTC closes below SMA200 or the mean 10/20/40/80/160d return, from the last closed daily candle, is 0% or less.';
          const label = pnlLabel;
          const hasTarget =
            data.takeProfitActive !== false &&
            typeof data.takeProfitPercent === 'number' &&
            Number.isFinite(data.takeProfitPercent) &&
            data.takeProfitPercent > 0;
          const isBtcPosition =
            data.symbol === 'BTC-USDT' || data.symbol === 'BTCUSDT';
          const signalExitReturn =
            btcSignal && isBtcPosition && typeof btcSignal.btc_sma === 'number'
              ? (btcSignal.btc_sma / data.entryPrice - 1) * 100
              : undefined;
          const minReturn = Math.min(
            hasStop ? -data.stopLossPercent! : -5,
            signalExitReturn ?? Infinity,
          );
          const maxReturn = Math.max(
            hasTarget ? data.takeProfitPercent! : 5,
            signalExitReturn ?? -Infinity,
          );
          const positionForReturn = (value: number) =>
            Math.max(
              10,
              Math.min(
                90,
                10 + ((value - minReturn) / (maxReturn - minReturn)) * 80,
              ),
            );
          const signalExitPosition =
            signalExitReturn === undefined
              ? undefined
              : positionForReturn(signalExitReturn);
          const breakEvenPosition = positionForReturn(0);
          const currentPosition = positionForReturn(returnPct);
          const fillLeft = Math.min(breakEvenPosition, currentPosition);
          const fillWidth = Math.abs(currentPosition - breakEvenPosition);
          const stopPosition = hasStop
            ? positionForReturn(-data.stopLossPercent!)
            : undefined;
          const targetPosition = hasTarget
            ? positionForReturn(data.takeProfitPercent!)
            : undefined;
          const strategyDiagnostic = state.strategyDiagnostics.find(
            (diagnostic) => diagnostic.strategy === data.strategy,
          );
          const symbolDiagnostic = strategyDiagnostic?.symbols.find(
            (item) => item.symbol === data.symbol,
          );
          const currentAssetPrice = data.markPrice ?? symbolDiagnostic?.close;
          const distanceFromPriceLevel = (level: number | undefined) =>
            typeof level === 'number' &&
            level > 0 &&
            typeof currentAssetPrice === 'number'
              ? (currentAssetPrice / level - 1) * 100
              : undefined;
          const marketReturnFromEntry =
            data.entryPrice > 0 && typeof data.markPrice === 'number'
              ? (data.markPrice / data.entryPrice - 1) *
                (data.signedQuantity >= 0 ? 100 : -100)
              : undefined;
          const exitFacts: Array<{
            label: string;
            value: string;
            near?: boolean;
            tone?: string;
            detail?: string;
          }> = [];
          if (data.entryOrderExpiresAt !== undefined) {
            const remaining = Math.max(0, data.entryOrderExpiresAt - now);
            if (remaining > 0)
              exitFacts.push({
                label: 'Post-only entry',
                value:
                  durationLabel(remaining) +
                  ' left · expires ' +
                  new Date(data.entryOrderExpiresAt).toLocaleString(),
                tone: 'is-rebalance',
                detail:
                  'Expiry date and time of the still-active post-only entry order. A partial fill may remain open after the unfilled remainder expires.',
              });
          }
          const thresholdDistanceLabel = (distance: number) =>
            distance === 0
              ? 'at threshold'
              : distance > 0
                ? '+' + distance.toFixed(2) + '% above'
                : Math.abs(distance).toFixed(2) + '% below';
          if (hasStop && marketReturnFromEntry !== undefined) {
            const distance = data.stopLossPercent! + marketReturnFromEntry;
            exitFacts.push({
              label: 'Stop −' + data.stopLossPercent!.toFixed(2) + '%',
              value: distance <= 0 ? 'at/past' : distance.toFixed(2) + '% away',
              near: distance <= 0.5,
            });
          }
          if (hasTarget && marketReturnFromEntry !== undefined) {
            const distance = data.takeProfitPercent! - marketReturnFromEntry;
            exitFacts.push({
              label: 'TP +' + data.takeProfitPercent!.toFixed(2) + '%',
              value: distance <= 0 ? 'at/past' : distance.toFixed(2) + '% away',
              tone: distance <= 0.5 ? 'is-positive' : undefined,
            });
          }
          const btcExitDistance =
            typeof btcNow === 'number' &&
            btcNow > 0 &&
            typeof strategyDiagnostic?.btc_sma === 'number'
              ? (btcNow / strategyDiagnostic.btc_sma - 1) * 100
              : undefined;
          if (isRiskAdjustedTrend) {
            const nextDailyCheck = Date.UTC(
              new Date(now).getUTCFullYear(),
              new Date(now).getUTCMonth(),
              new Date(now).getUTCDate() + 1,
            );
            exitFacts.push({
              label:
                durationLabel(Math.max(0, nextDailyCheck - now)) +
                ' · ' +
                utcDateLabel(nextDailyCheck),
              value: '· NEXT DAILY REBALANCE CHECK',
              tone: 'is-rebalance',
              detail:
                'Exit signals use the latest closed daily candle. Target weights are adjusted daily only when drift exceeds 5%; this is the next expected check date, not a guaranteed trade.',
            });
            exitFacts.push({
              label: 'BTC SMA200 buffer',
              value:
                btcExitDistance === undefined
                  ? 'unavailable'
                  : btcExitDistance >= 0
                    ? '+' + btcExitDistance.toFixed(2) + '% buffer'
                    : Math.abs(btcExitDistance).toFixed(2) +
                      '% below exit threshold',
              near: btcExitDistance !== undefined && btcExitDistance <= 1,
              detail: 'Exit on a closed daily candle below BTC SMA200',
            });
            exitFacts.push({
              label: '5-window mean',
              value:
                trendMeanReturn === undefined
                  ? 'unavailable'
                  : trendMeanReturn === 0
                    ? 'at 0%'
                    : thresholdDistanceLabel(trendMeanReturn) + ' zero',
              near: trendMeanReturn !== undefined && trendMeanReturn <= 1,
              detail: 'Exit at 0% or below using the last closed daily candle',
            });
          } else if (isMonthlyMomentum) {
            const nextMonthlyRebalance = Date.UTC(
              new Date(now).getUTCFullYear(),
              new Date(now).getUTCMonth() + 1,
              1,
            );
            exitFacts.push({
              label:
                durationLabel(Math.max(0, nextMonthlyRebalance - now)) +
                ' · ' +
                utcDateLabel(nextMonthlyRebalance),
              value: '· NEXT MONTHLY REBALANCE',
              tone: 'is-rebalance',
              detail:
                'The strategy evaluates the new monthly selection on the first daily scan after the UTC month begins, using the previous month’s final daily close. Exact scan time depends on data arrival.',
            });
            exitFacts.push({
              label: 'BTC SMA200 buffer',
              value:
                btcExitDistance === undefined
                  ? 'unavailable'
                  : btcExitDistance >= 0
                    ? '+' + btcExitDistance.toFixed(2) + '% buffer'
                    : Math.abs(btcExitDistance).toFixed(2) +
                      '% below exit threshold',
              near: btcExitDistance !== undefined && btcExitDistance <= 1,
            });
            const ownSmaDistance = distanceFromPriceLevel(
              symbolDiagnostic?.primary_level,
            );
            if (ownSmaDistance !== undefined)
              exitFacts.push({
                label: 'Own SMA100',
                value: thresholdDistanceLabel(ownSmaDistance),
                near: ownSmaDistance <= 1,
              });
            exitFacts.push({
              label: 'Leader',
              value: monthlyLeader
                ? monthlyLeader +
                  (typeof monthlyLeaderDiagnostic?.leader_gap_percent ===
                  'number'
                    ? ' · runner-up ' +
                      monthlyLeaderDiagnostic.leader_gap_percent.toFixed(1) +
                      '% behind'
                    : '')
                : 'none',
              detail:
                'Leader score gap uses the last completed monthly anchor. Rotation is evaluated at the next monthly signal.',
            });
          } else if (symbolDiagnostic) {
            if (data.strategy === 'rsi2-oversold-rebound') {
              const exitLevel = symbolDiagnostic.rsi_exit_level ?? 70;
              const rsi =
                symbolDiagnostic.primary_metric_unit === 'RSI'
                  ? symbolDiagnostic.primary_metric_value
                  : undefined;
              const distance =
                typeof rsi === 'number' ? exitLevel - rsi : undefined;
              exitFacts.push({
                label: 'RSI exit ' + exitLevel.toFixed(0),
                value:
                  distance === undefined
                    ? 'unavailable'
                    : distance <= 0
                      ? 'at/past'
                      : distance.toFixed(1) + ' points away',
                near: distance !== undefined && distance <= 5,
              });
            }
            if (data.strategy === 'rsi2-oversold-rebound') {
              const maximumHoldDays = symbolDiagnostic.maximum_hold_days ?? 5;
              const maximumHoldMs = maximumHoldDays * 86_400_000;
              const maximumHoldRemaining = Math.max(
                0,
                maximumHoldMs - (now - data.openedAt),
              );
              const maximumHoldDate = data.openedAt + maximumHoldMs;
              exitFacts.push({
                label: 'Max hold',
                value:
                  maximumHoldRemaining > 0
                    ? durationLabel(maximumHoldRemaining) +
                      ' left · ' +
                      dateLabel(maximumHoldDate)
                    : 'time limit reached',
                near: maximumHoldRemaining <= 86_400_000,
                detail:
                  'The rebound signal expires after ' +
                  maximumHoldDays +
                  ' daily candles; countdown is estimated from the position open time.',
              });
            }
            if (data.strategy === 'donchian-20-btc200-hold3') {
              const minimumHoldRemaining = Math.max(
                0,
                3 * 86_400_000 - (now - data.openedAt),
              );
              exitFacts.push({
                label: '72h minimum hold',
                value:
                  minimumHoldRemaining > 0
                    ? durationLabel(minimumHoldRemaining) + ' locked'
                    : 'cleared',
                near: minimumHoldRemaining <= 3_600_000,
                detail:
                  'A signal exit waits for the 72-hour minimum hold to expire.',
              });
            }
            const primaryRule = symbolDiagnostic.primary_rule;
            const primaryIsPriceLevel =
              /own \d+-day sma|prior \d+-day high/i.test(primaryRule);
            if (primaryIsPriceLevel) {
              const distance = distanceFromPriceLevel(
                symbolDiagnostic.primary_level,
              );
              exitFacts.push({
                label: primaryRule,
                value:
                  distance === undefined
                    ? 'unavailable'
                    : thresholdDistanceLabel(distance),
                near: distance !== undefined && distance <= 1,
              });
            } else if (/return vs eligible assets/i.test(primaryRule)) {
              const leaderDiagnostic = strategyDiagnostic?.symbols.find(
                (item) => item.signal_active,
              );
              const leaderGap = leaderDiagnostic?.relative_strength_gap_pp;
              exitFacts.push({
                label: '90d leader',
                value: leaderDiagnostic
                  ? leaderDiagnostic.symbol +
                    (typeof leaderGap === 'number'
                      ? ' · runner-up ' + leaderGap.toFixed(2) + ' pp behind'
                      : '')
                  : 'none',
                detail:
                  'Leader gap is measured from the latest daily close; a different eligible leader triggers rotation.',
              });
            }
            if (
              symbolDiagnostic.secondary_rule &&
              /own \d+-day sma/i.test(symbolDiagnostic.secondary_rule) &&
              typeof symbolDiagnostic.secondary_level === 'number'
            ) {
              const distance = distanceFromPriceLevel(
                symbolDiagnostic.secondary_level,
              );
              exitFacts.push({
                label: symbolDiagnostic.secondary_rule,
                value:
                  distance === undefined
                    ? 'unavailable'
                    : thresholdDistanceLabel(distance),
                near: distance !== undefined && distance <= 1,
              });
            }
            if (
              strategyDiagnostic?.btc_gate_required &&
              btcExitDistance !== undefined
            )
              exitFacts.push({
                label: 'BTC SMA' + (strategyDiagnostic.btc_window_days ?? ''),
                value: thresholdDistanceLabel(btcExitDistance),
                near: btcExitDistance <= 1,
              });
          }
          if (exitFacts.length === 0)
            exitFacts.push({
              label: 'Signal exit',
              value: 'distance unavailable',
              detail:
                'This strategy exits on a signal change rather than a fixed price threshold',
            });
          return (
            <div
              className="auto-trader-pnl-bar-cell"
              title={
                label +
                '. Estimated net P&L if closed now at the current mark, after entry fees, estimated exit fees, and estimated adverse exit slippage' +
                (data.strategy === 'risk-adjusted-trend-portfolio'
                  ? '. ' + trendDetail
                  : '')
              }
            >
              <div
                className="auto-trader-exit-summary"
                aria-label="Exit distances"
              >
                {exitFacts.map((fact, index) => (
                  <span
                    key={fact.label + index}
                    className={
                      [fact.near && 'is-near', fact.tone]
                        .filter(Boolean)
                        .join(' ') || undefined
                    }
                    title={fact.detail ?? fact.label}
                  >
                    <b>{fact.label}</b> {fact.value}
                  </span>
                ))}
              </div>
              <div
                className="auto-trader-pnl-bar-track"
                aria-label={
                  'Net return ' +
                  returnPct.toFixed(2) +
                  '%; colored fill shows profit or loss, blue marker is breakeven'
                }
              >
                <span
                  className={
                    'auto-trader-pnl-bar-fill ' +
                    (positive
                      ? 'is-positive'
                      : netUsd < 0
                        ? 'is-negative'
                        : 'is-flat')
                  }
                  style={{ left: fillLeft + '%', width: fillWidth + '%' }}
                />
                <span
                  className="auto-trader-pnl-marker is-breakeven"
                  style={{ left: breakEvenPosition + '%' }}
                  title="Net breakeven"
                />
                <span
                  className="auto-trader-pnl-marker is-current"
                  style={{ left: currentPosition + '%' }}
                  title="Current net result"
                />
                {stopPosition !== undefined && (
                  <span
                    className="auto-trader-pnl-marker is-stop"
                    style={{ left: stopPosition + '%' }}
                    title={
                      'Stop loss −' + data.stopLossPercent?.toFixed(2) + '%'
                    }
                  />
                )}
                {targetPosition !== undefined && (
                  <span
                    className="auto-trader-pnl-marker is-target"
                    style={{ left: targetPosition + '%' }}
                    title={
                      'Take profit +' + data.takeProfitPercent?.toFixed(2) + '%'
                    }
                  />
                )}
                {signalExitPosition !== undefined && (
                  <span
                    className="auto-trader-pnl-marker is-signal-exit"
                    style={{ left: signalExitPosition + '%' }}
                    title={
                      'Signal exit: BTC 200-day average ' +
                      formatForSymbol(
                        btcSignal!.btc_sma!,
                        'BTC-USDT',
                        'price',
                      ) +
                      ' (' +
                      signalExitReturn!.toFixed(2) +
                      '% from entry)'
                    }
                  />
                )}
              </div>
              <span
                className={
                  'auto-trader-pnl-bar-label ' +
                  (positive
                    ? 'is-positive'
                    : netUsd < 0
                      ? 'is-negative'
                      : 'is-flat')
                }
              >
                {label}
              </span>
            </div>
          );
        },
        minWidth: 320,
        flex: 2,
      },
      { field: 'status', headerName: 'State', minWidth: 75 },
      {
        field: 'openedAt',
        headerName: 'Opened',
        valueFormatter: ({ value }) =>
          value ? new Date(value).toLocaleString() : '—',
        minWidth: 150,
      },
      {
        field: 'closedAt',
        headerName: 'Closed',
        valueFormatter: ({ value, data }) =>
          value
            ? new Date(value).toLocaleString()
            : data?.status === 'Partially closed'
              ? ''
              : '—',
        minWidth: 150,
      },
      { field: 'symbol', headerName: 'Symbol', minWidth: 100 },
      {
        headerName: 'Live gates',
        colId: 'live-gates',
        sortable: false,
        filter: false,
        minWidth: 84,
        maxWidth: 84,
        cellRenderer: ({ data }: { data?: OutcomeGridRow }) =>
          (liveOnly && data?.status === 'Open') ||
          (closedOnly && data?.rowId.startsWith('ledger-summary:')) ? (
            <button
              type="button"
              className="auto-trader-parameters-button"
              onClick={() => setGatePosition(data)}
            >
              Gates
            </button>
          ) : null,
      },
      {
        headerName: 'Carry contract',
        valueGetter: ({ data }) =>
          data?.carryDetails
            ? data.carryDetails.future_symbol +
              ' · settles ' +
              new Date(data.carryDetails.expiry_ms).toLocaleDateString()
            : '',
        minWidth: 220,
      },
      {
        headerName: 'Open for',
        valueGetter: ({ data }) =>
          data?.status === 'Open'
            ? Math.max(0, now - data.openedAt)
            : undefined,
        valueFormatter: ({ value }) => durationLabel(value),
        minWidth: 95,
        maxWidth: 115,
        flex: 0,
      },
      {
        headerName: 'Manual close',
        colId: 'manual-close',
        sortable: false,
        filter: false,
        minWidth: 180,
        maxWidth: 190,
        flex: 0,
        suppressMouseEventHandling: ({ event }: { event: Event }) =>
          event.target instanceof Element &&
          Boolean(event.target.closest('button')),
        cellRenderer: ({ data }: { data?: OutcomeGridRow }) => {
          if (!liveOnly || !data || data.status !== 'Open') return null;
          if (data.carryDetails)
            return (
              <span title="The paired spot and futures legs are tracked in the dated-carry paper book.">
                Paired carry
              </span>
            );
          const confirming = state.confirmingCloseSymbols.has(data.symbol);
          const closing = state.closingSymbols.has(data.symbol);
          const firstForSymbol = rows.find(
            (row) => row.status === 'Open' && row.symbol === data.symbol,
          );
          if (firstForSymbol && firstForSymbol.rowId !== data.rowId) {
            return (
              <span
                title={
                  'The close action for ' +
                  data.symbol +
                  ' closes all strategy lots for this symbol.'
                }
              >
                Same-symbol close above
              </span>
            );
          }
          if (closing)
            return <span className="auto-trader-close-progress">Closing…</span>;
          if (confirming)
            return (
              <div className="auto-trader-position-close-actions">
                <span>Close all {data.symbol} strategy lots?</span>
                <button
                  className="is-confirm"
                  type="button"
                  disabled={closing}
                  onClick={(event) => {
                    event.stopPropagation();
                    void model.confirmClose(data.symbol);
                  }}
                >
                  Confirm
                </button>
                <button
                  className="is-cancel"
                  type="button"
                  disabled={closing}
                  onClick={(event) => {
                    event.stopPropagation();
                    model.cancelClose(data.symbol);
                  }}
                >
                  Cancel
                </button>
              </div>
            );
          return (
            <button
              className="auto-trader-position-close-button"
              type="button"
              disabled={closing}
              title="Submit an IOC order to close the combined AutoTrader position for this symbol at available market liquidity"
              onClick={(event) => {
                event.stopPropagation();
                model.requestClose(data.symbol);
              }}
            >
              Close symbol
            </button>
          );
        },
      },
      {
        field: 'strategy',
        headerName: 'Strategy',
        valueFormatter: ({ value }) => strategyLabel(value),
        minWidth: 170,
      },
      {
        field: 'strategyInstanceId',
        headerName: 'Strategy instance ID',
        minWidth: 190,
      },
      { field: 'ledgerSource', headerName: 'Ledger source', minWidth: 100 },
      {
        field: 'ledgerOrderType',
        headerName: 'Trade type',
        headerTooltip:
          'Shows whether AutoTrader or an operator closed the strategy instance; otherwise shows the execution order type.',
        valueFormatter: ({ value, data }) =>
          data?.status === 'Closed' && !data.ledgerTrade
            ? '—'
            : data?.manuallyClosed
              ? 'Manual close'
              : typeof value === 'string' && value
                ? entryOrderTypeLabel(value) || '—'
                : '—',
        minWidth: 88,
      },
      {
        field: 'source',
        headerName: 'Source',
        headerTooltip:
          'Entry selection path recorded by AutoTrader. Dated carry rows are paired paper positions.',
        valueFormatter: ({ value }) =>
          value === 'paper-carry'
            ? 'Paper carry'
            : value === 'scout'
              ? 'Scout'
              : value === 'exploratory'
                ? 'Exploratory'
                : value === 'strict'
                  ? 'Strict'
                  : 'Unknown',
        minWidth: 105,
      },
      {
        field: 'side',
        headerName: 'Side',
        valueFormatter: ({ value, data }) =>
          data?.carryDetails
            ? 'Long spot / short future'
            : typeof value === 'string'
              ? value.toUpperCase()
              : '—',
        minWidth: 150,
      },
      {
        field: 'entryOrderTypes',
        headerName: 'Entry order types',
        headerTooltip:
          'Distinct exchange order types used by entry fills for this position. VWAP/TWAP scheduled fills are represented by their marketable child execution type.',
        valueFormatter: ({ value }) =>
          Array.isArray(value) && value.length
            ? [...new Set(value.map(entryOrderTypeLabel).filter(Boolean))].join(
                ', ',
              ) || '—'
            : '—',
        minWidth: 140,
      },
      {
        field: 'quantity',
        headerName: 'Quantity',
        valueFormatter: ({ value, data }) =>
          data?.rowId.startsWith('ledger-today:')
            ? '—'
            : typeof value === 'number'
              ? value.toPrecision(6)
              : '—',
        minWidth: 100,
      },
      {
        field: 'feePercent',
        headerName: 'Fee %',
        headerTooltip:
          'Fees paid as a percentage of traded notional. Summary rows use total fees divided by total traded notional.',
        valueFormatter: ({ value }) =>
          typeof value === 'number' && Number.isFinite(value)
            ? value.toFixed(4) + '%'
            : '—',
        minWidth: 90,
      },
      {
        headerName: 'Remaining',
        valueGetter: ({ data }) =>
          data?.status === 'Partially closed'
            ? data.remainingQuantity
            : undefined,
        valueFormatter: ({ value }) =>
          typeof value === 'number' ? value.toPrecision(6) : '—',
        minWidth: 100,
      },
      {
        field: 'entryPrice',
        headerName: 'Entry',
        headerTooltip:
          'For paired carry, shows the spot and futures leg entry prices.',
        valueFormatter: ({ value, data }) =>
          data?.rowId.startsWith('ledger-today:')
            ? '—'
            : data?.carryDetails
              ? 'S ' +
                data.carryDetails.spot_entry_price.toPrecision(7) +
                ' / F ' +
                data.carryDetails.future_entry_price.toPrecision(7)
              : typeof value === 'number'
                ? value.toPrecision(7)
                : '—',
        minWidth: 190,
      },
      {
        field: 'markPrice',
        headerName: 'Mark',
        headerTooltip:
          'For dated carry, shows the current executable spot bid and futures ask used for its net liquidation estimate.',
        cellRenderer: ({
          value,
          data,
        }: {
          value?: number;
          data?: OutcomeGridRow;
        }) =>
          data?.carryDetails
            ? typeof data.carryDetails.spot_mark_bid === 'number' &&
              typeof data.carryDetails.future_mark_ask === 'number'
              ? 'S ' +
                data.carryDetails.spot_mark_bid.toPrecision(7) +
                ' / F ' +
                data.carryDetails.future_mark_ask.toPrecision(7)
              : '—'
            : typeof value === 'number'
              ? value.toPrecision(7)
              : '—',
        minWidth: 200,
      },
      {
        headerName: 'Net exposure USD',
        headerTooltip:
          'Absolute net position exposure at current executable marks, converted to USD. For paired carry, this is the absolute difference between long spot and short futures notionals.',
        valueGetter: ({ data }) => {
          if (
            data?.status !== 'Open' ||
            typeof data.quoteUsdRate !== 'number' ||
            !Number.isFinite(data.quoteUsdRate)
          )
            return undefined;
          if (data.carryDetails) {
            const spotPrice =
              data.carryDetails.spot_mark_bid ??
              data.carryDetails.spot_entry_price;
            const futurePrice =
              data.carryDetails.future_mark_ask ??
              data.carryDetails.future_entry_price;
            return Math.abs(
              data.quantity * (spotPrice - futurePrice) * data.quoteUsdRate,
            );
          }
          const price = data.markPrice ?? data.entryPrice;
          return Math.abs(data.signedQuantity * price * data.quoteUsdRate);
        },
        valueFormatter: ({ value }) =>
          typeof value === 'number' && Number.isFinite(value)
            ? money.format(value)
            : '—',
        minWidth: 112,
        maxWidth: 140,
      },
      {
        headerName: 'At stop (USD)',
        headerTooltip:
          'Estimated loss if the stop-loss threshold is reached, based on entry notional and converted to USD. Excludes fees and slippage.',
        valueGetter: ({ data }) => {
          if (
            data?.status !== 'Open' ||
            typeof data.stopLossPercent !== 'number' ||
            !Number.isFinite(data.stopLossPercent) ||
            data.stopLossPercent <= 0 ||
            typeof data.quoteUsdRate !== 'number' ||
            !Number.isFinite(data.quoteUsdRate)
          )
            return undefined;
          return (
            ((-Math.abs(data.quantity * data.entryPrice) *
              data.stopLossPercent) /
              100) *
            data.quoteUsdRate
          );
        },
        cellClassRules: signed,
        valueFormatter: ({ value }) =>
          typeof value === 'number' && Number.isFinite(value)
            ? usd.format(value)
            : '—',
        minWidth: 112,
        maxWidth: 135,
      },
      {
        headerName: 'At target (USD)',
        headerTooltip:
          'Estimated gain if the take-profit threshold is reached, based on entry notional and converted to USD. Excludes fees and slippage.',
        valueGetter: ({ data }) => {
          if (
            data?.status !== 'Open' ||
            typeof data.takeProfitPercent !== 'number' ||
            !Number.isFinite(data.takeProfitPercent) ||
            data.takeProfitPercent <= 0 ||
            typeof data.quoteUsdRate !== 'number' ||
            !Number.isFinite(data.quoteUsdRate)
          )
            return undefined;
          return (
            ((Math.abs(data.quantity * data.entryPrice) *
              data.takeProfitPercent) /
              100) *
            data.quoteUsdRate
          );
        },
        cellClassRules: signed,
        valueFormatter: ({ value }) =>
          typeof value === 'number' && Number.isFinite(value)
            ? usd.format(value)
            : '—',
        minWidth: 120,
        maxWidth: 145,
      },
      {
        headerName: 'Unrealized P&L (USD)',
        headerTooltip:
          'Current open P&L net of entry fees and estimated exit fees and adverse slippage.',
        valueGetter: ({ data }) => {
          const pnl = data?.status === 'Open' ? openPnl(data) : undefined;
          return pnl !== undefined &&
            typeof data?.quoteUsdRate === 'number' &&
            Number.isFinite(data.quoteUsdRate)
            ? pnl * data.quoteUsdRate
            : undefined;
        },
        cellClassRules: signed,
        valueFormatter: ({ value }) =>
          typeof value === 'number' && Number.isFinite(value)
            ? money.format(value)
            : '—',
        minWidth: 150,
      },
      {
        headerName: 'Entry fees',
        valueGetter: ({ data }) =>
          data?.status === 'Open' ? Math.abs(data.entryFees) : undefined,
        valueFormatter: ({ value, data }) =>
          typeof value === 'number'
            ? (data?.pnlQuoteCurrency ?? '') + ' ' + value.toFixed(4)
            : '—',
        minWidth: 110,
      },
      {
        headerName: 'Est. exit fee',
        valueGetter: ({ data }) =>
          data?.status === 'Open' && data.markPrice !== undefined
            ? Math.abs(data.signedQuantity * data.markPrice) *
              data.estimatedExitFeeRate
            : undefined,
        valueFormatter: ({ value, data }) =>
          typeof value === 'number'
            ? (data?.pnlQuoteCurrency ?? '') + ' ' + value.toFixed(4)
            : '—',
        minWidth: 120,
      },
      {
        headerName: 'Est. exit slippage',
        valueGetter: ({ data }) =>
          data?.status === 'Open' && data.markPrice !== undefined
            ? (Math.abs(data.signedQuantity * data.markPrice) *
                data.estimatedExitSlippageBps) /
              10_000
            : undefined,
        valueFormatter: ({ value, data }) =>
          typeof value === 'number'
            ? (data?.pnlQuoteCurrency ?? '') + ' ' + value.toFixed(4)
            : '—',
        minWidth: 135,
      },
      {
        headerName: 'Post-exit move',
        headerTooltip:
          'Signed price movement 30 minutes after a take-profit, stop-loss, or profit-protection exit. Positive values mean price continued in the original trade direction; negative values mean it reversed. Read this with the exit reason to judge the timing.',
        valueGetter: ({ data }) =>
          data?.status === 'Closed'
            ? data.postExitMissedReturnPercent
            : undefined,
        valueFormatter: ({ value, data }) => {
          if (typeof value !== 'number' || !Number.isFinite(value))
            return 'Pending';
          const quote = data?.pnlQuoteCurrency ?? '';
          const pnl = data?.postExitMissedPnlQuote;
          const amount =
            typeof pnl === 'number' && Number.isFinite(pnl)
              ? ` · ${quote} ${pnl >= 0 ? '+' : ''}${pnl.toFixed(4)}`
              : '';
          const best = data?.postExitBestReturnPercent;
          const worst = data?.postExitWorstReturnPercent;
          const range =
            typeof best === 'number' &&
            Number.isFinite(best) &&
            typeof worst === 'number' &&
            Number.isFinite(worst)
              ? ` · ${worst.toFixed(2)}…${best >= 0 ? '+' : ''}${best.toFixed(2)}% range`
              : '';
          return `${value >= 0 ? '+' : ''}${value.toFixed(2)}%${amount}${range}`;
        },
        cellClassRules: signed,
        minWidth: 190,
      },
      {
        field: 'netPnlUsd',
        headerName: 'Net today (USD)',
        headerTooltip:
          'Net realized P&L from this strategy instance during the current UTC day. Matches the position ledger Realized today total.',
        cellClassRules: signed,
        valueFormatter: ({ value }) =>
          typeof value === 'number' && Number.isFinite(value)
            ? money.format(value)
            : '—',
        minWidth: 145,
      },
      {
        headerName: 'Realized / open P&L (USD)',
        headerTooltip:
          'Realized closed-trade P&L or estimated open P&L converted from quote currency to USD.',
        valueGetter: ({ data }) =>
          data?.status === 'Open'
            ? openPnl(data) !== undefined &&
              typeof data.quoteUsdRate === 'number' &&
              Number.isFinite(data.quoteUsdRate)
              ? openPnl(data)! * data.quoteUsdRate
              : undefined
            : data?.netPnlUsd,
        cellClassRules: signed,
        valueFormatter: ({ value }) =>
          typeof value === 'number' && Number.isFinite(value)
            ? money.format(value)
            : '—',
        minWidth: 170,
      },
      { field: 'direction', headerName: 'Signal direction', minWidth: 110 },
      {
        field: 'strength',
        headerName: 'Strength',
        valueFormatter: ({ value }) =>
          typeof value === 'number' ? (value * 100).toFixed(0) + '%' : '—',
        minWidth: 85,
      },
      {
        field: 'scout',
        headerName: 'Scout',
        valueFormatter: ({ value }) =>
          value === true ? 'Yes' : value === false ? 'No' : '—',
        minWidth: 75,
      },
      {
        headerName: 'Trend 1m',
        valueGetter: ({ data }) => data?.components.trend_1m_percent,
        valueFormatter: ({ value }) => pct(value),
        minWidth: 95,
      },
      {
        headerName: 'Trend 5m / vol',
        valueGetter: ({ data }) => data?.components.trend_5m_volatility_units,
        valueFormatter: ({ value }) =>
          typeof value === 'number' ? value.toFixed(2) + 'σ' : '—',
        minWidth: 110,
      },
      {
        headerName: 'Momentum 10s',
        valueGetter: ({ data }) => data?.components.momentum_10s_percent,
        valueFormatter: ({ value }) => pct(value),
        minWidth: 105,
      },
      {
        headerName: 'Trade flow',
        valueGetter: ({ data }) => data?.components.trade_flow_imbalance,
        valueFormatter: ({ value }) =>
          typeof value === 'number' ? (value * 100).toFixed(0) + '%' : '—',
        minWidth: 95,
      },
      {
        headerName: 'Book pressure',
        valueGetter: ({ data }) => data?.components.book_imbalance,
        valueFormatter: ({ value }) =>
          typeof value === 'number' ? (value * 100).toFixed(0) + '%' : '—',
        minWidth: 105,
      },
      {
        headerName: 'VWAP mispricing',
        valueGetter: ({ data }) => data?.components.mispricing_vs_vwap_bps,
        valueFormatter: ({ value }) =>
          typeof value === 'number' ? value.toFixed(1) + ' bps' : '—',
        minWidth: 120,
      },
      {
        headerName: 'Pressure / no move',
        valueGetter: ({ data }) => data?.components.pressure_without_price_move,
        valueFormatter: ({ value }) =>
          typeof value === 'number' ? (value * 100).toFixed(0) + '%' : '—',
        minWidth: 130,
      },
    ],
    [
      now,
      liveOnly,
      model,
      rows,
      state.confirmingCloseSymbols,
      state.closingSymbols,
      state.datedCarryDiagnostics,
      state.strategyDiagnostics,
    ],
  );
  const visibleColumns = useMemo(
    () =>
      liveOnly
        ? [
            'Symbol',
            'Live gates',
            'Carry contract',
            'Strategy',
            'Side',
            'Entry order types',
            'Open for',
            'Entry',
            'Net exposure USD',
            'Net vs breakeven',
            'Manual close',
          ].flatMap((headerName) =>
            columns
              .filter((column) => column.headerName === headerName)
              .map((column) => {
                const widths: Record<string, number> = {
                  Symbol: 92,
                  'Live gates': 84,
                  'Carry contract': 150,
                  Strategy: 146,
                  Side: 112,
                  'Entry order types': 112,
                  'Open for': 68,
                  Entry: 148,
                  'Net exposure USD': 120,
                  'Manual close': 96,
                };
                if (headerName === 'Net vs breakeven')
                  return { ...column, width: 420, minWidth: 420, flex: 1 };
                return {
                  ...column,
                  width: widths[headerName] ?? 100,
                  minWidth: widths[headerName] ?? 100,
                  maxWidth: widths[headerName] ?? 100,
                  flex: 0,
                };
              }),
          )
        : closedOnly
          ? [
              {
                headerName: '',
                width: 38,
                minWidth: 38,
                maxWidth: 38,
                sortable: false,
                resizable: false,
                cellRenderer: ({ data }: { data?: OutcomeGridRow }) => {
                  if (!data?.rowId.startsWith('ledger-summary:'))
                    return (
                      <span
                        className="auto-trader-ledger-child-marker"
                        aria-hidden="true"
                      >
                        ↳
                      </span>
                    );
                  const expanded = expandedLedgerGroups.has(data.rowId);
                  return (
                    <button
                      type="button"
                      className="auto-trader-ledger-expander"
                      aria-label={
                        (expanded ? 'Hide' : 'Show') +
                        ' child trades for ' +
                        data.symbol
                      }
                      aria-expanded={expanded}
                      onClick={() =>
                        setExpandedLedgerGroups((current) => {
                          const next = new Set(current);
                          if (next.has(data.rowId)) next.delete(data.rowId);
                          else next.add(data.rowId);
                          return next;
                        })
                      }
                    >
                      {expanded ? '−' : '+'}
                    </button>
                  );
                },
              },
              ...columns
                .filter((column) =>
                  [
                    'Symbol',
                    'Live gates',
                    'Strategy',
                    'Strategy instance ID',
                    'Trade type',
                    'Side',
                    'Quantity',
                    'Fee %',
                    'Entry',
                    'Opened',
                    'Net today (USD)',
                  ].includes(String(column.headerName ?? '')),
                )
                .map((column) => {
                  const widths: Record<string, number> = {
                    Symbol: 90,
                    'Live gates': 84,
                    Strategy: 170,
                    'Strategy instance ID': 230,
                    'Trade type': 95,
                    Side: 80,
                    Quantity: 105,
                    'Fee %': 85,
                    Entry: 105,
                    Opened: 155,
                    'Net today (USD)': 135,
                  };
                  const originalHeader = String(column.headerName ?? '');
                  const width = widths[originalHeader] ?? 100;
                  return {
                    ...column,
                    ...(originalHeader === 'Opened'
                      ? { headerName: 'Trade time' }
                      : {}),
                    width,
                    minWidth: width,
                    maxWidth: width,
                    flex: 0,
                  };
                }),
            ]
          : columns.filter(
              (column) =>
                !['strategyInstanceId', 'ledgerSource'].includes(
                  String(column.field ?? ''),
                ),
            ),
    [closedOnly, columns, expandedLedgerGroups, liveOnly],
  );
  return (
    <section
      className={
        embedded
          ? 'auto-trader-outcomes-embedded'
          : 'feed-section signal-info-panel'
      }
      aria-label={
        strategyRatesOnly
          ? 'AutoTrader strategy win rates'
          : liveOnly
            ? 'Live AutoTrader positions'
            : 'AutoTrader position outcomes'
      }
    >
      {!embedded && (
        <FeedToolbar
          title={
            liveOnly
              ? 'AutoTrader live positions'
              : closedOnly
                ? 'AutoTrader closed positions'
                : 'AutoTrader positions & outcomes'
          }
        />
      )}
      {!liveOnly && !closedOnly && !strategyRatesOnly && (
        <p className="signal-info-note">
          {openRows.length} open positions · {summary.markedOpenCount}/
          {openRows.length} marked · indicative open P&amp;L{' '}
          {summary.markedOpenCount
            ? Object.entries(summary.openPnlByQuote)
                .map(
                  ([quote, value]) =>
                    quote + ' ' + (value >= 0 ? '+' : '') + value.toFixed(4),
                )
                .join(' · ')
            : 'unavailable'}{' '}
          · {summary.wins} closed winners · {summary.losses} closed losers ·{' '}
          {summary.avg.toFixed(3)}% average closed return. Open marks use the
          latest price tick and net out signed entry fees, estimated exit fees,
          and estimated adverse exit slippage based on AutoTrader fills for that
          symbol; closed P&amp;L uses audited realized values. Signal values
          show the recorded entry decision, not proof of causation.
        </p>
      )}
      {closedOnly && (
        <p className="signal-info-note">
          Today’s realized P&amp;L by strategy instance · total{' '}
          {money.format(loadedOutcomesTodayUsd)}. Expand an instance to see
          today’s individual fills. Totals use the same UTC-day ledger
          calculation as Realized today.
        </p>
      )}
      {!strategyRatesOnly && state.error && (
        <div className="positions-error">{state.error}</div>
      )}
      {!closedOnly && !strategyRatesOnly && (
        <div
          className={
            'auto-trader-pnl-marker-legend' + (liveOnly ? ' is-compact' : '')
          }
          aria-label="Position bar marker key"
        >
          <span>
            <i className="is-current" />
            Now
          </span>
          <span>
            <i className="is-breakeven" />
            BE
          </span>
          <span>
            <i className="is-signal-exit" />
            Signal
          </span>
          <span className="is-stop">SL</span>
          <span className="is-target">TP</span>
        </div>
      )}
      {(strategyRatesOnly || (!liveOnly && !closedOnly)) && (
        <>
          <div className="auto-trader-strategy-winrate-heading">
            <strong className="auto-trader-strategy-winrate-title">
              Strategy performance and today’s realized P&L
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
            Today P&amp;L includes all realized exits today, including partial
            exits on positions still open. Wins, losses, and Closed net use
            fully closed audited trades only. Unrealized P&amp;L is shown with
            open positions and is not included in Today P&amp;L.
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
        </>
      )}
      {closedOnly && state.hasOlderLedgerTrades && (
        <button
          type="button"
          className="auto-trader-audit-more"
          disabled={state.loadingOlderLedgerTrades}
          onClick={() => void model.loadOlderLedgerTrades()}
        >
          {state.loadingOlderLedgerTrades
            ? 'Loading today’s fills…'
            : 'Load more today’s fills'}
        </button>
      )}
      {!liveOnly &&
        !closedOnly &&
        !strategyRatesOnly &&
        state.auditCursor != null && (
          <button
            type="button"
            className="auto-trader-audit-more"
            disabled={state.loadingOlderAudit}
            onClick={() => void model.loadOlderAudit()}
          >
            {state.loadingOlderAudit
              ? 'Loading older history…'
              : 'Load older history'}
          </button>
        )}
      {!strategyRatesOnly && (
        <div
          className={
            'data-grid-wrap signal-info-grid' +
            (liveOnly
              ? ' auto-trader-live-position-grid'
              : closedOnly
                ? ' auto-trader-ledger-grid'
                : '')
          }
        >
          <AgGridReact<OutcomeGridRow>
            rowData={rows}
            columnDefs={visibleColumns}
            onGridReady={(event) => {
              outcomesGridApi.current = event.api;
              if (closedOnly) restoreClosedGridSort(event.api);
            }}
            onSortChanged={(event) => {
              if (closedOnly) persistClosedGridSort(event.api);
            }}
            getRowId={outcomeGridGetRowId}
            maintainColumnOrder
            getRowHeight={() => (closedOnly ? 30 : liveOnly ? 52 : 48)}
            getRowClass={({ data }) =>
              data?.rowId.startsWith('ledger-summary:')
                ? 'auto-trader-closed-position-row auto-trader-ledger-summary-row'
                : data?.rowId.startsWith('ledger-trade:')
                  ? 'auto-trader-closed-position-row auto-trader-ledger-child-row'
                  : data?.status === 'Open'
                    ? 'auto-trader-open-position-row'
                    : 'auto-trader-closed-position-row'
            }
            defaultColDef={outcomeGridDefaultColDef}
            postSortRows={
              closedOnly
                ? ({ nodes }) => keepLedgerGroupsTogether(nodes)
                : undefined
            }
            theme={theme}
            domLayout={closedOnly ? 'normal' : 'autoHeight'}
            rowHeight={closedOnly ? 30 : liveOnly ? 52 : 48}
            headerHeight={closedOnly || liveOnly ? 24 : 28}
          />
        </div>
      )}
      {!strategyRatesOnly &&
        !state.loading &&
        rows.length === 0 &&
        !state.error && (
          <p className="positions-empty">
            {liveOnly
              ? 'No live AutoTrader positions are open.'
              : closedOnly
                ? 'No AutoTrader position-ledger trades are available yet.'
                : 'No open AutoTrader positions or fully closed audited trades are available yet.'}
          </p>
        )}
      {gatePosition &&
        (() => {
          const entryDecision = gatePosition.entryStrategyDecision;
          const exitDecision = gatePosition.latestStrategyEvaluation;
          const gateNumber = (value: number | null | undefined, digits = 4) =>
            typeof value === 'number' && Number.isFinite(value)
              ? value.toLocaleString(undefined, {
                  maximumFractionDigits: digits,
                })
              : '—';
          const componentLabel = (name: string) =>
            name
              .split('_')
              .map((part) =>
                part ? part[0].toUpperCase() + part.slice(1) : part,
              )
              .join(' ');
          const floorKeyByComponent: Record<string, string> = {
            momentum_10s_percent: 'minimum_momentum_floor',
            trend_5m_volatility_units: 'minimum_momentum_floor',
            trade_flow_imbalance: 'minimum_flow_imbalance_floor',
            book_imbalance: 'minimum_book_imbalance_floor',
            mispricing_vs_vwap_bps: 'minimum_vwap_dislocation_bps_floor',
            pressure_without_price_move: 'minimum_absorption_pressure_floor',
          };
          const directionAdjusted = (name: string, value: number) => {
            const short = ['SHORT', 'SELL', 'BEARISH'].includes(
              (
                gatePosition.signalDirection ?? gatePosition.direction
              ).toUpperCase(),
            );
            return name === 'pressure_without_price_move'
              ? (short ? 1 : -1) * value
              : (short ? -1 : 1) * value;
          };
          const componentMargin = (orientedValue: number, floor: number) => {
            const excess = orientedValue - floor;
            if (floor === 0)
              return ` · ${excess >= 0 ? '+' : ''}${gateNumber(excess)} above floor`;
            return ` · ${excess >= 0 ? '+' : ''}${gateNumber((excess / Math.abs(floor)) * 100, 1)}% above floor`;
          };
          const strategyGateComponents: Record<string, string[]> = {
            'live-momentum': [
              'momentum_10s_percent',
              'trend_5m_volatility_units',
            ],
            'live-trade-flow': ['trade_flow_imbalance'],
            'live-order-book': ['book_imbalance'],
            'live-vwap-dislocation': ['mispricing_vs_vwap_bps'],
            'live-absorption-pressure': ['pressure_without_price_move'],
          };
          const gateComponents =
            strategyGateComponents[gatePosition.strategy] ?? [];
          const liveGateRows = gateComponents.flatMap((name) => {
            const value = gatePosition.components[name];
            const floorKey = floorKeyByComponent[name];
            const floor = floorKey
              ? gatePosition.components[floorKey]
              : undefined;
            if (typeof value !== 'number' || typeof floor !== 'number')
              return [];
            const orientedValue = directionAdjusted(name, value);
            const passed =
              name === 'momentum_10s_percent' ||
              name === 'trend_5m_volatility_units'
                ? orientedValue > floor
                : orientedValue >= floor;
            return [
              {
                label: componentLabel(name),
                passed,
                detail: `Entry value: ${gateNumber(value)} · direction-adjusted: ${gateNumber(orientedValue)} ${orientedValue >= floor ? '≥' : '<'} floor ${gateNumber(floor)}${componentMargin(orientedValue, floor)}`,
              },
            ];
          });
          const gateRows = [
            ...liveGateRows,
            ...(entryDecision?.btc_gate_required
              ? [
                  {
                    label: `BTC ${entryDecision.btc_window_days ?? ''}-day SMA trend`,
                    passed: entryDecision.btc_gate_passed,
                    detail: `BTC close ${gateNumber(entryDecision.btc_close, 2)} · required floor (SMA): ${gateNumber(entryDecision.btc_sma, 2)}`,
                  },
                ]
              : []),
            ...(entryDecision?.primary_rule
              ? [
                  {
                    label: `Primary: ${entryDecision.primary_rule}`,
                    passed: entryDecision.primary_passed,
                    detail: `Floor: ${gateNumber(entryDecision.primary_level)} ${entryDecision.primary_metric_unit ?? ''} · measured: ${gateNumber(entryDecision.primary_metric_value)} ${entryDecision.primary_metric_unit ?? ''} · distance ${gateNumber(entryDecision.primary_distance_percent, 2)}%`,
                  },
                  ...(entryDecision.secondary_rule
                    ? [
                        {
                          label: `Secondary: ${entryDecision.secondary_rule}`,
                          passed: entryDecision.secondary_passed,
                          detail: `Floor: ${gateNumber(entryDecision.secondary_level)} · distance ${gateNumber(entryDecision.secondary_distance_percent, 2)}%`,
                        },
                      ]
                    : []),
                  {
                    label: 'Entry decision',
                    passed: true,
                    detail:
                      'Strategy entry was accepted with this recorded gate evidence',
                  },
                ]
              : []),
          ];
          const entryContextRows = [
            ...Object.entries(gatePosition.components)
              .filter(
                ([name, value]) =>
                  typeof value === 'number' &&
                  Number.isFinite(value) &&
                  !name.startsWith('minimum_') &&
                  !name.endsWith('_floor'),
              )
              .map(([name, value]) => ({
                label: componentLabel(name),
                detail: `Recorded signal value: ${gateNumber(value)}`,
              })),
            ...(typeof entryDecision?.observed_close === 'number'
              ? [
                  {
                    label: 'Asset close at evaluation',
                    detail: formatForSymbol(
                      entryDecision.observed_close,
                      gatePosition.symbol,
                      'price',
                    ),
                  },
                ]
              : []),
            ...(entryDecision?.trend_return_windows ?? []).map((window) => ({
              label: `${window.days}-day asset return`,
              detail: `${window.return_percent >= 0 ? '+' : ''}${gateNumber(window.return_percent, 2)}% · reference close ${formatForSymbol(window.reference_close, gatePosition.symbol, 'price')}`,
            })),
            ...(typeof entryDecision?.primary_metric_value === 'number' &&
            entryDecision.primary_metric_unit
              ? [
                  {
                    label: `Entry metric (${entryDecision.primary_metric_unit})`,
                    detail: `Recorded value: ${gateNumber(entryDecision.primary_metric_value)}`,
                  },
                ]
              : []),
            ...(typeof entryDecision?.cross_sectional_metric_value === 'number'
              ? [
                  {
                    label: 'Cross-sectional ranking metric',
                    detail: `Recorded value ${gateNumber(entryDecision.cross_sectional_metric_value)}${typeof entryDecision.cross_sectional_threshold === 'number' ? ` · threshold ${gateNumber(entryDecision.cross_sectional_threshold)}` : ''}`,
                  },
                ]
              : []),
            ...(typeof entryDecision?.strategy_order_size_multiplier ===
            'number'
              ? [
                  {
                    label: 'Target allocation weight',
                    detail: `${gateNumber(entryDecision.strategy_order_size_multiplier * 100, 2)}% at entry evaluation`,
                  },
                ]
              : []),
          ];
          const positionStillOpen =
            gatePosition.status === 'Open' ||
            gatePosition.status === 'Partially closed';
          const carry = gatePosition.carryDetails;
          const exitStatus = (reason: string, active?: boolean) => {
            if (positionStillOpen)
              return active === false ? 'Inactive' : 'Armed';
            if (gatePosition.exitReason === reason) return 'Triggered · closed';
            return gatePosition.exitReason
              ? 'Not triggered'
              : 'Exit reason unavailable';
          };
          const exitRows = carry
            ? [
                {
                  label: 'Expiry settlement',
                  status:
                    carry.settlement_price != null
                      ? 'Settled'
                      : positionStillOpen
                        ? 'Pending'
                        : 'Settlement details unavailable',
                  detail:
                    carry.settlement_price != null
                      ? `Settlement ${gateNumber(carry.settlement_price, 6)} · source ${carry.settlement_source ?? 'unavailable'}${carry.spot_exit_twap != null ? ` · spot exit TWAP ${gateNumber(carry.spot_exit_twap, 6)}` : ''}`
                      : `Contract expiry: ${new Date(carry.expiry_ms).toLocaleString()}`,
                },
              ]
            : [
                ...(gatePosition.stopLossPercent !== undefined ||
                gatePosition.exitReason === 'stop-loss'
                  ? [
                      {
                        label: 'Stop loss',
                        status: exitStatus(
                          'stop-loss',
                          gatePosition.stopLossActive,
                        ),
                        detail:
                          gatePosition.stopLossPercent === undefined
                            ? 'Loss threshold unavailable'
                            : `Net return trigger: −${gateNumber(gatePosition.stopLossPercent, 2)}%`,
                      },
                    ]
                  : []),
                ...(gatePosition.takeProfitPercent !== undefined ||
                gatePosition.exitReason === 'take-profit'
                  ? [
                      {
                        label: 'Take profit',
                        status: exitStatus(
                          'take-profit',
                          gatePosition.takeProfitActive,
                        ),
                        detail:
                          gatePosition.takeProfitPercent === undefined
                            ? 'Profit threshold unavailable'
                            : `Net return trigger: +${gateNumber(gatePosition.takeProfitPercent, 2)}%`,
                      },
                    ]
                  : []),
                ...(gatePosition.exitReason &&
                !['stop-loss', 'take-profit', 'profit-protection'].includes(
                  gatePosition.exitReason,
                )
                  ? [
                      {
                        label: 'Other exit',
                        status: `Triggered · ${gatePosition.exitReason}`,
                        detail: 'Recorded exit reason',
                      },
                    ]
                  : []),
                ...(gatePosition.exitReason === 'profit-protection'
                  ? (() => {
                      const protection = gatePosition.profitProtection;
                      const activation = protection?.activationPercent;
                      const peak = protection?.peakReturnPercent;
                      const giveback = protection?.givebackPercent;
                      const minimumLock = protection?.minimumLockPercent;
                      const exitReturn = protection?.exitReturnPercent;
                      const floor =
                        typeof peak === 'number' &&
                        typeof giveback === 'number' &&
                        typeof minimumLock === 'number'
                          ? Math.max(peak - giveback, minimumLock)
                          : undefined;
                      return [
                        {
                          label: 'Profit protection',
                          status: 'Triggered · profit protected',
                          detail:
                            typeof peak === 'number' &&
                            typeof floor === 'number'
                              ? `Activated at +${gateNumber(activation, 3)}% · peak +${gateNumber(peak, 3)}% · lock floor +${gateNumber(floor, 3)}% (peak minus ${gateNumber(giveback!, 3)} pp, minimum lock +${gateNumber(minimumLock!, 3)}%)${typeof exitReturn === 'number' ? ` · exit return ${exitReturn >= 0 ? '+' : ''}${gateNumber(exitReturn, 3)}%` : ''}`
                              : 'Trailing net-return rule triggered after a peak profit; detailed threshold values were not retained.',
                        },
                      ];
                    })()
                  : []),
              ];
          const exitTrendWindows = exitDecision?.trend_return_windows ?? [];
          const exitTrendMeanReturn = exitTrendWindows.length
            ? exitTrendWindows.reduce(
                (sum, window) => sum + window.return_percent,
                0,
              ) / exitTrendWindows.length
            : undefined;
          const trendExitRows =
            gatePosition.strategy === 'risk-adjusted-trend-portfolio'
              ? [
                  {
                    label: 'BTC trend floor',
                    status:
                      typeof exitDecision?.btc_close === 'number' &&
                      typeof exitDecision.btc_sma === 'number'
                        ? exitDecision.btc_close <= exitDecision.btc_sma
                          ? 'Triggered · next daily check'
                          : 'Not triggered'
                        : 'Unavailable',
                    detail:
                      typeof exitDecision?.btc_close === 'number' &&
                      typeof exitDecision.btc_sma === 'number'
                        ? `Exit if BTC closes below its ${exitDecision.btc_window_days ?? 200}-day SMA · latest closed close ${gateNumber(exitDecision.btc_close, 2)} · SMA ${gateNumber(exitDecision.btc_sma, 2)} · ${gateNumber((exitDecision.btc_close / exitDecision.btc_sma) * 100 - 100, 2)}% from floor`
                        : 'BTC close or SMA is unavailable',
                  },
                  {
                    label: 'Asset trend mean',
                    status:
                      exitTrendWindows.length > 0 &&
                      exitTrendMeanReturn !== undefined
                        ? exitTrendMeanReturn <= 0
                          ? 'Triggered · next daily check'
                          : 'Not triggered'
                        : 'Unavailable',
                    detail:
                      exitTrendWindows.length > 0 &&
                      exitTrendMeanReturn !== undefined
                        ? `Exit if mean return across ${exitTrendWindows.map((window) => window.days).join('/')}-day windows is 0% or less · current mean ${exitTrendMeanReturn >= 0 ? '+' : ''}${exitTrendMeanReturn.toFixed(2)}% · ${Math.abs(exitTrendMeanReturn).toFixed(2)} percentage points ${exitTrendMeanReturn > 0 ? 'above' : 'below'} trigger`
                        : 'Daily trend return windows are unavailable',
                  },
                ]
              : [];
          return (
            <div
              className="auto-trader-gates-backdrop"
              role="presentation"
              onMouseDown={(event) => {
                if (event.target === event.currentTarget) setGatePosition(null);
              }}
            >
              <section
                className="auto-trader-gates-popup"
                role="dialog"
                aria-modal="true"
                aria-label={`${gatePosition.strategy} live gates`}
              >
                <header>
                  <h2>
                    {strategyLabel(gatePosition.strategy)} ·{' '}
                    {gatePosition.symbol}
                  </h2>
                  <button
                    type="button"
                    aria-label="Close"
                    onClick={() => setGatePosition(null)}
                  >
                    ×
                  </button>
                </header>
                <p>
                  {carry
                    ? 'Dated carry entry details'
                    : 'Strategy gate evidence recorded at entry'}
                </p>
                {!carry && (
                  <>
                    <details className="auto-trader-gate-parameters">
                      <summary>Strategy parameters used</summary>
                      {entryDecision?.strategy_parameters &&
                      Object.keys(entryDecision.strategy_parameters).length ? (
                        <ul>
                          {Object.entries(entryDecision.strategy_parameters)
                            .sort(([left], [right]) =>
                              left.localeCompare(right),
                            )
                            .map(([key, value]) => (
                              <li key={key}>
                                <strong>Used</strong>
                                <span>{componentLabel(key)}</span>
                                <small>{gateNumber(value, 6)}</small>
                                <strong>Base</strong>
                                <span>Default · Δ vs base</span>
                                {(() => {
                                  const base =
                                    state.strategyParameterDefaults[
                                      `${gatePosition.strategy}:${gatePosition.symbol}`
                                    ]?.[key];
                                  if (
                                    typeof base !== 'number' ||
                                    typeof value !== 'number'
                                  )
                                    return <small>Base unavailable</small>;
                                  const delta = value - base;
                                  return (
                                    <small>
                                      {gateNumber(base, 6)} ·{' '}
                                      <span
                                        className={
                                          delta > 0
                                            ? 'auto-trader-gate-delta-positive'
                                            : delta < 0
                                              ? 'auto-trader-gate-delta-negative'
                                              : 'auto-trader-gate-delta-zero'
                                        }
                                      >
                                        {delta > 0 ? '+' : ''}
                                        {gateNumber(delta, 6)}
                                      </span>
                                    </small>
                                  );
                                })()}
                              </li>
                            ))}
                        </ul>
                      ) : (
                        <p>
                          Parameters used at entry were not retained for this
                          position.
                        </p>
                      )}
                    </details>
                    <h3>Entry signal context</h3>
                    <p>
                      Recorded signal values available when this position was
                      opened; these provide context and are not additional
                      gates.
                    </p>
                    {entryContextRows.length ? (
                      <ul>
                        {entryContextRows.map((row) => (
                          <li key={row.label}>
                            <strong>Recorded</strong>
                            <span>{row.label}</span>
                            <small>{row.detail}</small>
                          </li>
                        ))}
                      </ul>
                    ) : (
                      <p>
                        Entry signal context was not retained for this position.
                      </p>
                    )}
                  </>
                )}
                {carry ? (
                  <ul>
                    <li>
                      <strong>Paired</strong>
                      <span>Spot long / futures short</span>
                      <small>
                        {carry.spot_symbol} · spot entry{' '}
                        {gateNumber(carry.spot_entry_price, 6)}
                      </small>
                    </li>
                    <li>
                      <strong>Paired</strong>
                      <span>Futures leg</span>
                      <small>
                        {carry.future_symbol} · futures entry{' '}
                        {gateNumber(carry.future_entry_price, 6)}
                      </small>
                    </li>
                    <li>
                      <strong>Recorded</strong>
                      <span>Entry basis</span>
                      <small>
                        {gateNumber(
                          (carry.future_entry_price / carry.spot_entry_price -
                            1) *
                            100,
                          3,
                        )}
                        % · expires {new Date(carry.expiry_ms).toLocaleString()}
                      </small>
                    </li>
                  </ul>
                ) : gateRows.length ? (
                  <ul>
                    {gateRows.map((gate) => (
                      <li key={gate.label}>
                        <strong>
                          {gate.passed ? '✓ Passed' : '○ Not passed'}
                        </strong>
                        <span>{gate.label}</span>
                        <small>{gate.detail}</small>
                      </li>
                    ))}
                  </ul>
                ) : (
                  <p>
                    Gate diagnostics are not available for this strategy and
                    symbol.
                  </p>
                )}
                {(trendExitRows.length > 0 || exitRows.length > 0) && (
                  <>
                    <h3>Exit gates</h3>
                    {gatePosition.exitReason === 'profit-protection' && (
                      <p>
                        Profit protection follows net return after it reaches
                        its activation level. It exits when return gives back
                        the configured amount from its peak, while preserving
                        the minimum locked profit. Volatility or a strategy
                        signal reversal did not trigger this rule.
                      </p>
                    )}
                    <ul>
                      {[...trendExitRows, ...exitRows].map((gate) => (
                        <li key={gate.label}>
                          <strong
                            className={
                              gate.status.startsWith('Triggered')
                                ? 'is-triggered'
                                : gate.status === 'Armed'
                                  ? 'is-armed'
                                  : 'is-pending'
                            }
                          >
                            {gate.status}
                          </strong>
                          <span>{gate.label}</span>
                          <small>{gate.detail}</small>
                        </li>
                      ))}
                    </ul>
                  </>
                )}
              </section>
            </div>
          );
        })()}
    </section>
  );
}
export const AutoTraderOutcomes = memo(AutoTraderOutcomesComponent);
