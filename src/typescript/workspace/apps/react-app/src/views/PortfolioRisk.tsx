import { memo, useEffect, useMemo, useState } from 'react';
import { usePausedValue } from '../use_paused_value';
import { AgGridReact } from 'ag-grid-react';
import {
  ClientSideRowModelModule,
  ModuleRegistry,
  themeQuartz,
  type ColDef,
} from 'ag-grid-community';
import type { Connection } from '../connection';
import { PnlModel } from '../models/PnlModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import { FeedToolbar } from './FeedToolbar';

type HedgeCorrelationResponse = { pair_correlations?: { symbol_a: string; symbol_b: string; correlation: number | null }[]; positions?: { symbol: string; candidates?: { symbol: string; correlation: number | null }[] }[] };

ModuleRegistry.registerModules([ClientSideRowModelModule]);
const moneyFormatter = new Intl.NumberFormat(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 });
const riskGridTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});
type RiskGridRow = {
  symbol: string;
  netExposureUsd?: number;
  grossExposureUsd?: number;
  concentration?: number;
  stressLossUsd?: number;
  markAge?: number;
};
const defaultRiskColDef: ColDef<RiskGridRow> = {
  sortable: true,
  resizable: true,
  flex: 1,
  minWidth: 68,
};

function PortfolioRiskComponent({ connection, paused = false }: { connection: Connection; paused?: boolean }) {
  const [shockPct, setShockPct] = useState(() => Number(localStorage.getItem('pnl-risk-shock-pct')) || 5);
  const [lossLimit, setLossLimit] = useState(() => Number(localStorage.getItem('pnl-risk-loss-limit')) || 500);
  const [correlations, setCorrelations] = useState<Record<string, number | null>>({});
  const [correlationsLoaded, setCorrelationsLoaded] = useState(false);
  useEffect(() => {
    localStorage.setItem('pnl-risk-shock-pct', String(shockPct));
    localStorage.setItem('pnl-risk-loss-limit', String(lossLimit));
  }, [shockPct, lossLimit]);

  const model = useMemo(() => new PnlModel(connection, 'portfolio-risk-mark'), []);
  useEffect(() => { model.setConnection(connection); }, [model, connection]);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  const { rows, loading, error } = useBehaviorSnapshot(model.state$);
  useEffect(() => {
    let active = true;
    const loadCorrelations = async () => {
      try {
        const response = await fetch('/positions/hedge-candidates');
        if (!response.ok) throw new Error('Could not load correlations');
        const data = await response.json() as HedgeCorrelationResponse;
        if (!active) return;
        const next: Record<string, number | null> = {};
        for (const pair of data.pair_correlations ?? []) {
          next[[pair.symbol_a, pair.symbol_b].sort().join('|')] = pair.correlation;
        }
        for (const position of data.positions ?? []) {
          for (const candidate of position.candidates ?? []) {
            const key = [position.symbol, candidate.symbol].sort().join('|');
            if (next[key] == null && candidate.correlation != null) next[key] = candidate.correlation;
          }
        }
        setCorrelations(next);
        setCorrelationsLoaded(true);
      } catch {
        if (active) setCorrelationsLoaded(false);
      }
    };
    void loadCorrelations();
    const timer = setInterval(() => void loadCorrelations(), 15000);
    return () => { active = false; clearInterval(timer); };
  }, []);
  const openRows = useMemo(() => rows.filter((row) => Math.abs(row.position.quantity) > 1e-12), [rows]);
  const exposureAvailable = openRows.every((row) => row.exposureUsd !== undefined);
  const signedExposureUsd = openRows.map((row) =>
    (row.exposureUsd ?? 0) * Math.sign(row.position.quantity),
  );
  const netExposureUsd = signedExposureUsd.reduce((total, exposure) => total + exposure, 0);
  const grossExposureUsd = openRows.reduce((total, row) => total + (row.exposureUsd ?? 0), 0);
  const longExposureUsd = signedExposureUsd.filter((exposure) => exposure > 0).reduce((total, exposure) => total + exposure, 0);
  const shortExposureUsd = -signedExposureUsd.filter((exposure) => exposure < 0).reduce((total, exposure) => total + exposure, 0);
  const correlationPairCount = openRows.length * (openRows.length - 1) / 2;
  let availableCorrelationCount = 0;
  for (let i = 0; i < openRows.length; i += 1) {
    for (let j = i + 1; j < openRows.length; j += 1) {
      const correlation = correlations[[openRows[i].position.symbol, openRows[j].position.symbol].sort().join('|')];
      if (typeof correlation === 'number' && Number.isFinite(correlation)) availableCorrelationCount += 1;
    }
  }
  const correlationAdjustedExposure = exposureAvailable && correlationsLoaded
    ? Math.sqrt(Math.max(0, signedExposureUsd.reduce((variance, exposure, i) => {
      let result = variance + exposure * exposure;
      for (let j = i + 1; j < signedExposureUsd.length; j += 1) {
        const rawCorrelation = correlations[[openRows[i].position.symbol, openRows[j].position.symbol].sort().join('|')];
        const correlation = typeof rawCorrelation === 'number' && Number.isFinite(rawCorrelation)
          ? Math.max(-1, Math.min(1, rawCorrelation))
          : 0;
        result += 2 * correlation * exposure * signedExposureUsd[j];
      }
      return result;
    }, 0)))
    : undefined;
  const rowData = useMemo<RiskGridRow[]>(
    () => openRows.map((row) => ({
      symbol: row.position.symbol,
      netExposureUsd: row.exposureUsd === undefined ? undefined : row.exposureUsd * Math.sign(row.position.quantity),
      grossExposureUsd: row.exposureUsd,
      concentration: row.exposureUsd !== undefined && grossExposureUsd > 0 && exposureAvailable
        ? row.exposureUsd / grossExposureUsd * 100
        : undefined,
      stressLossUsd: row.exposureUsd === undefined ? undefined : row.exposureUsd * shockPct / 100,
      markAge: row.markTimestamp === undefined ? undefined : Math.max(0, Math.floor((Date.now() - row.markTimestamp) / 1000)),
    })),
    [openRows, grossExposureUsd, exposureAvailable, shockPct],
  );
  const displayedRowData = usePausedValue(rowData, paused);
  const columnDefs = useMemo<ColDef<RiskGridRow>[]>(
    () => [
      { field: 'symbol', headerName: 'Symbol', minWidth: 90 },
      { field: 'netExposureUsd', headerName: 'Net Exposure (USD)', minWidth: 140, cellClassRules: { 'pnl-positive': ({ value }) => typeof value === 'number' && value > 0, 'pnl-negative': ({ value }) => typeof value === 'number' && value < 0 }, valueFormatter: ({ value }) => value == null ? 'Unavailable' : (value > 0 ? '+' : '') + moneyFormatter.format(value) + ' USD' },
      { field: 'concentration', headerName: 'Share', minWidth: 76, valueFormatter: ({ value }) => value == null ? 'N/A' : value.toFixed(1) + '%' },
      { field: 'stressLossUsd', headerName: 'Stress (USD)', minWidth: 110, valueFormatter: ({ value }) => value == null ? 'N/A' : moneyFormatter.format(value) + ' USD' },
      { field: 'markAge', headerName: 'Mark', minWidth: 75, valueFormatter: ({ value }) => value == null ? 'Unavailable' : value + 's' },
    ],
    [],
  );

  return (
    <section className="feed-section pnl-panel portfolio-risk-panel" aria-label="Portfolio risk">
      <FeedToolbar title="Portfolio Risk" />
      {error && <div className="positions-error" role="alert">{error}</div>}
      <div className="positions-totals risk-controls">
        <label className="position-total-card">
          <span>STRESS MOVE (%)</span>
          <input type="number" min="0.1" max="100" step="0.1" value={shockPct} onChange={(event) => setShockPct(Math.min(100, Math.max(0.1, Number(event.target.value) || 0.1)))} />
        </label>
        <label className="position-total-card">
          <span>STRESS LOSS LIMIT (USD)</span>
          <input type="number" min="0" step="any" value={lossLimit} onChange={(event) => setLossLimit(Math.max(0, Number(event.target.value) || 0))} />
        </label>
        {openRows.length > 0 && (
          <div className="position-total-card">
            <div className="correlation-exposure-heading">
              <span>CORRELATION-ADJUSTED EXPOSURE</span>
              <button
                type="button"
                className="correlation-exposure-info"
                title={`Assumes equal proportional moves across open positions. Pair correlations adjust for diversification; missing pair correlations are treated as uncorrelated. ${correlationAdjustedExposure === undefined ? 'Waiting for exposure and correlation data.' : correlationPairCount === 0 ? 'Single position; no pair correlation needed.' : `${availableCorrelationCount}/${correlationPairCount} pair correlations available.`}`}
                aria-label="Correlation-adjusted exposure details"
              >i</button>
            </div>
            <strong>{correlationAdjustedExposure === undefined ? 'Unavailable' : moneyFormatter.format(correlationAdjustedExposure) + ' USD'}</strong>
            <small>
              Estimated stress {correlationAdjustedExposure === undefined ? 'N/A' : moneyFormatter.format(correlationAdjustedExposure * shockPct / 100) + ' USD'}
              {correlationAdjustedExposure !== undefined && correlationAdjustedExposure * shockPct / 100 >= lossLimit && lossLimit > 0 ? ' - LOSS LIMIT' : ''}
            </small>


          </div>
        )}
        {openRows.length > 0 && (
          <div className="position-total-card">
            <span>NET USD EXPOSURE</span>
            <strong className={netExposureUsd > 0 ? 'pnl-positive' : netExposureUsd < 0 ? 'pnl-negative' : undefined}>{exposureAvailable ? (netExposureUsd > 0 ? '+' : '') + moneyFormatter.format(netExposureUsd) + ' USD' : 'Marking positions...'}</strong>
            <small>
              Long {exposureAvailable ? moneyFormatter.format(longExposureUsd) : 'N/A'} USD · Short {exposureAvailable ? moneyFormatter.format(shortExposureUsd) : 'N/A'} USD · Gross stress {exposureAvailable ? moneyFormatter.format(grossExposureUsd * shockPct / 100) + ' USD' : 'N/A'}
              {exposureAvailable && grossExposureUsd * shockPct / 100 >= lossLimit && lossLimit > 0 ? ' - LOSS LIMIT' : ''}
            </small>
          </div>
        )}
      </div>
      <div className="data-grid-wrap pnl-grid-wrap">
        <AgGridReact<RiskGridRow>
          rowData={displayedRowData}
          columnDefs={columnDefs}
          defaultColDef={defaultRiskColDef}
          getRowId={({ data }) => data.symbol}
          theme={riskGridTheme}
          domLayout="autoHeight"
          rowHeight={26}
          headerHeight={28}
          suppressColumnMoveAnimation
        />
        {!loading && rowData.length === 0 && <p className="positions-empty">No open positions to assess.</p>}
        {loading && rowData.length === 0 && <p className="positions-empty">Loading positions...</p>}
      </div>
    </section>
  );
}

export const PortfolioRisk = memo(PortfolioRiskComponent);
