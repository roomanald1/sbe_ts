import { useEffect, useState } from 'react';
import { AgGridReact } from 'ag-grid-react';
import {
  ClientSideRowModelModule,
  ModuleRegistry,
  themeQuartz,
  type ColDef,
} from 'ag-grid-community';
import { autoTraderRpc } from '../auto_trader_rsocket';
import type { AutoTraderStrategyProfile } from '../models/AutoTraderModel';

ModuleRegistry.registerModules([ClientSideRowModelModule]);
const walkForwardTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});

type WalkForwardRow = {
  id: string;
  symbol: string;
  strategy: string;
  samples: number;
  winRate: number | null;
  averageNet: number | null;
  expectancy: string;
  effectiveSamples: number | null;
  windows: number | null;
  maxDrawdown: number | null;
  meanDownside: number | null;
  lastSample: string;
};

const walkForwardColumns: ColDef<WalkForwardRow>[] = [
  { field: 'symbol', headerName: 'Symbol', pinned: 'left', width: 92 },
  { field: 'strategy', headerName: 'Strategy', width: 126 },
  { field: 'samples', headerName: 'Samples', width: 68 },
  {
    field: 'winRate',
    headerName: 'Win rate',
    width: 78,
    valueFormatter: ({ value }) => (value == null ? '—' : formatPercent(value)),
  },
  {
    field: 'averageNet',
    headerName: 'Avg net',
    width: 80,
    valueFormatter: ({ value }) => formatReturn(value),
    cellClass: ({ value }) =>
      value > 0
        ? 'walk-forward-positive'
        : value < 0
          ? 'walk-forward-negative'
          : '',
  },
  { field: 'expectancy', headerName: 'Expectancy 95% CI', width: 138 },
  {
    field: 'effectiveSamples',
    headerName: 'Eff. samples',
    width: 85,
    valueFormatter: ({ value }) => value?.toFixed(1) ?? '—',
  },
  { field: 'windows', headerName: 'Windows', width: 68 },
  {
    field: 'maxDrawdown',
    headerName: 'Max drawdown',
    width: 95,
    valueFormatter: ({ value }) => formatReturn(value),
  },
  {
    field: 'meanDownside',
    headerName: 'Mean downside',
    width: 96,
    valueFormatter: ({ value }) => formatReturn(value),
  },
  { field: 'lastSample', headerName: 'Last sample', width: 150 },
];

const formatPercent = (value?: number | null) =>
  value == null || !Number.isFinite(value)
    ? '—'
    : `${(value * 100).toFixed(1)}%`;
const formatReturn = (value?: number | null) =>
  value == null || !Number.isFinite(value)
    ? '—'
    : `${value > 0 ? '+' : ''}${value.toFixed(3)}%`;

export function WalkForwardProfiles({
  profiles,
}: {
  profiles: AutoTraderStrategyProfile[];
}) {
  return (
    <section className="feed-section" aria-label="Walk-forward state">
      {profiles.length === 0 ? (
        <div className="book-popup-message">
          No allowed strategy profiles are available.
        </div>
      ) : (
        <div className="walk-forward-list">
          <div className="data-grid-wrap walk-forward-grid">
            <AgGridReact<WalkForwardRow>
              rowData={profiles.map((profile) => {
                const evidence = profile.walk_forward_evidence;
                return {
                  id: `${profile.symbol}:${profile.strategy}`,
                  symbol: profile.symbol,
                  strategy: profile.strategy,
                  samples: evidence?.sampleCount ?? 0,
                  winRate: evidence?.positiveNetRate ?? null,
                  averageNet: evidence?.averageNetReturnPercent ?? null,
                  expectancy:
                    evidence?.expectancyCiLowPercent == null ||
                    evidence.expectancyCiHighPercent == null
                      ? '—'
                      : `${formatReturn(evidence.expectancyCiLowPercent)} to ${formatReturn(evidence.expectancyCiHighPercent)}`,
                  effectiveSamples: evidence?.effectiveSampleSize ?? null,
                  windows: evidence?.windowCount ?? null,
                  maxDrawdown: evidence?.maxDrawdownPercent ?? null,
                  meanDownside: evidence?.downsideLossPercent ?? null,
                  lastSample: evidence?.lastSampleTimestampMs
                    ? new Date(evidence.lastSampleTimestampMs).toLocaleString()
                    : '—',
                };
              })}
              columnDefs={walkForwardColumns}
              getRowId={({ data }) => data.id}
              defaultColDef={{ sortable: true, resizable: true }}
              maintainColumnOrder
              theme={walkForwardTheme}
              domLayout="normal"
              rowHeight={22}
              headerHeight={24}
            />
          </div>
          {profiles.map((profile) => {
            const regimes = Object.entries(
              profile.walk_forward_evidence_by_regime ?? {},
            );
            return regimes.length ? (
              <div
                className="walk-forward-regimes"
                key={`${profile.symbol}:${profile.strategy}:regimes`}
              >
                <b>
                  {profile.symbol} {profile.strategy}
                </b>
                {regimes.map(([regime, value]) => (
                  <span key={regime}>
                    {regime}: {value.sampleCount} samples ·{' '}
                    {formatPercent(value.positiveNetRate)} ·{' '}
                    {formatReturn(value.averageNetReturnPercent)}
                  </span>
                ))}
              </div>
            ) : null;
          })}
        </div>
      )}
    </section>
  );
}

export function WalkForwardWidget() {
  const [profiles, setProfiles] = useState<AutoTraderStrategyProfile[]>([]);
  const [error, setError] = useState<string>();

  useEffect(() => {
    let active = true;
    const refresh = async () => {
      try {
        const response = await autoTraderRpc(
          'GET',
          '/positions/auto-trader/parameters',
        );
        if (!response.ok)
          throw new Error(`Walk-forward request failed (${response.status}).`);
        const data = (await response.json()) as {
          strategy_profiles?: AutoTraderStrategyProfile[];
        };
        if (active) {
          setProfiles(data.strategy_profiles ?? []);
          setError(undefined);
        }
      } catch (reason) {
        if (active)
          setError(
            reason instanceof Error
              ? reason.message
              : 'Could not load walk-forward state.',
          );
      }
    };
    void refresh();
    const timer = setInterval(() => void refresh(), 30_000);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, []);

  if (error && profiles.length === 0) {
    return (
      <section className="feed-section" aria-label="Walk-forward state">
        <div className="book-popup-message">{error}</div>
      </section>
    );
  }
  return <WalkForwardProfiles profiles={profiles} />;
}
