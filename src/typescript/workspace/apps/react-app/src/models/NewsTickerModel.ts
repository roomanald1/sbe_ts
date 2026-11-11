export type NewsArticle = {
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

export type NewsSnapshot = {
  updated_at_ms: number;
  source: string;
  stale: boolean;
  error?: string | null;
  articles: NewsArticle[];
};

export function decodeNewsSnapshot(raw: Uint8Array): NewsSnapshot {
  const value: unknown = JSON.parse(new TextDecoder().decode(raw));
  if (
    typeof value !== 'object' ||
    value === null ||
    !('updated_at_ms' in value) ||
    typeof value.updated_at_ms !== 'number' ||
    !('source' in value) ||
    typeof value.source !== 'string' ||
    !('stale' in value) ||
    typeof value.stale !== 'boolean' ||
    !('articles' in value) ||
    !Array.isArray(value.articles)
  ) {
    throw new Error('Invalid news snapshot');
  }
  return value as NewsSnapshot;
}
