import { useEffect, useMemo } from 'react';
import { useBehaviorSnapshot } from '@workspace/ui-platform';
import { FeedToolbar } from './FeedToolbar';
import {
  financialSnapshotValue,
  PortfolioReconciliationWidgetModel,
  type EvidenceStatus,
  type PortfolioInterval,
  type PortfolioReconciliationState,
  visiblePortfolioIntervals,
} from '../models/PortfolioReconciliationWidgetModel';

function amount(value: unknown, currency = 'USD'): string {
  return typeof value === 'number' && Number.isFinite(value)
    ? new Intl.NumberFormat(undefined, {
        style: 'currency',
        currency,
        maximumFractionDigits: 2,
      }).format(value)
    : 'Unavailable';
}

function count(value: unknown): string {
  return typeof value === 'number' && Number.isFinite(value)
    ? String(value)
    : 'Unavailable';
}

function timestamp(value: unknown): string {
  return typeof value === 'number' && Number.isFinite(value)
    ? new Date(value).toLocaleString()
    : 'Unavailable';
}

function statusLabel(
  value: EvidenceStatus | string | null | undefined,
): string {
  if (!value) return 'Unavailable';
  return value.replaceAll('_', ' ');
}

const panelStyle = {
  background: '#20252b',
  border: '1px solid #3a424c',
  borderRadius: 8,
  padding: 12,
  color: '#d7dde5',
} as const;
const mutedStyle = { color: '#9ba7b4', fontSize: 12 } as const;
const tileStyle = {
  background: '#29323c',
  border: '1px solid #3a4652',
  borderRadius: 6,
  padding: '8px 10px',
  minWidth: 130,
} as const;
const sectionStyle = {
  borderTop: '1px solid #3a424c',
  marginTop: 12,
  paddingTop: 10,
} as const;

function badgeStyle(status: EvidenceStatus | string | null | undefined) {
  const normalized = status ?? 'unavailable';
  const color =
    normalized === 'complete'
      ? '#80d7a2'
      : normalized === 'incomplete'
        ? '#ff8f8f'
        : normalized === 'stale'
          ? '#ffbd69'
          : normalized === 'provisional' || normalized === 'not_evaluated'
            ? '#e3c477'
            : '#a4afbb';
  return {
    color,
    border: `1px solid ${color}66`,
    background: `${color}14`,
    borderRadius: 999,
    padding: '2px 8px',
    fontSize: 11,
    whiteSpace: 'nowrap' as const,
  };
}

function statusClass(
  status: EvidenceStatus | string | null | undefined,
): string {
  if (status === 'complete') return 'is-good';
  if (status === 'incomplete') return 'is-bad';
  if (
    status === 'provisional' ||
    status === 'not_evaluated' ||
    status === 'stale'
  )
    return 'is-warning';
  return 'is-unavailable';
}

function metric(label: string, value: string, detail?: string) {
  return (
    <div
      className="position-total-card v35-operator-metric"
      key={label}
      style={tileStyle}
    >
      <span style={mutedStyle}>{label}</span>
      <strong style={{ display: 'block', fontSize: 16, margin: '4px 0' }}>
        {value}
      </strong>
      {detail && <small style={mutedStyle}>{detail}</small>}
    </div>
  );
}

function reportedReturnChart(intervals: PortfolioInterval[]) {
  const points = intervals
    .filter(
      (interval) =>
        interval.completeness === 'complete' && interval.returnPercent !== null,
    )
    .slice(-40);
  if (points.length < 2)
    return (
      <p>Completed contribution-adjusted return history is unavailable.</p>
    );
  const values = points.map((point) => point.returnPercent as number);
  const min = Math.min(...values);
  const max = Math.max(...values);
  const span = max - min || 1;
  const coordinates = values
    .map((value, index) => {
      const x = 8 + (index / (values.length - 1)) * 284;
      const y = 92 - ((value - min) / span) * 82;
      return `${x},${y}`;
    })
    .join(' ');
  return (
    <figure aria-label="Portfolio return history" style={{ margin: '8px 0' }}>
      <svg
        role="img"
        aria-label="Authoritative complete interval returns"
        viewBox="0 0 300 100"
        width="100%"
        height="96"
        style={{ color: '#8ca8c1', background: '#252c33', borderRadius: 5 }}
      >
        <line
          x1="8"
          y1="92"
          x2="292"
          y2="92"
          stroke="#4a5662"
          strokeWidth="1"
        />
        <polyline
          points={coordinates}
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
        />
      </svg>
      <figcaption style={mutedStyle}>
        PM-reported complete interval returns only; {points.length} intervals.
        No returns are inferred from flat cash or incomplete intervals.
      </figcaption>
    </figure>
  );
}

function currencyRows(value: unknown): Array<[string, number]> {
  if (value === null || typeof value !== 'object' || Array.isArray(value))
    return [];
  return Object.entries(value as Record<string, unknown>).flatMap(
    ([currency, amountValue]) =>
      typeof amountValue === 'number' && Number.isFinite(amountValue)
        ? [[currency, amountValue]]
        : [],
  );
}

function intervalRow(interval: PortfolioInterval) {
  const raw = interval.raw;
  const id = interval.evidenceId ?? 'Evidence ID unavailable';
  const fxTimestamps = currencyRows(
    raw.closing_fx_rates_updated_at_by_currency,
  );
  return (
    <tr key={id}>
      <td>{timestamp(interval.endMs)}</td>
      <td>{id}</td>
      <td>
        <span
          className={`v35-operator-status ${statusClass(interval.completeness)}`}
          style={badgeStyle(interval.completeness)}
        >
          {statusLabel(interval.completeness)}
        </span>
      </td>
      <td>
        {interval.returnPercent === null
          ? 'Unavailable'
          : `${interval.returnPercent.toFixed(4)}%`}
      </td>
      <td>
        {interval.netPnlUsd === null
          ? 'Unavailable'
          : amount(interval.netPnlUsd)}
      </td>
      <td>
        <span
          className={`v35-operator-status ${statusClass(interval.independentStatus)}`}
          style={badgeStyle(interval.independentStatus)}
        >
          {statusLabel(interval.independentStatus)}
        </span>
      </td>
      <td>
        <details>
          <summary>Source detail</summary>
          <dl
            style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(auto-fit,minmax(160px,1fr))',
              gap: '5px 14px',
              ...mutedStyle,
            }}
          >
            <div>
              <dt>Evidence epoch</dt>
              <dd>{String(raw.ledger_epoch ?? 'Unavailable')}</dd>
            </div>
            <div>
              <dt>Fill cursor</dt>
              <dd>
                {String(raw.fill_cursor_start ?? 'Unavailable')} –{' '}
                {String(raw.fill_cursor_end ?? 'Unavailable')}
              </dd>
            </div>
            <div>
              <dt>PM trade-row cursor</dt>
              <dd>{count(raw.trade_row_cursor_end)}</dd>
            </div>
            <div>
              <dt>Independent source</dt>
              <dd>
                {String(
                  raw.baseline_independent_source_system ?? 'Unavailable',
                )}
              </dd>
            </div>
            <div>
              <dt>Independent source cursor</dt>
              <dd>{count(raw.baseline_independent_source_cursor)}</dd>
            </div>
            <div>
              <dt>Paper Exchange source cursor</dt>
              <dd>{count(raw.paper_exchange_source_cursor)}</dd>
            </div>
            <div>
              <dt>Interval valuation</dt>
              <dd>{String(raw.valuation_status ?? 'Unavailable')}</dd>
            </div>
            <div>
              <dt>FX observation</dt>
              <dd>{timestamp(raw.market_mark_at_utc_ms)}</dd>
            </div>
            <div>
              <dt>Independent execution comparison</dt>
              <dd>{statusLabel(interval.independentStatus)}</dd>
            </div>
            <div>
              <dt>Fee coverage</dt>
              <dd>
                {count(raw.known_fee_fills)} known /{' '}
                {count(raw.unknown_fee_fills)} unknown
              </dd>
            </div>
            <div>
              <dt>Cash-movement coverage</dt>
              <dd>
                {raw.coverage_gap === true
                  ? 'Gap reported'
                  : raw.coverage_gap === false
                    ? 'No gap reported'
                    : 'Unavailable'}
              </dd>
            </div>
            <div>
              <dt>Contribution coverage</dt>
              <dd>
                {raw.external_contribution_events === undefined
                  ? 'Unavailable'
                  : `${count((raw.external_contribution_events as unknown[]).length)} sourced events`}
              </dd>
            </div>
            <div>
              <dt>FX coverage and observation time</dt>
              <dd>
                {fxTimestamps.length
                  ? fxTimestamps
                      .map(
                        ([code, asOf]) =>
                          `${code}: ${timestamp(asOf)} (${interval.endMs === null ? 'age unavailable' : `${Math.max(0, interval.endMs - asOf).toLocaleString()} ms old at interval end`})`,
                      )
                      .join('; ')
                  : 'Unavailable'}
              </dd>
            </div>
          </dl>
          {interval.reasons.length > 0 && (
            <p>Completeness reasons: {interval.reasons.join('; ')}</p>
          )}
          <details>
            <summary>Source provenance</summary>
            <pre>{JSON.stringify(raw.provenance ?? raw, null, 2)}</pre>
          </details>
        </details>
      </td>
    </tr>
  );
}

export function PortfolioReconciliationPanel({
  state,
  onTabChange,
  onIntervalFilterChange,
  onRefresh,
}: {
  state: PortfolioReconciliationState;
  onTabChange?: (tab: PortfolioReconciliationState['tab']) => void;
  onIntervalFilterChange?: (
    filter: PortfolioReconciliationState['intervalFilter'],
  ) => void;
  onRefresh?: () => void;
}) {
  const tab = state.tab;
  const snapshot = state.snapshot;
  const account = financialSnapshotValue(snapshot, 'account');
  const ledgerCoverage = financialSnapshotValue(snapshot, 'ledger_coverage');
  const accountCurrency = financialSnapshotValue(account, 'currency');
  const currency =
    typeof accountCurrency === 'string' ? accountCurrency : 'USD';
  const latest = state.intervals.at(-1);
  const cashBalances = currencyRows(latest?.raw.cash_balances_by_currency);
  const contributions = currencyRows(
    latest?.raw.external_contributions_by_currency,
  );
  const allocation = state.allocation;
  const allocationRows = allocation?.rows ?? [];
  const cashValue = financialSnapshotValue(account, 'cash');
  const equityValue = financialSnapshotValue(account, 'equity');
  const realizedValue = financialSnapshotValue(account, 'realized_pnl');
  const unrealizedValue = financialSnapshotValue(account, 'unrealized_pnl');
  const paidFeesValue = financialSnapshotValue(account, 'fees_paid');
  const exitFeesValue = financialSnapshotValue(account, 'estimated_exit_fees');
  const snapshotCapturedAt = financialSnapshotValue(snapshot, 'captured_at_ms');
  const epoch = financialSnapshotValue(ledgerCoverage, 'ledger_evidence_epoch');
  const epochStatus = financialSnapshotValue(
    ledgerCoverage,
    'ledger_evidence_epoch_status',
  );
  const independent = financialSnapshotValue(
    ledgerCoverage,
    'independent_execution_ledger_reconciled',
  );
  const independentStatus: EvidenceStatus =
    independent === true
      ? 'complete'
      : independent === false
        ? 'incomplete'
        : 'unavailable';
  const allocationUtilization =
    allocation &&
    Number.isFinite(allocation.used) &&
    Number.isFinite(allocation.reserved) &&
    Number.isFinite(allocation.poolLimit) &&
    allocation.poolLimit > 0
      ? `${(((allocation.used + allocation.reserved) / allocation.poolLimit) * 100).toFixed(2)}%`
      : 'Unavailable';

  return (
    <section
      className="feed-section portfolio-reconciliation-widget v35-operator-card"
      aria-label="Portfolio and reconciliation"
      style={panelStyle}
    >
      <FeedToolbar title="Portfolio and reconciliation" />
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          margin: '8px 0',
        }}
      >
        <span
          className={`v35-operator-status ${statusClass(state.stale ? 'stale' : state.dataStatus === 'available' ? 'complete' : state.dataStatus === 'partial' ? 'provisional' : 'unavailable')}`}
          style={badgeStyle(
            state.stale
              ? 'stale'
              : state.dataStatus === 'available'
                ? 'complete'
                : state.dataStatus === 'partial'
                  ? 'provisional'
                  : 'unavailable',
          )}
        >
          {state.stale
            ? 'Showing stale evidence'
            : `Data ${state.dataStatus.replaceAll('_', ' ')}`}
        </span>
        {state.refreshedAtMs !== null && (
          <span style={mutedStyle}>
            Updated {timestamp(state.refreshedAtMs)}
          </span>
        )}
        <button
          type="button"
          onClick={onRefresh}
          disabled={!onRefresh || state.loading}
          style={{ ...mutedStyle, marginLeft: 'auto' }}
        >
          Refresh
        </button>
      </div>
      {state.error && (
        <div
          className="positions-error"
          role="alert"
          style={badgeStyle('incomplete')}
        >
          {state.error}
        </div>
      )}
      {state.intervalError && (
        <div
          className="positions-error"
          role="status"
          style={badgeStyle('provisional')}
        >
          {state.intervalError}
        </div>
      )}
      {state.loading && (
        <p role="status">Loading authoritative portfolio evidence…</p>
      )}
      <p style={mutedStyle}>
        Values below are sourced from the Position Management financial snapshot
        and append-only portfolio intervals. Fees paid are already included in
        ledger P&amp;L; estimated exit fees remain separate. No React-side
        accounting is applied.
      </p>
      <nav
        className="v35-operator-tabs"
        aria-label="Portfolio views"
        style={{
          display: 'flex',
          gap: 6,
          borderBottom: '1px solid #3a424c',
          paddingBottom: 8,
        }}
      >
        <button
          type="button"
          aria-pressed={tab === 'summary'}
          onClick={() => onTabChange?.('summary')}
          style={
            tab === 'summary'
              ? { ...tileStyle, borderColor: '#8ca8c1' }
              : {
                  ...mutedStyle,
                  background: 'transparent',
                  border: '1px solid #3a424c',
                  borderRadius: 5,
                  padding: '6px 10px',
                }
          }
        >
          Summary
        </button>
        <button
          type="button"
          aria-pressed={tab === 'intervals'}
          onClick={() => onTabChange?.('intervals')}
          style={
            tab === 'intervals'
              ? { ...tileStyle, borderColor: '#8ca8c1' }
              : {
                  ...mutedStyle,
                  background: 'transparent',
                  border: '1px solid #3a424c',
                  borderRadius: 5,
                  padding: '6px 10px',
                }
          }
        >
          Intervals ({state.intervals.length})
        </button>
        <span
          className={`v35-operator-status ${statusClass(independentStatus)}`}
          style={{
            ...badgeStyle(independentStatus),
            marginLeft: 'auto',
            alignSelf: 'center',
          }}
        >
          Independent reconciliation:{' '}
          {independent === true
            ? 'complete'
            : independent === false
              ? 'not reconciled'
              : 'unavailable'}
        </span>
      </nav>
      <p style={{ ...mutedStyle, margin: '8px 0' }}>
        Match probes are not settlements or fills. Independent evidence requires
        authoritative execution and settlement records; matching PM projections
        alone is insufficient.
      </p>
      {tab === 'summary' && (
        <>
          <div
            className="positions-totals v35-operator-metrics"
            style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(auto-fit,minmax(145px,1fr))',
              gap: 7,
              marginTop: 10,
            }}
          >
            {metric(
              'PM opening capital',
              amount(financialSnapshotValue(account, 'initial_cash'), currency),
              'Position Management account contract',
            )}
            {metric(
              'Current cash',
              amount(cashValue, currency),
              'PM account snapshot',
            )}
            {metric(
              'Current equity',
              financialSnapshotValue(account, 'equity_available') === true
                ? amount(equityValue, currency)
                : 'Unavailable',
              'Only when PM marks report equity available',
            )}
            {metric(
              'Shared-pool available',
              allocation ? amount(allocation.available) : 'Unavailable',
              allocation
                ? `Pool limit ${amount(allocation.poolLimit)}`
                : 'Allocation contract unavailable',
            )}
            {metric(
              'Reserved capital',
              allocation ? amount(allocation.reserved) : 'Unavailable',
              allocation
                ? 'AutoTrader shared-pool reservations'
                : 'Allocation contract unavailable',
            )}
            {metric(
              'Deployed capital',
              allocation ? amount(allocation.used) : 'Unavailable',
              allocation
                ? 'AutoTrader shared-pool usage'
                : 'Allocation contract unavailable',
            )}
            {metric(
              'Capital utilisation',
              allocationUtilization,
              allocation
                ? `${allocationRows.length} strategy/symbol/horizon allocation rows`
                : 'Allocation contract unavailable',
            )}
            {metric(
              'Realised P&L',
              amount(realizedValue, currency),
              'PM ledger projection; fees already reflected',
            )}
            {metric(
              'Unrealised P&L',
              financialSnapshotValue(account, 'equity_available') === true
                ? amount(unrealizedValue, currency)
                : 'Unavailable',
              'PM ledger projection',
            )}
            {metric(
              'Fees paid',
              amount(paidFeesValue, currency),
              'Already reflected in PM ledger P&L',
            )}
            {metric(
              'Estimated exit fees',
              financialSnapshotValue(account, 'equity_available') === true
                ? amount(exitFeesValue, currency)
                : 'Unavailable',
              'Estimated; not yet paid',
            )}
            {metric(
              'Current drawdown',
              'Unavailable',
              'No authoritative drawdown series in the supplied contract',
            )}
            {metric(
              'Maximum drawdown',
              'Unavailable',
              'No authoritative drawdown series in the supplied contract',
            )}
          </div>

          <section
            aria-label="Portfolio currency balances"
            style={sectionStyle}
          >
            <h3 style={{ margin: '0 0 5px', fontSize: 14 }}>
              Cash by currency
            </h3>
            {cashBalances.length ? (
              <ul>
                {cashBalances.map(([code, value]) => (
                  <li key={code}>
                    {code}: {amount(value, code)}
                  </li>
                ))}
              </ul>
            ) : (
              <p>Per-currency interval cash balances unavailable.</p>
            )}
            <h3 style={{ margin: '10px 0 5px', fontSize: 14 }}>
              External contributions and withdrawals
            </h3>
            {contributions.length ? (
              <ul>
                {contributions.map(([code, value]) => (
                  <li key={code}>
                    {code}: {amount(value, code)}
                  </li>
                ))}
              </ul>
            ) : (
              <p>
                Contribution totals unavailable; trade cashflows are not treated
                as contributions.
              </p>
            )}
          </section>

          <section aria-label="Portfolio evidence status" style={sectionStyle}>
            <h3 style={{ margin: '0 0 6px', fontSize: 14 }}>
              Evidence and reconciliation
            </h3>
            <dl
              style={{
                display: 'grid',
                gridTemplateColumns: 'repeat(auto-fit,minmax(190px,1fr))',
                gap: '5px 14px',
                ...mutedStyle,
              }}
            >
              <div>
                <dt>Snapshot</dt>
                <dd>{timestamp(snapshotCapturedAt)}</dd>
              </div>
              <div>
                <dt>Financial evidence epoch</dt>
                <dd>
                  {String(epoch ?? 'Unavailable')} ·{' '}
                  {String(epochStatus ?? 'Unavailable')}
                </dd>
              </div>
              <div>
                <dt>PM interval coverage</dt>
                <dd>
                  {state.intervalCoverage ?? 'Unavailable'}
                  {state.truncated === true ? ' (truncated)' : ''}
                </dd>
              </div>
              <div>
                <dt>Independent execution comparison</dt>
                <dd>
                  {independent === true
                    ? 'Complete'
                    : independent === false
                      ? 'Not reconciled'
                      : 'Unavailable'}
                </dd>
              </div>
              <div>
                <dt>Independent matched / unmatched records</dt>
                <dd>Unavailable in the PM snapshot contract</dd>
              </div>
              <div>
                <dt>Intervals returned</dt>
                <dd>{state.intervals.length}</dd>
              </div>
              <div>
                <dt>Complete intervals</dt>
                <dd>
                  {
                    state.intervals.filter(
                      (item) => item.completeness === 'complete',
                    ).length
                  }{' '}
                  / {state.intervals.length}
                </dd>
              </div>
              <div>
                <dt>Independent evidence</dt>
                <dd>
                  PM and Paper Exchange source records are distinct inputs where
                  linked; agreement between PM projections alone is not
                  independent reconciliation.
                </dd>
              </div>
            </dl>
            <p style={{ ...mutedStyle, ...badgeStyle('not_evaluated') }}>
              Flat-cash interval agreement does not establish trade
              profitability.
            </p>
          </section>
        </>
      )}
      {tab === 'intervals' && (
        <section
          aria-label="Portfolio historical evidence"
          style={sectionStyle}
        >
          <h3 style={{ margin: '0 0 6px', fontSize: 14 }}>
            Contribution-adjusted interval returns
          </h3>
          <label style={mutedStyle}>
            Interval status{' '}
            <select
              aria-label="Interval status filter"
              value={state.intervalFilter}
              onChange={(event) =>
                onIntervalFilterChange?.(
                  event.target
                    .value as PortfolioReconciliationState['intervalFilter'],
                )
              }
            >
              <option value="all">All intervals</option>
              <option value="complete">Complete</option>
              <option value="incomplete">Incomplete or unavailable</option>
            </select>
          </label>
          {reportedReturnChart(visiblePortfolioIntervals(state))}
          <h3 style={{ margin: '10px 0 6px', fontSize: 14 }}>
            Interval evidence and drill-down
          </h3>
          {visiblePortfolioIntervals(state).length ? (
            <div style={{ overflowX: 'auto' }}>
              <table
                className="v35-operator-table"
                style={{
                  width: '100%',
                  borderCollapse: 'collapse',
                  textAlign: 'left',
                  fontSize: 12,
                }}
              >
                <thead>
                  <tr>
                    {[
                      'End time',
                      'Evidence ID',
                      'PM completeness',
                      'Reported return',
                      'Net P&L',
                      'Independent status',
                      'Drill-down',
                    ].map((heading) => (
                      <th
                        key={heading}
                        style={{
                          color: '#9ba7b4',
                          padding: '6px',
                          borderBottom: '1px solid #3a424c',
                        }}
                      >
                        {heading}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {visiblePortfolioIntervals(state).map(intervalRow)}
                </tbody>
              </table>
            </div>
          ) : (
            <p style={mutedStyle}>
              No durable portfolio interval evidence is available.
            </p>
          )}
        </section>
      )}
    </section>
  );
}

export function PortfolioReconciliationWidget() {
  const model = useMemo(() => new PortfolioReconciliationWidgetModel(), []);
  const state = useBehaviorSnapshot(model.state$);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  return (
    <PortfolioReconciliationPanel
      state={state}
      onTabChange={(tab) => model.setTab(tab)}
      onIntervalFilterChange={(filter) => model.setIntervalFilter(filter)}
      onRefresh={() => void model.refresh()}
    />
  );
}
