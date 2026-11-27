import { useEffect, useMemo } from 'react';
import {
  CompactTable,
  OperatorMetricStrip,
  OperatorTabBar,
  useBehaviorSnapshot,
} from '@workspace/ui-platform';
import {
  financialSnapshotValue,
  PortfolioReconciliationWidgetModel,
  type EvidenceStatus,
  type PortfolioInterval,
  type PortfolioReconciliationState,
  visiblePortfolioIntervals,
} from '../models/PortfolioReconciliationWidgetModel';

function amount(value: unknown, currency: string | null): string {
  return typeof value === 'number' && Number.isFinite(value) && currency
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

const mutedStyle = { color: '#9ba7b4', fontSize: 12 } as const;

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
          : amount(interval.netPnlUsd, 'USD')}
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
  const snapshot = state.snapshot;
  const account = financialSnapshotValue(snapshot, 'account');
  const ledgerCoverage = financialSnapshotValue(snapshot, 'ledger_coverage');
  const accountCurrency = financialSnapshotValue(account, 'currency');
  const currency =
    typeof accountCurrency === 'string' && accountCurrency.trim()
      ? accountCurrency
      : null;
  const initialCash = financialSnapshotValue(account, 'initial_cash');
  const cashValue = financialSnapshotValue(account, 'cash');
  const equityValue = financialSnapshotValue(account, 'equity');
  const realizedValue = financialSnapshotValue(account, 'realized_pnl');
  const unrealizedValue = financialSnapshotValue(account, 'unrealized_pnl');
  const paidFeesValue = financialSnapshotValue(account, 'fees_paid');
  const exitFeesValue = financialSnapshotValue(account, 'estimated_exit_fees');
  const equityAvailable =
    financialSnapshotValue(account, 'equity_available') === true;
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
  const latest = state.intervals.at(-1);
  const openingBalances = new Map(
    currencyRows(latest?.raw.opening_cash_balances_by_currency),
  );
  const closingBalances = new Map(
    currencyRows(latest?.raw.cash_balances_by_currency),
  );
  const contributions = new Map(
    currencyRows(latest?.raw.external_contributions_by_currency),
  );
  const openingUsdValues = new Map(
    currencyRows(latest?.raw.opening_cash_value_usd_by_currency),
  );
  const closingUsdValues = new Map(
    currencyRows(latest?.raw.closing_cash_value_usd_by_currency),
  );
  const openingFxRates = new Map(
    currencyRows(latest?.raw.opening_fx_rates_usd_by_currency),
  );
  const closingFxRates = new Map(
    currencyRows(latest?.raw.closing_fx_rates_usd_by_currency),
  );
  const openingFxTimestamps = new Map(
    currencyRows(latest?.raw.opening_fx_rates_updated_at_by_currency),
  );
  const closingFxTimestamps = new Map(
    currencyRows(latest?.raw.closing_fx_rates_updated_at_by_currency),
  );
  const currencyCodes = [
    ...new Set([
      ...openingBalances.keys(),
      ...closingBalances.keys(),
      ...contributions.keys(),
    ]),
  ].sort();
  const completeCount = state.intervals.filter(
    (interval) => interval.completeness === 'complete',
  ).length;
  const dataLabel = state.stale
    ? 'Stale'
    : state.dataStatus === 'available'
      ? 'Available'
      : state.dataStatus === 'partial'
        ? 'Partial'
        : state.dataStatus === 'loading'
          ? 'Loading'
          : 'Unavailable';
  const latestReturn =
    latest?.returnPercent === null || latest?.returnPercent === undefined
      ? 'Unavailable'
      : latest.returnPercent.toFixed(4) + '%';
  const sourceDate = timestamp(state.refreshedAtMs);
  const statusTone = statusClass(
    state.stale
      ? 'stale'
      : state.dataStatus === 'available'
        ? 'complete'
        : state.dataStatus === 'partial'
          ? 'provisional'
          : 'unavailable',
  );
  const formatFxRate = (value: number | undefined) =>
    typeof value === 'number' && Number.isFinite(value) && value > 0
      ? new Intl.NumberFormat(undefined, {
          maximumSignificantDigits: 8,
        }).format(value)
      : 'Unavailable';
  const amountFrom = (map: Map<string, number>, code: string) =>
    map.has(code) ? amount(map.get(code), code) : 'Unavailable';
  const usdValueFrom = (map: Map<string, number>, code: string) =>
    map.has(code) ? amount(map.get(code), 'USD') : 'Unavailable';
  const fxTimeFrom = (map: Map<string, number>, code: string) => {
    const value = map.get(code);
    return typeof value === 'number' && value > 0
      ? timestamp(value)
      : 'Unavailable';
  };
  const fxAge = (map: Map<string, number>, code: string) => {
    const value = map.get(code);
    return typeof value === 'number' &&
      value > 0 &&
      latest?.endMs !== null &&
      latest?.endMs !== undefined
      ? Math.max(0, latest.endMs - value).toLocaleString() + ' ms'
      : 'Unavailable';
  };

  return (
    <section
      className="portfolio-reconciliation-widget operator-panel"
      aria-label="Portfolio and reconciliation"
    >
      <div className="portfolio-widget-toolbar">
        <span className={'portfolio-widget-state ' + statusTone}>
          {dataLabel}
        </span>
        <span className="portfolio-widget-updated">As of {sourceDate}</span>
        <button
          type="button"
          onClick={onRefresh}
          disabled={!onRefresh || state.loading}
          aria-label="Refresh portfolio evidence"
        >
          {state.loading ? 'Refreshing…' : 'Refresh'}
        </button>
      </div>

      <OperatorMetricStrip
        label="Portfolio summary"
        metrics={[
          {
            label: 'Equity',
            value: equityAvailable
              ? amount(equityValue, currency)
              : 'Unavailable',
          },
          {
            label: 'Cash',
            value: amount(cashValue, currency),
          },
          {
            label: 'Net realised',
            value: amount(realizedValue, currency),
            tone:
              typeof realizedValue === 'number' && realizedValue > 0
                ? 'positive'
                : typeof realizedValue === 'number' && realizedValue < 0
                  ? 'negative'
                  : 'neutral',
          },
          {
            label: 'Unrealised',
            value: equityAvailable
              ? amount(unrealizedValue, currency)
              : 'Unavailable',
            tone:
              typeof unrealizedValue === 'number' && unrealizedValue > 0
                ? 'positive'
                : typeof unrealizedValue === 'number' && unrealizedValue < 0
                  ? 'negative'
                  : 'neutral',
          },
          {
            label: 'Fees paid',
            value: amount(paidFeesValue, currency),
          },
        ]}
      />

      <OperatorTabBar
        label="Portfolio views"
        selectedId={state.tab}
        onSelect={(id) =>
          onTabChange?.(id as PortfolioReconciliationState['tab'])
        }
        tabs={[
          { id: 'summary', label: 'Overview' },
          { id: 'balances', label: 'Balances' },
          { id: 'intervals', label: 'Reconciliation' },
        ]}
      />

      {state.error && (
        <p className="portfolio-widget-message is-error" role="alert">
          {state.error}
        </p>
      )}
      {state.intervalError && (
        <p className="portfolio-widget-message is-warning" role="status">
          {state.intervalError}
        </p>
      )}
      {state.loading && !snapshot && (
        <p className="portfolio-widget-message" role="status">
          Loading portfolio evidence…
        </p>
      )}

      {state.tab === 'summary' && (
        <div className="portfolio-overview">
          <section
            className="portfolio-overview-chart"
            aria-label="Portfolio interval return history"
          >
            <header>
              <strong>Contribution-adjusted interval return</strong>
              <span>Last 40 complete intervals</span>
            </header>
            {reportedReturnChart(state.intervals)}
          </section>

          <section
            className="portfolio-pnl-attribution"
            aria-label="Portfolio P&L attribution"
          >
            <header>
              <strong>Realised attribution</strong>
              <span>PM ledger values</span>
            </header>
            <dl>
              <div>
                <dt>Net realised</dt>
                <dd>{amount(realizedValue, currency)}</dd>
              </div>
              <div>
                <dt>Gross winning outcomes</dt>
                <dd>Unavailable</dd>
              </div>
              <div>
                <dt>Gross losing outcomes</dt>
                <dd>Unavailable</dd>
              </div>
              <div>
                <dt>Fees paid</dt>
                <dd>{amount(paidFeesValue, currency)}</dd>
              </div>
              <div>
                <dt>Unrealised</dt>
                <dd>
                  {equityAvailable
                    ? amount(unrealizedValue, currency)
                    : 'Unavailable'}
                </dd>
              </div>
              <div>
                <dt>Estimated exit fees</dt>
                <dd>
                  {equityAvailable
                    ? amount(exitFeesValue, currency)
                    : 'Unavailable'}
                </dd>
              </div>
            </dl>
            <p>
              Fees paid are already reflected in realised P&amp;L; exit fees are
              estimates.
            </p>
          </section>

          <CompactTable label="Portfolio account summary">
            <thead>
              <tr>
                <th>Account</th>
                <th>Currency</th>
                <th>Trading capital</th>
                <th>Cash</th>
                <th>Equity</th>
                <th>Net P&amp;L</th>
                <th>Fees</th>
                <th>Latest return</th>
                <th>Coverage</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td>Position Management</td>
                <td>{currency ?? 'Unavailable'}</td>
                <td>{amount(initialCash, currency)}</td>
                <td>{amount(cashValue, currency)}</td>
                <td>
                  {equityAvailable
                    ? amount(equityValue, currency)
                    : 'Unavailable'}
                </td>
                <td>{amount(realizedValue, currency)}</td>
                <td>{amount(paidFeesValue, currency)}</td>
                <td>{latestReturn}</td>
                <td>
                  {state.intervalCoverage
                    ? statusLabel(state.intervalCoverage) +
                      (state.truncated ? ' · truncated' : '')
                    : 'Unavailable'}
                </td>
              </tr>
            </tbody>
          </CompactTable>

          <div
            className="portfolio-reconciliation-banner"
            data-status={independentStatus}
          >
            <span>
              Independent reconciliation:{' '}
              {independent === true
                ? 'Reconciled'
                : independent === false
                  ? 'Not reconciled'
                  : 'Unavailable'}
            </span>
            <span>
              {latest?.completeness === 'complete'
                ? 'Latest interval complete'
                : latest
                  ? 'Latest interval ' + statusLabel(latest.completeness)
                  : 'No interval evidence'}
            </span>
            <button type="button" onClick={() => onTabChange?.('intervals')}>
              Review evidence
            </button>
          </div>
        </div>
      )}

      {state.tab === 'balances' && (
        <section
          className="portfolio-balances"
          aria-label="Portfolio balances by currency"
        >
          <header className="portfolio-section-heading">
            <strong>Per-currency valuation and contributions</strong>
            <span>
              Latest interval ·{' '}
              {latest ? timestamp(latest.endMs) : 'Unavailable'}
              {latest ? ' · ' + statusLabel(latest.completeness) : ''}
            </span>
          </header>
          {currencyCodes.length > 0 ? (
            <CompactTable label="Opening and closing balances by currency">
              <thead>
                <tr>
                  <th>Currency</th>
                  <th>Opening cash</th>
                  <th>External contributions</th>
                  <th>Closing cash</th>
                  <th>Opening USD value</th>
                  <th>Closing USD value</th>
                  <th>Opening FX rate</th>
                  <th>Opening FX as of</th>
                  <th>Closing FX rate</th>
                  <th>FX as of</th>
                  <th>FX age at interval end</th>
                </tr>
              </thead>
              <tbody>
                {currencyCodes.map((code) => (
                  <tr key={code}>
                    <td>{code}</td>
                    <td>{amountFrom(openingBalances, code)}</td>
                    <td>{amountFrom(contributions, code)}</td>
                    <td>{amountFrom(closingBalances, code)}</td>
                    <td>{usdValueFrom(openingUsdValues, code)}</td>
                    <td>{usdValueFrom(closingUsdValues, code)}</td>
                    <td>{formatFxRate(openingFxRates.get(code))}</td>
                    <td>{fxTimeFrom(openingFxTimestamps, code)}</td>
                    <td>{formatFxRate(closingFxRates.get(code))}</td>
                    <td>{fxTimeFrom(closingFxTimestamps, code)}</td>
                    <td>{fxAge(closingFxTimestamps, code)}</td>
                  </tr>
                ))}
              </tbody>
            </CompactTable>
          ) : (
            <p className="portfolio-widget-message">
              Per-currency opening values, contributions, closing cash and FX
              valuation are unavailable in the latest interval.
            </p>
          )}
          <div className="portfolio-balance-evidence">
            <OperatorMetricStrip
              label="Currency valuation evidence"
              metrics={[
                {
                  label: 'Cash movements',
                  value: currencyRows(latest?.raw.cash_movements_by_currency)
                    .length
                    ? currencyRows(
                        latest?.raw.cash_movements_by_currency,
                      ).length.toLocaleString()
                    : 'Unavailable',
                },
                {
                  label: 'Contribution events',
                  value: Array.isArray(latest?.raw.external_contribution_events)
                    ? latest.raw.external_contribution_events.length.toLocaleString()
                    : 'Unavailable',
                },
                {
                  label: 'Opening FX sources',
                  value: openingFxRates.size.toLocaleString(),
                },
                {
                  label: 'Closing FX sources',
                  value: closingFxRates.size.toLocaleString(),
                },
              ]}
            />
          </div>
        </section>
      )}

      {state.tab === 'intervals' && (
        <section
          className="portfolio-reconciliation"
          aria-label="Financial reconciliation"
        >
          <div className="portfolio-reconciliation-summary">
            <span data-status={statusClass(independentStatus)}>
              Independent ledger:{' '}
              {independent === true
                ? 'reconciled'
                : independent === false
                  ? 'not reconciled'
                  : 'unavailable'}
            </span>
            <span>Epoch: {String(epoch ?? 'Unavailable')}</span>
            <span>Epoch status: {String(epochStatus ?? 'Unavailable')}</span>
            <span>
              Interval coverage: {state.intervalCoverage ?? 'Unavailable'}
            </span>
            <span>
              Complete intervals: {completeCount.toLocaleString()} /{' '}
              {state.intervals.length.toLocaleString()}
            </span>
            <span>
              History:{' '}
              {state.truncated === null
                ? 'truncation unknown'
                : state.truncated
                  ? 'truncated'
                  : 'not reported truncated'}
            </span>
            <span>Snapshot: {timestamp(snapshotCapturedAt)}</span>
          </div>
          <p className="portfolio-widget-message">
            Independent comparison requires authoritative execution events,
            settlements, fees and sourced FX; PM projections alone are not an
            independent reconciliation.
          </p>

          <div className="portfolio-interval-toolbar">
            <label>
              Interval status
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
                <option value="all">All</option>
                <option value="complete">Complete</option>
                <option value="incomplete">Incomplete or unavailable</option>
              </select>
            </label>
            <span>
              {visiblePortfolioIntervals(state).length.toLocaleString()} shown
            </span>
          </div>

          {reportedReturnChart(visiblePortfolioIntervals(state))}

          {visiblePortfolioIntervals(state).length > 0 ? (
            <CompactTable label="Portfolio valuation interval reconciliation">
              <thead>
                <tr>
                  <th>End time</th>
                  <th>Evidence ID</th>
                  <th>PM completeness</th>
                  <th>Reported return</th>
                  <th>Net P&amp;L</th>
                  <th>Independent status</th>
                  <th>Drill-down</th>
                </tr>
              </thead>
              <tbody>{visiblePortfolioIntervals(state).map(intervalRow)}</tbody>
            </CompactTable>
          ) : (
            <p className="portfolio-widget-message">
              No portfolio intervals match this filter.
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
