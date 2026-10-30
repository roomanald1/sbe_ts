import { useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { AutoTraderModel } from '../models/AutoTraderModel';
import type { Strategy } from '../models/AutoTraderModel';
import { autoTraderShadowModel } from '../models/AutoTraderShadowModel';
import { AutoTraderStrategyParameters } from './AutoTraderStrategyParameters';
import { AutoTraderOutcomes } from './AutoTraderOutcomes';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import { FeedToolbar } from './FeedToolbar';
import { explainAutoTraderReason, money, timestamp } from './AutoTraderOrderGrid';
const finiteFixed = (value: unknown, digits: number) => typeof value === 'number' && Number.isFinite(value) ? value.toFixed(digits) : 'n/a';
const intradayStrategyLabels: Array<{ strategy: Strategy; label: string }> = [
  { strategy: 'live-insights', label: 'Live insights' },
  { strategy: 'live-momentum', label: 'Live momentum' },
  { strategy: 'live-trade-flow', label: 'Live trade flow' },
  { strategy: 'live-order-book', label: 'Live order book' },
  { strategy: 'live-vwap-dislocation', label: 'Live VWAP dislocation' },
  { strategy: 'live-absorption-pressure', label: 'Live absorption pressure' },
];
const dailyStrategyLabels: Array<{ strategy: Strategy; label: string }> = [
  { strategy: 'donchian-55-btc200', label: '55-day Donchian breakout' },
  { strategy: 'donchian-20-sma200', label: '20-day Donchian · asset SMA200' },
  { strategy: 'donchian-20-btc200', label: '20-day Donchian · BTC SMA200' },
  { strategy: 'trend-sleeves-btc200', label: 'Trend sleeves' },
  { strategy: 'trend-50-200', label: 'Trend 50/200' },
  { strategy: 'relative-strength-90-sma200', label: '90-day relative strength' },
  { strategy: 'risk-adjusted-monthly-momentum', label: 'Risk-adjusted monthly momentum' },
  { strategy: 'rsi2-oversold-rebound', label: 'RSI2 oversold rebound' },
];
export function AutoTrader() {
  const model = useMemo(() => new AutoTraderModel(), []);
  useEffect(() => { model.start(); return () => model.stop(); }, [model]);
  const state = useBehaviorSnapshot(model.state$);
  const shadowState = useBehaviorSnapshot(autoTraderShadowModel.state$);
  useEffect(() => { autoTraderShadowModel.start(); return () => autoTraderShadowModel.stop(); }, []);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [parametersOpen, setParametersOpen] = useState(false);
  const [decisionDetailsOpen, setDecisionDetailsOpen] = useState(false);
  const scoutOrderCounts = useMemo(() => {
    const scouts = state.orders.filter((order) => order.purpose === 'scout');
    return {
      total: scouts.length,
      filled: scouts.filter((order) => order.filled_quantity > 0).length,
      pending: scouts.filter((order) => ['accepted', 'resting', 'armed', 'submitting'].includes(order.status)).length,
      failed: scouts.filter((order) => order.status === 'failed').length,
    };
  }, [state.orders]);
  const scoutShadowRows = shadowState.rows.filter((row) => row.source === 'Scout');
  const scoutShadowOpen = scoutShadowRows.filter((row) => row.status === 'Open').length;
  const scoutShadowResolved = scoutShadowRows.filter((row) => ['Won', 'Lost', 'Flat'].includes(row.status));
  const scoutShadowWins = scoutShadowRows.filter((row) => row.status === 'Won').length;
  const scoutShadowLosses = scoutShadowRows.filter((row) => row.status === 'Lost').length;
  const scoutShadowUnavailable = scoutShadowRows.filter((row) => row.status === 'Outcome unavailable').length;
  const scoutShadowNetReturns = scoutShadowResolved.flatMap((row) => typeof row.netReturnPercent === 'number' && Number.isFinite(row.netReturnPercent) ? [row.netReturnPercent] : []);
  const scoutShadowAverageReturn = scoutShadowNetReturns.length ? scoutShadowNetReturns.reduce((sum, value) => sum + value, 0) / scoutShadowNetReturns.length : undefined;
  const scoutOutcomeCohort = state.decisionQuality.cohorts.find((cohort) => cohort.dimension === 'selection' && cohort.value === 'scout');
  const selectedIntradayStrategies = intradayStrategyLabels.filter(({ strategy }) => state.strategies.includes(strategy));
  const selectedIntradayBudgetValuesValid = selectedIntradayStrategies.every(({ strategy }) => {
    const value = state.intradayStrategyBudgets[strategy] ?? '';
    return value.trim() !== '' && Number.isFinite(Number(value)) && Number(value) >= 0;
  });
  const selectedIntradayBudgetTotal = selectedIntradayStrategies.reduce((total, { strategy }) => total + (Number(state.intradayStrategyBudgets[strategy]) || 0), 0);
  const intradayBudgetOverallocated = !selectedIntradayBudgetValuesValid
    || selectedIntradayBudgetTotal > Number(state.intradayNotionalLimit) + 1e-9;
  const selectedDailyStrategies = dailyStrategyLabels.filter(({ strategy }) => state.strategies.includes(strategy));
  const selectedDailyBudgetValuesValid = selectedDailyStrategies.every(({ strategy }) => {
    const value = state.dailyStrategyBudgets[strategy] ?? '';
    return value.trim() !== '' && Number.isFinite(Number(value)) && Number(value) >= 0;
  });
  const selectedDailyBudgetTotal = selectedDailyStrategies.reduce((total, { strategy }) => total + (Number(state.dailyStrategyBudgets[strategy]) || 0), 0);
  const dailyBudgetOverallocated = !selectedDailyBudgetValuesValid
    || selectedDailyBudgetTotal > Number(state.dailyStrategyNotionalLimit) + 1e-9;
  return (
    <section className={"feed-section auto-trader-section" + (settingsOpen || parametersOpen ? " has-open-popup" : "")} aria-label="Auto Trader">
      <FeedToolbar title="Auto Trader" />
      <div className="auto-trader-content">
        <div className={"auto-trader-live-banner " + (state.enabled ? "is-live" : "") + (state.loading ? " is-loading" : "")}>
          <span className="auto-trader-live-dot" aria-hidden="true" />
          <strong>{state.loading ? 'Connecting' : state.enabled ? 'LIVE' : 'PAUSED'}</strong>
          <span>{state.loading ? 'Checking status' : state.enabled ? 'Auto Trader is active' : 'Auto Trader is off'}</span>
          <div className="auto-trader-live-actions">
            <button type="button" disabled={state.loading || state.busy} onClick={() => void model.setEnabled(!state.enabled)}>{state.busy ? 'Working…' : state.enabled ? 'Pause' : 'Go live'}</button>
          </div>
        </div>
        <div className="auto-trader-settings-anchor">
          <p>Paper · {state.serverExplorationPercent}% explore · SL {finiteFixed(state.stopLossPercent, 2)}% / TP {finiteFixed(state.takeProfitPercent, 2)}%</p>
          <div className="auto-trader-settings-actions">
            <button className="auto-trader-parameters-button" type="button" aria-expanded={parametersOpen} aria-haspopup="dialog" disabled={state.loading} onClick={() => { setParametersOpen((open) => !open); setSettingsOpen(false); }}>View parameters</button>
            <button className="auto-trader-settings-button" type="button" aria-expanded={settingsOpen} aria-haspopup="dialog" disabled={state.loading} onClick={() => { setSettingsOpen((open) => !open); setParametersOpen(false); }}>…</button>
          </div>
          {parametersOpen && createPortal(<div className="app-popup-backdrop auto-trader-popup-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) setParametersOpen(false); }}><div className="auto-trader-settings-popup auto-trader-parameters-popup" role="dialog" aria-modal="true" aria-label="Current Auto Trader parameters">
            <div className="auto-trader-settings-heading"><strong>Current Auto Trader parameters</strong><button type="button" aria-label="Close parameters" onClick={() => setParametersOpen(false)}>×</button></div>
            <small aria-live="polite">Live parameters from the AutoTrader service · refreshed {state.decisionParameters ? timestamp.format(state.decisionParameters.refreshed_at) : 'on open and every 5s'}</small>
            {state.decisionParametersError && <p className="auto-trader-error" role="alert">{state.decisionParametersError}</p>}
            {!state.decisionParameters && !state.decisionParametersError && <p>Loading live decision parameters…</p>}
            {state.decisionParameters && <div className="auto-trader-parameter-sections">
              <section><strong>Shared risk controls</strong><dl>
                <div><dt>Risk appetite</dt><dd>{Number(state.riskAppetite)} · {Number(state.riskAppetite) < 34 ? 'Safe' : Number(state.riskAppetite) > 66 ? 'Risky' : 'Balanced'}</dd></div>
                <div><dt>Maximum per order</dt><dd>{money.format(Number(state.maxOrderNotional))}</dd></div>
                <div><dt>Maximum allocated exposure</dt><dd>{money.format(Number(state.dailyNotionalLimit))}</dd></div>
                <div><dt>Daily loss cap</dt><dd>{Number(state.dailyRealizedLossLimit).toLocaleString()}</dd></div>
                <div><dt>Maximum open positions</dt><dd>{state.maxOpenPositions}</dd></div>
                <div><dt>Maximum entries per position</dt><dd>{state.maxEntriesPerPosition}</dd></div>
                <div><dt>Exploration allocation</dt><dd>{state.explorationPercent}%</dd></div>
                <div><dt>Protective stop / target</dt><dd>{finiteFixed(state.stopLossPercent, 2)}% / {finiteFixed(state.takeProfitPercent, 2)}%</dd></div>
              </dl></section>
              <section><strong>Service decision parameters</strong><dl>
                {state.decisionParameters.thresholds.map((parameter) => <div key={parameter.section + parameter.name}><dt>{parameter.section} · {parameter.name}</dt><dd>{Number.isInteger(parameter.value) ? parameter.value : finiteFixed(parameter.value, 4)} {parameter.unit}</dd></div>)}
              </dl></section>
              <AutoTraderStrategyParameters profiles={state.decisionParameters.strategy_profiles} overrides={state.strategyParameterOverrides} saving={state.busy} onChange={(key, value) => model.setStrategyParameter(key, value)} onSave={() => void model.saveLimits()} onResetProfile={(symbol, strategy) => void model.resetStrategyProfile(symbol, strategy)} />
              <section><strong>Learned symbol and strategy profiles</strong>
                {state.decisionParameters.contexts.length === 0 ? <p>No closed outcomes have produced learned profiles yet.</p> : <dl>
                  {state.decisionParameters.contexts.map((profile) => <div key={profile.context}><dt>{profile.context}</dt><dd>{profile.learning_samples} outcomes · {profile.positive_rate == null ? 'win rate n/a' : finiteFixed(profile.positive_rate * 100, 1) + '% positive'} · strength ≥ {finiteFixed(profile.strength_floor, 3)} · score ≥ {finiteFixed(profile.exploration_score_floor, 3)}{profile.exploration_allowed ? '' : ' · exploration disabled'}</dd></div>)}
                </dl>}
              </section>
            </div>}
          </div></div>, document.body)}
          {settingsOpen && createPortal(<div className="app-popup-backdrop auto-trader-popup-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) setSettingsOpen(false); }}><div className="auto-trader-settings-popup" role="dialog" aria-modal="true" aria-label="Auto Trader settings">
            <div className="auto-trader-settings-heading"><strong>Auto Trader settings</strong><button type="button" aria-label="Close settings" onClick={() => setSettingsOpen(false)}>×</button></div>
            <div className="auto-trader-limits">
              <fieldset className="auto-trader-strategy-options" title="Live and candle-derived insight strategies share the same AutoTrader portfolio exposure, daily loss, and open-position limits."><legend>STRATEGIES · SHARED PORTFOLIO LIMITS</legend>
                {([{ value: 'live-momentum', label: 'Live momentum · aligned 10-second momentum or 5-minute trend; configured entry order; shared protective stop/target' }, { value: 'live-trade-flow', label: 'Live trade flow · aligned imbalance ≥ 0.15; configured entry order; shared protective stop/target' }, { value: 'live-order-book', label: 'Live order book · aligned imbalance ≥ 0.15; configured entry order; shared protective stop/target' }, { value: 'live-vwap-dislocation', label: 'Live VWAP dislocation · aligned gap ≥ 1 bp; configured entry order; shared protective stop/target' }, { value: 'live-absorption-pressure', label: 'Live absorption · opposing pressure ≥ 0.2; configured entry order; shared protective stop/target' }, { value: 'donchian-55-btc200', label: '55-day Donchian breakout · trade highs only while BTC is above its SMA200' }, { value: 'donchian-20-sma200', label: '20-day Donchian breakout · buy assets above their own SMA200' }, { value: 'donchian-20-btc200', label: '20-day Donchian + BTC SMA200 · fixed 20/200 daily signal exit · no protective stop · paper research' }, { value: 'trend-sleeves-btc200', label: 'Trend sleeves · BTC/ETH SMA50 + BNB SMA100 · BTC SMA200 risk gate' }, { value: 'trend-50-200', label: 'Trend following · hold each asset above its SMA50 and SMA200' }, { value: 'relative-strength-90-sma200', label: 'Momentum rotation · hold the strongest 90-day asset above its SMA200' }, { value: 'risk-adjusted-monthly-momentum', label: 'Risk-adjusted monthly momentum · BTC SMA200 gate · 40% volatility target · signal exit, no fixed stop' }, { value: 'rsi2-oversold-rebound', label: 'Mean reversion · RSI(2) ≤ 10 above SMA200 · five-day max hold' }, { value: 'dated-futures-carry', label: 'Dated futures carry · Insights signal, paired BTC/ETH paper hedge, settlement-window exit' }] as Array<{ value: Strategy; label: string }>).map(({ value, label }) => <label key={value}><input type="checkbox" checked={state.strategies.includes(value)} onChange={(event) => model.setStrategies(event.target.checked ? [...state.strategies, value] : state.strategies.filter((strategy) => strategy !== value))} />{label}</label>)}
                <small>Intraday and daily strategies have editable per-strategy budget caps and separate position-slot limits. Learning adjusts effective strategy budgets and candidates with stronger measured net outcomes are submitted first when history is sufficient. Orders use shared account, exposure, and daily-loss limits; per-strategy P&L is tracked separately. The 20-day BTC-gated breakout and risk-adjusted monthly momentum use signal exits without fixed stops. Dated futures carry has its own paper size cap.</small>
                {state.strategies.length === 0 && <small className="auto-trader-strategy-validation">Select at least one strategy before saving.</small>}
              </fieldset>
              <label>MAX PER ORDER (USD)<input type="number" min="0.01" step="any" value={state.maxOrderNotional} onChange={(event) => model.setLimits(event.target.value, state.dailyNotionalLimit, state.dailyRealizedLossLimit, state.explorationPercent, state.maxOpenPositions)} /></label>
              <label title="Maximum USD notional allocated to Auto Trader positions and pending orders that add exposure. Opening positions uses available capacity; closing fills release it. Manual positions are not included.">MAX ALLOCATED EXPOSURE (USD)<input type="number" min="0.01" step="any" value={state.dailyNotionalLimit} onChange={(event) => model.setLimits(state.maxOrderNotional, event.target.value, state.dailyRealizedLossLimit, state.explorationPercent, state.maxOpenPositions)} /></label>
              <label>DAILY LOSS CAP (USD)<input type="number" min="0.01" step="any" value={state.dailyRealizedLossLimit} onChange={(event) => model.setLimits(state.maxOrderNotional, state.dailyNotionalLimit, event.target.value, state.explorationPercent, state.maxOpenPositions)} /></label>
              <label>EXPLORATION SIZE (%)<input type="number" min="0" max="100" step="1" value={state.explorationPercent} onChange={(event) => model.setLimits(state.maxOrderNotional, state.dailyNotionalLimit, state.dailyRealizedLossLimit, event.target.value, state.maxOpenPositions)} /></label>
              <label className="auto-trader-risk-slider" title="Shifts strength and score floors and adjusts take-profit. Riskier settings lower entry floors and move take-profit toward a 0.05% net win after fees; safer settings raise floors and preserve larger targets.">RISK APPETITE <span className="auto-trader-risk-value">{Number(state.riskAppetite)} · {Number(state.riskAppetite) < 34 ? 'Safe' : Number(state.riskAppetite) > 66 ? 'Risky' : 'Balanced'}</span><input aria-label="Auto Trader risk appetite" type="range" min="0" max="100" step="1" value={state.riskAppetite} onChange={(event) => model.setRiskAppetite(event.target.value)} /><span className="auto-trader-risk-scale"><span>Safe · higher floors</span><span>Risky · lower floors</span></span></label>
              <label title="Maximum number of distinct Auto Trader positions held at once.">MAX OPEN POSITIONS<input type="number" min="1" max="20" step="1" value={state.maxOpenPositions} onChange={(event) => model.setLimits(state.maxOrderNotional, state.dailyNotionalLimit, state.dailyRealizedLossLimit, state.explorationPercent, event.target.value)} /></label>
              <label title="Maximum number of fee-free Scout and Exploratory shadow samples being evaluated at once. This does not limit or create real positions.">MAX CONCURRENT SHADOW SAMPLES<input type="number" min="0" max="100" step="1" value={state.maxConcurrentShadowSamples} onChange={(event) => model.setMaxConcurrentShadowSamples(event.target.value)} /></label>
              <fieldset className="auto-trader-category-limits" title="Intraday signal and daily strategy budgets are separate; the overall position and exposure caps still apply."><legend>HORIZON BUDGETS</legend>
                <label>INTRADAY POSITIONS<input type="number" min="0" max="20" step="1" value={state.maxIntradayOpenPositions} onChange={(event) => model.setCategoryLimits({ maxIntradayOpenPositions: event.target.value })} /></label>
                <label>DAILY POSITIONS<input type="number" min="0" max="20" step="1" value={state.maxDailyOpenPositions} onChange={(event) => model.setCategoryLimits({ maxDailyOpenPositions: event.target.value })} /></label>
                <label>INTRADAY BUDGET (USD)<input type="number" min="0" step="any" value={state.intradayNotionalLimit} onChange={(event) => model.setCategoryLimits({ intradayNotionalLimit: event.target.value })} /></label>
                <label>DAILY BUDGET (USD)<input type="number" min="0" step="any" value={state.dailyStrategyNotionalLimit} onChange={(event) => model.setCategoryLimits({ dailyStrategyNotionalLimit: event.target.value })} /></label>
                <small>Overall limits still apply. Each symbol can belong to one horizon at a time.</small>
              </fieldset>
              {selectedIntradayStrategies.length > 0 && <details className="auto-trader-strategy-budgets">
                <summary className={intradayBudgetOverallocated ? 'has-error' : ''}>
                  <strong>INTRADAY STRATEGY BUDGETS</strong>
                  <span>{money.format(selectedIntradayBudgetTotal)} / {money.format(Number(state.intradayNotionalLimit))} USD · {selectedIntradayStrategies.length} strategies{intradayBudgetOverallocated ? ' · fix allocations' : ''}</span>
                </summary>
                <div className="auto-trader-strategy-budget-grid">
                  {selectedIntradayStrategies.map(({ strategy, label }) => {
                    const learning = state.intradayStrategyLearning.find((item) => item.strategy === strategy);
                    const outcome = state.closedStrategyOutcomes[strategy];
                    const learnedSummary = 'Learning ×' + finiteFixed(learning?.learning_multiplier ?? 1, 2) + ' · ' + (outcome ? outcome.trades + ' closed trades · ' + finiteFixed(outcome.winRate, 1) + '% wins · avg return ' + finiteFixed(outcome.averageReturn, 3) + '%' : 'no closed trades');
                    return <div className="auto-trader-strategy-budget-row" key={strategy}>
                      <label>{label}<input aria-label={label + ' base budget in USD'} type="number" min="0" step="any" value={state.intradayStrategyBudgets[strategy] ?? '0'} onChange={(event) => model.setIntradayStrategyBudget(strategy, event.target.value)} /></label>
                      <small>{learnedSummary} · effective cap {money.format(learning?.effective_budget ?? Number(state.intradayStrategyBudgets[strategy] ?? 0))} · used {money.format(learning?.used_notional ?? 0)}</small>
                    </div>;
                  })}
                  <small>Learning scales the effective cap. Closed trade counts, win rates, and average returns match the completed positions table.</small>
                </div>
              </details>}
              {selectedDailyStrategies.length > 0 && <details className="auto-trader-strategy-budgets">
                <summary className={dailyBudgetOverallocated ? 'has-error' : ''}>
                  <strong>DAILY STRATEGY BUDGETS</strong>
                  <span>{money.format(selectedDailyBudgetTotal)} / {money.format(Number(state.dailyStrategyNotionalLimit))} USD · {selectedDailyStrategies.length} strategies{dailyBudgetOverallocated ? ' · fix allocations' : ''}</span>
                </summary>
                <div className="auto-trader-strategy-budget-grid">
                  {selectedDailyStrategies.map(({ strategy, label }) => {
                    const learning = state.dailyStrategyLearning.find((item) => item.strategy === strategy);
                    const outcome = state.closedStrategyOutcomes[strategy];
                    const learnedSummary = 'Learning ×' + finiteFixed(learning?.learning_multiplier ?? 1, 2) + ' · ' + (outcome ? outcome.trades + ' closed trades · ' + finiteFixed(outcome.winRate, 1) + '% wins · avg return ' + finiteFixed(outcome.averageReturn, 3) + '%' : 'no closed trades');
                    return <div className="auto-trader-strategy-budget-row" key={strategy}>
                      <label>{label}<input aria-label={label + ' base budget in USD'} type="number" min="0" step="any" value={state.dailyStrategyBudgets[strategy] ?? '0'} onChange={(event) => model.setDailyStrategyBudget(strategy, event.target.value)} /></label>
                      <small>{learnedSummary} · effective cap {money.format(learning?.effective_budget ?? Number(state.dailyStrategyBudgets[strategy] ?? 0))} · used {money.format(learning?.used_notional ?? 0)}</small>
                    </div>;
                  })}
                  <small>Learning scales the effective cap. Closed trade counts, win rates, and average returns match the completed positions table.</small>
                </div>
              </details>}
              <label title="Maximum number of entries Auto Trader can make while a position remains open, including the initial entry.">ENTRIES PER POSITION<input type="number" min="1" max="10" step="1" value={state.maxEntriesPerPosition} onChange={(event) => model.setMaxEntriesPerPosition(event.target.value)} /></label>
              <label>VWAP / TWAP DURATION (MIN)<input type="number" min="5" max="390" step="1" value={state.vwapDurationMinutes} onChange={(event) => model.setExecutionParameters({ vwapDurationMinutes: event.target.value })} /></label>
              <label>VWAP PARTICIPATION (%)<input type="number" min="1" max="50" step="1" value={state.vwapParticipationPercent} onChange={(event) => model.setExecutionParameters({ vwapParticipationPercent: event.target.value })} /></label>
              <label>LIMIT OFFSET (BPS)<input type="number" min="-10" max="10" step="0.5" value={state.limitOffsetBps} onChange={(event) => model.setExecutionParameters({ limitOffsetBps: event.target.value })} /></label>
              <label>TRIGGER OFFSET (BPS)<input type="number" min="5" max="100" step="1" value={state.triggerOffsetBps} onChange={(event) => model.setExecutionParameters({ triggerOffsetBps: event.target.value })} /></label>
              <label>TRAILING DISTANCE (%)<input type="number" min="0.05" max="1" step="0.05" value={state.trailingDistancePercent} onChange={(event) => model.setExecutionParameters({ trailingDistancePercent: event.target.value })} /></label>
              <label title="Number of most recent fully closed exits used for rolling exit statistics. Range: 1–5000.">ROLLING CLOSED EXITS<input type="number" min="1" max="5000" step="1" value={state.closedTradeWindow} onChange={(event) => model.setClosedTradeWindow(event.target.value)} /></label>
              <button type="button" disabled={state.busy || state.loading || (state.strategies.length === 0) || dailyBudgetOverallocated || intradayBudgetOverallocated} onClick={() => void model.saveLimits()}>Save settings</button>
              <button type="button" disabled={state.busy || state.loading} onClick={() => void model.resetLearning()}>Reset learned strategy parameters</button>
            </div>
          </div></div>, document.body)}
        </div>
        <p className="auto-trader-allocation">Reserved <strong>USD {money.format(state.usedNotionalToday)}</strong> / {money.format(state.serverDailyNotionalLimit)} total · Intraday {money.format(state.intradayUsedNotional)}/{money.format(Number(state.intradayNotionalLimit))} · Daily {money.format(state.dailyStrategyUsedNotional)}/{money.format(Number(state.dailyStrategyNotionalLimit))} · Available <strong>USD {money.format(state.availableNotional)}</strong>{state.usedNotionalToday > state.serverDailyNotionalLimit && <span> · Overall cap exceeded; new exposure blocked.</span>}</p>
        <div className="auto-trader-insights">
          <div className="auto-trader-insight auto-trader-decision" role="status">
            <span className="auto-trader-insight-label">No trades · {state.decisionsSinceTrade.toLocaleString()} scans</span>
            {state.gapReasons.length > 0 ? <details className="auto-trader-gap-details">
              <summary>Reasons · {state.gapReasons.length}</summary>
              <ol className="auto-trader-gap-reasons">
                {state.gapReasons.slice().sort((a, b) => b.count - a.count).slice(0, 3).map((item) => <li key={item.reason}>
                  <span>{explainAutoTraderReason(item.reason)}</span><strong>{item.count.toLocaleString()}</strong>
                </li>)}
              </ol>
            </details> : <span className="auto-trader-decision-text">{state.lastDecision ? explainAutoTraderReason(state.lastDecision) : 'Waiting for the first decision.'}</span>}

            {state.lastDecisionAt > 0 && <time dateTime={new Date(state.lastDecisionAt).toISOString()}>{timestamp.format(state.lastDecisionAt)}</time>}
          </div>
          <div className="auto-trader-insight">
            <span className="auto-trader-insight-label">Learning · today UTC</span>
            <strong>{state.learningSamples} closed trades today · {state.learningWinRate == null ? '—' : finiteFixed(state.learningWinRate * 100, 0) + '% positive'}</strong>
            <span className="auto-trader-learning-detail">Execution samples P0/P1/P2: {state.parameterProfileSamples.join('/')} {state.preferredOrderType ? '· Lowest observed cost: ' + state.preferredOrderType : ''}</span>
          </div>
          <div className="auto-trader-insight auto-trader-scout-summary" aria-label="Scout shadow outcomes">
            <span className="auto-trader-insight-label">Scout · shadow · 5m · last 15m</span>
            <strong>{shadowState.loading ? 'Loading shadow samples…' : `${scoutShadowOpen} open · ${scoutShadowResolved.length} resolved`}</strong>
            <span className="auto-trader-learning-detail">{scoutShadowWins} wins · {scoutShadowLosses} losses · {scoutShadowUnavailable} outcome unavailable</span>
            <span className="auto-trader-learning-detail">{scoutShadowAverageReturn === undefined ? 'No matched fee-aware outcomes yet.' : `${scoutShadowAverageReturn >= 0 ? '+' : ''}${finiteFixed(scoutShadowAverageReturn, 3)}% average net return · ${scoutShadowNetReturns.length} matched outcomes`}</span>
          </div>
          <div className="auto-trader-insight auto-trader-scout-summary" aria-label="Executed Scout positions and trades">
            <span className="auto-trader-insight-label">Scout · executed · actual trades</span>
            <strong>{state.scoutOpenPositions} open · {state.decisionQuality.scout_completed_trade_samples} closed</strong>
            <span className="auto-trader-learning-detail">{state.decisionQuality.scout_filled_entry_samples} filled · {state.decisionQuality.scout_unresolved_trade_samples} unresolved</span>
            <span className="auto-trader-learning-detail">Orders: {scoutOrderCounts.filled} filled · {scoutOrderCounts.pending} pending · {scoutOrderCounts.failed} failed</span>
            {scoutOutcomeCohort ? <span className="auto-trader-learning-detail">Closed: {scoutOutcomeCohort.completed_trade_samples} · {scoutOutcomeCohort.average_net_return_percent >= 0 ? '+' : ''}{finiteFixed(scoutOutcomeCohort.average_net_return_percent, 2)}% avg · {finiteFixed(scoutOutcomeCohort.win_rate * 100, 0)}% wins{scoutOutcomeCohort.total_net_pnl == null ? '' : ' · ' + scoutOutcomeCohort.pnl_quote_currency + ' ' + money.format(scoutOutcomeCohort.total_net_pnl)}</span> : <span className="auto-trader-learning-detail">{state.scoutLearningSamples > 0 ? state.scoutLearningSamples + ' closed · ' + (state.scoutLearningWinRate == null ? 'win rate n/a' : finiteFixed(state.scoutLearningWinRate * 100, 0) + '% positive') : 'No scout outcomes closed today.'}</span>}
          </div>
          <div className="auto-trader-insight auto-trader-decision-quality">
            <span className="auto-trader-insight-label">Decisions · today · {state.decisionQuality.completed_trade_samples}/{state.decisionQuality.filled_entry_samples} closed</span>
            <strong>{state.decisionQuality.decision_samples.toLocaleString()} decisions · {state.decisionQuality.unresolved_trade_samples} open</strong>
            <button className="auto-trader-cohort-toggle" type="button" aria-expanded={decisionDetailsOpen} onClick={() => setDecisionDetailsOpen((open) => !open)}>
              {decisionDetailsOpen ? 'Hide breakdown' : 'Breakdown · ' + state.decisionQuality.cohorts.length}
            </button>
            {decisionDetailsOpen && <div className="auto-trader-learning-detail auto-trader-cohort-list">
              {state.decisionQuality.cohorts.map((cohort) => <span key={cohort.dimension + ':' + cohort.value}>
                {cohort.dimension === 'entry_time_utc' ? 'Entry time (UTC)' : cohort.dimension === 'market_regime' ? 'Market regime' : cohort.dimension.split('_').join(' ')}: {cohort.value} · {cohort.completed_trade_samples} trades · {cohort.average_net_return_percent >= 0 ? '+' : ''}{finiteFixed(cohort.average_net_return_percent, 2)}% avg · {finiteFixed(cohort.win_rate * 100, 0)}% wins{cohort.preliminary ? ' · preliminary' : ''}
              </span>)}
              {state.decisionQuality.cohorts.length === 0 && <span>Completed, linked outcomes will appear here as trades close.</span>}
              {state.decisionQuality.unpriced_closed_trade_samples > 0 && <span>{state.decisionQuality.unpriced_closed_trade_samples} closed outcomes lack an entry price and are excluded.</span>}
              {state.decisionQuality.history_truncated && <span>Based on the latest 50,000 audit updates.</span>}
            </div>}
          </div>
          <div className="auto-trader-insight">
            <span className="auto-trader-insight-label">Closed today · {state.exitLearningSamples}</span>
            <strong>{state.closedTradeAverageReturnPercent == null ? 'Average return unavailable' : (state.closedTradeAverageReturnPercent >= 0 ? '+' : '') + finiteFixed(state.closedTradeAverageReturnPercent, 3) + '%'}</strong>
            <span className="auto-trader-learning-detail">{state.closedTradeWins} wins · {state.closedTradeLosses} losses · {state.closedTradeFlat} near flat</span>
          </div>
        </div>
        <div className="auto-trader-pnl-summary" aria-label="Auto Trader profit and loss">
          <span>Realized today: <strong className={state.realizedPnl == null ? undefined : state.realizedPnl >= 0 ? 'auto-trader-pnl-positive' : 'auto-trader-pnl-negative'}>{state.realizedPnl == null ? 'Unavailable (position ledger or FX rate)' : 'USD ' + money.format(state.realizedPnl)}</strong></span>
          <span>Open now: <strong className={state.unrealizedPnl == null ? undefined : state.unrealizedPnl >= 0 ? 'auto-trader-pnl-positive' : 'auto-trader-pnl-negative'}>{state.unrealizedPnl == null ? 'Unavailable (missing mark or FX rate)' : 'USD ' + money.format(state.unrealizedPnl)}</strong></span>
          <span>Combined today: <strong className={state.realizedPnl == null || state.unrealizedPnl == null ? undefined : state.realizedPnl + state.unrealizedPnl >= 0 ? 'auto-trader-pnl-positive' : 'auto-trader-pnl-negative'}>{state.realizedPnl == null || state.unrealizedPnl == null ? 'Unavailable' : 'USD ' + money.format(state.realizedPnl + state.unrealizedPnl)}</strong></span>
        </div>
        {state.error && <p className="auto-trader-error" role="alert">{state.error}</p>}
        <AutoTraderOutcomes embedded strategyRatesOnly />
      </div>
    </section>
  );
}
