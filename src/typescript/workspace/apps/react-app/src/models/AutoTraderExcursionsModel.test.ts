import assert from 'node:assert/strict';
import test from 'node:test';
import { BehaviorSubject } from 'rxjs';
import {
  EXCURSION_PATH_PAGE_SIZE,
  AutoTraderExcursionsModel,
  EXCURSION_EVIDENCE_NOTICE,
  excursionPanelStatus,
  WHAT_IF_CANDIDATE_POLICY_ID,
  WHAT_IF_CANDIDATE_POLICY_VERSION,
  parseTradePricePathPage,
  type TradePricePathPage,
} from './AutoTraderExcursionsModel';
import type { AutoTraderOutcome } from './AutoTraderOutcomesModel';

const row = (id: string, extra: Partial<AutoTraderOutcome> = {}) =>
  ({
    rowId: id,
    tradeId: id,
    strategy: 'momentum',
    source: 'strict',
    status: 'Closed',
    closedAt: Date.now(),
    ...extra,
  }) as AutoTraderOutcome;

test('discloses surfaced records are non-empirical and bars performance or promotion claims', () => {
  assert.match(EXCURSION_EVIDENCE_NOTICE, /non-empirical/);
  assert.match(
    EXCURSION_EVIDENCE_NOTICE,
    /natural-lifecycle, profitability, Learning, or promotion/,
  );
});

test('each research panel reports its own evidence limits without synthetic results', () => {
  assert.match(
    excursionPanelStatus('what-if', 24, 0, 0),
    /research-only historical replay/i,
  );
  assert.match(
    excursionPanelStatus('continuation', 24, 0, 0),
    /no post-exit observation/,
  );
  assert.match(
    excursionPanelStatus('transitions', 24, 0, 0),
    /complete-window denominators/,
  );
  assert.match(
    excursionPanelStatus('sensitivity', 24, 0, 0),
    /no replay-backed/,
  );
  assert.match(
    excursionPanelStatus('replay', 24, 0, 0),
    /no complete executable paths/,
  );
  assert.equal(
    excursionPanelStatus('overview', 0, 0, 0),
    'No records match the selected filters.',
  );
});

function fixture(rows: AutoTraderOutcome[]) {
  const state$ = new BehaviorSubject({
    loading: false,
    rows,
    closedLedgerRows: [] as AutoTraderOutcome[],
  });
  let starts = 0;
  let stops = 0;
  const source = {
    state$,
    start: () => {
      starts++;
    },
    stop: () => {
      stops++;
    },
  };
  return { state$, source: source as never, counts: () => [starts, stops] };
}

test('projects source-backed complete paths and leaves incomplete metrics unavailable', () => {
  const complete = row('complete', {
    executablePricePathEvidence: {
      complete: true,
      grossExecutableMfePercent: 1.2,
      grossExecutableMaePercent: -0.4,
    },
    initialRiskEvidence: {
      eligibleForRMultiple: true,
      quoteAmount: 20,
    } as never,
  });
  const incomplete = row('partial', {
    executablePricePathEvidence: {
      complete: false,
      grossExecutableMfePercent: 9,
    },
  });
  const f = fixture([complete, incomplete]);
  const model = new AutoTraderExcursionsModel(f.source);
  model.start();
  assert.equal(model.state$.value.sourceCount, 2);
  assert.equal(model.state$.value.eligiblePathCount, 1);
  assert.equal(
    model.state$.value.trades.find((t) => t.id === 'partial')?.grossMfePercent,
    undefined,
  );
  assert.equal(model.state$.value.whatIfAvailable, true);
  model.stop();
  assert.deepEqual(f.counts(), [1, 1]);
});

test('classifies incomplete path records only from source-provided coverage', () => {
  const reported = row('reported', {
    executablePricePathEvidence: {
      complete: false,
      missingCoverage: ['pre-entry boundary missing', 'sampling gap'],
    },
  });
  const unspecified = row('unspecified', {
    executablePricePathEvidence: { complete: false },
  });
  const absent = row('absent');
  const f = fixture([reported, unspecified, absent]);
  const model = new AutoTraderExcursionsModel(f.source);
  model.start();
  assert.deepEqual(
    model.state$.value.trades.map((trade) => trade.incompletePathReason),
    [
      'pre-entry boundary missing, sampling gap',
      'Source reports an incomplete path without a specific reason.',
      'No executable path evidence was returned for this record.',
    ],
  );
  model.stop();
});

test('orders threshold crossing events chronologically without changing source values', () => {
  const trade = row('crossings', {
    executablePricePathEvidence: {
      complete: false,
      thresholdCrossingEvidenceStatus: 'observed_with_unverified_fee_schedule',
      thresholdCrossings: [
        { thresholdType: 'take_profit', timestampMs: 2_000 },
        { thresholdType: 'stop_loss', timestampMs: 1_000 },
      ],
    },
  });
  const f = fixture([trade]);
  const model = new AutoTraderExcursionsModel(f.source);
  model.start();
  const projected = model.state$.value.trades[0];
  assert.ok(projected);
  assert.deepEqual(
    projected.thresholdCrossings.map((crossing) => crossing.timestampMs),
    [1_000, 2_000],
  );
  assert.equal(
    projected.thresholdCrossingEvidenceStatus,
    'observed_with_unverified_fee_schedule',
  );
  model.stop();
});

function pathPayload(
  tradeId: string,
  samples: Array<{ cursor: number; timestamp: number }>,
  hasMore = false,
) {
  const last = samples.at(-1);
  return {
    schema_version: 1,
    trade_id: tradeId,
    coverage_status: 'incomplete',
    history_complete: false,
    total_sample_count: 3,
    segments: [
      {
        path_epoch: 'path-epoch-1',
        first_sequence: 1,
        last_sequence: 3,
        sample_count: 3,
        first_timestamp_ms: 1_000,
        last_timestamp_ms: 3_000,
        maximum_gap_ms: 1_000,
        complete: false,
        missing_reasons: ['post_exit_window_pending'],
      },
    ],
    missing_reasons: ['post_exit_window_pending'],
    samples: samples.map(({ cursor, timestamp }, index) => ({
      cursor,
      path_epoch: 'path-epoch-1',
      sequence: index + cursor,
      timestamp_ms: timestamp,
      sample: {
        timestamp,
        bid: 99 + index,
        ask: 101 + index,
        sequence: index + cursor,
        epoch: 'path-epoch-1',
        source_fresh: true,
        quote_source_system: 'paper_exchange.market_data_cache',
        quote_source_epoch: 'pex-epoch-1',
        quote_source_cursor: cursor + 100,
        quote_source_record_id: `quote-${cursor}`,
      },
    })),
    next_timestamp_ms: hasMore ? last?.timestamp : null,
    next_cursor: hasMore ? last?.cursor : null,
    has_more: hasMore,
  };
}

test('parses canonical chronological path samples and keeps incomplete history incomplete', () => {
  const parsed = parseTradePricePathPage(
    pathPayload('trade-1', [{ cursor: 1, timestamp: 1_000 }], true),
    'trade-1',
  );
  assert.ok(parsed);
  assert.equal(parsed.historyComplete, false);
  assert.deepEqual(parsed.missingReasons, ['post_exit_window_pending']);
  assert.equal(parsed.samples[0].quoteSourceRecordId, 'quote-1');
  assert.equal(parsed.samples[0].timestampMs, 1_000);
  assert.equal(
    parseTradePricePathPage(pathPayload('other', [], false), 'trade-1'),
    null,
  );
});

test('keeps lifetime excursion completeness separate from coarse-sampled replay eligibility', () => {
  const payload = {
    schema_version: 1,
    trade_id: 'lifetime-trade',
    coverage_status: 'complete',
    history_complete: true,
    total_sample_count: 23_483,
    segments: [
      {
        path_epoch: 'path-epoch-1',
        first_timestamp_ms: 1_000,
        last_timestamp_ms: 1_296_000_000,
        sample_count: 23_483,
        maximum_gap_ms: 60_000,
        complete: true,
        missing_reasons: [],
      },
    ],
    missing_reasons: [],
    samples: [],
    has_more: false,
    capture_policy: {
      fine_interval_ms: 1_000,
      fine_window_ms: 1_800_000,
      coarse_interval_ms: 60_000,
      coarse_after_ms: 1_800_000,
      event_driven_extrema: true,
      event_driven_threshold_crossings: true,
    },
    lifetime_excursion_evidence: {
      gross_mfe_percent: 2.4,
      gross_mae_percent: -1.1,
      event_sample_count: 9,
      event_maximum_gap_ms: 1_000,
      complete: true,
      threshold_crossings: [
        { threshold_type: 'take_profit', timestamp_ms: 2_000 },
        { threshold_type: 'stop_loss', timestamp_ms: 1_000 },
      ],
    },
    replay_eligibility: {
      status: 'ineligible_coarse_sampling',
      required_maximum_gap_ms: 1_000,
      maximum_observed_gap_ms: 60_000,
      reason:
        'Coarse lifetime tail is not eligible for finer-resolution replay.',
    },
    horizon_coverage: [
      {
        horizon: 'fine_0_30m',
        offset_start_ms: 0,
        offset_end_ms: 1_800_000,
        target_interval_ms: 1_000,
        sample_count: 1_800,
        maximum_observed_gap_ms: 1_000,
        complete: true,
        horizon_reached: true,
        missing_reasons: [],
      },
      {
        horizon: 'coarse_after_30m',
        offset_start_ms: 1_800_000,
        offset_end_ms: 1_296_000_000,
        target_interval_ms: 60_000,
        sample_count: 21_683,
        maximum_observed_gap_ms: 60_000,
        complete: true,
        horizon_reached: true,
        missing_reasons: [],
      },
    ],
  };
  const page = parseTradePricePathPage(payload, 'lifetime-trade');
  assert.ok(page);
  assert.equal(page.historyComplete, true);
  assert.equal(page.lifetimeExcursionEvidence?.complete, true);
  assert.equal(page.lifetimeExcursionEvidence?.grossMfePercent, 2.4);
  assert.equal(page.lifetimeExcursionEvidence?.grossMaePercent, -1.1);
  assert.deepEqual(
    page.lifetimeExcursionEvidence?.thresholdCrossings.map(
      (crossing) => crossing.timestampMs,
    ),
    [1_000, 2_000],
  );
  assert.equal(page.replayEligibility?.status, 'ineligible_coarse_sampling');
  assert.equal(page.replayEligibility?.maximumObservedGapMs, 60_000);
  assert.equal(page.capturePolicy?.fineIntervalMs, 1_000);
  assert.equal(page.capturePolicy?.coarseIntervalMs, 60_000);
  assert.deepEqual(
    page.horizonCoverage?.map((horizon) => [
      horizon.horizon,
      horizon.sampleCount,
      horizon.complete,
    ]),
    [
      ['fine_0_30m', 1_800, true],
      ['coarse_after_30m', 21_683, true],
    ],
  );
  assert.equal(page.totalSampleCount, 23_483);
  assert.equal(
    page.horizonCoverage?.[1]?.offsetEndMs,
    15 * 24 * 60 * 60 * 1_000,
  );
  const replayUnavailable = parseTradePricePathPage(
    {
      ...payload,
      replay_eligibility: {
        status: 'unavailable',
        required_maximum_gap_ms: 1_000,
        maximum_observed_gap_ms: 60_000,
        reason: 'entry_timestamp_unavailable',
      },
    },
    'lifetime-trade',
  );
  assert.equal(replayUnavailable?.replayEligibility?.status, 'unavailable');
  const unknownEntry = parseTradePricePathPage(
    {
      ...payload,
      coverage_status: 'no_durable_path',
      history_complete: false,
      total_sample_count: 0,
      segments: [],
      samples: [],
      lifetime_excursion_evidence: {
        gross_mfe_percent: null,
        gross_mae_percent: null,
        event_sample_count: 0,
        event_maximum_gap_ms: null,
        complete: false,
        threshold_crossings: [],
        missing_reasons: ['lifetime_event_summary_unavailable'],
      },
      replay_eligibility: {
        status: 'unavailable',
        required_maximum_gap_ms: 1_000,
        maximum_observed_gap_ms: null,
        reason: 'no_durable_path',
      },
      horizon_coverage: [
        {
          horizon: 'fine_0_30m',
          offset_start_ms: 0,
          offset_end_ms: 0,
          target_interval_ms: 1_000,
          sample_count: 0,
          maximum_observed_gap_ms: null,
          complete: false,
          horizon_reached: false,
          missing_reasons: ['entry_timestamp_unavailable'],
        },
      ],
    },
    'lifetime-trade',
  );
  assert.equal(unknownEntry?.lifetimeExcursionEvidence?.complete, false);
  assert.equal(unknownEntry?.replayEligibility?.status, 'unavailable');
  assert.equal(
    unknownEntry?.replayEligibility?.maximumObservedGapMs,
    undefined,
  );
  assert.equal(unknownEntry?.horizonCoverage?.[0]?.offsetStartMs, undefined);
  assert.deepEqual(unknownEntry?.horizonCoverage?.[0]?.missingReasons, [
    'entry_timestamp_unavailable',
  ]);
});

test('loads and pages selected trade paths through the stable-ID endpoint', async () => {
  const requested: string[] = [];
  const sourceFixture = fixture([row('trade-1')]);
  const model = new AutoTraderExcursionsModel(
    sourceFixture.source,
    async (path) => {
      requested.push(path);
      const pageNumber = requested.length;
      return new Response(
        JSON.stringify(
          pageNumber === 1
            ? pathPayload('trade-1', [{ cursor: 1, timestamp: 1_000 }], true)
            : pathPayload('trade-1', [{ cursor: 2, timestamp: 2_000 }], false),
        ),
        { status: 200 },
      );
    },
  );
  model.start();
  model.showTradePath('trade-1');
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(model.state$.value.pathPage?.samples.length, 1);
  assert.equal(model.state$.value.pathPage?.historyComplete, false);
  await model.loadMorePath();
  assert.equal(model.state$.value.pathPage?.samples.length, 2);
  assert.equal(model.state$.value.pathPage?.samples[1].timestampMs, 2_000);
  assert.equal(requested.length, 2);
  assert.match(requested[0], /trade_id=trade-1/);
  assert.match(requested[0], new RegExp(`limit=${EXCURSION_PATH_PAGE_SIZE}`));
  assert.match(requested[1], /after_timestamp_ms=1000/);
  assert.match(requested[1], /after_cursor=1/);
  model.stop();
});

test('ignores a late path response after selection changes', async () => {
  let releaseFirst: ((response: Response) => void) | undefined;
  const sourceFixture = fixture([row('trade-1'), row('trade-2')]);
  const model = new AutoTraderExcursionsModel(sourceFixture.source, (path) => {
    if (path.includes('trade_id=trade-1'))
      return new Promise((resolve) => {
        releaseFirst = resolve;
      });
    return Promise.resolve(
      new Response(
        JSON.stringify(
          pathPayload('trade-2', [{ cursor: 2, timestamp: 2_000 }]),
        ),
        { status: 200 },
      ),
    );
  });
  model.start();
  model.showTradePath('trade-1');
  model.selectTrade('trade-2');
  model.setView('replay');
  await new Promise((resolve) => setTimeout(resolve, 0));
  releaseFirst?.(
    new Response(
      JSON.stringify(pathPayload('trade-1', [{ cursor: 1, timestamp: 1_000 }])),
      { status: 200 },
    ),
  );
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(model.state$.value.pathPage?.tradeId, 'trade-2');
  model.stop();
});

test('keeps path selection, filters, and fetch state independent per instance', () => {
  const rows = [row('autotrader'), row('scout', { source: 'scout' })];
  const firstSource = fixture(rows);
  const secondSource = fixture(rows);
  let firstRequests = 0;
  let secondRequests = 0;
  const first = new AutoTraderExcursionsModel(firstSource.source, async () => {
    firstRequests++;
    return new Response(JSON.stringify(pathPayload('autotrader', [])));
  });
  const second = new AutoTraderExcursionsModel(
    secondSource.source,
    async () => {
      secondRequests++;
      return new Response(JSON.stringify(pathPayload('autotrader', [])));
    },
  );
  first.start();
  second.start();
  first.setSource('scouts');
  assert.deepEqual(
    first.state$.value.trades.map((trade) => trade.id),
    ['scout'],
  );
  assert.deepEqual(
    second.state$.value.trades.map((trade) => trade.id),
    ['autotrader', 'scout'],
  );
  first.selectTrade('scout');
  assert.equal(first.state$.value.selectedTradeId, 'scout');
  assert.equal(second.state$.value.selectedTradeId, undefined);
  assert.equal(firstRequests, 0);
  assert.equal(secondRequests, 0);
  first.stop();
  second.stop();
});

test('does not request a path from a presentation row ID when canonical trade ID is absent', () => {
  const f = fixture([row('legacy-row', { tradeId: undefined })]);
  let requests = 0;
  const model = new AutoTraderExcursionsModel(f.source, async () => {
    requests++;
    return new Response('{}');
  });
  model.start();
  model.showTradePath('legacy-row');
  assert.equal(requests, 0);
  assert.match(model.state$.value.pathError ?? '', /stable canonical trade ID/);
  model.stop();
});

test('filters by source, range and strategy, and clears stale selection', () => {
  const recent = row('recent');
  const old = row('old', { closedAt: Date.now() - 100 * 86_400_000 });
  const scout = row('scout', { source: 'scout', strategy: 'donchian' });
  const f = fixture([recent, old, scout]);
  const model = new AutoTraderExcursionsModel(f.source);
  model.start();
  assert.deepEqual(model.state$.value.strategyOptions, [
    'donchian',
    'momentum',
  ]);
  model.selectTrade('old');
  model.setRange('30d');
  assert.deepEqual(model.state$.value.strategyOptions, [
    'donchian',
    'momentum',
  ]);
  assert.deepEqual(
    model.state$.value.trades.map((t) => t.id),
    ['recent', 'scout'],
  );
  assert.equal(model.state$.value.selectedTradeId, undefined);
  model.setSource('scouts');
  assert.deepEqual(model.state$.value.strategyOptions, ['donchian']);
  assert.deepEqual(
    model.state$.value.trades.map((t) => t.id),
    ['scout'],
  );
  model.setStrategy('missing');
  assert.equal(model.state$.value.sourceCount, 0);
  model.stop();
});

test('repeated activation is idempotent and source updates reproject', () => {
  const f = fixture([row('one')]);
  const model = new AutoTraderExcursionsModel(f.source);
  model.start();
  model.start();
  assert.deepEqual(f.counts(), [1, 0]);
  f.state$.next({ loading: true, rows: [row('two')], closedLedgerRows: [] });
  assert.equal(model.state$.value.loading, true);
  assert.deepEqual(
    model.state$.value.trades.map((t) => t.id),
    ['two'],
  );
  model.stop();
  model.stop();
  assert.deepEqual(f.counts(), [1, 1]);
});

function whatIfTrade(
  id = 'what-if-trade',
  withLineage = true,
  withHoldSpec = true,
): AutoTraderOutcome {
  return row(id, {
    exitParameterProvenance: withLineage
      ? {
          profile_id: 'captured-profile-7',
          profile_version: 3,
          stop_loss: { selected_percent: 0.5 },
          take_profit: { selected_percent: 1.25 },
          profit_protection: {
            activation_percent: 0.4,
            giveback_percent: 0.2,
            minimum_lock_percent: 0.05,
          },
          trailing_protection: {
            activation_percent: 0.6,
            distance_percent: 0.15,
          },
          breakeven_protection: { activation_percent: 0.3 },
          maximum_holding_duration: { selected_days: 3 },
          ...(withHoldSpec
            ? {
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
              }
            : {}),
        }
      : undefined,
  });
}

function whatIfCompletePayload(tradeId: string) {
  return {
    schema_version: 1,
    status: 'complete',
    scope: 'research_only_historical_what_if',
    duration_unit: 'days',
    learning_eligible: false,
    trade_id: tradeId,
    durable_path_sample_count: 42,
    baseline_replay_net_return_percent: 0.12,
    candidate_replay_net_return_percent: 0.2,
    candidate_minus_baseline_net_return_percent: 0.08,
    candidate_exit: {
      timestamp_ms: 10_000,
      price: 102.5,
      reason: 'take_profit',
      quote_source_system: 'paper_exchange.market_data_cache',
      quote_source_epoch: 'pex-epoch-5',
      quote_source_cursor: 77,
      quote_source_record_id: 'quote-77',
    },
    baseline_policy: {
      maximum_holding_duration_days: 3,
      maximum_holding_duration_ms: 259_200_000,
    },
    candidate_policy: {
      maximum_holding_duration_days: 3,
      maximum_holding_duration_ms: 259_200_000,
    },
    fee_schedule_versions: [{ fee_schedule_id: 'pex-taker-v2' }],
  };
}

function seedReplayEligiblePath(
  model: AutoTraderExcursionsModel,
  tradeId: string,
) {
  const selected = model.state$.value.trades.find(
    (item) => item.id === tradeId,
  );
  assert.ok(selected?.stableTradeId);
  model.state$.next({
    ...model.state$.value,
    pathTradeId: tradeId,
    pathPage: {
      tradeId: selected.stableTradeId,
      coverageStatus: 'complete',
      historyComplete: true,
      totalSampleCount: 2,
      samples: [],
      segments: [],
      missingReasons: [],
      hasMore: false,
      capturePolicy: {
        fineIntervalMs: 1_000,
        fineWindowMs: 1_800_000,
        coarseIntervalMs: 60_000,
        coarseAfterMs: 1_800_000,
        eventDrivenExtrema: true,
        eventDrivenThresholdCrossings: true,
      },
      lifetimeExcursionEvidence: {
        grossMfePercent: 0.42,
        grossMaePercent: -0.18,
        eventSampleCount: 2,
        eventMaximumGapMs: 1_000,
        complete: true,
        thresholdCrossings: [],
        missingReasons: [],
      },
      replayEligibility: {
        status: 'eligible',
        requiredMaximumGapMs: 1_000,
        maximumObservedGapMs: 1_000,
      },
    } satisfies TradePricePathPage,
  });
}

test('runs a research-only What-if request from captured policy inputs and parses complete evidence', async () => {
  const f = fixture([whatIfTrade()]);
  let requestedPath = '';
  let requestBody: Record<string, unknown> | undefined;
  const model = new AutoTraderExcursionsModel(
    f.source,
    undefined,
    async (path, body) => {
      requestedPath = path;
      requestBody = body as Record<string, unknown>;
      return new Response(
        JSON.stringify(whatIfCompletePayload('what-if-trade')),
      );
    },
  );
  model.start();
  model.selectTrade('what-if-trade');
  assert.equal(
    model.canRunWhatIf(),
    false,
    'replay eligibility has not loaded',
  );
  seedReplayEligiblePath(model, 'what-if-trade');
  assert.equal(model.canRunWhatIf(), true);
  assert.deepEqual(model.state$.value.whatIfDraft.parameters, {
    stop_loss_percent: 0.5,
    take_profit_percent: 1.25,
    activation_percent: 0.4,
    giveback_percent: 0.2,
    minimum_lock_percent: 0.05,
    trailing_activation_percent: 0.6,
    trailing_distance_percent: 0.15,
    breakeven_activation_percent: 0.3,
  });
  assert.equal(model.state$.value.whatIfDraft.maximumHoldDays, 3);
  assert.deepEqual(model.state$.value.whatIfDraft.maximumHoldSpec, {
    minimum: 1,
    maximum: 30,
    default: 10,
    step: 0.5,
    unit: 'days',
  });
  await model.runWhatIf();
  assert.equal(
    requestedPath,
    '/positions/auto-trader/trade-price-path/what-if',
  );
  assert.equal(requestBody?.trade_id, 'what-if-trade');
  assert.equal(requestBody?.candidate_policy_id, WHAT_IF_CANDIDATE_POLICY_ID);
  assert.equal(
    requestBody?.candidate_policy_version,
    WHAT_IF_CANDIDATE_POLICY_VERSION,
  );
  assert.equal(
    (requestBody?.candidate_parameters as Record<string, unknown>)
      .stop_loss_percent,
    0.5,
  );
  assert.equal(requestBody?.candidate_maximum_holding_duration_days, 3);
  assert.equal(model.state$.value.whatIfReplay.status, 'complete');
  assert.equal(
    model.state$.value.whatIfReplay.result?.deltaNetReturnPercent,
    0.08,
  );
  assert.equal(model.state$.value.whatIfReplay.result?.learningEligible, false);
  assert.equal(
    model.state$.value.whatIfReplay.result?.baselineMaximumHoldingDurationDays,
    3,
  );
  assert.equal(
    model.state$.value.whatIfReplay.result?.baselineMaximumHoldingDurationMs,
    259_200_000,
  );
  assert.equal(
    model.state$.value.whatIfReplay.result?.candidateMaximumHoldingDurationDays,
    3,
  );
  assert.equal(
    f.state$.value.rows[0]?.exitParameterProvenance?.stop_loss
      ?.selected_percent,
    0.5,
    'draft replay must not modify captured source settings',
  );
  model.stop();
});

test('blocks What-if replay when stable identity or captured policy lineage is missing', async () => {
  let requests = 0;
  const f = fixture([whatIfTrade('missing-lineage', false)]);
  const model = new AutoTraderExcursionsModel(f.source, undefined, async () => {
    requests++;
    return new Response('{}');
  });
  model.start();
  model.selectTrade('missing-lineage');
  assert.equal(model.canRunWhatIf(), false);
  assert.match(model.whatIfReadinessReason() ?? '', /Captured policy lineage/);
  await model.runWhatIf();
  assert.equal(requests, 0);
  assert.equal(model.state$.value.whatIfReplay.status, 'unavailable');
  model.stop();
});

test('blocks What-if replay for lifetime-complete trades with a coarse sampled tail', async () => {
  let requests = 0;
  const f = fixture([whatIfTrade()]);
  const model = new AutoTraderExcursionsModel(f.source, undefined, async () => {
    requests++;
    return new Response('{}');
  });
  model.start();
  model.selectTrade('what-if-trade');
  seedReplayEligiblePath(model, 'what-if-trade');
  const state = model.state$.value;
  assert.ok(state.pathPage);
  model.state$.next({
    ...state,
    pathPage: {
      ...state.pathPage,
      historyComplete: true,
      replayEligibility: {
        status: 'ineligible_coarse_sampling',
        requiredMaximumGapMs: 1_000,
        maximumObservedGapMs: 60_000,
        reason:
          'Coarse lifetime tail is not eligible for finer-resolution replay.',
      },
    },
  });
  assert.equal(model.canRunWhatIf(), false);
  assert.match(model.whatIfReadinessReason() ?? '', /Coarse lifetime tail/);
  await model.runWhatIf();
  assert.equal(requests, 0);
  assert.equal(model.state$.value.whatIfReplay.status, 'unavailable');
  model.stop();
});

test('fails closed when source eligibility contradicts the observed path gap', async () => {
  let requests = 0;
  const f = fixture([whatIfTrade()]);
  const model = new AutoTraderExcursionsModel(f.source, undefined, async () => {
    requests++;
    return new Response('{}');
  });
  model.start();
  model.selectTrade('what-if-trade');
  seedReplayEligiblePath(model, 'what-if-trade');
  const state = model.state$.value;
  assert.ok(state.pathPage);
  model.state$.next({
    ...state,
    pathPage: {
      ...state.pathPage,
      replayEligibility: {
        status: 'eligible',
        requiredMaximumGapMs: 1_000,
        maximumObservedGapMs: 60_000,
      },
    },
  });
  assert.equal(model.canRunWhatIf(), false);
  assert.match(
    model.whatIfReadinessReason() ?? '',
    /conflicts with the observed maximum gap/,
  );
  await model.runWhatIf();
  assert.equal(requests, 0);
  model.stop();
});

test('keeps maximum-hold candidate blank and disabled when captured schema bounds are missing', () => {
  const f = fixture([whatIfTrade('no-hold-spec', true, false)]);
  const model = new AutoTraderExcursionsModel(f.source);
  model.start();
  model.selectTrade('no-hold-spec');
  assert.equal(model.state$.value.whatIfDraft.maximumHoldDays, null);
  assert.equal(model.state$.value.whatIfDraft.maximumHoldSpec, undefined);
  assert.match(
    model.state$.value.whatIfDraft.maximumHoldReason,
    /captured schema-1 maximum_hold_days bounds are unavailable/i,
  );
  model.stop();
});

test('rejects candidate maximum hold values outside the captured step grid', async () => {
  let requests = 0;
  const f = fixture([whatIfTrade()]);
  const model = new AutoTraderExcursionsModel(f.source, undefined, async () => {
    requests++;
    return new Response(JSON.stringify(whatIfCompletePayload('what-if-trade')));
  });
  model.start();
  model.selectTrade('what-if-trade');
  seedReplayEligiblePath(model, 'what-if-trade');
  model.setWhatIfMaximumHoldDays(3.25);
  assert.equal(model.canRunWhatIf(), false);
  assert.match(model.whatIfReadinessReason() ?? '', /bounds and step/);
  await model.runWhatIf();
  assert.equal(requests, 0);
  assert.equal(model.state$.value.whatIfReplay.status, 'unavailable');
  model.stop();
});

test('preserves backend unavailability when durable lineage cannot be replayed', async () => {
  const f = fixture([whatIfTrade()]);
  const model = new AutoTraderExcursionsModel(
    f.source,
    undefined,
    async () =>
      new Response(
        JSON.stringify({
          schema_version: 1,
          status: 'unavailable',
          trade_id: 'what-if-trade',
          reason: 'replay_input_lineage_incomplete_or_invalid',
        }),
      ),
  );
  model.start();
  model.selectTrade('what-if-trade');
  seedReplayEligiblePath(model, 'what-if-trade');
  await model.runWhatIf();
  assert.equal(model.state$.value.whatIfReplay.status, 'unavailable');
  assert.equal(
    model.state$.value.whatIfReplay.reason,
    'replay_input_lineage_incomplete_or_invalid',
  );
  model.stop();
});

test('surfaces bounded not_computed replay progress without treating it as a result', async () => {
  const f = fixture([whatIfTrade()]);
  const model = new AutoTraderExcursionsModel(
    f.source,
    undefined,
    async () =>
      new Response(
        JSON.stringify({
          schema_version: 1,
          status: 'not_computed',
          learning_eligible: false,
          trade_id: 'what-if-trade',
          reason: 'replay_time_budget_exceeded',
          processed_sample_count: 500,
          total_sample_count: 800,
          after_timestamp_ms: 1234,
          after_cursor: 501,
        }),
      ),
  );
  model.start();
  model.selectTrade('what-if-trade');
  seedReplayEligiblePath(model, 'what-if-trade');
  await model.runWhatIf();
  assert.equal(model.state$.value.whatIfReplay.status, 'not_computed');
  assert.equal(model.state$.value.whatIfReplay.processedSampleCount, 500);
  assert.equal(model.state$.value.whatIfReplay.totalSampleCount, 800);
  assert.equal(model.state$.value.whatIfReplay.afterCursor, 501);
  assert.equal(model.state$.value.whatIfReplay.result, undefined);
  model.stop();
});

test('cancellation prevents a late What-if response from replacing current state', async () => {
  const f = fixture([whatIfTrade()]);
  let resolveResponse: ((response: Response) => void) | undefined;
  const model = new AutoTraderExcursionsModel(
    f.source,
    undefined,
    () =>
      new Promise((resolve) => {
        resolveResponse = resolve;
      }),
  );
  model.start();
  model.selectTrade('what-if-trade');
  seedReplayEligiblePath(model, 'what-if-trade');
  const request = model.runWhatIf();
  assert.equal(model.state$.value.whatIfReplay.status, 'loading');
  model.cancelWhatIf();
  assert.equal(model.state$.value.whatIfReplay.status, 'cancelled');
  resolveResponse?.(
    new Response(JSON.stringify(whatIfCompletePayload('what-if-trade'))),
  );
  await request;
  assert.equal(model.state$.value.whatIfReplay.status, 'cancelled');
  assert.equal(model.state$.value.whatIfReplay.result, undefined);
  model.stop();
});
