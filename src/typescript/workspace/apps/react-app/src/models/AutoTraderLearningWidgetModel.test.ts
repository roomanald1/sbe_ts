import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { AutoTraderModel } from './AutoTraderModel';
import type { EvidenceRequestsResponse } from './AutoTraderLearningWidgetModel';
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

  it('integrates live AutoTrader blockers with a stale decision history', async () => {
    const autoTrader = new AutoTraderModel();
    const initial = autoTrader.state$.value;
    const now = Date.now();
    autoTrader.state$.next({
      ...initial,
      enabled: true,
      lastDecisionAt: now,
      latestStrategyBlockers: [
        {
          strategy: 'live-order-book' as const,
          symbol: 'LINK-USDT',
          reason: 'Strength 0.12 < 0.30',
          timestamp_ms: now,
        },
      ],
    });
    let decisions = [
      {
        stage: 'strategy_rejection',
        timestamp: now - 6 * 60_000,
        status: 'rejected',
        reason: 'Signal strength below the entry floor',
      },
    ];
    const widget = new AutoTraderLearningWidgetModel(
      { state$: autoTrader.state$, start() {}, stop() {} },
      async () => [],
      30_000,
      async () => ({ decisions, errors: [] }),
    );
    await widget.refreshDecisions();

    assert.equal(widget.state$.value.decisionCoverage.stalled, true);
    assert.equal(widget.state$.value.decisions[0]?.stage, 'strategy_rejection');
    assert.equal(
      widget.state$.value.decisionCoverage.latestAuditAt,
      now - 6 * 60_000,
    );
    assert.equal(
      widget.state$.value.decisionCoverage.blockers[0]?.reason,
      'Strength 0.12 < 0.30',
    );
    decisions = [
      {
        stage: 'decision',
        timestamp: now - 60_000,
        status: 'skipped',
        reason: 'Current decision recorded',
      },
    ];
    await widget.refreshDecisions();
    assert.equal(widget.state$.value.decisionCoverage.stalled, false);
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

  it('loads evidence requests without converting unavailable coverage into zero', async () => {
    const autoTrader = new AutoTraderModel();
    const source = {
      state$: autoTrader.state$,
      start() {},
      stop() {},
    };
    const evidence: EvidenceRequestsResponse = {
      as_of_ms: 20_000,
      total_count: 1,
      omitted_count: 0,
      total_coverage_cohorts: 1,
      omitted_coverage_cohorts: 0,
      coverage_cohorts: [
        {
          symbol: 'BTC-USDT',
          strategy: 'live-momentum',
          profile_id: 'profile-1',
          regime: null,
          horizon: null,
          coverage_scope: 'symbol_strategy_profile_aggregate',
          coverage_status: 'missing',
          gap_reason: 'No eligible Scout outcomes are recorded.',
          sample_count: 0,
          effective_sample_size: 0,
          last_sample_timestamp_ms: null,
          last_sample_age_ms: null,
          regime_horizon_coverage: 'unavailable',
        },
      ],
      requests: [
        {
          request: {
            symbol: 'BTC-USDT',
            strategy: 'live-momentum',
            profile_id: 'profile-1',
            regime: 'trend',
            horizon: '5m',
            status: 'pending',
            attempts: 1,
            created_at_ms: 10_000,
            updated_at_ms: 11_000,
            next_attempt_at_ms: 12_000,
            reason: null,
          },
          as_of_ms: 20_000,
          coverage_scope: 'symbol_strategy_profile_regime_horizon',
          coverage_status: 'missing',
          gap_reason: 'No eligible Scout outcomes are recorded.',
          sample_count: 0,
          effective_sample_size: 0,
          last_sample_timestamp_ms: null,
          last_sample_age_ms: null,
          regime_horizon_coverage: 'measured',
        },
      ],
    };
    const model = new AutoTraderLearningWidgetModel(
      source,
      async () => [],
      30_000,
      async () => ({ decisions: [], errors: [] }),
      async () => evidence,
    );

    await model.refreshEvidenceRequests();

    assert.equal(model.state$.value.evidenceRequests?.requests.length, 1);
    assert.equal(
      model.state$.value.evidenceRequests?.requests[0]?.coverage_status,
      'missing',
    );
    assert.equal(
      model.state$.value.evidenceRequests?.coverage_cohorts[0]?.sample_count,
      0,
    );
    assert.equal(
      model.state$.value.evidenceRequests?.requests[0]
        ?.last_sample_timestamp_ms,
      null,
    );
    assert.equal(model.state$.value.evidenceRequestError, undefined);
  });

  it('shows evidence request transport errors separately from empty coverage', async () => {
    const autoTrader = new AutoTraderModel();
    const source = {
      state$: autoTrader.state$,
      start() {},
      stop() {},
    };
    const model = new AutoTraderLearningWidgetModel(
      source,
      async () => [],
      30_000,
      async () => ({ decisions: [], errors: [] }),
      async () => {
        throw new Error('Learning request failed (503).');
      },
    );

    await model.refreshEvidenceRequests();

    assert.equal(model.state$.value.evidenceRequests, null);
    assert.equal(
      model.state$.value.evidenceRequestError,
      'Learning request failed (503).',
    );
  });
});
