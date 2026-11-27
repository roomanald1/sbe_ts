import { useEffect, useMemo } from 'react';
import { useBehaviorSnapshot } from '@workspace/ui-platform';
import { FeedToolbar } from './FeedToolbar';
import {
  AutoTraderExecutionQualityModel,
  decisionPriceDistanceLabel,
  executionFreshnessLabel,
  type MarketPathSample,
  type ExecutionOutcome,
  type ExecutionQualityState,
  type MatchedMarketPath,
  type UnlinkedMarketPath,
} from '../models/AutoTraderExecutionQualityModel';

function price(value: number | null | undefined): string {
  return typeof value === 'number' && Number.isFinite(value) && value > 0
    ? value.toPrecision(10)
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
}: {
  state: ExecutionQualityState;
  nowMs?: number;
  onSetFilters?: (filters: Partial<ExecutionQualityState['filters']>) => void;
  onRefresh?: () => void;
}) {
  const currentTime = nowMs ?? state.nowMs;
  const filteredRows = state.visibleRows;
  const summary = state.summary;
  const freshness = executionFreshnessLabel(
    summary.last_updated_at,
    currentTime,
  );
  const sampleDenominator = summary.order_samples;
  const referenceCoverage = summary.reference_price_coverage_percent;

  return (
    <section
      className="auto-trader-execution-check auto-trader-execution-quality v35-operator-card"
      aria-label="Execution quality"
    >
      <FeedToolbar title="Execution quality" />
      <p>
        Order outcomes and recorded price evidence only. Post-submit executable
        market path, queue position, cancellation actor, and counterfactual
        fillability are unavailable; no expected fills or counterfactual profit
        are inferred.
      </p>

      <section aria-label="Execution quality aggregate">
        <div>
          <strong>Aggregate execution quality</strong>
          <span role={freshness.stale ? 'status' : undefined}>
            {freshness.label}
          </span>
        </div>
        <dl
          className="v35-operator-metrics"
          aria-label="Execution quality metrics"
        >
          <div className="v35-operator-metric">
            <dt>Orders in quality sample</dt>
            <dd>{summary.order_samples}</dd>
          </div>
          <div className="v35-operator-metric">
            <dt>Filled orders</dt>
            <dd>
              {summary.filled_order_samples} / {sampleDenominator}
            </dd>
          </div>
          <div className="v35-operator-metric">
            <dt>Partial fills</dt>
            <dd>
              {summary.partial_fill_samples} / {sampleDenominator}
            </dd>
          </div>
          <div className="v35-operator-metric">
            <dt>Unfilled orders</dt>
            <dd>
              {summary.unfilled_order_samples} / {sampleDenominator}
            </dd>
          </div>
          <div className="v35-operator-metric">
            <dt>Measured fill samples</dt>
            <dd>{summary.measured_fill_samples}</dd>
          </div>
          <div className="v35-operator-metric">
            <dt>Reference price coverage</dt>
            <dd>
              {typeof referenceCoverage === 'number'
                ? `${referenceCoverage.toFixed(1)}%`
                : 'Unavailable'}
            </dd>
          </div>
          <div className="v35-operator-metric">
            <dt>Average implementation shortfall</dt>
            <dd>
              {typeof summary.average_implementation_shortfall_bps === 'number'
                ? `${summary.average_implementation_shortfall_bps.toFixed(2)} bps / ${summary.measured_fill_samples} measured fills`
                : 'Unavailable'}
            </dd>
          </div>
          <div className="v35-operator-metric">
            <dt>Average fee</dt>
            <dd>
              {typeof summary.average_fee_bps === 'number'
                ? `${summary.average_fee_bps.toFixed(2)} bps / ${summary.measured_fill_samples} measured fills`
                : 'Unavailable'}
            </dd>
          </div>
          <div className="v35-operator-metric">
            <dt>Average all-in cost</dt>
            <dd>
              {typeof summary.average_all_in_cost_bps === 'number'
                ? `${summary.average_all_in_cost_bps.toFixed(2)} bps / ${summary.measured_fill_samples} measured fills`
                : 'Unavailable'}
            </dd>
          </div>
          <div className="v35-operator-metric">
            <dt>Submit response latency</dt>
            <dd>
              {typeof summary.average_submit_response_ms === 'number'
                ? `${summary.average_submit_response_ms.toFixed(1)} ms / ${summary.response_latency_samples} samples`
                : `Unavailable / ${summary.response_latency_samples} samples`}
            </dd>
          </div>
        </dl>
        {summary.history_truncated && (
          <p role="status">Execution quality history is truncated.</p>
        )}
        {!summary.last_updated_at && !summary.order_samples && (
          <p role="status">
            Zero samples are reported, but the aggregate update timestamp is
            unavailable; summary freshness cannot be verified.
          </p>
        )}
      </section>

      <section aria-label="Durable matcher path evidence">
        <strong>Durable matcher path evidence</strong>
        <span
          className={`v35-operator-status ${state.pathEvidenceStatus === 'available' ? 'is-good' : state.pathEvidenceStatus === 'partial' || state.pathEvidenceStatus === 'loading' ? 'is-warning' : 'is-unavailable'}`}
        >
          {state.pathEvidenceStatus}
        </span>
        <dl className="v35-operator-metrics">
          <div>
            <dt>Terminal paths retained</dt>
            <dd>
              {state.pathEvidence.historyContractAvailable
                ? state.pathEvidence.terminalPaths.length
                : 'Unavailable'}
            </dd>
          </div>
          <div>
            <dt>Global terminal-path history truncated</dt>
            <dd>
              {state.pathEvidence.historyTruncated === null
                ? 'Unavailable'
                : String(state.pathEvidence.historyTruncated)}
            </dd>
          </div>
          <div>
            <dt>More order-history pages</dt>
            <dd>
              {state.pathEvidence.historyHasMoreOrders === null
                ? 'Unavailable'
                : String(state.pathEvidence.historyHasMoreOrders)}
            </dd>
          </div>
          <div>
            <dt>Unlinked path records</dt>
            <dd>
              {state.pathEvidence.historyContractAvailable
                ? state.unlinkedMarketPaths.length
                : 'Unavailable'}
            </dd>
          </div>
        </dl>
        {state.pathEvidenceStatus !== 'available' && (
          <p role="status">
            {!state.pathEvidence.historyContractAvailable
              ? 'The current Position Management history contract does not expose durable matcher-path records.'
              : state.pathEvidenceStatus === 'unavailable'
                ? 'The current Position Management contract does not expose durable matcher-path history.'
                : 'Matcher-path coverage is partial; missing source, retention, or active-order fields remain unavailable.'}
          </p>
        )}
        <p>
          Matcher probes and resting-limit observations are not settled fills.
          Fill quantities shown below come from AutoTrader activity and are not
          inferred from a price touch.
        </p>
      </section>

      <div>
        <strong>Order records</strong>
        <span>
          Showing {state.displayedOrderCount} of {state.availableOrderCount}{' '}
          returned records (view cap 100)
          {state.availableOrderCount > state.displayedOrderCount
            ? '; older returned records omitted'
            : '; backend history completeness unknown'}
          .
        </span>
        <button
          type="button"
          onClick={onRefresh}
          disabled={!onRefresh || state.pathEvidenceLoading}
        >
          {state.pathEvidenceLoading
            ? 'Refreshing…'
            : 'Refresh execution evidence'}
        </button>
      </div>

      <div className="auto-trader-execution-quality-filters">
        <label>
          Symbol
          <select
            aria-label="Filter execution orders by symbol"
            value={state.filters.symbol}
            onChange={(event) => onSetFilters?.({ symbol: event.target.value })}
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

      {state.pathEvidenceError && (
        <p role="alert">
          Matcher-path refresh failed: {state.pathEvidenceError}
          {state.pathEvidenceStale
            ? ' Showing the last successful path evidence; it is stale.'
            : ' No successful path evidence is available yet.'}
        </p>
      )}
      {state.pathEvidenceStale && !state.pathEvidenceError && (
        <p role="status">
          Showing the last successful matcher-path snapshot while a refresh is
          pending.
        </p>
      )}
      {state.loading && state.availableOrderCount === 0 ? (
        <p role="status">Loading execution order evidence…</p>
      ) : state.availableOrderCount === 0 ? (
        <p role="status">
          No AutoTrader order records were returned. The quality summary reports{' '}
          {summary.filled_order_samples} fills from {summary.order_samples}{' '}
          orders; zero orders in this response do not establish complete
          history.
        </p>
      ) : filteredRows.length === 0 ? (
        <p role="status">No order records match these filters.</p>
      ) : (
        <ol className="auto-trader-execution-quality-orders">
          {filteredRows.map((row) => {
            const order = row.order;
            const pending = row.outcome === 'pending';
            const elapsedMs = pending
              ? Math.max(currentTime - order.timestamp, 0)
              : null;
            return (
              <li key={row.key}>
                <header>
                  <strong>
                    {order.symbol} · {order.side.toUpperCase()} ·{' '}
                    {row.outcomeLabel}
                  </strong>
                  <span>{new Date(order.timestamp).toLocaleString()}</span>
                </header>
                <dl>
                  <div>
                    <dt>Decision ID</dt>
                    <dd>{row.decisionId ?? 'Not recorded'}</dd>
                  </div>
                  <div>
                    <dt>Execution order ID</dt>
                    <dd>{row.stableOrderId ?? 'Not recorded'}</dd>
                  </div>
                  <div>
                    <dt>Strategy / purpose</dt>
                    <dd>
                      {[order.strategy, order.purpose]
                        .filter(Boolean)
                        .join(' · ') || 'Unavailable'}
                    </dd>
                  </div>
                  <div>
                    <dt>Order type / status</dt>
                    <dd>
                      {order.order_type} · {order.status}
                    </dd>
                  </div>
                  <div>
                    <dt>Reservation evidence</dt>
                    <dd>
                      {typeof order.reserved_notional === 'number'
                        ? `Recorded notional ${order.reserved_notional.toFixed(2)}`
                        : 'Amount unavailable'}
                      {' · reservation ID and timestamp unavailable'}
                    </dd>
                  </div>
                  <div>
                    <dt>Decision bid / ask</dt>
                    <dd>
                      {price(order.decision_bid)} / {price(order.decision_ask)}
                    </dd>
                  </div>
                  <div>
                    <dt>Submitted limit</dt>
                    <dd>{price(order.submitted_limit_price)}</dd>
                  </div>
                  <div>
                    <dt>Limit vs same-side decision quote</dt>
                    <dd>
                      {decisionPriceDistanceLabel(row.decisionTouchDistanceBps)}
                    </dd>
                  </div>
                  <div>
                    <dt>Limit vs opposite decision quote</dt>
                    <dd>
                      {decisionPriceDistanceLabel(
                        row.decisionCrossingQuoteDistanceBps,
                      )}
                    </dd>
                  </div>
                  <div>
                    <dt>Filled / requested quantity</dt>
                    <dd>
                      {order.filled_quantity} / {order.quantity}
                    </dd>
                  </div>
                  <div>
                    <dt>Average fill price / fees</dt>
                    <dd>
                      {price(order.average_price)} /{' '}
                      {typeof order.fees_paid === 'number'
                        ? order.fees_paid.toPrecision(8)
                        : 'Unavailable'}
                    </dd>
                  </div>
                  <div>
                    <dt>Configured lifetime to expiry</dt>
                    <dd>{duration(row.configuredLifetimeMs)}</dd>
                  </div>
                  <div>
                    <dt>Order age</dt>
                    <dd>
                      {pending
                        ? `At least ${duration(elapsedMs)} since recorded order timestamp`
                        : 'Actual terminal lifetime unavailable; terminal timestamp not recorded'}
                    </dd>
                  </div>
                  <div>
                    <dt>Terminal / failure reason</dt>
                    <dd>
                      {order.failure_reason?.trim() ||
                        'No terminal reason recorded'}
                    </dd>
                  </div>
                  {row.outcome === 'cancelled' && (
                    <div>
                      <dt>Cancellation actor</dt>
                      <dd>Unavailable</dd>
                    </div>
                  )}
                </dl>
                {row.marketPaths.length > 0 ? (
                  <section aria-label="Linked Position Management market path">
                    <strong>Linked Position Management path evidence</strong>
                    {row.marketPaths.map((path) =>
                      marketPathPanel(path, order),
                    )}
                  </section>
                ) : (
                  <p>
                    Linked Position Management market path:{' '}
                    {state.pathEvidenceStatus === 'unavailable'
                      ? 'Unavailable'
                      : 'No path joined by stable order, client, or parent identifier'}
                    .
                  </p>
                )}
              </li>
            );
          })}
        </ol>
      )}
      {state.unlinkedMarketPaths.length > 0 && (
        <section aria-label="Unlinked PM market path records">
          <h3>Unlinked PM path records</h3>
          <p>
            These records are retained as observed evidence but are not
            attributed to AutoTrader activity because stable identifiers did not
            yield a unique join.
          </p>
          {state.unlinkedMarketPaths.map(unlinkedPathPanel)}
        </section>
      )}
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
      onRefresh={() => void model.refresh()}
    />
  );
}
