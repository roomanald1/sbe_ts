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
        title: 'Gate path',
        value: decision.final_outcome?.gate_path ?? [
          {
            stage: 'candidate_selection',
            status: decision.status ?? 'unknown',
            selection_mode: decision.entry_gate?.selection_mode,
            gate: decision.entry_gate,
          },
          {
            stage: 'execution',
            status: decision.final_outcome?.status ?? 'pending',
            gate_id: decision.final_outcome?.execution_gate?.gate_id,
            reason: decision.final_outcome?.reason,
            execution_gate: decision.final_outcome?.execution_gate,
          },
        ],
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
