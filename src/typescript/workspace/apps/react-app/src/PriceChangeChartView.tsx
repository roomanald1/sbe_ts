import { useEffect, useRef, useState } from 'react';
import type { PriceChartData } from './price_change_data';

type PriceChangeChartViewProps = {
    data: PriceChartData;
    onSymbolChange: (symbol: string) => void;
    onSelectSymbol?: (symbol: string) => void;
};

const CHART_HEIGHT = 340;
const CHART_PADDING = { top: 24, right: 24, bottom: 38, left: 94 };
const priceFormatter = new Intl.NumberFormat(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 10,
});
const percentFormatter = new Intl.NumberFormat(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
    signDisplay: 'always',
});

export function PriceChangeChartView({
    data,
    onSymbolChange,
    onSelectSymbol,
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
    const chartRef = useRef<HTMLDivElement>(null);
    const [chartWidth, setChartWidth] = useState(900);
    useEffect(() => {
        const container = chartRef.current;
        if (!container) return;

        const resizeObserver = new ResizeObserver(([entry]) => {
            const { width, height } = entry.contentRect;
            if (height > 0) {
                setChartWidth(CHART_HEIGHT * width / height);
            }
        });
        resizeObserver.observe(container);
        return () => resizeObserver.disconnect();
    }, []);

    const plotWidth = chartWidth - CHART_PADDING.left - CHART_PADDING.right;
    const plotHeight = CHART_HEIGHT - CHART_PADDING.top - CHART_PADDING.bottom;
    const priceRange = minPrice !== undefined && maxPrice !== undefined
        ? maxPrice - minPrice || Math.max(Math.abs(maxPrice) * 0.0001, 0.00000001)
        : 1;
    const paddedMin = (minPrice ?? 0) - priceRange * 0.08;
    const paddedMax = (maxPrice ?? 1) + priceRange * 0.08;
    const path = history.map((sample, index) => {
        const x = CHART_PADDING.left + (history.length <= 1 ? plotWidth : index / (history.length - 1) * plotWidth);
        const y = CHART_PADDING.top + (1 - (sample.price - paddedMin) / (paddedMax - paddedMin)) * plotHeight;
        return `${index === 0 ? 'M' : 'L'} ${x.toFixed(2)} ${y.toFixed(2)}`;
    }).join(' ');
    const isPositive = (priceChange ?? 0) >= 0;
    const changeClass = isPositive ? 'price-change-positive' : 'price-change-negative';
    const ready = history.length > 0 && latestPrice !== undefined && priceChange !== undefined;

    return (
        <section className="feed-section price-chart-section" aria-label="Price change chart">
            <div className="feed-toolbar">
                <div>
                    <span className="live-indicator" />
                    <span className="toolbar-title">Price history</span>
                </div>
                <label className="chart-symbol-control">
                    <span>SYMBOL</span>
                    <select
                        aria-label="Chart symbol"
                        value={symbol}
                        onChange={event => onSymbolChange(event.target.value)}
                    >
                        {symbols.map(item => <option key={item} value={item}>{item}</option>)}
                    </select>
                </label>
            </div>
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
                            <strong>{priceFormatter.format(latestPrice)}</strong>
                        </div>
                        <div className={changeClass}>
                            <span className="price-chart-label">CHANGE</span>
                            <strong>{priceFormatter.format(priceChange)}</strong>
                            <small>{percentFormatter.format(percentChange ?? 0)}%</small>
                        </div>
                        {onSelectSymbol && (
                            <button
                                className="chart-book-button"
                                type="button"
                                onClick={() => onSelectSymbol(symbol)}
                            >
                                View book
                            </button>
                        )}
                    </div>
                    <div className="price-chart-wrap" ref={chartRef}>
                        <svg
                            className="price-chart"
                            viewBox={`0 0 ${chartWidth} ${CHART_HEIGHT}`}
                            role="img"
                            aria-label={`${symbol} price chart since connection`}
                            preserveAspectRatio="none"
                        >
                            <defs>
                                <clipPath id="price-chart-plot-clip">
                                    <rect
                                        x={CHART_PADDING.left}
                                        y={CHART_PADDING.top}
                                        width={plotWidth}
                                        height={plotHeight}
                                    />
                                </clipPath>
                            </defs>
                            {Array.from({ length: 5 }, (_, index) => {
                                const y = CHART_PADDING.top + index / 4 * plotHeight;
                                const price = paddedMax - index / 4 * (paddedMax - paddedMin);
                                return (
                                    <g key={index}>
                                        <line
                                            className="price-chart-gridline"
                                            x1={CHART_PADDING.left}
                                            x2={chartWidth - CHART_PADDING.right}
                                            y1={y}
                                            y2={y}
                                        />
                                        <text className="price-chart-axis-label" x={CHART_PADDING.left - 12} y={y + 4}>
                                            {priceFormatter.format(price)}
                                        </text>
                                    </g>
                                );
                            })}
                            <path
                                className={`price-chart-line ${changeClass}`}
                                d={path}
                                clipPath="url(#price-chart-plot-clip)"
                            />
                        </svg>
                    </div>
                    <div className="price-chart-times">
                        <time>{startTime === undefined ? '' : new Date(startTime).toLocaleTimeString()}</time>
                        <span>Last-traded price · sampled once per second</span>
                        <time>{endTime === undefined ? '' : new Date(endTime).toLocaleTimeString()}</time>
                    </div>
                </>
            )}
        </section>
    );
}
