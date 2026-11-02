import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { CloseInsightsModel } from './CloseInsightsModel';

describe('CloseInsightsModel', () => {
  it('updates the barrier target and clears a cancelled close', () => {
    const model = new CloseInsightsModel();
    model.setBarrierTargetUsd('25');
    assert.equal(model.state$.value.barrierTargetUsd, '25');
    model.cancelClose();
    assert.equal(model.state$.value.confirmingInsight, undefined);
    model.stop();
  });
});
