import { BehaviorSubject, Subscription } from 'rxjs';
import type {
  AutoTraderModel,
  AutoTraderStrategyProfile,
} from './AutoTraderModel';
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

export type WalkForwardProfileRow = {
  id: string;
  symbol: string;
  strategy: string;
  samples: number | null;
  winRate: number | null;
  averageNet: number | null;
  expectancyCiLowPercent: number | null;
  expectancyCiHighPercent: number | null;
  effectiveSamples: number | null;
  windows: number | null;
  maxDrawdown: number | null;
  meanDownside: number | null;
  lastSampleTimestampMs: number | null;
};

/**
 * Keeps walk-forward values source-backed and gives the table an independently
 * testable row projection. Missing profile evidence remains unavailable.
 */
export function projectWalkForwardProfileRows(
  profiles: readonly AutoTraderStrategyProfile[],
): WalkForwardProfileRow[] {
  return profiles.map((profile) => {
    const evidence = profile.walk_forward_evidence;
    return {
      id: `${profile.symbol}:${profile.strategy}`,
      symbol: profile.symbol,
      strategy: profile.strategy,
      samples: evidence?.sampleCount ?? null,
      winRate: evidence?.positiveNetRate ?? null,
      averageNet: evidence?.averageNetReturnPercent ?? null,
      expectancyCiLowPercent: evidence?.expectancyCiLowPercent ?? null,
      expectancyCiHighPercent: evidence?.expectancyCiHighPercent ?? null,
      effectiveSamples: evidence?.effectiveSampleSize ?? null,
      windows: evidence?.windowCount ?? null,
      maxDrawdown: evidence?.maxDrawdownPercent ?? null,
      meanDownside: evidence?.downsideLossPercent ?? null,
      lastSampleTimestampMs: evidence?.lastSampleTimestampMs ?? null,
    };
  });
}

export type StrategyPerformanceColumnGroup =
  | 'performance'
  | 'entryEvidence'
  | 'sample'
  | 'status'
  | 'profile'
  | 'learning';

export type StrategyPerformanceView =
  | 'performance'
  | 'heatmap'
  | 'history'
  | 'exit-policies';
export type StrategyPerformanceSource = 'autotrader' | 'scouts';

export type ScoutOutcomeCountRow = {
  strategy: string;
  outcomes: number;
  positiveOutcomes: number;
};

export function projectScoutOutcomeCounts(
  rows: readonly { strategy: string; count: number; positive: number }[],
): ScoutOutcomeCountRow[] {
  return rows
    .filter((row) => Number.isFinite(row.count) && row.count >= 0)
    .map((row) => ({
      strategy: row.strategy,
      outcomes: row.count,
      positiveOutcomes: Number.isFinite(row.positive) ? row.positive : 0,
    }))
    .sort((a, b) => a.strategy.localeCompare(b.strategy));
}

export type StrategyOutcomeCoverageRow = {
  strategy: string;
  symbol: string;
  closedAuditRows: number;
  strictRows: number;
  exploratoryRows: number;
  scoutRows: number;
  carryRows: number;
  unknownRows: number;
  latestClosedAt: number | null;
};

export type StrategyCoverageHeatmap = {
  symbols: string[];
  rows: Array<{
    strategy: string;
    cells: Array<{
      symbol: string;
      closedAuditRows: number;
      intensity: number;
    }>;
  }>;
};

export type StrategyOutcomeHistoryRow = {
  rowId: string;
  symbol: string;
  strategy: string;
  source: string;
  closedAt: number | null;
  entryProfileId: string | null;
  entryProfileVersion: number | null;
};

export type ExitProfileSnapshotRow = {
  symbol: string;
  strategy: string;
  profile: string | null;
  parameters: Array<{ label: string; value: number | null; unit: string }>;
  effectiveSamples: number | null;
  validation: 'validated' | 'not-validated' | 'unavailable';
};

/** Groups audit-derived outcomes by strategy and symbol without interpreting
 * their financial fields as independently reconciled performance. */
export function projectStrategyOutcomeCoverage(
  rows: readonly AutoTraderOutcome[],
): StrategyOutcomeCoverageRow[] {
  const grouped = new Map<string, StrategyOutcomeCoverageRow>();
  for (const row of rows) {
    if (row.status !== 'Closed') continue;
    const strategy = row.strategy || 'Unattributed';
    const symbol = row.symbol || 'Unattributed';
    const key = `${strategy}\u0000${symbol}`;
    const result = grouped.get(key) ?? {
      strategy,
      symbol,
      closedAuditRows: 0,
      strictRows: 0,
      exploratoryRows: 0,
      scoutRows: 0,
      carryRows: 0,
      unknownRows: 0,
      latestClosedAt: null,
    };
    result.closedAuditRows += 1;
    switch (row.source) {
      case 'strict':
        result.strictRows += 1;
        break;
      case 'exploratory':
        result.exploratoryRows += 1;
        break;
      case 'scout':
        result.scoutRows += 1;
        break;
      case 'paper-carry':
        result.carryRows += 1;
        break;
      default:
        result.unknownRows += 1;
    }
    if (typeof row.closedAt === 'number' && Number.isFinite(row.closedAt)) {
      result.latestClosedAt = Math.max(
        result.latestClosedAt ?? 0,
        row.closedAt,
      );
    }
    grouped.set(key, result);
  }
  return [...grouped.values()].sort(
    (a, b) =>
      a.strategy.localeCompare(b.strategy) || a.symbol.localeCompare(b.symbol),
  );
}

export function isValidatedWalkForwardProfile(
  evidence: AutoTraderStrategyProfile['walk_forward_evidence'],
): boolean | null {
  if (!evidence) return null;
  if (evidence.finalHoldoutStatus !== 'validated') return false;
  if (
    evidence.historyCoverage !== 'complete' ||
    evidence.costCoverage !== 'complete' ||
    evidence.searchFamilyCoverage !== 'complete' ||
    (evidence.oosFoldCount ?? 0) <= 0 ||
    evidence.effectiveSampleSize === undefined ||
    evidence.effectiveSampleSize === null ||
    evidence.effectiveSampleSize <= 0
  )
    return false;
  return true;
}

export function projectStrategyCoverageByStrategy(
  rows: readonly StrategyOutcomeCoverageRow[],
): Array<{ strategy: string; closedAuditRows: number }> {
  const counts = new Map<string, number>();
  for (const row of rows)
    counts.set(
      row.strategy,
      (counts.get(row.strategy) ?? 0) + row.closedAuditRows,
    );
  return [...counts]
    .map(([strategy, closedAuditRows]) => ({ strategy, closedAuditRows }))
    .sort((a, b) => a.strategy.localeCompare(b.strategy));
}

export function projectStrategyCoverageHeatmap(
  rows: readonly StrategyOutcomeCoverageRow[],
): StrategyCoverageHeatmap {
  const symbols = [...new Set(rows.map((row) => row.symbol))].sort();
  const strategies = [...new Set(rows.map((row) => row.strategy))].sort();
  const counts = new Map(
    rows.map((row) => [
      `${row.strategy}\u0000${row.symbol}`,
      row.closedAuditRows,
    ]),
  );
  const maximum = Math.max(0, ...rows.map((row) => row.closedAuditRows));
  return {
    symbols,
    rows: strategies.map((strategy) => ({
      strategy,
      cells: symbols.map((symbol) => {
        const closedAuditRows = counts.get(`${strategy}\u0000${symbol}`) ?? 0;
        return {
          symbol,
          closedAuditRows,
          intensity:
            maximum === 0 || closedAuditRows === 0
              ? 0
              : Math.max(1, Math.ceil((closedAuditRows / maximum) * 5)),
        };
      }),
    })),
  };
}

export function projectStrategyOutcomeHistory(
  rows: readonly AutoTraderOutcome[],
  limit = 50,
): StrategyOutcomeHistoryRow[] {
  return rows
    .filter((row) => row.status === 'Closed')
    .slice()
    .sort((a, b) => (b.closedAt ?? 0) - (a.closedAt ?? 0))
    .slice(0, Math.max(0, limit))
    .map((row) => ({
      rowId: row.rowId,
      symbol: row.symbol,
      strategy: row.strategy,
      source: row.source,
      closedAt:
        typeof row.closedAt === 'number' && Number.isFinite(row.closedAt)
          ? row.closedAt
          : null,
      entryProfileId: row.entryProfileId ?? null,
      entryProfileVersion: row.entryProfileVersion ?? null,
    }));
}

export function projectExitProfileSnapshot(
  profiles: readonly AutoTraderStrategyProfile[],
): ExitProfileSnapshotRow[] {
  return profiles.map((profile) => {
    const evidence = profile.walk_forward_evidence;
    const validated = isValidatedWalkForwardProfile(evidence);
    return {
      symbol: profile.symbol,
      strategy: profile.strategy,
      profile: profile.profile_id
        ? `${profile.profile_id}${profile.profile_version === undefined ? '' : ` · v${profile.profile_version}`}`
        : null,
      parameters: profile.parameters
        .filter((parameter) =>
          /stop|target|profit|hold|trailing|breakeven/i.test(
            `${parameter.key} ${parameter.label}`,
          ),
        )
        .map((parameter) => ({
          label: parameter.label,
          value: Number.isFinite(parameter.value) ? parameter.value : null,
          unit: parameter.unit,
        })),
      effectiveSamples: evidence?.effectiveSampleSize ?? null,
      validation:
        validated === true
          ? 'validated'
          : validated === false
            ? 'not-validated'
            : 'unavailable',
    };
  });
}

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
  view: StrategyPerformanceView;
  sourceView: StrategyPerformanceSource;
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
      view: 'performance',
      sourceView: 'autotrader',
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

  setView(view: StrategyPerformanceView): void {
    if (this.disposed || this.state$.value.view === view) return;
    this.patch({ view });
  }

  setSourceView(sourceView: StrategyPerformanceSource): void {
    if (this.disposed || this.state$.value.sourceView === sourceView) return;
    this.patch({ sourceView });
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
