import type { ReactNode } from 'react';
import type { Connection, ConnectionState, FeedSource } from '../connection';
import type { PriceChanges } from '../price_change_data';
import type { DashboardWidget } from './dashboard_layout';
import { SymbolPriceGrid } from './SymbolPriceGrid';
import { SymbolPriceTable } from './SymbolPriceTable';
import { PriceChangeChart } from './PriceChangeChart';
import { Positions } from './Positions';
import { Fills } from './Fills';
import { Pnl } from './Pnl';
import { Account } from './Account';
import { PortfolioRisk } from './PortfolioRisk';
import { CloseInsights } from './CloseInsights';
import { Hedging } from './Hedging';
import { AutoTrader } from './AutoTrader';
import { DatedCarry } from './DatedCarry';
import { AutoTraderDiagnostics } from './AutoTraderDiagnostics';
import { AutoTraderOutcomes } from './AutoTraderOutcomes';
import { OrderBookPopup } from './OrderBookPopup';
import { TradesView } from './TradesView';
import { NewsTicker } from './NewsTicker';
import { MarketLiquidity } from './MarketLiquidity';
import { BestSignalsView, MissedOpportunityBandsView, SignalHistoryView, SignalPerformanceView, WalkForwardView } from './SignalInfoViews';

type Props = {
  widget: DashboardWidget;
  symbol: string;
  prices: PriceChanges;
  connection: Connection;
  feedSource: FeedSource;
  connectionState: ConnectionState;
  selectionSymbols: string[];
  paused: boolean;
  onTrade: (symbol: string, side?: 'buy' | 'sell') => void;
  onRangeOrders: (request: { symbol: string; lower: number; upper: number }) => void;
  onSymbolChange: (widgetId: string, symbol: string) => void;
};

export function DashboardWidgetContent(props: Props): ReactNode {
  const { widget, symbol, prices, connection, feedSource, connectionState, selectionSymbols, paused, onTrade, onRangeOrders, onSymbolChange } = props;
  const updateSymbol = (value: string) => onSymbolChange(widget.id, value);
  switch (widget.type) {
    case 'grid': return <SymbolPriceGrid ticks={prices.ticks} feedSource={feedSource} compact onSelectSymbol={onTrade} />;
    case 'table': return <SymbolPriceTable ticks={prices.ticks} histories={prices.histories} state={connectionState} onSelectSymbol={onTrade} />;
    case 'chart': return <PriceChangeChart changes={prices} selectedSymbol={symbol} onSymbolChange={updateSymbol} showSymbolSelector onTrade={(side) => symbol && onTrade(symbol, side)} />;
    case 'positions': return <Positions connection={connection} symbols={selectionSymbols} paused={paused} />;
    case 'fills': return <Fills paused={paused} />;
    case 'pnl': return <Pnl connection={connection} paused={paused} />;
    case 'account': return <Account />;
    case 'risk': return <PortfolioRisk connection={connection} paused={paused} />;
    case 'close-insights': return <CloseInsights histories={prices.histories} paused={paused} onTradeSymbol={onTrade} />;
    case 'hedging': return <Hedging paused={paused} />;
    case 'auto-trader': return <AutoTrader />;
    case 'dated-carry': return <DatedCarry />;
    case 'auto-trader-diagnostics': return <AutoTraderDiagnostics />;
    case 'auto-trader-outcomes': return <AutoTraderOutcomes ticks={prices.ticks} />;
    case 'auto-trader-live-positions': return <AutoTraderOutcomes ticks={prices.ticks} liveOnly />;
    case 'trades': return feedSource === 'okx' ? <TradesView streamKey={widget.id} symbol={symbol} symbols={selectionSymbols} onSymbolChange={updateSymbol} connection={connection} paused={paused} /> : <UnavailableFeed label="Recent trades" message="Recent trade data is available with the OKX feed." />;
    case 'liquidity': return feedSource === 'okx' ? <MarketLiquidity streamKey={widget.id} symbol={symbol} symbols={selectionSymbols} onSymbolChange={updateSymbol} connection={connection} paused={paused} /> : <UnavailableFeed label="Market liquidity" message="Market liquidity indicators are available with the OKX feed." />;
    case 'signals': return <BestSignalsView onTradeSymbol={onTrade} onRangeOrders={onRangeOrders} />;
    case 'news': return <NewsTicker connection={connection} streamKey={widget.id} />;
    case 'signal-history': return <SignalHistoryView paused={paused} onTradeSymbol={onTrade} />;
    case 'signal-performance': return <SignalPerformanceView paused={paused} onTradeSymbol={onTrade} />;
    case 'walk-forward': return <WalkForwardView paused={paused} onTradeSymbol={onTrade} />;
    case 'missed-opportunity-bands': return <MissedOpportunityBandsView paused={paused} />;
    case 'book': return feedSource === 'okx' ? <OrderBookPopup streamKey={widget.id} mode="view" symbol={symbol} symbols={selectionSymbols} onSymbolChange={updateSymbol} showSymbolSelector onTrade={onTrade} connection={connection} /> : <UnavailableFeed label="Order book" message="Order book depth is available with the OKX feed." />;
  }
}

function UnavailableFeed({ label, message }: { label: string; message: string }) {
  return <section className="feed-section" aria-label={label}><div className="book-popup-message">{message}</div></section>;
}
