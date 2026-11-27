import type { ReactNode } from 'react';

export type OperatorTone = 'neutral' | 'positive' | 'warning' | 'negative';

export type OperatorMetric = {
  label: string;
  value: ReactNode;
  detail?: ReactNode;
  tone?: OperatorTone;
};

export type OperatorKeyValue = {
  label: string;
  value: ReactNode;
  tone?: OperatorTone;
};

/** Compact, aligned operator settings/status rows with caller-owned values. */
export function OperatorKeyValueList({
  rows,
  label,
}: {
  rows: readonly OperatorKeyValue[];
  label: string;
}) {
  return (
    <dl className="operator-key-value-list" aria-label={label}>
      {rows.map((row) => (
        <div key={row.label} data-tone={row.tone ?? 'neutral'}>
          <dt>{row.label}</dt>
          <dd>{row.value}</dd>
        </div>
      ))}
    </dl>
  );
}

type OperatorPanelProps = {
  title: string;
  hideTitle?: boolean;
  context?: ReactNode;
  status?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
};

/** A compact, source-agnostic shell for operator dashboard panels. */
export function OperatorPanel({
  title,
  hideTitle = false,
  context,
  status,
  children,
  footer,
}: OperatorPanelProps) {
  return (
    <section className="operator-panel" aria-label={title}>
      <header className="operator-panel__header">
        {!hideTitle && <h2 className="operator-panel__title">{title}</h2>}
        <div className="operator-panel__context">
          {context}
          {status}
        </div>
      </header>
      <div className="operator-panel__body">{children}</div>
      {footer !== undefined && (
        <footer className="operator-panel__footer">{footer}</footer>
      )}
    </section>
  );
}

type OperatorMetricStripProps = {
  metrics: readonly OperatorMetric[];
  label?: string;
};

/** Displays caller-provided metrics without deriving or substituting values. */
export function OperatorMetricStrip({
  metrics,
  label = 'Summary metrics',
}: OperatorMetricStripProps) {
  return (
    <dl className="operator-metrics" aria-label={label}>
      {metrics.map((metric, index) => (
        <div
          className="operator-metric"
          data-tone={metric.tone ?? 'neutral'}
          key={`${metric.label}-${index}`}
        >
          <dt className="operator-metric__label">{metric.label}</dt>
          <dd className="operator-metric__value">{metric.value}</dd>
          {metric.detail !== undefined && (
            <dd className="operator-metric__detail">{metric.detail}</dd>
          )}
        </div>
      ))}
    </dl>
  );
}

type OperatorStatusProps = {
  children: ReactNode;
  tone?: OperatorTone;
  label?: string;
};

/** A compact, accessible state label; the text remains the source of truth. */
export function OperatorStatus({
  children,
  tone = 'neutral',
  label,
}: OperatorStatusProps) {
  return (
    <span
      className="operator-status"
      data-tone={tone}
      aria-label={label}
      role="status"
    >
      {children}
    </span>
  );
}

export type OperatorTab = {
  id: string;
  label: string;
  disabled?: boolean;
};

type OperatorTabBarProps = {
  tabs: readonly OperatorTab[];
  selectedId: string;
  onSelect: (id: string) => void;
  label: string;
};

/** Controlled navigation for panel views; selection stays with the caller. */
export function OperatorTabBar({
  tabs,
  selectedId,
  onSelect,
  label,
}: OperatorTabBarProps) {
  return (
    <nav className="operator-tabs" aria-label={label}>
      {tabs.map((tab) => (
        <button
          className="operator-tabs__item"
          type="button"
          key={tab.id}
          aria-current={tab.id === selectedId ? 'page' : undefined}
          aria-pressed={tab.id === selectedId}
          disabled={tab.disabled}
          onClick={() => onSelect(tab.id)}
        >
          {tab.label}
        </button>
      ))}
    </nav>
  );
}
