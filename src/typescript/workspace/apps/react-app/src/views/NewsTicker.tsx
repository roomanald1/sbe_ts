import { useEffect, useState } from 'react';
import type { Connection } from '../connection';
import { FeedToolbar } from './FeedToolbar';

type NewsArticle = {
  title: string;
  article_text: string;
  url: string;
  published_at: string;
  received_at_ms: number;
  matched_asset: string;
  direction: 'bullish' | 'bearish' | 'neutral';
  score: number;
  signal_reason: string;
  sentiment_model: string;
  sentiment_confidence: number;
};
type NewsSnapshot = {
  updated_at_ms: number;
  source: string;
  stale: boolean;
  error?: string | null;
  articles: NewsArticle[];
};
type Props = { connection: Connection; streamKey: string };

export function NewsTicker({ connection, streamKey }: Props) {
  const [snapshot, setSnapshot] = useState<NewsSnapshot>();
  const [error, setError] = useState<string>();
  useEffect(() => {
    const feed = connection.openNews(streamKey);
    const dataSub = feed.data.subscribe((raw) => {
      if (raw === undefined) return;
      try {
        const text = typeof raw === 'string' ? raw : new TextDecoder().decode(raw as Uint8Array);
        const next = JSON.parse(text) as NewsSnapshot;
        setSnapshot(next);
        setError(next.error ?? undefined);
      } catch {
        setError('Received an invalid backend news update.');
      }
    });
    const errorSub = feed.error.subscribe((message) => setError(message));
    return () => {
      dataSub.unsubscribe();
      errorSub.unsubscribe();
      connection.closeNews(streamKey, feed);
    };
  }, [connection, streamKey]);

  return <section className="feed-section news-widget" aria-label="Crypto news and news signals">
    <FeedToolbar title="Crypto news & signals" />
    <div className="news-widget-status">
      <span>{snapshot?.source ?? 'CoinDesk RSS'} · backend subscription</span>
      <span className={snapshot?.stale ? 'news-stale' : 'news-live'}>{snapshot?.stale ? 'STALE' : snapshot ? 'LIVE' : 'CONNECTING'}</span>
    </div>
    {error && <p className="news-widget-error" role="status">{error}</p>}
    {!snapshot?.articles.length ? <p className="positions-empty">Waiting for the backend RSS update…</p> : <div className="news-widget-list">
      {snapshot.articles.map((article) => <article className="news-widget-item" key={article.url}>
        <div className="news-widget-headline">
          <a href={article.url} target="_blank" rel="noreferrer">{article.title}</a>
          <time>{article.published_at || new Date(article.received_at_ms).toLocaleTimeString()}</time>
        </div>
        {article.article_text && <details className="news-article-context"><summary>RSS article context</summary><p>{article.article_text}</p></details>}
        <div className="news-widget-signal">
          <span className="news-asset">{article.matched_asset}</span>
          <span className={'news-signal news-signal-' + article.direction}>{article.direction.toUpperCase()} WATCH · {article.score > 0 ? '+' : ''}{article.score.toFixed(2)}</span>
          <span>{article.signal_reason}{article.sentiment_confidence > 0 && !article.signal_reason.toLowerCase().includes('confidence') && ' · text confidence ' + article.sentiment_confidence.toFixed(0) + '%'}</span>
        </div>
      </article>)}
    </div>}
    <small className="news-widget-note">Local crypto FinBERT uses RSS headline and description sentiment in the combined market score, with an adaptive weight capped at 20%. Keyword rules are used if the model is unavailable.</small>
  </section>;
}
