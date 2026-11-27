import { after, before, describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer, type ViteDevServer } from 'vite';
import { AutoTraderModel } from '../models/AutoTraderModel';
import { AutoTraderOutcomesModel } from '../models/AutoTraderOutcomesModel';
import { projectExitPolicyState } from '../models/AutoTraderExitPolicyModel';
import type {
  AutoTraderExitPolicyState,
  ExitPolicyAssessmentRow,
  ExitPositionRow,
  ExitProfileRow,
} from '../models/AutoTraderExitPolicyModel';
import {
  AutoTraderBootstrapFallbackSummary,
  AutoTraderGlobalControlsSummary,
} from './AutoTraderGlobalControlsSummary';

let viteServer: ViteDevServer;
let AutoTraderExitPoliciesPanel: typeof import('./AutoTraderExitPoliciesView.js').AutoTraderExitPoliciesPanel;

before(async () => {
  viteServer = await createServer({
    configFile: './vite.config.ts',
    server: { middlewareMode: true },
    appType: 'custom',
  });
  const module = await viteServer.ssrLoadModule(
    '/src/views/AutoTraderExitPoliciesView.tsx',
  );
  AutoTraderExitPoliciesPanel = module.AutoTraderExitPoliciesPanel;
});

after(async () => {
  await viteServer?.close();
});

function state(
  overrides: Partial<AutoTraderExitPolicyState> = {},
): AutoTraderExitPolicyState {
  const settings = new AutoTraderModel();
  settings.state$.next({
    ...settings.state$.value,
    loading: false,
    decisionParameters: {
      refreshed_at: 1_800_000_000_000,
      thresholds: [],
      active_exit_policy: { stop_loss_percent: 0.5, take_profit_percent: 0.8 },
      contexts: [],
      strategy_profiles: [],
    },
  });
  const outcomes = new AutoTraderOutcomesModel();
  outcomes.state$.next({ ...outcomes.state$.value, loading: false });
  const result = projectExitPolicyState(
    settings.state$.value,
    outcomes.state$.value,
    { strategy: 'all', symbol: 'all' },
    true,
  );
  settings.stop();
  outcomes.stop();
  return { ...result, ...overrides };
}

function position(overrides: Partial<ExitPositionRow> = {}): ExitPositionRow {
  return {
    id: 'open:BTC-USDT:trend-50-200',
    status: 'Open',
    symbol: 'BTC-USDT',
    strategy: 'trend-50-200',
    decisionId: 'decision-1',
    openedAt: 1_800_000_000_000,
    entryProfileId: 'entry-profile-1',
    entryProfileHash: 'sha256:v1:entry-profile',
    entryProfileVersion: 3,
    exitProfileId: 'exit-profile-4',
    exitProfileVersion: 4,
    exitPolicyAsOfMs: 1_800_000_000_000,
    executionMode: 'paper',
    entryPolicy: 'exploratory',
    lifecycleSource: 'auto_trader_entry_reservation',
    signalDirection: 'bullish',
    marketRegime: 'trend',
    selectedStopLossPercent: 0.42,
    selectedTakeProfitPercent: 1.1,
    selectedStopLossSource: 'validated_exit_policy',
    selectedTakeProfitSource: 'strategy_default_profile',
    selectedStopLossEvidenceSource:
      'activity.strategy_positions.selected_stop_loss_percent',
    selectedTakeProfitEvidenceSource:
      'activity.strategy_positions.selected_take_profit_percent',
    fallbackStopLossPercent: 0.5,
    fallbackTakeProfitPercent: 0.8,
    currentEffectiveStopLossPercent: 0.39,
    currentEffectiveTakeProfitPercent: 1.08,
    currentThresholdCompatibility: false,
    protectionAdjustments: [],
    stopLossActive: true,
    takeProfitActive: false,
    initialRiskEvidence: {
      eligibilityStatus: 'eligible_for_r_multiple',
      referenceStatus: 'complete',
      eligibleForRMultiple: true,
      quoteCurrency: 'USDT',
      quoteAmount: 5.4,
      stopLossPercent: 0.42,
      fillIdCount: 2,
    },
    ...overrides,
  };
}

function profile(overrides: Partial<ExitProfileRow> = {}): ExitProfileRow {
  return {
    id: 'BTC-USDT:trend-50-200',
    symbol: 'BTC-USDT',
    strategy: 'trend-50-200',
    learningSamples: 2,
    positiveRate: undefined,
    parameters: [],
    profileId: undefined,
    profileVersion: undefined,
    exchange: undefined,
    supportedDirection: undefined,
    regimeEvidenceKeys: [],
    finalHoldoutStatus: 'unavailable',
    historyCoverage: 'unavailable',
    costCoverage: 'unavailable',
    ...overrides,
  };
}

describe('AutoTraderExitPoliciesPanel', () => {
  it('shows source-backed strategy profile identity and explicit absence', () => {
    const html = renderToStaticMarkup(
      createElement(AutoTraderExitPoliciesPanel, {
        state: state({
          view: 'profiles',
          profileRows: [
            profile({ profileId: 'strategy-profile-9', profileVersion: 9 }),
            profile({
              id: 'ETH-USDT:mean-reversion',
              symbol: 'ETH-USDT',
              strategy: 'mean-reversion',
            }),
          ],
          filteredProfiles: [
            profile({ profileId: 'strategy-profile-9', profileVersion: 9 }),
            profile({
              id: 'ETH-USDT:mean-reversion',
              symbol: 'ETH-USDT',
              strategy: 'mean-reversion',
            }),
          ],
        }),
        onViewChange() {},
        onStrategyChange() {},
        onSymbolChange() {},
      }),
    );
    assert.match(
      html,
      /ID strategy-profile-9 · version 9 · source decision-parameter profile contract/,
    );
    assert.match(
      html,
      /ID Unavailable · version Unavailable · source profile identity fields absent/,
    );
  });

  it('separates exact profile identity, entry selection, fallback, effective values and eligible initial risk', () => {
    const row = position({
      maximumHoldingDurationMs: 86_400_000,
      maximumHoldingSource: 'validated_exit_policy',
      trailingActivationPercent: 0.8,
      trailingDistancePercent: 0.2,
      trailingSource: 'strategy_profile',
      breakevenActivationPercent: 0.4,
      breakevenSource: 'strategy_profile',
      profitProtectionActivationPercent: 0.6,
      profitProtectionGivebackPercent: 0.15,
      profitProtectionMinimumLockPercent: 0.1,
      profitProtectionSource: 'validated_exit_policy',
      currentPmBarrierId: 'barrier-4',
      currentPmBarrierTriggerPrice: 101.25,
      protectionAdjustments: [
        {
          id: 'pm-adjust-1',
          reason: 'trailing-stop',
          previousTriggerPrice: 100.5,
          newTriggerPrice: 101.25,
          source: 'validated_exit_policy',
          timestampMs: 1_800_000_002_000,
          recordSource: 'Position Management',
        },
      ],
    });
    const html = renderToStaticMarkup(
      createElement(AutoTraderExitPoliciesPanel, {
        state: state({
          view: 'positions',
          positionRows: [row],
          filteredPositions: [row],
        }),
        onViewChange() {},
        onStrategyChange() {},
        onSymbolChange() {},
      }),
    );
    assert.match(
      html,
      /Entry profile ID \/ version<\/dt><dd>entry-profile-1 \/ 3/,
    );
    assert.match(
      html,
      /Exit profile ID \/ version<\/dt><dd>exit-profile-4 \/ 4/,
    );
    assert.match(html, /Original selected stop<\/dt><dd>Active · 0\.420%/);
    assert.match(html, /validated_exit_policy/);
    assert.match(
      html,
      /Original selected target<\/dt><dd>Inactive · value 1\.100%/,
    );
    assert.match(
      html,
      /Bootstrap fallback stop \/ target<\/dt><dd>0\.500% \/ 0\.800%/,
    );
    assert.match(
      html,
      /AutoTrader monitor-driven IOC thresholds · not PM barrier<\/dt><dd>0\.390% \/ 1\.080%/,
    );
    assert.match(
      html,
      /Maximum holding duration<\/dt><dd>86400000 ms · source validated_exit_policy/,
    );
    assert.match(
      html,
      /Trailing stop activation \/ distance<\/dt><dd>0\.800% \/ 0\.200% · source strategy_profile/,
    );
    assert.match(
      html,
      /Breakeven activation<\/dt><dd>0\.400% · source strategy_profile/,
    );
    assert.match(
      html,
      /Profit protection activation \/ giveback \/ minimum lock<\/dt><dd>0\.600% \/ 0\.150% \/ 0\.100%/,
    );
    assert.match(
      html,
      /barrier-4 · trigger price 101\.2500000 · joined by exact profile ID\/version/,
    );
    assert.match(
      html,
      /100\.5000000 → 101\.2500000 · source validated_exit_policy/,
    );
    assert.match(html, /Position Management/);
    assert.match(html, /5\.400000 USDT · 2 linked entry fills/);
    assert.match(
      html,
      /Unavailable · activity\/outcome schema has no venue identifier/,
    );
  });

  it('does not infer selected values or initial risk from compatibility thresholds', () => {
    const row = position({
      entryProfileId: undefined,
      entryProfileVersion: undefined,
      exitProfileId: undefined,
      selectedStopLossPercent: undefined,
      selectedTakeProfitPercent: undefined,
      selectedStopLossSource: undefined,
      selectedTakeProfitSource: undefined,
      selectedStopLossEvidenceSource: undefined,
      selectedTakeProfitEvidenceSource: undefined,
      currentThresholdCompatibility: true,
      initialRiskEvidence: {
        eligibilityStatus: 'entry_fill_lineage_incomplete',
        referenceStatus: 'complete',
        eligibleForRMultiple: false,
        quoteCurrency: 'USDT',
        quoteAmount: undefined,
        fillIdCount: undefined,
      },
    });
    const html = renderToStaticMarkup(
      createElement(AutoTraderExitPoliciesPanel, {
        state: state({
          view: 'positions',
          positionRows: [row],
          filteredPositions: [row],
        }),
        onViewChange() {},
        onStrategyChange() {},
        onSymbolChange() {},
      }),
    );
    assert.match(html, /Original selected stop<\/dt><dd>Active · Unavailable/);
    assert.match(
      html,
      /AutoTrader monitor-driven IOC thresholds · not PM barrier<\/dt>/,
    );
    assert.match(
      html,
      /compatibility fields may include fallback or adjustment/,
    );
    assert.match(
      html,
      /Initial monetary risk<\/dt><dd>Unavailable · entry_fill_lineage_incomplete/,
    );
    assert.doesNotMatch(html, /5\.400000 USDT/);
    assert.match(
      html,
      /Current PM server-side protective barrier<\/dt><dd>Unavailable/,
    );
    assert.match(
      html,
      /no PM trigger_adjustments or AutoTrader protection_adjustments/,
    );
  });

  it('renders incomplete executable paths as unavailable rather than presenting raw excursions as complete', () => {
    const row = position({
      executablePricePathEvidence: {
        complete: false,
        grossExecutableMfePercent: 2.1,
        grossExecutableMaePercent: -1.3,
        retainedSampleCount: 5,
        totalSampleCount: 6,
        freshSampleCount: 4,
        missingCoverage: ['price_path_restart_interrupted'],
      },
    });
    const html = renderToStaticMarkup(
      createElement(AutoTraderExitPoliciesPanel, {
        state: state({
          view: 'positions',
          positionRows: [row],
          filteredPositions: [row],
        }),
        onViewChange() {},
        onStrategyChange() {},
        onSymbolChange() {},
      }),
    );
    assert.match(
      html,
      /Path incomplete · MFE Unavailable · incomplete path · MAE Unavailable · incomplete path/,
    );
    assert.match(html, /price_path_restart_interrupted/);
    assert.doesNotMatch(html, /2\.100%/);
    assert.doesNotMatch(html, /-1\.300%/);
  });

  it('shows raw profile values and explicit gaps for all unsupported Section 15 fields', () => {
    const raw = profile({
      parameters: [
        {
          key: 'stop_loss_percent',
          label: 'Stop loss',
          unit: '%',
          kind: 'stop',
          rawServiceValue: 0.42,
          configuredValue: undefined,
          defaultValue: 0.5,
          scoutVariantsAvailable: false,
          scoutVariants: [],
        },
      ],
      effectiveSampleSize: 2.5,
      oosFoldCount: 3,
      lastEvidenceSampleTimestampMs: 1_700_000_000_000,
      regimeEvidenceKeys: ['trend'],
      finalHoldoutStatus: 'not_validated',
      historyCoverage: 'partial',
      costCoverage: 'complete',
    });
    const html = renderToStaticMarkup(
      createElement(AutoTraderExitPoliciesPanel, {
        state: state({ profileRows: [raw], filteredProfiles: [raw] }),
        onViewChange() {},
        onStrategyChange() {},
        onSymbolChange() {},
      }),
    );
    assert.match(html, /raw profile value 0\.420 %/);
    assert.match(html, /Profile ID \/ version/);
    assert.match(html, /Supported direction/);
    assert.match(html, /Supported regime<\/dt><dd>trend/);
    assert.match(html, /Maximum holding duration/);
    assert.match(html, /Planned reward-to-risk/);
    assert.match(html, /Parameter source<\/dt><dd>Unavailable/);
    assert.match(html, /Effective sample size<\/dt><dd>2\.5/);
    assert.match(html, /Validation status<\/dt><dd>not validated/);
    assert.match(html, /Last evaluation<\/dt><dd>Unavailable/);
    assert.match(html, /Last promotion<\/dt><dd>Unavailable/);
    assert.doesNotMatch(html, /effective 0\.420%/);
  });

  it('renders read-only drill-down coverage and unavailable What-if/optimisation contracts', () => {
    const html = renderToStaticMarkup(
      createElement(AutoTraderExitPoliciesPanel, {
        state: state({ view: 'drilldown' }),
        onViewChange() {},
        onStrategyChange() {},
        onSymbolChange() {},
      }),
    );
    assert.match(html, /Previous and candidate values/);
    assert.match(html, /Executable excursion and post-exit paths/);
    assert.match(html, /What-if assumptions \/ results/);
    assert.match(html, /no read-only What-if API\/model contract/);
    assert.match(html, /Sensitivity heatmaps/);
    assert.match(html, /Winner \/ loser transitions/);
    assert.match(html, /Promotion history/);
    assert.match(html, /Accept \/ reject reasons/);
    assert.match(html, /never writes trading configuration/);
  });

  it('renders computed assessment metrics with coverage, blockers and non-active status', () => {
    const assessment: ExitPolicyAssessmentRow = {
      id: 'BTC-USDT:trend-50-200:1',
      symbol: 'BTC-USDT',
      strategy: 'trend-50-200',
      stopLossPercent: 0.42,
      takeProfitPercent: 1.1,
      activationPercent: 0.8,
      givebackPercent: 0.3,
      minimumLockPercent: 0.1,
      sampleCount: 18,
      trainingCount: 12,
      validationCount: 6,
      asOfMs: 1_800_000_000_000,
      promotionEligible: true,
      promotionBlockers: [],
      blockersAvailable: true,
      metrics: [
        {
          key: 'net_expectancy_percent',
          value: 0.27,
          coverageStatus: 'complete',
          sampleCount: 18,
          sourceFields: ['settled_trade.net_return_percent'],
          reason: 'complete source coverage',
        },
        {
          key: 'expectancy_r',
          value: undefined,
          coverageStatus: 'partial',
          sampleCount: 4,
          sourceFields: ['trade.realized_r'],
          reason: 'initial risk missing for other outcomes',
        },
        {
          key: 'max_drawdown_percent',
          value: undefined,
          coverageStatus: 'unavailable',
          sampleCount: 0,
          sourceFields: [],
          reason: 'incomplete portfolio interval history',
        },
      ],
      maximumHoldingDuration: {
        coverageStatus: 'missing',
        sourceFields: [],
        reason: 'No maximum_holding_duration_ms coverage record was returned.',
      },
    };
    const html = renderToStaticMarkup(
      createElement(AutoTraderExitPoliciesPanel, {
        state: state({
          view: 'assessments',
          assessmentStatus: 'available',
          assessmentError: undefined,
          assessments: [assessment],
          publishedPoliciesAvailable: true,
          publishedPolicies: [],
        }),
        onViewChange() {},
        onStrategyChange() {},
        onSymbolChange() {},
      }),
    );
    assert.match(html, /Assessment candidate · trend-50-200 · BTC-USDT/);
    assert.match(html, /ID Unavailable · version Unavailable/);
    assert.match(html, /Learning assessment snapshot available/);
    assert.match(html, /Eligible candidate · not published or active/);
    assert.match(html, /net_expectancy_percent/);
    assert.match(html, /0\.27/);
    assert.match(html, /settled_trade\.net_return_percent/);
    assert.match(html, /expectancy_r/);
    assert.match(html, /partial/);
    assert.match(html, /initial risk missing for other outcomes/);
    assert.match(html, /max_drawdown_percent/);
    assert.match(html, /Unavailable/);
    assert.match(html, /portfolio interval history/);
    assert.match(html, /Promotion eligibility/);
    assert.match(html, /not published or active/);
    assert.match(html, /Candidate maximum holding duration/);
    assert.match(
      html,
      /Unavailable · missing · No maximum_holding_duration_ms coverage record/,
    );
    assert.match(
      html,
      /AutoTrader active-policy application\/readback is unavailable/,
    );
  });

  it('renders a source-backed maximum-holding candidate only with metric coverage', () => {
    const assessment: ExitPolicyAssessmentRow = {
      id: 'BTC-USDT:trend-50-200:2',
      symbol: 'BTC-USDT',
      strategy: 'trend-50-200',
      asOfMs: 1_800_000_000_000,
      promotionEligible: false,
      blockersAvailable: true,
      promotionBlockers: ['candidate_not_promoted'],
      metrics: [],
      maximumHoldingDuration: {
        value: 86_400_000,
        coverageStatus: 'partial',
        sampleCount: 6,
        sourceFields: ['assessment.maximum_holding_duration_ms'],
        reason: 'partial holding-duration evidence coverage',
      },
    };
    const html = renderToStaticMarkup(
      createElement(AutoTraderExitPoliciesPanel, {
        state: state({
          view: 'assessments',
          assessmentStatus: 'available',
          assessments: [assessment],
        }),
        onViewChange() {},
        onStrategyChange() {},
        onSymbolChange() {},
      }),
    );
    assert.match(html, /Candidate maximum holding duration/);
    assert.match(
      html,
      /86400000 ms · partial · sources assessment\.maximum_holding_duration_ms/,
    );
    assert.match(html, /partial holding-duration evidence coverage/);
    assert.match(html, /Not promotion-eligible/);
  });

  it('renders supplied profile identity for a candidate and published policy without implying activation', () => {
    const assessment: ExitPolicyAssessmentRow = {
      id: 'BTC-USDT:trend-50-200:3',
      symbol: 'BTC-USDT',
      strategy: 'trend-50-200',
      profileId: 'exit-candidate-3',
      profileVersion: 3,
      trailingActivationPercent: 0.55,
      trailingDistancePercent: 0.18,
      trailingCoverageStatus: 'complete',
      breakevenActivationPercent: 0.3,
      breakevenCoverageStatus: 'available',
      promotionEligible: false,
      blockersAvailable: true,
      promotionBlockers: ['insufficient_evidence'],
      metrics: [],
    };
    const html = renderToStaticMarkup(
      createElement(AutoTraderExitPoliciesPanel, {
        state: state({
          view: 'assessments',
          assessmentStatus: 'available',
          assessments: [assessment],
          publishedPoliciesAvailable: true,
          publishedPolicies: [
            Object.assign(
              {
                symbol: 'ETH-USDT',
                strategy: 'mean-reversion',
                profile_id: 'exit-published-2',
                profile_version: 2,
                maximum_holding_duration_ms: 259_200_000,
                activation_percent: 0.8,
                giveback_percent: 0.3,
                minimum_lock_percent: 0.1,
                sample_count: 10,
                training_count: 7,
                validation_count: 3,
                baseline_validation_mean_percent: 0.1,
                selected_validation_mean_percent: 0.2,
                validation_gain_percent: 0.1,
                validation_positive_rate: 0.6,
                validation_negative_difference_rate: 0.2,
                validation_tenth_percentile_difference_percent: -0.1,
                validation_worst_difference_percent: -0.2,
                as_of_ms: 1_800_000_000_000,
              },
              {
                trailing_activation_percent: 0.6,
                trailing_distance_percent: 0.2,
                breakeven_activation_percent: 0.35,
              },
            ),
          ],
        }),
        onViewChange() {},
        onStrategyChange() {},
        onSymbolChange() {},
      }),
    );
    assert.match(html, /ID exit-candidate-3 · version 3/);
    assert.match(html, /Profile ID exit-published-2 · version 2/);
    assert.match(html, /maximum hold 259200000 ms/);
    assert.match(
      html,
      /Candidate trailing activation \/ distance<\/dt><dd>0\.550% \/ 0\.180% · complete · exact profile-version candidate/,
    );
    assert.match(
      html,
      /Candidate breakeven activation<\/dt><dd>0\.300% · exact profile-version candidate/,
    );
    assert.match(html, /trailing activation \/ distance 0\.600% \/ 0\.200%/);
    assert.match(html, /breakeven activation 0\.350%/);
    assert.match(
      html,
      /Learning publication is not proof of runtime activation/,
    );
  });

  it('shows partial trailing candidates without presenting them as a usable policy and keeps absent fields unavailable', () => {
    const html = renderToStaticMarkup(
      createElement(AutoTraderExitPoliciesPanel, {
        state: state({
          view: 'assessments',
          assessmentStatus: 'available',
          assessments: [
            {
              id: 'ETH-USDT:trend-50-200:4',
              symbol: 'ETH-USDT',
              strategy: 'trend-50-200',
              profileId: 'exit-profile-4',
              profileVersion: 4,
              trailingActivationPercent: 0.5,
              trailingCoverageStatus: 'partial',
              breakevenCoverageStatus: 'unavailable',
              metrics: [],
              promotionEligible: false,
              blockersAvailable: true,
              promotionBlockers: ['missing_trailing_distance'],
            },
          ],
        }),
        onViewChange() {},
        onStrategyChange() {},
        onSymbolChange() {},
      }),
    );
    assert.match(
      html,
      /0\.500% \/ Unavailable · partial · incomplete pair; not a usable trailing policy/,
    );
    assert.match(
      html,
      /Candidate breakeven activation<\/dt><dd>Unavailable · candidate field absent or exact profile identity missing/,
    );
    assert.match(html, /missing_trailing_distance/);
    assert.doesNotMatch(html, /0\.500% trailing policy active/);
  });

  it('does not replace an unavailable assessment contract with an empty-success message', () => {
    const html = renderToStaticMarkup(
      createElement(AutoTraderExitPoliciesPanel, {
        state: state({
          view: 'assessments',
          assessmentStatus: 'unavailable',
          assessmentError:
            'AutoTrader decision-parameter response omits exit_policy_assessments.',
          assessments: [],
        }),
        onViewChange() {},
        onStrategyChange() {},
        onSymbolChange() {},
      }),
    );
    assert.match(
      html,
      /Unavailable · AutoTrader decision-parameter response omits exit_policy_assessments/,
    );
    assert.doesNotMatch(
      html,
      /No candidate exit-policy assessments were returned/,
    );
  });

  it('renders filters from authoritative profile and position dimensions', () => {
    const html = renderToStaticMarkup(
      createElement(AutoTraderExitPoliciesPanel, {
        state: state({
          strategyOptions: ['trend-50-200'],
          symbolOptions: ['BTC-USDT'],
          strategyFilter: 'trend-50-200',
          symbolFilter: 'BTC-USDT',
        }),
        onViewChange() {},
        onStrategyChange() {},
        onSymbolChange() {},
      }),
    );
    assert.match(html, /Filter exit policies by strategy/);
    assert.match(html, /Filter exit policies by symbol/);
    assert.match(html, /value="trend-50-200"/);
    assert.match(html, /value="BTC-USDT"/);
  });
});

describe('AutoTrader global controls and fallback summary', () => {
  it('keeps fallback stop and target out of the primary global controls', () => {
    const settings = new AutoTraderModel();
    const html = renderToStaticMarkup(
      createElement(AutoTraderGlobalControlsSummary, {
        state: {
          ...settings.state$.value,
          availableNotional: 50,
          dailyNotionalLimit: '500',
          maxOrderNotional: '100',
          dailyRealizedLossLimit: '10',
          maxOpenPositions: '4',
          serverExplorationPercent: 2,
          stopLossPercent: 0.5,
          takeProfitPercent: 0.8,
        },
        onViewLearnedPolicies() {},
      }),
    );
    assert.match(html, /Global risk limits/);
    assert.match(html, /Daily loss limit/);
    assert.doesNotMatch(html, /Stop loss|Take profit|0\.50%|0\.80%/);
  });

  it('labels global exit settings as advanced fallbacks and withholds unloaded values', () => {
    const html = renderToStaticMarkup(
      createElement(AutoTraderBootstrapFallbackSummary, {
        stopLossPercent: 0.5,
        takeProfitPercent: 0.8,
        loading: false,
      }),
    );
    assert.match(html, /Advanced · bootstrap fallback thresholds/);
    assert.match(html, /Global fallback values only/);
    assert.match(html, /Fallback stop loss/);
    assert.match(html, /Fallback take profit/);
    assert.doesNotMatch(html, /Current stop loss|Active stop loss/);

    const loading = renderToStaticMarkup(
      createElement(AutoTraderBootstrapFallbackSummary, {
        stopLossPercent: 0.5,
        takeProfitPercent: 0.8,
        loading: true,
      }),
    );
    assert.match(loading, /Unavailable · settings not loaded/);
    assert.doesNotMatch(loading, /0\.50%|0\.80%/);
  });
});
