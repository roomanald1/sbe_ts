import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { AlternativeStrategiesModel, alternativeDataNeeds, alternativeLabels } from './AlternativeStrategiesModel';

describe('AlternativeStrategiesModel', () => {
  it('keeps edits isolated to configured sleeves', () => {
    const model = new AlternativeStrategiesModel();
    model.edit('missing', { enabled: true });
    assert.deepEqual(model.state$.value.drafts, {});
    assert.equal(alternativeLabels['residual-momentum'], 'Market-adjusted momentum');
    assert.match(alternativeDataNeeds['residual-momentum'], /Insights/);
    model.stop();
  });
});
