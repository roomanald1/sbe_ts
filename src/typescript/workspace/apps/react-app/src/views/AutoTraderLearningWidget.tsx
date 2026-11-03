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
type LearningRow = { id: string; symbol: string; strategy: string; parameter: string; unit: string; base: number; learned: number; delta: number; samples: number; positiveRate: number | null };
const columns: ColDef<LearningRow>[] = [
  { field: 'symbol', headerName: 'Symbol', pinned: 'left', minWidth: 110 },
  { field: 'strategy', headerName: 'Strategy', minWidth: 190 },
  { field: 'parameter', headerName: 'Parameter', flex: 1, minWidth: 170 },
  { field: 'base', headerName: 'Base', minWidth: 105, valueFormatter: ({ value, data }) => value == null ? '—' : value.toFixed(3) + ' ' + (data?.unit ?? '') },
  { field: 'learned', headerName: 'Learned', minWidth: 110, valueFormatter: ({ value, data }) => value == null ? '—' : value.toFixed(3) + ' ' + (data?.unit ?? '') },
  { field: 'delta', headerName: 'Change vs base', minWidth: 120, valueFormatter: ({ value, data }) => value == null ? '—' : (value === 0 ? '±' : value > 0 ? '+' : '−') + Math.abs(value).toFixed(3) + ' ' + (data?.unit ?? ''), cellClass: ({ value }) => Math.round(value * 1000) > 0 ? 'learning-increase' : Math.round(value * 1000) < 0 ? 'learning-decrease' : '' },
  { field: 'samples', headerName: 'Closed trades', minWidth: 110 },
  { field: 'positiveRate', headerName: 'Positive', minWidth: 95, valueFormatter: ({ value }) => value == null ? '—' : (value * 100).toFixed(1) + '%' },
];

export function AutoTraderLearningWidget() {
  const [hideUnchanged, setHideUnchanged] = useState(false);
  const model = useMemo(() => new AutoTraderModel(), []);
  useEffect(() => { model.start(); return () => model.stop(); }, [model]);
  const state = useBehaviorSnapshot(model.state$);
  const profiles = state.decisionParameters?.strategy_profiles ?? [];
  const rows = useMemo(() => profiles.flatMap((profile) => profile.parameters.map((parameter): LearningRow => ({
    id: profile.symbol + ':' + profile.strategy + ':' + parameter.key,
    symbol: profile.symbol,
    strategy: profile.strategy,
    parameter: parameter.label,
    unit: parameter.unit,
    base: parameter.configured_value ?? parameter.default_value,
    learned: parameter.learned_value,
    delta: parameter.learned_value - (parameter.configured_value ?? parameter.default_value),
    samples: profile.learning_samples,
    positiveRate: profile.positive_rate ?? null,
  }))), [profiles]);

  const visibleRows = hideUnchanged ? rows.filter((row) => Math.round(row.delta * 1000) !== 0) : rows;

  return <section className="auto-trader-strategy-parameters" aria-label="AutoTrader learning impact">
    <strong>Strategy learning impact</strong>
    <p>Base and learned parameter values for every symbol and strategy, with the signed change from base.</p>
    {rows.length === 0 ? <p>{state.loading ? 'Loading learned strategy profiles…' : 'No strategy parameter profiles are available.'}</p> : <>
      <button className="auto-trader-learning-filter" type="button" aria-pressed={hideUnchanged} onClick={() => setHideUnchanged((hidden) => !hidden)}>{hideUnchanged ? 'Show unchanged' : 'Hide unchanged'}</button>
      <div className="data-grid-wrap auto-trader-learning-grid">
        <AgGridReact<LearningRow> rowData={visibleRows} columnDefs={columns} getRowId={({ data }) => data.id} defaultColDef={{ sortable: true, resizable: true, filter: true, minWidth: 85 }} theme={learningGridTheme} domLayout="normal" rowHeight={30} headerHeight={28} />
      </div>
    </>}
  </section>;
}
