import {
  CompactTable,
  OperatorMetricStrip,
  OperatorStackedBar,
  OperatorTabBar,
} from '@workspace/ui-platform';
import type {
  CapitalAllocationGroup,
  CapitalAllocationTab,
  CapitalAllocationWidgetState,
} from '../models/CapitalAllocationWidgetModel';

const usd = new Intl.NumberFormat(undefined, {
  style: 'currency',
  currency: 'USD',
  maximumFractionDigits: 2,
});

function amount(value: number | null | undefined): string {
  return typeof value === 'number' && Number.isFinite(value)
    ? usd.format(Object.is(value, -0) ? 0 : value)
    : 'Unavailable';
}

function percent(value: number | null | undefined): string {
  return typeof value === 'number' && Number.isFinite(value)
    ? value.toFixed(2) + '%'
    : 'Unavailable';
}

function eligibility(value: boolean | null): string {
  return value === true
    ? 'Eligible'
    : value === false
      ? 'Not eligible'
      : 'Unknown';
}

function rowReason(reasons: string[]): string {
  return reasons.length ? reasons.join('; ') : 'Unavailable';
}

export function CapitalAllocationView({
  state,
  onSetTab,
  onSetGroupBy,
}: {
  state: CapitalAllocationWidgetState;
  onSetTab: (tab: CapitalAllocationTab) => void;
  onSetGroupBy: (groupBy: CapitalAllocationGroup) => void;
}) {
  const allocation = state.capitalAllocation;
  const metrics = allocation
    ? [
        { label: 'Pool', value: amount(allocation.poolLimit) },
        { label: 'Deployed', value: amount(allocation.used) },
        { label: 'Reserved', value: amount(allocation.reserved) },
        { label: 'Available', value: amount(allocation.available) },
        { label: 'Utilisation', value: percent(state.utilizationPercent) },
      ]
    : [
        { label: 'Pool', value: 'Unavailable' },
        { label: 'Deployed', value: 'Unavailable' },
        { label: 'Reserved', value: 'Unavailable' },
        { label: 'Available', value: 'Unavailable' },
        { label: 'Utilisation', value: 'Unavailable' },
      ];

  return (
    <section
      className="capital-allocation-view"
      aria-label="Effective shared capital allocation"
      data-testid="capital-allocation"
    >
      {allocation ? (
        <>
          <OperatorMetricStrip
            label="Shared capital summary"
            metrics={metrics}
          />
          <OperatorStackedBar
            label="Shared capital pool composition"
            segments={state.poolSegments.map((segment) => ({
              id: segment.id,
              label:
                segment.id === 'used'
                  ? 'Deployed'
                  : segment.id === 'reserved'
                    ? 'Reserved'
                    : 'Available',
              value: amount(segment.value),
              sharePercent: segment.sharePercent,
              tone:
                segment.id === 'used'
                  ? 'positive'
                  : segment.id === 'reserved'
                    ? 'warning'
                    : 'neutral',
            }))}
          />
          {state.poolReconciles === false && (
            <p
              className="capital-allocation-message"
              data-tone="warning"
              role="status"
            >
              Reported deployed, reserved and available amounts do not sum to
              the pool limit. Values are shown as reported; no adjustment is
              inferred.
            </p>
          )}
          <div
            className="capital-allocation-legend"
            aria-label="Pool composition"
          >
            {state.poolSegments.map((segment) => (
              <span key={segment.id} data-segment={segment.id}>
                {segment.id === 'used'
                  ? 'Deployed'
                  : segment.id === 'reserved'
                    ? 'Reserved'
                    : 'Available'}{' '}
                {percent(segment.sharePercent)} · {amount(segment.value)}
              </span>
            ))}
            {!state.poolSegments.length && (
              <span>Pool composition unavailable from source values.</span>
            )}
          </div>
        </>
      ) : (
        <>
          <OperatorMetricStrip
            label="Shared capital summary"
            metrics={metrics}
          />
          <p className="capital-allocation-message" role="status">
            {state.loading
              ? 'Loading authoritative allocation status…'
              : 'Allocation details are unavailable from the current status contract.'}
            {!state.loading && Number.isFinite(state.availableNotional)
              ? ' Global available capacity: ' +
                amount(state.availableNotional) +
                '. This does not indicate zero allocation.'
              : ''}
          </p>
        </>
      )}

      <OperatorTabBar
        label="Capital allocation views"
        selectedId={state.tab}
        onSelect={(id) => onSetTab(id as CapitalAllocationTab)}
        tabs={[
          { id: 'allocation', label: 'Allocation' },
          { id: 'candidates', label: 'Candidates' },
          { id: 'history', label: 'History' },
        ]}
      />

      {state.tab !== 'history' && allocation && (
        <div className="capital-allocation-toolbar">
          <span className="capital-allocation-pool-context">
            Shared pool · all reported strategy, symbol and horizon rows
          </span>
          <label>
            Group by
            <select
              aria-label="Group allocations by"
              value={state.groupBy}
              onChange={(event) =>
                onSetGroupBy(event.target.value as CapitalAllocationGroup)
              }
            >
              <option value="strategy">Strategy</option>
              <option value="symbol">Symbol</option>
              <option value="horizon">Horizon</option>
            </select>
          </label>
        </div>
      )}

      {state.tab === 'allocation' && allocation && (
        <div className="capital-allocation-main">
          {state.displayedRows.length ? (
            <CompactTable label="Effective allocation grouped by source dimension">
              <thead>
                <tr>
                  <th>Group · {state.groupBy}</th>
                  <th>Effective</th>
                  <th>Deployed</th>
                  <th>Reserved</th>
                  <th>Pool share</th>
                  <th>Utilisation</th>
                  <th>Eligibility</th>
                  <th>State</th>
                  <th>Binding reason</th>
                </tr>
              </thead>
              <tbody>
                {state.displayedRows.map((row) => (
                  <tr key={row.key}>
                    <td title={row.label}>
                      <strong>{row.label}</strong>
                      <small>{row.rowCount} source row(s)</small>
                    </td>
                    <td>{amount(row.effectiveAllocation)}</td>
                    <td>{amount(row.used)}</td>
                    <td>{amount(row.reserved)}</td>
                    <td>{percent(row.sharePercent)}</td>
                    <td>{percent(row.utilizationPercent)}</td>
                    <td>{eligibility(row.evidenceEligible)}</td>
                    <td>{row.states.join(' · ') || 'Unknown'}</td>
                    <td
                      title={rowReason(row.reasons)}
                      className="capital-allocation-reason"
                    >
                      {rowReason(row.reasons)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </CompactTable>
          ) : (
            <p className="capital-allocation-message">
              No effective allocation rows are reported for the shared pool.
            </p>
          )}

          <div className="capital-allocation-lower-grid">
            <section aria-label="Reported allocation reasons">
              <header>
                <strong>Reported allocation reasons</strong>
                <span>Counts are source rows, not amounts</span>
              </header>
              {state.reportedReasonRows.length ? (
                <ul>
                  {state.reportedReasonRows.slice(0, 6).map((item) => (
                    <li key={item.reason} title={item.reason}>
                      <span>{item.reason}</span>
                      <strong>{item.rowCount.toLocaleString()}</strong>
                    </li>
                  ))}
                </ul>
              ) : (
                <p>No reason text supplied by the allocation status.</p>
              )}
            </section>

            <section aria-label="Remaining global limits">
              <header>
                <strong>Remaining limits</strong>
                <span>Source-backed AutoTrader status</span>
              </header>
              <dl>
                <div>
                  <dt>Position cap</dt>
                  <dd>
                    {state.maxOpenPositions === null
                      ? 'Unavailable'
                      : state.maxOpenPositions.toLocaleString()}
                  </dd>
                </div>
                <div>
                  <dt>Open position slots</dt>
                  <dd>Unavailable · slot usage not exposed</dd>
                </div>
                <div>
                  <dt>Daily notional remaining</dt>
                  <dd
                    data-tone={
                      state.dailyNotionalRemaining !== null &&
                      state.dailyNotionalRemaining < 0
                        ? 'negative'
                        : 'neutral'
                    }
                  >
                    {amount(state.dailyNotionalRemaining)}
                  </dd>
                </div>
                <div>
                  <dt>Exploration limit</dt>
                  <dd>{percent(state.explorationLimitPercent)}</dd>
                </div>
                <div>
                  <dt>Exploration usage</dt>
                  <dd>Unavailable · usage not exposed by allocation status</dd>
                </div>
              </dl>
            </section>
          </div>
        </div>
      )}

      {state.tab === 'candidates' && allocation && (
        <section
          className="capital-allocation-candidates"
          aria-label="Current allocation rows"
        >
          <header className="capital-allocation-section-heading">
            <strong>Current allocation rows</strong>
            <span>
              Expected edge, confidence and opportunity-specific proposal are
              unavailable in the v1 allocation status.
            </span>
          </header>
          {allocation.rows.length ? (
            <CompactTable label="Current allocation candidate details">
              <thead>
                <tr>
                  <th>Strategy · symbol · horizon</th>
                  <th>Effective</th>
                  <th>Deployed</th>
                  <th>Reserved</th>
                  <th>Utilisation</th>
                  <th>Eligibility</th>
                  <th>State</th>
                  <th>Binding reason</th>
                </tr>
              </thead>
              <tbody>
                {allocation.rows.map((row, index) => (
                  <tr
                    key={
                      row.strategy +
                      ':' +
                      row.symbol +
                      ':' +
                      row.horizon +
                      ':' +
                      index
                    }
                  >
                    <td>
                      <strong>{row.strategy}</strong>
                      <small>
                        {row.symbol} · {row.horizon}
                      </small>
                    </td>
                    <td>{amount(row.effectiveAllocation)}</td>
                    <td>{amount(row.used)}</td>
                    <td>{amount(row.reserved)}</td>
                    <td>
                      {percent(
                        row.utilization === null ? null : row.utilization * 100,
                      )}
                    </td>
                    <td>{eligibility(row.evidenceEligible)}</td>
                    <td>{row.state || 'Unknown'}</td>
                    <td className="capital-allocation-reason">
                      {row.reason || 'Unavailable'}
                    </td>
                  </tr>
                ))}
              </tbody>
            </CompactTable>
          ) : (
            <p className="capital-allocation-message">
              No allocation candidate rows are reported.
            </p>
          )}
        </section>
      )}

      {state.tab === 'candidates' && !allocation && (
        <p className="capital-allocation-message" role="status">
          Candidate eligibility is unavailable without the allocation status
          contract.
        </p>
      )}

      {state.tab === 'history' && (
        <p className="capital-allocation-message" role="status">
          Allocation history is unavailable. The current v1 status contract
          exposes a snapshot, not a historical series.
        </p>
      )}

      {state.error && (
        <p className="capital-allocation-message" role="alert">
          Allocation status error: {state.error}
        </p>
      )}
    </section>
  );
}
