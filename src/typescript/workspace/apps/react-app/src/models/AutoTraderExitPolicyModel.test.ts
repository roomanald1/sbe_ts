import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { AutoTraderModel } from './AutoTraderModel';
import { AutoTraderOutcomesModel } from './AutoTraderOutcomesModel';
import type { AutoTraderOutcome } from './AutoTraderOutcomesModel';
import {
  AutoTraderExitPolicyModel,
  projectExitPolicyState,
} from './AutoTraderExitPolicyModel';
import type { AutoTraderDecisionParameters } from './AutoTraderModel';

const parameter = (key: string, value = 1) => ({
  key,
  label: key,
  value,
  learned_value: value,
  configured_value: null,
  default_value: value,
  min: 0,
  max: 10,
  step: 0.1,
  unit: '%',
  overridden: false,
  scout_variants: [],
});

function decisionParameters(): AutoTraderDecisionParameters {
  return {
    refreshed_at: 1_800_000_000_000,
    thresholds: [],
    active_exit_policy: { stop_loss_percent: 0.5, take_profit_percent: 0.8 },
    contexts: [
      {
        context: 'long:trend:BTC-USDT:trend-50-200',
        learning_samples: 3,
        positive_rate: undefined,
        scout_learning_samples: 5,
        scout_positive_rate: 0.6,
        strength_floor: 0.7,
        exploration_score_floor: 0.8,
        exploration_allowed: true,
      },
    ],
    strategy_profiles: [
      {
        symbol: 'BTC-USDT',
        strategy: 'trend-50-200',
        learning_samples: 3,
        realized_pnl: 0,
        walk_forward_evidence: {
          sampleCount: 4,
          effectiveSampleSize: 2.5,
          lastSampleTimestampMs: 1_700_000_000_000,
          finalHoldoutStatus: 'not_validated',
          historyCoverage: 'partial',
          costCoverage: 'complete',
        },
        walk_forward_evidence_by_regime: {
          trend: {
            sampleCount: 4,
            effectiveSampleSize: 2.5,
            lastSampleTimestampMs: 1_700_000_000_000,
            finalHoldoutStatus: 'not_validated',
          },
        },
        parameters: [
          parameter('entry_sma_period', 50),
          parameter('stop_loss_percent', 0.42),
        ],
      },
      {
        symbol: 'ETH-USDT',
        strategy: 'rsi2-oversold-rebound',
        learning_samples: 0,
        realized_pnl: 0,
        parameters: [parameter('rsi_period', 2)],
      },
    ],
  };
}

function outcome(
  overrides: Partial<AutoTraderOutcome> = {},
): AutoTraderOutcome {
  return {
    rowId: 'open:BTC-USDT:trend-50-200',
    status: 'Open',
    decisionId: 'decision-1',
    symbol: 'BTC-USDT',
    side: 'buy',
    openedAt: 1_800_000_000_000,
    quantity: 1,
    signedQuantity: 1,
    entryPrice: 100,
    pnl: 0,
    entryFees: 0,
    estimatedExitFeeRate: 0,
    estimatedExitSlippageBps: 0,
    pnlQuoteCurrency: 'USDT',
    strategy: 'trend-50-200',
    direction: 'bullish',
    strength: null,
    stopLossPercent: 0.5,
    takeProfitPercent: 0.8,
    currentEffectiveStopLossPercent: 0.5,
    currentEffectiveTakeProfitPercent: 0.8,
    currentThresholdCompatibility: true,
    scout: false,
    source: 'strict',
    components: {},
    ...overrides,
  };
}

describe('AutoTraderExitPolicyModel', () => {
  it('projects only declared strategy-profile identity fields', () => {
    const profile = {
      ...decisionParameters().strategy_profiles[0],
      profile_id: 'strategy-profile-9',
      profile_version: 9,
    };
    const state = projectExitPolicyState(
      {
        ...new AutoTraderModel().state$.value,
        decisionParameters: {
          ...decisionParameters(),
          strategy_profiles: [profile],
        },
      },
      new AutoTraderOutcomesModel().state$.value,
      { strategy: 'all', symbol: 'all' },
      true,
    );
    assert.equal(state.profileRows[0].profileId, 'strategy-profile-9');
    assert.equal(state.profileRows[0].profileVersion, 9);
  });

  it('keeps omitted Scout candidate diagnostics unavailable without crashing', () => {
    const profile = decisionParameters().strategy_profiles[0];
    profile.parameters = [
      { ...parameter('stop_loss_percent', 0.5), scout_variants: undefined },
    ];
    const state = projectExitPolicyState(
      {
        ...new AutoTraderModel().state$.value,
        decisionParameters: {
          ...decisionParameters(),
          strategy_profiles: [profile],
        },
      },
      {
        ...new AutoTraderOutcomesModel().state$.value,
        rows: [],
        closedLedgerRows: [],
      },
      { strategy: 'all', symbol: 'all' },
      true,
    );
    assert.equal(
      state.profileRows[0].parameters[0].scoutVariantsAvailable,
      false,
    );
    assert.deepEqual(state.profileRows[0].parameters[0].scoutVariants, []);
  });

  it('uses raw profile fields, exact entry identity and selected position thresholds', () => {
    const settings = new AutoTraderModel();
    settings.state$.next({
      ...settings.state$.value,
      loading: false,
      decisionParameters: decisionParameters(),
    });
    const outcomes = new AutoTraderOutcomesModel();
    outcomes.state$.next({
      ...outcomes.state$.value,
      loading: false,
      rows: [
        outcome({
          entryProfileId: 'entry-profile-1',
          entryProfileVersion: 3,
          exitProfileId: 'exit-profile-4',
          exitProfileVersion: 4,
          lifecycleSource: 'auto_trader_entry_reservation',
          signalDirection: 'bullish',
          marketRegime: 'trend',
          selectedStopLossPercent: 0.42,
          selectedTakeProfitPercent: 1.1,
          selectedStopLossSource: 'validated_exit_policy',
          selectedTakeProfitSource: 'strategy_default_profile',
          selectedStopLossEvidenceSource: 'activity.selected_stop_loss_percent',
          selectedTakeProfitEvidenceSource:
            'activity.selected_take_profit_percent',
          fallbackStopLossPercent: 0.5,
          fallbackTakeProfitPercent: 0.8,
          currentEffectiveStopLossPercent: 0.43,
          currentEffectiveTakeProfitPercent: 0.9,
          currentThresholdCompatibility: false,
          stopLossActive: true,
          takeProfitActive: false,
          initialRiskEvidence: {
            eligibilityStatus: 'eligible_for_r_multiple',
            referenceStatus: 'complete',
            eligibleForRMultiple: true,
            quoteCurrency: 'USDT',
            quoteAmount: 5.4,
            fillIdCount: 2,
          },
          exitParameterProvenance: {
            profile_id: 'exit-profile-4',
            profile_version: 4,
            maximum_holding_duration: {
              source: 'validated_exit_policy',
              selected_ms: 86_400_000,
            },
            trailing_protection: {
              source: 'strategy_profile',
              activation_percent: 0.8,
              distance_percent: 0.2,
            },
            breakeven_protection: {
              source: 'strategy_profile',
              activation_percent: 0.4,
            },
            profit_protection: {
              source: 'validated_exit_policy',
              activation_percent: 0.6,
              giveback_percent: 0.15,
              minimum_lock_percent: 0.1,
            },
            protection_adjustments: [
              {
                request_id: 'adjust-1',
                reason: 'trailing-stop',
                previous_trigger_price: 99,
                trigger_price: 100,
                policy_source: 'validated_exit_policy',
                updated_at_ms: 1_800_000_001_000,
              },
            ],
          },
          currentPmBarrier: {
            id: 'barrier-1',
            profile_id: 'exit-profile-4',
            profile_version: 4,
            trigger_price: 100,
            trigger_adjustments: [
              {
                request_id: 'adjust-1',
                reason: 'trailing-stop',
                expected_trigger_price: 99,
                new_trigger_price: 100,
                policy_source: 'validated_exit_policy',
                adjusted_at_ms: 1_800_000_001_000,
                profile_id: 'exit-profile-4',
                profile_version: 4,
              },
              {
                request_id: 'adjust-2',
                reason: 'breakeven-protection',
                expected_trigger_price: 100,
                new_trigger_price: 101,
                policy_source: 'validated_exit_policy',
                adjusted_at_ms: 1_800_000_002_000,
                profile_id: 'exit-profile-4',
                profile_version: 4,
              },
            ],
          },
        }),
        outcome({
          rowId: 'open:ETH-USDT:rsi2-oversold-rebound',
          symbol: 'ETH-USDT',
          strategy: 'rsi2-oversold-rebound',
          decisionId: 'decision-2',
        }),
      ],
    });

    const view = projectExitPolicyState(
      settings.state$.value,
      outcomes.state$.value,
      { strategy: 'trend-50-200', symbol: 'BTC-USDT' },
      true,
    );
    assert.equal(view.filteredProfiles.length, 1);
    assert.deepEqual(
      view.filteredProfiles[0].parameters.map((row) => row.key),
      ['stop_loss_percent'],
    );
    assert.equal(view.filteredProfiles[0].parameters[0].rawServiceValue, 0.42);
    assert.equal(view.filteredProfiles[0].effectiveSampleSize, 2.5);
    assert.equal(view.filteredProfiles[0].finalHoldoutStatus, 'not_validated');
    assert.deepEqual(view.filteredProfiles[0].regimeEvidenceKeys, ['trend']);
    assert.equal(view.filteredPositions.length, 1);
    assert.equal(view.filteredPositions[0].selectedStopLossPercent, 0.42);
    assert.equal(view.filteredPositions[0].selectedTakeProfitPercent, 1.1);
    assert.equal(
      view.filteredPositions[0].currentEffectiveStopLossPercent,
      0.43,
    );
    assert.equal(view.filteredPositions[0].entryProfileId, 'entry-profile-1');
    assert.equal(view.filteredPositions[0].entryProfileVersion, 3);
    assert.equal(view.filteredPositions[0].exitProfileId, 'exit-profile-4');
    assert.equal(view.filteredPositions[0].exitProfileVersion, 4);
    assert.equal(view.filteredPositions[0].signalDirection, 'bullish');
    assert.equal(view.filteredPositions[0].marketRegime, 'trend');
    assert.equal(
      view.filteredPositions[0].initialRiskEvidence?.eligibleForRMultiple,
      true,
    );
    assert.equal(
      view.filteredPositions[0].selectedTakeProfitSource,
      'strategy_default_profile',
    );
    assert.equal(
      view.filteredPositions[0].maximumHoldingDurationMs,
      86_400_000,
    );
    assert.equal(
      view.filteredPositions[0].maximumHoldingSource,
      'validated_exit_policy',
    );
    assert.equal(view.filteredPositions[0].trailingActivationPercent, 0.8);
    assert.equal(view.filteredPositions[0].trailingDistancePercent, 0.2);
    assert.equal(view.filteredPositions[0].breakevenActivationPercent, 0.4);
    assert.equal(
      view.filteredPositions[0].profitProtectionGivebackPercent,
      0.15,
    );
    assert.equal(view.filteredPositions[0].currentPmBarrierTriggerPrice, 100);
    assert.equal(view.filteredPositions[0].protectionAdjustments.length, 2);
    assert.equal(
      view.filteredPositions[0].protectionAdjustments[0].recordSource,
      'Position Management',
    );
    assert.equal(
      view.filteredPositions[0].protectionAdjustments[1].reason,
      'breakeven-protection',
    );
    settings.stop();
    outcomes.stop();
  });

  it('does not treat legacy effective position values or global fallbacks as selected policy', () => {
    const settings = new AutoTraderModel();
    settings.state$.next({
      ...settings.state$.value,
      loading: false,
      decisionParameters: decisionParameters(),
    });
    const outcomes = new AutoTraderOutcomesModel();
    outcomes.state$.next({
      ...outcomes.state$.value,
      loading: false,
      rows: [outcome()],
    });
    const view = projectExitPolicyState(
      settings.state$.value,
      outcomes.state$.value,
      { strategy: 'all', symbol: 'all' },
      false,
    );
    const unpersisted = view.positionRows[0];
    assert.equal(unpersisted.selectedStopLossPercent, undefined);
    assert.equal(unpersisted.selectedTakeProfitPercent, undefined);
    assert.equal(unpersisted.currentThresholdCompatibility, true);
    assert.equal(unpersisted.initialRiskEvidence, undefined);
    assert.equal(unpersisted.maximumHoldingDurationMs, undefined);
    assert.equal(unpersisted.trailingDistancePercent, undefined);
    assert.equal(unpersisted.currentPmBarrierTriggerPrice, undefined);
    assert.deepEqual(unpersisted.protectionAdjustments, []);
    assert.equal(
      view.profileRows.find((row) => row.strategy === 'rsi2-oversold-rebound')
        ?.parameters.length,
      0,
    );
    assert.equal(view.contexts[0].exitPolicyStatus, 'unavailable');
    settings.stop();
    outcomes.stop();
  });

  it('rejects a PM trigger whose profile identity does not match the open position', () => {
    const settings = new AutoTraderModel();
    const outcomes = new AutoTraderOutcomesModel();
    outcomes.state$.next({
      ...outcomes.state$.value,
      loading: false,
      rows: [
        outcome({
          exitProfileId: 'profile-a',
          exitProfileVersion: 2,
          currentPmBarrier: {
            id: 'barrier-wrong-profile',
            profile_id: 'profile-b',
            profile_version: 2,
            trigger_price: 91,
            trigger_adjustments: [
              {
                request_id: 'unrelated',
                reason: 'trailing-stop',
                expected_trigger_price: 90,
                new_trigger_price: 91,
                adjusted_at_ms: 1_800_000_002_000,
                profile_id: 'profile-b',
                profile_version: 2,
              },
            ],
          },
        }),
      ],
    });
    const view = projectExitPolicyState(
      settings.state$.value,
      outcomes.state$.value,
      { strategy: 'all', symbol: 'all' },
      true,
    );
    assert.equal(view.positionRows[0].currentPmBarrierTriggerPrice, undefined);
    assert.deepEqual(view.positionRows[0].protectionAdjustments, []);
    settings.stop();
    outcomes.stop();
  });

  it('projects Learning assessments without deriving metric values or promotion state', () => {
    const settings = new AutoTraderModel();
    const assessment = {
      symbol: 'BTC-USDT',
      strategy: 'trend-50-200',
      stop_loss_percent: 0.42,
      take_profit_percent: 1.1,
      activation_percent: 0.8,
      giveback_percent: 0.3,
      minimum_lock_percent: 0.1,
      sample_count: 18,
      training_count: 12,
      validation_count: 6,
      as_of_ms: 1_800_000_000_123,
      status: 'metrics_blocked',
      metrics: {
        values: {
          net_expectancy_percent: 0.27,
          expectancy_r: 0.54,
        },
        coverage: {
          net_expectancy_percent: {
            status: 'complete' as const,
            sample_count: 18,
            source_fields: ['settled_trade.net_return_percent'],
            reason: null,
          },
          expectancy_r: {
            status: 'partial' as const,
            sample_count: 4,
            source_fields: ['trade.realized_r'],
            reason: 'initial risk evidence is missing for other outcomes',
          },
          max_drawdown_percent: {
            status: 'unavailable' as const,
            sample_count: 0,
            source_fields: [],
            reason: 'insufficient contiguous portfolio intervals',
          },
        },
        promotion_eligible: false,
        promotion_blockers: [
          'portfolio_history_incomplete',
          'risk_coverage_partial',
        ],
      },
    };
    const params = {
      ...decisionParameters(),
      exit_policy_assessments: [assessment],
      exit_protection_policies: [],
    };
    settings.state$.next({
      ...settings.state$.value,
      loading: false,
      decisionParameters: params,
    });
    const outcomes = new AutoTraderOutcomesModel();
    outcomes.state$.next({ ...outcomes.state$.value, loading: false });
    const view = projectExitPolicyState(
      settings.state$.value,
      outcomes.state$.value,
      { strategy: 'all', symbol: 'all' },
      true,
      'assessments',
    );

    assert.equal(view.assessmentStatus, 'available');
    assert.equal(view.assessments.length, 1);
    assert.equal(view.assessments[0].metrics[0].key, 'expectancy_r');
    assert.equal(view.assessments[0].metrics[0].value, undefined);
    assert.equal(view.assessments[0].metrics[0].coverageStatus, 'partial');
    assert.equal(
      view.assessments[0].metrics[0].reason,
      'initial risk evidence is missing for other outcomes',
    );
    const missing = view.assessments[0].metrics.find(
      (row) => row.key === 'max_drawdown_percent',
    );
    assert.equal(missing?.value, undefined);
    assert.equal(missing?.coverageStatus, 'unavailable');
    assert.equal(view.assessments[0].promotionEligible, false);
    assert.deepEqual(view.assessments[0].promotionBlockers, [
      'portfolio_history_incomplete',
      'risk_coverage_partial',
    ]);
    assert.equal(view.publishedPoliciesAvailable, true);
    assert.equal(view.publishedPolicies.length, 0);
    assert.equal(view.activeAutoTraderPolicyStatus, 'unavailable');
    assert.equal(view.assessments[0].maximumHoldingDuration?.value, undefined);
    assert.equal(
      view.assessments[0].maximumHoldingDuration?.coverageStatus,
      'missing',
    );
    settings.stop();
    outcomes.stop();
  });

  it('projects maximum holding duration only when the candidate field has metric source coverage', () => {
    const settings = new AutoTraderModel();
    settings.state$.next({
      ...settings.state$.value,
      loading: false,
      decisionParameters: {
        ...decisionParameters(),
        exit_policy_assessments: [
          {
            symbol: 'BTC-USDT',
            strategy: 'trend-50-200',
            profile_id: 'candidate-profile-7',
            profile_version: 7,
            stop_loss_percent: 0.42,
            take_profit_percent: 1.1,
            activation_percent: 0.8,
            giveback_percent: 0.3,
            minimum_lock_percent: 0.1,
            maximum_holding_duration_ms: 86_400_000,
            sample_count: 12,
            training_count: 8,
            validation_count: 4,
            as_of_ms: 1_800_000_000_000,
            status: 'candidate',
            metrics: {
              values: {},
              coverage: {
                maximum_holding_duration_ms: {
                  status: 'complete',
                  sample_count: 12,
                  source_fields: ['assessment.maximum_holding_duration_ms'],
                  reason: 'candidate parameter source is complete',
                },
              },
              promotion_eligible: false,
              promotion_blockers: ['insufficient_holdout_evidence'],
            },
          },
        ],
      },
    });
    const outcomes = new AutoTraderOutcomesModel();
    const view = projectExitPolicyState(
      settings.state$.value,
      outcomes.state$.value,
      { strategy: 'all', symbol: 'all' },
      false,
      'assessments',
    );
    assert.equal(view.assessments[0].maximumHoldingDuration?.value, 86_400_000);
    assert.equal(view.assessments[0].profileId, 'candidate-profile-7');
    assert.equal(view.assessments[0].profileVersion, 7);
    assert.deepEqual(view.assessments[0].maximumHoldingDuration?.sourceFields, [
      'assessment.maximum_holding_duration_ms',
    ]);
    assert.equal(
      view.assessments[0].maximumHoldingDuration?.coverageStatus,
      'complete',
    );
    settings.stop();
    outcomes.stop();
  });

  it('projects trailing and breakeven candidates only with exact profile-version identity and preserves partial pairs', () => {
    const settings = new AutoTraderModel();
    const complete = Object.assign(
      {
        symbol: 'BTC-USDT',
        strategy: 'trend-50-200',
        profile_id: 'exit-profile-8',
        profile_version: 8,
        sample_count: 12,
        training_count: 8,
        validation_count: 4,
        as_of_ms: 1_800_000_000_000,
        status: 'candidate',
        metrics: {
          values: {},
          coverage: {},
          promotion_eligible: false,
          promotion_blockers: ['insufficient_evidence'],
        },
      },
      {
        trailing_activation_percent: 0.7,
        trailing_distance_percent: 0.2,
        breakeven_activation_percent: 0.3,
      },
    );
    const partial = Object.assign(
      {
        symbol: 'ETH-USDT',
        strategy: 'trend-50-200',
        profile_id: 'exit-profile-9',
        profile_version: 9,
        sample_count: 6,
        training_count: 4,
        validation_count: 2,
        as_of_ms: 1_800_000_000_001,
        status: 'candidate',
        metrics: {
          values: {},
          coverage: {},
          promotion_eligible: false,
          promotion_blockers: ['insufficient_evidence'],
        },
      },
      { trailing_activation_percent: 0.6 },
    );
    const unscoped = Object.assign(
      {
        symbol: 'SOL-USDT',
        strategy: 'trend-50-200',
        sample_count: 4,
        training_count: 3,
        validation_count: 1,
        as_of_ms: 1_800_000_000_002,
        status: 'candidate',
        metrics: {
          values: {},
          coverage: {},
          promotion_eligible: false,
          promotion_blockers: ['profile_identity_missing'],
        },
      },
      {
        trailing_activation_percent: 0.5,
        trailing_distance_percent: 0.15,
        breakeven_activation_percent: 0.25,
      },
    );
    settings.state$.next({
      ...settings.state$.value,
      loading: false,
      decisionParameters: {
        ...decisionParameters(),
        exit_policy_assessments: [complete, partial, unscoped],
      },
    });
    const outcomes = new AutoTraderOutcomesModel();
    const view = projectExitPolicyState(
      settings.state$.value,
      outcomes.state$.value,
      { strategy: 'all', symbol: 'all' },
      false,
      'assessments',
    );
    assert.deepEqual(
      [
        view.assessments[0].trailingActivationPercent,
        view.assessments[0].trailingDistancePercent,
        view.assessments[0].trailingCoverageStatus,
        view.assessments[0].breakevenActivationPercent,
        view.assessments[0].breakevenCoverageStatus,
      ],
      [0.7, 0.2, 'complete', 0.3, 'available'],
    );
    assert.equal(view.assessments[1].trailingActivationPercent, 0.6);
    assert.equal(view.assessments[1].trailingDistancePercent, undefined);
    assert.equal(view.assessments[1].trailingCoverageStatus, 'partial');
    assert.equal(view.assessments[1].breakevenCoverageStatus, 'unavailable');
    assert.equal(view.assessments[2].trailingActivationPercent, undefined);
    assert.equal(view.assessments[2].trailingDistancePercent, undefined);
    assert.equal(view.assessments[2].breakevenActivationPercent, undefined);
    assert.deepEqual(view.assessments[2].promotionBlockers, [
      'profile_identity_missing',
    ]);
    settings.stop();
    outcomes.stop();
  });

  it('exposes expectancy R only with complete authoritative initial-risk sources', () => {
    const settings = new AutoTraderModel();
    const assessment = {
      symbol: 'BTC-USDT',
      strategy: 'trend-50-200',
      stop_loss_percent: 0.42,
      take_profit_percent: 1.1,
      activation_percent: 0.8,
      giveback_percent: 0.3,
      minimum_lock_percent: 0.1,
      sample_count: 2,
      training_count: 1,
      validation_count: 1,
      as_of_ms: 100,
      status: 'metrics_blocked',
      metrics: {
        values: { expectancy_r: 0.3 },
        coverage: {
          expectancy_r: {
            status: 'complete' as const,
            sample_count: 2,
            source_fields: [
              'entry_context.initial_risk_reference.schema_version',
              'entry_context.initial_risk_reference.status',
              'entry_context.initial_risk_reference.quote_amount',
              'entry_context.initial_risk_reference.quote_currency',
              'entry_context.initial_risk_reference.fill_ids',
              'replayed_net_return_percent',
            ],
          },
        },
        promotion_eligible: false,
        promotion_blockers: [],
      },
    };
    settings.state$.next({
      ...settings.state$.value,
      loading: false,
      decisionParameters: {
        ...decisionParameters(),
        exit_policy_assessments: [assessment],
      },
    });
    const outcomes = new AutoTraderOutcomesModel();
    const view = projectExitPolicyState(
      settings.state$.value,
      outcomes.state$.value,
      { strategy: 'all', symbol: 'all' },
      false,
      'assessments',
    );
    assert.equal(view.assessments[0].metrics[0].value, 0.3);
    settings.stop();
    outcomes.stop();
  });

  it('marks absent assessments unavailable instead of treating them as an empty candidate set', () => {
    const settings = new AutoTraderModel();
    settings.state$.next({
      ...settings.state$.value,
      loading: false,
      decisionParameters: decisionParameters(),
    });
    const outcomes = new AutoTraderOutcomesModel();
    const view = projectExitPolicyState(
      settings.state$.value,
      outcomes.state$.value,
      { strategy: 'all', symbol: 'all' },
      false,
      'assessments',
    );
    assert.equal(view.assessmentStatus, 'unavailable');
    assert.match(view.assessmentError ?? '', /omits exit_policy_assessments/);
    assert.equal(view.assessments.length, 0);
    settings.stop();
    outcomes.stop();
  });

  it('owns start, stop, filter and view lifecycle without constructor effects', () => {
    const settings = new AutoTraderModel();
    const outcomes = new AutoTraderOutcomesModel();
    let starts = 0;
    let stops = 0;
    let refreshes = 0;
    settings.start = () => {
      starts += 1;
    };
    settings.stop = () => {
      stops += 1;
    };
    settings.refreshDecisionParameters = async () => {
      refreshes += 1;
    };
    outcomes.start = () => {
      starts += 1;
    };
    outcomes.stop = () => {
      stops += 1;
    };
    const model = new AutoTraderExitPolicyModel(settings, outcomes);

    assert.equal(starts, 0);
    model.start();
    model.start();
    assert.equal(starts, 2);
    assert.equal(refreshes, 1);
    model.setStrategyFilter('trend-50-200');
    model.setSymbolFilter('BTC-USDT');
    model.setView('positions');
    assert.equal(model.state$.value.view, 'positions');
    assert.equal(model.state$.value.strategyFilter, 'trend-50-200');
    model.stop();
    model.stop();
    assert.equal(stops, 2);
    assert.equal(model.state$.value.active, false);
  });
});
