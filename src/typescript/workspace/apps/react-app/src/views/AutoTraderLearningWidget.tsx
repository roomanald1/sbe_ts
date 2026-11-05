import { useEffect, useMemo, useState } from 'react';
import { AgGridReact } from 'ag-grid-react';
import { ClientSideRowModelModule, ModuleRegistry, themeQuartz, type ColDef } from 'ag-grid-community';
import { AutoTraderModel } from '../models/AutoTraderModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';

ModuleRegistry.registerModules([ClientSideRowModelModule]);
const learningGridTheme = themeQuartz.withParams({
  accentColor: '#8296ad', backgroundColor: '#0d0f12', borderColor: '#242a31',
  foregroundColor: '#e7e7e7', headerBackgroundColor: '#11151a', headerTextColor: '#a0a6ad', rowHoverColor: '#141b24',
});
type LearningRow = { id: string; symbol: string; strategy: string; parameter: string; unit: string; base: number; candidate: number; effective: number; samples: number; minimumSamples: number; averageLift: number; lower95: number | null; promoted: boolean; status: string };
const columns: ColDef<LearningRow>[] = [
  { field: 'symbol', headerName: 'Symbol', pinned: 'left', minWidth: 110 },
  { field: 'strategy', headerName: 'Strategy', minWidth: 190 },
  { field: 'parameter', headerName: 'Parameter', flex: 1, minWidth: 170 },
  { field: 'base', headerName: 'Base', minWidth: 95, valueFormatter: ({ value, data }) => value == null ? '—' : value.toFixed(3) + ' ' + (data?.unit ?? '') },
  { field: 'candidate', headerName: 'Tested value', minWidth: 110, valueFormatter: ({ value, data }) => value == null ? '—' : value.toFixed(3) + ' ' + (data?.unit ?? '') },
  { field: 'averageLift', headerName: 'Avg net lift', headerTooltip: 'Mean paired variant-minus-baseline net return, in percentage points.', minWidth: 115, valueFormatter: ({ value }) => (value > 0 ? '+' : '') + value.toFixed(3) + ' pp', cellClass: ({ value }) => value > 0 ? 'learning-increase' : value < 0 ? 'learning-decrease' : '' },
  { field: 'lower95', headerName: '90% one-sided bound', headerTooltip: 'One-sided 90% Student-t approximation lower bound for paired net improvement.', minWidth: 140, valueFormatter: ({ value }) => Number.isFinite(value) ? (value > 0 ? '+' : '') + value.toFixed(3) + ' pp' : '—', cellClass: ({ value }) => value > 0 ? 'learning-increase' : value < 0 ? 'learning-decrease' : '' },
  { field: 'samples', headerName: 'Paired n', headerTooltip: 'Paired scout comparisons pooled for this strategy, parameter, and candidate value.', minWidth: 90 },
  { field: 'minimumSamples', headerName: 'Required n', minWidth: 95 },
  { field: 'effective', headerName: 'Effective value', minWidth: 115, valueFormatter: ({ value, data }) => value == null ? '—' : value.toFixed(3) + ' ' + (data?.unit ?? '') },
  { field: 'status', headerName: 'Promotion', minWidth: 130, cellClass: ({ data }) => data?.promoted ? 'learning-increase' : '' },
];

export function AutoTraderLearningWidget() {
  const [promotedOnly, setPromotedOnly] = useState(false);
  const model = useMemo(() => new AutoTraderModel(), []);
  useEffect(() => { model.start(); return () => model.stop(); }, [model]);
  const state = useBehaviorSnapshot(model.state$);
  const profiles = state.decisionParameters?.strategy_profiles ?? [];
  const rows = useMemo(() => profiles.flatMap((profile) => profile.parameters.flatMap((parameter) =>
    (parameter.scout_variants ?? []).map((variant): LearningRow => ({
      id: profile.symbol + ':' + profile.strategy + ':' + parameter.key + ':' + variant.candidate_value,
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
      promoted: variant.promoted,
      status: variant.promoted ? 'Promoted' : variant.sample_count < variant.minimum_samples ? 'Collecting' : (variant.lower_90_one_sided_improvement_percent ?? 0) > 0 ? 'Qualified' : 'Not promoted',
    })))), [profiles]);

  const visibleRows = promotedOnly ? rows.filter((row) => row.promoted) : rows;

  return <section className="auto-trader-strategy-parameters" aria-label="AutoTrader learning impact">
    <strong>Scout parameter tests</strong>
    <p>Paired scout results show whether each tested parameter value beat its baseline. Results pool by strategy, parameter, and candidate value; promotion requires the strategy’s sample minimum and a positive one-sided 90% Student-t approximation lower bound.</p>
    {rows.length === 0 ? <p>{state.loading ? 'Loading scout parameter results…' : 'No paired scout parameter results are available yet.'}</p> : <>
      <button className="auto-trader-learning-filter" type="button" aria-pressed={promotedOnly} onClick={() => setPromotedOnly((value) => !value)}>{promotedOnly ? 'Show all tests' : 'Promoted only'}</button>
      {visibleRows.length === 0 ? <p>No parameter variants have been promoted yet.</p> :
        <div className="data-grid-wrap auto-trader-learning-grid">
          <AgGridReact<LearningRow> rowData={visibleRows} columnDefs={columns} getRowId={({ data }) => data.id} defaultColDef={{ sortable: true, resizable: true, filter: true, minWidth: 85 }} theme={learningGridTheme} domLayout="normal" rowHeight={30} headerHeight={28} />
        </div>}
    </>}
  </section>;
}
