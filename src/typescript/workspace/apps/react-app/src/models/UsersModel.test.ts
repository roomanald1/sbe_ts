import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { UsersModel } from './UsersModel';

describe('UsersModel', () => {
  it('accepts valid one-based page input and ignores invalid input', () => {
    const model = new UsersModel();
    model.setPageInput('0');
    model.goToPage();
    assert.equal(model.state$.value.page, 0);
    model.setPageInput('3');
    model.goToPage();
    assert.equal(model.state$.value.page, 2);
    model.stop();
  });
});
