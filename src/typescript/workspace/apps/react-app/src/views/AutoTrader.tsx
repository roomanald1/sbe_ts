import { useEffect, useMemo } from 'react';
import { AutoTraderPageModel } from '../models/AutoTraderPageModel';
import { AutoTraderOutcomes } from './AutoTraderOutcomes';
import { useBehaviorSnapshot } from '@workspace/ui-platform';
import { FeedToolbar } from './FeedToolbar';
import {
  explainAutoTraderReason,
  money,
  timestamp,
} from './AutoTraderOrderGrid';
import {
  AutoTraderParametersDialog,
  AutoTraderSettingsDialog,
} from './AutoTraderDialogs';

const finiteFixed = (value: unknown, digits: number) =>
  typeof value === 'number' && Number.isFinite(value)
    ? value.toFixed(digits)
    : 'n/a';
export function AutoTrader() {
  const pageModel = useMemo(() => new AutoTraderPageModel(), []);
  useEffect(() => {
    pageModel.start();
    return () => pageModel.stop();
  }, [pageModel]);
  const page = useBehaviorSnapshot(pageModel.state$);
  const model = pageModel.trader;
  const state = page.trader;
  const { settingsOpen, parametersOpen, decisionDetailsOpen } = page;
  const scoutClosedCount = page.scoutClosedCount;
  const scoutWinRate = page.scoutWinRate;
  return (
    <section
      className={
        'feed-section auto-trader-section' +
        (settingsOpen || parametersOpen ? ' has-open-popup' : '')
      }
      aria-label="Auto Trader"
    >
      <FeedToolbar title="Auto Trader" />
      <div className="auto-trader-content">
        <div
          className={
            'auto-trader-live-banner ' +
            (state.enabled ? 'is-live' : '') +
            (state.loading ? ' is-loading' : '')
          }
        >
          <span className="auto-trader-live-dot" aria-hidden="true" />
          <strong>
            {state.loading ? 'Connecting' : state.enabled ? 'LIVE' : 'PAUSED'}
          </strong>
          <span>
            {state.loading
              ? 'Checking status'
              : state.enabled
                ? 'Auto Trader is active'
                : 'Auto Trader is off'}
          </span>
          <div className="auto-trader-live-actions">
            <button
              type="button"
              disabled={state.loading || state.busy}
              onClick={() => void pageModel.setEnabled(!state.enabled)}
            >
              {state.busy ? 'Working…' : state.enabled ? 'Pause' : 'Go live'}
            </button>
          </div>
        </div>
        <div className="auto-trader-settings-anchor">
          <p>
            Paper · {state.serverExplorationPercent}% explore · SL{' '}
            {finiteFixed(state.stopLossPercent, 2)}% / TP{' '}
            {finiteFixed(state.takeProfitPercent, 2)}%
          </p>
          <div className="auto-trader-settings-actions">
            <button
              className="auto-trader-parameters-button"
              type="button"
              aria-expanded={parametersOpen}
              aria-haspopup="dialog"
              disabled={state.loading}
              onClick={() => pageModel.toggleParameters()}
            >
              View parameters
            </button>
            <button
              className="auto-trader-settings-button"
              type="button"
              aria-expanded={settingsOpen}
              aria-haspopup="dialog"
              disabled={state.loading}
              onClick={() => pageModel.toggleSettings()}
            >
              …
            </button>
          </div>
          {parametersOpen && (
            <AutoTraderParametersDialog
              state={state}
              onClose={() => pageModel.closeParameters()}
            />
          )}
          {settingsOpen && (
            <AutoTraderSettingsDialog
              state={state}
              model={model}
              onClose={() => pageModel.closeSettings()}
            />
          )}
        </div>
        {!state.auditHealthy && (
          <div className="auto-trader-audit-alert" role="alert">
            <strong>
              Audit persistence degraded · new strategy entries paused
            </strong>
            <span>
              Decision queue {state.decisionAuditQueueDepth.toLocaleString()} ·
              execution queue {state.executionAuditQueueDepth.toLocaleString()}{' '}
              · {state.auditDroppedEvents.toLocaleString()} audit events
              dropped.{' '}
              {state.auditFailureLatched
                ? 'Restore Insights audit storage, then restart Auto Trader to clear the latched queue overflow.'
                : 'Auto Trader will resume entries after queued audit writes recover.'}{' '}
              Position protection remains active.
            </span>
          </div>
        )}
        {state.auditCoalescedDecisions > 0 && (
          <p className="auto-trader-audit-coalesced" role="status">
            Coalesced {state.auditCoalescedDecisions.toLocaleString()}{' '}
            superseded rejected/skipped decision audits for symbols with newer
            decisions.
          </p>
        )}
        <p className="auto-trader-allocation">
          Reserved <strong>USD {money.format(state.usedNotionalToday)}</strong>{' '}
          / {money.format(state.serverDailyNotionalLimit)} total · Intraday{' '}
          {money.format(state.intradayUsedNotional)}/
          {money.format(Number(state.intradayNotionalLimit))} · Daily{' '}
          {money.format(state.dailyStrategyUsedNotional)}/
          {money.format(Number(state.dailyStrategyNotionalLimit))} · Available{' '}
          <strong>USD {money.format(state.availableNotional)}</strong>
          {state.usedNotionalToday > state.serverDailyNotionalLimit && (
            <span> · Overall cap exceeded; new exposure blocked.</span>
          )}
        </p>
        <div className="auto-trader-insights">
          <div
            className="auto-trader-insight auto-trader-decision"
            role="status"
          >
            <span className="auto-trader-insight-label">
              No trades · {state.decisionsSinceTrade.toLocaleString()} scans
            </span>
            {state.gapReasons.length > 0 ? (
              <details className="auto-trader-gap-details">
                <summary>Reasons · {state.gapReasons.length}</summary>
                <ol className="auto-trader-gap-reasons">
                  {state.gapReasons
                    .slice()
                    .sort((a, b) => b.count - a.count)
                    .slice(0, 3)
                    .map((item) => (
                      <li key={item.reason}>
                        <span>{explainAutoTraderReason(item.reason)}</span>
                        <strong>{item.count.toLocaleString()}</strong>
                      </li>
                    ))}
                </ol>
              </details>
            ) : (
              <span className="auto-trader-decision-text">
                {state.lastDecision
                  ? explainAutoTraderReason(state.lastDecision)
                  : 'Waiting for the first decision.'}
              </span>
            )}

            {state.lastDecisionAt > 0 && (
              <time dateTime={new Date(state.lastDecisionAt).toISOString()}>
                {timestamp.format(state.lastDecisionAt)}
              </time>
            )}
          </div>
          <div className="auto-trader-insight">
            <span className="auto-trader-insight-label">
              Learning · today UTC
            </span>
            <strong>
              {state.learningSamples} closed trades today ·{' '}
              {state.learningWinRate == null
                ? '—'
                : finiteFixed(state.learningWinRate * 100, 0) + '% positive'}
            </strong>
            <span className="auto-trader-learning-detail">
              Execution samples P0/P1/P2:{' '}
              {state.parameterProfileSamples.join('/')}{' '}
              {state.preferredOrderType
                ? '· Lowest observed cost: ' + state.preferredOrderType
                : ''}
            </span>
          </div>
          <div
            className="auto-trader-insight auto-trader-scout-metrics"
            role="status"
            aria-label="Scout trade summary"
          >
            <span className="auto-trader-insight-label">Scout trades</span>
            <strong>
              {page.scoutOpenCount} open · {scoutClosedCount} closed ·{' '}
              {scoutWinRate === undefined
                ? '—'
                : finiteFixed(scoutWinRate, 0) + '%'}{' '}
              win rate
            </strong>
          </div>
          <div className="auto-trader-insight auto-trader-decision-quality">
            <span className="auto-trader-insight-label">
              Decisions · today ·{' '}
              {state.decisionQuality.completed_trade_samples}/
              {state.decisionQuality.filled_entry_samples} closed
            </span>
            <strong>
              {state.decisionQuality.decision_samples.toLocaleString()}{' '}
              decisions · {state.decisionQuality.unresolved_trade_samples} open
            </strong>
            <button
              className="auto-trader-cohort-toggle"
              type="button"
              aria-expanded={decisionDetailsOpen}
              onClick={() => pageModel.toggleDecisionDetails()}
            >
              {decisionDetailsOpen
                ? 'Hide breakdown'
                : 'Breakdown · ' + state.decisionQuality.cohorts.length}
            </button>
            {decisionDetailsOpen && (
              <div className="auto-trader-learning-detail auto-trader-cohort-list">
                {state.decisionQuality.cohorts.map((cohort) => (
                  <span key={cohort.dimension + ':' + cohort.value}>
                    {cohort.dimension === 'entry_time_utc'
                      ? 'Entry time (UTC)'
                      : cohort.dimension === 'market_regime'
                        ? 'Market regime'
                        : cohort.dimension.split('_').join(' ')}
                    : {cohort.value} · {cohort.completed_trade_samples} trades ·{' '}
                    {cohort.average_net_return_percent >= 0 ? '+' : ''}
                    {finiteFixed(cohort.average_net_return_percent, 2)}% avg ·{' '}
                    {finiteFixed(cohort.win_rate * 100, 0)}% wins
                    {cohort.preliminary ? ' · preliminary' : ''}
                  </span>
                ))}
                {state.decisionQuality.cohorts.length === 0 && (
                  <span>
                    Completed, linked outcomes will appear here as trades close.
                  </span>
                )}
                {state.decisionQuality.unpriced_closed_trade_samples > 0 && (
                  <span>
                    {state.decisionQuality.unpriced_closed_trade_samples} closed
                    outcomes lack an entry price and are excluded.
                  </span>
                )}
                {state.decisionQuality.history_truncated && (
                  <span>Based on the latest 50,000 audit updates.</span>
                )}
              </div>
            )}
          </div>
          <div className="auto-trader-insight">
            <span className="auto-trader-insight-label">
              Closed today · {state.exitLearningSamples}
            </span>
            <strong>
              {state.closedTradeAverageReturnPercent == null
                ? 'Average return unavailable'
                : (state.closedTradeAverageReturnPercent >= 0 ? '+' : '') +
                  finiteFixed(state.closedTradeAverageReturnPercent, 3) +
                  '%'}
            </strong>
            <span className="auto-trader-learning-detail">
              {state.closedTradeWins} wins · {state.closedTradeLosses} losses ·{' '}
              {state.closedTradeFlat} near flat
            </span>
          </div>
        </div>
        <div
          className="auto-trader-pnl-summary"
          aria-label="Auto Trader profit and loss"
        >
          <span>
            Realized today:{' '}
            <strong
              className={
                state.realizedPnl == null
                  ? undefined
                  : state.realizedPnl >= 0
                    ? 'auto-trader-pnl-positive'
                    : 'auto-trader-pnl-negative'
              }
            >
              {state.realizedPnl == null
                ? 'Unavailable (position ledger or FX rate)'
                : 'USD ' + money.format(state.realizedPnl)}
            </strong>
          </span>
          <span>
            Open now:{' '}
            <strong
              className={
                state.unrealizedPnl == null
                  ? undefined
                  : state.unrealizedPnl >= 0
                    ? 'auto-trader-pnl-positive'
                    : 'auto-trader-pnl-negative'
              }
            >
              {state.unrealizedPnl == null
                ? 'Unavailable (missing mark or FX rate)'
                : 'USD ' + money.format(state.unrealizedPnl)}
            </strong>
          </span>
          <span>
            Combined today:{' '}
            <strong
              className={
                state.realizedPnl == null || state.unrealizedPnl == null
                  ? undefined
                  : state.realizedPnl + state.unrealizedPnl >= 0
                    ? 'auto-trader-pnl-positive'
                    : 'auto-trader-pnl-negative'
              }
            >
              {state.realizedPnl == null || state.unrealizedPnl == null
                ? 'Unavailable'
                : 'USD ' +
                  money.format(state.realizedPnl + state.unrealizedPnl)}
            </strong>
          </span>
        </div>
        {state.error && (
          <p className="auto-trader-error" role="alert">
            {state.error}
          </p>
        )}
        <AutoTraderOutcomes embedded strategyRatesOnly />
      </div>
    </section>
  );
}
