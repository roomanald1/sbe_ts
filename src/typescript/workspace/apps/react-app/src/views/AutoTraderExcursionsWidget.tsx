import { useEffect, useMemo } from 'react';
import {
  CompactTable,
  OperatorMetricStrip,
  OperatorTabBar,
  useBehaviorSnapshot,
} from '@workspace/ui-platform';
import {
  EXCURSION_PATH_PAGE_SIZE,
  AutoTraderExcursionsModel,
  EXCURSION_EVIDENCE_NOTICE,
  excursionPanelStatus,
  type ExcursionRange,
  type ExcursionSource,
  type ExcursionView,
  type AutoTraderExcursionsState,
  type WhatIfParameterName,
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
const whatIfParameters: { key: WhatIfParameterName; label: string }[] = [
  { key: 'stop_loss_percent', label: 'Stop loss (%)' },
  { key: 'take_profit_percent', label: 'Take profit (%)' },
  { key: 'activation_percent', label: 'Profit activation (%)' },
  { key: 'giveback_percent', label: 'Giveback (%)' },
  { key: 'minimum_lock_percent', label: 'Minimum lock (%)' },
  { key: 'trailing_activation_percent', label: 'Trailing activation (%)' },
  { key: 'trailing_distance_percent', label: 'Trailing distance (%)' },
  { key: 'breakeven_activation_percent', label: 'Breakeven activation (%)' },
];

export function AutoTraderExcursionsWidget() {
  const model = useMemo(() => new AutoTraderExcursionsModel(outcomes), []);
  const state = useBehaviorSnapshot(model.state$);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  return <AutoTraderExcursionsPanel model={model} state={state} />;
}

export function AutoTraderExcursionsPanel({
  model,
  state,
}: {
  model: AutoTraderExcursionsModel;
  state: AutoTraderExcursionsState;
}) {
  const selected = state.trades.find(
    (trade) => trade.id === state.selectedTradeId,
  );
  const selectedPath =
    selected?.stableTradeId === state.pathPage?.tradeId
      ? state.pathPage
      : undefined;
  const panelStatus =
    state.view !== 'replay'
      ? excursionPanelStatus(
          state.view,
          state.sourceCount,
          state.eligiblePathCount,
          state.trades.filter((trade) => trade.postExitAvailable).length,
        )
      : !selected
        ? 'Select a trade from Overview to inspect its durable quote path.'
        : state.pathLoading
          ? 'Loading source-backed chronological path samples…'
          : (state.pathError ??
            (selectedPath
              ? `${selectedPath.historyComplete ? 'Complete' : 'Incomplete'} durable path · ${selectedPath.samples.length} loaded / ${selectedPath.totalSampleCount} retained samples`
              : 'No durable path has been loaded for this trade.'));
  const format = (value?: number, suffix = '') =>
    typeof value === 'number' && Number.isFinite(value)
      ? `${value.toFixed(3)}${suffix}`
      : 'Unavailable';
  const quote = (value?: number) =>
    typeof value === 'number' && Number.isFinite(value)
      ? value.toLocaleString(undefined, { maximumFractionDigits: 8 })
      : 'Unavailable';
  const horizonOffset = (value?: number) =>
    typeof value === 'number' && Number.isFinite(value)
      ? `${(value / 60_000).toFixed(1)} min after entry`
      : 'Entry offset unavailable';
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
            <option value="all">All sources</option>
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
          <select
            aria-label="Strategy"
            value={state.strategy}
            onChange={(e) => model.setStrategy(e.target.value)}
          >
            <option value="all">All strategies</option>
            {state.strategyOptions.map((strategy) => (
              <option key={strategy} value={strategy}>
                {strategy}
              </option>
            ))}
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
              label: 'Complete source paths',
              value: state.eligiblePathCount.toLocaleString(),
              detail: 'Acquisition completeness; replay checked separately',
            },
            {
              label: 'Incomplete paths',
              value: state.incompletePathCount.toLocaleString(),
              detail: 'Excluded from path metrics',
              tone: 'warning',
            },
            {
              label: 'What-if replay',
              value:
                state.whatIfReplay.status === 'complete'
                  ? 'Complete'
                  : state.whatIfReplay.status === 'loading'
                    ? 'Running'
                    : state.whatIfReplay.status === 'not_computed'
                      ? 'Not computed'
                      : state.whatIfReplay.status === 'unavailable'
                        ? 'Unavailable'
                        : 'Research only',
              detail:
                state.whatIfReplay.status === 'complete'
                  ? 'Historical, learning-ineligible'
                  : 'Historical replay · research only',
            },
          ]}
        />
        {panelStatus && (
          <p className="muted" role="status">
            {panelStatus}
          </p>
        )}
        {state.view === 'what-if' && (
          <section
            className="operator-panel"
            aria-label="Historical What-if replay controls"
          >
            <strong>Alternative exit policy · research only</strong>
            <p className="muted">
              Draft values affect this historical replay only. They never write
              trading settings or Learning evidence; every result is
              learning-ineligible.
            </p>
            <label>
              Trade
              <select
                aria-label="Trade for What-if replay"
                value={state.selectedTradeId ?? ''}
                onChange={(event) =>
                  model.selectTrade(event.target.value || undefined)
                }
              >
                <option value="">Select a trade</option>
                {state.trades.map((row) => (
                  <option key={row.id} value={row.id}>
                    {row.trade.symbol ?? row.id} ·{' '}
                    {row.stableTradeId ?? 'stable identity unavailable'}
                  </option>
                ))}
              </select>
            </label>
            <p className="muted">
              Candidate policy: {state.whatIfDraft.candidatePolicyId} ·{' '}
              {state.whatIfDraft.candidatePolicyVersion}
              {selected?.trade.exitParameterProvenance?.profile_id
                ? ` · captured policy ${selected.trade.exitParameterProvenance.profile_id} v${selected.trade.exitParameterProvenance.profile_version ?? 'unavailable'}`
                : ' · captured policy provenance unavailable'}
            </p>
            <div className="operator-key-value-list">
              {whatIfParameters.map(({ key, label }) => (
                <div key={key}>
                  <dt>{label}</dt>
                  <dd>
                    <input
                      aria-label={label}
                      type="number"
                      step="any"
                      value={state.whatIfDraft.parameters[key] ?? ''}
                      onChange={(event) => {
                        const value = event.currentTarget.value;
                        model.setWhatIfParameter(
                          key,
                          value === '' ? null : Number(value),
                        );
                      }}
                    />
                  </dd>
                </div>
              ))}
            </div>
            <label>
              Maximum hold candidate (days)
              <input
                aria-label="Maximum hold candidate in days"
                type="number"
                min={state.whatIfDraft.maximumHoldSpec?.minimum}
                max={state.whatIfDraft.maximumHoldSpec?.maximum}
                step={state.whatIfDraft.maximumHoldSpec?.step ?? 'any'}
                disabled={!state.whatIfDraft.maximumHoldSpec}
                value={state.whatIfDraft.maximumHoldDays ?? ''}
                onChange={(event) => {
                  const value = event.currentTarget.value;
                  model.setWhatIfMaximumHoldDays(
                    value === '' ? null : Number(value),
                  );
                }}
              />
            </label>
            <p className="muted" role="note">
              {state.whatIfDraft.maximumHoldReason}
              {' · Captured baseline: '}
              {selected?.trade.exitParameterProvenance?.maximum_holding_duration
                ?.selected_days ?? 'Unavailable'}{' '}
              days. Candidate duration remains a draft replay input.
            </p>
            {model.whatIfReadinessReason() && (
              <p className="muted" role="status">
                {model.whatIfReadinessReason()}
              </p>
            )}
            <div className="execution-quality-toolbar">
              {state.whatIfReplay.status === 'loading' ? (
                <button type="button" onClick={() => model.cancelWhatIf()}>
                  Cancel replay
                </button>
              ) : (
                <button
                  type="button"
                  disabled={!model.canRunWhatIf()}
                  onClick={() => void model.runWhatIf()}
                >
                  Run historical What-if
                </button>
              )}
              <span>Research only · Learning ineligible</span>
            </div>
            {state.whatIfReplay.status === 'loading' && (
              <p role="status">Replaying durable historical evidence…</p>
            )}
            {state.whatIfReplay.status === 'cancelled' && (
              <p role="status">{state.whatIfReplay.reason}</p>
            )}
            {state.whatIfReplay.status === 'error' && (
              <p role="alert">{state.whatIfReplay.error}</p>
            )}
            {state.whatIfReplay.status === 'unavailable' && (
              <p role="status">
                Replay unavailable: {state.whatIfReplay.reason}
              </p>
            )}
            {state.whatIfReplay.status === 'not_computed' && (
              <p role="status">
                Replay not computed: {state.whatIfReplay.reason} ·{' '}
                {state.whatIfReplay.processedSampleCount ?? 'Unavailable'} of{' '}
                {state.whatIfReplay.totalSampleCount ?? 'Unavailable'} samples
                processed · cursor{' '}
                {state.whatIfReplay.afterCursor ?? 'Unavailable'}
              </p>
            )}
            {state.whatIfReplay.result && (
              <div aria-label="Historical replay result">
                <h4>Historical replay result · learning-ineligible</h4>
                <dl
                  className="operator-metrics"
                  aria-label="What-if comparison"
                >
                  <div className="operator-metric">
                    <dt className="operator-metric__label">Baseline net</dt>
                    <dd className="operator-metric__value">
                      {format(
                        state.whatIfReplay.result.baselineNetReturnPercent,
                        '%',
                      )}
                    </dd>
                  </div>
                  <div className="operator-metric">
                    <dt className="operator-metric__label">Candidate net</dt>
                    <dd className="operator-metric__value">
                      {format(
                        state.whatIfReplay.result.candidateNetReturnPercent,
                        '%',
                      )}
                    </dd>
                  </div>
                  <div className="operator-metric">
                    <dt className="operator-metric__label">
                      Candidate − baseline
                    </dt>
                    <dd className="operator-metric__value">
                      {format(
                        state.whatIfReplay.result.deltaNetReturnPercent,
                        '%',
                      )}
                    </dd>
                  </div>
                </dl>
                <p>
                  Candidate exit:{' '}
                  {state.whatIfReplay.result.candidateExit.reason}
                  {' · '}
                  {new Date(
                    state.whatIfReplay.result.candidateExit.timestampMs,
                  ).toLocaleString()}
                  {' · '}
                  {quote(state.whatIfReplay.result.candidateExit.price)}
                </p>
                <p className="muted">
                  {state.whatIfReplay.result.durablePathSampleCount} durable
                  samples ·{' '}
                  {state.whatIfReplay.result.feeScheduleVersions.length} fee
                  schedule records · {state.whatIfReplay.result.scope} ·
                  learning eligible: no
                </p>
                <p>
                  Baseline maximum hold:{' '}
                  {state.whatIfReplay.result
                    .baselineMaximumHoldingDurationDays !== undefined
                    ? `${state.whatIfReplay.result.baselineMaximumHoldingDurationDays} days`
                    : state.whatIfReplay.result
                          .baselineMaximumHoldingDurationMs === undefined
                      ? 'Unavailable'
                      : `${(state.whatIfReplay.result.baselineMaximumHoldingDurationMs / 86_400_000).toFixed(3)} days`}
                  {' · Candidate maximum hold: '}
                  {state.whatIfReplay.result
                    .candidateMaximumHoldingDurationDays === undefined
                    ? 'Unavailable'
                    : `${state.whatIfReplay.result.candidateMaximumHoldingDurationDays} days`}
                </p>
              </div>
            )}
          </section>
        )}
        {state.view === 'overview' && (
          <>
            <CompactTable label="Excursion trade overview">
              <thead>
                <tr>
                  <th>Trade</th>
                  <th>Path history</th>
                  <th>Missing path evidence</th>
                  <th>Threshold crossings</th>
                  <th>Replay path MFE</th>
                  <th>Replay path MAE</th>
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
                    data-selected={state.selectedTradeId === row.id}
                  >
                    <td>
                      <button
                        type="button"
                        className="auto-trader-excursions__trade-select"
                        aria-label={`Inspect ${row.trade.symbol ?? row.id} exit evidence`}
                        aria-pressed={state.selectedTradeId === row.id}
                        onClick={() => model.selectTrade(row.id)}
                      >
                        {row.trade.symbol ?? row.id}
                      </button>
                    </td>
                    <td>
                      {row.executablePathComplete
                        ? 'Complete source path'
                        : 'Incomplete source path'}
                    </td>
                    <td>{row.incompletePathReason ?? '—'}</td>
                    <td>
                      {row.thresholdCrossings.length
                        ? `${row.thresholdCrossings.length} observed · fee basis unverified`
                        : (row.thresholdCrossingEvidenceStatus ??
                          'Unavailable')}
                    </td>
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
            </CompactTable>
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
                <h4>Chronological threshold crossings</h4>
                {selected.thresholdCrossings.length ? (
                  <>
                    <CompactTable label="Chronological threshold crossings">
                      <thead>
                        <tr>
                          <th>Timestamp</th>
                          <th>Threshold</th>
                          <th>Source</th>
                          <th>Quote side / close mark</th>
                          <th>Observed return</th>
                          <th>Quote identity</th>
                          <th>Evidence status</th>
                        </tr>
                      </thead>
                      <tbody>
                        {selected.thresholdCrossings.map((crossing, index) => (
                          <tr
                            key={`${crossing.positionPathEpoch ?? 'path'}:${crossing.timestampMs}:${crossing.thresholdType}:${index}`}
                          >
                            <td>
                              {new Date(crossing.timestampMs).toLocaleString()}
                            </td>
                            <td>
                              {crossing.thresholdType} ·{' '}
                              {format(crossing.thresholdPercent, '%')} ·{' '}
                              {crossing.thresholdComparator ?? 'Unavailable'}
                            </td>
                            <td>{crossing.thresholdSource ?? 'Unavailable'}</td>
                            <td>
                              {crossing.quoteSide ?? 'Unavailable'} ·{' '}
                              {quote(crossing.closeMark)}
                            </td>
                            <td>
                              {format(crossing.observedNetReturnPercent, '%')}
                            </td>
                            <td>
                              {crossing.quoteSourceSystem ?? 'Unavailable'} ·{' '}
                              {crossing.quoteSourceEpoch ?? 'Unavailable'} ·{' '}
                              {crossing.quoteSourceCursor ?? 'Unavailable'} ·{' '}
                              {crossing.quoteSourceRecordId ?? 'Unavailable'}
                            </td>
                            <td>
                              {crossing.crossingEvidenceStatus ?? 'Unavailable'}
                              {' · '}
                              {crossing.feeRateAttribution ??
                                'Fee basis unavailable'}
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </CompactTable>
                    <p className="muted">
                      Threshold touches with unverified fee schedules remain
                      ineligible for fee-aware replay and Learning.
                    </p>
                  </>
                ) : (
                  <p>
                    {selected.thresholdCrossingEvidenceStatus ??
                      'No source-backed threshold crossing evidence is available.'}
                  </p>
                )}
                <div className="execution-quality-toolbar">
                  <button
                    type="button"
                    onClick={() => model.showTradePath(selected.id)}
                  >
                    Inspect durable path
                  </button>
                </div>
              </div>
            )}
          </>
        )}
        {state.view === 'continuation' && (
          <CompactTable label="Post-exit continuation observations">
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
                    <td>{format(row.trade.postExitBestReturnPercent, '%')}</td>
                    <td>{format(row.trade.postExitWorstReturnPercent, '%')}</td>
                    <td>
                      {format(row.trade.postExitMissedReturnPercent, '%')}
                    </td>
                  </tr>
                ))}
            </tbody>
          </CompactTable>
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
          <section
            className="operator-panel"
            role="region"
            aria-label="Durable trade price path"
          >
            <h3>
              {selected?.trade.symbol ?? 'Trade'} · chronological quote path
            </h3>
            {state.pathError && <p role="status">{state.pathError}</p>}
            {state.pathLoading && <p role="status">Loading path samples…</p>}
            {selectedPath && (
              <>
                <p>
                  {selectedPath.historyComplete
                    ? 'Lifetime path acquisition complete'
                    : 'Lifetime path acquisition incomplete'}{' '}
                  · {selectedPath.coverageStatus} ·{' '}
                  {selectedPath.samples.length} loaded /{' '}
                  {selectedPath.totalSampleCount} stored samples ·{' '}
                  {selectedPath.segments.length} source segments
                </p>
                <p className="muted">
                  Timestamped sampled source quotes only. The path table does
                  not represent event-driven lifetime extrema or infer fills.
                </p>
                {selectedPath.capturePolicy ? (
                  <p role="note">
                    Sampling horizons:{' '}
                    {selectedPath.capturePolicy.fineIntervalMs}
                    {' ms through the first '}
                    {(selectedPath.capturePolicy.fineWindowMs / 60_000).toFixed(
                      0,
                    )}
                    {' min; '}
                    {selectedPath.capturePolicy.coarseIntervalMs}
                    {' ms after '}
                    {(
                      selectedPath.capturePolicy.coarseAfterMs / 60_000
                    ).toFixed(0)}
                    {' min. Event-driven extrema: '}
                    {selectedPath.capturePolicy.eventDrivenExtrema
                      ? 'enabled'
                      : 'unavailable'}
                    ; event-driven threshold crossings:{' '}
                    {selectedPath.capturePolicy.eventDrivenThresholdCrossings
                      ? 'enabled'
                      : 'unavailable'}
                    .
                  </p>
                ) : (
                  <p role="status">
                    Capture cadence and event-driven lifetime evidence are
                    unavailable from this path response.
                  </p>
                )}
                {selectedPath.lifetimeExcursionEvidence ? (
                  <section aria-label="Lifetime event-driven excursion evidence">
                    <h4>
                      {selectedPath.lifetimeExcursionEvidence.complete
                        ? selectedPath.capturePolicy?.eventDrivenExtrema
                          ? 'Lifetime event-driven gross excursion'
                          : 'Lifetime gross excursion · event-driven attribution unverified'
                        : selectedPath.capturePolicy?.eventDrivenExtrema
                          ? 'Partial event-driven gross excursion'
                          : 'Partial gross excursion · event-driven attribution unverified'}
                    </h4>
                    <dl className="operator-metrics">
                      <div className="operator-metric">
                        <dt className="operator-metric__label">
                          {selectedPath.lifetimeExcursionEvidence.complete
                            ? selectedPath.capturePolicy?.eventDrivenExtrema
                              ? 'Lifetime gross MFE'
                              : 'Reported gross MFE'
                            : 'Partial gross MFE'}
                        </dt>
                        <dd className="operator-metric__value">
                          {format(
                            selectedPath.lifetimeExcursionEvidence
                              .grossMfePercent,
                            '%',
                          )}
                        </dd>
                      </div>
                      <div className="operator-metric">
                        <dt className="operator-metric__label">
                          {selectedPath.lifetimeExcursionEvidence.complete
                            ? selectedPath.capturePolicy?.eventDrivenExtrema
                              ? 'Lifetime gross MAE'
                              : 'Reported gross MAE'
                            : 'Partial gross MAE'}
                        </dt>
                        <dd className="operator-metric__value">
                          {format(
                            selectedPath.lifetimeExcursionEvidence
                              .grossMaePercent,
                            '%',
                          )}
                        </dd>
                      </div>
                      <div className="operator-metric">
                        <dt className="operator-metric__label">
                          Event samples / maximum gap
                        </dt>
                        <dd className="operator-metric__value">
                          {selectedPath.lifetimeExcursionEvidence.eventSampleCount.toLocaleString()}
                          {' / '}
                          {selectedPath.lifetimeExcursionEvidence
                            .eventMaximumGapMs === undefined
                            ? 'Unavailable'
                            : `${selectedPath.lifetimeExcursionEvidence.eventMaximumGapMs.toLocaleString()} ms`}
                        </dd>
                      </div>
                    </dl>
                    {!selectedPath.lifetimeExcursionEvidence.complete && (
                      <p role="status">
                        Lifetime extrema are incomplete and are not presented as
                        final trade values.{' '}
                        {selectedPath.lifetimeExcursionEvidence.missingReasons.join(
                          ', ',
                        )}
                      </p>
                    )}
                    {selectedPath.lifetimeExcursionEvidence.thresholdCrossings
                      .length > 0 ? (
                      <CompactTable label="Lifetime event-driven threshold crossings">
                        <thead>
                          <tr>
                            <th>Timestamp</th>
                            <th>Threshold</th>
                            <th>Executable side / mark</th>
                            <th>Quote source identity</th>
                          </tr>
                        </thead>
                        <tbody>
                          {selectedPath.lifetimeExcursionEvidence.thresholdCrossings.map(
                            (crossing, index) => (
                              <tr
                                key={`${crossing.positionPathEpoch ?? 'lifetime'}:${crossing.timestampMs}:${crossing.thresholdType}:${index}`}
                              >
                                <td>
                                  {new Date(
                                    crossing.timestampMs,
                                  ).toLocaleString()}
                                </td>
                                <td>
                                  {crossing.thresholdType} ·{' '}
                                  {format(crossing.thresholdPercent, '%')}
                                </td>
                                <td>
                                  {crossing.quoteSide ?? 'Unavailable'} ·{' '}
                                  {quote(crossing.closeMark)}
                                </td>
                                <td>
                                  {crossing.quoteSourceSystem ?? 'Unavailable'}
                                  {' · '}
                                  {crossing.quoteSourceEpoch ?? 'Unavailable'}
                                  {' · '}
                                  {crossing.quoteSourceCursor ?? 'Unavailable'}
                                  {' · '}
                                  {crossing.quoteSourceRecordId ??
                                    'Unavailable'}
                                </td>
                              </tr>
                            ),
                          )}
                        </tbody>
                      </CompactTable>
                    ) : (
                      <p className="muted">
                        {selectedPath.lifetimeExcursionEvidence.complete
                          ? 'No lifetime threshold crossings were reported.'
                          : 'Lifetime threshold crossing evidence is incomplete.'}
                      </p>
                    )}
                  </section>
                ) : (
                  <p role="status">
                    Lifetime MFE/MAE evidence is unavailable; sampled path
                    values are not substituted.
                  </p>
                )}
                {selectedPath.replayEligibility ? (
                  <p
                    role="status"
                    data-replay-eligibility={
                      selectedPath.replayEligibility.status
                    }
                  >
                    Replay eligibility:{' '}
                    {selectedPath.replayEligibility.status === 'eligible'
                      ? 'eligible for finer-resolution replay'
                      : selectedPath.replayEligibility.status ===
                          'ineligible_coarse_sampling'
                        ? 'ineligible: coarse samples are lifetime context, not finer-resolution policy replay'
                        : selectedPath.replayEligibility.status === 'incomplete'
                          ? 'incomplete: policy replay is unavailable'
                          : 'unavailable: policy replay eligibility is not established'}
                    {' · maximum gap '}
                    {selectedPath.replayEligibility.maximumObservedGapMs ===
                    undefined
                      ? 'Unavailable'
                      : `${selectedPath.replayEligibility.maximumObservedGapMs.toLocaleString()} ms`}
                    {'; required ≤ '}
                    {selectedPath.replayEligibility.requiredMaximumGapMs.toLocaleString()}
                    {' ms'}
                    {selectedPath.replayEligibility.reason
                      ? ` · ${selectedPath.replayEligibility.reason}`
                      : ''}
                  </p>
                ) : (
                  <p role="status">
                    Replay eligibility unavailable; do not infer
                    finer-resolution policy replay from lifetime coverage.
                  </p>
                )}
                {selectedPath.horizonCoverage?.length ? (
                  <>
                    <h4>Observed path coverage by horizon</h4>
                    <CompactTable label="Sampling coverage by trade horizon">
                      <thead>
                        <tr>
                          <th>Horizon</th>
                          <th>Observed offset</th>
                          <th>Target cadence</th>
                          <th>Samples</th>
                          <th>Maximum gap</th>
                          <th>Coverage</th>
                        </tr>
                      </thead>
                      <tbody>
                        {selectedPath.horizonCoverage.map((horizon) => (
                          <tr key={horizon.horizon}>
                            <td>
                              {horizon.horizon === 'fine_0_30m'
                                ? 'Fine · first 30m'
                                : 'Coarse · after 30m'}
                            </td>
                            <td>
                              {horizonOffset(horizon.offsetStartMs)} ·{' '}
                              {horizon.offsetEndMs === undefined
                                ? 'ongoing'
                                : horizonOffset(horizon.offsetEndMs)}
                            </td>
                            <td>
                              {horizon.targetIntervalMs.toLocaleString()} ms
                            </td>
                            <td>{horizon.sampleCount.toLocaleString()}</td>
                            <td>
                              {horizon.maximumObservedGapMs === undefined
                                ? 'Unavailable'
                                : `${horizon.maximumObservedGapMs.toLocaleString()} ms`}
                            </td>
                            <td>
                              {!horizon.complete
                                ? 'Incomplete'
                                : horizon.horizonReached
                                  ? 'Complete'
                                  : 'In progress · horizon not reached'}
                              {horizon.missingReasons.length
                                ? ` · ${horizon.missingReasons.join(', ')}`
                                : ''}
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </CompactTable>
                  </>
                ) : (
                  <p role="status">
                    Observed sample counts and completeness by horizon are
                    unavailable from this path response.
                  </p>
                )}
                {selectedPath.missingReasons.length > 0 && (
                  <p role="status">
                    Missing path evidence:{' '}
                    {selectedPath.missingReasons.join(', ')}
                  </p>
                )}
                <CompactTable label="Chronological executable quote path">
                  <thead>
                    <tr>
                      <th>Timestamp</th>
                      <th>Path epoch</th>
                      <th>Sequence / cursor</th>
                      <th>Bid</th>
                      <th>Ask</th>
                      <th>Quote source</th>
                      <th>Freshness</th>
                    </tr>
                  </thead>
                  <tbody>
                    {selectedPath.samples.map((sample) => (
                      <tr
                        key={`${sample.pathEpoch}:${sample.sequence}:${sample.cursor}`}
                      >
                        <td>{new Date(sample.timestampMs).toLocaleString()}</td>
                        <td>{sample.pathEpoch}</td>
                        <td>
                          {sample.sequence} / {sample.cursor}
                        </td>
                        <td>{quote(sample.bid)}</td>
                        <td>{quote(sample.ask)}</td>
                        <td>
                          {sample.quoteSourceSystem ?? 'Unavailable'} ·{' '}
                          {sample.quoteSourceEpoch ?? 'Unavailable'} ·{' '}
                          {sample.quoteSourceCursor ?? 'Unavailable'} ·{' '}
                          {sample.quoteSourceRecordId ?? 'Unavailable'}
                        </td>
                        <td>
                          {sample.sourceFresh === true
                            ? 'Fresh'
                            : sample.sourceFresh === false
                              ? 'Stale'
                              : 'Unverified'}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </CompactTable>
                {selectedPath.segments.map((segment) => (
                  <p className="muted" key={segment.pathEpoch}>
                    Segment {segment.pathEpoch}: {segment.sampleCount} samples ·
                    max observed gap {segment.maximumGapMs} ms ·{' '}
                    {segment.complete ? 'complete' : 'incomplete'}
                    {segment.missingReasons.length
                      ? ` · ${segment.missingReasons.join(', ')}`
                      : ''}
                  </p>
                ))}
                {selectedPath.hasMore && (
                  <div className="execution-quality-toolbar">
                    <button
                      type="button"
                      onClick={() => void model.loadMorePath()}
                      disabled={state.pathLoading}
                    >
                      Load next {EXCURSION_PATH_PAGE_SIZE} samples
                    </button>
                  </div>
                )}
              </>
            )}
          </section>
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
