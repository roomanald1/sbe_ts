import { useEffect, useMemo } from 'react';
import {
  CompactTable,
  OperatorMetricStrip,
  OperatorTabBar,
  SortableHeader,
  useBehaviorSnapshot,
} from '@workspace/ui-platform';
import {
  AutoTraderExecutionQualityModel,
  decisionPriceDistanceLabel,
  executionFreshnessLabel,
  type MarketPathSample,
  type ExecutionOutcome,
  type ExecutionQualityState,
  type MatchedMarketPath,
  type UnlinkedMarketPath,
  type ExecutionOrderSortField,
} from '../models/AutoTraderExecutionQualityModel';

function price(value: number | null | undefined): string {
  return typeof value === 'number' && Number.isFinite(value) && value > 0
    ? new Intl.NumberFormat(undefined, { maximumSignificantDigits: 8 }).format(
        value,
      )
    : 'Unavailable';
}

function duration(milliseconds: number | null): string {
  if (
    milliseconds == null ||
    !Number.isFinite(milliseconds) ||
    milliseconds < 0
  )
    return 'Unavailable';
  if (milliseconds < 1_000) return `${milliseconds} ms`;
  if (milliseconds < 60_000) return `${(milliseconds / 1_000).toFixed(1)} s`;
  return `${(milliseconds / 60_000).toFixed(1)} min`;
}

const outcomeFilters: Array<{ value: ExecutionOutcome | ''; label: string }> = [
  { value: '', label: 'All outcomes' },
  { value: 'filled', label: 'Filled' },
  { value: 'partially_filled', label: 'Partially filled' },
  { value: 'pending', label: 'Pending' },
  { value: 'post_only_cross_rejection', label: 'Post-only crossing rejection' },
  { value: 'passive_expiry', label: 'Passive expiry' },
  { value: 'cancelled', label: 'Cancelled' },
  { value: 'rejected', label: 'Rejected' },
  { value: 'terminal_no_fill', label: 'Terminal without fill' },
  { value: 'unknown', label: 'Unknown' },
];

function sourceValue(value: unknown): string {
  if (typeof value === 'string' && value.trim()) return value;
  if (typeof value === 'number' && Number.isFinite(value)) return String(value);
  if (typeof value === 'boolean') return String(value);
  return 'Unavailable';
}

function exactPrice(value: unknown): string {
  return typeof value === 'number' && Number.isFinite(value) && value > 0
    ? String(value)
    : 'Unavailable';
}

function pathStatusClass(value: string): string {
  if (value === 'observed_only') return 'is-warning';
  if (value === 'source_incomplete' || value === 'truncated') return 'is-bad';
  if (value === 'unavailable') return 'is-unavailable';
  return 'is-unavailable';
}

function renderMarketPathSample(sample: MarketPathSample, index: number) {
  return (
    <tr
      key={`${sample.source_epoch ?? ''}:${sample.source_cursor ?? ''}:${sample.book_updated_at_ms ?? ''}:${index}`}
    >
      <td>{sourceValue(sample.source_system)}</td>
      <td>{sourceValue(sample.source_epoch)}</td>
      <td>
        {sourceValue(sample.source_cursor)}
        {sample.source_cursor_kind ? ` · ${sample.source_cursor_kind}` : ''}
      </td>
      <td>{sourceValue(sample.book_updated_at_ms)}</td>
      <td>
        {exactPrice(sample.best_bid)} / {exactPrice(sample.best_ask)}
      </td>
      <td>{exactPrice(sample.reference_price)}</td>
      <td>
        <details>
          <summary>Provenance</summary>
          <dl>
            <div>
              <dt>Recorded at</dt>
              <dd>{sourceValue(sample.recorded_at_utc_ms)}</dd>
            </div>
            <div>
              <dt>Source capture complete</dt>
              <dd>{sourceValue(sample.source_capture_complete)}</dd>
            </div>
            <div>
              <dt>Request fingerprint</dt>
              <dd>{sourceValue(sample.request_fingerprint)}</dd>
            </div>
            <div>
              <dt>Attempt order ID</dt>
              <dd>{sourceValue(sample.attempt_order_id)}</dd>
            </div>
            <div>
              <dt>Parent order ID</dt>
              <dd>{sourceValue(sample.parent_order_id)}</dd>
            </div>
          </dl>
        </details>
      </td>
    </tr>
  );
}

function marketPathPanel(
  path: MatchedMarketPath,
  activityOrder?: ExecutionQualityState['rows'][number]['order'],
) {
  const completeness = path.order.market_path_completeness ?? 'unavailable';
  const samples = Array.isArray(path.order.market_path)
    ? path.order.market_path
    : [];
  return (
    <details key={path.key} className="v35-operator-card">
      <summary>
        {path.source === 'active_order'
          ? 'Active resting order'
          : 'Terminal resting order'}{' '}
        · {sourceValue(path.order.id)} ·{' '}
        <span
          className={`v35-operator-status ${pathStatusClass(completeness)}`}
        >
          {completeness}
        </span>{' '}
        · {samples.length} observed matcher responses
      </summary>
      <dl className="v35-operator-metrics">
        <div>
          <dt>Path completeness</dt>
          <dd>{completeness}</dd>
        </div>
        <div>
          <dt>Dropped samples</dt>
          <dd>{sourceValue(path.order.market_path_dropped_samples)}</dd>
        </div>
        <div>
          <dt>Terminal reason</dt>
          <dd>{sourceValue(path.terminalReason)}</dd>
        </div>
        <div>
          <dt>Terminal time</dt>
          <dd>{sourceValue(path.terminalAtMs)}</dd>
        </div>
        <div>
          <dt>Remaining quantity in PM record</dt>
          <dd>{sourceValue(path.order.remaining_quantity)}</dd>
        </div>
        <div>
          <dt>Linked AutoTrader filled quantity</dt>
          <dd>
            {activityOrder
              ? `${sourceValue(activityOrder.filled_quantity)} / ${sourceValue(activityOrder.quantity)}; reported separately from matcher observations`
              : 'Unavailable for unlinked path'}
          </dd>
        </div>
      </dl>
      <p>
        These are bounded matcher response observations. They do not establish
        continuous quote coverage, queue priority, or fillability. A price touch
        is not proof an order would have filled. Settlement is evidenced
        separately by linked fills.
      </p>
      {samples.length ? (
        <div style={{ overflowX: 'auto' }}>
          <table className="v35-operator-table">
            <thead>
              <tr>
                <th>Source</th>
                <th>Epoch</th>
                <th>Cursor</th>
                <th>Book timestamp (ms)</th>
                <th>Bid / ask</th>
                <th>Reference price</th>
                <th>Details</th>
              </tr>
            </thead>
            <tbody>{samples.map(renderMarketPathSample)}</tbody>
          </table>
        </div>
      ) : (
        <p>No match-response samples are recorded for this order.</p>
      )}
    </details>
  );
}

function unlinkedPathPanel(item: UnlinkedMarketPath) {
  const reason = item.reason.replaceAll('_', ' ');
  return (
    <div key={`${item.path.key}:${item.reason}`}>
      <p>
        Unlinked PM path ({reason}); no AutoTrader order attribution was made.
      </p>
      {marketPathPanel(item.path)}
    </div>
  );
}

export function ExecutionQualityPanel({
  state,
  nowMs,
  onSetFilters,
  onRefresh,
  onSetView,
  onSelectOrder,
  onSetSort,
}: {
  state: ExecutionQualityState;
  nowMs?: number;
  onSetFilters?: (filters: Partial<ExecutionQualityState['filters']>) => void;
  onRefresh?: () => void;
  onSetView?: (view: ExecutionQualityState['view']) => void;
  onSelectOrder?: (key: string | null) => void;
  onSetSort?: (field: ExecutionOrderSortField) => void;
}) {
  const currentTime = nowMs ?? state.nowMs;
  const summary = state.summary;
  const freshness = executionFreshnessLabel(
    summary.last_updated_at,
    currentTime,
  );
  const fillCount = summary.filled_order_samples + summary.partial_fill_samples;
  const fillRate =
    summary.order_samples > 0
      ? (fillCount / summary.order_samples) * 100
      : null;
  const observedOutcomes = [
    {
      id: 'filled',
      label: 'Filled',
      count: state.outcomeCounts.filled + state.outcomeCounts.partially_filled,
      tone: 'filled',
    },
    {
      id: 'cancelled',
      label: 'Cancelled',
      count: state.outcomeCounts.cancelled,
      tone: 'cancelled',
    },
    {
      id: 'expired',
      label: 'Expired',
      count: state.outcomeCounts.passive_expiry,
      tone: 'expired',
    },
    {
      id: 'rejected',
      label: 'Rejected',
      count:
        state.outcomeCounts.rejected +
        state.outcomeCounts.post_only_cross_rejection,
      tone: 'rejected',
    },
    {
      id: 'pending',
      label: 'Pending',
      count: state.outcomeCounts.pending,
      tone: 'pending',
    },
    {
      id: 'other',
      label: 'Other',
      count: state.outcomeCounts.terminal_no_fill + state.outcomeCounts.unknown,
      tone: 'other',
    },
  ];
  const outcomeTotal = observedOutcomes.reduce(
    (total, item) => total + item.count,
    0,
  );
  const selected = state.selectedOrder;
  const terminalPaths = state.pathEvidence.terminalPaths.map(
    (record, index) => ({
      key: 'terminal:' + (record.order.id ?? index),
      source: 'terminal_history' as const,
      order: record.order,
      terminalReason: record.terminal_reason ?? null,
      terminalAtMs: record.terminal_at_ms ?? null,
    }),
  );
  const activePaths = state.pathEvidence.activeOrders.map((order, index) => ({
    key: 'active:' + (order.id ?? index),
    source: 'active_order' as const,
    order,
    terminalReason: null,
    terminalAtMs: null,
  }));
  const statusLabel = (row: ExecutionQualityState['rows'][number]) =>
    row.outcomeLabel;
  const rowAge = (row: ExecutionQualityState['rows'][number]) => {
    if (row.outcome === 'pending')
      return '≥ ' + duration(Math.max(currentTime - row.order.timestamp, 0));
    const terminal = row.marketPaths.find((path) => path.terminalAtMs !== null);
    return terminal
      ? duration(Math.max(terminal.terminalAtMs! - row.order.timestamp, 0))
      : 'Unavailable';
  };
  const compactPercent = (value: number | null) =>
    value === null || !Number.isFinite(value)
      ? 'Unavailable'
      : value.toFixed(1) + '%';
  const sortHeader = (field: ExecutionOrderSortField, label: string) => (
    <SortableHeader
      key={field}
      label={label}
      active={state.sort.field === field}
      direction={state.sort.direction}
      onSort={() => onSetSort?.(field)}
    />
  );

  return (
    <section
      className="auto-trader-execution-quality operator-panel"
      aria-label="Execution quality"
    >
      <div className="execution-quality-toolbar">
        <span className="execution-quality-source">AutoTrader · Paper</span>
        <span
          className="execution-quality-freshness"
          role={freshness.stale ? 'status' : undefined}
        >
          {freshness.label}
        </span>
        <button
          type="button"
          onClick={onRefresh}
          disabled={!onRefresh || state.pathEvidenceLoading}
        >
          {state.pathEvidenceLoading ? 'Refreshing…' : 'Refresh'}
        </button>
      </div>

      <OperatorMetricStrip
        label="Execution quality summary"
        metrics={[
          {
            label: 'Submitted',
            value: summary.order_samples.toLocaleString(),
            detail: summary.history_truncated
              ? 'History truncated'
              : 'Aggregate sample',
          },
          {
            label: 'Filled',
            value:
              summary.filled_order_samples.toLocaleString() +
              (summary.partial_fill_samples > 0
                ? ' + ' +
                  summary.partial_fill_samples.toLocaleString() +
                  ' partial'
                : ''),
            detail: summary.order_samples
              ? 'of ' + summary.order_samples.toLocaleString() + ' orders'
              : 'No denominator',
          },
          {
            label: 'Fill rate',
            value: compactPercent(fillRate),
            tone:
              fillRate === null
                ? 'neutral'
                : fillRate > 0
                  ? 'positive'
                  : 'warning',
            detail:
              fillRate === null ? 'Unavailable' : 'includes partial fills',
          },
          {
            label: 'Avg fee',
            value:
              typeof summary.average_fee_bps === 'number' &&
              Number.isFinite(summary.average_fee_bps)
                ? summary.average_fee_bps.toFixed(2) + ' bps'
                : 'Unavailable',
            detail:
              summary.measured_fill_samples.toLocaleString() +
              ' measured fills',
          },
          {
            label: 'All-in cost',
            value:
              typeof summary.average_all_in_cost_bps === 'number' &&
              Number.isFinite(summary.average_all_in_cost_bps)
                ? summary.average_all_in_cost_bps.toFixed(2) + ' bps'
                : 'Unavailable',
            detail:
              typeof summary.average_all_in_cost_bps === 'number'
                ? summary.measured_fill_samples.toLocaleString() +
                  ' measured fills'
                : 'No complete cost evidence',
          },
        ]}
      />

      <div
        className="execution-quality-outcome-bar"
        role="img"
        aria-label={
          'Returned order outcomes: ' +
          observedOutcomes
            .filter((item) => item.count > 0)
            .map((item) => item.label + ' ' + item.count)
            .join(', ')
        }
      >
        {outcomeTotal > 0 ? (
          observedOutcomes.map((item) =>
            item.count > 0 ? (
              <span
                key={item.id}
                className={'execution-quality-outcome-bar__' + item.tone}
                style={{ width: (item.count / outcomeTotal) * 100 + '%' }}
              />
            ) : null,
          )
        ) : (
          <span className="execution-quality-outcome-bar__empty" />
        )}
      </div>
      <div className="execution-quality-outcome-legend">
        {observedOutcomes.map((item) => (
          <span key={item.id} data-tone={item.tone}>
            {item.count.toLocaleString()} {item.label.toLowerCase()}
          </span>
        ))}
        <span className="execution-quality-outcome-coverage">
          Returned rows only · {state.displayedOrderCount} shown
          {state.availableOrderCount > state.displayedOrderCount
            ? ' of ' + state.availableOrderCount.toLocaleString()
            : ''}
        </span>
      </div>

      <OperatorTabBar
        label="Execution quality views"
        selectedId={state.view}
        onSelect={(id) => onSetView?.(id as ExecutionQualityState['view'])}
        tabs={[
          { id: 'overview', label: 'Overview' },
          { id: 'orders', label: 'Orders' },
          { id: 'history', label: 'History' },
        ]}
      />

      {state.pathEvidenceError && (
        <p className="execution-quality-message" role="alert">
          Matcher path refresh failed: {state.pathEvidenceError}
          {state.pathEvidenceStale
            ? ' Last successful evidence is shown as stale.'
            : ' No successful matcher-path evidence is available.'}
        </p>
      )}
      {state.pathEvidenceStale && !state.pathEvidenceError && (
        <p className="execution-quality-message" role="status">
          Showing the last successful matcher-path snapshot while refresh is
          pending.
        </p>
      )}

      {state.view === 'overview' && (
        <div className="execution-quality-overview">
          <section aria-label="Strategy execution breakdown">
            <header className="execution-quality-section-heading">
              <strong>Returned orders by strategy</strong>
              <span>
                {state.displayedOrderCount} order rows · fees are shown as
                evidence coverage, not summed across currencies
              </span>
            </header>
            {state.strategyRows.length === 0 ? (
              <p className="execution-quality-message">
                No strategy breakdown is available from returned order records.
              </p>
            ) : (
              <CompactTable label="Returned order strategy breakdown">
                <thead>
                  <tr>
                    <th className="is-left">Strategy</th>
                    <th>Orders</th>
                    <th>With fill</th>
                    <th>Fill %</th>
                    <th>Avg age</th>
                    <th>Fee records</th>
                  </tr>
                </thead>
                <tbody>
                  {state.strategyRows.map((row) => (
                    <tr key={row.strategy}>
                      <td className="is-left">{row.strategy}</td>
                      <td>{row.orderCount.toLocaleString()}</td>
                      <td>{row.filledCount.toLocaleString()}</td>
                      <td>{compactPercent(row.fillRatePercent)}</td>
                      <td>
                        {row.averageTerminalLifetimeMs === null
                          ? 'Unavailable'
                          : duration(row.averageTerminalLifetimeMs) +
                            ' · ' +
                            row.terminalAgeSamples.toLocaleString() +
                            ' terminal records'}
                      </td>
                      <td>
                        {row.feeEvidenceCount.toLocaleString()} /{' '}
                        {row.orderCount.toLocaleString()}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </CompactTable>
            )}
            {state.availableOrderCount > state.displayedOrderCount && (
              <p className="execution-quality-message" role="status">
                Strategy counts cover the returned, capped records only; older
                orders are omitted from this breakdown.
              </p>
            )}
          </section>

          {selected ? (
            <SelectedExecutionOrder
              row={selected}
              nowMs={currentTime}
              onClose={() => onSelectOrder?.(null)}
            />
          ) : (
            <p className="execution-quality-message">
              Select an order from the Orders view to inspect its price and
              lifecycle evidence.
            </p>
          )}
        </div>
      )}

      {state.view === 'orders' && (
        <section
          aria-label="Execution order records"
          className="execution-quality-orders-view"
        >
          <div className="execution-quality-filters">
            <label>
              Symbol
              <select
                aria-label="Filter execution orders by symbol"
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
              Outcome
              <select
                aria-label="Filter execution orders by outcome"
                value={state.filters.outcome}
                onChange={(event) =>
                  onSetFilters?.({
                    outcome: event.target.value as ExecutionOutcome | '',
                  })
                }
              >
                {outcomeFilters.map((filter) => (
                  <option key={filter.value || 'all'} value={filter.value}>
                    {filter.label}
                  </option>
                ))}
              </select>
            </label>
          </div>

          {state.loading && state.availableOrderCount === 0 ? (
            <p className="execution-quality-message" role="status">
              Loading execution orders…
            </p>
          ) : state.availableOrderCount === 0 ? (
            <p className="execution-quality-message" role="status">
              No AutoTrader order records were returned. Zero records do not
              establish complete history.
            </p>
          ) : state.visibleRows.length === 0 ? (
            <p className="execution-quality-message" role="status">
              No order records match these filters.
            </p>
          ) : (
            <CompactTable label="Execution order records">
              <thead>
                <tr>
                  {sortHeader('symbol', 'Symbol')}
                  {sortHeader('strategy', 'Strategy')}
                  {sortHeader('outcome', 'Outcome')}
                  {sortHeader('timestamp', 'Submitted')}
                  <th>Limit</th>
                  <th>Filled</th>
                  <th>Age</th>
                </tr>
              </thead>
              <tbody>
                {state.visibleRows.map((row) => (
                  <tr
                    key={row.key}
                    className={
                      selected?.key === row.key ? 'is-selected' : undefined
                    }
                  >
                    <td className="is-left">
                      <button
                        className="execution-quality-order-select"
                        type="button"
                        aria-pressed={selected?.key === row.key}
                        onClick={() => onSelectOrder?.(row.key)}
                      >
                        {row.order.symbol || 'Unavailable'}
                      </button>
                    </td>
                    <td>{row.order.strategy?.trim() || 'Unavailable'}</td>
                    <td data-outcome={row.outcome}>{statusLabel(row)}</td>
                    <td>
                      {new Intl.DateTimeFormat(undefined, {
                        month: 'numeric',
                        day: 'numeric',
                        hour: '2-digit',
                        minute: '2-digit',
                        second: '2-digit',
                      }).format(row.order.timestamp)}
                    </td>
                    <td>{price(row.order.submitted_limit_price)}</td>
                    <td>
                      {sourceValue(row.order.filled_quantity)} /{' '}
                      {sourceValue(row.order.quantity)}
                    </td>
                    <td>{rowAge(row)}</td>
                  </tr>
                ))}
              </tbody>
            </CompactTable>
          )}

          {selected && (
            <SelectedExecutionOrder
              row={selected}
              nowMs={currentTime}
              onClose={() => onSelectOrder?.(null)}
            />
          )}
        </section>
      )}

      {state.view === 'history' && (
        <section
          className="execution-quality-history"
          aria-label="Durable matcher path history"
        >
          <header className="execution-quality-section-heading">
            <strong>Durable matcher path evidence</strong>
            <span
              className={
                'execution-quality-evidence-state is-' +
                state.pathEvidenceStatus
              }
            >
              {state.pathEvidenceStatus}
            </span>
          </header>
          <div className="execution-quality-history-summary">
            <span>
              Terminal paths{' '}
              {state.pathEvidence.historyContractAvailable
                ? state.pathEvidence.terminalPaths.length.toLocaleString()
                : 'Unavailable'}
            </span>
            <span>
              Unlinked records{' '}
              {state.pathEvidence.historyContractAvailable
                ? state.unlinkedMarketPaths.length.toLocaleString()
                : 'Unavailable'}
            </span>
            <span>
              History truncated{' '}
              {state.pathEvidence.historyTruncated === null
                ? 'Unavailable'
                : String(state.pathEvidence.historyTruncated)}
            </span>
          </div>
          {state.pathEvidenceStatus !== 'available' && (
            <p className="execution-quality-message" role="status">
              {!state.pathEvidence.historyContractAvailable
                ? 'The current Position Management history contract does not expose durable matcher-path records.'
                : state.pathEvidenceStatus === 'unavailable'
                  ? 'The current Position Management contract does not expose durable matcher-path history.'
                  : 'Matcher-path coverage is partial; missing source, retention, or active-order fields remain unavailable.'}
            </p>
          )}
          {state.pathEvidenceStatus === 'unavailable' ? (
            <p className="execution-quality-message">
              Matcher probes and resting-limit observations are not settled
              fills. A price touch does not prove order fillability.
            </p>
          ) : (
            <div className="execution-quality-history-list">
              {[...activePaths, ...terminalPaths].map((path) =>
                marketPathPanel(path),
              )}
              {state.unlinkedMarketPaths.map(unlinkedPathPanel)}
            </div>
          )}
          <details className="execution-quality-provenance-note">
            <summary>Coverage and attribution limits</summary>
            <p>
              Path records are joined only by stable order or parent identity.
              They do not establish continuous quote coverage, queue priority,
              or fillability; settled fills remain sourced from AutoTrader
              activity.
            </p>
            <p>
              More order-history pages:{' '}
              {state.pathEvidence.historyHasMoreOrders === null
                ? 'Unavailable'
                : String(state.pathEvidence.historyHasMoreOrders)}
            </p>
          </details>
        </section>
      )}
    </section>
  );
}

function SelectedExecutionOrder({
  row,
  nowMs,
  onClose,
}: {
  row: ExecutionQualityState['rows'][number];
  nowMs: number;
  onClose: () => void;
}) {
  const order = row.order;
  const pending = row.outcome === 'pending';
  const elapsed = pending
    ? 'At least ' + duration(Math.max(nowMs - order.timestamp, 0))
    : 'Unavailable';
  return (
    <section
      className="execution-quality-selected"
      aria-label="Selected order detail"
    >
      <header>
        <div>
          <strong>
            {order.symbol} · {order.side.toUpperCase()} · {row.outcomeLabel}
          </strong>
          <span>
            {order.strategy?.trim() || 'Strategy unavailable'} ·{' '}
            {order.order_type || 'Order type unavailable'}
          </span>
        </div>
        <button
          type="button"
          onClick={onClose}
          aria-label="Clear selected order"
        >
          ×
        </button>
      </header>
      <div className="execution-quality-selected-grid">
        <dl>
          <div>
            <dt>Submitted limit</dt>
            <dd>{price(order.submitted_limit_price)}</dd>
          </div>
          <div>
            <dt>Decision bid / ask</dt>
            <dd>
              {price(order.decision_bid)} / {price(order.decision_ask)}
            </dd>
          </div>
          <div>
            <dt>Limit / same-side quote</dt>
            <dd>{decisionPriceDistanceLabel(row.decisionTouchDistanceBps)}</dd>
          </div>
          <div>
            <dt>Limit / opposite quote</dt>
            <dd>
              {decisionPriceDistanceLabel(row.decisionCrossingQuoteDistanceBps)}
            </dd>
          </div>
        </dl>
        <dl>
          <div>
            <dt>Filled / requested</dt>
            <dd>
              {sourceValue(order.filled_quantity)} /{' '}
              {sourceValue(order.quantity)}
            </dd>
          </div>
          <div>
            <dt>Average fill</dt>
            <dd>{price(order.average_price)}</dd>
          </div>
          <div>
            <dt>Reservation amount</dt>
            <dd>
              {typeof order.reserved_notional === 'number'
                ? order.reserved_notional.toLocaleString(undefined, {
                    maximumFractionDigits: 2,
                  })
                : 'Unavailable'}
            </dd>
          </div>
          <div>
            <dt>Configured expiry</dt>
            <dd>{duration(row.configuredLifetimeMs)}</dd>
          </div>
        </dl>
      </div>
      <details>
        <summary>Identifiers, terminal reason and source evidence</summary>
        <dl className="execution-quality-identifiers">
          <div>
            <dt>Decision ID</dt>
            <dd>{row.decisionId ?? 'Unavailable'}</dd>
          </div>
          <div>
            <dt>Execution order ID</dt>
            <dd>{row.stableOrderId ?? 'Unavailable'}</dd>
          </div>
          <div>
            <dt>Order age</dt>
            <dd>{elapsed}</dd>
          </div>
          <div>
            <dt>Terminal lifetime</dt>
            <dd>
              {row.marketPaths.find((path) => path.terminalAtMs !== null)
                ? duration(
                    Math.max(
                      row.marketPaths.find(
                        (path) => path.terminalAtMs !== null,
                      )!.terminalAtMs! - order.timestamp,
                      0,
                    ),
                  )
                : 'Unavailable · terminal timestamp not recorded'}
            </dd>
          </div>
          <div>
            <dt>Failure / terminal reason</dt>
            <dd>{order.failure_reason?.trim() || 'Not recorded'}</dd>
          </div>
          <div>
            <dt>Fee amount</dt>
            <dd>
              {typeof order.fees_paid === 'number'
                ? order.fees_paid.toPrecision(8)
                : 'Unavailable'}
            </dd>
          </div>
          <div>
            <dt>Cancellation actor</dt>
            <dd>
              {row.outcome === 'cancelled' ? 'Unavailable' : 'Not applicable'}
            </dd>
          </div>
        </dl>
      </details>
      <details>
        <summary>
          Linked Position Management market path · {row.marketPaths.length}{' '}
          record(s)
        </summary>
        {row.marketPaths.length > 0 ? (
          row.marketPaths.map((path) => marketPathPanel(path, order))
        ) : (
          <p>
            {row.marketPaths.length === 0
              ? 'No durable path joined by stable order, client, or parent identifier.'
              : 'Unavailable'}
          </p>
        )}
      </details>
      <p className="execution-quality-disclaimer">
        Matcher observations are not settled fills; a price touch does not prove
        fillability.
      </p>
    </section>
  );
}

export function AutoTraderExecutionCheck() {
  const model = useMemo(() => new AutoTraderExecutionQualityModel(), []);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  const state = useBehaviorSnapshot(model.state$);
  return (
    <ExecutionQualityPanel
      state={state}
      onSetFilters={(filters) => model.setFilters(filters)}
      onSetView={(view) => model.setView(view)}
      onSelectOrder={(key) => model.selectOrder(key)}
      onSetSort={(field) => model.setSort(field)}
      onRefresh={() => void model.refresh()}
    />
  );
}
