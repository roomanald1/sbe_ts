import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { pairedScoutDeltas } from './AutoTraderShadowModel';

describe('pairedScoutDeltas', () => {
  it('shows a positive comparison when both raw outcomes are losses', () => {
    const deltas = pairedScoutDeltas([
      { stage: 'scout-outcome', scout_comparison_id: 'pair-1', scout_comparison_role: 'baseline', shadow_net_return_percent: -0.609 },
      { stage: 'scout-outcome', scout_comparison_id: 'pair-1', scout_comparison_role: 'variant', shadow_net_return_percent: -0.578 },
    ]);
    assert.ok(Math.abs((deltas.get('pair-1') ?? 0) - 0.031) < 1e-9);
  });

  it('uses the server-computed delta when a rejected side has no outcome row', () => {
    const deltas = pairedScoutDeltas([
      { stage: 'scout-outcome', scout_comparison_id: 'pair-2', scout_comparison_role: 'baseline', shadow_net_return_percent: -0.4, paired_delta_percent: 0.4 },
    ]);
    assert.equal(deltas.get('pair-2'), 0.4);
  });

  it('does not infer a delta from an incomplete pair without a server result', () => {
    const deltas = pairedScoutDeltas([
      { stage: 'scout-outcome', scout_comparison_id: 'pair-3', scout_comparison_role: 'baseline', shadow_net_return_percent: -0.4 },
    ]);
    assert.equal(deltas.has('pair-3'), false);
  });
});
