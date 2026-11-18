import { useEffect, useState } from 'react';
import { autoTraderRpc } from '../auto_trader_rsocket';
import type { AutoTraderStrategyProfile } from '../models/AutoTraderModel';

const formatPercent = (value?: number | null) =>
  value == null || !Number.isFinite(value)
    ? '—'
    : `${(value * 100).toFixed(1)}%`;
const formatReturn = (value?: number | null) =>
  value == null || !Number.isFinite(value)
    ? '—'
    : `${value > 0 ? '+' : ''}${value.toFixed(3)}%`;

export function WalkForwardWidget() {
  const [profiles, setProfiles] = useState<AutoTraderStrategyProfile[]>([]);
  const [error, setError] = useState<string>();

  useEffect(() => {
    let active = true;
    const refresh = async () => {
      try {
        const response = await autoTraderRpc(
          'GET',
          '/internal/strategy/daily-scan-parameters',
        );
        if (!response.ok)
          throw new Error(`Walk-forward request failed (${response.status}).`);
        const data = (await response.json()) as {
          strategy_profiles?: AutoTraderStrategyProfile[];
        };
        if (active) {
          setProfiles(data.strategy_profiles ?? []);
          setError(undefined);
        }
      } catch (reason) {
        if (active)
          setError(
            reason instanceof Error
              ? reason.message
              : 'Could not load walk-forward state.',
          );
      }
    };
    void refresh();
    const timer = setInterval(() => void refresh(), 30_000);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, []);

  return (
    <section className="feed-section" aria-label="Walk-forward state">
      {error && profiles.length === 0 ? (
        <div className="book-popup-message">{error}</div>
      ) : profiles.length === 0 ? (
        <div className="book-popup-message">
          No allowed strategy profiles are available.
        </div>
      ) : (
        <div className="walk-forward-list">
          {profiles.map((profile) => {
            const evidence = profile.walk_forward_evidence;
            const regimes = Object.entries(
              profile.walk_forward_evidence_by_regime ?? {},
            );
            return (
              <article
                className="walk-forward-profile"
                key={`${profile.symbol}:${profile.strategy}`}
              >
                <header>
                  <strong>{profile.symbol}</strong>
                  <span>{profile.strategy}</span>
                </header>
                <div className="walk-forward-metrics">
                  <span>
                    <small>Samples</small>
                    <b>{evidence?.sample_count ?? 0}</b>
                  </span>
                  <span>
                    <small>Positive</small>
                    <b>{formatPercent(evidence?.positive_net_rate)}</b>
                  </span>
                  <span>
                    <small>Average net</small>
                    <b>{formatReturn(evidence?.average_net_return_percent)}</b>
                  </span>
                  <span>
                    <small>Last sample</small>
                    <b>
                      {evidence?.last_sample_timestamp_ms
                        ? new Date(
                            evidence.last_sample_timestamp_ms,
                          ).toLocaleString()
                        : '—'}
                    </b>
                  </span>
                </div>
                {regimes.length > 0 && (
                  <div className="walk-forward-regimes">
                    {regimes.map(([regime, value]) => (
                      <span key={regime}>
                        {regime}: {value.sample_count} samples ·{' '}
                        {formatPercent(value.positive_net_rate)} ·{' '}
                        {formatReturn(value.average_net_return_percent)}
                      </span>
                    ))}
                  </div>
                )}
              </article>
            );
          })}
        </div>
      )}
    </section>
  );
}
