import { afterEach, describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { LearningScoutActivityModel } from './LearningScoutActivityModel';
import { setLearningRpcRequesterForTests } from '../learning_rsocket';
import { setAutoTraderRpcRequesterForTests } from '../auto_trader_rsocket';

afterEach(() => {
  setLearningRpcRequesterForTests();
  setAutoTraderRpcRequesterForTests();
});

describe('LearningScoutActivityModel', () => {
  it('keeps active assignment status unavailable when the API omits it', async () => {
    const activity = {
      assignment_id: 'assignment-1',
      request_id: 'request-1',
      evaluated_at_ms: 1234,
      rank: 1,
      symbol: 'BTC-USD',
      strategy: 'momentum',
      decision: 'not_evaluated',
      mode: 'observe',
      candidate: {},
      evidence_sample_count: 0,
      rank_basis: 'decision_then_prior_scout_samples',
    };
    setLearningRpcRequesterForTests(async () =>
      Response.json({ items: [activity] }),
    );
    setAutoTraderRpcRequesterForTests(async () =>
      Response.json({ strategy_profiles: [] }),
    );

    const model = new LearningScoutActivityModel();
    model.start();
    await new Promise((resolve) => setTimeout(resolve, 0));
    model.stop();

    assert.deepEqual(model.state$.value.rows, [activity]);
    assert.deepEqual(model.state$.value.activeAssignments, []);
    assert.equal(model.state$.value.activeAssignmentsAvailable, false);
  });

  it('loads dispatched assignments and preserves explicit missing profiles', async () => {
    const assignments = [
      {
        assignment_id: 'assignment-with-profile',
        strategy: 'momentum',
        symbol: 'BTC-USD',
        mode: 'observe',
        lifecycle_status: 'assigned',
        assigned_at_ms: 1_000,
        elapsed_ms: 12_000,
        profile: {
          profile_id: 'profile-btc-momentum',
          version: 3,
          parameters: { lookback: 20 },
          learning_samples: 12,
          positive_samples: 7,
          realized_pnl: 4.5,
        },
      },
      {
        assignment_id: 'assignment-without-profile',
        strategy: 'mean_reversion',
        symbol: 'ETH-USD',
        mode: 'observe',
        lifecycle_status: 'assigned',
        assigned_at_ms: 2_000,
        elapsed_ms: 3_000,
        profile: null,
      },
    ];
    setLearningRpcRequesterForTests(async () =>
      Response.json({ items: [], active_assignments: assignments }),
    );
    setAutoTraderRpcRequesterForTests(async () =>
      Response.json({ strategy_profiles: [] }),
    );

    const model = new LearningScoutActivityModel();
    model.start();
    await new Promise((resolve) => setTimeout(resolve, 0));
    model.stop();

    assert.equal(model.state$.value.activeAssignmentsAvailable, true);
    assert.deepEqual(model.state$.value.activeAssignments, assignments);
    assert.equal(
      model.state$.value.activeAssignments[0].profile?.profile_id,
      'profile-btc-momentum',
    );
    assert.equal(model.state$.value.activeAssignments[1].profile, null);
  });
});
