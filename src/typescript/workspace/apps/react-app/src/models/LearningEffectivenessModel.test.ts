import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { summarizeLearningEffectiveness } from './LearningEffectivenessModel';

describe('summarizeLearningEffectiveness', () => {
  it('weights paired, closed-trade, and Scout returns by their samples', () => {
    const result = summarizeLearningEffectiveness([
      {
        strategy: 'live-momentum',
        learning_samples: 6,
        average_net_return_percent: -0.5,
        active_profile_trade_samples: 5,
        active_profile_average_net_return_percent: -0.3,
        active_profile_trade_positive_rate: 0.4,
        other_profile_trade_samples: 1,
        other_profile_average_net_return_percent: -1.5,
        other_profile_trade_positive_rate: 0,
        scout_outcome_samples: 3,
        scout_average_net_return_percent: 0.2,
        paired_scout_samples: 2,
        average_paired_scout_lift_percent: 1,
      },
      {
        strategy: 'live-order-book',
        learning_samples: 2,
        average_net_return_percent: 1,
        active_profile_trade_samples: 2,
        active_profile_average_net_return_percent: 1,
        active_profile_trade_positive_rate: 1,
        other_profile_trade_samples: 0,
        scout_outcome_samples: 1,
        scout_average_net_return_percent: -0.4,
        paired_scout_samples: 1,
        average_paired_scout_lift_percent: -1,
      },
    ]);

    assert.equal(result.pairedSamples, 3);
    assert.ok(Math.abs((result.pairedLift ?? 0) - 1 / 3) < 1e-12);
    assert.equal(result.closedTrades, 8);
    assert.equal(result.closedTradeReturn, -0.125);
    assert.equal(result.activeProfileTrades, 7);
    assert.ok(
      Math.abs((result.activeProfileReturn ?? 0) - 0.07142857142857142) < 1e-12,
    );
    assert.ok(
      Math.abs((result.activeProfilePositiveRate ?? 0) - 4 / 7) < 1e-12,
    );
    assert.equal(result.otherProfileTrades, 1);
    assert.equal(result.otherProfileReturn, -1.5);
    assert.equal(result.otherProfilePositiveRate, 0);
    assert.equal(result.scoutOutcomes, 4);
    assert.ok(Math.abs((result.scoutReturn ?? 0) - 0.05) < 1e-12);
  });

  it('does not fabricate averages when a metric has no valid samples', () => {
    const result = summarizeLearningEffectiveness([
      {
        strategy: 'live-momentum',
        learning_samples: 0,
        paired_scout_samples: 0,
        active_profile_trade_samples: 0,
        other_profile_trade_samples: 0,
      },
    ]);

    assert.equal(result.pairedSamples, 0);
    assert.equal(result.pairedLift, null);
    assert.equal(result.closedTrades, 0);
    assert.equal(result.closedTradeReturn, null);
    assert.equal(result.scoutOutcomes, 0);
    assert.equal(result.scoutReturn, null);
  });
});
