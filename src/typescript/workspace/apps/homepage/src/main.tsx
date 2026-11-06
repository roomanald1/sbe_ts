import React, { useMemo } from 'react';
import { createRoot } from 'react-dom/client';
import { useObservable } from '@workspace/ui-platform';
import '@workspace/ui-platform/styles/theme.css';
import './style.css';
import {
  ServiceControlModel,
  services,
  swaggerPorts,
  statusFor,
  initialServiceSnapshot,
  initialServiceTotals,
  type Project,
} from './models/ServiceControlModel';

function App() {
  const model = useMemo(() => new ServiceControlModel(), []);
  const { state, updated, available } = useObservable(
    model.snapshot$,
    initialServiceSnapshot,
    [model],
  );
  const notice = useObservable(model.notice$, null, [model]);
  const totals = useObservable(model.totals$, initialServiceTotals, [model]);

  function ServiceTable({ project }: { project: Project }) {
    return (
      <div className="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Service</th>
              <th>Status</th>
              <th>Actions</th>
            </tr>
          </thead>
          <tbody>
            {services[project].map((name) => {
              const status = statusFor(state[project].get(name));
              const port = swaggerPorts[name];
              return (
                <tr className={status.kind} key={name}>
                  <td>{name}</td>
                  <td>
                    <div className="status">
                      <span className="badge">{status.label}</span>
                      <span className="detail">{status.detail}</span>
                    </div>
                  </td>
                  <td>
                    <div className="actions">
                      <button
                        className="start"
                        disabled={status.kind === 'running'}
                        onClick={() =>
                          void model.action(project, 'start', name)
                        }
                      >
                        Start
                      </button>
                      <button
                        className="stop"
                        disabled={status.kind === 'stopped'}
                        onClick={() => void model.action(project, 'stop', name)}
                      >
                        Stop
                      </button>
                      <button
                        disabled={status.kind === 'stopped'}
                        onClick={() =>
                          void model.action(project, 'restart', name)
                        }
                      >
                        Restart
                      </button>
                      <button
                        className="start"
                        onClick={() =>
                          void model.action(project, 'rebuild-restart', name)
                        }
                      >
                        Rebuild + restart
                      </button>
                      {port && (
                        <a
                          href={`http://localhost:${port}/docs`}
                          target="_blank"
                          rel="noreferrer"
                        >
                          Swagger ↗
                        </a>
                      )}
                    </div>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    );
  }

  return (
    <main className="shell">
      <header className="topbar">
        <div className="brand">
          <div className="brand-mark">S</div>
          <div>
            <div className="eyebrow">SBE TS · LOCAL</div>
            <h1>Service control</h1>
          </div>
        </div>
        <nav aria-label="Dashboard sections">
          <a href="#code">Code</a>
          <a href="#system-title">System</a>
          <a href="#infra-title">Infrastructure</a>
        </nav>
      </header>
      <section className="hero">
        <div>
          <h2>Your stack, at a glance.</h2>
          <p>
            Start, stop, or restart services independently. A service going
            offline won’t take the rest of the system with it.
          </p>
        </div>
        <div className="hero-actions">
          <button
            className="button primary"
            onClick={() => void model.action('all', 'start')}
          >
            ▶&nbsp; Launch everything
          </button>
          <button
            className="button danger"
            onClick={() => void model.action('all', 'stop')}
          >
            ■&nbsp; Stop everything
          </button>
        </div>
      </section>
      <div className="overview">
        <span>
          <strong>{totals.running}</strong> running
        </span>
        <span className="divider" />
        <span>
          <strong>{totals.pending}</strong> starting
        </span>
        <span className="divider" />
        <span>
          <strong>{totals.stopped}</strong> stopped
        </span>
        <span className="refresh">
          <i className={available ? 'live-dot' : 'live-dot unavailable'} />
          {updated}
        </span>
      </div>
      <section className="section" id="code">
        <div className="section-heading">
          <div className="section-title">
            <div className="section-icon">⌘</div>
            <div>
              <h2>Code</h2>
              <p>Build quality, coverage, and observability</p>
            </div>
          </div>
        </div>
        <div className="hero-actions">
          <a className="button" href="/benchmarks/">
            ◷&nbsp; Benchmarks
          </a>
          <a className="button" href="/coverage/">
            ◫&nbsp; Coverage
          </a>
          <a
            className="button"
            href="http://localhost:3000"
            target="_blank"
            rel="noreferrer"
          >
            ▦&nbsp; Grafana ↗
          </a>
        </div>
      </section>
      <section className="section">
        <div className="section-heading">
          <div className="section-title">
            <div className="section-icon">⌘</div>
            <div>
              <h2 id="system-title">System</h2>
              <p>Application services and web client</p>
            </div>
          </div>
          <div className="section-tools">
            <button
              className="button small"
              onClick={() => void model.action('system', 'start')}
            >
              Start system
            </button>
            <button
              className="button small"
              onClick={() => void model.action('system', 'stop')}
            >
              Stop system
            </button>
          </div>
        </div>
        <ServiceTable project="system" />
      </section>
      <section className="section">
        <div className="section-heading">
          <div className="section-title">
            <div className="section-icon">◈</div>
            <div>
              <h2 id="infra-title">Infrastructure</h2>
              <p>Monitoring, metrics, logs, and controls</p>
            </div>
          </div>
          <div className="section-tools">
            <button
              className="button small"
              onClick={() => void model.action('infra', 'start')}
            >
              Start infra
            </button>
            <button
              className="button small"
              onClick={() => void model.action('infra', 'stop')}
            >
              Stop infra
            </button>
          </div>
        </div>
        <ServiceTable project="infra" />
      </section>
      <footer>
        <span>Service status refreshes every 10 seconds</span>
        <span>Local development environment</span>
      </footer>
      {notice && (
        <div
          className={`message${notice.error ? ' error' : ''}`}
          role="status"
          aria-live="polite"
        >
          {notice.text}
        </div>
      )}
    </main>
  );
}

createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
