import { useEffect, useMemo, useState } from 'react';
import { AgGridReact } from 'ag-grid-react';
import {
  ClientSideRowModelModule,
  ModuleRegistry,
  themeQuartz,
  type ColDef,
} from 'ag-grid-community';
import { AutoTraderModel } from '../models/AutoTraderModel';
import { useBehaviorSnapshot } from '@workspace/ui-platform';

ModuleRegistry.registerModules([ClientSideRowModelModule]);
const learningGridTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});
type LearningRow = {
  id: string;
  symbol: string;
  strategy: string;
  parameter: string;
  unit: string;
  base: number;
  candidate: number;
  effective: number;
  samples: number;
  minimumSamples: number;
  averageLift: number | null;
  lower95: number | null;
  upper95: number | null;
  promoted: boolean;
  rejected: boolean;
  status: string;
};
const columns: ColDef<LearningRow>[] = [
  { field: 'symbol', headerName: 'Symbol', pinned: 'left', width: 105 },
  { field: 'strategy', headerName: 'Strategy', width: 130 },
  { field: 'parameter', headerName: 'Parameter', width: 145 },
  {
    field: 'base',
    headerName: 'Base',
    width: 95,
    valueFormatter: ({ value, data }) =>
      value == null ? '—' : value.toFixed(3) + ' ' + (data?.unit ?? ''),
  },
  {
    field: 'candidate',
    headerName: 'Tested value',
    width: 105,
    valueFormatter: ({ value, data }) =>
      value == null ? '—' : value.toFixed(3) + ' ' + (data?.unit ?? ''),
  },
  {
    field: 'averageLift',
    headerName: 'Avg net lift',
    headerTooltip:
      'Mean paired variant-minus-baseline net return, in percentage points.',
    width: 110,
    valueFormatter: ({ value }) =>
      Number.isFinite(value)
        ? (value > 0 ? '+' : '') + value.toFixed(3) + ' pp'
        : '—',
    cellClass: ({ value }) =>
      value > 0 ? 'learning-increase' : value < 0 ? 'learning-decrease' : '',
  },
  {
    field: 'lower95',
    headerName: '90% one-sided bound',
    headerTooltip:
      'One-sided 90% Student-t approximation lower bound for paired net improvement.',
    width: 125,
    valueFormatter: ({ value }) =>
      Number.isFinite(value)
        ? (value > 0 ? '+' : '') + value.toFixed(3) + ' pp'
        : '—',
    cellClass: ({ value }) =>
      value > 0 ? 'learning-increase' : value < 0 ? 'learning-decrease' : '',
  },
  {
    field: 'upper95',
    headerName: '90% upper bound',
    headerTooltip:
      'A negative one-sided 90% upper bound marks a candidate to avoid.',
    width: 115,
    valueFormatter: ({ value }) =>
      Number.isFinite(value)
        ? (value > 0 ? '+' : '') + value.toFixed(3) + ' pp'
        : '—',
    cellClass: ({ value }) =>
      value > 0 ? 'learning-increase' : value < 0 ? 'learning-decrease' : '',
  },
  {
    field: 'samples',
    headerName: 'Paired n',
    headerTooltip:
      'Paired scout comparisons pooled for this strategy, parameter, and candidate value.',
    width: 75,
  },
  { field: 'minimumSamples', headerName: 'Required n', width: 85 },
  {
    field: 'effective',
    headerName: 'Effective value',
    width: 110,
    valueFormatter: ({ value, data }) =>
      value == null ? '—' : value.toFixed(3) + ' ' + (data?.unit ?? ''),
  },
  {
    field: 'status',
    headerName: 'Outcome',
    width: 120,
    cellClass: ({ data }) =>
      data?.promoted
        ? 'learning-increase'
        : data?.rejected
          ? 'learning-decrease'
          : '',
  },
];

export function AutoTraderLearningWidget() {
  const [promotedOnly, setPromotedOnly] = useState(false);
  const model = useMemo(() => new AutoTraderModel(), []);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  const state = useBehaviorSnapshot(model.state$);
  const profiles = state.decisionParameters?.strategy_profiles ?? [];
  const rows = useMemo(() => {
    const results = profiles.flatMap((profile) =>
      profile.parameters.flatMap((parameter) =>
        (parameter.scout_variants ?? []).map((variant): LearningRow => ({
          id: `${profile.symbol}:${profile.strategy}:${parameter.key}:${variant.candidate_value.toFixed(8)}`,
          symbol: profile.symbol,
          strategy: profile.strategy,
          parameter: parameter.label,
          unit: parameter.unit,
          base: parameter.configured_value ?? parameter.default_value,
          candidate: variant.candidate_value,
          effective: parameter.learned_value,
          samples: variant.sample_count,
          minimumSamples: variant.minimum_samples,
          averageLift: variant.average_net_improvement_percent,
          lower95: variant.lower_90_one_sided_improvement_percent ?? null,
          upper95: variant.upper_90_one_sided_improvement_percent ?? null,
          promoted: variant.promoted,
          rejected: variant.rejected,
          status: variant.promoted
            ? 'Promoted'
            : variant.rejected
              ? 'Avoid'
              : variant.sample_count < variant.minimum_samples
                ? 'Collecting'
                : (variant.lower_90_one_sided_improvement_percent ?? 0) > 0
                  ? 'Qualified'
                  : 'Not promoted',
        })),
      ),
    );
    const rowsById = new Map(results.map((row) => [row.id, row]));
    for (const pending of state.pendingScoutParameters) {
      const profile = profiles.find(
        (candidate) =>
          candidate.symbol === pending.symbol &&
          candidate.strategy === pending.strategy,
      );
      const parameter = profile?.parameters.find(
        (candidate) => candidate.key === pending.parameter,
      );
      if (!profile || !parameter) continue;
      const id = `${profile.symbol}:${profile.strategy}:${parameter.key}:${pending.candidateValue.toFixed(8)}`;
      const existing = rowsById.get(id);
      if (existing) {
        if (!existing.promoted && existing.samples < existing.minimumSamples) {
          existing.status = `Collecting · ${pending.openSamples} open`;
        }
        continue;
      }
      const row: LearningRow = {
        id,
        symbol: profile.symbol,
        strategy: profile.strategy,
        parameter: parameter.label,
        unit: parameter.unit,
        base: parameter.configured_value ?? parameter.default_value,
        candidate: pending.candidateValue,
        effective: parameter.learned_value,
        samples: 0,
        minimumSamples: Number(state.minimumScoutParameterSamples),
        averageLift: null,
        lower95: null,
        upper95: null,
        promoted: false,
        rejected: false,
        status: `Collecting · ${pending.openSamples} open`,
      };
      results.push(row);
      rowsById.set(id, row);
    }
    return results;
  }, [profiles, state.pendingScoutParameters]);

  const visibleRows = promotedOnly ? rows.filter((row) => row.promoted) : rows;

  return (
    <section
      className="auto-trader-strategy-parameters"
      aria-label="AutoTrader learning impact"
    >
      <strong>Scout parameter tests</strong>
      <p>
        Paired scout results show whether each tested parameter value beat its
        baseline. Results pool by strategy, parameter, and candidate value;
        promotion requires the configured sample minimum and a positive
        one-sided 90% Student-t approximation lower bound. “Avoid” means the
        upper bound is below zero, so the candidate is consistently worse.
      </p>
      {rows.length === 0 ? (
        <p>
          {state.loading
            ? 'Loading scout parameter results…'
            : 'No paired scout parameter results are available yet.'}
        </p>
      ) : (
        <>
          <button
            className="auto-trader-learning-filter"
            type="button"
            aria-pressed={promotedOnly}
            onClick={() => setPromotedOnly((value) => !value)}
          >
            {promotedOnly ? 'Show all tests' : 'Promoted only'}
          </button>
          {visibleRows.length === 0 ? (
            <p>No parameter variants have been promoted yet.</p>
          ) : (
            <div className="data-grid-wrap auto-trader-learning-grid">
              <AgGridReact<LearningRow>
                rowData={visibleRows}
                columnDefs={columns}
                getRowId={({ data }) => data.id}
                defaultColDef={{
                  sortable: true,
                  resizable: true,
                  filter: true,
                  minWidth: 65,
                }}
                theme={learningGridTheme}
                domLayout="normal"
                rowHeight={30}
                headerHeight={28}
              />
            </div>
          )}
        </>
      )}
    </section>
  );
}
