import { formatForSymbol } from '../instrument_precision';
import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { usePausedValue } from '../use_paused_value';
import { AgGridReact } from 'ag-grid-react';
import {
  ClientSideRowModelModule,
  ModuleRegistry,
  themeQuartz,
  type ColDef,
} from 'ag-grid-community';
import type { Position } from '../models/PositionTypes';
import { FeedToolbar } from './FeedToolbar';

ModuleRegistry.registerModules([ClientSideRowModelModule]);

type HedgeCandidate = { symbol: string; correlation: number | null; paired_samples: number; latest_price: number | null };
type HedgePosition = Position & { latest_price: number | null; candidates: HedgeCandidate[] };
type Response = { positions: HedgePosition[] };
type HedgeGridRow = {
  id: string;
  kind: 'position' | 'candidate';
  position: HedgePosition;
  candidate?: HedgeCandidate;
  symbol: string;
  side: string;
  quantity?: number;
  correlation?: number | null;
};

const gridTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});

function sortedCandidates(position: HedgePosition): HedgeCandidate[] {
  return position.candidates.filter((candidate) => candidate.symbol !== position.symbol).sort((a, b) => {
    if (a.correlation === null) return 1;
    if (b.correlation === null) return -1;
    return a.correlation - b.correlation;
  });
}

function HedgingComponent({ paused = false }: { paused?: boolean }) {
  const [positions, setPositions] = useState<HedgePosition[]>([]);
  const fractionsRef = useRef<Record<string, string>>({});
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [busy, setBusy] = useState<string>();
  const [error, setError] = useState<string>();
  const refresh = useCallback(async () => {
    if (document.activeElement instanceof HTMLElement && document.activeElement.closest('.hedging-grid-input')) return;
    try {
      const response = await fetch('/positions/hedge-candidates');
      if (!response.ok) throw new Error('Could not load positions');
      const data = await response.json() as Response;
      setPositions(data.positions.filter((position) => Math.abs(position.quantity) > 1e-12));
      setError(undefined);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Could not load positions');
    }
  }, []);
  useEffect(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), 5000);
    return () => clearInterval(timer);
  }, [refresh]);

  async function hedge(position: HedgePosition, symbol: string, candidate?: HedgeCandidate) {
    const ratio = Number(fractionsRef.current[position.symbol] ?? '50');
    if (!Number.isFinite(ratio) || ratio <= 0 || ratio > 100) {
      setError('Enter a hedge percentage greater than 0 and at most 100.');
      return;
    }
    const correlation = symbol === position.symbol ? 1 : candidate?.correlation ?? undefined;
    const positionPrice = position.latest_price ?? undefined;
    const candidatePrice = symbol === position.symbol ? position.latest_price ?? undefined : candidate?.latest_price ?? undefined;
    if (correlation === undefined || !positionPrice || !candidatePrice || candidatePrice <= 0) {
      setError('Not enough live price history to size this hedge yet.');
      return;
    }
    const quantity = symbol === position.symbol
      ? Math.abs(position.quantity) * ratio / 100
      : Math.abs(position.quantity) * positionPrice * Math.abs(correlation) * ratio / 100 / candidatePrice;
    const side = (position.quantity > 0) !== (correlation < 0) ? 'sell' : 'buy';
    setBusy(position.symbol);
    setError(undefined);
    try {
      const response = await fetch('/positions/orders', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ symbol, side, quantity, order_type: 'ioc' }),
      });
      if (!response.ok) throw new Error((await response.text()).trim() || 'Hedge order rejected');
      await refresh();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Could not submit hedge order');
    } finally {
      setBusy(undefined);
    }
  }

  const rowData = useMemo<HedgeGridRow[]>(() => positions.flatMap((position) => {
    const parent: HedgeGridRow = {
      id: position.symbol,
      kind: 'position',
      position,
      symbol: position.symbol,
      side: position.quantity > 0 ? 'LONG' : 'SHORT',
      quantity: Math.abs(position.quantity),
    };
    if (!expanded[position.symbol]) return [parent];
    return [parent, ...sortedCandidates(position).map((candidate) => ({
      id: position.symbol + ':' + candidate.symbol,
      kind: 'candidate' as const,
      position,
      candidate,
      symbol: candidate.symbol,
      side: candidate.correlation === null
        ? '—'
        : (position.quantity > 0) !== (candidate.correlation < 0) ? 'SELL' : 'BUY',
      correlation: candidate.correlation,
    }))];
  }), [positions, expanded]);

  const displayedRowData = usePausedValue(rowData, paused);
  const columnDefs = useMemo<ColDef<HedgeGridRow>[]>(() => [
    {
      field: 'symbol',
      headerName: 'Position / hedge',
      minWidth: 150,
      flex: 1.2,
      cellRenderer: ({ data }: { data?: HedgeGridRow }) => {
        if (!data) return null;
        if (data.kind === 'candidate') return <span className="hedging-child-symbol">↳ {data.symbol}</span>;
        const isExpanded = !!expanded[data.position.symbol];
        return (
          <button
            className="hedging-expand"
            type="button"
            aria-expanded={isExpanded}
            onClick={() => setExpanded({ ...expanded, [data.position.symbol]: !isExpanded })}
          >
            <span aria-hidden="true">{isExpanded ? '▾' : '▸'}</span>{data.symbol}
          </button>
        );
      },
    },
    { field: 'side', headerName: 'Side', width: 82 },
    {
      field: 'quantity',
      headerName: 'Net qty',
      width: 105,
      valueFormatter: ({ value, data }) => data?.kind === 'candidate' || value == null ? '' : formatForSymbol(value, data?.symbol ?? '', 'quantity'),
    },
    {
      field: 'correlation',
      headerName: 'Correlation',
      width: 105,
      valueFormatter: ({ value, data }) => data?.kind !== 'candidate'
        ? ''
        : value == null ? 'Sampling' : value.toFixed(2),
    },
    {
      colId: 'hedgePercent',
      headerName: 'Hedge %',
      width: 100,
      cellRenderer: ({ data }: { data?: HedgeGridRow }) => data?.kind !== 'position' ? null : (
        <input
          className="hedging-grid-input"
          aria-label={'Hedge percentage for ' + data.position.symbol}
          type="number"
          min="0.01"
          max="100"
          step="any"
          defaultValue={fractionsRef.current[data.position.symbol] ?? '50'}
          onChange={(event) => { fractionsRef.current[data.position.symbol] = event.target.value; }}
        />
      ),
    },
    {
      colId: 'action',
      headerName: 'Action',
      width: 105,
      cellRenderer: ({ data }: { data?: HedgeGridRow }) => {
        if (!data || data.kind !== 'candidate') return null;
        const symbol = data.symbol;
        const disabled = busy !== undefined || data.correlation === null;
        return (
          <button
            className="hedging-submit"
            type="button"
            disabled={disabled}
            onClick={() => void hedge(data.position, symbol, data.candidate)}
          >
            {busy === data.position.symbol ? 'Sending…' : 'Hedge ' + data.side}
          </button>
        );
      },
    },
  ], [expanded, busy]);

  return (
    <section className="feed-section close-insights-panel risk-controls" aria-label="Hedging">
      <FeedToolbar title="Hedging" />
      {error && <div className="positions-error" role="alert">{error}</div>}
      {positions.length === 0 ? (
        <p className="positions-empty">No open positions to hedge.</p>
      ) : (
        <div className="data-grid-wrap hedging-grid-wrap">
          <AgGridReact<HedgeGridRow>
            rowData={displayedRowData}
            columnDefs={columnDefs}
            getRowId={({ data }) => data.id}
            rowClassRules={{ 'hedging-child-row': ({ data }) => data?.kind === 'candidate' }}
            theme={gridTheme}
            domLayout="autoHeight"
            rowHeight={26}
            headerHeight={28}
            suppressColumnMoveAnimation
          />
        </div>
      )}
      <p className="hedging-hint">Cross-asset size accounts for correlation.</p>
    </section>
  );
}

export const Hedging = memo(HedgingComponent);
