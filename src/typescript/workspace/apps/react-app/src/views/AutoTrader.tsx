import { useEffect, useMemo, useState } from 'react';
import { AutoTraderPageModel } from '../models/AutoTraderPageModel';
import { useBehaviorSnapshot } from '@workspace/ui-platform';
import { FeedToolbar } from './FeedToolbar';
import { money } from './AutoTraderOrderGrid';
import {
  AutoTraderParametersDialog,
  AutoTraderSettingsDialog,
} from './AutoTraderDialogs';

const finiteFixed = (value: unknown, digits: number) =>
  typeof value === 'number' && Number.isFinite(value)
    ? value.toFixed(digits)
    : 'n/a';

const STALE_AUTO_TRADER_DECISION_MS = 6 * 60_000;
const pnlMoney = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 5,
  maximumFractionDigits: 5,
});

export function AutoTrader() {
  const pageModel = useMemo(() => new AutoTraderPageModel(), []);
  const mountedAt = useMemo(() => Date.now(), []);
  const [clockNow, setClockNow] = useState(() => Date.now());
  useEffect(() => {
    pageModel.start();
    return () => pageModel.stop();
  }, [pageModel]);
  useEffect(() => {
    const timer = setInterval(() => setClockNow(Date.now()), 5_000);
    return () => clearInterval(timer);
  }, []);
  const page = useBehaviorSnapshot(pageModel.state$);
  const model = pageModel.trader;
  const state = page.trader;
  const { settingsOpen, parametersOpen } = page;
  const decisionReferenceAt = state.lastDecisionAt || mountedAt;
  const decisionAgeMs = Math.max(0, clockNow - decisionReferenceAt);
  const staleDecision =
    state.enabled && decisionAgeMs >= STALE_AUTO_TRADER_DECISION_MS;
  const decisionAgeMinutes = Math.floor(decisionAgeMs / 60_000);
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
        {staleDecision && (
          <div className="auto-trader-audit-alert" role="alert">
            <strong>No new AutoTrader decisions</strong>
            <span>
              No decision has completed for {decisionAgeMinutes} minutes.
              Strategy workers normally report at least every five minutes;
              check their report failures and AutoTrader RSocket health.
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
                : 'USD ' + pnlMoney.format(state.realizedPnl)}
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
                : 'USD ' + pnlMoney.format(state.unrealizedPnl)}
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
                  pnlMoney.format(state.realizedPnl + state.unrealizedPnl)}
            </strong>
          </span>
        </div>
        {state.error && (
          <p className="auto-trader-error" role="alert">
            {state.error}
          </p>
        )}
      </div>
    </section>
  );
}
