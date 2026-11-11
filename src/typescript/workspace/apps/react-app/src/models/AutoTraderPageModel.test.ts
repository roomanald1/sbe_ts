import { BehaviorSubject } from 'rxjs';
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { AutoTraderModel } from './AutoTraderModel';
import { autoTraderScoutModel } from './AutoTraderScoutModel';
import { AutoTraderPageModel } from './AutoTraderPageModel';

describe('AutoTraderPageModel', () => {
  it('coordinates exclusive settings dialogs and forwards page actions', () => {
    const source = new AutoTraderModel();
    let enabled: boolean | undefined;
    let refreshCount = 0;
    const trader = {
      state$: source.state$,
      start() {},
      stop() {},
      setEnabled(value: boolean) {
        enabled = value;
      },
      refreshDecisionParameters() {
        refreshCount += 1;
        return Promise.resolve();
      },
    } as unknown as AutoTraderModel;
    const scouts = {
      state$: new BehaviorSubject(autoTraderScoutModel.state$.value),
      start() {},
      stop() {},
    };
    const model = new AutoTraderPageModel(trader, scouts);

    model.toggleSettings();
    assert.equal(model.state$.value.settingsOpen, true);
    model.toggleParameters();
    assert.equal(model.state$.value.settingsOpen, false);
    assert.equal(model.state$.value.parametersOpen, true);
    assert.equal(refreshCount, 1);
    model.closeParameters();
    assert.equal(model.state$.value.parametersOpen, false);

    model.toggleDecisionDetails();
    assert.equal(model.state$.value.decisionDetailsOpen, true);
    model.setEnabled(true);
    assert.equal(enabled, true);
  });

  it('derives Scout counts and win rate from its state stream', () => {
    const trader = new AutoTraderModel();
    const scoutState = autoTraderScoutModel.state$.value;
    const scouts = {
      state$: new BehaviorSubject({
        ...scoutState,
        summary: {
          ...scoutState.summary,
          all: {
            ...scoutState.summary.all,
            open: 3,
            resolved: 4,
            unavailable: 2,
            wins: 3,
            losses: 1,
          },
        },
      }),
      start() {},
      stop() {},
    };
    const model = new AutoTraderPageModel(trader, scouts);

    assert.equal(model.state$.value.scoutOpenCount, 3);
    assert.equal(model.state$.value.scoutClosedCount, 6);
    assert.equal(model.state$.value.scoutWinRate, 75);
  });
});
