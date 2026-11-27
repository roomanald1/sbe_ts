import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import {
  OperatorMetricStrip,
  OperatorPanel,
  OperatorStatus,
  OperatorTabBar,
} from './operator_dashboard.tsx';

describe('shared operator dashboard components', () => {
  it('renders a labelled panel with optional context, state, and footer', () => {
    const html = renderToStaticMarkup(
      createElement(OperatorPanel, {
        title: 'Portfolio',
        context: 'All exchanges',
        status: createElement(OperatorStatus, {
          tone: 'positive',
          children: 'Reconciled',
        }),
        footer: 'Last checked 12s ago',
        children: createElement('p', null, 'No verified intervals available'),
      }),
    );

    assert.match(html, /aria-label="Portfolio"/);
    assert.match(html, /All exchanges/);
    assert.match(html, /data-tone="positive"/);
    assert.match(html, /No verified intervals available/);
    assert.match(html, /Last checked 12s ago/);
  });

  it('keeps metric labels and caller-provided unavailable values explicit', () => {
    const html = renderToStaticMarkup(
      createElement(OperatorMetricStrip, {
        label: 'Execution summary',
        metrics: [
          { label: 'Filled', value: 0 },
          { label: 'Average fill', value: 'N/A', detail: 'No settled fills' },
        ],
      }),
    );

    assert.match(html, /aria-label="Execution summary"/);
    assert.match(html, /<dt[^>]*>Filled<\/dt><dd[^>]*>0<\/dd>/);
    assert.match(html, /<dt[^>]*>Average fill<\/dt><dd[^>]*>N\/A<\/dd>/);
    assert.match(html, /No settled fills/);
    assert.doesNotMatch(html, />undefined</);
  });

  it('exposes status tone while retaining readable status text', () => {
    const html = renderToStaticMarkup(
      createElement(OperatorStatus, {
        tone: 'warning',
        label: 'Evidence state',
        children: 'Incomplete',
      }),
    );

    assert.match(html, /role="status"/);
    assert.match(html, /aria-label="Evidence state"/);
    assert.match(html, /data-tone="warning"/);
    assert.match(html, /Incomplete/);
  });

  it('renders controlled tab selection without selecting disabled views', () => {
    const html = renderToStaticMarkup(
      createElement(OperatorTabBar, {
        label: 'Execution views',
        tabs: [
          { id: 'summary', label: 'Summary' },
          { id: 'path', label: 'Market path', disabled: true },
        ],
        selectedId: 'summary',
        onSelect: () => assert.fail('SSR must not trigger selection'),
      }),
    );

    assert.match(html, /aria-label="Execution views"/);
    assert.match(html, /aria-pressed="true"[^>]*>Summary/);
    assert.match(html, /disabled=""[^>]*>Market path/);
  });
});
