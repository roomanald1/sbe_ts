import { useEffect, useMemo } from 'react';
import { AgGridReact } from 'ag-grid-react';
import {
  ClientSideRowModelModule,
  ModuleRegistry,
  themeQuartz,
  type ColDef,
} from 'ag-grid-community';
import { AutoTraderLearningWidgetModel } from '../models/AutoTraderLearningWidgetModel';
import type {
  LearningRow,
  TradeEffectivenessRow,
} from '../models/AutoTraderLearningWidgetModel';
import { useBehaviorSnapshot } from '@workspace/ui-platform';

ModuleRegistry.registerModules([ClientSideRowModelModule]);
const learningGridTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});

const signedPercentPoints = (value: number | null) =>
  value == null ? '—' : `${value > 0 ? '+' : ''}${value.toFixed(3)} pp`;

const tradeEffectivenessColumns: ColDef<TradeEffectivenessRow>[] = [
  { field: 'strategy', headerName: 'Strategy', pinned: 'left', width: 180 },
  { field: 'activeSamples', headerName: 'Current profile n', width: 105 },
  {
    field: 'activePositiveRate',
    headerName: 'Current win rate',
    width: 115,
    valueFormatter: ({ value }) =>
      Number.isFinite(value) ? `${(value * 100).toFixed(0)}%` : '—',
  },
  {
    field: 'activeAverage',
    headerName: 'Current avg net',
    width: 120,
    valueFormatter: ({ value }) => signedPercentPoints(value ?? null),
    cellClass: ({ value }) =>
      value > 0 ? 'learning-increase' : value < 0 ? 'learning-decrease' : '',
  },
  { field: 'otherSamples', headerName: 'Other profile n', width: 105 },
  {
    field: 'otherPositiveRate',
    headerName: 'Other win rate',
    width: 105,
    valueFormatter: ({ value }) =>
      Number.isFinite(value) ? `${(value * 100).toFixed(0)}%` : '—',
  },
  {
    field: 'otherAverage',
    headerName: 'Other avg net',
    width: 115,
    valueFormatter: ({ value }) => signedPercentPoints(value ?? null),
    cellClass: ({ value }) =>
      value > 0 ? 'learning-increase' : value < 0 ? 'learning-decrease' : '',
  },
  {
    field: 'averageDifference',
    headerName: 'Current − other',
    headerTooltip:
      'Difference between realized average net returns for trades using currently active profile parameters and trades using other parameter profiles in the last seven days. Descriptive only; it is not a causal or risk-adjusted estimate.',
    width: 125,
    valueFormatter: ({ value }) => signedPercentPoints(value ?? null),
    cellClass: ({ value }) =>
      value > 0 ? 'learning-increase' : value < 0 ? 'learning-decrease' : '',
  },
];

const columns: ColDef<LearningRow>[] = [
  { field: 'symbol', headerName: 'Symbol', pinned: 'left', width: 105 },
  { field: 'strategy', headerName: 'Strategy', width: 130 },
  { field: 'parameter', headerName: 'Parameter', width: 145 },
  {
    field: 'base',
    headerName: 'Configured/default',
    headerTooltip:
      'Configured override or strategy default. Paired tests compare against the baseline arm used for each scout, which is not necessarily this value.',
    width: 95,
    valueFormatter: ({ value, data }) =>
      value == null ? '—' : value.toFixed(3) + ' ' + (data?.unit ?? ''),
  },
  {
    field: 'candidate',
    headerName: 'Tested value',
    width: 105,
    valueFormatter: ({ value, data }) =>
      value == null ? '—' : value.toFixed(3) + ' ' + (data?.unit ?? ''),
  },
  {
    field: 'pairedDelta',
    colId: 'pairedDelta',
    headerName: 'Δ change',
    headerTooltip:
      'Mean tested value minus paired-control value. Green is an increase; red is a decrease. If the control was not retained, the asterisk marks an estimate against Configured/default.',
    width: 185,
    valueFormatter: ({ value, data }) => {
      if (!Number.isFinite(value) || !data) return '—';
      const amount = `${value > 0 ? '+' : ''}${value.toFixed(3)} ${data.unit}`;
      if (data.pairedDeltaEstimated)
        return `${amount}* · est n=${data.pairedDeltaSamples}`;
      const legacy = data.pairedUnknownSamples
        ? ` · ${data.pairedUnknownSamples} unknown`
        : '';
      return `${amount} · n=${data.pairedDeltaSamples}${legacy}`;
    },
    cellClass: ({ value }) =>
      value > 0
        ? 'learning-paired-up'
        : value < 0
          ? 'learning-paired-down'
          : 'learning-paired-neutral',
  },
  {
    field: 'averageLift',
    headerName: 'Avg paired lift',
    headerTooltip:
      'Mean paired variant-minus-baseline net return, in percentage points. A plus means better paired return, regardless of parameter direction.',
    width: 110,
    valueFormatter: ({ value }) =>
      Number.isFinite(value)
        ? (value > 0 ? '+' : '') + value.toFixed(3) + ' pp'
        : '—',
    cellClass: ({ value }) =>
      value > 0 ? 'learning-increase' : value < 0 ? 'learning-decrease' : '',
  },
  {
    field: 'lower95',
    headerName: '90% one-sided bound',
    headerTooltip:
      'One-sided 90% Student-t approximation lower bound for paired net improvement.',
    width: 125,
    valueFormatter: ({ value }) =>
      Number.isFinite(value)
        ? (value > 0 ? '+' : '') + value.toFixed(3) + ' pp'
        : '—',
    cellClass: ({ value }) =>
      value > 0 ? 'learning-increase' : value < 0 ? 'learning-decrease' : '',
  },
  {
    field: 'upper95',
    headerName: '90% upper bound',
    headerTooltip:
      'A negative one-sided 90% upper bound marks a candidate to avoid.',
    width: 115,
    valueFormatter: ({ value }) =>
      Number.isFinite(value)
        ? (value > 0 ? '+' : '') + value.toFixed(3) + ' pp'
        : '—',
    cellClass: ({ value }) =>
      value > 0 ? 'learning-increase' : value < 0 ? 'learning-decrease' : '',
  },
  {
    field: 'samples',
    headerName: 'Paired n',
    headerTooltip:
      'Paired scout comparisons pooled for this strategy, parameter, and candidate value.',
    width: 75,
  },
  { field: 'minimumSamples', headerName: 'Required n', width: 85 },
  {
    field: 'effective',
    headerName: 'Current active value',
    headerTooltip:
      'Current parameter value used by this strategy profile. Historical scout baselines can differ.',
    width: 110,
    valueFormatter: ({ value, data }) =>
      value == null ? '—' : value.toFixed(3) + ' ' + (data?.unit ?? ''),
  },
  {
    field: 'status',
    headerName: 'Outcome',
    width: 120,
    cellClass: ({ data }) =>
      data?.promoted
        ? 'learning-increase'
        : data?.rejected
          ? 'learning-decrease'
          : '',
  },
];

export function AutoTraderLearningWidget() {
  const model = useMemo(() => new AutoTraderLearningWidgetModel(), []);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  const {
    source: state,
    summary: learningSummary,
    summaryError: learningSummaryError,
    rows,
    visibleRows,
    promotedOnly,
    effectiveness,
    actualTradeRows,
    profileReturnDifference,
    activeCandidates,
    rejectedCandidates,
    collectingCandidates,
  } = useBehaviorSnapshot(model.state$);
  return (
    <section
      className="auto-trader-strategy-parameters"
      aria-label="AutoTrader learning impact"
    >
      <strong>Learning effectiveness · realized outcomes</strong>
      <p>
        This compares actual closed-trade returns from the currently active
        learned parameter profiles with actual trades taken under other
        parameter profiles over the last seven days. It answers whether live
        realized results are moving in the right direction; the comparison is
        descriptive, not a controlled causal test.
      </p>
      <div
        className="auto-trader-learning-overview"
        aria-label="Realized trade performance over the last seven days"
      >
        <div>
          <span>Current learned profile · actual trades</span>
          <strong>
            {learningSummary == null
              ? '—'
              : `${effectiveness.activeProfileTrades.toLocaleString()} trades`}
          </strong>
          <small>
            {effectiveness.activeProfilePositiveRate == null
              ? '—'
              : `${(effectiveness.activeProfilePositiveRate * 100).toFixed(0)}% positive`}
            {' · mean net '}
            {signedPercentPoints(effectiveness.activeProfileReturn)}
          </small>
        </div>
        <div>
          <span>Other parameter profiles · actual trades</span>
          <strong>
            {learningSummary == null
              ? '—'
              : `${effectiveness.otherProfileTrades.toLocaleString()} trades`}
          </strong>
          <small>
            {effectiveness.otherProfilePositiveRate == null
              ? '—'
              : `${(effectiveness.otherProfilePositiveRate * 100).toFixed(0)}% positive`}
            {' · mean net '}
            {signedPercentPoints(effectiveness.otherProfileReturn)}
          </small>
        </div>
        <div>
          <span>Active profile minus other profiles</span>
          <strong>{signedPercentPoints(profileReturnDifference)}</strong>
          <small>
            {effectiveness.activeProfileTrades === 0 ||
            effectiveness.otherProfileTrades === 0
              ? 'Need trades in both groups to compare'
              : 'Difference in mean realized net return'}
          </small>
        </div>
        <div>
          <span>Actual trade coverage · 7d</span>
          <strong>
            {learningSummary == null
              ? '—'
              : `${effectiveness.closedTrades.toLocaleString()} trades`}
          </strong>
          <small>Across active and other parameter profiles</small>
        </div>
      </div>
      <p className="auto-trader-learning-caveat">
        {!effectiveness.activeProfileTrades || !effectiveness.otherProfileTrades
          ? 'There is not enough realized-trade evidence in both groups to judge effectiveness yet.'
          : 'This is an observed comparison, not proof that Learning caused the difference. Trade count, market conditions, and risk-adjusted return matter; Scout simulations are excluded from these realized results.'}
      </p>
      {learningSummaryError && (
        <p className="auto-trader-learning-error" role="alert">
          Learning summary unavailable: {learningSummaryError}
        </p>
      )}
      {learningSummary == null ? (
        <p>
          {learningSummaryError
            ? 'Actual trade results could not be loaded.'
            : 'Loading actual trade results…'}
        </p>
      ) : actualTradeRows.length === 0 ? (
        <p>No closed-trade results are available for the last seven days.</p>
      ) : (
        <div className="data-grid-wrap auto-trader-learning-grid">
          <AgGridReact<TradeEffectivenessRow>
            rowData={actualTradeRows}
            columnDefs={tradeEffectivenessColumns}
            getRowId={({ data }) => data.strategy}
            defaultColDef={{
              sortable: true,
              resizable: true,
              filter: true,
              minWidth: 65,
            }}
            theme={learningGridTheme}
            domLayout="normal"
            rowHeight={30}
            headerHeight={28}
          />
        </div>
      )}
      <details className="auto-trader-learning-experiments">
        <summary>Scout parameter experiments · simulated evidence</summary>
        <p>
          Paired scout results show whether each tested parameter value beat its
          baseline in simulation. “Qualified” means the configured sample
          minimum and positive one-sided 90% Student-t lower bound are met;
          “Active” means the current profile uses that candidate. “Avoid” means
          the upper bound is below zero. These results guide profile selection
          but are not realized trading performance.
        </p>
        <div className="auto-trader-learning-overview">
          <div>
            <span>Paired Scout evidence · 7d</span>
            <strong>
              {learningSummary == null
                ? '—'
                : `${effectiveness.pairedSamples.toLocaleString()} samples`}
            </strong>
            <small>
              Mean variant-vs-control lift{' '}
              {signedPercentPoints(effectiveness.pairedLift)}
            </small>
          </div>
          <div>
            <span>Parameter decisions</span>
            <strong>
              {activeCandidates} active · {rejectedCandidates} avoided
            </strong>
            <small>{collectingCandidates} still collecting evidence</small>
          </div>
          <div>
            <span>Scout outcomes · 7d</span>
            <strong>
              {learningSummary == null
                ? '—'
                : `${effectiveness.scoutOutcomes.toLocaleString()} observations`}
            </strong>
            <small>
              Mean counterfactual return{' '}
              {signedPercentPoints(effectiveness.scoutReturn)}
            </small>
          </div>
        </div>
        {rows.length === 0 ? (
          <p>
            {state.loading
              ? 'Loading scout parameter results…'
              : 'No paired scout parameter results are available yet.'}
          </p>
        ) : (
          <>
            <button
              className="auto-trader-learning-filter"
              type="button"
              aria-pressed={promotedOnly}
              onClick={() => model.setPromotedOnly(!promotedOnly)}
            >
              {promotedOnly ? 'Show all tests' : 'Active only'}
            </button>
            {visibleRows.length === 0 ? (
              <p>No parameter variants are active yet.</p>
            ) : (
              <div className="data-grid-wrap auto-trader-learning-grid">
                <AgGridReact<LearningRow>
                  rowData={visibleRows}
                  columnDefs={columns}
                  getRowId={({ data }) => data.id}
                  defaultColDef={{
                    sortable: true,
                    resizable: true,
                    filter: true,
                    minWidth: 65,
                  }}
                  theme={learningGridTheme}
                  domLayout="normal"
                  rowHeight={30}
                  headerHeight={28}
                />
              </div>
            )}
          </>
        )}
      </details>
    </section>
  );
}
