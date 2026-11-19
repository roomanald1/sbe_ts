import { useEffect, useMemo } from 'react';
import { LearningScoutActivityModel } from '../models/LearningScoutActivityModel';
import { useBehaviorSnapshot } from '@workspace/ui-platform';

function why(row: Record<string, unknown>) {
  const candidate = row.candidate as Record<string, unknown> | undefined;
  const details = candidate?.strategy_decision as
    | Record<string, unknown>
    | undefined;
  const reasons = details?.rejection_reasons;
  if (Array.isArray(reasons) && reasons.length) return reasons.join(' · ');
  const checks = details?.checks as
    | Record<string, { passed?: boolean; observed?: unknown }>
    | undefined;
  const failures = Object.entries(checks ?? {})
    .filter(([, check]) => check?.passed === false)
    .map(
      ([name, check]) =>
        `${name}${check.observed == null ? '' : ` (observed ${String(check.observed)})`}`,
    );
  if (failures.length) return `Failed checks: ${failures.join(' · ')}`;
  const reason = details?.reason ?? details?.rejection_reason;
  if (typeof reason === 'string' && reason) return reason;
  return 'Strategy worker returned this evaluation; no specific explanation was attached.';
}

export function LearningScoutActivityWidget() {
  const model = useMemo(() => new LearningScoutActivityModel(), []);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  const state = useBehaviorSnapshot(model.state$);
  const rows = state.rows;
  const profiles = state.profiles;

  return (
    <section
      className="auto-trader-strategy-parameters learning-scout-activity"
      aria-label="Learning Scout activity"
    >
      <header className="auto-trader-learning-decisions-head">
        <strong>Configured Scout profiles</strong>
        <small>
          {profiles.length} symbol and strategy profiles currently supplied by
          Auto Trader.
        </small>
      </header>
      {state.profilesError && (
        <p className="auto-trader-learning-error" role="alert">
          Could not load Scout profiles: {state.profilesError}
        </p>
      )}
      {!state.loading && !state.profilesError && profiles.length === 0 && (
        <p>No Scout profiles are currently configured.</p>
      )}
      {profiles.length > 0 && (
        <div className="auto-trader-learning-decisions-table-wrap">
          <table className="auto-trader-learning-decisions-table">
            <thead>
              <tr>
                <th>Profile</th>
                <th>Symbol</th>
                <th>Strategy</th>
                <th>Parameters</th>
                <th>Learning samples</th>
              </tr>
            </thead>
            <tbody>
              {profiles.map((profile) => {
                const parameters =
                  profile.parameters
                    .map((parameter) => `${parameter.key}: ${parameter.value}`)
                    .join(' · ') || 'Default parameters';
                return (
                  <tr key={`${profile.symbol}:${profile.strategy}`}>
                    <td>{`${profile.symbol} · ${profile.strategy}`}</td>
                    <td>{profile.symbol}</td>
                    <td>{profile.strategy}</td>
                    <td title={parameters}>{parameters}</td>
                    <td>{profile.learning_samples}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
      <header className="auto-trader-learning-decisions-head">
        <strong>Learning Scout runs and rankings</strong>
        <small>
          {rows.length} recent evaluations · rank orders each batch by worker
          decision, then prior Scout sample count. It is a triage order, not a
          return forecast.
          {state.updatedAt
            ? ` Updated ${new Date(state.updatedAt).toLocaleTimeString()}.`
            : ''}
        </small>
      </header>
      {state.loading && rows.length === 0 && (
        <p>Loading Learning Scout activity…</p>
      )}
      {state.error && (
        <p className="auto-trader-learning-error" role="alert">
          Could not load Scout activity: {state.error}
        </p>
      )}
      {!state.loading && !state.error && rows.length === 0 && (
        <p>
          No recent Scout evaluations have been recorded since this service
          started. The configured profiles above are available for scouting.
        </p>
      )}
      {rows.length > 0 && (
        <div className="auto-trader-learning-decisions-table-wrap">
          <table className="auto-trader-learning-decisions-table">
            <thead>
              <tr>
                <th>Time</th>
                <th>Rank</th>
                <th>Symbol</th>
                <th>Strategy</th>
                <th>Profile / parameters</th>
                <th>Decision</th>
                <th>Prior Scout n</th>
                <th>Why / evidence</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => {
                const evidence = row.walk_forward_evidence;
                const explanation = why(
                  row as unknown as Record<string, unknown>,
                );
                const candidate = row.candidate as Record<string, unknown>;
                const decision = candidate.strategy_decision as
                  | Record<string, unknown>
                  | undefined;
                const parameters = decision?.strategy_parameters;
                const profile =
                  decision?.learning_profile_id ??
                  decision?.learning_profile_symbol ??
                  decision?.scout_parameter_profile_symbol;
                const profileText =
                  [profile, parameters && JSON.stringify(parameters)]
                    .filter(Boolean)
                    .join(' · ') || 'Baseline profile';
                const evidenceText = evidence
                  ? `Prior outcomes: ${evidence.sample_count}; positive ${((evidence.positive_net_rate ?? 0) * 100).toFixed(0)}%; average ${(evidence.average_net_return_percent ?? 0).toFixed(3)}%; lower bound ${evidence.expectancy_ci_low_percent?.toFixed(3) ?? 'unavailable'}%.`
                  : 'No prior matching Scout outcomes.';
                return (
                  <tr key={row.assignment_id}>
                    <td>{new Date(row.evaluated_at_ms).toLocaleString()}</td>
                    <td>{row.rank}</td>
                    <td>{row.symbol}</td>
                    <td>{row.strategy}</td>
                    <td title={profileText}>{profileText}</td>
                    <td>{row.decision.replaceAll('_', ' ')}</td>
                    <td>{row.evidence_sample_count}</td>
                    <td title={explanation}>
                      {explanation} {evidenceText}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
