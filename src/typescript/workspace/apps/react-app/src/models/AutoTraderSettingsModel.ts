import type { AutoTraderState, Strategy } from './AutoTraderModel';

const intradayStrategyLabels: Array<{ strategy: Strategy; label: string }> = [
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
  {
    strategy: 'donchian-10-btc200',
    label: '10-day Donchian · BTC SMA200 · higher-frequency paper candidate',
  },
  {
    strategy: 'donchian-20-btc200-hold3',
    label:
      '20-day Donchian · BTC SMA200 · 3-day minimum hold · capped paper sizing',
  },
  {
    strategy: 'risk-adjusted-trend-portfolio',
    label:
      '16-coin risk-adjusted trend · BTC SMA200 · 25% per-coin cap · signal exit',
  },
  { strategy: 'trend-sleeves-btc200', label: 'Trend sleeves' },
  { strategy: 'trend-50-200', label: 'Trend 50/200' },
  {
    strategy: 'relative-strength-90-sma200',
    label: '90-day relative strength',
  },
  {
    strategy: 'risk-adjusted-monthly-momentum',
    label: 'Risk-adjusted monthly momentum',
  },
  { strategy: 'rsi2-oversold-rebound', label: 'RSI2 oversold rebound' },
];

export function projectAutoTraderBudgetState(state: AutoTraderState) {
  const selectedIntradayStrategies = intradayStrategyLabels.filter(
    ({ strategy }) => state.strategies.includes(strategy),
  );
  const selectedDailyStrategies = dailyStrategyLabels.filter(({ strategy }) =>
    state.strategies.includes(strategy),
  );
  const budgetTotal = (
    selected: typeof selectedIntradayStrategies,
    budgets: Partial<Record<Strategy, string>>,
  ) =>
    selected.reduce(
      (total, { strategy }) => total + (Number(budgets[strategy]) || 0),
      0,
    );
  const valuesValid = (
    selected: typeof selectedIntradayStrategies,
    budgets: Partial<Record<Strategy, string>>,
    defaultValue: string,
  ) =>
    selected.every(({ strategy }) => {
      const value = budgets[strategy] ?? defaultValue;
      return (
        value.trim() !== '' &&
        Number.isFinite(Number(value)) &&
        Number(value) >= 0
      );
    });
  const selectedIntradayBudgetTotal = budgetTotal(
    selectedIntradayStrategies,
    state.intradayStrategyBudgets,
  );
  const selectedDailyBudgetTotal = budgetTotal(
    selectedDailyStrategies,
    state.dailyStrategyBudgets,
  );
  return {
    selectedIntradayStrategies,
    selectedDailyStrategies,
    selectedIntradayBudgetTotal,
    selectedDailyBudgetTotal,
    intradayBudgetOverallocated:
      !valuesValid(
        selectedIntradayStrategies,
        state.intradayStrategyBudgets,
        '',
      ) ||
      selectedIntradayBudgetTotal > Number(state.intradayNotionalLimit) + 1e-9,
    dailyBudgetOverallocated:
      !valuesValid(selectedDailyStrategies, state.dailyStrategyBudgets, '0') ||
      selectedDailyBudgetTotal >
        Number(state.dailyStrategyNotionalLimit) + 1e-9,
  };
}
