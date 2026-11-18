import { useMemo, useState, useEffect } from 'react';
import { memo } from 'react';
import { gridTheme, ratio, useSignalInfo } from './SignalInfoViewShared';
import { usePausedValue } from '@workspace/ui-platform';
import { AgGridReact } from 'ag-grid-react';
import type { ColDef } from 'ag-grid-community';
import { FeedToolbar } from './FeedToolbar';
import type { SignalSnapshot } from '../models/SignalsModel';
import { insightsFetch } from '../insights_rsocket';

type ReplayRow = SignalSnapshot & { time: string; flow: number | null };
type SnapshotHistoryPage = {
  records: { snapshot: SignalSnapshot }[];
  nextCursor?: string | null;
  moreAvailable: boolean;
  rangeExhausted: boolean;
  archivedDaysUnscanned: number;
  archivedDaysUnavailable: number;
  corruptRecords: number;
};
type LoadedSnapshotHistory = {
  symbol: string;
  daysBack: number;
  snapshots: SignalSnapshot[];
};

function SignalReplayWidgetComponent({ paused = false }: { paused?: boolean }) {
  const state = useSignalInfo();
  const [loadedHistory, setLoadedHistory] = useState<SignalSnapshot[]>([]);
  const [replayHistory, setReplayHistory] = useState<LoadedSnapshotHistory>();
  const [loadError, setLoadError] = useState<string>();
  const [historyNotice, setHistoryNotice] = useState('');
  const [historyLoading, setHistoryLoading] = useState(false);
  useEffect(() => {
    let active = true;
    void insightsFetch('/signals/history')
      .then(async (response) => {
        if (!response.ok)
          throw new Error(
            `Signal history request failed (${response.status}).`,
          );
        const payload = (await response.json()) as SignalSnapshot[];
        if (active) {
          setLoadedHistory(payload);
          setLoadError(undefined);
        }
      })
      .catch((error: unknown) => {
        if (active)
          setLoadError(error instanceof Error ? error.message : String(error));
      });
    return () => {
      active = false;
    };
  }, []);
  const history = usePausedValue(
    loadedHistory.length > 0 ? loadedHistory : state.history,
    paused,
  );
  const [symbol, setSymbol] = useState('');
  const [windowSeconds, setWindowSeconds] = useState(60);
  const [daysBack, setDaysBack] = useState(7);
  const [minimumFlow, setMinimumFlow] = useState(0.15);
  const [minimumStrength, setMinimumStrength] = useState(0.3);
  const symbols = useMemo(
    () => [...new Set(history.map((item) => item.symbol))].sort(),
    [history],
  );
  const flowWindows = useMemo(() => {
    const windows = new Set(
      history.flatMap(
        (item) =>
          item.components.trade_flow_by_window?.map(
            (feature) => feature.window_seconds,
          ) ?? [],
      ),
    );
    if (history.some((item) => item.components.trade_flow_imbalance != null))
      windows.add(60);
    return [...windows].sort((a, b) => a - b);
  }, [history]);
  const selectedFlowWindow = flowWindows.includes(windowSeconds)
    ? windowSeconds
    : (flowWindows[0] ?? 60);
  const selectedSymbol = symbols.includes(symbol) ? symbol : (symbols[0] ?? '');
  useEffect(() => {
    if (!selectedSymbol) return;
    let active = true;
    const loadHistory = async () => {
      setHistoryLoading(true);
      setLoadError(undefined);
      setHistoryNotice('');
      const throughMs = Date.now();
      const fromMs = throughMs - daysBack * 24 * 60 * 60 * 1_000;
      const snapshots: SignalSnapshot[] = [];
      let cursor: string | undefined;
      let moreAvailable: boolean;
      let archivedDaysUnscanned = 0;
      let archivedDaysUnavailable = 0;
      let corruptRecords = 0;
      let pages = 0;
      do {
        const params = new URLSearchParams({
          symbol: selectedSymbol,
          fromMs: String(fromMs),
          throughMs: String(throughMs),
          limit: '512',
        });
        if (cursor) params.set('cursor', cursor);
        const response = await insightsFetch(
          `/internal/strategy-snapshot-history?${params.toString()}`,
        );
        if (!response.ok)
          throw new Error(
            `Could not load ${selectedSymbol} history (${response.status}).`,
          );
        const page = (await response.json()) as SnapshotHistoryPage;
        snapshots.push(...page.records.map((record) => record.snapshot));
        cursor = page.nextCursor ?? undefined;
        moreAvailable = page.moreAvailable;
        archivedDaysUnscanned = Math.max(
          archivedDaysUnscanned,
          page.archivedDaysUnscanned,
        );
        archivedDaysUnavailable = Math.max(
          archivedDaysUnavailable,
          page.archivedDaysUnavailable,
        );
        corruptRecords += page.corruptRecords;
        pages += 1;
      } while (moreAvailable && cursor && pages < 20);

      if (!active) return;
      setReplayHistory({ symbol: selectedSymbol, daysBack, snapshots });
      if (moreAvailable)
        setHistoryNotice(
          `Showing the first ${snapshots.length} records; more raw history is available.`,
        );
      else if (
        archivedDaysUnscanned ||
        archivedDaysUnavailable ||
        corruptRecords
      )
        setHistoryNotice(
          `Loaded ${snapshots.length} raw records. Coverage may omit ${archivedDaysUnscanned} archived days and ${archivedDaysUnavailable} unavailable days${corruptRecords ? `; ${corruptRecords} records could not be read` : ''}.`,
        );
      else
        setHistoryNotice(
          `Loaded ${snapshots.length} records for the last ${daysBack} days.`,
        );
      setHistoryLoading(false);
    };
    void loadHistory().catch((error: unknown) => {
      if (!active) return;
      setLoadError(error instanceof Error ? error.message : String(error));
      setHistoryLoading(false);
    });
    return () => {
      active = false;
    };
  }, [daysBack, selectedSymbol]);
  const rows = useMemo<ReplayRow[]>(() => {
    const snapshots =
      replayHistory?.symbol === selectedSymbol &&
      replayHistory.daysBack === daysBack
        ? replayHistory.snapshots
        : [];
    const selected = snapshots
      .filter((item) => item.symbol === selectedSymbol)
      .sort((a, b) => a.timestamp - b.timestamp);
    return selected.map((item) => {
      const windowed = item.components.trade_flow_by_window?.find(
        (feature) => feature.window_seconds === selectedFlowWindow,
      );
      return {
        ...item,
        time: new Date(item.timestamp).toLocaleString(),
        flow:
          windowed?.value ??
          (selectedFlowWindow === 60
            ? item.components.trade_flow_imbalance
            : null),
      };
    });
  }, [daysBack, replayHistory, selectedFlowWindow, selectedSymbol]);
  const baselineEntries = rows.filter(
    (row) =>
      (row.direction === 'bullish' || row.direction === 'bearish') &&
      row.strength >= 0.3 &&
      row.components.trade_flow_imbalance != null &&
      (row.direction === 'bullish'
        ? row.components.trade_flow_imbalance
        : -row.components.trade_flow_imbalance) >= 0.15,
  ).length;
  const replayEntries = rows.filter(
    (row) =>
      (row.direction === 'bullish' || row.direction === 'bearish') &&
      row.strength >= minimumStrength &&
      row.flow != null &&
      (row.direction === 'bullish' ? row.flow : -row.flow) >= minimumFlow,
  ).length;
  const columns = useMemo<ColDef<ReplayRow>[]>(
    () => [
      { field: 'time', headerName: 'Time', minWidth: 150 },
      { field: 'direction', headerName: 'Direction', minWidth: 100 },
      {
        field: 'components.trade_flow_imbalance',
        headerName: 'Stored flow',
        valueGetter: ({ data }) => data?.components.trade_flow_imbalance,
        valueFormatter: ({ value }) => ratio(value),
        minWidth: 110,
      },
      {
        field: 'flow',
        headerName: `Candidate ${selectedFlowWindow}s flow`,
        valueFormatter: ({ value }) => ratio(value),
        minWidth: 140,
      },
      {
        colId: 'decision',
        headerName: 'Candidate decision',
        valueGetter: ({ data }) => {
          if (!data || !['bullish', 'bearish'].includes(data.direction))
            return 'Reject';
          if (data.strength < minimumStrength || data.flow == null)
            return 'Reject';
          const aligned =
            data.direction === 'bullish' ? data.flow : -data.flow!;
          return aligned >= minimumFlow ? 'Enter' : 'Reject';
        },
        minWidth: 130,
      },
    ],
    [minimumFlow, minimumStrength, selectedFlowWindow],
  );
  return (
    <section
      className="feed-section signal-info-panel"
      aria-label="Trade-flow what-if"
    >
      <FeedToolbar title="Trade-flow entry what-if" />
      <p className="signal-info-note">
        Each row is a saved signal snapshot. Its recorded direction stays fixed;
        the candidate settings only recheck the flow and strength entry gates.
        Baseline uses 60s flow, 15% aligned flow, and 30% strength. This shows
        how many snapshots pass each gate, not trades or profit and loss.
      </p>
      <div className="signal-replay-controls">
        <label>
          Symbol
          <select
            value={selectedSymbol}
            onChange={(event) => setSymbol(event.target.value)}
          >
            {symbols.map((value) => (
              <option key={value} value={value}>
                {value}
              </option>
            ))}
          </select>
        </label>
        <label>
          Flow window
          <select
            value={selectedFlowWindow}
            onChange={(event) => setWindowSeconds(Number(event.target.value))}
          >
            {flowWindows.map((value) => (
              <option key={value} value={value}>
                {value}s
              </option>
            ))}
          </select>
        </label>
        <label>
          History range
          <select
            value={daysBack}
            onChange={(event) => setDaysBack(Number(event.target.value))}
          >
            {[1, 7, 30, 90].map((value) => (
              <option key={value} value={value}>
                {value} days
              </option>
            ))}
          </select>
        </label>
        <label>
          Minimum aligned flow
          <input
            type="number"
            min="0.02"
            max="0.8"
            step="0.01"
            value={minimumFlow}
            onChange={(event) => setMinimumFlow(Number(event.target.value))}
          />
        </label>
        <label>
          Minimum strength
          <input
            type="number"
            min="0"
            max="1"
            step="0.05"
            value={minimumStrength}
            onChange={(event) => setMinimumStrength(Number(event.target.value))}
          />
        </label>
        <span>
          {rows.length} snapshots checked · baseline passes {baselineEntries} ·
          candidate passes {replayEntries}
        </span>
      </div>
      {historyLoading && (
        <p className="signal-info-note">Loading historical signal records…</p>
      )}
      {historyNotice && <p className="signal-info-note">{historyNotice}</p>}
      {(loadError || state.historyError) && (
        <div className="positions-error">{loadError || state.historyError}</div>
      )}
      <div className="data-grid-wrap signal-info-grid">
        <AgGridReact<ReplayRow>
          rowData={rows}
          columnDefs={columns}
          defaultColDef={{
            sortable: true,
            resizable: true,
            flex: 1,
            minWidth: 90,
          }}
          theme={gridTheme}
          domLayout="autoHeight"
          rowHeight={26}
          headerHeight={28}
        />
      </div>
      {rows.length === 0 && (
        <p className="positions-empty">
          No stored signal snapshots for this symbol.
        </p>
      )}
    </section>
  );
}

export const SignalReplayWidget = memo(SignalReplayWidgetComponent);
