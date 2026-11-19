import { useEffect, useMemo } from 'react';
import { createPortal } from 'react-dom';
import type { LearningDecision } from '../models/AutoTraderLearningWidgetModel';
import { AutoTraderLearningDecisionDetailsModel } from '../models/AutoTraderLearningDecisionDetailsModel';

function CandidateEvidence({ value }: { value: unknown }) {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    return (
      <p>
        No additional candidate eligibility evidence was captured for this
        strategy. See Strategy decision and parameters for the recorded gate
        values.
      </p>
    );
  const evidence = value as Record<string, unknown>;
  const checks = evidence.checks;
  const reasons = Array.isArray(evidence.reasons)
    ? evidence.reasons.filter(
        (reason): reason is string => typeof reason === 'string',
      )
    : [];
  if (!checks || typeof checks !== 'object' || Array.isArray(checks))
    return <pre>{JSON.stringify(evidence, null, 2)}</pre>;
  return (
    <>
      <p>
        Candidate {evidence.eligible === true ? 'eligible' : 'not eligible'}
        {reasons.length ? ` · ${reasons.join(' · ')}` : ''}
      </p>
      <table className="auto-trader-learning-evidence">
        <thead>
          <tr>
            <th>Check</th>
            <th>Result</th>
            <th>Observed evidence</th>
          </tr>
        </thead>
        <tbody>
          {Object.entries(checks as Record<string, unknown>).map(
            ([name, raw]) => {
              const check =
                raw && typeof raw === 'object'
                  ? (raw as Record<string, unknown>)
                  : { passed: raw };
              const result =
                check.passed === true
                  ? 'Pass'
                  : check.passed === false
                    ? 'Fail'
                    : 'Unavailable';
              const observed =
                Object.entries(check)
                  .filter(([key]) => key !== 'passed')
                  .map(
                    ([key, item]) =>
                      `${key}: ${item == null ? 'unavailable' : typeof item === 'object' ? JSON.stringify(item) : String(item)}`,
                  )
                  .join(' · ') || 'No values recorded';
              return (
                <tr key={name}>
                  <td>{name.replaceAll('_', ' ')}</td>
                  <td>{result}</td>
                  <td>{observed}</td>
                </tr>
              );
            },
          )}
        </tbody>
      </table>
    </>
  );
}

export function AutoTraderLearningDecisionDetails({
  decision,
  onClose,
}: {
  decision: LearningDecision;
  onClose: () => void;
}) {
  const model = useMemo(
    () => new AutoTraderLearningDecisionDetailsModel(decision),
    [decision],
  );
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [onClose]);

  return createPortal(
    <div
      className="trade-popup-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <section
        className="trade-popup auto-trader-learning-details-popup"
        role="dialog"
        aria-modal="true"
        aria-label={`Decision details: ${model.title}`}
      >
        <header>
          <div>
            <span>LEARNING DECISION DETAILS</span>
            <h2>{model.title}</h2>
          </div>
          <button
            type="button"
            aria-label="Close decision details"
            onClick={onClose}
          >
            ×
          </button>
        </header>
        <p className="auto-trader-learning-details-summary">
          {decision.status ?? 'Unknown status'} → final:{' '}
          {decision.final_outcome?.status ?? 'pending'} ·{' '}
          {decision.final_outcome?.reason ??
            decision.reason ??
            decision.entry_gate?.rejection_reasons?.join(', ') ??
            'No reason recorded'}
        </p>
        <div className="auto-trader-learning-details-content">
          {model.sections.map((section) => (
            <section key={section.title}>
              <strong>{section.title}</strong>
              {section.title === 'Eligibility evidence' ? (
                <CandidateEvidence value={section.value} />
              ) : (
                <pre>{JSON.stringify(section.value, null, 2)}</pre>
              )}
            </section>
          ))}
        </div>
      </section>
    </div>,
    document.body,
  );
}
