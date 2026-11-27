import { useEffect } from 'react';
import {
  OperatorMetricStrip,
  OperatorPanel,
  OperatorStatus,
  OperatorTabBar,
  useBehaviorSnapshot,
} from '@workspace/ui-platform';
import type {
  AutoTraderExitPolicyModel,
  AutoTraderExitPolicyState,
  ExitPolicyAssessmentRow,
  ExitPositionRow,
  ExitProfileParameter,
  ExitProfileRow,
} from '../models/AutoTraderExitPolicyModel';

type Props = {
  state: AutoTraderExitPolicyState;
  onViewChange: (view: AutoTraderExitPolicyState['view']) => void;
  onStrategyChange: (strategy: string) => void;
  onSymbolChange: (symbol: string) => void;
};

export function AutoTraderExitPoliciesView({
  model,
}: {
  model: AutoTraderExitPolicyModel;
}) {
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  const state = useBehaviorSnapshot(model.state$);
  return (
    <AutoTraderExitPoliciesPanel
      state={state}
      onViewChange={(view) => model.setView(view)}
      onStrategyChange={(strategy) => model.setStrategyFilter(strategy)}
      onSymbolChange={(symbol) => model.setSymbolFilter(symbol)}
    />
  );
}

const percent = (value: number | undefined) =>
  typeof value === 'number' && Number.isFinite(value)
    ? `${value.toFixed(3)}%`
    : 'Unavailable';

const rawValue = (value: number | undefined, unit: string) =>
  typeof value === 'number' && Number.isFinite(value)
    ? `${value.toFixed(3)}${unit ? ` ${unit}` : ''}`
    : 'Unavailable';

function scopedPercent(value: unknown, profileId: unknown, version: unknown) {
  const hasProfile =
    typeof profileId === 'string' && profileId.trim().length > 0;
  const hasVersion = typeof version === 'number' && Number.isFinite(version);
  if (!hasProfile || !hasVersion)
    return 'Unavailable · exact profile ID/version is absent';
  return percent(typeof value === 'number' ? value : undefined);
}

function dateTime(value: number | undefined) {
  return typeof value === 'number' && Number.isFinite(value)
    ? new Date(value).toLocaleString()
    : 'Unavailable';
}

function triggerPrice(value: number | undefined) {
  return typeof value === 'number' && Number.isFinite(value)
    ? value.toPrecision(10)
    : 'Unavailable';
}

function durationMs(value: number | undefined) {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0
    ? `${value} ms`
    : 'Unavailable';
}

function FieldGrid({ fields }: { fields: Array<[string, string]> }) {
  return (
    <dl className="auto-trader-exit-policy__field-grid">
      {fields.map(([label, value]) => (
        <div key={label}>
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function FilterControls({
  state,
  onStrategyChange,
  onSymbolChange,
}: Pick<Props, 'state' | 'onStrategyChange' | 'onSymbolChange'>) {
  return (
    <div className="auto-trader-exit-policy__filters">
      <label>
        Strategy
        <select
          aria-label="Filter exit policies by strategy"
          value={state.strategyFilter}
          onChange={(event) => onStrategyChange(event.target.value)}
        >
          <option value="all">All strategies</option>
          {state.strategyOptions.map((strategy) => (
            <option key={strategy} value={strategy}>
              {strategy}
            </option>
          ))}
        </select>
      </label>
      <label>
        Symbol
        <select
          aria-label="Filter exit policies by symbol"
          value={state.symbolFilter}
          onChange={(event) => onSymbolChange(event.target.value)}
        >
          <option value="all">All symbols</option>
          {state.symbolOptions.map((symbol) => (
            <option key={symbol} value={symbol}>
              {symbol}
            </option>
          ))}
        </select>
      </label>
    </div>
  );
}

function rawParameter(row: ExitProfileRow, kind: ExitProfileParameter['kind']) {
  const parameters = row.parameters.filter(
    (parameter) => parameter.kind === kind,
  );
  if (parameters.length === 0)
    return 'Unavailable · no matching profile parameter is published';
  return parameters
    .map(
      (parameter) =>
        `${parameter.label}: raw profile value ${rawValue(parameter.rawServiceValue, parameter.unit)}`,
    )
    .join('; ');
}

function profileFields(row: ExitProfileRow): Array<[string, string]> {
  const noSource =
    'Unavailable · profile contract has no parameter source field';
  return [
    [
      'Exchange',
      'Unavailable · no exchange field in decision-parameter profile contract',
    ],
    ['Symbol', row.symbol],
    ['Strategy', row.strategy],
    [
      'Profile ID / version',
      `ID ${row.profileId ?? 'Unavailable'} · version ${row.profileVersion ?? 'Unavailable'} · source ${row.profileId || row.profileVersion !== undefined ? 'decision-parameter profile contract' : 'profile identity fields absent'}`,
    ],
    [
      'Supported direction',
      'Unavailable · no direction dimension in profile contract',
    ],
    [
      'Supported regime',
      row.regimeEvidenceKeys.length
        ? row.regimeEvidenceKeys.join(', ') +
          ' · walk-forward evidence keys only'
        : 'Unavailable · no regime-specific walk-forward rows',
    ],
    ['Stop loss', rawParameter(row, 'stop')],
    ['Take profit', rawParameter(row, 'target')],
    ['Maximum holding duration', rawParameter(row, 'maximumHolding')],
    ['Trailing stop', rawParameter(row, 'trailing')],
    ['Profit protection', rawParameter(row, 'profitProtection')],
    ['Planned reward-to-risk', rawParameter(row, 'plannedRewardRisk')],
    [
      'Scout candidate evidence',
      row.parameters.every((parameter) => parameter.scoutVariantsAvailable)
        ? 'Available · no candidate rows returned'
        : 'Unavailable · candidate endpoint did not populate one or more profile parameters',
    ],
    ['Parameter source', noSource],
    [
      'Effective sample size',
      row.effectiveSampleSize === undefined
        ? 'Unavailable · walk-forward evidence omits effectiveSampleSize'
        : String(row.effectiveSampleSize),
    ],
    [
      'Validation status',
      row.finalHoldoutStatus.replaceAll('_', ' ') +
        ` · history ${row.historyCoverage} · costs ${row.costCoverage}`,
    ],
    ['Last evidence sample', dateTime(row.lastEvidenceSampleTimestampMs)],
    [
      'Last evaluation',
      'Unavailable · API provides last sample time, not evaluation time',
    ],
    [
      'Last promotion',
      'Unavailable · decision-parameter profile has no promotion history',
    ],
  ];
}

function Profiles({ rows }: { rows: ExitProfileRow[] }) {
  if (rows.length === 0)
    return <p>No strategy profile matches the selected filters.</p>;
  const withExitParameters = rows.filter(
    (row) => row.parameters.length > 0,
  ).length;
  return (
    <>
      <p>
        Profile values below are raw service-reported parameters. They do not
        establish that AutoTrader selected the value for a live position or that
        it passed validation.
      </p>
      <p>
        {withExitParameters} of {rows.length} visible profiles publish an
        exit-named parameter. The decision-parameter API does not attach a
        selected-value source to those values.
      </p>
      <div className="auto-trader-exit-policy__profile-list">
        {rows.map((row) => (
          <article className="auto-trader-exit-policy__profile" key={row.id}>
            <header>
              <strong>
                {row.strategy} · {row.symbol}
              </strong>
              <span>{row.learningSamples} raw profile samples</span>
            </header>
            <FieldGrid fields={profileFields(row)} />
            {row.parameters.some(
              (parameter) => parameter.scoutVariants.length > 0,
            ) && (
              <div className="auto-trader-exit-policy__candidate-list">
                <strong>Service-reported Scout parameter candidates</strong>
                {row.parameters.flatMap((parameter) =>
                  parameter.scoutVariants.map((variant, index) => (
                    <span key={`${parameter.key}:${index}`}>
                      {parameter.label}:{' '}
                      {rawValue(variant.candidateValue, parameter.unit)}
                      {' · '}
                      {variant.sampleCount} samples
                      {' · '}
                      {variant.promoted
                        ? 'promoted flag'
                        : variant.rejected
                          ? 'rejected flag'
                          : 'no promotion/rejection flag'}
                    </span>
                  )),
                )}
                <small>
                  These parameter candidates are not an exit-policy promotion
                  history.
                </small>
              </div>
            )}
            {!row.parameters.some(
              (parameter) => parameter.scoutVariants.length > 0,
            ) && (
              <small>
                Scout candidate rows{' '}
                {row.parameters.every(
                  (parameter) => parameter.scoutVariantsAvailable,
                )
                  ? 'were not returned for this profile.'
                  : 'are unavailable from the current response.'}
              </small>
            )}
          </article>
        ))}
      </div>
    </>
  );
}

function initialRiskLabel(row: ExitPositionRow) {
  const evidence = row.initialRiskEvidence;
  if (!evidence)
    return 'Unavailable · active position response has no initial risk reference';
  if (!evidence.eligibleForRMultiple)
    return (
      `Unavailable · ${evidence.eligibilityStatus}` +
      (evidence.referenceStatus
        ? ` · reference ${evidence.referenceStatus}`
        : '')
    );
  return `${evidence.quoteAmount!.toFixed(6)} ${evidence.quoteCurrency} · ${evidence.fillIdCount} linked entry fills`;
}

function thresholdCell(value: number | undefined, active: boolean | undefined) {
  if (active === false) return `Inactive · value ${percent(value)}`;
  if (active === true) return `Active · ${percent(value)}`;
  return `${percent(value)} · active state unavailable`;
}

function Positions({ rows }: { rows: ExitPositionRow[] }) {
  if (rows.length === 0)
    return <p>No open positions are reported by the outcome contract.</p>;
  return (
    <div className="auto-trader-exit-policy__position-list">
      {rows.map((row) => (
        <article className="auto-trader-exit-policy__position" key={row.id}>
          <header>
            <strong>
              {row.symbol} · {row.strategy}
            </strong>
            <span>Decision {row.decisionId}</span>
          </header>
          <FieldGrid
            fields={[
              [
                'Exchange',
                'Unavailable · activity/outcome schema has no venue identifier',
              ],
              [
                'Entry profile ID / version',
                `${row.entryProfileId ?? 'Unavailable'} / ${row.entryProfileVersion ?? 'Unavailable'}`,
              ],
              ['Entry profile hash', row.entryProfileHash ?? 'Unavailable'],
              [
                'Exit profile ID / version',
                `${row.exitProfileId ?? 'Unavailable'} / ${row.exitProfileVersion ?? 'Unavailable'}`,
              ],
              ['Exit policy as of', dateTime(row.exitPolicyAsOfMs)],
              ['Lifecycle source', row.lifecycleSource ?? 'Unavailable'],
              [
                'Entry policy / execution mode',
                `${row.entryPolicy ?? 'Unavailable'} / ${row.executionMode ?? 'Unavailable'}`,
              ],
              [
                'Direction',
                row.signalDirection ??
                  'Unavailable · no linked entry audit direction',
              ],
              [
                'Regime',
                row.marketRegime ??
                  'Unavailable · no linked entry audit regime',
              ],
              [
                'Original selected stop',
                `${thresholdCell(row.selectedStopLossPercent, row.stopLossActive)} · source ${row.selectedStopLossSource ?? 'Unavailable'} · evidence ${row.selectedStopLossEvidenceSource ?? 'Unavailable'}`,
              ],
              [
                'Original selected target',
                `${thresholdCell(row.selectedTakeProfitPercent, row.takeProfitActive)} · source ${row.selectedTakeProfitSource ?? 'Unavailable'} · evidence ${row.selectedTakeProfitEvidenceSource ?? 'Unavailable'}`,
              ],
              [
                'Bootstrap fallback stop / target',
                `${percent(row.fallbackStopLossPercent)} / ${percent(row.fallbackTakeProfitPercent)} · unavailable when not supplied by activity contract`,
              ],
              [
                'AutoTrader monitor-driven IOC thresholds · not PM barrier',
                row.currentThresholdCompatibility
                  ? `${percent(row.currentEffectiveStopLossPercent)} / ${percent(row.currentEffectiveTakeProfitPercent)} · compatibility fields may include fallback or adjustment; selection source unavailable`
                  : `${percent(row.currentEffectiveStopLossPercent)} / ${percent(row.currentEffectiveTakeProfitPercent)} · activity effective fields`,
              ],
              [
                'Maximum holding duration',
                `${durationMs(row.maximumHoldingDurationMs)} · source ${row.maximumHoldingSource ?? 'Unavailable · provenance field absent'}`,
              ],
              [
                'Trailing stop activation / distance',
                `${percent(row.trailingActivationPercent)} / ${percent(row.trailingDistancePercent)} · source ${row.trailingSource ?? 'Unavailable · provenance field absent'}`,
              ],
              [
                'Breakeven activation',
                `${percent(row.breakevenActivationPercent)} · source ${row.breakevenSource ?? 'Unavailable · provenance field absent'}`,
              ],
              [
                'Profit protection activation / giveback / minimum lock',
                `${percent(row.profitProtectionActivationPercent)} / ${percent(row.profitProtectionGivebackPercent)} / ${percent(row.profitProtectionMinimumLockPercent)} · source ${row.profitProtectionSource ?? 'Unavailable · provenance field absent'}`,
              ],
              [
                'Current PM server-side protective barrier',
                row.currentPmBarrierId
                  ? `${row.currentPmBarrierId} · trigger price ${triggerPrice(row.currentPmBarrierTriggerPrice)} · joined by exact profile ID/version`
                  : 'Unavailable · activity strategy_positions has no exact-profile PM protective_barrier DTO',
              ],
              [
                'Persisted protection adjustment count',
                `${row.protectionAdjustments.length}${row.protectionAdjustmentsDropped ? ` · ${row.protectionAdjustmentsDropped} older AutoTrader records dropped by retention` : ''}`,
              ],
              [
                'Planned reward-to-risk',
                'Unavailable · no selected reward-to-risk field in current activity contract',
              ],
              ['Initial monetary risk', initialRiskLabel(row)],
              [
                'Effective sample size / validation',
                'Unavailable · position cannot be joined to an exact versioned profile evidence row',
              ],
              [
                'Last evaluation / promotion',
                'Unavailable · no position-linked evaluation/promotion history',
              ],
            ]}
          />
          <section
            className="auto-trader-exit-policy__path"
            aria-label={`Protection adjustments for ${row.symbol} ${row.strategy}`}
          >
            <strong>Durable protection adjustments</strong>
            {row.protectionAdjustments.length ? (
              <ul>
                {row.protectionAdjustments.map((adjustment) => (
                  <li key={adjustment.id}>
                    {adjustment.reason ?? 'Reason unavailable'} ·{' '}
                    {triggerPrice(adjustment.previousTriggerPrice)} →{' '}
                    {triggerPrice(adjustment.newTriggerPrice)} · source{' '}
                    {adjustment.source ?? 'Unavailable'} ·{' '}
                    {dateTime(adjustment.timestampMs)} ·{' '}
                    {adjustment.recordSource}
                  </li>
                ))}
              </ul>
            ) : (
              <p>
                Unavailable · no PM trigger_adjustments or AutoTrader
                protection_adjustments were returned.
              </p>
            )}
          </section>
          <div className="auto-trader-exit-policy__path">
            <strong>Executable path and post-exit evidence</strong>
            {row.executablePricePathEvidence ? (
              <p>
                Path{' '}
                {row.executablePricePathEvidence.complete === true
                  ? 'complete'
                  : 'incomplete'}{' '}
                · MFE{' '}
                {row.executablePricePathEvidence.complete === true
                  ? percent(
                      row.executablePricePathEvidence.grossExecutableMfePercent,
                    )
                  : 'Unavailable · incomplete path'}{' '}
                · MAE{' '}
                {row.executablePricePathEvidence.complete === true
                  ? percent(
                      row.executablePricePathEvidence.grossExecutableMaePercent,
                    )
                  : 'Unavailable · incomplete path'}{' '}
                · samples{' '}
                {row.executablePricePathEvidence.retainedSampleCount ??
                  'Unavailable'}{' '}
                /{' '}
                {row.executablePricePathEvidence.totalSampleCount ??
                  'Unavailable'}
                {row.executablePricePathEvidence.missingCoverage?.length
                  ? ` · gaps ${row.executablePricePathEvidence.missingCoverage.join(', ')}`
                  : ''}
              </p>
            ) : (
              <p>
                Unavailable · no linked executable price-path evidence in the
                current outcome response.
              </p>
            )}
            <p>
              Post-exit observation:{' '}
              {row.postExitObservedAt === undefined
                ? 'Unavailable · position is still open or no observation was returned'
                : `observed ${dateTime(row.postExitObservedAt)} · best ${percent(row.postExitBestReturnPercent)} · worst ${percent(row.postExitWorstReturnPercent)} · missed ${percent(row.postExitMissedReturnPercent)}`}
            </p>
          </div>
        </article>
      ))}
    </div>
  );
}

function Contexts({ state }: { state: AutoTraderExitPolicyState }) {
  if (!state.decisionParametersAvailable)
    return <p>Decision-parameter contexts are unavailable.</p>;
  if (state.contexts.length === 0)
    return <p>No decision-parameter contexts were returned.</p>;
  return (
    <>
      <p>
        These contexts report learning and entry-eligibility summaries. They do
        not publish exit-policy values or exit-policy validation history.
      </p>
      <div className="auto-trader-exit-policy__table-wrap">
        <table>
          <thead>
            <tr>
              <th scope="col">Context</th>
              <th scope="col">Trade samples</th>
              <th scope="col">Positive rate</th>
              <th scope="col">Scout samples</th>
              <th scope="col">Exit policy</th>
            </tr>
          </thead>
          <tbody>
            {state.contexts.map((context) => (
              <tr key={context.context}>
                <th scope="row">{context.context}</th>
                <td>{context.learningSamples}</td>
                <td>
                  {context.positiveRate === undefined
                    ? 'Unavailable'
                    : `${(context.positiveRate * 100).toFixed(1)}%`}
                </td>
                <td>{context.scoutLearningSamples}</td>
                <td>Unavailable · not published by context contract</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </>
  );
}

function EvidenceDrilldown({ state }: { state: AutoTraderExitPolicyState }) {
  const candidates = state.filteredProfiles.flatMap((profile) =>
    profile.parameters.flatMap((parameter) =>
      parameter.scoutVariants.map((variant) => ({
        profile,
        parameter,
        variant,
      })),
    ),
  );
  const paths = [
    ...state.filteredPositions.map((row) => ({
      id: row.id,
      symbol: row.symbol,
      strategy: row.strategy,
      evidence: row.executablePricePathEvidence,
      postExitObservedAt: row.postExitObservedAt,
      postExitBest: row.postExitBestReturnPercent,
      postExitWorst: row.postExitWorstReturnPercent,
    })),
    ...state.filteredClosedEvidence.map((row) => ({
      id: row.id,
      symbol: row.symbol,
      strategy: row.strategy,
      evidence: row.executablePricePathEvidence,
      postExitObservedAt: row.postExitObservedAt,
      postExitBest: row.postExitBestReturnPercent,
      postExitWorst: row.postExitWorstReturnPercent,
    })),
  ];
  const profileEvidence = state.filteredProfiles;
  return (
    <>
      <p>
        Drill-down is read-only. Candidate flags and path evidence are displayed
        only when their source fields exist; this view never writes trading
        configuration.
      </p>
      <section>
        <h3>Previous and candidate values</h3>
        {candidates.length === 0 ? (
          <p>
            {profileEvidence.every((profile) =>
              profile.parameters.every(
                (parameter) => parameter.scoutVariantsAvailable,
              ),
            )
              ? 'No exit-parameter Scout candidate rows were returned for the selected profiles.'
              : 'Unavailable · one or more selected profile responses omit Scout candidate details; this is not evidence that no candidates exist.'}
          </p>
        ) : (
          <ul>
            {candidates.map(({ profile, parameter, variant }, index) => (
              <li key={`${profile.id}:${parameter.key}:${index}`}>
                {profile.strategy} · {profile.symbol} · {parameter.label}: raw
                profile value{' '}
                {rawValue(parameter.rawServiceValue, parameter.unit)}
                {' → '}
                {rawValue(variant.candidateValue, parameter.unit)} ·{' '}
                {variant.sampleCount} candidate samples · promoted flag{' '}
                {String(variant.promoted)} · rejected flag{' '}
                {String(variant.rejected)}· candidate policy provenance
                unavailable
              </li>
            ))}
          </ul>
        )}
      </section>
      <section>
        <h3>Executable excursion and post-exit paths</h3>
        {paths.length === 0 ? (
          <p>
            Unavailable · no linked open-position or closed-summary path rows
            match the filters.
          </p>
        ) : (
          <ul>
            {paths.map((path) => (
              <li key={path.id}>
                {path.symbol} · {path.strategy}:{' '}
                {path.evidence
                  ? `path ${path.evidence.complete === true ? 'complete' : 'incomplete'}; executable MFE ${path.evidence.complete === true ? percent(path.evidence.grossExecutableMfePercent) : 'unavailable'}; executable MAE ${path.evidence.complete === true ? percent(path.evidence.grossExecutableMaePercent) : 'unavailable'}`
                  : 'executable MFE/MAE unavailable'}
                ; post-exit{' '}
                {path.postExitObservedAt === undefined
                  ? 'unavailable'
                  : `observed ${dateTime(path.postExitObservedAt)}, best ${percent(path.postExitBest)}, worst ${percent(path.postExitWorst)}`}
              </li>
            ))}
          </ul>
        )}
      </section>
      <FieldGrid
        fields={[
          [
            'What-if assumptions / results',
            'Unavailable · no read-only What-if API/model contract is connected; no settings-write action is exposed',
          ],
          [
            'Sensitivity heatmaps',
            'Unavailable · current profile/outcome contracts have no policy sensitivity series',
          ],
          [
            'Winner / loser transitions',
            'Unavailable · no policy-version transition history is exposed',
          ],
          [
            'Out-of-sample validation',
            profileEvidence.some((row) => row.oosFoldCount !== undefined)
              ? `${profileEvidence.filter((row) => row.oosFoldCount !== undefined).length} filtered profiles expose walk-forward fold counts; per-profile status is in Strategy profiles`
              : 'Unavailable · filtered profiles expose no OOS fold count',
          ],
          [
            'Promotion history',
            'Unavailable · current candidate flags are not a versioned promotion history',
          ],
          [
            'Accept / reject reasons',
            'Unavailable · profile candidate rows expose flags and measurements, not decision reasons',
          ],
        ]}
      />
    </>
  );
}

function CandidateAssessments({ state }: { state: AutoTraderExitPolicyState }) {
  if (state.assessmentStatus === 'loading')
    return <p role="status">Loading Learning exit-policy assessments…</p>;
  if (state.assessmentStatus === 'unavailable')
    return (
      <p role="status">
        Unavailable ·{' '}
        {state.assessmentError ?? 'assessment contract is not exposed'}
      </p>
    );
  if (state.assessments.length === 0)
    return <p>No candidate exit-policy assessments were returned.</p>;

  return (
    <>
      <p>
        These are computed candidate assessments from Learning. Promotion
        eligibility does not mean a candidate is published or active in
        AutoTrader. No metric is recalculated in the view.
      </p>
      <div className="auto-trader-exit-policy__profile-list">
        {state.assessments.map((assessment) => (
          <AssessmentCard key={assessment.id} assessment={assessment} />
        ))}
      </div>
      <section className="auto-trader-exit-policy__published">
        <h3>Learning-published exit protection policies</h3>
        {!state.publishedPoliciesAvailable ? (
          <p>
            Unavailable · exit_protection_policies is absent from the returned
            snapshot.
          </p>
        ) : state.publishedPolicies.length === 0 ? (
          <p>No policy is currently published by the Learning snapshot.</p>
        ) : (
          <ul>
            {state.publishedPolicies.map((policy, index) => (
              <li key={`${policy.symbol}:${policy.strategy}:${index}`}>
                Profile ID {policy.profile_id?.trim() || 'Unavailable'} ·
                version {policy.profile_version ?? 'Unavailable'} ·{' '}
                {policy.strategy} · {policy.symbol}: stop{' '}
                {percent(policy.stop_loss_percent ?? undefined)}, target{' '}
                {percent(policy.take_profit_percent ?? undefined)}, protection
                maximum hold{' '}
                {policy.maximum_holding_duration_ms ?? 'Unavailable'} ms,
                protection activation {percent(policy.activation_percent)},
                giveback {percent(policy.giveback_percent)}, minimum lock{' '}
                {percent(policy.minimum_lock_percent)}, trailing activation /
                distance{' '}
                {scopedPercent(
                  (policy as unknown as Record<string, unknown>)[
                    'trailing_activation_percent'
                  ],
                  policy.profile_id,
                  policy.profile_version,
                )}{' '}
                /{' '}
                {scopedPercent(
                  (policy as unknown as Record<string, unknown>)[
                    'trailing_distance_percent'
                  ],
                  policy.profile_id,
                  policy.profile_version,
                )}
                , breakeven activation{' '}
                {scopedPercent(
                  (policy as unknown as Record<string, unknown>)[
                    'breakeven_activation_percent'
                  ],
                  policy.profile_id,
                  policy.profile_version,
                )}{' '}
                · {policy.sample_count} samples ({policy.training_count}{' '}
                training / {policy.validation_count} validation)
              </li>
            ))}
          </ul>
        )}
        <p>
          AutoTrader active-policy application/readback is unavailable from this
          contract. Learning publication is not proof of runtime activation.
        </p>
      </section>
    </>
  );
}

function AssessmentCard({
  assessment,
}: {
  assessment: ExitPolicyAssessmentRow;
}) {
  const eligibility =
    assessment.promotionEligible === true
      ? 'Eligible candidate · not published or active'
      : assessment.promotionEligible === false
        ? 'Not promotion-eligible'
        : 'Promotion eligibility unavailable';
  return (
    <article className="auto-trader-exit-policy__profile">
      <header>
        <strong>
          Assessment candidate · {assessment.strategy} · {assessment.symbol}
        </strong>
        <span>{eligibility}</span>
      </header>
      <FieldGrid
        fields={[
          ['Assessment as of', dateTime(assessment.asOfMs)],
          [
            'Profile ID / version',
            `ID ${assessment.profileId ?? 'Unavailable'} · version ${assessment.profileVersion ?? 'Unavailable'} · Learning assessment identity; publication/activation are separate`,
          ],
          [
            'Direction / regime',
            'Unavailable · assessment contract exposes no direction or regime',
          ],
          [
            'Candidate stop / target',
            `${percent(assessment.stopLossPercent)} / ${percent(assessment.takeProfitPercent)}`,
          ],
          [
            'Candidate protection',
            `activation ${percent(assessment.activationPercent)} · giveback ${percent(assessment.givebackPercent)} · minimum lock ${percent(assessment.minimumLockPercent)}`,
          ],
          [
            'Candidate trailing activation / distance',
            `${percent(assessment.trailingActivationPercent)} / ${percent(assessment.trailingDistancePercent)} · ${assessment.trailingCoverageStatus}${assessment.trailingCoverageStatus === 'partial' ? ' · incomplete pair; not a usable trailing policy' : assessment.trailingCoverageStatus === 'unavailable' ? ' · unavailable or exact profile identity missing' : ' · exact profile-version candidate'}`,
          ],
          [
            'Candidate breakeven activation',
            assessment.breakevenActivationPercent === undefined
              ? 'Unavailable · candidate field absent or exact profile identity missing'
              : `${percent(assessment.breakevenActivationPercent)} · exact profile-version candidate`,
          ],
          [
            'Candidate maximum holding duration',
            assessment.maximumHoldingDuration?.value === undefined
              ? `Unavailable · ${assessment.maximumHoldingDuration?.coverageStatus ?? 'coverage unavailable'} · ${assessment.maximumHoldingDuration?.reason ?? 'no source-backed candidate duration returned'}`
              : `${assessment.maximumHoldingDuration.value} ms · ${assessment.maximumHoldingDuration.coverageStatus} · sources ${assessment.maximumHoldingDuration.sourceFields.join(', ') || 'unavailable'}${assessment.maximumHoldingDuration.reason ? ` · ${assessment.maximumHoldingDuration.reason}` : ''}`,
          ],
          [
            'Samples',
            `${assessment.sampleCount ?? 'Unavailable'} total · ${assessment.trainingCount ?? 'Unavailable'} training · ${assessment.validationCount ?? 'Unavailable'} validation`,
          ],
          ['Promotion eligibility', eligibility],
        ]}
      />
      <h4>Computed metrics with per-metric evidence coverage</h4>
      {assessment.metrics.length === 0 ? (
        <p>No metric value or coverage key was returned.</p>
      ) : (
        <div className="auto-trader-exit-policy__table-wrap">
          <table>
            <thead>
              <tr>
                <th scope="col">Metric (key carries units)</th>
                <th scope="col">Computed value</th>
                <th scope="col">Coverage</th>
                <th scope="col">Samples</th>
                <th scope="col">Source fields</th>
                <th scope="col">Reason</th>
              </tr>
            </thead>
            <tbody>
              {assessment.metrics.map((metric) => (
                <tr key={metric.key}>
                  <th scope="row">{metric.key}</th>
                  <td>
                    {metric.value === undefined
                      ? 'Unavailable'
                      : String(metric.value)}
                  </td>
                  <td>{metric.coverageStatus.replaceAll('_', ' ')}</td>
                  <td>{metric.sampleCount ?? 'Unavailable'}</td>
                  <td>
                    {metric.sourceFields.length
                      ? metric.sourceFields.join(', ')
                      : 'Unavailable · no source fields returned'}
                  </td>
                  <td>{metric.reason ?? 'Unavailable · no reason returned'}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      <section aria-label="Assessment promotion blockers">
        <strong>Promotion blockers</strong>
        {!assessment.blockersAvailable ? (
          <p>Unavailable · blocker list is absent from the report.</p>
        ) : assessment.promotionBlockers.length > 0 ? (
          <ul>
            {assessment.promotionBlockers.map((blocker) => (
              <li key={blocker}>{blocker}</li>
            ))}
          </ul>
        ) : assessment.promotionEligible === true ? (
          <p>No blockers returned; the assessment remains a candidate.</p>
        ) : (
          <p>
            No blocker strings returned; this does not establish eligibility.
          </p>
        )}
      </section>
    </article>
  );
}

export function AutoTraderExitPoliciesPanel({
  state,
  onViewChange,
  onStrategyChange,
  onSymbolChange,
}: Props) {
  const hasProfileExitParameters = state.profileRows.some(
    (row) => row.parameters.length > 0,
  );
  const status =
    state.view === 'assessments'
      ? state.assessmentStatus === 'available'
        ? {
            tone: 'neutral' as const,
            label: 'Learning assessment snapshot available',
          }
        : state.assessmentStatus === 'loading'
          ? { tone: 'warning' as const, label: 'Loading Learning assessments' }
          : {
              tone: 'warning' as const,
              label: 'Learning assessments unavailable',
            }
      : !state.decisionParametersAvailable
        ? { tone: 'warning' as const, label: 'Decision parameters unavailable' }
        : hasProfileExitParameters
          ? {
              tone: 'warning' as const,
              label: 'Raw profile values; selected source not provided',
            }
          : {
              tone: 'warning' as const,
              label: 'Profile-level exit values unavailable',
            };

  return (
    <div className="auto-trader-exit-policy">
      <OperatorPanel
        title="Learned Exit Policies"
        context={
          state.decisionParametersRefreshedAt === undefined
            ? 'Profile refresh unavailable'
            : `Profile refresh ${dateTime(state.decisionParametersRefreshedAt)}`
        }
        status={
          <OperatorStatus tone={status.tone}>{status.label}</OperatorStatus>
        }
        footer="Selected, fallback, and current effective thresholds are separate. Missing provenance remains unavailable; this widget is read-only."
      >
        <OperatorMetricStrip
          label="Exit policy evidence coverage"
          metrics={[
            { label: 'Strategy profiles', value: state.profileRows.length },
            {
              label: 'Profiles with exit-named fields',
              value: state.profileRows.filter(
                (row) => row.parameters.length > 0,
              ).length,
            },
            { label: 'Open positions', value: state.positionRows.length },
            {
              label: 'Closed evidence rows',
              value: state.closedEvidenceRows.length,
            },
          ]}
        />
        <OperatorTabBar
          label="Exit policy evidence views"
          tabs={[
            { id: 'profiles', label: 'Strategy profiles' },
            { id: 'positions', label: 'Open positions' },
            { id: 'contexts', label: 'Learning contexts' },
            { id: 'drilldown', label: 'Evidence drill-down' },
            { id: 'assessments', label: 'Candidate assessments' },
          ]}
          selectedId={state.view}
          onSelect={(id) =>
            onViewChange(id as AutoTraderExitPolicyState['view'])
          }
        />
        {state.view !== 'contexts' && (
          <FilterControls
            state={state}
            onStrategyChange={onStrategyChange}
            onSymbolChange={onSymbolChange}
          />
        )}
        {state.settingsLoading && !state.decisionParametersAvailable ? (
          <p role="status">Loading decision-parameter profiles…</p>
        ) : state.view === 'profiles' ? (
          <Profiles rows={state.filteredProfiles} />
        ) : state.view === 'positions' ? (
          state.outcomesLoading && state.positionRows.length === 0 ? (
            <p role="status">Loading position selection and risk evidence…</p>
          ) : (
            <Positions rows={state.filteredPositions} />
          )
        ) : state.view === 'contexts' ? (
          <Contexts state={state} />
        ) : state.view === 'assessments' ? (
          <CandidateAssessments state={state} />
        ) : (
          <EvidenceDrilldown state={state} />
        )}
      </OperatorPanel>
    </div>
  );
}
