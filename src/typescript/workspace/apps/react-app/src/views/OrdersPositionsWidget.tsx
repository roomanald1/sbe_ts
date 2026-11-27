import { useEffect, useMemo, useState } from 'react';
import {
  CompactTable,
  ExpandableDetails,
  InlineNetPnlBar,
  OperatorMetricStrip,
  OperatorPanel,
  OperatorTabBar,
  SortableHeader,
  useBehaviorSnapshot,
} from '@workspace/ui-platform';
import {
  barriersForPosition,
  buildActivityOnlyTerminalOrders,
  linkActivityOrder,
  terminalPathForOrder,
  OrdersPositionsWidgetModel,
  sortClosedPositions,
  sortOpenPositions,
  type ActivityOrderIdentity,
  type ClosedSort,
  type LifecycleBarrier,
  type LifecycleOrder,
  type OrdersPositionsState,
  type PositionSort,
} from '../models/OrdersPositionsWidgetModel';
import type { AutoTraderOutcome } from '../models/AutoTraderOutcomesModel';

type Tab = 'positions' | 'active' | 'protection' | 'history' | 'closed';
const viewTabs: Array<{ id: Tab; label: string }> = [
  { id: 'positions', label: 'Open' },
  { id: 'active', label: 'Orders' },
  { id: 'closed', label: 'Closed' },
  { id: 'protection', label: 'Protection' },
];

const unavailable = '—';
function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}
function value(value: unknown): string {
  return typeof value === 'string' && value.trim() ? value : unavailable;
}
function finite(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}
function number(value: unknown, digits = 3): string {
  return finite(value)
    ? new Intl.NumberFormat(undefined, {
        maximumFractionDigits: digits,
      }).format(value)
    : unavailable;
}
function money(value: unknown, currency = 'USD'): string {
  if (!finite(value)) return unavailable;
  if (currency !== 'USD') return `${number(value, 4)} ${currency}`;
  return new Intl.NumberFormat(undefined, {
    style: 'currency',
    currency,
    notation: Math.abs(value) >= 10_000 ? 'compact' : 'standard',
    maximumFractionDigits: Math.abs(value) >= 100 ? 0 : 2,
  }).format(value);
}
function duration(from: unknown, to: number): string {
  if (!finite(from) || from <= 0 || from > to) return unavailable;
  const seconds = Math.floor((to - from) / 1_000);
  return seconds >= 3_600
    ? `${Math.floor(seconds / 3_600)}h ${Math.floor((seconds % 3_600) / 60)}m`
    : seconds >= 60
      ? `${Math.floor(seconds / 60)}m`
      : `${seconds}s`;
}
function stamp(value: unknown): string {
  return finite(value) && value > 0
    ? new Date(value).toLocaleString()
    : unavailable;
}
function identity(value: unknown): Record<string, unknown> | null {
  return record(value);
}
function profileId(position: AutoTraderOutcome): unknown {
  const lifecycle = identity(position.ledgerTrade?.lifecycle);
  return (
    position.entryProfileId ?? position.exitProfileId ?? lifecycle?.profile_id
  );
}

function entryMode(position: AutoTraderOutcome): string {
  const mode = position.entryPolicy ?? position.source;
  if (mode === 'strict') return 'Strict';
  if (mode === 'exploratory') return 'Explore';
  if (mode === 'scout') return 'Scout';
  return unavailable;
}

function directionSign(position: AutoTraderOutcome): number {
  const direction = `${position.direction} ${position.side}`.toLowerCase();
  return direction.includes('short') ||
    direction.includes('bear') ||
    direction.includes('sell')
    ? -1
    : 1;
}

function usesSignalManagedExits(position: AutoTraderOutcome): boolean {
  return (
    position.exitControlMode === 'signal_managed' ||
    (position.stopLossActive === false && position.takeProfitActive === false)
  );
}

function netBar(position: AutoTraderOutcome, stale = false) {
  const operational = position.operationalPosition;
  const returnPercent = operational?.net_edge_vs_breakeven_percent;
  const protectionMarkers = (operational?.protection_markers ?? []).flatMap(
    (marker, index) => {
      const value = marker.projected_net_return_percent;
      if (!finite(value)) return [];
      const orderKind = (marker.order_kind ?? '').toLowerCase();
      const kind = /stop|loss/.test(orderKind)
        ? 'stop'
        : /take|profit/.test(orderKind)
          ? 'target'
          : 'dynamic';
      return [
        {
          label: `PM ${marker.order_kind ?? `trigger ${index + 1}`}`,
          value,
          kind: kind as 'stop' | 'target' | 'dynamic',
        },
      ];
    },
  );
  const markers = [
    ...(finite(returnPercent)
      ? [{ label: 'Close now', value: returnPercent, kind: 'current' as const }]
      : []),
    ...protectionMarkers,
  ];
  const maxAbs =
    Math.max(1, ...markers.map((marker) => Math.abs(marker.value))) * 1.15;
  const mark = operational?.executable_mark;
  const markLabel = finite(mark)
    ? `Executable ${operational?.executable_close_side ?? 'close'} ${number(mark, 6)} ${operational?.quote_currency ?? position.pnlQuoteCurrency}`
    : operational
      ? 'Executable mark unavailable'
      : 'No exact PM operational identity linked';
  const currentLabel = finite(returnPercent)
    ? `Close now ${returnPercent >= 0 ? '+' : ''}${number(returnPercent, 3)}% · ${money(operational?.net_unrealized_usd)}`
    : `${markLabel} · net return unavailable`;
  return (
    <InlineNetPnlBar
      min={-maxAbs}
      max={maxAbs}
      currentLabel={stale ? `${currentLabel} · stale snapshot` : currentLabel}
      breakevenAt={finite(returnPercent) ? 0 : undefined}
      breakevenLabel={
        finite(returnPercent)
          ? 'Net breakeven 0.000%'
          : 'Net breakeven unavailable'
      }
      markers={[...markers]}
    />
  );
}

function OpenRowDetails({
  position,
  state,
  nowMs,
}: {
  position: AutoTraderOutcome;
  state: OrdersPositionsState;
  nowMs: number;
}) {
  const barriers = barriersForPosition(
    position,
    state.history.activeBarrierOrders,
    state.activityOrders,
  );
  const lifecycle = identity(position.ledgerTrade?.lifecycle);
  const provenance = position.exitParameterProvenance;
  const protection = position.currentPmBarrier;
  const operational = position.operationalPosition;
  const signalManagedExits = usesSignalManagedExits(position);
  return (
    <ExpandableDetails label="Details">
      <dl className="operator-details-grid">
        <div>
          <dt>Symbol / strategy</dt>
          <dd>
            {position.symbol} / {position.strategy}
          </dd>
        </div>
        <div>
          <dt>Profile / version / selection</dt>
          <dd>
            {value(profileId(position))} /{' '}
            {value(position.entryProfileVersion ?? position.exitProfileVersion)}{' '}
            / {position.entryPolicy ?? value(position.source)}
          </dd>
        </div>
        <div>
          <dt>Selected stop / target</dt>
          <dd>
            {signalManagedExits
              ? 'Inactive · signal-managed exit contract'
              : finite(position.selectedStopLossPercent)
                ? `${number(position.selectedStopLossPercent, 3)}% · ${value(position.selectedStopLossSource)}`
                : unavailable}{' '}
            /{' '}
            {signalManagedExits
              ? 'Inactive · signal-managed exit contract'
              : finite(position.selectedTakeProfitPercent)
                ? `${number(position.selectedTakeProfitPercent, 3)}% · ${value(position.selectedTakeProfitSource)}`
                : unavailable}
          </dd>
        </div>
        <div>
          <dt>Effective stop / target</dt>
          <dd>
            {finite(position.currentEffectiveStopLossPercent)
              ? `${number(position.currentEffectiveStopLossPercent, 3)}%`
              : unavailable}{' '}
            /{' '}
            {finite(position.currentEffectiveTakeProfitPercent)
              ? `${number(position.currentEffectiveTakeProfitPercent, 3)}%`
              : unavailable}
          </dd>
        </div>
        <div>
          <dt>Profit protection at entry</dt>
          <dd>
            {provenance?.profit_protection
              ? `${number(provenance.profit_protection.activation_percent, 3)}% activation · ${number(provenance.profit_protection.giveback_percent, 3)}% giveback · ${value(provenance.profit_protection.source)}`
              : unavailable}
          </dd>
        </div>
        <div>
          <dt>Durable barrier</dt>
          <dd>
            {protection?.id
              ? `${value(protection.id)} · ${value(protection.status)} · trigger ${number(protection.trigger_price, 6)}`
              : barriers
                  .map(
                    (barrier) =>
                      `${value(barrier.id)} · ${value(barrier.status)}`,
                  )
                  .join(', ') || 'No stable-linked barrier record'}
          </dd>
        </div>
        <div>
          <dt>Protection adjustments</dt>
          <dd>
            {protection?.trigger_adjustments?.length
              ? protection.trigger_adjustments
                  .map(
                    (row) =>
                      `${value(row.reason)} ${number(row.expected_trigger_price, 6)} → ${number(row.new_trigger_price, 6)} · ${stamp(row.adjusted_at_ms)}`,
                  )
                  .join('; ')
              : unavailable}
          </dd>
        </div>
        <div>
          <dt>Decision / opportunity / episode / trade</dt>
          <dd>
            {value(position.decisionId)} /{' '}
            {value(position.opportunityId ?? lifecycle?.opportunity_id)} /{' '}
            {value(position.episodeId ?? lifecycle?.episode_id)} /{' '}
            {value(position.tradeId ?? lifecycle?.trade_id)}
          </dd>
        </div>
        <div>
          <dt>Operational position identity</dt>
          <dd>
            {value(operational?.position_identity)} ·{' '}
            {operational
              ? `${state.operationalSummary?.stale ? 'Stale · ' : ''}Captured ${stamp(state.operationalSummary?.capturedAtMs)}`
              : 'No unique canonical trade ID match; symbol-only match withheld'}
          </dd>
        </div>
        <div>
          <dt>Entry notional</dt>
          <dd>
            {finite(operational?.entry_value_usd)
              ? money(operational.entry_value_usd)
              : finite(operational?.entry_value_quote)
                ? money(
                    operational.entry_value_quote,
                    operational.quote_currency ?? position.pnlQuoteCurrency,
                  )
                : money(position.entryNotionalUsd)}
          </dd>
        </div>
        <div>
          <dt>Executable mark</dt>
          <dd>
            {finite(operational?.executable_mark)
              ? `${value(operational.executable_close_side)} ${number(operational.executable_mark, 6)} ${value(operational.quote_currency)} · ${number(operational.mark_age_ms, 0)} ms old · ${value(operational.mark_source_record_id)}`
              : unavailable}
          </dd>
        </div>
        <div>
          <dt>Gross / net unrealised</dt>
          <dd>
            {finite(operational?.gross_unrealized_quote)
              ? money(
                  operational.gross_unrealized_quote,
                  operational.quote_currency ?? position.pnlQuoteCurrency,
                )
              : unavailable}{' '}
            /{' '}
            {finite(operational?.net_unrealized_quote)
              ? money(
                  operational.net_unrealized_quote,
                  operational.quote_currency ?? position.pnlQuoteCurrency,
                )
              : unavailable}{' '}
            ({money(operational?.net_unrealized_usd)})
          </dd>
        </div>
        <div>
          <dt>Signed / gross exposure</dt>
          <dd>
            {finite(operational?.signed_exposure_usd)
              ? money(operational.signed_exposure_usd)
              : unavailable}{' '}
            /{' '}
            {finite(operational?.gross_exposure_usd)
              ? money(operational.gross_exposure_usd)
              : unavailable}
          </dd>
        </div>
        <div>
          <dt>Valuation evidence</dt>
          <dd>
            {value(operational?.valuation_status)} ·{' '}
            {operational?.valuation_reasons?.join(', ') ||
              value(operational?.net_pnl_cost_scope)}
          </dd>
        </div>
        <div>
          <dt>Executable source</dt>
          <dd>
            {value(operational?.mark_source_system)} /{' '}
            {value(operational?.mark_source_epoch)} /{' '}
            {value(operational?.mark_source_cursor)} /{' '}
            {value(operational?.mark_source_record_id)} ·{' '}
            {stamp(operational?.mark_updated_at_ms)}
          </dd>
        </div>
        <div>
          <dt>Quote FX evidence</dt>
          <dd>
            {number(operational?.quote_fx_rate, 8)} ·{' '}
            {stamp(operational?.quote_fx_updated_at_ms)} ·{' '}
            {value(operational?.quote_fx_source_record_id)}
          </dd>
        </div>
        <div>
          <dt>Net P&amp;L cost scope</dt>
          <dd>{value(operational?.net_pnl_cost_scope)}</dd>
        </div>
        <div>
          <dt>Fee evidence</dt>
          <dd>
            {number(operational?.entry_fees_quote, 6)}{' '}
            {value(operational?.quote_currency)} entry fees · close rate{' '}
            {number(operational?.projected_exit_fee_rate, 6)} ·{' '}
            {value(operational?.fee_schedule_status)} ·{' '}
            {number(operational?.known_fee_fills, 0)} known /{' '}
            {number(operational?.unknown_fee_fills, 0)} unknown fills
          </dd>
        </div>
        <div>
          <dt>Exit fee schedule evidence</dt>
          <dd>
            {value(operational?.exit_fee_schedule_id)} v
            {value(operational?.exit_fee_schedule_version)} ·{' '}
            {value(operational?.exit_fee_schedule_source_epoch)} /{' '}
            {value(operational?.exit_fee_schedule_source_cursor)} /{' '}
            {value(operational?.exit_fee_schedule_source_record_id)} ·{' '}
            {stamp(operational?.exit_fee_schedule_source_timestamp_ms)}
          </dd>
        </div>
        <div>
          <dt>Protection exception</dt>
          <dd>
            {operational?.protection_exception === true
              ? `Exception · ${value(operational.protection_status)}`
              : operational?.protection_exception === false
                ? value(operational.protection_status)
                : unavailable}
          </dd>
        </div>
        <div>
          <dt>Protection orders</dt>
          <dd>
            {operational?.protection_orders?.length
              ? operational.protection_orders
                  .map(
                    (order) =>
                      `${value(order.id)} · ${value(order.order_kind)} · ${value(order.status)}${order.error ? ` · ${order.error}` : ''}`,
                  )
                  .join('; ')
              : value(operational?.protection_status)}
          </dd>
        </div>
        <div>
          <dt>Entry fees / initial risk</dt>
          <dd>
            {finite(position.entryFees)
              ? money(position.entryFees, position.pnlQuoteCurrency)
              : unavailable}{' '}
            /{' '}
            {position.initialRiskEvidence?.eligibleForRMultiple &&
            finite(position.initialRiskEvidence.quoteAmount)
              ? `${money(position.initialRiskEvidence.quoteAmount, position.initialRiskEvidence.quoteCurrency ?? position.pnlQuoteCurrency)} · eligible`
              : value(position.initialRiskEvidence?.eligibilityStatus)}
          </dd>
        </div>
        <div>
          <dt>Holding duration</dt>
          <dd>{duration(position.openedAt, nowMs)}</dd>
        </div>
        <div>
          <dt>Entry / quantity / quote</dt>
          <dd>
            {number(position.entryPrice, 6)} /{' '}
            {number(position.remainingQuantity ?? position.quantity, 6)} /{' '}
            {value(position.pnlQuoteCurrency)}
          </dd>
        </div>
        <div>
          <dt>Entry timestamp / execution mode</dt>
          <dd>
            {stamp(position.openedAt)} / {value(position.executionMode)}
          </dd>
        </div>
        <div>
          <dt>Technical IDs</dt>
          <dd>
            <details className="operator-row-details">
              <summary>Expand / copy</summary>
              <div className="orders-positions-ids">
                <code>Position {position.rowId}</code>
                <code>Decision {position.decisionId}</code>
                {position.tradeId && <code>Trade {position.tradeId}</code>}
                {position.opportunityId && (
                  <code>Opportunity {position.opportunityId}</code>
                )}
                {position.episodeId && (
                  <code>Episode {position.episodeId}</code>
                )}
              </div>
            </details>
          </dd>
        </div>
      </dl>
      {position.exitControlMode === 'signal_managed' && (
        <p>
          Signal-managed strategy exits; a fixed PM stop/target is not required
          by this strategy contract.
        </p>
      )}
    </ExpandableDetails>
  );
}

function OpenPositionsTable({
  rows,
  state,
  nowMs,
  onSort,
}: {
  rows: AutoTraderOutcome[];
  state: OrdersPositionsState;
  nowMs: number;
  onSort?: (key: PositionSort['key']) => void;
}) {
  const sort = state.openSort ?? {
    key: 'symbol' as const,
    direction: 'asc' as const,
  };
  return (
    <CompactTable label="Open positions">
      <thead>
        <tr>
          <th
            scope="col"
            aria-sort={
              sort.key === 'symbol' || sort.key === 'strategy'
                ? sort.direction === 'asc'
                  ? 'ascending'
                  : 'descending'
                : 'none'
            }
          >
            <button
              type="button"
              className="operator-sort-button"
              onClick={() => onSort?.('symbol')}
            >
              Symbol
            </button>
            <span className="operator-muted"> / </span>
            <button
              type="button"
              className="operator-sort-button"
              onClick={() => onSort?.('strategy')}
            >
              Strategy
            </button>
            {sort.key === 'symbol' || sort.key === 'strategy' ? (
              <span aria-hidden="true">
                {sort.direction === 'asc' ? ' ↑' : ' ↓'}
              </span>
            ) : null}
          </th>
          <SortableHeader
            label="Side"
            active={sort.key === 'side'}
            direction={sort.direction}
            onSort={() => onSort?.('side')}
          />
          <SortableHeader
            label="Entry value"
            active={sort.key === 'notional'}
            direction={sort.direction}
            onSort={() => onSort?.('notional')}
          />
          <SortableHeader
            label="Held"
            active={sort.key === 'holding'}
            direction={sort.direction}
            onSort={() => onSort?.('holding')}
          />
          <th scope="col">Net vs breakeven</th>
          <th scope="col">Mode</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((position) => (
          <tr key={position.rowId}>
            <td>
              <strong title={position.symbol}>{position.symbol}</strong>
              <br />
              <span
                className="operator-muted open-position-strategy"
                title={position.strategy}
              >
                {position.strategy}
              </span>
              <OpenRowDetails position={position} state={state} nowMs={nowMs} />
            </td>
            <td
              className={
                directionSign(position) > 0
                  ? 'operator-positive'
                  : 'operator-negative'
              }
            >
              {position.direction === 'bullish'
                ? 'Long'
                : position.direction === 'bearish'
                  ? 'Short'
                  : value(position.direction)}
            </td>
            <td>
              {finite(position.operationalPosition?.entry_value_usd) ? (
                <>
                  {money(position.operationalPosition.entry_value_usd)}
                  <small> entry</small>
                </>
              ) : finite(position.operationalPosition?.entry_value_quote) ? (
                <>
                  {money(
                    position.operationalPosition.entry_value_quote,
                    position.operationalPosition.quote_currency ??
                      position.pnlQuoteCurrency,
                  )}
                  <small> entry</small>
                </>
              ) : finite(position.entryNotionalUsd) ? (
                <>
                  {money(position.entryNotionalUsd)}
                  <small> entry</small>
                </>
              ) : (
                unavailable
              )}
            </td>
            <td>{duration(position.openedAt, nowMs)}</td>
            <td>{netBar(position, state.operationalSummary?.stale)}</td>
            <td>{entryMode(position)}</td>
          </tr>
        ))}
      </tbody>
    </CompactTable>
  );
}

function ClosedRowDetails({
  position,
}: {
  position: OrdersPositionsState['closedPositions'][number];
}) {
  const lifecycle = identity(position.ledgerTrade?.lifecycle);
  const risk = position.initialRiskEvidence;
  return (
    <ExpandableDetails label="Details">
      <dl className="operator-details-grid">
        <div>
          <dt>Symbol / strategy</dt>
          <dd>
            {position.symbol} / {position.strategy}
          </dd>
        </div>
        <div>
          <dt>Trade / decision / opportunity / episode</dt>
          <dd>
            {value(position.tradeId ?? lifecycle?.trade_id)} /{' '}
            {value(position.decisionId)} /{' '}
            {value(position.opportunityId ?? lifecycle?.opportunity_id)} /{' '}
            {value(position.episodeId ?? lifecycle?.episode_id)}
          </dd>
        </div>
        <div>
          <dt>Profile / version / mode</dt>
          <dd>
            {value(lifecycle?.profile_id)} / {value(lifecycle?.profile_version)}{' '}
            / {value(lifecycle?.entry_policy ?? position.source)}
          </dd>
        </div>
        <div>
          <dt>Entry / exit</dt>
          <dd>
            {number(position.entryPrice, 6)} /{' '}
            {number(position.ledgerTrade?.execution_price, 6)}
          </dd>
        </div>
        <div>
          <dt>Reported P&amp;L / fees</dt>
          <dd>
            {money(position.pnl, position.pnlQuoteCurrency)} /{' '}
            {finite(position.totalFees)
              ? `${number(position.totalFees, 6)} ${position.pnlQuoteCurrency}`
              : unavailable}
          </dd>
        </div>
        <div>
          <dt>Independent reconciliation</dt>
          <dd>Unavailable in this position contract</dd>
        </div>
        <div>
          <dt>Net return</dt>
          <dd>Unavailable in this position contract</dd>
        </div>
        <div>
          <dt>Realised R multiple</dt>
          <dd>
            {risk?.eligibleForRMultiple
              ? 'Initial risk is eligible; realised R remains unavailable'
              : value(risk?.eligibilityStatus)}
          </dd>
        </div>
        <div>
          <dt>Initial risk</dt>
          <dd>
            {finite(risk?.quoteAmount)
              ? `${money(risk.quoteAmount, risk.quoteCurrency ?? position.pnlQuoteCurrency)} · ${value(risk.eligibilityStatus)}`
              : unavailable}
          </dd>
        </div>
        <div>
          <dt>Learning eligibility</dt>
          <dd>Unavailable in this position contract</dd>
        </div>
        <div>
          <dt>Executable path evidence</dt>
          <dd>
            {position.executablePricePathEvidence?.complete
              ? `${position.executablePricePathEvidence.retainedSampleCount} / ${position.executablePricePathEvidence.totalSampleCount} complete samples`
              : position.executablePricePathEvidence
                ? `${position.executablePricePathEvidence.retainedSampleCount} / ${position.executablePricePathEvidence.totalSampleCount} samples · incomplete`
                : 'Unavailable'}
          </dd>
        </div>
        <div>
          <dt>MFE / MAE</dt>
          <dd>
            {position.executablePricePathEvidence?.complete
              ? `${number(position.executablePricePathEvidence.grossExecutableMfePercent, 3)}% / ${number(position.executablePricePathEvidence.grossExecutableMaePercent, 3)}% gross executable`
              : 'Incomplete executable path · not eligible as complete MFE/MAE evidence'}
          </dd>
        </div>
        <div>
          <dt>Fill records</dt>
          <dd>{position.fillDetails.length}</dd>
        </div>
      </dl>
    </ExpandableDetails>
  );
}

function ClosedPositionsTable({
  rows,
  state,
  onSort,
}: {
  rows: OrdersPositionsState['closedPositions'];
  state: OrdersPositionsState;
  onSort?: (key: ClosedSort['key']) => void;
}) {
  const sort = state.closedSort ?? {
    key: 'closedAt' as const,
    direction: 'desc' as const,
  };
  return (
    <CompactTable label="Closed positions">
      <thead>
        <tr>
          <SortableHeader
            label="Symbol"
            active={sort.key === 'symbol'}
            direction={sort.direction}
            onSort={() => onSort?.('symbol')}
          />
          <SortableHeader
            label="Strategy"
            active={sort.key === 'strategy'}
            direction={sort.direction}
            onSort={() => onSort?.('strategy')}
          />
          <SortableHeader
            label="Side"
            active={sort.key === 'side'}
            direction={sort.direction}
            onSort={() => onSort?.('side')}
          />
          <SortableHeader
            label="Net P&L"
            active={sort.key === 'net'}
            direction={sort.direction}
            onSort={() => onSort?.('net')}
          />
          <th scope="col">Net %</th>
          <th scope="col">R</th>
          <th scope="col">MFE</th>
          <th scope="col">MAE</th>
          <th scope="col">Exit</th>
          <SortableHeader
            label="Evidence"
            active={sort.key === 'closedAt'}
            direction={sort.direction}
            onSort={() => onSort?.('closedAt')}
          />
        </tr>
      </thead>
      <tbody>
        {rows.map((position) => {
          const net = position.netPnlUsd;
          const path = position.executablePricePathEvidence;
          const risk = position.initialRiskEvidence;
          return (
            <tr key={position.rowId}>
              <td>
                <strong title={position.symbol}>{position.symbol}</strong>
                <ClosedRowDetails position={position} />
              </td>
              <td>
                <span
                  className="closed-strategy-text"
                  title={position.strategy}
                >
                  {position.strategy}
                </span>
              </td>
              <td>
                {position.direction === 'bullish'
                  ? 'Long'
                  : position.direction === 'bearish'
                    ? 'Short'
                    : unavailable}
              </td>
              <td
                className={
                  finite(net)
                    ? net >= 0
                      ? 'operator-positive'
                      : 'operator-negative'
                    : 'operator-muted'
                }
              >
                {finite(net) ? money(net) : unavailable}
              </td>
              <td>{unavailable}</td>
              <td>{unavailable}</td>
              <td>
                {path?.complete && finite(path.grossExecutableMfePercent)
                  ? `${number(path.grossExecutableMfePercent, 2)}%`
                  : unavailable}
              </td>
              <td>
                {path?.complete && finite(path.grossExecutableMaePercent)
                  ? `${number(path.grossExecutableMaePercent, 2)}%`
                  : unavailable}
              </td>
              <td>
                <span
                  className="closed-exit-text"
                  title={value(position.exitReason)}
                >
                  {value(position.exitReason)}
                </span>
              </td>
              <td>
                <span
                  className="closed-evidence"
                  title={
                    (path?.complete ? 'Path complete' : 'Path incomplete') +
                    (risk?.eligibleForRMultiple ? ' · R eligible' : '')
                  }
                >
                  {path?.complete ? 'Path complete' : 'Path incomplete'}
                  {risk?.eligibleForRMultiple ? ' · R eligible' : ''}
                </span>
              </td>
            </tr>
          );
        })}
      </tbody>
    </CompactTable>
  );
}

function statusTone(status: unknown) {
  const normalized = typeof status === 'string' ? status.toLowerCase() : '';
  return /armed|active|filled|complete/.test(normalized)
    ? 'operator-positive'
    : /interrupt|fail|reject|cancel|expir/.test(normalized)
      ? 'operator-negative'
      : 'operator-muted';
}

function OrderTable({
  rows,
  state,
  nowMs,
  active,
}: {
  rows: LifecycleOrder[];
  state: OrdersPositionsState;
  nowMs: number;
  active: boolean;
}) {
  const records = [
    ...new Map(
      rows.map((row, index) => [row.id ?? `missing-${index}`, row]),
    ).values(),
  ];
  return (
    <CompactTable label={active ? 'Active orders' : 'Order history'}>
      <thead>
        <tr>
          <th scope="col">Symbol / side</th>
          <th scope="col">Type</th>
          <th scope="col">Price</th>
          <th scope="col">Filled / requested</th>
          <th scope="col">Status</th>
          <th scope="col">Age</th>
          <th scope="col">Details</th>
        </tr>
      </thead>
      <tbody>
        {records.map((order) => {
          const linked = linkActivityOrder(order, state.activityOrders);
          const terminal = active
            ? null
            : terminalPathForOrder(order, state.history.terminalPaths);
          const orderLifecycle = identity(order.lifecycle);
          return (
            <tr key={value(order.id)}>
              <td>
                <strong>{value(order.symbol)}</strong>
                <br />
                {value(order.side).toUpperCase()}
              </td>
              <td>{value(linked?.order_type ?? order.order_type)}</td>
              <td>{number(order.limit_price, 6)}</td>
              <td>
                {number(linked?.filled_quantity, 6)} /{' '}
                {number(order.quantity, 6)}
              </td>
              <td
                className={statusTone(
                  active ? 'active' : terminal?.terminal_reason,
                )}
              >
                {active ? 'Active' : value(terminal?.terminal_reason)}
              </td>
              <td>{duration(order.timestamp, nowMs)}</td>
              <td>
                <ExpandableDetails label="Order details">
                  <dl className="operator-details-grid">
                    <div>
                      <dt>Order / client / parent</dt>
                      <dd>
                        {value(order.id)} / {value(order.client_order_id)} /{' '}
                        {value(order.parent_order_id)}
                      </dd>
                    </div>
                    <div>
                      <dt>Decision / strategy</dt>
                      <dd>
                        {value(linked?.decision_id)} / {value(linked?.strategy)}
                      </dd>
                    </div>
                    <div>
                      <dt>Execution mode / profile</dt>
                      <dd>
                        {value(orderLifecycle?.execution_mode)} /{' '}
                        {value(orderLifecycle?.profile_id)}
                      </dd>
                    </div>
                    <div>
                      <dt>Created / updated / terminal</dt>
                      <dd>
                        {stamp(order.timestamp)} / {stamp(order.updated_at)} /{' '}
                        {stamp(terminal?.terminal_at_ms)}
                      </dd>
                    </div>
                    <div>
                      <dt>Recovery / acknowledgement</dt>
                      <dd>
                        {value(order.recovery_state)} /{' '}
                        {value(order.acknowledgement_state)}
                      </dd>
                    </div>
                  </dl>
                </ExpandableDetails>
              </td>
            </tr>
          );
        })}
      </tbody>
    </CompactTable>
  );
}

function BarrierTable({ rows }: { rows: LifecycleBarrier[] }) {
  return (
    <CompactTable label="Protective barriers">
      <thead>
        <tr>
          <th scope="col">Symbol</th>
          <th scope="col">Type / status</th>
          <th scope="col">Trigger</th>
          <th scope="col">Quantity</th>
          <th scope="col">Details</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((barrier) => (
          <tr key={value(barrier.id)}>
            <td>
              <strong>{value(barrier.symbol)}</strong>
              <br />
              {value(barrier.side).toUpperCase()}
            </td>
            <td>
              {value(barrier.order_kind)}
              <br />
              <span className={statusTone(barrier.status)}>
                {value(barrier.status)}
              </span>
            </td>
            <td>{number(barrier.trigger_price, 6)}</td>
            <td>{number(barrier.remaining_quantity ?? barrier.quantity, 6)}</td>
            <td>
              <ExpandableDetails label="Barrier details">
                <dl className="operator-details-grid">
                  <div>
                    <dt>Barrier / client / parent</dt>
                    <dd>
                      {value(barrier.id)} / {value(barrier.client_order_id)} /{' '}
                      {value(barrier.parent_order_id)}
                    </dd>
                  </div>
                  <div>
                    <dt>Created / triggered</dt>
                    <dd>
                      {stamp(barrier.created_at)} /{' '}
                      {stamp(barrier.triggered_at)}
                    </dd>
                  </div>
                  <div>
                    <dt>Recovery status</dt>
                    <dd>{value(barrier.error)}</dd>
                  </div>
                </dl>
              </ExpandableDetails>
            </td>
          </tr>
        ))}
      </tbody>
    </CompactTable>
  );
}

export function OrdersPositionsPanel({
  state,
  nowMs = Date.now(),
  initialTab = 'positions',
  onOpenSort,
  onClosedSort,
}: {
  state: OrdersPositionsState;
  nowMs?: number;
  initialTab?: Tab;
  onOpenSort?: (key: PositionSort['key']) => void;
  onClosedSort?: (key: ClosedSort['key']) => void;
}) {
  const [tab, setTab] = useState<Tab>(initialTab);
  const [population, setPopulation] = useState<'autotrader' | 'scouts'>(
    'autotrader',
  );
  const openRows = sortOpenPositions(
    state.positions.filter((row) =>
      population === 'scouts' ? row.scout : !row.scout,
    ),
    state.openSort ?? { key: 'symbol', direction: 'asc' },
  );
  const closedRows = sortClosedPositions(
    state.closedPositions.filter((row) =>
      population === 'scouts' ? row.scout : !row.scout,
    ),
    state.closedSort ?? { key: 'closedAt', direction: 'desc' },
  );
  const activityOnly = buildActivityOnlyTerminalOrders(state.activityOrders, [
    ...state.history.orders,
    ...state.history.activeOrders,
  ]);
  const metrics =
    tab === 'closed'
      ? [
          { label: 'Closed', value: closedRows.length },
          { label: 'Net realised', value: unavailable },
          { label: 'Fees', value: unavailable },
          { label: 'Reconciled', value: unavailable },
        ]
      : [
          {
            label: population === 'scouts' ? 'Scout observations' : 'Open',
            value: openRows.length,
          },
          { label: 'Pending', value: state.history.activeOrders.length },
          {
            label: 'Exposure',
            value: finite(state.operationalSummary?.grossExposureUsd)
              ? money(state.operationalSummary.grossExposureUsd)
              : unavailable,
            detail:
              state.operationalSummary?.error ??
              state.operationalSummary?.valuationStatus ??
              'Operational valuation unavailable',
          },
          {
            label: 'Net unrealised',
            value: finite(state.operationalSummary?.netUnrealizedUsd)
              ? money(state.operationalSummary.netUnrealizedUsd)
              : unavailable,
            detail:
              state.operationalSummary?.error ??
              state.operationalSummary?.valuationReasons?.join(', ') ??
              'Operational valuation unavailable',
          },
          {
            label: 'Protection exceptions',
            value: finite(state.operationalSummary?.protectionExceptionCount)
              ? state.operationalSummary.protectionExceptionCount
              : unavailable,
            detail:
              state.operationalSummary === null
                ? 'Operational protection unavailable'
                : 'Current PM barrier status',
          },
        ];
  return (
    <div className="orders-positions-redesign" data-view={tab}>
      <OperatorPanel
        hideTitle
        title={
          tab === 'closed'
            ? 'Orders & positions · closed'
            : 'Orders & positions'
        }
        context={
          <span>
            {state.pmAvailable
              ? 'Paper ledger'
              : state.loading
                ? 'Loading'
                : 'PM unavailable'}{' '}
            · {stamp(state.fetchedAtMs)}
          </span>
        }
        status={
          state.pmError ? (
            <span className="operator-status" data-tone="warning">
              History incomplete
            </span>
          ) : undefined
        }
        footer={
          <span>
            Paper execution · missing marks and evidence remain unavailable.
          </span>
        }
      >
        {state.outcomesError && (
          <p role="status" className="operator-muted">
            Positions unavailable · {state.outcomesError}
          </p>
        )}
        <OperatorMetricStrip
          metrics={metrics}
          label="Orders and positions summary"
        />
        <div
          className="orders-positions-population"
          role="group"
          aria-label="Execution population"
        >
          <button
            type="button"
            aria-pressed={population === 'autotrader'}
            onClick={() => setPopulation('autotrader')}
          >
            AutoTrader
          </button>
          <button
            type="button"
            aria-pressed={population === 'scouts'}
            onClick={() => setPopulation('scouts')}
          >
            Scouts
          </button>
        </div>
        <OperatorTabBar
          tabs={viewTabs}
          selectedId={tab === 'history' ? 'active' : tab}
          label="Orders and positions views"
          onSelect={(id) => setTab(id as Tab)}
        />
        {(tab === 'active' || tab === 'history') && (
          <OperatorTabBar
            tabs={[
              { id: 'active', label: 'Active' },
              { id: 'history', label: 'History' },
            ]}
            selectedId={tab}
            label="Order views"
            onSelect={(id) => setTab(id as Tab)}
          />
        )}
        {tab === 'positions' &&
          (openRows.length ? (
            <>
              <OpenPositionsTable
                rows={openRows}
                state={state}
                nowMs={nowMs}
                onSort={onOpenSort}
              />
            </>
          ) : (
            <p role="status" className="operator-muted">
              {state.outcomesLoading
                ? 'Loading positions…'
                : population === 'scouts'
                  ? 'No Scout observations in this view.'
                  : 'No open positions reported.'}
            </p>
          ))}
        {tab === 'closed' &&
          (closedRows.length ? (
            <ClosedPositionsTable
              rows={closedRows}
              state={state}
              onSort={onClosedSort}
            />
          ) : (
            <p role="status" className="operator-muted">
              {state.outcomesLoading
                ? 'Loading closed positions…'
                : 'No closed positions reported.'}
            </p>
          ))}
        {tab === 'active' &&
          (state.pmAvailable ? (
            <OrderTable
              rows={state.history.activeOrders}
              state={state}
              nowMs={nowMs}
              active
            />
          ) : (
            <p role="status" className="operator-muted">
              Active-order state unavailable.
            </p>
          ))}
        {tab === 'protection' &&
          (state.history.activeBarrierOrders.length ||
          state.history.barrierOrders.length ? (
            <BarrierTable
              rows={[
                ...new Map(
                  [
                    ...state.history.activeBarrierOrders,
                    ...state.history.barrierOrders,
                  ].map((barrier, index) => [
                    barrier.id ?? `missing-${index}`,
                    barrier,
                  ]),
                ).values(),
              ]}
            />
          ) : (
            <p role="status" className="operator-muted">
              No active protective barriers are reported in this snapshot.
            </p>
          ))}
        {tab === 'history' && (
          <>
            {state.history.orders.length ? (
              <OrderTable
                rows={state.history.orders}
                state={state}
                nowMs={nowMs}
                active={false}
              />
            ) : (
              <p role="status" className="operator-muted">
                {state.pmAvailable
                  ? 'No order-history rows in this page.'
                  : 'Order history unavailable.'}
              </p>
            )}
            {state.history.executionPathsTruncated && (
              <p className="operator-warning">
                Terminal execution-path retention is truncated.
              </p>
            )}
            {activityOnly.rows.length > 0 && (
              <ExpandableDetails
                label={`AutoTrader-only terminal submissions · ${activityOnly.sourceCount}`}
              >
                <CompactTable label="AutoTrader-only terminal submissions">
                  <thead>
                    <tr>
                      <th>Symbol / side</th>
                      <th>Status</th>
                      <th>Strategy</th>
                      <th>Reason / details</th>
                    </tr>
                  </thead>
                  <tbody>
                    {activityOnly.rows.map(
                      ({
                        key,
                        activity,
                      }: {
                        key: string;
                        activity: ActivityOrderIdentity;
                      }) => (
                        <tr key={key}>
                          <td>
                            {value(activity.symbol)} / {value(activity.side)}
                          </td>
                          <td>{value(activity.status)}</td>
                          <td>{value(activity.strategy)}</td>
                          <td>
                            {value(activity.failure_reason)}
                            <details className="operator-row-details">
                              <summary>Lineage</summary>
                              <p>
                                Decision {value(activity.decision_id)} ·
                                execution {value(activity.execution_order_id)} ·
                                client {value(activity.client_order_id)} ·
                                parent {value(activity.parent_order_id)}
                              </p>
                            </details>
                          </td>
                        </tr>
                      ),
                    )}
                  </tbody>
                </CompactTable>
              </ExpandableDetails>
            )}
          </>
        )}
      </OperatorPanel>
    </div>
  );
}

export function OrdersPositionsWidget() {
  const model = useMemo(() => new OrdersPositionsWidgetModel(), []);
  const state = useBehaviorSnapshot(model.state$);
  const [nowMs, setNowMs] = useState(() => Date.now());
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  useEffect(() => {
    const timer = window.setInterval(() => setNowMs(Date.now()), 5_000);
    return () => window.clearInterval(timer);
  }, []);
  return (
    <OrdersPositionsPanel
      state={state}
      nowMs={nowMs}
      onOpenSort={(key) => model.sortOpenBy(key)}
      onClosedSort={(key) => model.sortClosedBy(key)}
    />
  );
}
