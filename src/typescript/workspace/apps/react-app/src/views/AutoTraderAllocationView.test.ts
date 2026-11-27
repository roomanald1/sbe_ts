import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { CapitalAllocation } from '../models/AutoTraderModel';
import { CapitalAllocationView } from './AutoTraderAllocationView';

describe('CapitalAllocationView', () => {
  it('renders a zero cold-start allocation and unknown evidence distinctly', () => {
    const allocation: CapitalAllocation = {
      version: 'v1',
      model: 'shared_pool',
      poolLimit: 500,
      used: 0,
      reserved: 0,
      available: 500,
      rows: [
        {
          strategy: 'trend-50-200',
          symbol: 'BTC-USDT',
          horizon: 'daily',
          effectiveAllocation: 0,
          reserved: 0,
          used: 0,
          utilization: 0,
          evidenceEligible: null,
          state: 'cold_start',
          reason: 'Evidence collection is pending',
        },
      ],
    };
    const markup = renderToStaticMarkup(
      createElement(CapitalAllocationView, {
        allocation,
        availableNotional: 500,
      }),
    );
    assert.match(markup, /trend-50-200 · BTC-USDT · daily/);
    assert.match(markup, /Effective USD 0\.00/);
    assert.match(markup, /Evidence eligibility: unknown/);
    assert.match(markup, /State: cold_start/);
    assert.match(markup, /Evidence collection is pending/);
    assert.doesNotMatch(markup, /profit|return|win rate/i);
  });

  it('shows missing contract as unavailable rather than zero allocation', () => {
    const markup = renderToStaticMarkup(
      createElement(CapitalAllocationView, {
        availableNotional: 500,
      }),
    );
    assert.match(markup, /Effective allocation details are unavailable/);
    assert.match(markup, /USD 500\.00/);
    assert.match(markup, /does not indicate zero allocation/);
  });
});
