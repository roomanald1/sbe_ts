import { BehaviorSubject, type Subscription } from 'rxjs';
import {
  AutoTraderModel,
  type AutoTraderState,
  type CapitalAllocation,
} from './AutoTraderModel';

export type CapitalAllocationWidgetState = {
  active: boolean;
  capitalAllocation: CapitalAllocation | undefined;
  availableNotional: number;
};

type AllocationSource = Pick<AutoTraderModel, 'state$' | 'start' | 'stop'>;

function project(
  source: AutoTraderState,
  active: boolean,
): CapitalAllocationWidgetState {
  return {
    active,
    capitalAllocation: source.capitalAllocation,
    availableNotional: source.availableNotional,
  };
}

/** Per-widget lifecycle boundary over the authoritative shared AutoTrader status source. */
export class CapitalAllocationWidgetModel {
  readonly state$: BehaviorSubject<CapitalAllocationWidgetState>;
  private subscription?: Subscription;
  private running = false;
  private generation = 0;
  readonly instanceId: string;
  private readonly source: AllocationSource;

  constructor(
    instanceId: string,
    source: AllocationSource = new AutoTraderModel(),
  ) {
    this.instanceId = instanceId;
    this.source = source;
    this.state$ = new BehaviorSubject(project(source.state$.value, false));
  }

  start(): void {
    if (this.running) return;
    this.running = true;
    const generation = ++this.generation;
    this.subscription = this.source.state$.subscribe((state) => {
      if (this.running && generation === this.generation)
        this.state$.next(project(state, true));
    });
    this.source.start();
  }

  stop(): void {
    if (!this.running) return;
    this.running = false;
    this.generation += 1;
    this.subscription?.unsubscribe();
    this.subscription = undefined;
    this.source.stop();
    this.state$.next(project(this.source.state$.value, false));
  }

  dispose(): void {
    this.stop();
    this.state$.complete();
  }
}
