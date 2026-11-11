import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { getLimitPriceMarkWarning } from './limit_price_guard';

describe('getLimitPriceMarkWarning', () => {
  it('warns when a valid limit is at least ten times from the mark', () => {
    assert.match(getLimitPriceMarkWarning('100', 10) ?? '', /10×/);
    assert.match(getLimitPriceMarkWarning('10', 100) ?? '', /10×/);
  });
  it('does not warn for closer prices or invalid inputs', () => {
    for (const [price, mark] of [
      ['99', 10],
      ['0', 10],
      ['abc', 10],
      ['10', 0],
    ] as const)
      assert.equal(getLimitPriceMarkWarning(price, mark), undefined);
    assert.equal(getLimitPriceMarkWarning('100'), undefined);
  });
});
