import type { Strategy } from '../models/AutoTraderModel';

export const scoutableStrategies: Array<{ strategy: Strategy; label: string }> = [
  { strategy: 'live-momentum', label: 'Live momentum' },
  { strategy: 'live-trade-flow', label: 'Live trade flow' },
  { strategy: 'live-order-book', label: 'Live order book' },
  { strategy: 'live-vwap-dislocation', label: 'Live VWAP dislocation' },
  { strategy: 'live-absorption-pressure', label: 'Live absorption pressure' },
  { strategy: 'donchian-55-btc200', label: '55-day Donchian breakout' },
  { strategy: 'donchian-20-sma200', label: '20-day Donchian · asset SMA200' },
  { strategy: 'donchian-20-btc200', label: '20-day Donchian · BTC SMA200' },
  { strategy: 'donchian-10-btc200', label: '10-day Donchian · BTC SMA200' },
  { strategy: 'donchian-20-btc200-hold3', label: '20-day Donchian · BTC SMA200 · 3-day minimum hold' },
  { strategy: 'risk-adjusted-trend-portfolio', label: 'Risk-adjusted trend portfolio' },
  { strategy: 'trend-sleeves-btc200', label: 'Trend sleeves' },
  { strategy: 'trend-50-200', label: 'Trend 50/200' },
  { strategy: 'relative-strength-90-sma200', label: '90-day relative strength' },
  { strategy: 'risk-adjusted-monthly-momentum', label: 'Risk-adjusted monthly momentum' },
  { strategy: 'rsi2-oversold-rebound', label: 'RSI2 oversold rebound' },
  { strategy: 'dated-futures-carry', label: 'Dated futures carry' },
];

export function AutoTraderScoutOptions({ scoutStrategies, saving, onChange }: { scoutStrategies: Strategy[]; saving: boolean; onChange: (strategies: Strategy[]) => void }) {
  return <fieldset className="auto-trader-strategy-options" title="Scouts evaluate eligible signals without submitting orders, even when the strategy is disabled for trading.">
    <legend>SCOUTS ONLY</legend>
    {scoutableStrategies.map(({ strategy, label }) => <label key={strategy}><input type="checkbox" disabled={saving} checked={scoutStrategies.includes(strategy)} onChange={(event) => onChange(event.target.checked ? [...scoutStrategies, strategy] : scoutStrategies.filter((item) => item !== strategy))} />{label}</label>)}
    <small>Choose strategies to evaluate as Scouts. This works independently of the strategy trading checkboxes above.</small>
  </fieldset>;
}
