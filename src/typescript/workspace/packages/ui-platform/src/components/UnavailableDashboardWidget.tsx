import type { ReactNode } from 'react';

export function UnavailableDashboardWidget({
  type,
}: {
  type: string;
}): ReactNode {
  return (
    <section
      className="ui-unavailable-dashboard-widget"
      role="status"
      aria-label="Saved widget unavailable"
    >
      <p>This saved widget type is unavailable in this version.</p>
      <details>
        <summary>Saved widget details</summary>
        <dl>
          <dt>Type</dt>
          <dd>{type}</dd>
          <dt>Saved layout</dt>
          <dd>Preserved. Replace or remove this widget manually.</dd>
        </dl>
      </details>
    </section>
  );
}
