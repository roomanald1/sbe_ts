import { useMemo, useState } from 'react';
import type { AutoTraderStrategyProfile, Strategy } from '../models/AutoTraderModel';

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

  return <section className="auto-trader-strategy-parameters" aria-label="Per-strategy learning parameters">
    <strong>Strategy learning parameters</strong>
    <p>Each symbol and strategy adapts its own rule values from recent outcomes. Values stay within service-defined bounds; shared risk controls and walk-forward exit estimates remain active until this profile has enough outcomes.</p>
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
      <small>{profile.learning_samples} recent-weighted outcomes · {profile.positive_rate == null ? 'positive rate unavailable' : (profile.positive_rate * 100).toFixed(1) + '% positive'} · learning starts after 10 outcomes · set Outcome adaptation weight to 0 to freeze this profile</small>
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
