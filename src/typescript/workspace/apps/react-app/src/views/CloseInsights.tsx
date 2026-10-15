import { useEffect, useMemo } from 'react';
import { FeedToolbar } from './FeedToolbar';
import { CloseInsightsModel } from '../models/CloseInsightsModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';

const money = new Intl.NumberFormat(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 8 });
const quantity = new Intl.NumberFormat(undefined, { minimumFractionDigits: 0, maximumFractionDigits: 8 });

export function CloseInsights() {
  const model = useMemo(() => new CloseInsightsModel(), []);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  const state = useBehaviorSnapshot(model.state$);

  return (
    <section className="feed-section close-insights-panel" aria-label="Close-out insights">
      <FeedToolbar title="Close-out insights" />
      <p className="close-insights-note">
        Exit estimates use current visible book depth and exclude fees. The five-minute probability is an experimental estimate from recent drift and volatility, not a calibrated forecast.
      </p>
      {state.error && <div className="positions-error" role="alert">{state.error}</div>}
      {state.message && <div className="close-insights-message" role="status">{state.message}</div>}
      <div className="pnl-insight-grid">
        {state.insights.map((insight) => {
          const symbol = insight.position.symbol;
          const currency = symbol.split('-')[1] ?? '';
          const unrealized = insight.closePnl;
          const profitable = (unrealized ?? 0) > 0;
          return (
            <article className="pnl-insight-card" key={symbol}>
              <span>{symbol} · {insight.closeSide.toUpperCase()} {quantity.format(insight.closeQuantity)}</span>
              <strong className={profitable ? 'pnl-positive' : unrealized === undefined ? '' : 'pnl-negative'}>
                {unrealized === undefined
                  ? 'Executable P&L unavailable'
                  : (profitable ? 'Currently profitable · ' : 'Currently below entry · ') + money.format(unrealized) + ' ' + currency}
              </strong>
              <small>{insight.closePrice === undefined
                ? insight.depthCoversPosition ? 'Waiting for live book' : 'Visible depth cannot cover full position'
                : 'Estimated average close ' + money.format(insight.closePrice) + ' ' + currency}</small>
              <small>{insight.profitableCloseProbability5m === undefined
                ? 'Five-minute profitable probability: collecting market samples (' + insight.probabilitySamples + '/31)'
                : 'Experimental five-minute profitable probability: ' + (insight.profitableCloseProbability5m * 100).toFixed(1) + '%'}</small>
              <small>{insight.closePrice === undefined
                ? 'Recheck liquidity or reduce the exit size.'
                : profitable
                  ? 'Estimate excludes fees and slippage beyond visible depth; actual fills may differ.'
                  : 'Closing realizes a loss; compare with your recovery target and risk limit.'}</small>
              {insight.closePnl !== undefined && insight.closePnl > 0 && (
                <button
                  className="pnl-close-button"
                  type="button"
                  disabled={state.closingSymbol !== undefined}
                  onClick={() => void model.closePosition(insight)}
                >
                  {state.closingSymbol === symbol ? 'Closing…' : 'Close at IOC'}
                </button>
              )}
            </article>
          );
        })}
        {!state.loading && state.insights.length === 0 && (
          <p className="positions-empty">No open positions to analyze.</p>
        )}
        {state.loading && <p className="positions-empty">Loading close-out insights…</p>}
      </div>
    </section>
  );
}
