import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { AutoTraderModel } from './AutoTraderModel';
import type { LearningSummary } from './AutoTraderOutcomesModel';
import {
  AutoTraderLearningWidgetModel,
  projectLearningWidgetState,
} from './AutoTraderLearningWidgetModel';

describe('AutoTraderLearningWidgetModel', () => {
  it('projects paired variant and open-scout rows into view state', () => {
    const initial = new AutoTraderModel().state$.value;
    const source = {
      ...initial,
      minimumScoutParameterSamples: '4',
      decisionParameters: {
        refreshed_at: 1,
        thresholds: [],
        contexts: [],
        strategy_profiles: [
          {
            symbol: 'BTC-USDT',
            strategy: 'live-momentum' as const,
            learning_samples: 0,
            realized_pnl: 0,
            parameters: [
              {
                key: 'minimum_strength',
                label: 'Minimum strength',
                value: 0.5,
                learned_value: 0.5,
                default_value: 0.4,
                min: 0,
                max: 1,
                step: 0.1,
                unit: 'score',
                overridden: false,
                scout_variants: [
                  {
                    candidate_value: 0.6,
                    sample_count: 2,
                    minimum_samples: 4,
                    average_net_improvement_percent: 0.2,
                    paired_baseline_unknown_samples: 2,
                    promoted: false,
                    rejected: false,
                  },
                ],
              },
            ],
          },
        ],
        active_exit_policy: { stop_loss_percent: 1, take_profit_percent: 2 },
      },
      pendingScoutParameters: [
        {
          symbol: 'BTC-USDT',
          strategy: 'live-momentum' as const,
          parameter: 'minimum_strength',
          candidateValue: 0.7,
          openSamples: 3,
        },
      ],
    };
    const projected = projectLearningWidgetState(
      source,
      null,
      undefined,
      false,
    );

    assert.equal(projected.rows.length, 2);
    assert.ok(Math.abs((projected.rows[0].pairedDelta ?? 0) - 0.2) < 1e-12);
    assert.equal(projected.rows[0].pairedDeltaEstimated, true);
    assert.equal(projected.rows[0].status, 'Collecting');
    assert.equal(projected.rows[1].status, 'Collecting · 3 open');
    assert.equal(projected.collectingCandidates, 2);
    assert.deepEqual(projected.visibleRows, projected.rows);
    assert.equal(
      projectLearningWidgetState(source, null, undefined, true).visibleRows
        .length,
      0,
    );
  });

  it('publishes summary refresh success and error state and can recover', async () => {
    const autoTrader = new AutoTraderModel();
    const source = {
      state$: autoTrader.state$,
      start() {},
      stop() {},
    };
    let shouldFail: boolean = true;
    const model = new AutoTraderLearningWidgetModel(
      source,
      async (): Promise<LearningSummary[]> => {
        if (shouldFail) throw new Error('temporary failure');
        return [
          {
            strategy: 'live-momentum',
            learning_samples: 1,
            average_net_return_percent: 0.5,
          },
        ];
      },
    );
    const emissions: Array<string | undefined> = [];
    const subscription = model.state$.subscribe((state) =>
      emissions.push(state.summaryError),
    );

    await model.refreshSummary();
    assert.equal(model.state$.value.summaryError, 'temporary failure');

    shouldFail = false;
    await model.refreshSummary();
    assert.equal(model.state$.value.summary?.length, 1);
    assert.equal(model.state$.value.summaryError, undefined);
    assert.ok(emissions.includes('temporary failure'));
    assert.equal(emissions.at(-1), undefined);
    subscription.unsubscribe();
  });
});
