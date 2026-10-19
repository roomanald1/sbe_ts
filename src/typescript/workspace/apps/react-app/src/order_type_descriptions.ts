export const orderTypeDescriptions = {
  market: 'Executes immediately against the best available book prices; any amount beyond displayed liquidity may remain unfilled.',
  ioc: 'Executes immediately for any available quantity and cancels the unfilled remainder.',
  limit: 'Rests at your limit price and executes only at that price or better.',
  'post-only-limit': 'Adds liquidity only. Rejected if it would execute immediately; canceled if it later becomes marketable.',
  fok: 'Must fill the full quantity immediately or the entire order is canceled.',
  gtd: 'A limit order that is automatically canceled at the selected date and time.',
  'stop-market': 'Watches the trigger price, then submits an immediate-or-cancel market order when triggered.',
  'stop-limit': 'Watches the trigger price, then places a limit order when triggered.',
  'trailing-stop': 'Moves its trigger with the market by the selected percentage or price amount, then submits an IOC order.',
  barrier: 'Activates when the selected trigger price is crossed, then follows the configured execution behavior.',
  twap: 'Splits the quantity into evenly sized child orders over the selected duration.',
  vwap: 'Schedules child orders against a provisional volume curve over the selected duration and participation limit.',
} as const;
export function getOrderTypeDescription(value: unknown): string {
  const raw = String(value ?? '').trim();
  const key = raw
    .replace(/([a-z0-9])([A-Z])/g, '-')
    .toLowerCase()
    .replace(/[ _]+/g, '-')
    .replace(/-(exit|hedge)$/, '');
  const normalizedKey = key === 'gtd-limit' ? 'gtd' : key;
  return orderTypeDescriptions[normalizedKey as keyof typeof orderTypeDescriptions] ?? raw;
}
