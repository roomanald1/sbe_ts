import { useEffect, useState } from 'react';
import { insightsRequestStream } from '../insights_rsocket';
import {
  decodeNewsSnapshot,
  type NewsSnapshot,
} from '../models/NewsTickerModel';
import { FeedToolbar } from './FeedToolbar';

type Props = { streamKey: string };

export function NewsTicker({ streamKey }: Props) {
  const [snapshot, setSnapshot] = useState<NewsSnapshot>();
  const [error, setError] = useState<string>();
  useEffect(() => {
    let active = true;
    let cancel: (() => void) | undefined;
    void insightsRequestStream(
      'news.snapshot',
      (raw) => {
        if (!active) return;
        try {
          const next = decodeNewsSnapshot(raw);
          setSnapshot(next);
          setError(next.error ?? undefined);
        } catch {
          setError('Received an invalid backend news update.');
        }
      },
      (streamError) => {
        if (active) setError(streamError.message);
      },
    ).then((stream) => {
      cancel = stream.cancel;
      if (!active) cancel();
    });
    return () => {
      active = false;
      cancel?.();
    };
  }, [streamKey]);

  return (
    <section
      className="feed-section news-widget"
      aria-label="Crypto news and news signals"
    >
      <FeedToolbar title="Crypto news & signals" />
      <div className="news-widget-status">
        <span>{snapshot?.source ?? 'CoinDesk RSS'} · backend subscription</span>
        <span className={snapshot?.stale ? 'news-stale' : 'news-live'}>
          {snapshot?.stale ? 'STALE' : snapshot ? 'LIVE' : 'CONNECTING'}
        </span>
      </div>
      {error && (
        <p className="news-widget-error" role="status">
          {error}
        </p>
      )}
      {!snapshot?.articles.length ? (
        <p className="positions-empty">Waiting for the backend RSS update…</p>
      ) : (
        <div className="news-widget-list">
          {snapshot.articles.map((article) => (
            <article className="news-widget-item" key={article.url}>
              <div className="news-widget-headline">
                <a href={article.url} target="_blank" rel="noreferrer">
                  {article.title}
                </a>
                <time>
                  {article.published_at ||
                    new Date(article.received_at_ms).toLocaleTimeString()}
                </time>
              </div>
              {article.article_text && (
                <details className="news-article-context">
                  <summary>RSS article context</summary>
                  <p>{article.article_text}</p>
                </details>
              )}
              <div className="news-widget-signal">
                <span className="news-asset">{article.matched_asset}</span>
                <span
                  className={'news-signal news-signal-' + article.direction}
                >
                  {article.direction.toUpperCase()} WATCH ·{' '}
                  {article.score > 0 ? '+' : ''}
                  {article.score.toFixed(2)}
                </span>
                <span>
                  {article.signal_reason}
                  {article.sentiment_confidence > 0 &&
                    !article.signal_reason
                      .toLowerCase()
                      .includes('confidence') &&
                    ' · text confidence ' +
                      article.sentiment_confidence.toFixed(0) +
                      '%'}
                </span>
              </div>
            </article>
          ))}
        </div>
      )}
      <small className="news-widget-note">
        Local crypto FinBERT uses RSS headline and description sentiment in the
        combined market score, with an adaptive weight capped at 20%. Keyword
        rules are used if the model is unavailable.
      </small>
    </section>
  );
}
