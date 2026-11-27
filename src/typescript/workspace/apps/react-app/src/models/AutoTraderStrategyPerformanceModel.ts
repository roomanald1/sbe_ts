import { BehaviorSubject, Subscription } from 'rxjs';
import type { AutoTraderModel } from './AutoTraderModel';
import type { AutoTraderOutcomesModel } from './AutoTraderOutcomesModel';
import type { AutoTraderScoutModel } from './AutoTraderScoutModel';
import type { AutoTraderOutcome } from './AutoTraderOutcomesModel';

/**
 * Describes the source coverage exposed by the canonical outcomes model.
 * It deliberately does not calculate financial performance or infer that an
 * audited outcome has passed independent reconciliation.
 */
export type StrategyOutcomeEvidenceSummary = {
  auditClosedRows: number;
  strictRows: number;
  exploratoryRows: number;
  scoutRows: number;
  paperCarryRows: number;
  unknownSourceRows: number;
  independentReconciliation: 'not-provided-by-outcomes-contract';
};

export function summarizeStrategyOutcomeEvidence(
  rows: readonly AutoTraderOutcome[],
): StrategyOutcomeEvidenceSummary {
  const summary: StrategyOutcomeEvidenceSummary = {
    auditClosedRows: 0,
    strictRows: 0,
    exploratoryRows: 0,
    scoutRows: 0,
    paperCarryRows: 0,
    unknownSourceRows: 0,
    independentReconciliation: 'not-provided-by-outcomes-contract',
  };
  for (const row of rows) {
    if (row.status !== 'Closed') continue;
    summary.auditClosedRows += 1;
    switch (row.source) {
      case 'strict':
        summary.strictRows += 1;
        break;
      case 'exploratory':
        summary.exploratoryRows += 1;
        break;
      case 'scout':
        summary.scoutRows += 1;
        break;
      case 'paper-carry':
        summary.paperCarryRows += 1;
        break;
      case 'unknown':
        summary.unknownSourceRows += 1;
        break;
    }
  }
  return summary;
}

export type StrategyPerformanceColumnGroup =
  | 'performance'
  | 'entryEvidence'
  | 'sample'
  | 'status'
  | 'profile'
  | 'learning';

export const STRATEGY_PERFORMANCE_COLUMN_GROUPS: Array<{
  id: StrategyPerformanceColumnGroup;
  children: string[];
}> = [
  { id: 'performance', children: ['todayRealizedUsd'] },
  {
    id: 'entryEvidence',
    children: [
      'entryAverageNetReturn',
      'entryPositiveRate',
      'entrySampleCount',
      'entryRiskMultiplier',
      'entryWhyNoEntry',
    ],
  },
  {
    id: 'sample',
    children: [
      'trades',
      'strictTrades',
      'exploratoryTrades',
      'scoutTrades',
      'paperCarryTrades',
      'unknownSourceTrades',
    ],
  },
  { id: 'status', children: ['enabled', 'scoutEnabled'] },
  { id: 'profile', children: ['medianHoldingMs'] },
  {
    id: 'learning',
    children: [
      'learningAverageNet',
      'learningSamples',
      'feedbackMultiplier',
      'effectiveSizeMultiplier',
      'scoutOutcomeAverageNet',
      'pairedScoutAverageLift',
      'pairedScoutSamples',
    ],
  },
];
export type StrategyPerformanceColumnVisibility = Record<
  StrategyPerformanceColumnGroup,
  boolean
>;

type OutcomesState = AutoTraderOutcomesModel['state$']['value'];
type SettingsState = AutoTraderModel['state$']['value'];
type ScoutState = AutoTraderScoutModel['state$']['value'];
type LifecycleSource<T> = {
  state$: BehaviorSubject<T>;
  start: () => void;
  stop: () => void;
};

export type StrategyPerformanceSources = {
  outcomes: LifecycleSource<OutcomesState>;
  settings: LifecycleSource<SettingsState>;
  scout: LifecycleSource<ScoutState>;
  refresh?: () => void | Promise<unknown>;
};

export type StrategyPerformanceStorage = Pick<Storage, 'getItem' | 'setItem'>;

export type AutoTraderStrategyPerformanceState = {
  active: boolean;
  outcomes: OutcomesState;
  settings: SettingsState;
  scout: ScoutState;
  hideDisabledStrategies: boolean;
  visibleColumnGroups: StrategyPerformanceColumnVisibility;
  expandedEntryStrategies: Set<string>;
  gridColumnStateJson: string | null;
  error?: string;
};

const DEFAULT_COLUMNS: StrategyPerformanceColumnVisibility = {
  performance: true,
  entryEvidence: true,
  sample: true,
  status: true,
  profile: true,
  learning: true,
};
const LEGACY_HIDE_DISABLED_KEY = 'autotrader-hide-disabled-strategies-v1';
const LEGACY_COLUMNS_KEY = 'autotrader-strategy-column-groups-v1';

function browserStorage(): StrategyPerformanceStorage | undefined {
  try {
    return typeof localStorage === 'undefined' ? undefined : localStorage;
  } catch {
    return undefined;
  }
}

export class AutoTraderStrategyPerformanceModel {
  readonly state$: BehaviorSubject<AutoTraderStrategyPerformanceState>;
  private subscriptions = new Subscription();
  private readonly storage?: StrategyPerformanceStorage;
  private started = false;
  private disposed = false;
  private readonly sourceConsumers: LifecycleSource<unknown>[];
  private readonly hideDisabledKey: string;
  private readonly columnsKey: string;
  private readonly gridStateKey: string;
  private readonly sources: StrategyPerformanceSources;

  constructor(
    sources: StrategyPerformanceSources,
    instanceId: string,
    storage: StrategyPerformanceStorage | undefined = browserStorage(),
  ) {
    this.sources = sources;
    const scope = encodeURIComponent(instanceId || 'default');
    this.hideDisabledKey = `autotrader-strategy-performance:${scope}:hide-disabled-v1`;
    this.columnsKey = `autotrader-strategy-performance:${scope}:columns-v1`;
    this.gridStateKey = `autotrader-strategy-performance:${scope}:grid-state-v1`;
    this.storage = storage;
    this.sourceConsumers = [
      sources.outcomes as LifecycleSource<unknown>,
      sources.settings as LifecycleSource<unknown>,
      sources.scout as LifecycleSource<unknown>,
    ];
    this.state$ = new BehaviorSubject<AutoTraderStrategyPerformanceState>({
      active: false,
      outcomes: sources.outcomes.state$.value,
      settings: sources.settings.state$.value,
      scout: sources.scout.state$.value,
      hideDisabledStrategies: this.readHideDisabled(),
      visibleColumnGroups: this.readColumnGroups(),
      expandedEntryStrategies: new Set(),
      gridColumnStateJson: this.readGridState(),
    });
  }

  start(): void {
    if (this.disposed || this.started) return;
    this.started = true;
    this.subscriptions.add(
      this.sources.outcomes.state$.subscribe((outcomes) =>
        this.patch({ outcomes }),
      ),
    );
    this.subscriptions.add(
      this.sources.settings.state$.subscribe((settings) =>
        this.patch({ settings }),
      ),
    );
    this.subscriptions.add(
      this.sources.scout.state$.subscribe((scout) => this.patch({ scout })),
    );
    try {
      for (const source of this.sourceConsumers) source.start();
      this.patch({ active: true, error: undefined });
    } catch (error) {
      this.subscriptions.unsubscribe();
      this.subscriptions = new Subscription();
      this.started = false;
      for (const source of this.sourceConsumers) source.stop();
      this.patch({
        active: false,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }

  stop(): void {
    if (!this.started) return;
    this.started = false;
    this.subscriptions.unsubscribe();
    this.subscriptions = new Subscription();
    for (const source of this.sourceConsumers) source.stop();
    this.patch({ active: false });
  }

  dispose(): void {
    if (this.disposed) return;
    this.stop();
    this.disposed = true;
    this.state$.complete();
  }

  async refresh(): Promise<void> {
    if (!this.started || this.disposed) return;
    this.patch({ error: undefined });
    try {
      await this.sources.refresh?.();
    } catch (error) {
      this.patch({
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }

  setHideDisabledStrategies(value: boolean): void {
    if (this.disposed || this.state$.value.hideDisabledStrategies === value)
      return;
    this.persist(this.hideDisabledKey, String(value));
    this.patch({ hideDisabledStrategies: value });
  }

  setColumnGroupVisible(
    group: StrategyPerformanceColumnGroup,
    visible: boolean,
  ): void {
    if (
      this.disposed ||
      this.state$.value.visibleColumnGroups[group] === visible
    )
      return;
    const next = { ...this.state$.value.visibleColumnGroups, [group]: visible };
    this.persist(this.columnsKey, JSON.stringify(next));
    this.patch({ visibleColumnGroups: next });
  }

  toggleExpandedEntryStrategy(strategy: string): void {
    if (this.disposed) return;
    const next = new Set(this.state$.value.expandedEntryStrategies);
    if (next.has(strategy)) next.delete(strategy);
    else next.add(strategy);
    this.patch({ expandedEntryStrategies: next });
  }

  saveGridColumnState(serialized: string): void {
    if (this.disposed) return;
    this.persist(this.gridStateKey, serialized);
    this.patch({ gridColumnStateJson: serialized });
  }

  private patch(values: Partial<AutoTraderStrategyPerformanceState>): void {
    if (this.disposed) return;
    this.state$.next({ ...this.state$.value, ...values });
  }

  private readHideDisabled(): boolean {
    try {
      const stored =
        this.storage?.getItem(this.hideDisabledKey) ??
        this.storage?.getItem(LEGACY_HIDE_DISABLED_KEY);
      return stored === 'true';
    } catch {
      return false;
    }
  }

  private readColumnGroups(): StrategyPerformanceColumnVisibility {
    try {
      const stored =
        this.storage?.getItem(this.columnsKey) ??
        this.storage?.getItem(LEGACY_COLUMNS_KEY);
      if (!stored) return { ...DEFAULT_COLUMNS };
      const value: unknown = JSON.parse(stored);
      if (!value || typeof value !== 'object' || Array.isArray(value))
        return { ...DEFAULT_COLUMNS };
      const columns = value as Partial<StrategyPerformanceColumnVisibility>;
      return Object.fromEntries(
        STRATEGY_PERFORMANCE_COLUMN_GROUPS.map(({ id }) => [
          id,
          typeof columns[id] === 'boolean' ? columns[id] : true,
        ]),
      ) as StrategyPerformanceColumnVisibility;
    } catch {
      return { ...DEFAULT_COLUMNS };
    }
  }

  private readGridState(): string | null {
    try {
      return (
        this.storage?.getItem(this.gridStateKey) ??
        this.storage?.getItem('autotrader-strategy-grid-state-v1') ??
        null
      );
    } catch {
      return null;
    }
  }

  private persist(key: string, value: string): void {
    try {
      this.storage?.setItem(key, value);
    } catch (error) {
      this.patch({
        error:
          error instanceof Error
            ? error.message
            : 'View settings could not be saved.',
      });
    }
  }
}
