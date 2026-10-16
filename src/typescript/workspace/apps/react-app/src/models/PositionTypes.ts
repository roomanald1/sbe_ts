export type PositionTrade = {
  id: string;
  symbol: string;
  side: 'buy' | 'sell';
  quantity: number;
  execution_price: number;
  reference_price?: number;
  timestamp: number;
  fee?: number;
  fee_currency?: string;
  fee_rate?: number;
  fee_liquidity?: 'maker' | 'taker';
};
export type Position = {
  symbol: string;
  quantity: number;
  average_entry_price: number;
  realized_pnl: number;
  entry_fees: number;
  fees_paid: number;
  estimated_exit_fee_rate: number;
  last_trade_timestamp: number;
};



