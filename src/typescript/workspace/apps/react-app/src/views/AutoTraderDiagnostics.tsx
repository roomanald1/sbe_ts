import { useEffect, useMemo, useState } from 'react';
import { AgGridReact } from 'ag-grid-react';
import {
  ClientSideRowModelModule,
  ModuleRegistry,
  themeQuartz,
  type ColDef,
  type ICellRendererParams,
} from 'ag-grid-community';
import type {
  LatestEntryDiagnostic,
  StrategyDiagnostic,
} from '../models/AutoTraderModel';
import { useBehaviorSnapshot } from '@workspace/ui-platform';
import { FeedToolbar } from './FeedToolbar';
import {
  AutoTraderDiagnosticsModel,
  candidateFunnelRecords,
  groupCandidateFunnelRecords,
  MAX_FUNNEL_CYCLES,
  type FunnelCycle,
  type DiagnosticsRow,
} from '../models/AutoTraderDiagnosticsModel';

ModuleRegistry.registerModules([ClientSideRowModelModule]);
const diagnosticsGridTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});
type StrategyDiagnosticRow = StrategyDiagnostic['symbols'][number];
const ENTRY_DIAGNOSTIC_STALE_AFTER_MS = 60_000;

export function BtcRegimeSourceLabel({
  diagnostic,
}: {
  diagnostic: StrategyDiagnostic;
}) {
  if (!diagnostic.btc_gate_required) return null;
  const sourceTime = diagnostic.btc_source_close_time_ms;
  if (typeof sourceTime !== 'number' || !Number.isFinite(sourceTime)) {
    return <span>BTC source anchor unavailable</span>;
  }
  const sourceLabel =
    diagnostic.btc_source_timeframe === 'monthly_rebalance_anchor_daily'
      ? 'prior completed monthly rebalance anchor (daily close)'
      : diagnostic.btc_source_timeframe === 'daily'
        ? 'latest completed daily close'
        : `BTC source ${diagnostic.btc_source_timeframe ?? 'timeframe unknown'}`;
  return (
    <span>
      BTC source: {sourceLabel} · {new Date(sourceTime).toISOString()}
    </span>
  );
}

function entryOutcomeLabel(outcome: LatestEntryDiagnostic['outcome']): string {
  switch (outcome) {
    case 'no_signals':
      return 'No signals received';
    case 'no_eligible_candidates':
      return 'No eligible candidates';
    case 'no_fresh_candidates':
      return 'No fresh candidates';
    case 'candidates_rejected':
      return 'Candidates rejected by entry gates';
    case 'execution_blocked':
      return 'Selected candidate blocked during execution';
    case 'global_risk_block':
      return 'Blocked by global risk limit';
    case 'order_queued':
      return 'Order queued';
    case 'order_submitted':
      return 'Order accepted by execution service';
    case 'waiting_for_strategy_signals':
      return 'Waiting for strategy signals';
    case 'cycle_error':
      return 'Entry cycle failed';
  }
}

function entryDiagnosticFreshness(
  timestampMs: number,
  nowMs: number,
): {
  label: string;
  stale: boolean;
} {
  const ageMs = nowMs - timestampMs;
  if (!Number.isFinite(timestampMs) || timestampMs <= 0 || ageMs < 0)
    return { label: 'Timestamp unavailable', stale: true };
  if (ageMs > ENTRY_DIAGNOSTIC_STALE_AFTER_MS)
    return {
      label: `Stale · last update ${Math.floor(ageMs / 1000)}s ago`,
      stale: true,
    };
  if (ageMs < 1_000) return { label: 'Updated just now', stale: false };
  return { label: `Updated ${Math.floor(ageMs / 1000)}s ago`, stale: false };
}

type GateColumnParams = ICellRendererParams<DiagnosticsRow> & {
  onToggle: (symbol: string, direction: string) => void;
};
function GateColumnCell({ data, onToggle }: GateColumnParams) {
  if (!data) return null;
  if (data.kind === 'gate')
    return <span className="auto-trader-detail-gate">{data.gate}</span>;
  return (
    <button
      className="auto-trader-detail-toggle"
      type="button"
      onClick={(event) => {
        event.stopPropagation();
        onToggle(data.symbol, data.direction);
      }}
    >
      {data.expanded
        ? 'Hide gates'
        : 'Show ' +
          data.gates.length +
          ' gate' +
          (data.gates.length === 1 ? '' : 's')}
    </button>
  );
}

function explicitEligibility(value: boolean | null | undefined): string {
  if (value === true) return 'Eligible';
  if (value === false) return 'Not eligible';
  return 'Unknown';
}

export function DecisionFunnelPanel({
  cycles,
  availableFromApiCount = cycles.length,
  displayedCycleCount = cycles.length,
  nowMs = Date.now(),
}: {
  cycles: readonly FunnelCycle[];
  availableFromApiCount?: number;
  displayedCycleCount?: number;
  nowMs?: number;
}) {
  const [dimension, setDimension] = useState<
    'symbol' | 'strategy' | 'symbol_strategy'
  >('symbol_strategy');
  const [symbolFilter, setSymbolFilter] = useState('');
  const [strategyFilter, setStrategyFilter] = useState('');
  const [cycleFilter, setCycleFilter] = useState('all');
  const allRecords = useMemo(() => candidateFunnelRecords(cycles), [cycles]);
  const symbols = Array.from(
    new Set(allRecords.map(({ candidate }) => candidate.symbol)),
  ).sort();
  const strategies = Array.from(
    new Set(
      allRecords
        .map(({ candidate }) => candidate.strategy)
        .filter((strategy): strategy is string => Boolean(strategy)),
    ),
  ).sort();
  const records = allRecords.filter(
    ({ candidate, cycleId }) =>
      (!symbolFilter || candidate.symbol === symbolFilter) &&
      (!strategyFilter || candidate.strategy === strategyFilter) &&
      (cycleFilter === 'all' || cycleId === cycleFilter),
  );
  const groups = groupCandidateFunnelRecords(records, dimension);
  const selectedCycles =
    cycleFilter === 'all'
      ? cycles
      : cycles.filter((cycle) => cycle.cycleId === cycleFilter);

  return (
    <section
      className="auto-trader-diagnostics-block auto-trader-decision-funnel v35-operator-card"
      aria-label="Decision funnel diagnostics"
    >
      <div className="auto-trader-diagnostics-heading">
        <strong>Decision funnel diagnostics</strong>
        <span>
          Showing {displayedCycleCount} of {availableFromApiCount} cycle reports
          returned by API (view cap {MAX_FUNNEL_CYCLES});{' '}
          {availableFromApiCount > displayedCycleCount
            ? 'older backend history availability is unknown'
            : 'server history completeness is unknown'}
          {' · '}candidate detail records: {allRecords.length}
        </span>
      </div>
      <p className="auto-trader-diagnostics-note">
        This view reports only recorded populations. Candidate detail rows are
        not complete signal or eligibility denominators. Missing backend
        populations and unclassified eligibility remain unavailable.
      </p>

      <div className="auto-trader-funnel-filters" aria-label="Funnel filters">
        <label>
          Group by
          <select
            aria-label="Group candidates by"
            value={dimension}
            onChange={(event) =>
              setDimension(
                event.target.value as 'symbol' | 'strategy' | 'symbol_strategy',
              )
            }
          >
            <option value="symbol_strategy">Symbol and strategy</option>
            <option value="symbol">Symbol</option>
            <option value="strategy">Strategy</option>
          </select>
        </label>
        <label>
          Cycle
          <select
            aria-label="Filter by cycle"
            value={cycleFilter}
            onChange={(event) => setCycleFilter(event.target.value)}
          >
            <option value="all">All retained cycles</option>
            {cycles
              .slice()
              .reverse()
              .map((cycle, index) => (
                <option
                  key={`${cycle.cycleId}:${cycle.timestampMs}:${index}`}
                  value={cycle.cycleId}
                >
                  {cycle.cycleId}
                </option>
              ))}
          </select>
        </label>
        <label>
          Symbol
          <select
            aria-label="Filter by symbol"
            value={symbolFilter}
            onChange={(event) => setSymbolFilter(event.target.value)}
          >
            <option value="">All recorded symbols</option>
            {symbols.map((symbol) => (
              <option key={symbol} value={symbol}>
                {symbol}
              </option>
            ))}
          </select>
        </label>
        <label>
          Strategy
          <select
            aria-label="Filter by strategy"
            value={strategyFilter}
            onChange={(event) => setStrategyFilter(event.target.value)}
          >
            <option value="">All recorded strategies</option>
            {strategies.map((strategy) => (
              <option key={strategy} value={strategy}>
                {strategy}
              </option>
            ))}
          </select>
        </label>
      </div>

      {selectedCycles.map((cycle, index) => (
        <div
          className="auto-trader-funnel-cycle"
          key={`${cycle.cycleId}:${cycle.timestampMs}:${index}`}
        >
          <strong>
            Cycle {cycle.cycleId} ·{' '}
            {new Date(cycle.timestampMs).toLocaleString()}
          </strong>
          <span>
            {entryDiagnosticFreshness(cycle.timestampMs, nowMs).label}
          </span>
          <span>
            Primary blocker:{' '}
            {cycle.primaryBlocker ?? cycle.primaryGateId ?? 'None reported'}
          </span>
          <span>
            Candidate detail coverage:{' '}
            {cycle.candidateCoverage
              ? `${cycle.candidateCoverage.state}; ${cycle.candidateCoverage.retained} retained${typeof cycle.candidateCoverage.observed === 'number' ? ` of ${cycle.candidateCoverage.observed} observed` : ''}; ${cycle.candidateCoverage.truncated ? 'truncated' : 'not marked truncated'}`
              : 'unavailable'}
          </span>
        </div>
      ))}

      <div className="data-grid-wrap">
        <table className="auto-trader-funnel-table">
          <caption>
            Reported portfolio cycle counters; these populations do not share a
            denominator
          </caption>
          <thead>
            <tr>
              <th scope="col">Cycle</th>
              <th scope="col">Raw signals</th>
              <th scope="col">Fresh directional before final gates</th>
              <th scope="col">Selected</th>
              <th scope="col">Order attempts</th>
              <th scope="col">Submitted</th>
              <th scope="col">Fill confirmation</th>
            </tr>
          </thead>
          <tbody>
            {selectedCycles.map((cycle, index) => (
              <tr key={`${cycle.cycleId}:${cycle.timestampMs}:${index}`}>
                <th scope="row">{cycle.cycleId}</th>
                <td>{cycle.rawSignalCount}</td>
                <td>{cycle.eligibleCandidateCount}</td>
                <td>{cycle.selectedCandidateCount}</td>
                <td>{cycle.orderAttemptCount}</td>
                <td>{cycle.submittedOrderCount}</td>
                <td>{cycle.fillConfirmed ? 'Confirmed' : 'Not confirmed'}</td>
              </tr>
            ))}
            {!selectedCycles.length && (
              <tr>
                <td colSpan={7}>Cycle counters unavailable.</td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      <div className="data-grid-wrap">
        <table className="auto-trader-funnel-table">
          <caption>
            Recorded candidate diagnostic rows by {dimension.replace('_', ' ')}
          </caption>
          <thead>
            <tr>
              <th scope="col">Group</th>
              <th scope="col">Decision records</th>
              <th scope="col">Strict eligibility</th>
              <th scope="col">Exploratory eligibility</th>
              <th scope="col">Actual selection modes</th>
            </tr>
          </thead>
          <tbody>
            {groups.map((group) => {
              const strictKnown = group.records.filter(
                ({ candidate }) => candidate.strictEligibility != null,
              );
              const exploratoryKnown = group.records.filter(
                ({ candidate }) => candidate.exploratoryEligibility != null,
              );
              const modes = new Map<string, number>();
              for (const { candidate } of group.records) {
                if (candidate.selectionMode != null) {
                  modes.set(
                    candidate.selectionMode,
                    (modes.get(candidate.selectionMode) ?? 0) + 1,
                  );
                }
              }
              const eligibleCount = (
                values: typeof group.records,
                field: 'strictEligibility' | 'exploratoryEligibility',
              ) =>
                values.filter(({ candidate }) => candidate[field] === true)
                  .length;
              return (
                <tr key={group.key}>
                  <th scope="row">{group.key}</th>
                  <td>{group.records.length}</td>
                  <td>
                    {strictKnown.length
                      ? `${eligibleCount(group.records, 'strictEligibility')} eligible / ${strictKnown.length} classified`
                      : 'Unavailable'}
                  </td>
                  <td>
                    {exploratoryKnown.length
                      ? `${eligibleCount(group.records, 'exploratoryEligibility')} eligible / ${exploratoryKnown.length} classified`
                      : 'Unavailable'}
                  </td>
                  <td>
                    {modes.size
                      ? Array.from(
                          modes,
                          ([mode, count]) => `${mode}: ${count}`,
                        ).join(', ')
                      : 'Unavailable'}
                  </td>
                </tr>
              );
            })}
            {!groups.length && (
              <tr>
                <td colSpan={5}>
                  {cycles.length
                    ? 'No candidate detail records are available for these filters.'
                    : 'Decision funnel data is unavailable; no entry-cycle reports were received.'}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      <details className="auto-trader-diagnostics-block">
        <summary className="auto-trader-diagnostics-heading">
          <strong>Candidate decisions</strong>
          <span>{records.length} recorded detail rows</span>
        </summary>
        {records.length ? (
          <ul className="auto-trader-gate-list">
            {records.map(({ cycleId, cycleTimestampMs, candidate }, index) => (
              <li
                key={`${cycleId}:${candidate.decisionId}:${candidate.strategy ?? ''}:${candidate.symbol}:${index}`}
              >
                <span>
                  <strong>{candidate.symbol}</strong>
                  {candidate.strategy ? ` · ${candidate.strategy}` : ''}
                  {candidate.profileId
                    ? ` · profile ${candidate.profileId}`
                    : ''}
                  {candidate.workerEventId
                    ? ` · event ${candidate.workerEventId}`
                    : ''}
                  {` · decision ${candidate.decisionId} · cycle ${cycleId}`}
                </span>
                <strong>
                  Stage: {candidate.stage} · mode:{' '}
                  {candidate.selectionMode ?? 'Unknown'} · strict:{' '}
                  {explicitEligibility(candidate.strictEligibility)} ·
                  exploratory:{' '}
                  {explicitEligibility(candidate.exploratoryEligibility)}
                </strong>
                {candidate.reasons.length ? (
                  <span>
                    Reported reasons (unordered): {candidate.reasons.join('; ')}
                  </span>
                ) : (
                  <span>No candidate reasons reported.</span>
                )}
                {(candidate.measurements?.length ?? 0) > 0 && (
                  <ul>
                    {candidate.measurements!.map((measurement, index) => (
                      <li key={`${measurement.gateId}:${index}`}>
                        {measurement.gateId}: observed {measurement.observed}{' '}
                        {measurement.unit ?? ''} {measurement.comparator}{' '}
                        {measurement.required} {measurement.unit ?? ''} ·{' '}
                        {measurement.passed == null
                          ? 'result unavailable'
                          : measurement.passed
                            ? 'passed'
                            : 'failed'}
                      </li>
                    ))}
                  </ul>
                )}
                <details>
                  <summary>Observed, required and checks</summary>
                  <pre>
                    {JSON.stringify(
                      {
                        observed: candidate.observed,
                        required: candidate.required,
                        checks: candidate.checks,
                        evidenceContract: candidate.evidenceContract,
                        timestampMs: cycleTimestampMs,
                      },
                      null,
                      2,
                    )}
                  </pre>
                </details>
              </li>
            ))}
          </ul>
        ) : (
          <p className="auto-trader-diagnostics-empty" role="status">
            Candidate drill-down is unavailable for the selected filters; no
            recorded candidate rows are being treated as zero generated signals.
          </p>
        )}
      </details>
    </section>
  );
}

export function AutoTraderDiagnostics() {
  const model = useMemo(() => new AutoTraderDiagnosticsModel(), []);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  const state = useBehaviorSnapshot(model.state$);
  const [nowMs, setNowMs] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNowMs(Date.now()), 5_000);
    return () => window.clearInterval(timer);
  }, []);
  const { diagnosticRows: gridRows, symbolGroupCount, funnelHistory } = state;
  const entryDiagnostic = state.latestEntryDiagnostic;
  const workerResults = state.latestSymbolEntryResults
    .slice()
    .sort(
      (left, right) => (right.timestamp_ms ?? 0) - (left.timestamp_ms ?? 0),
    );
  const entryFreshness = entryDiagnostic
    ? entryDiagnosticFreshness(entryDiagnostic.timestampMs, nowMs)
    : null;
  const primaryGates = state.gateHits
    .slice()
    .sort((a, b) => b.count - a.count)
    .slice(0, 5);
  const strategyColumns = useMemo<ColDef<StrategyDiagnosticRow>[]>(
    () => [
      { field: 'symbol', headerName: 'Symbol', minWidth: 110, flex: 0.7 },
      {
        field: 'close',
        headerName: 'Daily close',
        minWidth: 110,
        valueFormatter: ({ value }) =>
          typeof value === 'number' ? value.toPrecision(8) : '—',
      },
      {
        field: 'primary_rule',
        headerName: 'Primary rule',
        minWidth: 150,
        flex: 1,
      },
      {
        field: 'primary_level',
        headerName: 'Primary level',
        minWidth: 110,
        valueFormatter: ({ value }) =>
          typeof value === 'number' && value > 0 ? value.toPrecision(8) : '—',
      },
      {
        field: 'primary_distance_percent',
        headerName: 'Distance to primary',
        minWidth: 150,
        valueFormatter: ({ value, data }) =>
          typeof value === 'number'
            ? `${Math.abs(value).toFixed(2)} ${data?.primary_distance_unit ?? '%'} ${value >= 0 ? 'above' : 'below'}`
            : '—',
      },
      {
        field: 'secondary_rule',
        headerName: 'Secondary rule',
        minWidth: 140,
        valueFormatter: ({ value }) => value ?? '—',
      },
      {
        field: 'secondary_level',
        headerName: 'Secondary level',
        minWidth: 115,
        valueFormatter: ({ value }) =>
          typeof value === 'number' ? value.toPrecision(8) : '—',
      },
      {
        field: 'secondary_distance_percent',
        headerName: 'Distance to secondary',
        minWidth: 165,
        valueFormatter: ({ value }) =>
          typeof value === 'number'
            ? Math.abs(value).toFixed(2) +
              '% ' +
              (value >= 0 ? 'above' : 'below')
            : '—',
      },
      {
        field: 'signal_active',
        headerName: 'Signal',
        minWidth: 95,
        valueFormatter: ({ value }) => (value ? 'Active' : 'No signal'),
      },
    ],
    [],
  );
  const symbolColumns = useMemo<ColDef<DiagnosticsRow>[]>(
    () => [
      {
        field: 'symbol',
        headerName: 'Symbol',
        minWidth: 120,
        flex: 1,
        sort: 'asc',
        sortIndex: 0,
        comparator: (a, b, left, right) => {
          const bySymbol = String(a ?? '').localeCompare(String(b ?? ''));
          if (bySymbol) return bySymbol;
          const leftRow = left.data;
          const rightRow = right.data;
          if (!leftRow || !rightRow) return 0;
          const byDirection = leftRow.direction.localeCompare(
            rightRow.direction,
          );
          if (byDirection) return byDirection;
          if (leftRow.kind !== rightRow.kind)
            return leftRow.kind === 'symbol' ? -1 : 1;
          return leftRow.kind === 'gate' && rightRow.kind === 'gate'
            ? leftRow.gate.localeCompare(rightRow.gate)
            : 0;
        },
        cellRenderer: (params: ICellRendererParams<DiagnosticsRow>) =>
          params.data?.kind === 'symbol' ? params.value : '',
      },
      {
        field: 'direction',
        headerName: 'Direction',
        minWidth: 100,
        valueFormatter: ({ value, data }) =>
          data?.kind === 'symbol' ? String(value ?? '').toUpperCase() : '',
      },
      {
        field: 'distance',
        headerName: 'Distance to pass',
        minWidth: 145,
        valueFormatter: ({ value, data }) =>
          data?.kind !== 'symbol'
            ? ''
            : value == null
              ? 'Unavailable'
              : Number(value).toFixed(2) + ' gap',
      },
      {
        field: 'readinessScore',
        headerName: 'Readiness',
        minWidth: 110,
        valueFormatter: ({ value, data }) =>
          data?.kind === 'symbol' ? String(value ?? 0) + '/100' : '',
      },
      {
        field: 'gate',
        colId: 'gate',
        headerName: 'Gate',
        minWidth: 170,
        sortable: false,
        filter: false,
        cellRenderer: GateColumnCell,
        cellRendererParams: {
          onToggle: (symbol: string, direction: string) =>
            model.toggleGroup(symbol, direction),
        },
      },
      {
        field: 'count',
        headerName: 'Blocked',
        minWidth: 90,
        sortable: false,
        filter: false,
        valueFormatter: ({ value }) =>
          value == null ? '' : Number(value).toLocaleString(),
      },
      {
        field: 'detail',
        headerName: 'Latest detail',
        minWidth: 280,
        flex: 2,
        sortable: false,
        filter: false,
        tooltipField: 'detail',
        cellClass: 'auto-trader-nested-detail',
        wrapText: true,
        autoHeight: true,
      },
    ],
    [],
  );

  return (
    <section
      className="feed-section auto-trader-diagnostics"
      aria-label="Auto Trader diagnostics"
    >
      <FeedToolbar title="Auto Trader diagnostics" />
      <div className="auto-trader-diagnostics-content">
        <DecisionFunnelPanel
          cycles={funnelHistory}
          availableFromApiCount={state.availableFromApiCount}
          displayedCycleCount={state.displayedCycleCount}
          nowMs={nowMs}
        />
        <section
          className="auto-trader-diagnostics-block"
          aria-label="Latest entry outcome"
          aria-live="polite"
        >
          <div className="auto-trader-diagnostics-heading">
            <strong>Latest entry outcome</strong>
            {entryDiagnostic && entryFreshness && (
              <span
                role={entryFreshness.stale ? 'alert' : undefined}
                aria-label={`Diagnostic freshness: ${entryFreshness.label}`}
              >
                {entryFreshness.label}
              </span>
            )}
          </div>
          {entryDiagnostic ? (
            <>
              <h3>{entryOutcomeLabel(entryDiagnostic.outcome)}</h3>
              <p className="auto-trader-diagnostics-note">
                <strong>Primary blocker: </strong>
                {entryDiagnostic.primaryBlocker || 'None reported'}
                {entryDiagnostic.primaryGateId
                  ? ` · Gate: ${entryDiagnostic.primaryGateId}`
                  : ''}
              </p>
              {entryDiagnostic.detail &&
                entryDiagnostic.detail !== entryDiagnostic.primaryBlocker && (
                  <p className="auto-trader-diagnostics-note">
                    <strong>Cycle detail: </strong>
                    {entryDiagnostic.detail}
                  </p>
                )}
              <dl className="auto-trader-entry-funnel">
                <div>
                  <dt>Signals</dt>
                  <dd>{entryDiagnostic.rawSignalCount}</dd>
                </div>
                <div>
                  <dt>Allowed</dt>
                  <dd>{entryDiagnostic.allowedSignalCount}</dd>
                </div>
                <div>
                  <dt>Strategy candidates</dt>
                  <dd>{entryDiagnostic.strategyCandidateCount}</dd>
                </div>
                <div>
                  <dt>Fresh directional</dt>
                  <dd>
                    {entryDiagnostic.freshDirectionalCandidateCount ?? '—'}
                  </dd>
                </div>
                <div>
                  <dt>Stale</dt>
                  <dd>{entryDiagnostic.staleCandidateCount ?? '—'}</dd>
                </div>
                <div>
                  <dt>Non directional</dt>
                  <dd>{entryDiagnostic.nonDirectionalCandidateCount ?? '—'}</dd>
                </div>
                <div>
                  <dt>Position blocked</dt>
                  <dd>
                    {entryDiagnostic.positionBlockedCandidateCount ?? '—'}
                  </dd>
                </div>
                <div>
                  <dt>Strategy rejected</dt>
                  <dd>
                    {entryDiagnostic.strategyRejectedCandidateCount ?? '—'}
                  </dd>
                </div>
                <div>
                  <dt>Walk-forward field present</dt>
                  <dd>
                    {entryDiagnostic.evidencePresentCandidateCount ?? '—'}
                  </dd>
                </div>
                <div>
                  <dt>Fresh-directional preselection count</dt>
                  <dd>{entryDiagnostic.eligibleCandidateCount}</dd>
                </div>
                <div>
                  <dt>Rejected</dt>
                  <dd>{entryDiagnostic.rejectedCandidateCount}</dd>
                </div>
                <div>
                  <dt>Selected</dt>
                  <dd>{entryDiagnostic.selectedCandidateCount}</dd>
                </div>
                <div>
                  <dt>Order attempts</dt>
                  <dd>{entryDiagnostic.orderAttemptCount}</dd>
                </div>
                <div>
                  <dt>Orders submitted</dt>
                  <dd>{entryDiagnostic.submittedOrderCount}</dd>
                </div>
                <div>
                  <dt>Trade fill</dt>
                  <dd>
                    {entryDiagnostic.fillConfirmed
                      ? 'Confirmed'
                      : 'Not confirmed'}
                  </dd>
                </div>
              </dl>
              {(entryDiagnostic.gateCounts?.length ?? 0) > 0 && (
                <p className="auto-trader-diagnostics-note">
                  <strong>Gate counts this cycle: </strong>
                  {entryDiagnostic
                    .gateCounts!.map(({ gate, count }) => `${gate} (${count})`)
                    .join(' · ')}
                </p>
              )}
              {(entryDiagnostic.candidateGateCounts?.length ?? 0) > 0 && (
                <ul className="auto-trader-gate-list">
                  {entryDiagnostic.candidateGateCounts!.map((item) => (
                    <li
                      key={`${item.gateId}:${item.symbol}:${item.strategy ?? ''}`}
                    >
                      <span>
                        {item.gateId} · {item.symbol}
                        {item.strategy ? ` · ${item.strategy}` : ''}
                      </span>
                      <strong>
                        {item.count} candidate{item.count === 1 ? '' : 's'}
                      </strong>
                    </li>
                  ))}
                </ul>
              )}
              {(entryDiagnostic.candidateDetails?.length ?? 0) > 0 && (
                <details className="auto-trader-diagnostics-block">
                  <summary className="auto-trader-diagnostics-heading">
                    <strong>Candidate checks</strong>
                    <span>
                      {entryDiagnostic.candidateDetails!.length} candidates
                    </span>
                  </summary>
                  <ul className="auto-trader-gate-list">
                    {entryDiagnostic.candidateDetails!.map(
                      (candidate, index) => (
                        <li
                          key={`${candidate.decisionId}:${candidate.strategy ?? ''}:${candidate.symbol}:${candidate.profileId ?? ''}:${index}`}
                        >
                          <span>
                            <strong>
                              {candidate.strategy ?? 'Unattributed'}
                            </strong>
                            {' · '}
                            {candidate.symbol}
                            {candidate.profileId
                              ? ` · profile ${candidate.profileId}`
                              : ''}
                            {candidate.workerEventId
                              ? ` · event ${candidate.workerEventId}`
                              : ''}
                          </span>
                          <strong>
                            {candidate.reasons.length
                              ? candidate.reasons.join('; ')
                              : 'No preparation-stage threshold failure'}
                          </strong>
                          <pre>
                            {JSON.stringify(
                              {
                                observed: candidate.observed,
                                required: candidate.required,
                                checks: candidate.checks,
                                measurements: candidate.measurements ?? [],
                                evidenceContract: candidate.evidenceContract,
                              },
                              null,
                              2,
                            )}
                          </pre>
                        </li>
                      ),
                    )}
                  </ul>
                </details>
              )}
              <p className="auto-trader-diagnostics-note">
                Cycle {entryDiagnostic.cycleId} ·{' '}
                {new Date(entryDiagnostic.timestampMs).toLocaleString()}
              </p>
            </>
          ) : (
            <p className="auto-trader-diagnostics-empty" role="status">
              {state.loading
                ? 'Loading entry diagnostics…'
                : state.latestSignalDiagnostic
                  ? `No entry cycle was reached. Latest signal blocker: ${state.latestSignalDiagnostic}`
                  : 'Diagnostics unavailable: no entry-cycle report has been recorded yet.'}
            </p>
          )}
          {state.entryDiagnosticHistory.length > 1 && (
            <details className="auto-trader-diagnostics-block">
              <summary className="auto-trader-diagnostics-heading">
                <strong>Recent entry cycles</strong>
                <span>{state.entryDiagnosticHistory.length} retained</span>
              </summary>
              <ol>
                {state.entryDiagnosticHistory
                  .slice(-10)
                  .reverse()
                  .map((cycle, index) => (
                    <li key={`${cycle.cycleId}:${cycle.timestampMs}:${index}`}>
                      <strong>{entryOutcomeLabel(cycle.outcome)}</strong>
                      {' · '}
                      {new Date(cycle.timestampMs).toLocaleString()}
                      {cycle.primaryBlocker ? ` · ${cycle.primaryBlocker}` : ''}
                    </li>
                  ))}
              </ol>
            </details>
          )}
          <details className="auto-trader-diagnostics-block">
            <summary className="auto-trader-diagnostics-heading">
              <strong>Worker profile decisions and skips</strong>
              <span>{workerResults.length} rows</span>
            </summary>
            {workerResults.length ? (
              <ul className="auto-trader-gate-list">
                {workerResults.slice(0, 30).map((result, index) => (
                  <li
                    key={`${result.strategy}:${result.symbol}:${result.profile_id ?? 'unscoped'}:${result.event_id ?? ''}:${result.timestamp_ms ?? 0}:${index}`}
                  >
                    <span>
                      <strong>{result.strategy}</strong> · {result.symbol}
                      {result.profile_id
                        ? ` · profile ${result.profile_id}`
                        : ''}
                      {result.event_id ? ` · event ${result.event_id}` : ''}
                      {result.timestamp_ms
                        ? ` · ${entryDiagnosticFreshness(result.timestamp_ms, nowMs).label}`
                        : ' · no report received'}
                    </span>
                    <strong>
                      {result.decision ?? 'Skipped'} · {result.reason}
                    </strong>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="auto-trader-diagnostics-empty" role="status">
                Worker diagnostics unavailable: no profile decision or skip
                report has been received.
              </p>
            )}
          </details>
          <div className="auto-trader-diagnostics-heading">
            <strong>Most frequent blocked gates</strong>
          </div>
          {primaryGates.length ? (
            <ul className="auto-trader-gate-list">
              {primaryGates.map((item) => (
                <li key={item.gate}>
                  <span>{item.gate}</span>
                  <strong>{item.count.toLocaleString()} scans</strong>
                </li>
              ))}
            </ul>
          ) : (
            <p className="auto-trader-diagnostics-empty">
              No blocked gates are recorded in the current diagnostic window.
            </p>
          )}
        </section>
        <p className="auto-trader-diagnostics-note">
          Gate hit counts are blocked decision scans since the current UTC day
          began; each gate is counted at most once per scan, even when several
          candidates failed it. Counts reset after a submitted trade or at UTC
          day rollover. Readiness is a ranking index: numeric blockers add their
          normalized gap (at least 0.05), and qualitative blockers add one unit.
          A submitted entry is 100/100 after passing the gates. Distance
          separately shows the largest normalized numeric shortfall.
        </p>
        <details className="auto-trader-diagnostics-block auto-trader-gate-hits">
          <summary className="auto-trader-diagnostics-heading">
            <strong>Gate hits</strong>
            <span>{state.decisionsSinceTrade.toLocaleString()} scans</span>
          </summary>
          {state.gateHits.length > 0 ? (
            <ul className="auto-trader-gate-list">
              {state.gateHits
                .slice()
                .sort((a, b) => b.count - a.count)
                .map((item) => (
                  <li key={item.gate}>
                    <span>{item.gate}</span>
                    <strong>{item.count.toLocaleString()} scans</strong>
                  </li>
                ))}
            </ul>
          ) : (
            <p className="auto-trader-diagnostics-empty">
              {state.loading
                ? 'Loading current gate counts…'
                : 'No blocked gates recorded for this running session.'}
            </p>
          )}
        </details>

        <section
          className="auto-trader-diagnostics-block"
          aria-label="Daily strategy decisions"
        >
          <div className="auto-trader-diagnostics-heading">
            <strong>Daily strategy diagnostics</strong>
            <span>
              {
                state.strategyDiagnostics.filter((item) =>
                  state.strategies.includes(item.strategy),
                ).length
              }{' '}
              enabled strategies
            </span>
          </div>
          {state.strategyDiagnostics.filter((item) =>
            state.strategies.includes(item.strategy),
          ).length > 0 ? (
            state.strategyDiagnostics
              .filter((item) => state.strategies.includes(item.strategy))
              .map((diagnostic) => (
                <section
                  key={diagnostic.strategy}
                  className="auto-trader-strategy-diagnostic"
                >
                  <div className="auto-trader-diagnostics-heading">
                    <strong>{diagnostic.strategy}</strong>
                    <span>
                      Evaluated {new Date(diagnostic.as_of_ms).toLocaleString()}
                    </span>
                  </div>
                  <p className="auto-trader-diagnostics-note">
                    {diagnostic.btc_gate_required
                      ? 'BTC ' +
                        diagnostic.btc_window_days +
                        '-day SMA gate: ' +
                        (diagnostic.btc_gate_passed ? 'pass' : 'blocked') +
                        ' · BTC close ' +
                        diagnostic.btc_close?.toPrecision(8) +
                        ' vs SMA ' +
                        diagnostic.btc_sma?.toPrecision(8) +
                        ' · ' +
                        Math.abs(diagnostic.btc_distance_percent ?? 0).toFixed(
                          2,
                        ) +
                        '% ' +
                        ((diagnostic.btc_distance_percent ?? 0) >= 0
                          ? 'above'
                          : 'below') +
                        '.'
                      : 'This strategy does not use a BTC regime gate.'}{' '}
                    {diagnostic.btc_gate_required ? (
                      <>
                        <BtcRegimeSourceLabel diagnostic={diagnostic} />{' '}
                      </>
                    ) : null}
                    Symbol distances use the evaluated symbol close against each
                    strategy threshold; negative values are below the level.
                  </p>
                  <div className="data-grid-wrap auto-trader-symbol-grid">
                    <AgGridReact<StrategyDiagnosticRow>
                      animateRows={false}
                      rowData={diagnostic.symbols}
                      columnDefs={strategyColumns}
                      defaultColDef={{
                        sortable: true,
                        resizable: true,
                        filter: true,
                        minWidth: 85,
                      }}
                      theme={diagnosticsGridTheme}
                      domLayout="autoHeight"
                      rowHeight={34}
                      headerHeight={29}
                      getRowId={({ data }) => data.symbol}
                    />
                  </div>
                </section>
              ))
          ) : (
            <p className="auto-trader-diagnostics-empty">
              Daily candle diagnostics for enabled strategies will appear after
              the next successful evaluation.
            </p>
          )}
        </section>

        <section
          className="auto-trader-diagnostics-block"
          aria-label="Gate counts by symbol"
        >
          <div className="auto-trader-diagnostics-heading">
            <strong>By symbol and direction</strong>
            <span>{symbolGroupCount} groups</span>
          </div>
          {symbolGroupCount > 0 ? (
            <div className="data-grid-wrap auto-trader-symbol-grid">
              <AgGridReact<DiagnosticsRow>
                animateRows={false}
                rowData={gridRows}
                columnDefs={symbolColumns}
                defaultColDef={{
                  sortable: true,
                  resizable: true,
                  filter: true,
                  minWidth: 85,
                  cellStyle: ({ data }) => {
                    if (data?.distance == null)
                      return { backgroundColor: '#20252c', color: '#c1cad4' };
                    const hue = Math.round(
                      120 * (1 - Math.min(Math.max(data.distance, 0), 1)),
                    );
                    return {
                      backgroundColor: 'hsl(' + hue + ' 58% 19%)',
                      color: 'hsl(' + hue + ' 75% 83%)',
                    };
                  },
                }}
                theme={diagnosticsGridTheme}
                domLayout="autoHeight"
                rowHeight={34}
                headerHeight={29}
                getRowId={({ data }) =>
                  data.kind === 'symbol'
                    ? 'symbol:' + data.symbol + ':' + data.direction
                    : 'gate:' +
                      data.symbol +
                      ':' +
                      data.direction +
                      ':' +
                      data.gate
                }
                getRowClass={({ data }) =>
                  data?.kind === 'gate'
                    ? 'auto-trader-gate-detail-row'
                    : undefined
                }
              />
            </div>
          ) : (
            <p className="auto-trader-diagnostics-empty">
              Symbol-level counts cover signal quality and freshness checks plus
              post-selection market-data, position/cooldown, capacity,
              order-type, and deep-book gates when a candidate was selected.
            </p>
          )}
        </section>

        <section
          className="auto-trader-diagnostics-block"
          aria-label="Latest signal gate checks"
        >
          <div className="auto-trader-diagnostics-heading">
            <strong>Latest signal check</strong>
          </div>
          {state.latestSignalDiagnostic ? (
            <pre className="auto-trader-latest-check">
              {state.latestSignalDiagnostic}
            </pre>
          ) : (
            <p className="auto-trader-diagnostics-empty">
              Waiting for a completed decision cycle.
            </p>
          )}
        </section>

        {state.lastDecisionAt > 0 && (
          <time
            className="auto-trader-diagnostics-time"
            dateTime={new Date(state.lastDecisionAt).toISOString()}
          >
            Last decision{' '}
            {new Intl.DateTimeFormat(undefined, {
              hour: '2-digit',
              minute: '2-digit',
              second: '2-digit',
            }).format(state.lastDecisionAt)}{' '}
            · {state.lastDecision}
          </time>
        )}
      </div>
    </section>
  );
}
