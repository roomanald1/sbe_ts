import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

const styles = readFileSync(
  new URL('./operator_dashboard.css', import.meta.url),
  'utf8',
);
const widgetStyles = readFileSync(
  new URL('./dashboard_widgets.css', import.meta.url),
  'utf8',
);

describe('shared operator table sizing', () => {
  it('uses responsive minimum widths so compact tables scroll only when content needs it', () => {
    assert.match(
      styles,
      /\.operator-compact-table\s*\{[^}]*min-width:\s*min\(620px,\s*100%\)/s,
    );
    assert.match(
      styles,
      /@media\s*\(max-width:\s*520px\)\s*\{\s*\.operator-compact-table\s*\{[^}]*min-width:\s*min\(540px,\s*100%\)/s,
    );
  });

  it('keeps one panel scrollport, sticky column headers, and no nested table scroller', () => {
    assert.match(styles, /\.operator-panel__body\s*\{[^}]*overflow:\s*auto/s);
    assert.match(
      styles,
      /\.operator-compact-table-scroll\s*\{[^}]*overflow:\s*visible/s,
    );
    assert.match(
      styles,
      /\.operator-compact-table\s+th\s*\{[^}]*position:\s*sticky[^}]*top:\s*0/s,
    );
    assert.match(
      widgetStyles,
      /\.dashboard-widget-content:not\(:has\(\.operator-panel\)\)[\s\S]*?\[class\$='-table-wrap'\][\s\S]*?overflow:\s*auto/s,
    );
  });
});

describe('excursion table row selection', () => {
  it('shows a visible selected row and keyboard focus for the trade selector', () => {
    assert.match(
      styles,
      /\.auto-trader-excursions__trade-select:focus-visible\s*\{[^}]*outline:/s,
    );
    assert.match(
      styles,
      /\.auto-trader-excursions \.operator-compact-table tr\[data-selected='true'\]/,
    );
  });
});
