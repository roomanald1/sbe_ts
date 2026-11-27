import type { CapitalAllocation } from '../models/AutoTraderModel';

const allocationMoney = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
});
const allocationAmount = (value: number) =>
  allocationMoney.format(value === 0 ? 0 : value);
const allocationFixed = (value: number, digits: number) =>
  Number.isFinite(value) ? value.toFixed(digits) : 'unknown';

export function CapitalAllocationView({
  allocation,
  availableNotional,
}: {
  allocation?: CapitalAllocation;
  availableNotional: number;
}) {
  return (
    <section
      className="auto-trader-allocation"
      aria-label="Effective shared capital allocation"
      data-testid="capital-allocation"
    >
      <strong>Shared capital allocation</strong>
      {allocation ? (
        <>
          <small className="v35-operator-subtitle">
            {allocation.model === 'shared_pool'
              ? 'Autonomous pool across eligible strategies, symbols and horizons'
              : 'Reported capital allocation model'}
          </small>
          <dl
            className="v35-operator-metrics"
            aria-label="Shared capital metrics"
          >
            <div className="v35-operator-metric">
              <dt>Capital</dt>
              <dd>USD {allocationAmount(allocation.poolLimit)}</dd>
            </div>
            <div className="v35-operator-metric">
              <dt>Deployed</dt>
              <dd>USD {allocationAmount(allocation.used)}</dd>
            </div>
            <div className="v35-operator-metric">
              <dt>Reserved</dt>
              <dd>USD {allocationAmount(allocation.reserved)}</dd>
            </div>
            <div className="v35-operator-metric">
              <dt>Available</dt>
              <dd>USD {allocationAmount(allocation.available)}</dd>
            </div>
            <div className="v35-operator-metric">
              <dt>Utilisation</dt>
              <dd>
                {allocation.poolLimit > 0
                  ? `${allocationFixed(((allocation.used + allocation.reserved) / allocation.poolLimit) * 100, 1)}%`
                  : 'Unavailable'}
              </dd>
            </div>
          </dl>
          {allocation.rows.length === 0 ? (
            <p>No effective allocation rows are currently reported.</p>
          ) : (
            <div className="auto-trader-allocation-rows">
              {allocation.rows.map((row, index) => (
                <article
                  className="auto-trader-allocation-row"
                  key={`${row.strategy}:${row.symbol}:${row.horizon}:${index}`}
                >
                  <strong>
                    {row.strategy} · {row.symbol} · {row.horizon}
                  </strong>
                  <span>
                    Effective USD {allocationAmount(row.effectiveAllocation)} ·
                    reserved USD {allocationAmount(row.reserved)} · used USD{' '}
                    {allocationAmount(row.used)} · utilization{' '}
                    {row.utilization == null
                      ? 'unknown'
                      : allocationFixed(row.utilization * 100, 1) + '%'}
                  </span>
                  <span>
                    Evidence eligibility:{' '}
                    {row.evidenceEligible == null
                      ? 'unknown'
                      : row.evidenceEligible
                        ? 'eligible'
                        : 'not eligible'}
                    {' · '}State: {row.state || 'unknown'} ·{' '}
                    {row.reason || 'No allocation reason supplied'}
                  </span>
                </article>
              ))}
            </div>
          )}
        </>
      ) : (
        <p role="status">
          Effective allocation details are unavailable from the current
          AutoTrader status contract. Global available capacity: USD{' '}
          {allocationAmount(availableNotional)}. This does not indicate zero
          allocation.
        </p>
      )}
    </section>
  );
}
