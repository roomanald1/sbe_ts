import { useEffect, useMemo } from 'react';
import {
  CompactTable,
  OperatorMetricStrip,
  OperatorTabBar,
  useBehaviorSnapshot,
} from '@workspace/ui-platform';
import {
  LearningEvidenceWidgetModel,
  type LearningEvidenceWidgetView,
  type LearningEvidenceWidgetState,
  type LearningEvidenceTab,
} from '../models/LearningEvidenceWidgetModel';

function numberOrUnavailable(value: number | null | undefined, digits = 0) {
  return value == null
    ? 'Unavailable'
    : value.toLocaleString(undefined, {
        maximumFractionDigits: digits,
        minimumFractionDigits: digits,
      });
}

function timeOrUnavailable(value: number | null) {
  return value == null || value <= 0
    ? 'Unavailable'
    : new Date(value).toLocaleString();
}

function requestStatus(status: string) {
  const labels: Record<string, string> = {
    pending: 'Queued',
    running: 'Active',
    dispatched: 'Dispatched; worker completion not reported',
    capacity_blocked: 'Capacity blocked',
    waiting_for_market: 'Waiting for market conditions',
    completed: 'Completed; evidence acceptance is separate',
    failed: 'Failed',
    other_reported_state: 'Other reported state',
  };
  return labels[status] ?? status.replaceAll('_', ' ');
}

export function LearningEvidenceWidget() {
  const model = useMemo(() => new LearningEvidenceWidgetModel(), []);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  const state = useBehaviorSnapshot(model.state$);
  return (
    <LearningEvidenceView
      state={state}
      view={state.view}
      onSelectTab={(tab) => model.setTab(tab)}
      onSetFilters={(filters) => model.setFilters(filters)}
      onRefresh={() => void model.refresh()}
    />
  );
}

export function LearningEvidenceView({
  view,
  state,
  tab: tabOverride,
  onSelectTab,
  onSetFilters,
  onRefresh,
}: {
  view: LearningEvidenceWidgetView;
  state?: LearningEvidenceWidgetState;
  tab?: LearningEvidenceTab;
  onSelectTab?: (tab: LearningEvidenceTab) => void;
  onSetFilters?: (
    filters: Partial<LearningEvidenceWidgetState['filters']>,
  ) => void;
  onRefresh?: () => void;
}) {
  const tab = state?.tab ?? tabOverride ?? 'evidence';
  const cohorts = state?.visibleCohorts ?? view.cohorts;
  const requests = state?.visibleRequests ?? view.requests;
  const scoutEvidence = state?.visibleScoutEvidence ?? view.scoutEvidence;
  const actualTradeEvidence =
    state?.visibleActualTradeEvidence ?? view.actualTradeEvidence;
  const gapReasons = state?.visibleGapReasons ?? null;
  const tabs: Array<{ id: LearningEvidenceTab; label: string }> = [
    { id: 'evidence', label: 'Evidence' },
    { id: 'activity', label: 'Activity' },
    { id: 'history', label: 'History' },
    { id: 'promotion', label: 'Promotion' },
  ];
  const metrics = [
    {
      label: 'Profiles',
      value: numberOrUnavailable(view.profilesMonitored),
    },
    { label: 'Strict ready', value: 'Unavailable' },
    {
      label: 'Coverage cohorts',
      value: numberOrUnavailable(view.cohortTotal),
    },
    {
      label: 'Coverage gaps',
      value: numberOrUnavailable(view.evidenceGaps),
      tone:
        view.evidenceGaps == null
          ? ('neutral' as const)
          : view.evidenceGaps > 0
            ? ('warning' as const)
            : ('positive' as const),
    },
    { label: 'Accepted gain / 24h', value: 'Unavailable' },
  ];
  const sourceDescription =
    'Strict-ready profiles and accepted effective gains are not exposed by the current Learning contracts.';

  return (
    <section
      className="learning-evidence-widget operator-panel"
      aria-label="Learning and evidence acquisition"
    >
      <div className="learning-evidence-toolbar">
        <span
          className={
            'learning-evidence-data-state' + (state?.stale ? ' is-warning' : '')
          }
          role={state?.stale ? 'status' : undefined}
        >
          {state?.stale
            ? 'Stale evidence'
            : state
              ? 'Data ' + state.dataStatus
              : 'Source-backed evidence'}
        </span>
        <span className="learning-evidence-profile-count">
          {numberOrUnavailable(view.profilesMonitored)} profiles ·{' '}
          {numberOrUnavailable(view.strategiesMonitored)} strategies
        </span>
        {state && (
          <>
            <button
              type="button"
              onClick={onRefresh}
              disabled={!onRefresh || state.refreshing}
            >
              {state.refreshing ? 'Refreshing…' : 'Refresh'}
            </button>
            <small>
              {state.lastRefreshAttemptAt == null
                ? 'Refresh time unavailable'
                : 'Updated ' + timeOrUnavailable(state.lastRefreshAttemptAt)}
            </small>
          </>
        )}
      </div>

      <OperatorMetricStrip
        label="Learning evidence summary"
        metrics={metrics}
      />

      <OperatorTabBar
        label="Learning evidence views"
        selectedId={tab}
        onSelect={(id) => onSelectTab?.(id as LearningEvidenceTab)}
        tabs={tabs}
      />

      {(view.profileError ||
        view.coverageError ||
        view.learningSummaryError ||
        state?.refreshError) && (
        <div className="learning-evidence-errors" role="alert">
          {view.profileError && (
            <span>Profiles unavailable · {view.profileError}</span>
          )}
          {view.coverageError && (
            <span>Evidence coverage unavailable · {view.coverageError}</span>
          )}
          {view.learningSummaryError && (
            <span>
              Learning summary unavailable · {view.learningSummaryError}
            </span>
          )}
          {state?.refreshError && (
            <span>Refresh failed · {state.refreshError}</span>
          )}
        </div>
      )}

      {state && (
        <div className="learning-evidence-filters">
          <label>
            Symbol
            <select
              aria-label="Filter Learning evidence by symbol"
              value={state.filters.symbol}
              onChange={(event) =>
                onSetFilters?.({ symbol: event.target.value })
              }
            >
              <option value="">All symbols</option>
              {state.symbols.map((symbol) => (
                <option key={symbol} value={symbol}>
                  {symbol}
                </option>
              ))}
            </select>
          </label>
          <label>
            Strategy
            <select
              aria-label="Filter Learning evidence by strategy"
              value={state.filters.strategy}
              onChange={(event) =>
                onSetFilters?.({ strategy: event.target.value })
              }
            >
              <option value="">All strategies</option>
              {state.strategies.map((strategy) => (
                <option key={strategy} value={strategy}>
                  {strategy}
                </option>
              ))}
            </select>
          </label>
        </div>
      )}

      {tab === 'evidence' && (
        <div className="learning-evidence-tab">
          <div className="learning-evidence-section-heading">
            <strong>Evidence coverage by profile</strong>
            <span>
              {view.cohortTotal == null
                ? 'Coverage history unavailable'
                : numberOrUnavailable(cohorts?.length ?? 0) +
                  ' shown · ' +
                  numberOrUnavailable(view.cohortTotal) +
                  ' total' +
                  ((view.omittedCohorts ?? 0) > 0
                    ? ' · ' +
                      numberOrUnavailable(view.omittedCohorts) +
                      ' omitted'
                    : '')}
            </span>
          </div>
          {cohorts == null ? (
            <p className="learning-evidence-message" role="status">
              Coverage is unavailable. Sample counts and eligibility are not
              assumed to be zero.
            </p>
          ) : cohorts.length === 0 ? (
            <p className="learning-evidence-message" role="status">
              No coverage cohorts were returned. Completeness cannot be inferred
              from an empty page.
            </p>
          ) : (
            <CompactTable label="Learning evidence coverage by profile">
              <thead>
                <tr>
                  <th>Symbol · strategy · profile</th>
                  <th>Raw N</th>
                  <th>Effective N</th>
                  <th>Coverage</th>
                  <th>Last sample</th>
                  <th>24h gain</th>
                  <th>Blocker</th>
                </tr>
              </thead>
              <tbody>
                {cohorts.slice(0, 100).map((cohort) => (
                  <tr key={cohort.id}>
                    <td
                      title={
                        cohort.symbol +
                        ' · ' +
                        cohort.strategy +
                        ' · ' +
                        cohort.profile
                      }
                    >
                      <strong>
                        {cohort.symbol} · {cohort.strategy}
                      </strong>
                      <small>Profile {cohort.profile}</small>
                      <details className="learning-evidence-row-details">
                        <summary>Details</summary>
                        <dl>
                          <div>
                            <dt>Regime · horizon</dt>
                            <dd>
                              {cohort.regime ?? 'Unavailable'} ·{' '}
                              {cohort.horizon ?? 'Unavailable'}
                            </dd>
                          </div>
                          <div>
                            <dt>Coverage scope</dt>
                            <dd>{cohort.coverageScope || 'Unavailable'}</dd>
                          </div>
                          <div>
                            <dt>Sample age</dt>
                            <dd>
                              {cohort.sampleAgeMs == null
                                ? 'Unavailable'
                                : numberOrUnavailable(
                                    cohort.sampleAgeMs / 1000,
                                  ) + ' s'}
                            </dd>
                          </div>
                        </dl>
                      </details>
                    </td>
                    <td>{numberOrUnavailable(cohort.rawSamples)}</td>
                    <td>{numberOrUnavailable(cohort.effectiveSamples, 2)}</td>
                    <td data-status={cohort.freshness ?? 'unavailable'}>
                      {cohort.freshness ?? 'Unavailable'}
                    </td>
                    <td>{timeOrUnavailable(cohort.lastSampleAt)}</td>
                    <td>Unavailable</td>
                    <td
                      className="learning-evidence-blocker"
                      title={cohort.gapReason ?? 'Gap reason unavailable'}
                    >
                      {cohort.gapReason ?? 'Unavailable'}
                    </td>
                  </tr>
                ))}
              </tbody>
            </CompactTable>
          )}

          <div className="learning-evidence-lower-grid">
            <section aria-label="Reported acquisition blockers">
              <header>
                <strong>Reported acquisition blockers</strong>
                <span>Counts are cohort rows</span>
              </header>
              {gapReasons == null ? (
                <p>Reason counts unavailable.</p>
              ) : gapReasons.length === 0 ? (
                <p>No explicit gap reasons reported.</p>
              ) : (
                <ul>
                  {gapReasons.slice(0, 6).map((item) => (
                    <li key={item.reason} title={item.reason}>
                      <span>{item.reason}</span>
                      <strong>{item.cohortRows.toLocaleString()}</strong>
                    </li>
                  ))}
                </ul>
              )}
            </section>
            <section aria-label="Learning acquisition coverage">
              <header>
                <strong>Acquisition coverage</strong>
                <span>Returned source fields</span>
              </header>
              <dl>
                <div>
                  <dt>Explicit coverage gaps</dt>
                  <dd>{numberOrUnavailable(view.evidenceGaps)}</dd>
                </div>
                <div>
                  <dt>Evidence requests</dt>
                  <dd>{numberOrUnavailable(view.requestTotal)}</dd>
                </div>
                <div>
                  <dt>Omitted requests</dt>
                  <dd>{numberOrUnavailable(view.omittedRequests)}</dd>
                </div>
                <div>
                  <dt>Accepted independent gain</dt>
                  <dd>Unavailable</dd>
                </div>
              </dl>
            </section>
          </div>
          <p className="learning-evidence-limitation">{sourceDescription}</p>
        </div>
      )}

      {tab === 'activity' && (
        <div className="learning-evidence-tab">
          <div className="learning-evidence-section-heading">
            <strong>Evidence acquisition requests</strong>
            <span>
              {view.requestTotal == null
                ? 'Request totals unavailable'
                : numberOrUnavailable(view.requestTotal) +
                  ' total · ' +
                  numberOrUnavailable(view.omittedRequests) +
                  ' omitted'}
            </span>
          </div>
          {view.requestStates == null ? (
            <p className="learning-evidence-message">
              Request state is unavailable; no state counts are inferred.
            </p>
          ) : (
            <CompactTable label="Evidence request counts by reported state">
              <thead>
                <tr>
                  <th>Reported state</th>
                  <th>Requests</th>
                </tr>
              </thead>
              <tbody>
                {view.requestStates.map((item) => (
                  <tr key={item.status}>
                    <td>{requestStatus(item.status)}</td>
                    <td>{item.count.toLocaleString()}</td>
                  </tr>
                ))}
              </tbody>
            </CompactTable>
          )}
          {requests == null ? (
            <p className="learning-evidence-message">
              Request details unavailable.
            </p>
          ) : requests.length === 0 ? (
            <p className="learning-evidence-message">
              No request details returned. This does not establish that no
              evidence gaps exist.
            </p>
          ) : (
            <CompactTable label="Learning evidence acquisition request details">
              <thead>
                <tr>
                  <th>Symbol · strategy · profile</th>
                  <th>Status</th>
                  <th>Attempts</th>
                  <th>Updated</th>
                  <th>Reason</th>
                </tr>
              </thead>
              <tbody>
                {requests.slice(0, 100).map((request) => (
                  <tr key={request.id}>
                    <td>
                      <strong>
                        {request.symbol} · {request.strategy}
                      </strong>
                      <small>
                        {request.profile} · {request.regime} · {request.horizon}
                      </small>
                    </td>
                    <td>{requestStatus(request.status)}</td>
                    <td>{numberOrUnavailable(request.attempts)}</td>
                    <td>{timeOrUnavailable(request.updatedAt)}</td>
                    <td className="learning-evidence-blocker">
                      {request.reason ?? 'Unavailable'}
                    </td>
                  </tr>
                ))}
              </tbody>
            </CompactTable>
          )}
        </div>
      )}

      {tab === 'history' && (
        <div className="learning-evidence-tab">
          <div className="learning-evidence-section-heading">
            <strong>Research and actual trading outcomes</strong>
            <span>Separate source populations</span>
          </div>
          {scoutEvidence == null ? (
            <p className="learning-evidence-message">
              Scout outcome summary unavailable.
            </p>
          ) : scoutEvidence.length === 0 ? (
            <p className="learning-evidence-message">
              No Scout outcome summary was returned.
            </p>
          ) : (
            <>
              <CompactTable label="Scout research outcomes reported to Learning">
                <thead>
                  <tr>
                    <th>Strategy</th>
                    <th>Reported outcomes</th>
                    <th>Positive rate</th>
                    <th>Mean return</th>
                    <th>Independent acceptance</th>
                  </tr>
                </thead>
                <tbody>
                  {scoutEvidence.map((row) => (
                    <tr key={row.strategy}>
                      <td>{row.strategy}</td>
                      <td>{row.samples.toLocaleString()}</td>
                      <td>
                        {row.positiveRate == null
                          ? 'Unavailable'
                          : (row.positiveRate * 100).toFixed(1) + '%'}
                      </td>
                      <td>
                        {row.averageReturnPercent == null
                          ? 'Unavailable'
                          : row.averageReturnPercent.toFixed(3) + '%'}
                      </td>
                      <td>Unavailable · source aggregate is not acceptance</td>
                    </tr>
                  ))}
                </tbody>
              </CompactTable>
              <p className="learning-evidence-limitation">
                Scout totals are source-reported counts with no common interval;
                they are not accepted independent gain.
              </p>
            </>
          )}
          {actualTradeEvidence == null ? (
            <p className="learning-evidence-message">
              Profile-linked actual-trade summary unavailable.
            </p>
          ) : actualTradeEvidence.length === 0 ? (
            <p className="learning-evidence-message">
              No profile-linked actual-trade summary was returned.
            </p>
          ) : (
            <>
              <CompactTable label="Profile-linked actual trade outcome summaries">
                <thead>
                  <tr>
                    <th>Strategy</th>
                    <th>Active-profile N</th>
                    <th>Active-profile mean</th>
                    <th>Other-profile N</th>
                    <th>Other-profile mean</th>
                    <th>Independent reconciliation</th>
                  </tr>
                </thead>
                <tbody>
                  {actualTradeEvidence.map((row) => (
                    <tr key={row.strategy}>
                      <td>{row.strategy}</td>
                      <td>{row.activeSamples.toLocaleString()}</td>
                      <td>
                        {row.activeAverageReturnPercent == null
                          ? 'Unavailable'
                          : row.activeAverageReturnPercent.toFixed(3) + '%'}
                      </td>
                      <td>{row.otherSamples.toLocaleString()}</td>
                      <td>
                        {row.otherAverageReturnPercent == null
                          ? 'Unavailable'
                          : row.otherAverageReturnPercent.toFixed(3) + '%'}
                      </td>
                      <td>Unavailable in summary contract</td>
                    </tr>
                  ))}
                </tbody>
              </CompactTable>
              <p className="learning-evidence-limitation">
                These are Learning-reported aggregates, not independently
                reconciled event-level profitability.
              </p>
            </>
          )}
        </div>
      )}

      {tab === 'promotion' && (
        <section
          className="learning-evidence-promotion"
          aria-label="Promotion evidence"
        >
          <strong>Promotion status unavailable</strong>
          <p>
            The current Learning summary does not expose accepted-gain,
            validation-event or promotion-history attribution. No promotion or
            profitability claim is inferred.
          </p>
        </section>
      )}
    </section>
  );
}
