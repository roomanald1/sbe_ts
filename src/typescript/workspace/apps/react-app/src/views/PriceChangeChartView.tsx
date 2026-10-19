import { formatForSymbol } from '../instrument_precision';
import { useEffect, useId, useRef } from 'react';
import { FeedToolbar } from './FeedToolbar';
import type { PriceChartData } from '../price_change_data';

type PriceChangeChartViewProps = {
  data: PriceChartData;
  onSymbolChange: (symbol: string) => void;
  showSymbolSelector?: boolean;
  chartSize: { width: number; height: number };
  onResize: (width: number, height: number) => void;
  onTrade?: (side: 'buy' | 'sell') => void;
};

const percentFormatter = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
  signDisplay: 'always',
});

export function PriceChangeChartView({
  data,
  onSymbolChange,
  showSymbolSelector = true,
  chartSize,
  onResize,
  onTrade,
}: PriceChangeChartViewProps) {
  const {
    symbols,
    symbol,
    history,
    latestPrice,
    priceChange,
    percentChange,
    minPrice,
    maxPrice,
    startTime,
    endTime,
  } = data;
  const sortedSymbols = [...symbols].sort((left, right) =>
    left.localeCompare(right),
  );
  const chartRef = useRef<HTMLDivElement>(null);
  const clipId = 'price-chart-clip-' + useId().replace(/:/g, '');
  useEffect(() => {
    const container = chartRef.current;
    if (!container) return;

    const resizeObserver = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      if (width > 0 && height > 0) {
        onResize(width, height);
      }
    });
    resizeObserver.observe(container);
    return () => resizeObserver.disconnect();
  }, [onResize]);

  const chartWidth = Math.max(1, chartSize.width);
  const chartHeight = Math.max(1, chartSize.height);
  const axisFontSize = Math.max(7, Math.min(11, chartWidth * 0.035));
  const padding = {
    top: Math.min(24, chartHeight * 0.08),
    right: Math.min(18, chartWidth * 0.05),
    bottom: Math.min(28, chartHeight * 0.16),
    left: Math.min(chartWidth * 0.32, Math.max(42, chartWidth * 0.22)),
  };
  const plotWidth = Math.max(1, chartWidth - padding.left - padding.right);
  const plotHeight = Math.max(1, chartHeight - padding.top - padding.bottom);
  const priceRange =
    minPrice !== undefined && maxPrice !== undefined
      ? maxPrice - minPrice || Math.max(Math.abs(maxPrice) * 0.0001, 0.00000001)
      : 1;
  const paddedMin = (minPrice ?? 0) - priceRange * 0.08;
  const paddedMax = (maxPrice ?? 1) + priceRange * 0.08;
  const sampleStep = Math.max(
    1,
    Math.ceil(history.length / Math.max(1, plotWidth * 2)),
  );
  const plottedHistory = history.filter(
    (_, index) => index % sampleStep === 0 || index === history.length - 1,
  );
  const path = plottedHistory
    .map((sample, index) => {
      const x =
        padding.left +
        (plottedHistory.length <= 1
          ? plotWidth
          : (index / (plottedHistory.length - 1)) * plotWidth);
      const y =
        padding.top +
        (1 - (sample.price - paddedMin) / (paddedMax - paddedMin)) * plotHeight;
      return (index === 0 ? 'M ' : 'L ') + x.toFixed(2) + ' ' + y.toFixed(2);
    })
    .join(' ');
  const isPositive = (priceChange ?? 0) >= 0;
  const changeClass = isPositive
    ? 'price-change-positive'
    : 'price-change-negative';
  const ready =
    history.length > 0 &&
    latestPrice !== undefined &&
    priceChange !== undefined;

  return (
    <section
      className="feed-section price-chart-section"
      aria-label="Price change chart"
    >
      <FeedToolbar
        title={symbol ? 'Price history · ' + symbol : 'Price history'}
      >
        {showSymbolSelector && (
          <label className="chart-symbol-control">
            <span>SYMBOL</span>
            <select
              aria-label="Chart symbol"
              value={symbol}
              onChange={(event) => onSymbolChange(event.target.value)}
            >
              {sortedSymbols.map((item) => (
                <option key={item} value={item}>
                  {item}
                </option>
              ))}
            </select>
          </label>
        )}
      </FeedToolbar>
      {!ready ? (
        <div className="book-popup-message">
          <span className="book-loading-dot" />
          Waiting for price samples…
        </div>
      ) : (
        <>
          <div className="price-chart-summary">
            <div>
              <span className="price-chart-label">LAST PRICE</span>
              <strong>{formatForSymbol(latestPrice, symbol, 'price')}</strong>
            </div>
            <div className={changeClass}>
              <span className="price-chart-label">CHANGE</span>
              <strong>{formatForSymbol(priceChange, symbol, 'price')}</strong>
              <small>{percentFormatter.format(percentChange ?? 0)}%</small>
            </div>
            {onTrade && (
              <>
                <button
                  className="book-trade-button is-buy"
                  type="button"
                  onClick={() => onTrade('buy')}
                >
                  Buy
                </button>
                <button
                  className="book-trade-button is-sell"
                  type="button"
                  onClick={() => onTrade('sell')}
                >
                  Sell
                </button>
              </>
            )}
          </div>
          <div className="price-chart-wrap" ref={chartRef}>
            <svg
              className="price-chart"
              viewBox={'0 0 ' + chartWidth + ' ' + chartHeight}
              role="img"
              aria-label={`${symbol} price chart since connection`}
              preserveAspectRatio="xMidYMid meet"
            >
              <defs>
                <clipPath id={clipId}>
                  <rect
                    x={padding.left}
                    y={padding.top}
                    width={plotWidth}
                    height={plotHeight}
                  />
                </clipPath>
              </defs>
              {Array.from({ length: 5 }, (_, index) => {
                const y = padding.top + (index / 4) * plotHeight;
                const price = paddedMax - (index / 4) * (paddedMax - paddedMin);
                return (
                  <g key={index}>
                    <line
                      className="price-chart-gridline"
                      x1={padding.left}
                      x2={chartWidth - padding.right}
                      y1={y}
                      y2={y}
                    />
                    <text
                      className="price-chart-axis-label"
                      x={padding.left - 7}
                      y={y + 3}
                      style={{ fontSize: axisFontSize }}
                    >
                      {formatForSymbol(price, symbol, 'price')}
                    </text>
                  </g>
                );
              })}
              <path
                className={`price-chart-line ${changeClass}`}
                d={path}
                clipPath={'url(#' + clipId + ')'}
              />
            </svg>
          </div>
          <div className="price-chart-times">
            <time>
              {startTime === undefined
                ? ''
                : new Date(startTime).toLocaleTimeString()}
            </time>
            <span>Last-traded price · sampled once per second</span>
            <time>
              {endTime === undefined
                ? ''
                : new Date(endTime).toLocaleTimeString()}
            </time>
          </div>
        </>
      )}
    </section>
  );
}
