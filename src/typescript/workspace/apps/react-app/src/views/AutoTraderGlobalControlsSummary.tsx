import type { AutoTraderState } from '../models/AutoTraderModel';

const limitMoney = new Intl.NumberFormat(undefined, {
  maximumFractionDigits: 2,
});

function controlUsd(value: string | number): string {
  const amount = typeof value === 'number' ? value : Number(value);
  return Number.isFinite(amount)
    ? `USD ${limitMoney.format(amount)}`
    : 'Unavailable';
}

/** Primary summary contains account-wide limits, never per-position exits. */
export function AutoTraderGlobalControlsSummary({
  state,
}: {
  state: AutoTraderState;
}) {
  const controls = [
    ['Available capacity', controlUsd(state.availableNotional)],
    ['Daily notional', controlUsd(state.dailyNotionalLimit)],
    ['Max order', controlUsd(state.maxOrderNotional)],
    ['Daily loss limit', controlUsd(state.dailyRealizedLossLimit)],
    ['Max positions', state.maxOpenPositions || 'Unavailable'],
    [
      'Exploration',
      Number.isFinite(state.serverExplorationPercent)
        ? `${state.serverExplorationPercent}%`
        : 'Unavailable',
    ],
  ] as const;

  return (
    <dl
      className="v35-operator-metrics v35-operator-metrics--controls"
      aria-label="Effective global trading limits"
    >
      {controls.map(([label, value]) => (
        <div className="v35-operator-metric" key={label}>
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
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
