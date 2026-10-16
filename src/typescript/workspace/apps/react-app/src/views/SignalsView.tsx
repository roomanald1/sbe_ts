import { useEffect, useMemo } from 'react';
import type { Connection } from '../connection';
import { SignalsModel } from '../models/SignalsModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import { FeedToolbar } from './FeedToolbar';

type Props = { symbol: string; symbols: string[]; connection: Connection; streamKey: string; onSymbolChange: (symbol: string) => void };
const percent = (value?: number | null) => value == null ? 'Warming up' : value.toFixed(3) + '%';
const imbalance = (value?: number | null) => value == null ? 'Warming up' : (value * 100).toFixed(1) + '%';
export function SignalsView({ symbol, symbols, connection, streamKey, onSymbolChange }: Props) {
  const model = useMemo(() => new SignalsModel({ connection, symbol, streamKey }), []);
  const state = useBehaviorSnapshot(model.state$);
  useEffect(() => { model.updateProps({ connection, symbol, streamKey }); }, [model, connection, symbol, streamKey]);
  useEffect(() => { model.start(); return () => model.stop(); }, [model]);
  const signal = state.snapshot;
  return <section className="feed-section signal-panel" aria-label="Trading signals">
    <FeedToolbar title="Backend signals">
      <select className="chart-symbol-select" aria-label="Signal instrument" value={symbol} onChange={(event) => onSymbolChange(event.target.value)}>
        {symbols.map((item) => <option key={item} value={item}>{item}</option>)}
      </select>
    </FeedToolbar>
    {state.error && <div className="book-popup-message">{state.error}</div>}
    {!signal ? <div className="book-popup-message">Waiting for backend signal data…</div> : <>
      <div className={"signal-direction signal-" + signal.direction}><strong>{signal.direction.toUpperCase()}</strong><span>Score {signal.score.toFixed(2)} · strength {(signal.strength * 100).toFixed(0)}%</span></div>
      <div className="signal-components">
        <div><span>Trend · 1m</span><strong>{percent(signal.components.trend_1m_percent)}</strong></div>
        <div><span>Momentum · 10s</span><strong>{percent(signal.components.momentum_10s_percent)}</strong></div>
        <div><span>Trade flow · 1m</span><strong>{imbalance(signal.components.trade_flow_imbalance)}</strong></div>
        <div><span>Book pressure</span><strong>{imbalance(signal.components.book_imbalance)}</strong></div>
        <div><span>Realized volatility · 1m</span><strong>{percent(signal.components.volatility_1m_percent)}</strong></div>
      </div>
      <ul className="signal-reasons">{signal.reasons.map((reason) => <li key={reason}>{reason}</li>)}</ul>
      <small>{signal.stale ? 'Market data is stale' : 'Updated'} · {new Date(signal.timestamp).toLocaleTimeString()}</small>
    </>}
  </section>;
}
