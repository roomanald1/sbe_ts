import { useEffect, useMemo } from 'react';
import {
  OperatorMetricStrip,
  OperatorTabBar,
  useBehaviorSnapshot,
} from '@workspace/ui-platform';
import {
  AutoTraderExcursionsModel,
  EXCURSION_EVIDENCE_NOTICE,
  excursionPanelStatus,
  type ExcursionRange,
  type ExcursionSource,
  type ExcursionView,
} from '../models/AutoTraderExcursionsModel';
import { AutoTraderOutcomesModel } from '../models/AutoTraderOutcomesModel';

const outcomes = new AutoTraderOutcomesModel();
const tabs: { id: ExcursionView; label: string }[] = [
  { id: 'what-if', label: 'What-if' },
  { id: 'overview', label: 'Overview' },
  { id: 'continuation', label: 'Continuation' },
  { id: 'transitions', label: 'Transitions' },
  { id: 'sensitivity', label: 'Sensitivity' },
  { id: 'replay', label: 'Trade replay' },
];

export function AutoTraderExcursionsWidget() {
  const model = useMemo(() => new AutoTraderExcursionsModel(outcomes), []);
  const state = useBehaviorSnapshot(model.state$);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  const selected = state.trades.find(
    (trade) => trade.id === state.selectedTradeId,
  );
  const panelStatus = excursionPanelStatus(
    state.view,
    state.sourceCount,
    state.eligiblePathCount,
    state.trades.filter((trade) => trade.postExitAvailable).length,
  );
  const format = (value?: number, suffix = '') =>
    typeof value === 'number' && Number.isFinite(value)
      ? `${value.toFixed(3)}${suffix}`
      : 'Unavailable';
  const riskLabel = (row: (typeof state.trades)[number]) =>
    row.initialRiskEligible
      ? `${format(row.trade.initialRiskEvidence?.quoteAmount)} ${row.trade.initialRiskEvidence?.quoteCurrency ?? ''} · R eligible`
      : 'Unavailable · R ineligible';
  const timeline = (row: (typeof state.trades)[number]) => {
    const policy = row.trade.exitParameterProvenance;
    const barrier = row.trade.currentPmBarrier;
    return (
      [
        policy?.selected_at_ms
          ? `Selected ${new Date(policy.selected_at_ms).toLocaleString()}`
          : undefined,
        barrier ? `PM barrier ${barrier.status ?? 'present'}` : undefined,
        row.trade.profitProtection?.triggered
          ? 'Profit protection triggered'
          : undefined,
        row.trade.closedAt
          ? `Closed ${new Date(row.trade.closedAt).toLocaleString()}`
          : undefined,
      ]
        .filter(Boolean)
        .join(' → ') || 'Unavailable from source evidence'
    );
  };
  return (
    <section
      className="operator-panel auto-trader-excursions"
      aria-label="Excursions and exit analysis"
    >
      <header className="operator-panel__header">
        <div className="execution-quality-toolbar">
          <span>AutoTrader / Scouts</span>
          <select
            aria-label="Source"
            value={state.source}
            onChange={(e) => model.setSource(e.target.value as ExcursionSource)}
          >
            <option value="all">All strategies</option>
            <option value="autotrader">AutoTrader</option>
            <option value="scouts">Scouts</option>
          </select>
          <select
            aria-label="Date range"
            value={state.range}
            onChange={(e) => model.setRange(e.target.value as ExcursionRange)}
          >
            <option value="30d">Last 30 days</option>
            <option value="90d">Last 90 days</option>
            <option value="all">All time</option>
          </select>
          <span>
            {state.eligiblePathCount} complete paths / {state.sourceCount}{' '}
            records
          </span>
        </div>
      </header>
      <div className="operator-panel__body">
        <OperatorTabBar
          tabs={tabs}
          selectedId={state.view}
          onSelect={(id) => model.setView(id as ExcursionView)}
          label="Excursion analysis views"
        />
        <p className="muted" role="note">
          {EXCURSION_EVIDENCE_NOTICE}
        </p>
        <OperatorMetricStrip
          label="Executable excursion evidence"
          metrics={[
            {
              label: 'Records',
              value: state.sourceCount.toLocaleString(),
              detail: state.loading
                ? 'Loading'
                : (state.error ?? 'Selected cohort'),
            },
            {
              label: 'Complete paths',
              value: state.eligiblePathCount.toLocaleString(),
              detail: 'Executable price path',
            },
            {
              label: 'Incomplete paths',
              value: state.incompletePathCount.toLocaleString(),
              detail: 'Excluded from path metrics',
              tone: 'warning',
            },
            {
              label: 'What-if replay',
              value: 'Unavailable',
              detail: 'No authoritative replay results',
            },
          ]}
        />
        {panelStatus && (
          <p className="muted" role="status">
            {panelStatus}
          </p>
        )}
        {state.view === 'what-if' && (
          <div className="operator-panel">
            <strong>Alternative exit policy · research only</strong>
            <p className="muted">
              Read-only. No authoritative policy replay is available, so
              alternative results cannot be calculated or compared.
            </p>
          </div>
        )}
        {state.view === 'overview' && (
          <>
            <div className="execution-quality-table-wrap">
              <table className="operator-table">
                <thead>
                  <tr>
                    <th>Trade</th>
                    <th>Path</th>
                    <th>Missing path evidence</th>
                    <th>Gross MFE</th>
                    <th>Gross MAE</th>
                    <th>Net P&amp;L</th>
                    <th>Initial risk / R</th>
                    <th>Post-exit</th>
                    <th>Profile</th>
                  </tr>
                </thead>
                <tbody>
                  {state.trades.map((row) => (
                    <tr
                      key={row.id}
                      onClick={() => model.selectTrade(row.id)}
                      aria-selected={state.selectedTradeId === row.id}
                    >
                      <td>{row.trade.symbol ?? row.id}</td>
                      <td>
                        {row.executablePathComplete ? 'Complete' : 'Incomplete'}
                      </td>
                      <td>{row.incompletePathReason ?? '—'}</td>
                      <td>
                        {row.executablePathComplete
                          ? format(row.grossMfePercent, '%')
                          : 'Unavailable'}
                      </td>
                      <td>
                        {row.executablePathComplete
                          ? format(row.grossMaePercent, '%')
                          : 'Unavailable'}
                      </td>
                      <td>{format(row.trade.netPnlUsd, ' USD')}</td>
                      <td>{riskLabel(row)}</td>
                      <td>
                        {row.postExitAvailable ? 'Observed' : 'Unavailable'}
                      </td>
                      <td>
                        {row.profileProvenanceAvailable
                          ? `${row.trade.exitProfileId} v${row.trade.exitProfileVersion}`
                          : 'Unavailable'}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            {selected && (
              <div
                className="operator-panel"
                role="region"
                aria-label="Exit and protection evidence"
              >
                <strong>
                  {selected.trade.symbol ?? selected.id} · exit and protection
                  timeline
                </strong>
                <p>{timeline(selected)}</p>
                <p>
                  Profile: {selected.trade.exitProfileId ?? 'Unavailable'} ·
                  version: {selected.trade.exitProfileVersion ?? 'Unavailable'}{' '}
                  · as of:{' '}
                  {selected.trade.exitPolicyAsOfMs
                    ? new Date(selected.trade.exitPolicyAsOfMs).toLocaleString()
                    : 'Unavailable'}
                </p>
                <p>
                  Stop loss:{' '}
                  {format(selected.trade.selectedStopLossPercent, '%')} · take
                  profit:{' '}
                  {format(selected.trade.selectedTakeProfitPercent, '%')} · exit
                  reason: {selected.trade.exitReason ?? 'Unavailable'}
                </p>
                <p>
                  Net excursion: Unavailable from source contract · executable
                  path points: Unavailable
                </p>
                {!selected.executablePathComplete ? (
                  <p>Missing path evidence: {selected.incompletePathReason}</p>
                ) : null}
              </div>
            )}
          </>
        )}
        {state.view === 'continuation' && (
          <div className="execution-quality-table-wrap">
            <table className="operator-table">
              <thead>
                <tr>
                  <th>Trade</th>
                  <th>Observed at</th>
                  <th>Horizon</th>
                  <th>Best return</th>
                  <th>Worst return</th>
                  <th>Missed return</th>
                </tr>
              </thead>
              <tbody>
                {state.trades
                  .filter((row) => row.postExitAvailable)
                  .map((row) => (
                    <tr key={row.id}>
                      <td>{row.trade.symbol ?? row.id}</td>
                      <td>
                        {row.trade.postExitObservedAt
                          ? new Date(
                              row.trade.postExitObservedAt,
                            ).toLocaleString()
                          : 'Unavailable'}
                      </td>
                      <td>{format(row.trade.postExitHorizonMs, ' ms')}</td>
                      <td>
                        {format(row.trade.postExitBestReturnPercent, '%')}
                      </td>
                      <td>
                        {format(row.trade.postExitWorstReturnPercent, '%')}
                      </td>
                      <td>
                        {format(row.trade.postExitMissedReturnPercent, '%')}
                      </td>
                    </tr>
                  ))}
              </tbody>
            </table>
          </div>
        )}
        {state.view === 'transitions' && (
          <div className="operator-panel">
            <strong>Outcome transitions</strong>
            <p className="muted">
              Unavailable: original-cohort winner/loser/breakeven transitions
              and complete observation-window denominators are not present in
              the source contract.
            </p>
          </div>
        )}
        {state.view === 'sensitivity' && (
          <div className="operator-panel">
            <strong>Policy sensitivity</strong>
            <p className="muted">
              Unavailable: no deterministic replay results or comparable
              eligible-cohort metrics are provided.
            </p>
          </div>
        )}
        {state.view === 'replay' && (
          <div className="operator-panel">
            <strong>Trade replay</strong>
            <p className="muted">
              Unavailable: the source provides aggregate excursion evidence
              only; timestamped executable path points and fill/cost replay
              evidence are absent.
            </p>
          </div>
        )}
        {state.error && <div role="status">Source error: {state.error}</div>}
        <p className="muted">
          Incomplete or restart-interrupted observations remain visible and are
          not treated as terminal outcomes. Path metrics include only complete
          source evidence.
        </p>
      </div>
    </section>
  );
}
