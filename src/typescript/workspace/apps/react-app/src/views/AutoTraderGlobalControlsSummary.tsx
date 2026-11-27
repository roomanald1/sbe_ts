import type { AutoTraderState } from '../models/AutoTraderModel';
import { OperatorKeyValueList } from '@workspace/ui-platform';

function controlUsd(value: string | number, loading: boolean): string {
  if (loading) return 'Unavailable';
  const amount = typeof value === 'number' ? value : Number(value);
  return Number.isFinite(amount)
    ? new Intl.NumberFormat(undefined, {
        style: 'currency',
        currency: 'USD',
        minimumFractionDigits: Number.isInteger(amount) ? 0 : 2,
        maximumFractionDigits: 2,
      }).format(amount)
    : 'Unavailable';
}

/** Primary summary contains account-wide limits, never per-position exits. */
export function AutoTraderGlobalControlsSummary({
  state,
  onViewLearnedPolicies,
  learnedPoliciesOpen = false,
}: {
  state: AutoTraderState;
  onViewLearnedPolicies: () => void;
  learnedPoliciesOpen?: boolean;
}) {
  const controlsLeft = [
    {
      label: 'Available capital',
      value: controlUsd(state.availableNotional, state.loading),
    },
    {
      label: 'Max order',
      value: controlUsd(state.maxOrderNotional, state.loading),
    },
    {
      label: 'Max positions',
      value:
        !state.loading && state.maxOpenPositions.trim()
          ? state.maxOpenPositions
          : 'Unavailable',
    },
  ];
  const controlsRight = [
    {
      label: 'Daily notional limit',
      value: controlUsd(
        Number.isFinite(state.serverDailyNotionalLimit)
          ? state.serverDailyNotionalLimit
          : state.dailyNotionalLimit,
        state.loading,
      ),
    },
    {
      label: 'Daily loss limit',
      value: controlUsd(state.dailyRealizedLossLimit, state.loading),
    },
    {
      label: 'Exploration',
      value:
        !state.loading && Number.isFinite(state.serverExplorationPercent)
          ? `${state.serverExplorationPercent}%`
          : 'Unavailable',
    },
  ];

  return (
    <div
      className="auto-trader-controls-grid"
      role="group"
      aria-label="Global risk limits"
    >
      <h2 className="auto-trader-controls-heading">Global risk limits</h2>
      <section role="group" aria-label="Capital and order limits">
        <OperatorKeyValueList
          label="Capital and order limits"
          rows={controlsLeft}
        />
      </section>
      <section role="group" aria-label="Daily risk limits">
        <OperatorKeyValueList label="Daily risk limits" rows={controlsRight} />
      </section>
      <section role="group" aria-label="Profile-owned exit parameters">
        <OperatorKeyValueList
          label="Profile-owned exit parameters"
          rows={[
            { label: 'Exit policy', value: 'Profile-owned' },
            {
              label: 'Parameter source',
              value: (
                <button
                  type="button"
                  aria-expanded={learnedPoliciesOpen}
                  aria-controls="autotrader-exit-policies"
                  onClick={onViewLearnedPolicies}
                >
                  {learnedPoliciesOpen
                    ? 'Hide learned policies'
                    : 'View learned policies'}
                </button>
              ),
            },
          ]}
        />
      </section>
      <section role="group" aria-label="Audit and safety latch">
        <OperatorKeyValueList
          label="Audit and safety latch"
          rows={[
            {
              label: 'Audit',
              value: state.auditHealthy ? 'OK' : 'Degraded',
              tone: state.auditHealthy ? 'positive' : 'negative',
            },
            {
              label: 'Safety latch',
              value: state.auditFailureLatched ? 'Set' : 'Clear',
              tone: state.auditFailureLatched ? 'negative' : 'positive',
            },
          ]}
        />
      </section>
    </div>
  );
}

export function AutoTraderBootstrapFallbackSummary({
  stopLossPercent,
  takeProfitPercent,
  loading,
}: {
  stopLossPercent: number;
  takeProfitPercent: number;
  loading: boolean;
}) {
  const value = (percent: number) =>
    !loading && Number.isFinite(percent)
      ? `${percent.toFixed(2)}%`
      : 'Unavailable · settings not loaded';
  return (
    <section aria-label="Advanced bootstrap fallback thresholds">
      <strong>Advanced · bootstrap fallback thresholds</strong>
      <p>
        Global fallback values only. Per-position persisted stop and target
        thresholds are shown in Learned Exit Policies; missing position values
        remain unavailable there.
      </p>
      <dl>
        <div>
          <dt>Fallback stop loss</dt>
          <dd>{value(stopLossPercent)}</dd>
        </div>
        <div>
          <dt>Fallback take profit</dt>
          <dd>{value(takeProfitPercent)}</dd>
        </div>
      </dl>
    </section>
  );
}
