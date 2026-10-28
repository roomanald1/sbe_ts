export type FeeRates = { maker: number; taker: number };

export async function fetchFeeRates(): Promise<FeeRates | undefined> {
  try {
    const response = await fetch('/positions/fee-rates');
    if (!response.ok) return undefined;
    const rates = await response.json() as FeeRates;
    return Number.isFinite(rates.maker) && Number.isFinite(rates.taker) ? rates : undefined;
  } catch {
    return undefined;
  }
}
