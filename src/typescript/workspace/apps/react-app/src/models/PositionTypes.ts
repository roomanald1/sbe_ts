export type PositionTrade = {
  id: string;
  symbol: string;
  side: 'buy' | 'sell';
  quantity: number;
  execution_price: number;
  timestamp: number;
};
export type Position = {
  symbol: string;
  quantity: number;
  average_entry_price: number;
  realized_pnl: number;
  last_trade_timestamp: number;
};
