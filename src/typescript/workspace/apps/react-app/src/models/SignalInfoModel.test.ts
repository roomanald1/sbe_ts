import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { insightsRequestStream } from '../insights_rsocket';
import { setInsightsStreamRequesterForTests } from '../insights_stream';
import { SignalInfoModel } from './SignalInfoModel';

describe('SignalInfoModel', () => {
  it('applies insight snapshots and cancels its stream subscription on stop', async () => {
    let onData: ((data: Uint8Array) => void) | undefined;
    let cancelled = false;
    setInsightsStreamRequesterForTests(async (_route, listener) => {
      onData = listener;
      return { cancel: () => (cancelled = true) };
    });
    try {
      const model = new SignalInfoModel();
      model.start();
      onData!(
        Buffer.from(
          JSON.stringify({
            type: 'snapshot',
            state: {
              signal_dashboard: {
                best: [{ symbol: 'BTC-USDT', direction: 'bullish' }],
                history: [],
                performance: [],
                walkForward: [],
              },
              auto_trader_dashboard: {
                missedOpportunityBands: [
                  {
                    gate: 'strength',
                    threshold: 0.5,
                    horizonSeconds: 300,
                    sampleCount: 10,
                    timeToProfitSampleCount: 2,
                    stopTargetSampleCount: 2,
                  },
                ],
              },
              decision_audit: {
                items: [
                  {
                    status: 'skipped',
                    timestamp: 1,
                    symbol: 'BTC-USDT',
                    direction: 'bullish',
                    score: 0.3,
                    strength: 0.4,
                    reason: 'score below floor',
                  },
                ],
              },
              execution_audit: {
                items: [
                  {
                    symbol: 'BTC-USDT',
                    strategy: 'trend',
                    post_exit_observation: {
                      observed_at_ms: 2,
                      side: 'sell',
                      missed_return_percent: 0.25,
                    },
                  },
                ],
              },
            },
          }),
        ),
      );
      assert.equal(model.state$.value.best[0].symbol, 'BTC-USDT');
      assert.equal(
        model.state$.value.missedOpportunityBands[0].gate,
        'strength',
      );
      assert.equal(
        model.state$.value.skippedSignalAudit[0].reason,
        'score below floor',
      );
      assert.equal(
        model.state$.value.postExitOpportunities[0].missedReturnPercent,
        0.25,
      );
      await Promise.resolve();
      model.stop();
      assert.equal(cancelled, true);
    } finally {
      setInsightsStreamRequesterForTests(insightsRequestStream);
    }
  });
});
