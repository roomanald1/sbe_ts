import { useEffect, useMemo } from 'react';
import { createPortal } from 'react-dom';
import type { LearningDecision } from '../models/AutoTraderLearningWidgetModel';
import { AutoTraderLearningDecisionDetailsModel } from '../models/AutoTraderLearningDecisionDetailsModel';

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function formatEvidence(value: unknown) {
  if (value == null) return 'unavailable';
  return typeof value === 'object' ? JSON.stringify(value) : String(value);
}

function CandidateEvidence({
  value,
  showSummary = true,
}: {
  value: unknown;
  showSummary?: boolean;
}) {
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
  const reasonsValue = evidence.reasons ?? evidence.rejection_reasons;
  const reasons = Array.isArray(reasonsValue)
    ? reasonsValue.filter(
        (reason): reason is string => typeof reason === 'string',
      )
    : [];
  if (!checks || typeof checks !== 'object' || Array.isArray(checks))
    return <pre>{JSON.stringify(evidence, null, 2)}</pre>;
  return (
    <>
      {showSummary && (
        <p className="auto-trader-learning-gate-summary">
          {[
            typeof evidence.eligible === 'boolean'
              ? `Candidate ${evidence.eligible ? 'eligible' : 'not eligible'}`
              : 'Candidate gate results',
            typeof evidence.selection_mode === 'string'
              ? `${evidence.selection_mode.replaceAll('_', ' ')} selection`
              : null,
            reasons.length ? reasons.join(' · ') : null,
          ]
            .filter(Boolean)
            .join(' · ')}
        </p>
      )}
      <table className="auto-trader-learning-evidence">
        <thead>
          <tr>
            <th scope="col">Gate</th>
            <th scope="col">Result</th>
            <th scope="col">Observed / required</th>
          </tr>
        </thead>
        <tbody>
          {Object.entries(checks as Record<string, unknown>).map(
            ([name, raw]) => {
              const check = isRecord(raw) ? raw : { passed: raw };
              const exploratoryStrengthBypass =
                name === 'strength_pass' &&
                check.passed === false &&
                evidence.selection_mode === 'exploratory';
              const result = exploratoryStrengthBypass
                ? 'bypassed'
                : check.passed === true
                  ? 'pass'
                  : check.passed === false
                    ? 'fail'
                    : 'unknown';
              const observedValues = isRecord(evidence.observed)
                ? evidence.observed
                : {};
              const thresholds = isRecord(evidence.thresholds)
                ? evidence.thresholds
                : {};
              const observedKey =
                name === 'strength_pass'
                  ? 'strength'
                  : name === 'exploration_score_pass'
                    ? 'absolute_score'
                    : name === 'strategy_walk_forward_evidence_present'
                      ? 'evidence_contract'
                      : name === 'exploration_context_pass'
                        ? 'market_context'
                        : name;
              const thresholdKey =
                name === 'strength_pass'
                  ? 'strength_floor'
                  : name === 'exploration_score_pass'
                    ? 'exploration_score_absolute_floor'
                    : name;
              const observed =
                check.observed ??
                observedValues[observedKey] ??
                (observedKey === 'market_context'
                  ? evidence.market_context
                  : undefined);
              const required = check.required ?? thresholds[thresholdKey];
              const details = [
                observed === undefined
                  ? null
                  : `observed: ${formatEvidence(observed)}`,
                required === undefined
                  ? null
                  : `required: ${formatEvidence(required)}`,
                Object.entries(check)
                  .filter(
                    ([key]) =>
                      !['passed', 'observed', 'required'].includes(key),
                  )
                  .map(([key, item]) => `${key}: ${formatEvidence(item)}`)
                  .join(' · '),
              ].filter(Boolean);
              return (
                <tr key={name}>
                  <td className="auto-trader-learning-gate-name">
                    {name === 'strength_pass' &&
                    evidence.selection_mode === 'exploratory'
                      ? 'strict strength floor'
                      : name.replaceAll('_', ' ')}
                  </td>
                  <td>
                    <span
                      className={`auto-trader-gate-result auto-trader-gate-result--${result}`}
                      title={
                        exploratoryStrengthBypass
                          ? 'The strict strength floor is not required for this exploratory selection; exploration score and context checks still apply.'
                          : undefined
                      }
                    >
                      {result === 'unknown' ? 'unavailable' : result}
                    </span>
                  </td>
                  <td>{details.join(' · ') || 'No values recorded'}</td>
                </tr>
              );
            },
          )}
        </tbody>
      </table>
    </>
  );
}

function GatePath({ value }: { value: unknown }) {
  if (!Array.isArray(value))
    return (
      <pre className="auto-trader-learning-gate-fallback">
        {JSON.stringify(value, null, 2)}
      </pre>
    );

  return (
    <ol className="auto-trader-learning-gate-path">
      {value.map((raw, index) => {
        if (!isRecord(raw)) return <li key={index}>{formatEvidence(raw)}</li>;
        const stage = raw.stage;
        const stageName =
          stage === 'candidate_selection'
            ? 'Selection'
            : stage === 'execution'
              ? 'Execution'
              : typeof stage === 'string'
                ? stage.replaceAll('_', ' ')
                : `Stage ${index + 1}`;
        const gateId = raw.gate_id;
        const reason = raw.reason;
        const gate = raw.gate;
        const entry = raw.entry;
        const executionGate = raw.execution_gate;
        const subsequentChecks = raw.subsequent_checks;
        const status = typeof raw.status === 'string' ? raw.status : 'unknown';
        const orderSubmitted = isRecord(executionGate)
          ? executionGate.order_submitted
          : raw.order_submitted;
        const displayStatus =
          stage === 'execution' && status === 'pending'
            ? orderSubmitted === true
              ? 'order submitted · awaiting execution result'
              : orderSubmitted === false
                ? 'not submitted'
                : 'awaiting submission confirmation'
            : status.replaceAll('_', ' ');
        const tone =
          status === 'blocked' ||
          status === 'fail' ||
          status === 'rejected' ||
          status === 'skipped'
            ? 'blocked'
            : status === 'pending' ||
                status === 'unfilled' ||
                status === 'expired_or_cancelled' ||
                status === 'cancelled'
              ? 'pending'
              : 'passed';
        return (
          <li
            className={`auto-trader-learning-gate-stage auto-trader-learning-gate-stage--${tone}`}
            key={`${String(stage)}-${index}`}
          >
            <div className="auto-trader-learning-gate-stage-header">
              <div className="auto-trader-learning-gate-stage-title">
                <span className="auto-trader-learning-gate-stage-number">
                  {index + 1}
                </span>
                <div>
                  <strong>{stageName}</strong>
                  {typeof gateId === 'string' && (
                    <span className="auto-trader-learning-gate-stage-meta">
                      {gateId.replaceAll('_', ' ')}
                    </span>
                  )}
                </div>
              </div>
              <span
                className={`auto-trader-gate-result auto-trader-gate-result--${tone}`}
              >
                {displayStatus}
              </span>
            </div>
            {typeof (
              reason ??
              (isRecord(executionGate) && executionGate.reason)
            ) === 'string' && (
              <p className="auto-trader-learning-gate-reason">
                {String(
                  reason ??
                    (isRecord(executionGate) ? executionGate.reason : ''),
                )}
              </p>
            )}
            {gate !== undefined && <CandidateEvidence value={gate} />}
            {isRecord(executionGate) && (
              <dl className="auto-trader-learning-gate-facts">
                {Object.entries(executionGate)
                  .filter(([key]) => key !== 'reason')
                  .map(([key, fact]) => (
                    <div key={key}>
                      <dt>{key.replaceAll('_', ' ')}</dt>
                      <dd>{formatEvidence(fact)}</dd>
                    </div>
                  ))}
              </dl>
            )}
            {entry !== undefined && (
              <details className="auto-trader-learning-gate-entry">
                <summary>Order and execution details</summary>
                <pre>{JSON.stringify(entry, null, 2)}</pre>
              </details>
            )}
            {subsequentChecks === 'not_evaluated_after_block' && (
              <p className="auto-trader-learning-gate-not-evaluated">
                Later execution checks were not evaluated after this block.
              </p>
            )}
          </li>
        );
      })}
    </ol>
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
  const executionGate = decision.final_outcome?.execution_gate;
  const finalLabel =
    decision.final_outcome?.status === 'pending'
      ? executionGate?.order_submitted === true
        ? 'order submitted · awaiting execution result'
        : executionGate?.order_submitted === false
          ? 'not submitted'
          : 'awaiting submission confirmation'
      : (decision.final_outcome?.status ?? 'pending');
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
          {decision.status ?? 'Unknown status'} → final: {finalLabel} ·{' '}
          {decision.final_outcome?.reason ??
            decision.reason ??
            decision.entry_gate?.rejection_reasons?.join(', ') ??
            'No reason recorded'}
        </p>
        <div className="auto-trader-learning-details-content">
          {model.sections.map((section) => (
            <section
              className={
                section.title === 'Gate path'
                  ? 'auto-trader-learning-gate-path-section'
                  : undefined
              }
              key={section.title}
            >
              <strong>{section.title}</strong>
              {section.title === 'Gate path' ? (
                <GatePath value={section.value} />
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
