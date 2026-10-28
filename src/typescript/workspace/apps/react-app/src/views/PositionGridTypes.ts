export type TradeSide = 'buy' | 'sell';
export const quoteCurrency = (symbol: string) => symbol.split('-')[1] ?? '';

export type PositionGridRow = {
  id: string;
  kind: 'order' | 'vwap' | 'barrier';
  orderType: string;
  createdAt: number;
  expiresAt?: number;
  orderTimeInForce?: 'gtc' | 'gtd';
  status: string;
  symbol: string;
  side: TradeSide;
  executed: number;
  remaining: number;
  limitPrice?: number;
  triggerDirection?: 'above' | 'below';
  executionType?: 'market' | 'limit';
  activationLimit?: number;
  estimatedFee?: number;
  current?: number;
  quote: string;
  bookError?: string;
  error?: string;
  progress?: number;
  etaMinutes?: number;
  executionVwap?: number;
  marketVwap?: number;
  slippage?: number;
};

