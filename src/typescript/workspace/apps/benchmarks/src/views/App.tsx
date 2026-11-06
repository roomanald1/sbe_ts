import { useEffect, useMemo } from 'react';
import { useObservable } from '@workspace/ui-platform';
import {
  BenchmarkReportModel,
  formatDate,
  plainText,
  readTables,
  type Table,
} from '../models/BenchmarkReportModel';

type ComparisonRow = {
  operation: string;
  baseline: string;
  previous: string;
  current: string;
  previousChange: string;
  baselineChange: string;
};

function comparisonRows(
  history: { report: { title: string }; tables: Table[] }[],
  baselineTables: Table[],
): ComparisonRow[] {
  const baselineValues = new Map<string, string>();
  for (const table of baselineTables) {
    const operationIndex = table.headers.findIndex((header) =>
      /workload|operation/i.test(header),
    );
    const valueIndex = table.headers.findIndex((header) =>
      /^(result|baseline)$/i.test(header.trim()),
    );
    if (operationIndex < 0 || valueIndex < 0) continue;
    for (const row of table.rows) {
      if (row[operationIndex] && row[valueIndex])
        baselineValues.set(row[operationIndex], row[valueIndex]);
    }
  }

  const latest = new Map<string, ComparisonRow>();
  const priorValues = new Map<string, string>();
  for (const run of [...history].reverse()) {
    for (const table of run.tables) {
      const operationIndex = table.headers.findIndex((header) =>
        /workload|operation/i.test(header),
      );
      const currentIndex = table.headers.findIndex((header) =>
        /current|repeat|final run/i.test(header),
      );
      const previousIndex = table.headers.findIndex((header) =>
        /previous run|latest comparable|prior run/i.test(header),
      );
      const baselineIndex = table.headers.findIndex((header) =>
        /baseline/i.test(header),
      );
      const changeIndex = table.headers.findIndex((header) =>
        /^(change|difference)$/i.test(header.trim()),
      );
      if (operationIndex < 0 || currentIndex < 0) continue;
      for (const row of table.rows) {
        const operation = row[operationIndex]?.replace(/`/g, '').trim();
        const current = row[currentIndex]?.replace(/`/g, '').trim();
        if (!operation || !current) continue;
        const previous =
          (previousIndex >= 0 ? row[previousIndex] : priorValues.get(operation))
            ?.replace(/`/g, '')
            .trim() ?? '';
        const baseline =
          (baselineIndex >= 0
            ? row[baselineIndex]
            : baselineValues.get(operation)
          )
            ?.replace(/`/g, '')
            .trim() ?? '';
        const change = changeIndex >= 0 ? (row[changeIndex]?.trim() ?? '') : '';
        latest.set(operation, {
          operation,
          baseline,
          previous,
          current,
          previousChange: previousIndex >= 0 ? change : '',
          baselineChange: baselineIndex >= 0 ? change : '',
        });
        priorValues.set(operation, current);
      }
    }
  }
  return [...latest.values()];
}

function ComparisonTable({ rows }: { rows: ComparisonRow[] }) {
  if (!rows.length) return null;
  return (
    <div className="table-scroll comparison-table">
      <table>
        <thead>
          <tr>
            <th>Operation</th>
            <th>Baseline</th>
            <th>Previous</th>
            <th>Current</th>
            <th>Change vs baseline</th>
            <th>Change vs previous</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.operation}>
              <td>{row.operation}</td>
              <td>{row.baseline || '—'}</td>
              <td>{row.previous || '—'}</td>
              <td>{row.current}</td>
              <td>
                {row.baselineChange ? (
                  <ChangePercent value={row.baselineChange} />
                ) : (
                  '—'
                )}
              </td>
              <td>
                {row.previousChange ? (
                  <ChangePercent value={row.previousChange} />
                ) : (
                  '—'
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function MetricTable({ table }: { table: Table }) {
  const changeIndex = table.headers.findIndex((header) =>
    /^(change|difference)$/i.test(header.trim()),
  );
  const reference = table.headers.find((header) =>
    /baseline|previous|latest comparable|prior|recorded run/i.test(header),
  );
  const referenceLabel = reference
    ? /baseline/i.test(reference)
      ? 'baseline'
      : 'previous run'
    : 'comparison reference';
  const headers = table.headers.map((header, index) => {
    if (index === changeIndex) return `Change vs ${referenceLabel}`;
    if (/^current(?: run)?$/i.test(header.trim())) return 'Current run';
    return header;
  });

  return (
    <div className="table-scroll">
      <table>
        <thead>
          <tr>
            {headers.map((header, index) => (
              <th key={`${header}-${index}`}>{header}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {table.rows.map((row, rowIndex) => (
            <tr key={rowIndex}>
              {row.map((cell, index) => (
                <td
                  key={index}
                  className={index === changeIndex ? 'change-cell' : undefined}
                >
                  {index === changeIndex ? (
                    <ChangeValue value={cell} reference={referenceLabel} />
                  ) : (
                    cell.replace(/`/g, '')
                  )}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function ChangeValue({
  value,
  reference,
}: {
  value: string;
  reference: string;
}) {
  const change = /^([+-])\s*(\d+(?:\.\d+)?%.*)$/.exec(value.trim());
  if (!change) return <>{value.replace(/`/g, '')}</>;

  const timingDirection = /\b(slower|faster)\b/i.exec(change[2])?.[1];
  const magnitude = Math.abs(
    Number(change[2].match(/[+-]?\d+(?:\.\d+)?/)?.[0] ?? 0),
  );
  const tone = timingDirection
    ? timingDirection.toLowerCase()
    : change[1] === '+'
      ? 'increase'
      : 'decrease';
  const strength = Math.min(magnitude / 20, 1);
  const changePercent = `${Math.round(strength * 100)}%`;
  const description = timingDirection
    ? `${timingDirection} vs ${reference}`
    : `${change[1] === '+' ? 'increase' : 'decrease'} vs ${reference}`;
  return (
    <span
      className={`change-value ${tone}`}
      style={{ '--change-percent': changePercent } as React.CSSProperties}
      aria-label={`${change[1] === '+' ? 'Increase' : 'Decrease'} of ${change[2]}, ${description}`}
      title={`${change[1]} change vs ${reference}`}
    >
      <b>{change[1]}</b> {change[2]} <small>{description}</small>
    </span>
  );
}

function ChangePercent({ value }: { value: string }) {
  const match = /^([+-])\s*(\d+(?:\.\d+)?%)/.exec(value.trim());
  if (!match) return <>{value.replace(/`/g, '')}</>;
  const direction = /\b(slower|faster)\b/i.exec(value)?.[1]?.toLowerCase();
  const tone = direction ?? (match[1] === '+' ? 'increase' : 'decrease');
  const magnitude = Math.abs(Number(match[2].slice(0, -1)));
  const strength = `${Math.round(Math.min(magnitude / 20, 1) * 100)}%`;
  return (
    <span
      className={`change-value percent-only ${tone}`}
      style={{ '--change-percent': strength } as React.CSSProperties}
    >
      {match[1]} {match[2]}
    </span>
  );
}

export default function App() {
  const model = useMemo(() => new BenchmarkReportModel(), []);
  const state = useObservable(model.state$, model.state$.value, [model]);
  const baselineTables = readTables(
    state.reports.find((report) => report.title === 'Results')?.content ?? [],
  );
  const comparisons = comparisonRows(state.runHistory, baselineTables);

  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);

  return (
    <main className="page">
      <header className="topbar">
        <a className="brand" href="/">
          <span className="brand-mark">S</span>
          <span>
            <small>SBE TS / RESEARCH</small>
            <strong>Benchmarks</strong>
          </span>
        </a>
        <nav>
          <a href="/">Home</a>
          <a href="/coverage/">Coverage</a>
          <span className="live">
            <i /> Live from README
          </span>
        </nav>
      </header>
      <section className="intro">
        <div>
          <div className="eyebrow">RUST PERFORMANCE</div>
          <h1>Benchmarks</h1>
          <p>Status and history from the latest benchmark report.</p>
        </div>
        <a
          className="source-link"
          href={`${import.meta.env.BASE_URL}benchmark-baseline.md`}
        >
          Open source report <span>↗</span>
        </a>
      </section>

      {state.loading && (
        <div className="notice">Loading the benchmark report…</div>
      )}
      {state.error && <div className="notice error">{state.error}</div>}
      {!state.loading && !state.error && (
        <>
          <section className="overview-grid">
            <article className="status-card">
              <div className="card-label">LATEST RECORDED RUN</div>
              <div className={`status-pill ${state.status.tone}`}>
                <i />
                {state.status.label}
              </div>
              <h2>
                {state.latestRun?.title.replace(/\s*\([^)]*\)$/, '') ??
                  'No run heading found'}
              </h2>
              <p className="date-line">
                {formatDate(state.latestRun?.date ?? '')}
              </p>
              <p className="status-detail">
                {state.latestSummary ||
                  'The report contains no additional status summary for this run.'}
              </p>
            </article>
            <article className="stat-card">
              <span className="card-label">RECORDED RUNS</span>
              <strong>{state.runHistory.length}</strong>
              <span className="stat-caption">
                benchmark and follow-up entries
              </span>
              <div className="stat-foot">
                <span>Report source</span>
                <b>BENCHMARK_BASELINE.md</b>
              </div>
            </article>
            <article className="stat-card">
              <span className="card-label">BASELINE</span>
              <strong className="baseline-date">
                {formatDate(state.baselineDate)}
              </strong>
              <span className="stat-caption">first persisted comparison</span>
              <div className="stat-foot">
                <span>Environment</span>
                <b>WSL2 · Linux x86_64</b>
              </div>
            </article>
          </section>

          <div className="comparison-key">
            <strong>Reading the change</strong>
            <span>
              Change columns name their reference. Timing changes show faster or
              slower; other changes show increase or decrease.
            </span>
            <span>
              <b className="key-slower">+</b> timing is slower
            </span>
            <span>
              <b className="key-faster">−</b> timing is faster
            </span>
          </div>

          <section className="section">
            <div className="section-heading">
              <div>
                <div className="eyebrow">WORKLOAD COMPARISON</div>
                <h2>Baseline · previous · current</h2>
              </div>
              <span className="count">{comparisons.length} workloads</span>
            </div>
            <ComparisonTable rows={comparisons} />
            {!comparisons.length && (
              <p className="context">
                No run yet contains structured timing results with a current
                value.
              </p>
            )}
          </section>

          {state.currentTables.length > 0 && (
            <section className="section">
              <div className="section-heading">
                <div>
                  <div className="eyebrow">MOST RECENT MEASUREMENTS</div>
                  <h2>{state.latestRun?.title}</h2>
                </div>
                <span className="count">
                  {state.currentTables.reduce(
                    (sum, table) => sum + table.rows.length,
                    0,
                  )}{' '}
                  data rows
                </span>
              </div>
              {state.currentTables.map((table, index) => (
                <MetricTable key={index} table={table} />
              ))}
              {plainText(state.latestRun?.content ?? []).map((line, index) => (
                <p className="context" key={index}>
                  {line}
                </p>
              ))}
            </section>
          )}

          <section className="section history-section">
            <div className="section-heading">
              <div>
                <div className="eyebrow">THE RECORD</div>
                <h2>Run history</h2>
              </div>
              <span className="count">Newest first</span>
            </div>
            {state.runHistory.map(
              ({ report, status, tables, summary, detailReports }, index) => (
                <details
                  className="history-item"
                  key={`${report.title}-${index}`}
                  open={index === 0}
                >
                  <summary>
                    <span className="history-marker" />
                    <span className="history-main">
                      <strong>{report.title}</strong>
                      <small>
                        {formatDate(report.date)} ·{' '}
                        {tables.reduce(
                          (sum, table) => sum + table.rows.length,
                          0,
                        )}{' '}
                        result rows
                      </small>
                    </span>
                    <span className={`status-pill compact ${status.tone}`}>
                      <i />
                      {status.label}
                    </span>
                    <span className="chevron">⌄</span>
                  </summary>
                  <div className="history-body">
                    {summary && <p>{summary}</p>}
                    {tables.map((table, tableIndex) => (
                      <MetricTable key={tableIndex} table={table} />
                    ))}
                    {detailReports.map((item, reportIndex) => (
                      <div className="history-subsection" key={reportIndex}>
                        <h3>{item.title}</h3>
                        {plainText(item.content).map((line, lineIndex) => (
                          <p key={lineIndex}>{line}</p>
                        ))}
                      </div>
                    ))}
                  </div>
                </details>
              ),
            )}
          </section>
          <section className="reading-note">
            <span className="note-icon">i</span>
            <p>
              <strong>How to compare.</strong> These are single-run
              microbenchmarks, not CI thresholds. Compare matching workloads,
              build profiles, hardware, and load. Large movements without
              controlled repeat runs are investigation signals, not confirmed
              regressions.
            </p>
          </section>
        </>
      )}
      <footer>
        Report source: <code>research/BENCHMARK_BASELINE.md</code>
        <span> · </span>Rendered from the current build’s README
      </footer>
    </main>
  );
}
