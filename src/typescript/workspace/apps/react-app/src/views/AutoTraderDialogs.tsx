import { createPortal } from 'react-dom';
import { AutoTraderScoutOptions } from './AutoTraderStrategyParameters';
import type {
  AutoTraderModel,
  AutoTraderState,
  Strategy,
} from '../models/AutoTraderModel';
import { projectAutoTraderBudgetState } from '../models/AutoTraderSettingsModel';
import { money, timestamp } from './AutoTraderOrderGrid';

const finiteFixed = (value: unknown, digits: number) =>
  typeof value === 'number' && Number.isFinite(value)
    ? value.toFixed(digits)
    : 'n/a';

type DialogProps = { state: AutoTraderState; onClose: () => void };

export function AutoTraderParametersDialog({ state, onClose }: DialogProps) {
  return createPortal(
    <div
      className="app-popup-backdrop auto-trader-popup-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        className="auto-trader-settings-popup auto-trader-parameters-popup"
        role="dialog"
        aria-modal="true"
        aria-label="Current Auto Trader parameters"
      >
        <div className="auto-trader-settings-heading">
          <strong>Current Auto Trader parameters</strong>
          <button
            type="button"
            aria-label="Close parameters"
            onClick={() => onClose()}
          >
            ×
          </button>
        </div>
        <small aria-live="polite">
          Live parameters from the AutoTrader service · refreshed{' '}
          {state.decisionParameters
            ? timestamp.format(state.decisionParameters.refreshed_at)
            : 'on open and every 5s'}
        </small>
        {state.decisionParametersError && (
          <p className="auto-trader-error" role="alert">
            {state.decisionParametersError}
          </p>
        )}
        {!state.decisionParameters && !state.decisionParametersError && (
          <p>Loading live decision parameters…</p>
        )}
        {state.decisionParameters && (
          <div className="auto-trader-parameter-sections">
            <section>
              <strong>Shared risk controls</strong>
              <dl>
                <div>
                  <dt>Risk appetite</dt>
                  <dd>
                    {Number(state.riskAppetite)} ·{' '}
                    {Number(state.riskAppetite) < 34
                      ? 'Safe'
                      : Number(state.riskAppetite) > 66
                        ? 'Risky'
                        : 'Balanced'}
                  </dd>
                </div>
                <div>
                  <dt>Per-order and per-symbol cap</dt>
                  <dd>{money.format(Number(state.maxOrderNotional))}</dd>
                </div>
                <div>
                  <dt>Maximum allocated exposure</dt>
                  <dd>{money.format(Number(state.dailyNotionalLimit))}</dd>
                </div>
                <div>
                  <dt>Daily loss cap</dt>
                  <dd>
                    {Number(state.dailyRealizedLossLimit).toLocaleString()}
                  </dd>
                </div>
                <div>
                  <dt>Maximum open positions</dt>
                  <dd>{state.maxOpenPositions}</dd>
                </div>
                <div>
                  <dt>Maximum entries per position</dt>
                  <dd>{state.maxEntriesPerPosition}</dd>
                </div>
                <div>
                  <dt>Exploration allocation</dt>
                  <dd>{state.explorationPercent}%</dd>
                </div>
                <div>
                  <dt>Protective stop / target</dt>
                  <dd>
                    {finiteFixed(state.stopLossPercent, 2)}% /{' '}
                    {finiteFixed(state.takeProfitPercent, 2)}%
                  </dd>
                </div>
              </dl>
            </section>
            <section>
              <strong>Service decision parameters</strong>
              <dl>
                {state.decisionParameters.thresholds.map((parameter) => (
                  <div key={parameter.section + parameter.name}>
                    <dt>
                      {parameter.section} · {parameter.name}
                    </dt>
                    <dd>
                      {Number.isInteger(parameter.value)
                        ? parameter.value
                        : finiteFixed(parameter.value, 4)}{' '}
                      {parameter.unit}
                    </dd>
                  </div>
                ))}
              </dl>
            </section>
            <section>
              <strong>Learned symbol and strategy profiles</strong>
              {state.decisionParameters.contexts.length === 0 ? (
                <p>No closed outcomes have produced learned profiles yet.</p>
              ) : (
                <dl>
                  {state.decisionParameters.contexts.map((profile) => (
                    <div key={profile.context}>
                      <dt>{profile.context}</dt>
                      <dd>
                        {profile.learning_samples} trade outcomes ·{' '}
                        {profile.positive_rate == null
                          ? 'win rate n/a'
                          : finiteFixed(profile.positive_rate * 100, 1) +
                            '% positive'}{' '}
                        · Scouts {profile.scout_learning_samples} outcomes
                        {profile.scout_positive_rate == null
                          ? ''
                          : ' (' +
                            finiteFixed(profile.scout_positive_rate * 100, 1) +
                            '% positive)'}{' '}
                        · strength ≥ {finiteFixed(profile.strength_floor, 3)} ·
                        score ≥{' '}
                        {finiteFixed(profile.exploration_score_floor, 3)}
                        {profile.exploration_allowed
                          ? ''
                          : ' · exploration disabled'}
                      </dd>
                    </div>
                  ))}
                </dl>
              )}
            </section>
          </div>
        )}
      </div>
    </div>,
    document.body,
  );
}

type SettingsDialogProps = DialogProps & { model: AutoTraderModel };

export function AutoTraderSettingsDialog({
  state,
  model,
  onClose,
}: SettingsDialogProps) {
  const {
    selectedIntradayStrategies,
    selectedDailyStrategies,
    selectedIntradayBudgetTotal,
    selectedDailyBudgetTotal,
    intradayBudgetOverallocated,
    dailyBudgetOverallocated,
  } = projectAutoTraderBudgetState(state);
  return createPortal(
    <div
      className="app-popup-backdrop auto-trader-popup-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        className="auto-trader-settings-popup"
        role="dialog"
        aria-modal="true"
        aria-label="Auto Trader settings"
      >
        <div className="auto-trader-settings-heading">
          <strong>Auto Trader settings</strong>
          <button
            type="button"
            aria-label="Close settings"
            onClick={() => onClose()}
          >
            ×
          </button>
        </div>
        <div className="auto-trader-limits">
          <fieldset
            className="auto-trader-strategy-options"
            title="Live and candle-derived insight strategies share the same AutoTrader portfolio exposure, daily loss, and open-position limits."
          >
            <legend>STRATEGIES · SHARED PORTFOLIO LIMITS</legend>
            {(
              [
                {
                  value: 'live-momentum',
                  label:
                    'Live momentum · aligned 10-second momentum or 5-minute trend; configured entry order; shared protective stop/target',
                },
                {
                  value: 'live-trade-flow',
                  label:
                    'Live trade flow · aligned imbalance ≥ 0.15; configured entry order; shared protective stop/target',
                },
                {
                  value: 'live-order-book',
                  label:
                    'Live order book · aligned imbalance ≥ 0.15; configured entry order; shared protective stop/target',
                },
                {
                  value: 'live-vwap-dislocation',
                  label:
                    'Live VWAP dislocation · aligned gap ≥ 1 bp; configured entry order; shared protective stop/target',
                },
                {
                  value: 'live-absorption-pressure',
                  label:
                    'Live absorption · opposing pressure ≥ 0.2; configured entry order; shared protective stop/target',
                },
                {
                  value: 'donchian-55-btc200',
                  label:
                    '55-day Donchian breakout · trade highs only while BTC is above its SMA200',
                },
                {
                  value: 'donchian-20-sma200',
                  label:
                    '20-day Donchian breakout · buy assets above their own SMA200',
                },
                {
                  value: 'donchian-20-btc200',
                  label:
                    '20-day Donchian + BTC SMA200 · fixed 20/200 daily signal exit · no protective stop · paper research',
                },
                {
                  value: 'donchian-10-btc200',
                  label:
                    '10-day Donchian + BTC SMA200 · higher-frequency candidate · signal exit · no protective stop',
                },
                {
                  value: 'donchian-20-btc200-hold3',
                  label:
                    'A2 · 20-day Donchian + BTC SMA200 · 72-hour minimum hold · 25% per-order size cap · paper only',
                },
                {
                  value: 'risk-adjusted-trend-portfolio',
                  label:
                    'B · 16-coin trend score / volatility · BTC SMA200 · 25% target weight cap · paper only',
                },
                {
                  value: 'trend-sleeves-btc200',
                  label:
                    'Trend sleeves · BTC/ETH SMA50 + BNB SMA100 · BTC SMA200 risk gate',
                },
                {
                  value: 'trend-50-200',
                  label:
                    'Trend following · hold each asset above its SMA50 and SMA200',
                },
                {
                  value: 'relative-strength-90-sma200',
                  label:
                    'Momentum rotation · hold the strongest 90-day asset above its SMA200',
                },
                {
                  value: 'risk-adjusted-monthly-momentum',
                  label:
                    'Risk-adjusted monthly momentum · BTC SMA200 gate · 40% volatility target · signal exit, no fixed stop',
                },
                {
                  value: 'rsi2-oversold-rebound',
                  label:
                    'Mean reversion · RSI(2) ≤ 10 above SMA200 · five-day max hold',
                },
                {
                  value: 'dated-futures-carry',
                  label:
                    'Dated futures carry · Insights signal, paired BTC/ETH paper hedge, settlement-window exit',
                },
              ] as Array<{ value: Strategy; label: string }>
            ).map(({ value, label }) => (
              <label key={value}>
                <input
                  type="checkbox"
                  checked={state.strategies.includes(value)}
                  onChange={(event) =>
                    model.setStrategies(
                      event.target.checked
                        ? [...state.strategies, value]
                        : state.strategies.filter(
                            (strategy) => strategy !== value,
                          ),
                    )
                  }
                />
                {label}
              </label>
            ))}
            <small>
              Intraday and daily strategies have editable per-strategy budget
              caps and separate position-slot limits. Learning adjusts effective
              strategy budgets and candidates with stronger measured net
              outcomes are submitted first when history is sufficient. Orders
              use shared account, exposure, and daily-loss limits; per-strategy
              P&L is tracked separately. The 20-day BTC-gated breakout, A2, and
              B use signal exits without fixed stops. A2 applies a 72-hour
              minimum hold and 0.25 order-size multiplier. B rebalances open
              paper positions toward capped inverse-volatility target weights
              when drift exceeds 5% of target value, subject to shared limits
              and book liquidity. Dated futures carry has its own paper size
              cap.
            </small>
            {state.strategies.length === 0 &&
              state.scoutStrategies.length === 0 && (
                <small className="auto-trader-strategy-validation">
                  Select at least one strategy or Scout before saving.
                </small>
              )}
          </fieldset>
          <AutoTraderScoutOptions
            scoutStrategies={state.scoutStrategies}
            saving={state.busy}
            onChange={(strategies) => model.setScoutStrategies(strategies)}
          />
          <label title="Maximum per order and combined open plus pending exposure per symbol across all AutoTrader strategies.">
            MAX PER ORDER / SYMBOL (USD)
            <input
              type="number"
              min="0.01"
              step="any"
              value={state.maxOrderNotional}
              onChange={(event) =>
                model.setLimits(
                  event.target.value,
                  state.dailyNotionalLimit,
                  state.dailyRealizedLossLimit,
                  state.explorationPercent,
                  state.maxOpenPositions,
                )
              }
            />
          </label>
          <label title="Maximum USD notional allocated to Auto Trader positions and pending orders that add exposure. Opening positions uses available capacity; closing fills release it. Manual positions are not included.">
            MAX ALLOCATED EXPOSURE (USD)
            <input
              type="number"
              min="0.01"
              step="any"
              value={state.dailyNotionalLimit}
              onChange={(event) =>
                model.setLimits(
                  state.maxOrderNotional,
                  event.target.value,
                  state.dailyRealizedLossLimit,
                  state.explorationPercent,
                  state.maxOpenPositions,
                )
              }
            />
          </label>
          <label>
            DAILY LOSS CAP (USD)
            <input
              type="number"
              min="0.01"
              step="any"
              value={state.dailyRealizedLossLimit}
              onChange={(event) =>
                model.setLimits(
                  state.maxOrderNotional,
                  state.dailyNotionalLimit,
                  event.target.value,
                  state.explorationPercent,
                  state.maxOpenPositions,
                )
              }
            />
          </label>
          <label>
            EXPLORATION SIZE (%)
            <input
              type="number"
              min="0"
              max="100"
              step="1"
              value={state.explorationPercent}
              onChange={(event) =>
                model.setLimits(
                  state.maxOrderNotional,
                  state.dailyNotionalLimit,
                  state.dailyRealizedLossLimit,
                  event.target.value,
                  state.maxOpenPositions,
                )
              }
            />
          </label>
          <label
            className="auto-trader-risk-slider"
            title="Shifts strength and score floors and adjusts take-profit. Riskier settings lower entry floors and move take-profit toward a 0.05% net win after fees; safer settings raise floors and preserve larger targets."
          >
            RISK APPETITE{' '}
            <span className="auto-trader-risk-value">
              {Number(state.riskAppetite)} ·{' '}
              {Number(state.riskAppetite) < 34
                ? 'Safe'
                : Number(state.riskAppetite) > 66
                  ? 'Risky'
                  : 'Balanced'}
            </span>
            <input
              aria-label="Auto Trader risk appetite"
              type="range"
              min="0"
              max="100"
              step="1"
              value={state.riskAppetite}
              onChange={(event) => model.setRiskAppetite(event.target.value)}
            />
            <span className="auto-trader-risk-scale">
              <span>Safe · higher floors</span>
              <span>Risky · lower floors</span>
            </span>
          </label>
          <label title="Maximum number of distinct Auto Trader positions held at once.">
            MAX OPEN POSITIONS
            <input
              type="number"
              min="1"
              max="20"
              step="1"
              value={state.maxOpenPositions}
              onChange={(event) =>
                model.setLimits(
                  state.maxOrderNotional,
                  state.dailyNotionalLimit,
                  state.dailyRealizedLossLimit,
                  state.explorationPercent,
                  event.target.value,
                )
              }
            />
          </label>
          <label title="Maximum number of active Scout positions or pending Scout orders.">
            MAX ACTIVE SCOUT POSITIONS
            <input
              type="number"
              min="0"
              max="10000"
              step="1"
              value={state.scoutMaxActivePositions}
              onChange={(event) =>
                model.setScoutMaxActivePositions(event.target.value)
              }
            />
          </label>
          <label title="Maximum number of fee-free Scout samples being evaluated at once. This does not limit or create real positions.">
            MAX CONCURRENT SCOUT SAMPLES
            <input
              type="number"
              min="0"
              max="5000"
              step="1"
              value={state.maxConcurrentScoutSamples}
              onChange={(event) =>
                model.setMaxConcurrentScoutSamples(event.target.value)
              }
            />
          </label>
          <fieldset
            className="auto-trader-category-limits"
            title="Intraday signal and daily strategy budgets are separate; the overall position and exposure caps still apply."
          >
            <legend>HORIZON BUDGETS</legend>
            <label>
              INTRADAY POSITIONS
              <input
                type="number"
                min="0"
                max="20"
                step="1"
                value={state.maxIntradayOpenPositions}
                onChange={(event) =>
                  model.setCategoryLimits({
                    maxIntradayOpenPositions: event.target.value,
                  })
                }
              />
            </label>
            <label>
              DAILY POSITIONS
              <input
                type="number"
                min="0"
                max="20"
                step="1"
                value={state.maxDailyOpenPositions}
                onChange={(event) =>
                  model.setCategoryLimits({
                    maxDailyOpenPositions: event.target.value,
                  })
                }
              />
            </label>
            <label>
              INTRADAY BUDGET (USD)
              <input
                type="number"
                min="0"
                step="any"
                value={state.intradayNotionalLimit}
                onChange={(event) =>
                  model.setCategoryLimits({
                    intradayNotionalLimit: event.target.value,
                  })
                }
              />
            </label>
            <label>
              DAILY BUDGET (USD)
              <input
                type="number"
                min="0"
                step="any"
                value={state.dailyStrategyNotionalLimit}
                onChange={(event) =>
                  model.setCategoryLimits({
                    dailyStrategyNotionalLimit: event.target.value,
                  })
                }
              />
            </label>
            <small>
              Overall limits still apply. Each symbol can belong to one horizon
              at a time.
            </small>
          </fieldset>
          {selectedIntradayStrategies.length > 0 && (
            <details className="auto-trader-strategy-budgets">
              <summary
                className={intradayBudgetOverallocated ? 'has-error' : ''}
              >
                <strong>INTRADAY STRATEGY BUDGETS</strong>
                <span>
                  {money.format(selectedIntradayBudgetTotal)} /{' '}
                  {money.format(Number(state.intradayNotionalLimit))} USD ·{' '}
                  {selectedIntradayStrategies.length} strategies
                  {intradayBudgetOverallocated ? ' · fix allocations' : ''}
                </span>
              </summary>
              <div className="auto-trader-strategy-budget-grid">
                {selectedIntradayStrategies.map(({ strategy, label }) => {
                  const learning = state.intradayStrategyLearning.find(
                    (item) => item.strategy === strategy,
                  );
                  const outcome = state.closedStrategyOutcomes[strategy];
                  const learnedSummary =
                    'Learning ×' +
                    finiteFixed(learning?.learning_multiplier ?? 1, 2) +
                    ' · ' +
                    (outcome
                      ? outcome.trades +
                        ' closed trades · ' +
                        finiteFixed(outcome.winRate, 1) +
                        '% wins · avg return ' +
                        finiteFixed(outcome.averageReturn, 3) +
                        '%'
                      : 'no closed trades');
                  return (
                    <div
                      className="auto-trader-strategy-budget-row"
                      key={strategy}
                    >
                      <label>
                        {label}
                        <input
                          aria-label={label + ' base budget in USD'}
                          type="number"
                          min="0"
                          step="any"
                          value={state.intradayStrategyBudgets[strategy] ?? '0'}
                          onChange={(event) =>
                            model.setIntradayStrategyBudget(
                              strategy,
                              event.target.value,
                            )
                          }
                        />
                      </label>
                      <small>
                        {learnedSummary} · effective cap{' '}
                        {money.format(
                          learning?.effective_budget ??
                            Number(
                              state.intradayStrategyBudgets[strategy] ?? 0,
                            ),
                        )}{' '}
                        · used {money.format(learning?.used_notional ?? 0)}
                      </small>
                    </div>
                  );
                })}
                <small>
                  Learning scales the effective cap. Closed trade counts, win
                  rates, and average returns match the completed positions
                  table.
                </small>
              </div>
            </details>
          )}
          {selectedDailyStrategies.length > 0 && (
            <details className="auto-trader-strategy-budgets">
              <summary className={dailyBudgetOverallocated ? 'has-error' : ''}>
                <strong>DAILY STRATEGY BUDGETS</strong>
                <span>
                  {money.format(selectedDailyBudgetTotal)} /{' '}
                  {money.format(Number(state.dailyStrategyNotionalLimit))} USD ·{' '}
                  {selectedDailyStrategies.length} strategies
                  {dailyBudgetOverallocated ? ' · fix allocations' : ''}
                </span>
              </summary>
              <div className="auto-trader-strategy-budget-grid">
                {selectedDailyStrategies.map(({ strategy, label }) => {
                  const learning = state.dailyStrategyLearning.find(
                    (item) => item.strategy === strategy,
                  );
                  const outcome = state.closedStrategyOutcomes[strategy];
                  const learnedSummary =
                    'Learning ×' +
                    finiteFixed(learning?.learning_multiplier ?? 1, 2) +
                    ' · ' +
                    (outcome
                      ? outcome.trades +
                        ' closed trades · ' +
                        finiteFixed(outcome.winRate, 1) +
                        '% wins · avg return ' +
                        finiteFixed(outcome.averageReturn, 3) +
                        '%'
                      : 'no closed trades');
                  return (
                    <div
                      className="auto-trader-strategy-budget-row"
                      key={strategy}
                    >
                      <label>
                        {label}
                        <input
                          aria-label={label + ' base budget in USD'}
                          type="number"
                          min="0"
                          step="any"
                          value={state.dailyStrategyBudgets[strategy] ?? '0'}
                          onChange={(event) =>
                            model.setDailyStrategyBudget(
                              strategy,
                              event.target.value,
                            )
                          }
                        />
                      </label>
                      <small>
                        {learnedSummary} · effective cap{' '}
                        {money.format(
                          learning?.effective_budget ??
                            Number(state.dailyStrategyBudgets[strategy] ?? 0),
                        )}{' '}
                        · used {money.format(learning?.used_notional ?? 0)}
                      </small>
                    </div>
                  );
                })}
                <small>
                  Learning scales the effective cap. Closed trade counts, win
                  rates, and average returns match the completed positions
                  table.
                </small>
              </div>
            </details>
          )}
          <label title="Maximum number of entries Auto Trader can make while a position remains open, including the initial entry.">
            ENTRIES PER POSITION
            <input
              type="number"
              min="1"
              max="10"
              step="1"
              value={state.maxEntriesPerPosition}
              onChange={(event) =>
                model.setMaxEntriesPerPosition(event.target.value)
              }
            />
          </label>
          <label>
            VWAP / TWAP DURATION (MIN)
            <input
              type="number"
              min="5"
              max="390"
              step="1"
              value={state.vwapDurationMinutes}
              onChange={(event) =>
                model.setExecutionParameters({
                  vwapDurationMinutes: event.target.value,
                })
              }
            />
          </label>
          <label>
            VWAP PARTICIPATION (%)
            <input
              type="number"
              min="1"
              max="50"
              step="1"
              value={state.vwapParticipationPercent}
              onChange={(event) =>
                model.setExecutionParameters({
                  vwapParticipationPercent: event.target.value,
                })
              }
            />
          </label>
          <label>
            LIMIT OFFSET (BPS)
            <input
              type="number"
              min="-10"
              max="10"
              step="0.5"
              value={state.limitOffsetBps}
              onChange={(event) =>
                model.setExecutionParameters({
                  limitOffsetBps: event.target.value,
                })
              }
            />
          </label>
          <label>
            TRIGGER OFFSET (BPS)
            <input
              type="number"
              min="5"
              max="100"
              step="1"
              value={state.triggerOffsetBps}
              onChange={(event) =>
                model.setExecutionParameters({
                  triggerOffsetBps: event.target.value,
                })
              }
            />
          </label>
          <label>
            TRAILING DISTANCE (%)
            <input
              type="number"
              min="0.05"
              max="1"
              step="0.05"
              value={state.trailingDistancePercent}
              onChange={(event) =>
                model.setExecutionParameters({
                  trailingDistancePercent: event.target.value,
                })
              }
            />
          </label>
          <label title="Number of most recent fully closed exits used for rolling exit statistics. Range: 1–5000.">
            ROLLING CLOSED EXITS
            <input
              type="number"
              min="1"
              max="5000"
              step="1"
              value={state.closedTradeWindow}
              onChange={(event) =>
                model.setClosedTradeWindow(event.target.value)
              }
            />
          </label>
          <label title="Paired scout comparisons required before a strategy parameter can be promoted. Applies to live and daily strategies. Range: 2–100.">
            SCOUT SAMPLES TO PROMOTE
            <input
              type="number"
              min="2"
              max="100"
              step="1"
              value={state.minimumScoutParameterSamples}
              onChange={(event) =>
                model.setMinimumScoutParameterSamples(event.target.value)
              }
            />
          </label>
          <button
            type="button"
            disabled={
              state.busy ||
              state.loading ||
              (state.strategies.length === 0 &&
                state.scoutStrategies.length === 0) ||
              dailyBudgetOverallocated ||
              intradayBudgetOverallocated
            }
            onClick={() => void model.saveLimits()}
          >
            Save settings
          </button>
          <button
            type="button"
            disabled={state.busy || state.loading}
            onClick={() => void model.resetLearning()}
          >
            Reset learned strategy parameters
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
