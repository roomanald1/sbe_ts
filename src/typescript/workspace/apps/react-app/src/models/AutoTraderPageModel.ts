import { BehaviorSubject, Subscription } from 'rxjs';
import {
  sharedAutoTraderSettingsModel,
  type AutoTraderModel,
  type AutoTraderState,
} from './AutoTraderModel';
import { autoTraderScoutModel } from './AutoTraderScoutModel';

const DECISION_PARAMETER_REFRESH_MS = 5_000;

export type AutoTraderPageState = {
  trader: AutoTraderState;
  settingsOpen: boolean;
  parametersOpen: boolean;
  decisionDetailsOpen: boolean;
  scoutOpenCount: number;
  scoutClosedCount: number;
  scoutWinRate?: number;
};

type ScoutState = typeof autoTraderScoutModel.state$.value;
type ScoutSource = Pick<
  typeof autoTraderScoutModel,
  'state$' | 'start' | 'stop'
>;

function createState(
  trader: AutoTraderState,
  scout: ScoutState,
): AutoTraderPageState {
  const counts = scout.summary.all;
  const decided = counts.wins + counts.losses;
  return {
    trader,
    settingsOpen: false,
    parametersOpen: false,
    decisionDetailsOpen: false,
    scoutOpenCount: counts.open,
    scoutClosedCount: counts.resolved + counts.unavailable,
    scoutWinRate: decided > 0 ? (counts.wins / decided) * 100 : undefined,
  };
}

export class AutoTraderPageModel {
  readonly trader: AutoTraderModel;
  readonly state$: BehaviorSubject<AutoTraderPageState>;
  private subscription?: Subscription;
  private parametersTimer?: ReturnType<typeof setInterval>;
  private running = false;
  private readonly scouts: ScoutSource;

  constructor(
    trader = sharedAutoTraderSettingsModel,
    scouts: ScoutSource = autoTraderScoutModel,
  ) {
    this.trader = trader;
    this.scouts = scouts;
    this.state$ = new BehaviorSubject(
      createState(trader.state$.value, scouts.state$.value),
    );
  }

  start() {
    if (this.running) return;
    this.running = true;
    this.trader.start();
    this.scouts.start();
    this.subscription = new Subscription();
    this.subscription.add(
      this.trader.state$.subscribe((traderState) =>
        this.publish({ trader: traderState }),
      ),
    );
    this.subscription.add(
      this.scouts.state$.subscribe((scoutState) =>
        this.publish({ scout: scoutState }),
      ),
    );
  }

  stop() {
    if (!this.running) return;
    this.running = false;
    this.subscription?.unsubscribe();
    this.subscription = undefined;
    this.clearParametersTimer();
    this.trader.stop();
    this.scouts.stop();
  }

  setEnabled(enabled: boolean) {
    return this.trader.setEnabled(enabled);
  }

  toggleSettings() {
    this.patch({
      settingsOpen: !this.state$.value.settingsOpen,
      parametersOpen: false,
    });
    this.clearParametersTimer();
  }

  toggleParameters() {
    const parametersOpen = !this.state$.value.parametersOpen;
    this.patch({ parametersOpen, settingsOpen: false });
    this.clearParametersTimer();
    if (parametersOpen) {
      void this.trader.refreshDecisionParameters();
      this.parametersTimer = setInterval(
        () => void this.trader.refreshDecisionParameters(),
        DECISION_PARAMETER_REFRESH_MS,
      );
    }
  }

  closeSettings() {
    this.patch({ settingsOpen: false });
  }

  closeParameters() {
    this.patch({ parametersOpen: false });
    this.clearParametersTimer();
  }

  toggleDecisionDetails() {
    this.patch({ decisionDetailsOpen: !this.state$.value.decisionDetailsOpen });
  }

  private publish(update: { trader?: AutoTraderState; scout?: ScoutState }) {
    const current = this.state$.value;
    const trader = update.trader ?? current.trader;
    const scout = update.scout ?? this.scouts.state$.value;
    const next = createState(trader, scout);
    this.state$.next({
      ...next,
      settingsOpen: current.settingsOpen,
      parametersOpen: current.parametersOpen,
      decisionDetailsOpen: current.decisionDetailsOpen,
    });
  }

  private patch(
    update: Partial<
      Pick<
        AutoTraderPageState,
        'settingsOpen' | 'parametersOpen' | 'decisionDetailsOpen'
      >
    >,
  ) {
    this.state$.next({ ...this.state$.value, ...update });
  }

  private clearParametersTimer() {
    if (this.parametersTimer) clearInterval(this.parametersTimer);
    this.parametersTimer = undefined;
  }
}
