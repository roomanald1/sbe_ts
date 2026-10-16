import { BehaviorSubject, type Subscription } from 'rxjs';
import type { BookFeed, Connection, TradeFeed } from '../connection';
import { decodeBookDepth, decodeTradeTick, type BookDepth, type TradeTick } from '../types/symbol_price';
import { getLiquidityBands, isUnusualTrade, type LiquidityBand } from '../market_liquidity';

type Props = { symbol: string; connection: Connection; streamKey: string };
type Sample = { timestamp: number; bands: LiquidityBand[] };
export type MarketLiquidityState = { symbol: string; book?: BookDepth; bands: LiquidityBand[]; trends: Array<'rising' | 'falling' | 'steady' | 'warming'>; unusualTrades: TradeTick[]; error?: string };
const HISTORY_MS = 5 * 60_000;
const MAX_TRADES = 100;
export class MarketLiquidityModel {
  readonly state$: BehaviorSubject<MarketLiquidityState>;
  private props: Props;
  private bookFeed?: BookFeed;
  private tradeFeed?: TradeFeed;
  private subscriptions: Subscription[] = [];
  private history: Sample[] = [];
  private trades: TradeTick[] = [];
  constructor(props: Props) { this.props = props; this.state$ = new BehaviorSubject<MarketLiquidityState>({ symbol: props.symbol, bands: [], trends: [] as MarketLiquidityState['trends'], unusualTrades: [] }); }
  updateProps(props: Props) {
    const changed = this.props.connection !== props.connection || this.props.symbol !== props.symbol || this.props.streamKey !== props.streamKey;
    if (!changed) { this.props = props; return; }
    this.stop();
    this.props = props;
    this.history = [];
    this.trades = [];
    this.state$.next({ symbol: props.symbol, bands: [], trends: [], unusualTrades: [] });
    this.start();
  }
  start() {
    if (this.bookFeed) return;
    const { connection, symbol, streamKey } = this.props;
    const book = connection.openBook(streamKey + ':book', symbol); this.bookFeed = book;
    this.subscriptions.push(book.data.subscribe((raw) => { if (raw === undefined) { this.state$.next({ ...this.state$.value, book: undefined, bands: [] }); return; } const depth = decodeBookDepth(raw); if (!depth || depth.symbol !== symbol) return; const bands = getLiquidityBands(depth), now = Date.now(); this.history.push({ timestamp: now, bands }); this.history = this.history.filter((sample) => sample.timestamp >= now - HISTORY_MS); const prior = [...this.history].reverse().find((sample) => sample.timestamp <= now - 60_000); const trends = bands.map((band, index) => { const old = prior?.bands[index]; if (!old) return 'warming' as const; const delta = band.imbalance - old.imbalance; return Math.abs(delta) < 0.03 ? 'steady' as const : delta > 0 ? 'rising' as const : 'falling' as const; }); this.state$.next({ ...this.state$.value, symbol, book: depth, bands, trends, error: undefined }); }));
    this.subscriptions.push(book.error.subscribe((error) => { if (error) this.state$.next({ ...this.state$.value, error }); }));
    const trade = connection.openTrades(streamKey + ':trades', symbol); this.tradeFeed = trade;
    this.subscriptions.push(trade.data.subscribe((raw) => { const tick = decodeTradeTick(raw); if (!tick || tick.symbol !== symbol) return; const previous = this.trades; const unusual = isUnusualTrade(tick, previous); this.trades = [tick, ...previous].slice(0, MAX_TRADES); if (unusual) this.state$.next({ ...this.state$.value, unusualTrades: [tick, ...this.state$.value.unusualTrades].slice(0, 20) }); }));
    this.subscriptions.push(trade.error.subscribe((error) => { if (error) this.state$.next({ ...this.state$.value, error }); }));
  }
  stop() { this.subscriptions.forEach((sub) => sub.unsubscribe()); this.subscriptions = []; if (this.bookFeed) this.props.connection.closeBook(this.bookFeed.key, this.bookFeed); if (this.tradeFeed) this.props.connection.closeTrades(this.tradeFeed.key, this.tradeFeed); this.bookFeed = undefined; this.tradeFeed = undefined; }
}
