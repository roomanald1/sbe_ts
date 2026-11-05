import { useEffect, useMemo } from 'react';
import { useObservable } from '@workspace/ui-platform';
import {
  BenchmarkReportModel,
  formatDate,
  plainText,
  type Table,
} from '../models/BenchmarkReportModel';

function MetricTable({ table }: { table: Table }) {
  const changeIndex = table.headers.findIndex((header) =>
    /^(change|difference)$/i.test(header.trim()),
  );
  const reference = table.headers.find((header) =>
    /baseline|previous|latest comparable|prior/i.test(header),
  );
  const headers = table.headers.map((header, index) => {
    if (index === changeIndex)
      return reference ? `Change vs ${reference}` : 'Change';
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
                    <ChangeValue value={cell} />
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

function ChangeValue({ value }: { value: string }) {
  const change = /^([+-])\s*(\d+(?:\.\d+)?%.*)$/.exec(value.trim());
  if (!change) return <>{value.replace(/`/g, '')}</>;

  const direction = change[1] === '+' ? 'slower' : 'faster';
  return (
    <span
      className={`change-value ${direction}`}
      aria-label={`${change[1] === '+' ? 'Increase' : 'Decrease'} of ${change[2]}, ${direction} for timing`}
      title={`${change[1]} means the current timing is ${direction} than its comparison reference`}
    >
      <b>{change[1]}</b> {change[2]} <small>{direction}</small>
    </span>
  );
}

export default function App() {
  const model = useMemo(() => new BenchmarkReportModel(), []);
  const state = useObservable(model.state$, model.state$.value, [model]);

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
              Each change is measured against the reference named in its column
              header.
            </span>
            <span>
              <b className="key-slower">+</b> current timing is slower
            </span>
            <span>
              <b className="key-faster">−</b> current timing is faster
            </span>
          </div>

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
