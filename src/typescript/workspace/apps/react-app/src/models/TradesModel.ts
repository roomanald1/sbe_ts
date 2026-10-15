import { BehaviorSubject, type Subscription } from 'rxjs';
import type { Connection, TradeFeed } from '../connection';
import { decodeTradeTick } from '../types/symbol_price';

export type RecentTrade = {
  tradeId: string;
  price: number;
  size: number;
  side: 'buy' | 'sell';
  timestamp: number;
};

export type TradesState = {
  symbol: string;
  trades: RecentTrade[];
  error?: string;
};

type Props = { symbol: string; connection: Connection; streamKey: string };

const TRADE_RETENTION_MS = 5 * 60 * 1000;

export class TradesModel {
  readonly state$: BehaviorSubject<TradesState>;
  private props: Props;
  private feed?: TradeFeed;
  private dataSub?: Subscription;
  private errorSub?: Subscription;
  private expiryTimer?: ReturnType<typeof setInterval>;

  constructor(props: Props) {
    this.props = props;
    this.state$ = new BehaviorSubject<TradesState>({
      symbol: props.symbol,
      trades: [],
    });
  }

  updateProps(props: Props) {
    const changed =
      this.props.connection !== props.connection ||
      this.props.symbol !== props.symbol ||
      this.props.streamKey !== props.streamKey;
    this.props = props;
    if (!changed) return;
    this.stop();
    this.state$.next({ symbol: props.symbol, trades: [] });
    this.start();
  }

  start() {
    if (this.feed) return;
    const { connection, symbol, streamKey } = this.props;
    const feed = connection.openTrades(streamKey, symbol);
    this.feed = feed;
    this.expiryTimer = setInterval(() => this.removeExpiredTrades(), 1000);
    this.dataSub = feed.data.subscribe((raw) => {
      if (raw === undefined) {
        this.state$.next({ symbol, trades: [] });
        return;
      }
      const trade = decodeTradeTick(raw);
      if (!trade || trade.symbol !== symbol) return;
      const cutoff = Date.now() - TRADE_RETENTION_MS;
      if (trade.timestamp < cutoff) return;
      this.state$.next({
        symbol,
        trades: [trade, ...this.state$.value.trades].filter(
          (item) => item.timestamp >= cutoff,
        ),
      });
    });
    this.errorSub = feed.error.subscribe((error) => {
      if (error) this.state$.next({ ...this.state$.value, error });
    });
  }

  private removeExpiredTrades() {
    const cutoff = Date.now() - TRADE_RETENTION_MS;
    const trades = this.state$.value.trades.filter(
      (trade) => trade.timestamp >= cutoff,
    );
    if (trades.length !== this.state$.value.trades.length) {
      this.state$.next({ ...this.state$.value, trades });
    }
  }

  stop() {
    if (!this.feed) return;
    if (this.expiryTimer) clearInterval(this.expiryTimer);
    this.expiryTimer = undefined;
    this.dataSub?.unsubscribe();
    this.errorSub?.unsubscribe();
    this.props.connection.closeTrades(this.feed.key, this.feed);
    this.feed = undefined;
    this.dataSub = undefined;
    this.errorSub = undefined;
  }
}
