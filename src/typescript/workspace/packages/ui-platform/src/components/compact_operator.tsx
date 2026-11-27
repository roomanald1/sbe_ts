import type { ReactNode } from 'react';

export function CompactTable({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <div className="operator-compact-table-scroll">
      <table className="operator-compact-table" aria-label={label}>
        {children}
      </table>
    </div>
  );
}

export function SortableHeader({
  label,
  active,
  direction,
  onSort,
}: {
  label: string;
  active: boolean;
  direction: 'asc' | 'desc';
  onSort: () => void;
}) {
  return (
    <th
      scope="col"
      aria-sort={
        active ? (direction === 'asc' ? 'ascending' : 'descending') : 'none'
      }
    >
      <button type="button" className="operator-sort-button" onClick={onSort}>
        {label}
        {active && (
          <span aria-hidden="true">{direction === 'asc' ? ' ↑' : ' ↓'}</span>
        )}
      </button>
    </th>
  );
}

export type OperatorStackedSegment = {
  id: string;
  label: string;
  value: string;
  sharePercent: number;
  tone: 'positive' | 'warning' | 'negative' | 'neutral';
};

/** Renders source-computed segment shares without deriving allocation values. */
export function OperatorStackedBar({
  label,
  segments,
}: {
  label: string;
  segments: readonly OperatorStackedSegment[];
}) {
  const reported = segments.filter(
    (segment) =>
      Number.isFinite(segment.sharePercent) && segment.sharePercent > 0,
  );
  return (
    <div
      className="operator-stacked-bar"
      role="img"
      aria-label={
        reported.length
          ? label +
            ': ' +
            reported
              .map((segment) => `${segment.label} ${segment.value}`)
              .join(', ')
          : label + ': unavailable'
      }
    >
      {reported.length ? (
        reported.map((segment) => (
          <span
            key={segment.id}
            className="operator-stacked-bar__segment"
            data-tone={segment.tone}
            style={{
              width: `${Math.max(0, Math.min(100, segment.sharePercent))}%`,
            }}
            title={`${segment.label}: ${segment.value}`}
          />
        ))
      ) : (
        <span className="operator-stacked-bar__empty" />
      )}
    </div>
  );
}

export type NetPnlMarker = {
  label: string;
  value: number;
  kind: 'stop' | 'target' | 'dynamic' | 'current' | 'breakeven';
};

/** Source values are supplied by the caller; this component only lays out the bar. */
export function InlineNetPnlBar({
  markers,
  min = -1,
  max = 1,
  breakevenAt,
  breakevenLabel = 'Net breakeven unavailable',
  currentLabel = 'Close now unavailable',
}: {
  markers: readonly NetPnlMarker[];
  min?: number;
  max?: number;
  breakevenAt?: number;
  breakevenLabel?: string;
  currentLabel?: string;
}) {
  const position = (value: number) =>
    `${Math.max(0, Math.min(100, ((value - min) / (max - min)) * 100))}%`;
  return (
    <div className="operator-net-pnl" aria-label={currentLabel}>
      <div className="operator-net-pnl__labels">
        <span>{currentLabel}</span>
        <span>{breakevenLabel}</span>
      </div>
      <div
        className="operator-net-pnl__track"
        role="img"
        aria-label={
          markers
            .map((marker) => `${marker.label} ${marker.value.toFixed(2)}%`)
            .join(', ') || 'No sourced protection markers'
        }
      >
        {breakevenAt !== undefined && Number.isFinite(breakevenAt) && (
          <i
            className="operator-net-pnl__breakeven"
            style={{ left: position(breakevenAt) }}
            title="Net breakeven"
          />
        )}
        {markers.map((marker) => (
          <i
            key={`${marker.kind}:${marker.label}`}
            className={`operator-net-pnl__marker is-${marker.kind}`}
            style={{ left: position(marker.value) }}
            title={`${marker.label} entry-price move ${marker.value >= 0 ? '+' : ''}${marker.value.toFixed(2)}%`}
          />
        ))}
      </div>
      <div className="operator-net-pnl__legend">
        {markers.map((marker) => (
          <span
            key={`${marker.kind}:${marker.label}`}
            className={`is-${marker.kind}`}
          >
            <i />
            {marker.label} {marker.value >= 0 ? '+' : ''}
            {marker.value.toFixed(2)}%
          </span>
        ))}
      </div>
    </div>
  );
}

export function ExpandableDetails({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <details className="operator-row-details">
      <summary>{label}</summary>
      <div className="operator-row-details__body">{children}</div>
    </details>
  );
}
