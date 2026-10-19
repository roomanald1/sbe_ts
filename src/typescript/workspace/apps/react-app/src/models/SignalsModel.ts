import { BehaviorSubject, type Subscription } from 'rxjs';
import type { Connection, SignalFeed } from '../connection';

export type SignalComponents = {
  trend_1m_percent: number | null;
  momentum_10s_percent: number | null;
  trade_flow_imbalance: number | null;
  book_imbalance: number | null;
  volatility_1m_percent: number | null;
  trend_5m_percent?: number | null;
  trend_5m_volatility_units?: number | null;
  mispricing_vs_vwap_bps?: number | null;
  pressure_without_price_move?: number | null;
  aggressor_cost_bps?: number | null;
};
export type SignalSnapshot = {
  symbol: string;
  timestamp: number;
  direction: 'bullish' | 'neutral' | 'bearish' | 'ranging';
  ranging_probability?: number | null;
  range_lower_bound?: number | null;
  range_upper_bound?: number | null;
  score: number;
  strength: number;
  components: SignalComponents;
  reasons: string[];
  stale: boolean;
};
export type SignalsState = { symbol: string; snapshot?: SignalSnapshot; error?: string };
type Props = { connection: Connection; symbol: string; streamKey: string };

export class SignalsModel {
  readonly state$: BehaviorSubject<SignalsState>;
  private props: Props;
  private feed?: SignalFeed;
  private dataSub?: Subscription;
  private errorSub?: Subscription;
  constructor(props: Props) { this.props = props; this.state$ = new BehaviorSubject<SignalsState>({ symbol: props.symbol }); }
  updateProps(props: Props) {
    const changed = this.props.connection !== props.connection || this.props.symbol !== props.symbol || this.props.streamKey !== props.streamKey;
    this.props = props;
    if (changed) { this.stop(); this.state$.next({ symbol: props.symbol }); this.start(); }
  }
  start() {
    if (this.feed) return;
    const { connection, symbol, streamKey } = this.props;
    const feed = connection.openSignals(streamKey, symbol); this.feed = feed;
    this.dataSub = feed.data.subscribe((raw) => {
      if (raw === undefined) { this.state$.next({ symbol }); return; }
      try {
        const text = typeof raw === 'string' ? raw : new TextDecoder().decode(raw as Uint8Array);
        const snapshot = JSON.parse(text) as SignalSnapshot;
        if (snapshot.symbol === symbol) this.state$.next({ symbol, snapshot });
      } catch { this.state$.next({ ...this.state$.value, error: 'Received an invalid signal update.' }); }
    });
    this.errorSub = feed.error.subscribe((error) => { if (error) this.state$.next({ ...this.state$.value, error }); });
  }
  stop() {
    if (!this.feed) return;
    this.dataSub?.unsubscribe(); this.errorSub?.unsubscribe();
    this.props.connection.closeSignals(this.feed.key, this.feed);
    this.feed = undefined; this.dataSub = undefined; this.errorSub = undefined;
  }
}
