import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { WalkForwardProfiles } from './WalkForwardWidget';

describe('WalkForwardWidget', () => {
  it('renders camel-case sample evidence returned by AutoTrader', () => {
    const markup = renderToStaticMarkup(
      createElement(WalkForwardProfiles, {
        profiles: [
          {
            symbol: 'BTC-USDT',
            strategy: 'live-momentum',
            learning_samples: 0,
            realized_pnl: 0,
            walk_forward_evidence: {
              sampleCount: 37,
              positiveNetRate: 0.6,
              averageNetReturnPercent: 0.12,
              lastSampleTimestampMs: 1_800_000_000_000,
            },
            walk_forward_evidence_by_regime: {
              trend: {
                sampleCount: 14,
                positiveNetRate: 0.7,
                averageNetReturnPercent: 0.2,
                lastSampleTimestampMs: 1_800_000_000_000,
              },
            },
            parameters: [],
          },
        ],
      }),
    );

    assert.match(markup, />37</);
    assert.match(markup, />60\.0%<\/b>/);
    assert.match(markup, />\+0\.120%<\/b>/);
    assert.match(markup, /trend: 14 samples/);
  });
});
