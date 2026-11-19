import type { LearningDecision } from './AutoTraderLearningWidgetModel';

export type LearningDecisionDetailSection = {
  title: string;
  value: unknown;
};

export class AutoTraderLearningDecisionDetailsModel {
  readonly decision: LearningDecision;

  constructor(decision: LearningDecision) {
    this.decision = decision;
  }

  get title() {
    return `${this.decision.symbol ?? 'Unknown symbol'} · ${this.decision.strategy ?? 'Unknown strategy'}`;
  }

  get sections(): LearningDecisionDetailSection[] {
    const decision = this.decision;
    return [
      {
        title: 'Eligibility evidence',
        value: decision.strategy_decision?.candidate_evidence,
      },
      {
        title: 'Final outcome and execution',
        value: decision.final_outcome ?? {
          status: 'No final outcome recorded',
          reason:
            'No entry order or downstream decision outcome was recorded for this selection.',
        },
      },
      {
        title: 'Gate checks',
        value: decision.entry_gate ?? 'No entry gate captured',
      },
      {
        title: 'Strategy decision and parameters',
        value: decision.strategy_decision ?? 'No strategy decision captured',
      },
      {
        title: 'Signal components',
        value: decision.signal_components ?? 'No signal components captured',
      },
    ];
  }
}
