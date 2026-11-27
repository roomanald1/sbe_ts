import { after, before, describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer, type ViteDevServer } from 'vite';
import { AutoTraderModel } from '../models/AutoTraderModel';
import {
  projectLearningWidgetState,
  type EvidenceRequestsResponse,
} from '../models/AutoTraderLearningWidgetModel';
import type { LearningSummary } from '../models/AutoTraderOutcomesModel';
import { projectLearningEvidenceWidget } from '../models/LearningEvidenceWidgetModel';

let viteServer: ViteDevServer;
let LearningEvidenceView: typeof import('./LearningEvidenceWidget').LearningEvidenceView;

before(async () => {
  viteServer = await createServer({
    configFile: './vite.config.ts',
    server: { middlewareMode: true },
    appType: 'custom',
  });
  const module = await viteServer.ssrLoadModule(
    '/src/views/LearningEvidenceWidget.tsx',
  );
  LearningEvidenceView =
    module.LearningEvidenceView as typeof LearningEvidenceView;
});

after(async () => {
  await viteServer?.close();
});

function emptyEvidenceView() {
  const source = new AutoTraderModel().state$.value;
  return projectLearningEvidenceWidget(
    projectLearningWidgetState(source, null, undefined, false),
  );
}

function populatedEvidenceView() {
  const source = {
    ...new AutoTraderModel().state$.value,
    decisionParameters: {
      refreshed_at: 50_000,
      thresholds: [],
      contexts: [],
      strategy_profiles: [
        {
          symbol: 'BTC-USDT',
          strategy: 'live-momentum' as const,
          learning_samples: 0,
          realized_pnl: 0,
          parameters: [],
        },
        {
          symbol: 'ETH-USDT',
          strategy: 'live-momentum' as const,
          learning_samples: 0,
          realized_pnl: 0,
          parameters: [],
        },
      ],
      active_exit_policy: { stop_loss_percent: 1, take_profit_percent: 2 },
    },
  };
  const evidence: EvidenceRequestsResponse = {
    as_of_ms: 60_000,
    total_count: 1,
    omitted_count: 0,
    total_coverage_cohorts: 1,
    omitted_coverage_cohorts: 0,
    requests: [
      {
        request: {
          symbol: 'BTC-USDT',
          strategy: 'live-momentum',
          profile_id: 'profile-a',
          regime: 'trend',
          horizon: '5m',
          status: 'completed',
          attempts: 2,
          created_at_ms: 10_000,
          updated_at_ms: 55_000,
          next_attempt_at_ms: 0,
          reason: 'Collect additional paired Scout evidence',
        },
        as_of_ms: 60_000,
        coverage_scope: 'symbol_strategy_profile_regime_horizon',
        coverage_status: 'stale',
        gap_reason: 'Effective sample size remains below evaluator requirement',
        sample_count: 12,
        effective_sample_size: 4.5,
        last_sample_timestamp_ms: 30_000,
        last_sample_age_ms: 30_000,
        regime_horizon_coverage: 'available',
      },
    ],
    coverage_cohorts: [
      {
        symbol: 'BTC-USDT',
        strategy: 'live-momentum',
        profile_id: 'profile-a',
        regime: 'trend',
        horizon: '5m',
        coverage_scope: 'symbol_strategy_profile_regime_horizon',
        coverage_status: 'stale',
        gap_reason: 'Effective sample size remains below evaluator requirement',
        sample_count: 12,
        effective_sample_size: 4.5,
        last_sample_timestamp_ms: 30_000,
        last_sample_age_ms: 30_000,
        regime_horizon_coverage: 'available',
      },
    ],
  };
  const summary: LearningSummary[] = [
    {
      strategy: 'live-momentum',
      learning_samples: 99,
      scout_outcome_samples: 12,
      scout_positive_rate: 0.6,
      scout_average_net_return_percent: 0.3,
      active_profile_trade_samples: 2,
      active_profile_average_net_return_percent: 0.1,
      other_profile_trade_samples: 1,
      other_profile_average_net_return_percent: -0.2,
    },
  ];
  return projectLearningEvidenceWidget(
    projectLearningWidgetState(
      source,
      summary,
      undefined,
      false,
      [],
      undefined,
      evidence,
    ),
  );
}

describe('LearningEvidenceView', () => {
  it('renders missing contracts as unavailable rather than zero', () => {
    const view = emptyEvidenceView();
    assert.equal(view.profilesMonitored, null);
    assert.equal(view.strategiesMonitored, null);
    assert.equal(view.evidenceGaps, null);
    assert.equal(view.requestTotal, null);
    assert.equal(view.cohorts, null);
    assert.equal(view.requestStates, null);
    assert.equal(view.scoutEvidence, null);
    assert.equal(view.actualTradeEvidence, null);

    const evidenceMarkup = renderToStaticMarkup(
      createElement(LearningEvidenceView, { view, tab: 'evidence' }),
    );
    const promotionMarkup = renderToStaticMarkup(
      createElement(LearningEvidenceView, { view, tab: 'promotion' }),
    );
    assert.match(
      evidenceMarkup,
      /Sample counts and eligibility are not assumed to be zero/,
    );
    assert.match(
      promotionMarkup,
      /Accepted evidence gains and promotion history are unavailable/,
    );
    assert.doesNotMatch(promotionMarkup, /Actual promotion count: 0/);
  });

  it('separates evidence, activity, history and unavailable promotion views', () => {
    const view = populatedEvidenceView();
    assert.equal(view.profilesMonitored, 2);
    assert.equal(view.strategiesMonitored, 1);
    assert.equal(view.evidenceGaps, 1);
    assert.equal(view.requestStates?.[0]?.status, 'completed');
    assert.equal(view.cohorts?.[0]?.rawSamples, 12);
    assert.equal(view.cohorts?.[0]?.effectiveSamples, 4.5);
    assert.equal(view.scoutEvidence?.[0]?.samples, 12);
    assert.equal(view.actualTradeEvidence?.[0]?.activeSamples, 2);
    assert.equal(view.actualTradeEvidence?.[0]?.otherSamples, 1);

    const evidenceMarkup = renderToStaticMarkup(
      createElement(LearningEvidenceView, { view, tab: 'evidence' }),
    );
    const activityMarkup = renderToStaticMarkup(
      createElement(LearningEvidenceView, { view, tab: 'activity' }),
    );
    const historyMarkup = renderToStaticMarkup(
      createElement(LearningEvidenceView, { view, tab: 'history' }),
    );
    const promotionMarkup = renderToStaticMarkup(
      createElement(LearningEvidenceView, { view, tab: 'promotion' }),
    );
    assert.match(activityMarkup, /Completed; evidence acceptance is separate/);
    assert.match(evidenceMarkup, /Raw samples/);
    assert.match(evidenceMarkup, /Effective samples/);
    assert.match(historyMarkup, /12 reported Scout outcomes/);
    assert.match(
      historyMarkup,
      /aria-label="Profile-linked actual trading outcomes"/i,
    );
    assert.match(historyMarkup, /not cadence or accepted-gain measurements/);
    assert.doesNotMatch(historyMarkup, /99 Scout outcomes/);
    assert.match(
      promotionMarkup,
      /Accepted evidence gains and promotion history are unavailable/,
    );
  });
});
