import { useMemo, useState } from 'react';
import type { AutoTraderStrategyProfile, Strategy } from '../models/AutoTraderModel';

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

type Props = {
  profiles: AutoTraderStrategyProfile[];
  overrides: Record<string, number>;
  saving: boolean;
  onChange: (key: string, value?: number) => void;
  onSave: () => void;
  onResetProfile: (symbol: string, strategy: Strategy) => void;
};

export function AutoTraderStrategyParameters({ profiles, overrides, saving, onChange, onSave, onResetProfile }: Props) {
  const symbols = useMemo(() => [...new Set(profiles.map((profile) => profile.symbol))], [profiles]);
  const [symbol, setSymbol] = useState('BTC-USDT');
  const strategyOptions = useMemo(() => profiles.filter((profile) => profile.symbol === symbol), [profiles, symbol]);
  const [strategy, setStrategy] = useState<Strategy>('donchian-55-btc200');
  const profile = strategyOptions.find((item) => item.strategy === strategy) ?? strategyOptions[0];

  return <section className="auto-trader-strategy-parameters" aria-label="Strategy parameters and scouting">
    <strong>Strategy parameters and scouting</strong>
    <p>Each symbol and strategy adapts its own rule values from recent outcomes. Early updates are smoothed, values stay within service-defined bounds, and shared risk controls remain active.</p>
    <div className="auto-trader-parameter-selectors">
      <label>SYMBOL<select value={symbols.includes(symbol) ? symbol : (symbols[0] ?? '')} onChange={(event) => setSymbol(event.target.value)}>
        {symbols.map((item) => <option key={item} value={item}>{item}</option>)}
      </select></label>
      <label>STRATEGY<select value={profile?.strategy ?? strategy} onChange={(event) => setStrategy(event.target.value as Strategy)}>
        {strategyOptions.map((item) => <option key={item.strategy} value={item.strategy}>{item.strategy}</option>)}
      </select></label>
    </div>
    {!profile && <p>No tunable parameters are registered for this strategy.</p>}
    {profile && <>
      <small>{profile.learning_samples} recent-weighted closed trades · {profile.positive_rate == null ? 'positive rate unavailable' : (profile.positive_rate * 100).toFixed(1) + '% positive'} · parameters adapt after each completed position · set Outcome adaptation weight to 0 to freeze this profile</small>
      <div className="auto-trader-parameter-list">
        {profile.parameters.map((parameter) => {
          const key = profile.symbol + ':' + profile.strategy + ':' + parameter.key;
          const configured = overrides[key];
          const value = configured ?? parameter.default_value;
          return <label key={key} className="auto-trader-parameter-input">
            <span>{parameter.label}<small>{parameter.unit} · active {parameter.learned_value.toFixed(3)} · default {parameter.default_value.toFixed(3)} · range {parameter.min}–{parameter.max} · step {parameter.step}{configured === undefined ? '' : ' · custom baseline ' + configured.toFixed(3)}</small></span>
            <input type="number" min={parameter.min} max={parameter.max} step={parameter.step} value={value} onChange={(event) => {
              const next = Number(event.target.value);
              const stepPosition = (next - parameter.min) / parameter.step;
              if (Number.isFinite(next) && next >= parameter.min && next <= parameter.max && Math.abs(stepPosition - Math.round(stepPosition)) < 1e-6) onChange(key, next);
            }} />
            <button type="button" disabled={configured === undefined} onClick={() => onChange(key)}>Use learned value</button>
          </label>;
        })}
      </div>
      <div className="auto-trader-profile-actions">
        <button type="button" disabled={saving} onClick={onSave}>Save strategy parameters</button>
        <button type="button" disabled={saving || (profile.learning_samples === 0 && !profile.parameters.some((parameter) => parameter.overridden))} onClick={() => onResetProfile(profile.symbol, profile.strategy)}>Reset this profile's learning</button>
      </div>
    </>}
  </section>;
}


export function AutoTraderScoutOptions({ scoutStrategies, saving, onChange }: { scoutStrategies: Strategy[]; saving: boolean; onChange: (strategies: Strategy[]) => void }) {
  return <fieldset className="auto-trader-strategy-options" title="Scouts evaluate eligible signals in shadow mode. They cannot place trades, even when the strategy is disabled for trading.">
    <legend>SCOUTS · SHADOW ONLY</legend>
    {scoutableStrategies.map(({ strategy, label }) => <label key={strategy}><input type="checkbox" disabled={saving} checked={scoutStrategies.includes(strategy)} onChange={(event) => onChange(event.target.checked ? [...scoutStrategies, strategy] : scoutStrategies.filter((item) => item !== strategy))} />{label}</label>)}
    <small>Choose strategies to evaluate as Scouts. This works independently of the strategy trading checkboxes above.</small>
  </fieldset>;
}
