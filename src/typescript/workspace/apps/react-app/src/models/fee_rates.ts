import { currentPositionSnapshot } from '../position_stream';

export type FeeRates = { maker: number; taker: number };

export async function fetchFeeRates(): Promise<FeeRates | undefined> {
  const rates = currentPositionSnapshot()?.fee_rates;
  return rates && Number.isFinite(rates.maker) && Number.isFinite(rates.taker) ? rates : undefined;
}
