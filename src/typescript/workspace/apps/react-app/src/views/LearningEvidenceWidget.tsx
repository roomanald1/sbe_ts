import { useEffect, useMemo } from 'react';
import { useBehaviorSnapshot } from '@workspace/ui-platform';
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
  const decisionBlockers =
    state?.visibleDecisionBlockers ?? view.decisionBlockers;
  const tabs: Array<{ id: LearningEvidenceTab; label: string }> = [
    { id: 'evidence', label: 'Evidence' },
    { id: 'activity', label: 'Activity' },
    { id: 'history', label: 'History' },
    { id: 'promotion', label: 'Promotion' },
  ];
  return (
    <section
      className="auto-trader-evidence-coverage learning-evidence-widget v35-operator-card"
      aria-label="Learning and evidence acquisition"
    >
      <header className="auto-trader-evidence-coverage-heading">
        <div>
          <strong>Learning and evidence acquisition</strong>
          <small>
            Learning-owned evidence requests and profile coverage. Scout
            research evidence is reported separately from actual trading
            outcomes.
          </small>
        </div>
        <small>
          {view.profilesMonitored == null
            ? 'Profile data unavailable'
            : `${view.profilesMonitored} profiles · ${numberOrUnavailable(view.strategiesMonitored)} ${view.strategiesMonitored === 1 ? 'strategy' : 'strategies'} monitored`}
        </small>
      </header>

      {view.profileError && (
        <p role="alert">Profile coverage unavailable: {view.profileError}</p>
      )}
      {view.coverageError && (
        <p role="alert">
          Evidence request coverage unavailable: {view.coverageError}
        </p>
      )}
      {view.learningSummaryError && (
        <p role="alert">
          Learning summary unavailable: {view.learningSummaryError}
        </p>
      )}
      {state?.refreshError && (
        <p role="alert">Refresh failed: {state.refreshError}</p>
      )}
      {state && (
        <div className="auto-trader-evidence-coverage-subheading">
          <span role={state.stale ? 'status' : undefined}>
            {state.stale
              ? 'Stale evidence reported by source'
              : `Evidence status: ${state.dataStatus}`}
          </span>
          <button
            type="button"
            onClick={onRefresh}
            disabled={!onRefresh || state.refreshing}
          >
            {state.refreshing ? 'Refreshing…' : 'Refresh evidence'}
          </button>
          <small>
            {state.lastRefreshAttemptAt == null
              ? 'No refresh attempt recorded'
              : `Last refresh attempt ${new Date(state.lastRefreshAttemptAt).toLocaleTimeString()}`}
          </small>
        </div>
      )}
      <nav aria-label="Learning evidence views" role="tablist">
        {tabs.map((item) => (
          <button
            key={item.id}
            type="button"
            role="tab"
            aria-selected={tab === item.id}
            onClick={() => onSelectTab?.(item.id)}
          >
            {item.label}
          </button>
        ))}
      </nav>
      {state && (
        <div className="auto-trader-execution-quality-filters">
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

      {tab === 'activity' && (
        <>
          <div className="auto-trader-evidence-coverage-subheading">
            <strong>Evidence acquisition</strong>
            <small>
              {view.requestTotal == null
                ? 'Request totals unavailable'
                : `${numberOrUnavailable(view.requestTotal)} total requests · ${numberOrUnavailable(view.omittedRequests)} omitted from this page`}
            </small>
          </div>
          {view.requestStates == null ? (
            <p>Request state is unavailable; no state counts are inferred.</p>
          ) : view.requestStates.length === 0 ? (
            <p>
              No evidence requests were returned. This does not establish that
              no evidence gaps exist.
            </p>
          ) : (
            <ul aria-label="Evidence requests by reported state">
              {view.requestStates.map(({ status, count }) => (
                <li key={status}>
                  {requestStatus(status)}: {count}
                </li>
              ))}
            </ul>
          )}
        </>
      )}

      {tab === 'evidence' && (
        <>
          {view.evidenceGaps == null ? (
            <p>Evidence gap count unavailable.</p>
          ) : (
            <p>
              {view.evidenceGaps} explicit missing, stale or reasoned coverage
              gaps in the returned cohort page.
            </p>
          )}

          <div className="auto-trader-evidence-coverage-subheading">
            <strong>Evidence coverage by profile</strong>
            <small>
              {view.cohortTotal == null
                ? 'Coverage cohorts unavailable'
                : `${view.cohorts?.length ?? 0} / ${view.cohortTotal} cohorts returned${(view.omittedCohorts ?? 0) > 0 ? ` · ${view.omittedCohorts} omitted` : ''}`}
            </small>
          </div>
          {view.cohorts == null ? (
            <p>
              Coverage is unavailable. Sample counts and eligibility are not
              assumed to be zero.
            </p>
          ) : view.cohorts.length === 0 ? (
            <p>
              No coverage cohorts were returned. Completeness cannot be inferred
              from an empty page.
            </p>
          ) : (
            <div className="auto-trader-evidence-request-list">
              {cohorts!.slice(0, 100).map((cohort) => (
                <article
                  className="auto-trader-evidence-request"
                  key={cohort.id}
                >
                  <header>
                    <div>
                      <strong>
                        {cohort.symbol} · {cohort.strategy}
                      </strong>
                      <small>Profile {cohort.profile}</small>
                    </div>
                    <span>{cohort.freshness ?? 'Freshness unavailable'}</span>
                  </header>
                  <dl>
                    <div>
                      <dt>Raw samples</dt>
                      <dd>{numberOrUnavailable(cohort.rawSamples)}</dd>
                    </div>
                    <div>
                      <dt>Effective samples</dt>
                      <dd>{numberOrUnavailable(cohort.effectiveSamples, 2)}</dd>
                    </div>
                    <div>
                      <dt>Required raw/effective samples</dt>
                      <dd>Unavailable from this coverage contract</dd>
                    </div>
                    <div>
                      <dt>Last sample</dt>
                      <dd>{timeOrUnavailable(cohort.lastSampleAt)}</dd>
                    </div>
                    <div>
                      <dt>Sample age</dt>
                      <dd>
                        {cohort.sampleAgeMs == null
                          ? 'Unavailable'
                          : `${numberOrUnavailable(cohort.sampleAgeMs / 1000, 0)} s`}
                      </dd>
                    </div>
                    <div>
                      <dt>Regime · horizon</dt>
                      <dd>
                        {cohort.regime ?? 'Unavailable'} ·{' '}
                        {cohort.horizon ?? 'Unavailable'}
                      </dd>
                    </div>
                    <div>
                      <dt>Coverage scope</dt>
                      <dd>{cohort.coverageScope}</dd>
                    </div>
                  </dl>
                  <p>{cohort.gapReason ?? 'No gap reason reported'}</p>
                </article>
              ))}
            </div>
          )}
          <div className="auto-trader-evidence-coverage-subheading">
            <strong>Recent strict-decision blockers</strong>
            <small>
              Only blockers returned by the current AutoTrader decision audit
              are shown.
            </small>
          </div>
          {decisionBlockers.length === 0 ? (
            <p>
              No recent blockers were reported, or the audit has no blocker
              detail.
            </p>
          ) : (
            <ul aria-label="Strict decision blockers">
              {decisionBlockers.slice(0, 20).map((blocker, index) => (
                <li key={`${blocker.strategy}:${blocker.symbol}:${index}`}>
                  {blocker.strategy} · {blocker.symbol}: {blocker.reason}
                </li>
              ))}
            </ul>
          )}
        </>
      )}

      {tab === 'activity' && (
        <>
          <div className="auto-trader-evidence-coverage-subheading">
            <strong>Request detail</strong>
            <small>
              Completed acquisition work does not by itself mean evidence was
              accepted.
            </small>
          </div>
          {requests == null ? (
            <p>Request details unavailable.</p>
          ) : requests.length === 0 ? (
            <p>No request details returned.</p>
          ) : (
            <div className="auto-trader-evidence-request-list">
              {requests.slice(0, 100).map((request) => (
                <article
                  className="auto-trader-evidence-request"
                  key={request.id}
                >
                  <header>
                    <strong>
                      {request.symbol} · {request.strategy} · {request.profile}
                    </strong>
                    <span>{requestStatus(request.status)}</span>
                  </header>
                  <p>{request.reason ?? 'Request reason unavailable'}</p>
                  <small>
                    Target {request.regime} · {request.horizon} · attempts{' '}
                    {numberOrUnavailable(request.attempts)} · updated{' '}
                    {timeOrUnavailable(request.updatedAt)} · next attempt{' '}
                    {timeOrUnavailable(request.nextAttemptAt)}
                  </small>
                </article>
              ))}
            </div>
          )}
        </>
      )}

      {tab === 'history' && (
        <>
          <div className="auto-trader-evidence-coverage-subheading">
            <strong>Evidence and trading outcomes</strong>
            <small>
              Scout outcome totals have no reported interval and are not cadence
              or accepted-gain measurements.
            </small>
          </div>
          {scoutEvidence == null ? (
            <p>Scout outcome summary unavailable.</p>
          ) : scoutEvidence.length === 0 ? (
            <p>
              {state?.filters.symbol && view.scoutEvidence?.length
                ? 'Scout aggregates are strategy-level; the source does not link them to this symbol.'
                : 'No Scout outcome samples are reported.'}
            </p>
          ) : (
            <ul aria-label="Scout research evidence">
              {scoutEvidence.map((row) => (
                <li key={row.strategy}>
                  {row.strategy}: {row.samples} reported Scout outcomes ·
                  positive rate{' '}
                  {row.positiveRate == null
                    ? 'unavailable'
                    : `${(row.positiveRate * 100).toFixed(1)}%`}{' '}
                  · Scout return{' '}
                  {row.averageReturnPercent == null
                    ? 'unavailable'
                    : `${row.averageReturnPercent.toFixed(3)}%`}
                </li>
              ))}
            </ul>
          )}
          {actualTradeEvidence == null ? (
            <p>Profile-linked actual-trade summary unavailable.</p>
          ) : actualTradeEvidence.length === 0 ? (
            <p>
              {state?.filters.symbol && view.actualTradeEvidence?.length
                ? 'Trade aggregates are strategy-level; the source does not link them to this symbol.'
                : 'No profile-linked actual-trade sample fields were returned.'}
            </p>
          ) : (
            <ul aria-label="Profile-linked actual trading outcomes">
              {actualTradeEvidence.map((row) => (
                <li key={row.strategy}>
                  {row.strategy}: active profile {row.activeSamples} samples,
                  mean net return{' '}
                  {row.activeAverageReturnPercent == null
                    ? 'unavailable'
                    : `${row.activeAverageReturnPercent.toFixed(3)}%`}
                  ; other profiles {row.otherSamples} samples, mean net return{' '}
                  {row.otherAverageReturnPercent == null
                    ? 'unavailable'
                    : `${row.otherAverageReturnPercent.toFixed(3)}%`}
                  .
                </li>
              ))}
            </ul>
          )}
          {actualTradeEvidence?.length ? (
            <p>
              These are Learning-reported aggregates. Event-level independent
              financial reconciliation is not available in this summary.
            </p>
          ) : null}
          <p>
            Scout sample totals are cumulative or interval-unspecified source
            counts. They do not show collection cadence, newly accepted
            evidence, or effective gain over a selected period.
          </p>
        </>
      )}

      {tab === 'promotion' && (
        <>
          <p>
            Accepted evidence gains and promotion history are unavailable here:
            current contracts do not provide stable accepted-gain or promotion
            event attribution. No promotion or profitability claim is inferred.
          </p>
        </>
      )}
    </section>
  );
}
