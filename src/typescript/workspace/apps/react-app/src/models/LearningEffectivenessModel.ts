import type { LearningSummary } from './AutoTraderOutcomesModel';

export type LearningEffectivenessSummary = {
  pairedSamples: number;
  pairedLift: number | null;
  closedTrades: number;
  closedTradeReturn: number | null;
  activeProfileTrades: number;
  activeProfileReturn: number | null;
  activeProfilePositiveRate: number | null;
  otherProfileTrades: number;
  otherProfileReturn: number | null;
  otherProfilePositiveRate: number | null;
  scoutOutcomes: number;
  scoutReturn: number | null;
};

export function summarizeLearningEffectiveness(
  summaries: LearningSummary[],
): LearningEffectivenessSummary {
  const weightedAverage = (
    countKey: keyof LearningSummary,
    valueKey: keyof LearningSummary,
  ) => {
    let count = 0;
    let weightedCount = 0;
    let total = 0;
    for (const summary of summaries) {
      const sampleCount = summary[countKey];
      const average = summary[valueKey];
      if (typeof sampleCount === 'number' && sampleCount > 0) {
        count += sampleCount;
        if (typeof average === 'number' && Number.isFinite(average)) {
          weightedCount += sampleCount;
          total += sampleCount * average;
        }
      }
    }
    return {
      count,
      average: weightedCount > 0 ? total / weightedCount : null,
    };
  };
  const paired = weightedAverage(
    'paired_scout_samples',
    'average_paired_scout_lift_percent',
  );
  const trades = weightedAverage(
    'learning_samples',
    'average_net_return_percent',
  );
  const activeProfileTrades = weightedAverage(
    'active_profile_trade_samples',
    'active_profile_average_net_return_percent',
  );
  const otherProfileTrades = weightedAverage(
    'other_profile_trade_samples',
    'other_profile_average_net_return_percent',
  );
  const activeProfilePositiveRate = weightedAverage(
    'active_profile_trade_samples',
    'active_profile_trade_positive_rate',
  );
  const otherProfilePositiveRate = weightedAverage(
    'other_profile_trade_samples',
    'other_profile_trade_positive_rate',
  );
  const scouts = weightedAverage(
    'scout_outcome_samples',
    'scout_average_net_return_percent',
  );
  return {
    pairedSamples: paired.count,
    pairedLift: paired.average,
    closedTrades: trades.count,
    closedTradeReturn: trades.average,
    activeProfileTrades: activeProfileTrades.count,
    activeProfileReturn: activeProfileTrades.average,
    activeProfilePositiveRate: activeProfilePositiveRate.average,
    otherProfileTrades: otherProfileTrades.count,
    otherProfileReturn: otherProfileTrades.average,
    otherProfilePositiveRate: otherProfilePositiveRate.average,
    scoutOutcomes: scouts.count,
    scoutReturn: scouts.average,
  };
}
