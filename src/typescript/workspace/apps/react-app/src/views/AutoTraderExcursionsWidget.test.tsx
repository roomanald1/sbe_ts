import { after, before, describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { BehaviorSubject } from 'rxjs';
import { createServer, type ViteDevServer } from 'vite';
import {
  AutoTraderExcursionsModel,
  type TradePricePathPage,
  type WhatIfReplayState,
} from '../models/AutoTraderExcursionsModel';
import type {
  AutoTraderOutcome,
  AutoTraderOutcomesModel,
} from '../models/AutoTraderOutcomesModel';

let viteServer: ViteDevServer;
let ExcursionsPanel: typeof import('./AutoTraderExcursionsWidget').AutoTraderExcursionsPanel;

before(async () => {
  viteServer = await createServer({
    configFile: './vite.config.ts',
    server: { middlewareMode: true },
    appType: 'custom',
  });
  const module = await viteServer.ssrLoadModule(
    '/src/views/AutoTraderExcursionsWidget.tsx',
  );
  ExcursionsPanel = module.AutoTraderExcursionsPanel as typeof ExcursionsPanel;
});

after(async () => {
  await viteServer?.close();
});

function trade(
  id: string,
  extra: Partial<AutoTraderOutcome> = {},
): AutoTraderOutcome {
  return {
    rowId: id,
    tradeId: id,
    symbol: id === 'trade-btc' ? 'BTC-USDT' : 'ETH-USDT',
    strategy: 'momentum',
    source: 'strict',
    status: 'Closed',
    closedAt: Date.now(),
    executablePricePathEvidence: {
      complete: true,
      grossExecutableMfePercent: 0.42,
      grossExecutableMaePercent: -0.18,
    },
    ...extra,
  } as AutoTraderOutcome;
}

describe('Excursions tables', () => {
  it('renders populated overview and continuation rows through shared CompactTable', () => {
    const state$ = new BehaviorSubject({
      loading: false,
      rows: [
        trade('trade-btc', {
          netPnlUsd: 0.12,
          postExitObservedAt: Date.now(),
          postExitHorizonMs: 60_000,
          postExitBestReturnPercent: 0.3,
          postExitWorstReturnPercent: -0.1,
          postExitMissedReturnPercent: 0.18,
        }),
        trade('trade-eth', {
          netPnlUsd: -0.07,
          executablePricePathEvidence: {
            complete: false,
            missingCoverage: ['one-second sampling gap'],
          },
        }),
      ],
      closedLedgerRows: [] as AutoTraderOutcome[],
    });
    const outcomes = {
      state$,
      start() {},
      stop() {},
    } as unknown as AutoTraderOutcomesModel;
    const model = new AutoTraderExcursionsModel(
      outcomes,
      async () =>
        ({
          ok: true,
          json: async () => ({}),
        }) as Response,
    );
    model.start();
    model.selectTrade('trade-btc');

    const overview = renderToStaticMarkup(
      createElement(ExcursionsPanel, {
        model,
        state: model.state$.value,
      }),
    );
    assert.match(overview, /aria-label="Excursion trade overview"/);
    assert.match(overview, /operator-compact-table-scroll/);
    assert.match(
      overview,
      /<button[^>]*aria-label="Inspect BTC-USDT exit evidence"[^>]*aria-pressed="true"/,
    );
    assert.match(overview, /BTC-USDT/);
    assert.match(overview, /ETH-USDT/);
    assert.match(overview, /one-second sampling gap/);

    model.setView('continuation');
    const continuation = renderToStaticMarkup(
      createElement(ExcursionsPanel, {
        model,
        state: model.state$.value,
      }),
    );
    assert.match(
      continuation,
      /aria-label="Post-exit continuation observations"/,
    );
    assert.match(continuation, /60000\.000 ms/);
    assert.match(continuation, /0\.300%/);
    assert.match(continuation, /-0\.100%/);

    model.stop();
  });
});

describe('Excursions What-if presentation', () => {
  it('renders complete, unavailable-lineage, and bounded not-computed states', () => {
    const state$ = new BehaviorSubject({
      loading: false,
      rows: [
        trade('trade-btc', {
          exitParameterProvenance: {
            profile_id: 'captured-profile',
            profile_version: 2,
            maximum_holding_duration: { selected_days: 3 },
            strategy_parameter_specification: {
              schema_version: 1,
              maximum_hold_days: {
                key: 'maximum_hold_days',
                min: 1,
                max: 30,
                default: 10,
                step: 0.5,
                unit: 'days',
              },
            },
          } as unknown as AutoTraderOutcome['exitParameterProvenance'],
        }),
      ],
      closedLedgerRows: [] as AutoTraderOutcome[],
    });
    const model = new AutoTraderExcursionsModel(
      {
        state$,
        start() {},
        stop() {},
      } as unknown as AutoTraderOutcomesModel,
      async () => new Response('{}'),
      async () => new Response('{}'),
    );
    model.start();
    model.selectTrade('trade-btc');
    const replayPath: TradePricePathPage = {
      tradeId: 'trade-btc',
      coverageStatus: 'complete',
      historyComplete: true,
      totalSampleCount: 1,
      samples: [],
      segments: [],
      missingReasons: [],
      hasMore: false,
      replayEligibility: {
        status: 'eligible',
        requiredMaximumGapMs: 1_000,
        maximumObservedGapMs: 1_000,
      },
    };
    model.state$.next({
      ...model.state$.value,
      pathTradeId: 'trade-btc',
      pathPage: replayPath,
    });
    model.setView('what-if');
    model.setWhatIfParameter('stop_loss_percent', 0.5);
    model.setWhatIfParameter('take_profit_percent', 1);
    model.setWhatIfParameter('activation_percent', 0.5);
    model.setWhatIfParameter('giveback_percent', 0.2);
    model.setWhatIfParameter('minimum_lock_percent', 0.05);

    model.setWhatIfMaximumHoldDays(3.25);
    const invalidStepMarkup = renderToStaticMarkup(
      createElement(ExcursionsPanel, {
        model,
        state: model.state$.value,
      }),
    );
    assert.match(
      invalidStepMarkup,
      /Candidate maximum hold must match the captured strategy bounds and step/,
    );
    assert.match(invalidStepMarkup, /Run historical What-if<\/button>/);
    assert.match(invalidStepMarkup, /disabled=""/);
    model.setWhatIfMaximumHoldDays(3);

    const complete: WhatIfReplayState = {
      status: 'complete',
      tradeId: 'trade-btc',
      result: {
        tradeId: 'trade-btc',
        scope: 'research_only_historical_what_if',
        learningEligible: false,
        baselineNetReturnPercent: 0.12,
        candidateNetReturnPercent: 0.2,
        deltaNetReturnPercent: 0.08,
        durablePathSampleCount: 42,
        baselineMaximumHoldingDurationDays: 3,
        baselineMaximumHoldingDurationMs: 259_200_000,
        candidateMaximumHoldingDurationDays: 3,
        candidateMaximumHoldingDurationMs: 259_200_000,
        durationUnit: 'days',
        candidateExit: {
          timestampMs: 10_000,
          price: 102.5,
          reason: 'take_profit',
          quoteSourceRecordId: 'quote-77',
        },
        feeScheduleVersions: [{ id: 'pex-fees-v2' }],
      },
    };
    model.state$.next({ ...model.state$.value, whatIfReplay: complete });
    const completedMarkup = renderToStaticMarkup(
      createElement(ExcursionsPanel, {
        model,
        state: model.state$.value,
      }),
    );
    assert.match(
      completedMarkup,
      /Historical replay result · learning-ineligible/,
    );
    assert.match(completedMarkup, /0\.120%/);
    assert.match(completedMarkup, /0\.200%/);
    assert.match(completedMarkup, /0\.080%/);
    assert.match(completedMarkup, /Research only · Learning ineligible/);
    assert.match(completedMarkup, /Maximum hold candidate \(days\)/);
    assert.match(completedMarkup, /value="3"/);
    assert.match(completedMarkup, /step="0\.5"/);
    assert.match(completedMarkup, /Baseline maximum hold: 3 days/);
    assert.match(completedMarkup, /Candidate maximum hold: 3 days/);

    model.state$.next({
      ...model.state$.value,
      whatIfReplay: {
        status: 'unavailable',
        tradeId: 'trade-btc',
        reason: 'Captured policy lineage is unavailable for this trade.',
      },
    });
    const unavailableMarkup = renderToStaticMarkup(
      createElement(ExcursionsPanel, {
        model,
        state: model.state$.value,
      }),
    );
    assert.match(unavailableMarkup, /Replay unavailable/);
    assert.match(unavailableMarkup, /Captured policy lineage is unavailable/);

    model.state$.next({
      ...model.state$.value,
      whatIfReplay: {
        status: 'not_computed',
        tradeId: 'trade-btc',
        reason: 'replay_time_budget_exceeded',
        processedSampleCount: 500,
        totalSampleCount: 800,
        afterTimestampMs: 1234,
        afterCursor: 501,
      },
    });
    const partialMarkup = renderToStaticMarkup(
      createElement(ExcursionsPanel, {
        model,
        state: model.state$.value,
      }),
    );
    assert.match(partialMarkup, /Replay not computed/);
    assert.match(partialMarkup, /500 of 800 samples processed/);
    assert.match(partialMarkup, /cursor 501/);
    assert.doesNotMatch(partialMarkup, /Historical replay result/);

    model.state$.next({
      ...model.state$.value,
      view: 'replay',
      pathPage: {
        ...replayPath,
        coverageStatus: 'complete',
        historyComplete: true,
        totalSampleCount: 83,
        capturePolicy: {
          fineIntervalMs: 1_000,
          fineWindowMs: 1_800_000,
          coarseIntervalMs: 60_000,
          coarseAfterMs: 1_800_000,
          eventDrivenExtrema: true,
          eventDrivenThresholdCrossings: true,
        },
        lifetimeExcursionEvidence: {
          grossMfePercent: 2.4,
          grossMaePercent: -1.1,
          eventSampleCount: 9,
          eventMaximumGapMs: 1_000,
          complete: true,
          thresholdCrossings: [
            {
              thresholdType: 'take_profit',
              timestampMs: 20_000,
              thresholdPercent: 1,
              quoteSide: 'long_bid',
              closeMark: 103.5,
              quoteSourceSystem: 'paper_exchange.market_data_cache',
              quoteSourceEpoch: 'pex-epoch-3',
              quoteSourceCursor: 91,
              quoteSourceRecordId: 'quote-event-91',
            },
          ],
          missingReasons: [],
        },
        replayEligibility: {
          status: 'ineligible_coarse_sampling',
          requiredMaximumGapMs: 1_000,
          maximumObservedGapMs: 60_000,
          reason: 'coarse after the first 30 minutes',
        },
        horizonCoverage: [
          {
            horizon: 'fine_0_30m',
            offsetStartMs: 0,
            offsetEndMs: 1_800_000,
            targetIntervalMs: 1_000,
            sampleCount: 1_800,
            maximumObservedGapMs: 1_000,
            complete: true,
            horizonReached: true,
            missingReasons: [],
          },
          {
            horizon: 'coarse_after_30m',
            offsetStartMs: 1_800_000,
            offsetEndMs: 3_600_000,
            targetIntervalMs: 60_000,
            sampleCount: 31,
            maximumObservedGapMs: 60_000,
            complete: true,
            horizonReached: true,
            missingReasons: [],
          },
        ],
      },
    });
    const coarseLifetimeMarkup = renderToStaticMarkup(
      createElement(ExcursionsPanel, {
        model,
        state: model.state$.value,
      }),
    );
    assert.match(coarseLifetimeMarkup, /Lifetime event-driven gross excursion/);
    assert.match(
      coarseLifetimeMarkup,
      /Lifetime event-driven threshold crossings/,
    );
    assert.match(coarseLifetimeMarkup, /quote-event-91/);
    assert.match(coarseLifetimeMarkup, /2\.400%/);
    assert.match(coarseLifetimeMarkup, /-1\.100%/);
    assert.match(coarseLifetimeMarkup, /1000 ms through the first 30 min/);
    assert.match(coarseLifetimeMarkup, /60000 ms after 30 min/);
    assert.match(
      coarseLifetimeMarkup,
      /ineligible: coarse samples are lifetime context, not finer-resolution policy replay/,
    );
    assert.match(coarseLifetimeMarkup, /Observed path coverage by horizon/);
    assert.match(coarseLifetimeMarkup, /Fine · first 30m/);
    assert.match(coarseLifetimeMarkup, /Coarse · after 30m/);
    assert.match(coarseLifetimeMarkup, /1,800/);
    model.stop();

    const noSpecSource = new BehaviorSubject({
      loading: false,
      rows: [
        trade('trade-no-spec', {
          exitParameterProvenance: {
            profile_id: 'captured-profile',
            profile_version: 2,
          },
        }),
      ],
      closedLedgerRows: [] as AutoTraderOutcome[],
    });
    const noSpecModel = new AutoTraderExcursionsModel(
      {
        state$: noSpecSource,
        start() {},
        stop() {},
      } as unknown as AutoTraderOutcomesModel,
      async () => new Response('{}'),
    );
    noSpecModel.start();
    noSpecModel.selectTrade('trade-no-spec');
    noSpecModel.setView('what-if');
    const noSpecMarkup = renderToStaticMarkup(
      createElement(ExcursionsPanel, {
        model: noSpecModel,
        state: noSpecModel.state$.value,
      }),
    );
    assert.match(
      noSpecMarkup,
      /Disabled: captured schema-1 maximum_hold_days bounds are unavailable/,
    );
    assert.match(
      noSpecMarkup,
      /aria-label="Maximum hold candidate in days"[^>]*disabled=""/,
    );
    noSpecModel.stop();
  });
});
