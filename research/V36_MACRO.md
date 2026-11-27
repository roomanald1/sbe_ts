# Trading Strategy Improvement Plan v36

## Macro, Market Regime & Cross-Asset Intelligence

**Status:** Planned — post-v35 implementation  
**Dependency:** Completed and accepted v35 trading infrastructure  
**Primary objective:** Sustainable, risk-adjusted net profitability

---

# 1. Programme Objectives

Extend the trading system to understand the broader economic, technical and cryptocurrency market environment in which trading decisions occur.

The system must progress beyond evaluating individual cryptocurrency signals in isolation.

It should understand:

- Macroeconomic announcements.
- Monetary policy.
- Economic surprises.
- Cross-asset market movements.
- Technical market regimes.
- Long-term trend transitions.
- Cryptocurrency market breadth.
- Relative strength and capital rotation.
- Derivatives positioning.
- Funding and open interest.
- Liquidation activity.
- Volatility regimes.
- On-chain valuation.
- Bitcoin market cycles.
- Institutional investment flows.
- Stablecoin liquidity.
- Market sentiment.
- Correlated portfolio exposure.

The system should autonomously learn:

- Which indicators provide genuine predictive value.
- Which indicators improve existing strategies.
- Which market conditions favour particular strategies.
- Which market conditions invalidate existing signals.
- When trading should be reduced, delayed or avoided.
- Which indicators improve entry and exit timing.
- How execution should adapt to changing market conditions.
- Which indicators justify dedicated trading strategies.
- How capital should be allocated across competing opportunities.
- When previously validated relationships have deteriorated.

The objective is not to maximise the number of indicators or trading strategies.

The objective is to improve net profitability, capital efficiency, execution quality and risk-adjusted returns.

---

# 2. Dependencies and Scope

v36 follows completion of v35.

Do not divert current v35 implementation resources into this programme.

Low-risk passive data collection may be proposed separately where delaying collection would result in irrecoverable historical evidence loss.

## 2.1 Reuse existing architecture

Reuse:

- Feed ingestion.
- Insights.
- Shared feature generation.
- Strategy evaluation.
- Scout evidence acquisition.
- Learning.
- Profile validation and promotion.
- AutoTrader.
- Autonomous capital allocation.
- Position Management.
- Paper Exchange.
- Execution quality analysis.
- Financial reconciliation.
- Historical replay.
- Excursion analysis.
- What-if simulation.
- RSocket infrastructure.
- Dashboard widgets.
- Shared ui-platform components.
- Grafana monitoring.

Do not introduce:

- A separate macroeconomic execution engine.
- A separate technical indicator trading engine.
- A parallel Learning system.
- A separate financial ledger.
- A separate capital allocator.
- Duplicated market-data pipelines.
- Independent risk controls.
- Unnecessary additional services.

## 2.2 Existing capability audit

Before implementation, inventory existing indicators, features, feeds, strategies, research capabilities and UI components.

For each proposed capability classify it as:

- Already implemented and validated.
- Implemented but requiring improvement.
- Available from existing data but not calculated.
- Requiring additional historical evidence.
- Requiring a new data source.
- Unsupported.
- Deferred.

Do not reimplement indicators already available through existing services.

Consolidate redundant implementations where appropriate.

---

# 3. Shared Market Intelligence Architecture

Establish a common market intelligence framework covering seven domains.

## Domain A — Macroeconomic intelligence

- Economic calendars.
- Economic releases.
- Consensus forecasts.
- Economic surprises.
- Monetary policy.
- Economic regimes.

## Domain B — Cross-asset intelligence

- Treasury yields.
- Dollar strength.
- Equity futures.
- Gold.
- Interest rate expectations.
- Traditional market volatility.

## Domain C — Technical market intelligence

- Trend.
- Moving-average relationships.
- Momentum.
- Breakouts.
- Reversals.
- Volatility.
- Volume.
- Market structure.

## Domain D — Cryptocurrency market intelligence

- Market breadth.
- Bitcoin dominance.
- Relative strength.
- Altcoin rotation.
- Correlation.
- Return dispersion.

## Domain E — Derivatives and positioning intelligence

- Funding.
- Open interest.
- Liquidations.
- Futures basis.
- Options positioning.
- Spot versus derivatives divergence.

## Domain F — On-chain and cycle intelligence

- Realised valuation.
- Holder profitability.
- Miner activity.
- Network activity.
- Bitcoin supply cycles.
- Exchange flows.

## Domain G — Capital flows and sentiment

- ETF flows.
- Stablecoin supply.
- Stablecoin flows.
- Institutional demand proxies.
- Fear and greed.
- Aggregate positioning sentiment.

All domains should use consistent feature contracts, provenance, freshness, evidence quality and Learning integration.

## 3.1 Separate information availability

Distinguish:

- Information known in advance.
- Information newly published.
- Information first observed by the system.
- Derived features.
- Subsequent observed market reactions.
- Estimated historical information.

Historical replay must respect when information actually became available.

## 3.2 Shared feature contract

Each feature should expose, where applicable:

- Feature identifier.
- Feature category.
- Value.
- Units.
- Scope.
- Symbol or asset group.
- Market-data venue.
- Calculation horizon.
- Observation timestamp.
- First-observed timestamp.
- Feature availability timestamp.
- Source.
- Source version.
- Calculation version.
- Freshness.
- Availability.
- Evidence quality.
- Confidence where statistically supported.
- Related event identity.
- Related market regime.

Missing, stale, estimated and directly observed values must remain distinguishable.

---

# WP0 — Existing Capability Audit and Data Feasibility

**Priority: Critical**

## 0.1 Repository capability audit

Inventory existing:

- Technical indicators.
- Moving averages.
- Momentum features.
- Market structure features.
- Funding and basis feeds.
- Open interest feeds.
- Liquidation feeds.
- Market breadth calculations.
- Cross-symbol features.
- Historical data.
- Scout capabilities.
- Strategy profiles.
- UI components.

Identify reusable implementations and authoritative data sources.

## 0.2 Official macroeconomic sources

Investigate:

- BLS.
- BEA.
- Federal Reserve.
- FRED.
- ALFRED.
- ECB.
- Bank of England.
- Other relevant official statistical agencies.

## 0.3 Economic calendar providers

Investigate available providers for:

- Scheduled announcements.
- Historical releases.
- Consensus forecasts.
- Forecast revision history.
- Publication timestamps.
- Historical economic surprises.
- Monetary policy expectations.

Potential providers include Trading Economics and other appropriately licensed services.

Do not assume official sources provide consensus forecasts.

## 0.4 Cryptocurrency data sources

Investigate:

- Existing exchange APIs.
- Historical trade archives.
- Order-book archives.
- Derivatives market data.
- Open interest.
- Funding.
- Liquidation streams.
- Options market data.
- Market capitalisation.
- Historical asset universes.

## 0.5 On-chain and flow data sources

Investigate:

- Bitcoin blockchain data.
- Public blockchain analytics.
- Coin Metrics.
- Glassnode.
- CryptoQuant.
- DefiLlama.
- ETF issuer disclosures.
- Stablecoin issuer disclosures.
- Other suitable licensed sources.

These are candidate sources, not approved integrations.

## 0.6 Cross-asset data sources

Investigate:

- Dollar indices and proxies.
- Treasury yields and futures.
- Equity index futures.
- Gold.
- Volatility indices.
- Interest rate futures.
- Market-implied monetary policy expectations.

## 0.7 Source capability matrix

Document:

- Available data.
- Historical coverage.
- Resolution.
- Timestamp accuracy.
- Publication latency.
- First-observed timestamp availability.
- Revision handling.
- Source methodology.
- API reliability.
- Rate limits.
- Licensing.
- Storage restrictions.
- Redistribution restrictions.
- Cost.
- Operational dependencies.
- Supported research capabilities.

## 0.8 Feasibility decisions

Prioritise indicators calculable from existing market data.

Prefer official and free sources where practical.

Do not purchase subscriptions without operator approval.

Do not build speculative integrations for indicators whose historical evidence is unavailable or whose research value cannot reasonably be assessed.

## Acceptance

- Existing capability inventory.
- Data-source capability matrix.
- Reuse and consolidation decisions.
- Initial source selection.
- Explicit historical coverage limitations.
- Estimated costs.
- Prioritised research backlog.

---

# WP1 — Economic Calendar and Announcement Ingestion

**Priority: Critical**

## 1.1 Tier 1 economic events

- Nonfarm Payrolls (NFP).
- Consumer Price Index (CPI).
- Core CPI.
- Personal Consumption Expenditures (PCE).
- Core PCE.
- FOMC interest rate decisions.
- FOMC policy statements.
- Federal Reserve press conferences.

## 1.2 Tier 2 economic events

- Unemployment rate.
- Average hourly earnings.
- Producer Price Index.
- GDP.
- Retail sales.
- ISM manufacturing.
- ISM services.
- Initial jobless claims.
- JOLTS.
- FOMC minutes.
- Federal Reserve economic projections.
- Significant Federal Reserve speeches.

## 1.3 Tier 3 economic events

- ECB policy decisions.
- Bank of England policy decisions.
- Bank of Japan policy decisions.
- Major Chinese economic announcements.
- Other internationally significant economic releases.

Initial importance classifications are research defaults.

Learning should establish actual relevance.

## 1.4 Event data contract

Capture:

- Stable event identifier.
- Announcement-group identifier.
- Event type.
- Category.
- Country.
- Currency.
- Reporting period.
- Original scheduled timestamp.
- Revised scheduled timestamp.
- Official publication timestamp.
- Source publication timestamp.
- First system-observed timestamp.
- Ingestion timestamp.
- Feature availability timestamp.
- Previous published value.
- Revised previous value.
- Consensus forecast.
- Consensus publication timestamp.
- Actual published value.
- Units.
- Source.
- Source authority.
- Source version.
- Freshness.
- Publication history.
- Revision history.
- Importance.
- Status.
- Related announcement components.

Store timestamps in UTC while retaining source timezone information.

Handle daylight-saving changes, holidays and rescheduled releases.

## 1.5 Announcement lifecycle

Support:

- Scheduled.
- Rescheduled.
- Approaching.
- Awaiting release.
- Published.
- Partially received.
- Delayed.
- Cancelled.
- Corrected.
- Revised.
- Conflicting.
- Unavailable.

Do not infer publication merely because the scheduled release time has passed.

## 1.6 Announcement grouping

Simultaneous releases must share a common market-shock identity.

Examples:

- NFP, unemployment and wage growth.
- Headline and core CPI.
- Headline and core PCE.

Preserve individual observations and their availability timestamps.

Do not count simultaneous components as independent announcement episodes.

Prevent duplicate trading triggers from one common market shock.

## 1.7 FOMC and non-numeric announcements

Represent related phases:

- Rate decision.
- Policy statement.
- Economic projections.
- Dot plot.
- Press conference.
- Subsequent clarification.

Capture:

- Actual rate change.
- Market-implied expected change where available.
- Rate surprise.
- Projected rate-path changes.
- Economic projection changes.
- Versioned policy documents.
- Observed market reactions.

Do not require LLM-based hawkish/dovish scoring.

Initially prioritise structured information and observable market reactions.

## 1.8 Unscheduled announcements

Support emergency policy decisions and unexpected announcements.

Do not retrospectively assign pre-event awareness to information that was not known in advance.

## Acceptance

Reliable, restart-safe, versioned and point-in-time-correct announcement ingestion with explicit provenance and announcement grouping.

---

# WP2 — Historical Evidence and Point-in-Time Integrity

**Priority: Critical**

## 2.1 Historical acquisition

Acquire multiple years of reliable historical evidence where available.

Include:

- Original economic releases.
- Forecasts and forecast revisions.
- Economic revisions.
- Historical cryptocurrency market data.
- Historical technical indicators.
- Historical derivatives positioning.
- Historical cross-asset data.
- Historical on-chain metrics.
- Historical institutional and stablecoin flows.
- Historical symbol universes.
- Historical execution costs.

## 2.2 Information availability

Maintain separate timestamps for:

- Official publication.
- Source publication.
- First observation.
- Ingestion.
- Feature availability.
- Strategy decision.

Historical replay must respect actual information availability.

Where historical first-observed timestamps are unavailable:

- Apply documented conservative latency assumptions.
- Perform sensitivity analysis.
- Label estimated availability explicitly.

Do not classify estimated availability as directly observed evidence.

## 2.3 Immutable historical observations

Preserve original observations and subsequent revisions.

Do not overwrite historical information states.

This applies to:

- Economic releases.
- Consensus forecasts.
- On-chain metrics.
- Market capitalisation.
- ETF flows.
- Stablecoin supply.
- Provider methodology changes.
- Corrected market observations.

## 2.4 Historical technical indicator correctness

Calculate indicators using only information available at the decision timestamp.

Prevent:

- Use of unfinished candle closes as completed candles.
- Future-confirmed swing highs or lows.
- Retrospectively identified divergences.
- Future-normalised indicator percentiles.
- Future regime classifications.
- Forward-shifted chart components being interpreted as future observations.

Where intrabar indicator values are used, explicitly distinguish provisional from confirmed values.

## 2.5 Survivorship bias

Preserve historical asset eligibility where supported.

Account for:

- Listings.
- Delistings.
- Exchange availability.
- Symbol migrations.
- Changing liquidity.
- Changing fees.
- Changing market capitalisation.
- Historical stablecoin classifications.

Do not construct historical breadth or relative-strength research solely from currently listed assets.

## 2.6 Evidence quality

Classify historical evidence by:

- Resolution.
- Completeness.
- Timestamp accuracy.
- Executable price availability.
- Liquidity coverage.
- Execution-cost coverage.
- Observation gaps.
- Source methodology.

Do not infer chronological executable paths from candle extrema.

## Acceptance

Reproducible historical datasets with explicit information availability, versioning, evidence quality and automated leakage-prevention tests.

---

# WP3 — Macroeconomic Feature Generation

**Priority: Critical**

## 3.1 Scheduled-event features

- Time until next announcement.
- Time since previous announcement.
- Event category.
- Event importance.
- Current event phase.
- Overlapping announcements.
- Announcement-group identity.
- Calendar freshness.
- Historical event volatility.
- Historical event liquidity impact.

## 3.2 Economic surprise features

Where reliable consensus exists:

- Actual minus consensus.
- Percentage surprise where meaningful.
- Standardised surprise.
- Surprise magnitude.
- Surprise direction.
- Historical surprise percentile.
- Previous-value revision.
- Forecast revision trend.
- Combined announcement surprise.
- Conflicting announcement components.

Normalise surprises independently by indicator using historical information available at the decision timestamp.

Do not hardcode economic surprise direction as bullish or bearish for crypto.

## 3.3 Market reaction features

- BTC event return.
- ETH event return.
- Individual cryptocurrency event return.
- Relative return versus BTC.
- Price acceleration.
- Realised volatility expansion.
- Spread expansion.
- Executable depth reduction.
- Order-book imbalance.
- Trade-flow imbalance.
- Volume expansion.
- Liquidation activity.
- Funding changes.
- Basis changes.
- Liquidity recovery.
- Directional continuation.
- Price reversal.

## 3.4 Economic regime features

Initially support:

- Inflation trend.
- Employment trend.
- Growth trend.
- Monetary policy direction.
- Recent surprise history.
- Crypto risk appetite.
- Crypto volatility regime.

Extend after cross-asset integration with:

- Dollar strength.
- Treasury yield direction.
- Equity market risk appetite.
- Market-implied rate expectations.
- Financial conditions.

## Acceptance

Shared, versioned and point-in-time-correct macroeconomic features available to existing trading and research services.

---

# WP4 — Cross-Asset Market Intelligence

**Priority: High**

## 4.1 Initial market coverage

- US Dollar Index or suitable proxies.
- US two-year Treasury yields.
- US ten-year Treasury yields.
- Treasury futures.
- Nasdaq futures.
- S&P 500 futures.
- Gold.
- VIX or suitable volatility proxies.
- Interest rate futures.
- Market-implied monetary policy expectations.

## 4.2 Features

- Price changes.
- Yield changes.
- Momentum.
- Volatility.
- Relative strength.
- BTC and ETH correlation.
- Lead-lag relationships.
- Cross-asset confirmation.
- Cross-asset divergence.
- Monetary policy repricing.
- Session status.
- Data freshness.

## 4.3 Market session awareness

Handle:

- Regular trading sessions.
- Overnight sessions.
- Futures maintenance.
- Weekends.
- Public holidays.
- Early closures.
- Market reopening.
- Illiquid periods.
- Delayed feeds.

Distinguish:

- Open and liquid.
- Open but illiquid.
- Closed.
- Reopening.
- Stale.
- Unavailable.

Do not interpret unchanged closed-market prices as neutral market reactions.

## 4.4 Research objectives

Investigate:

- Yield sensitivity.
- Dollar sensitivity.
- Equity market confirmation.
- Cross-asset divergence.
- Lead-lag opportunities.
- False-breakout reduction.
- Regime-dependent relationships.

Do not assume cross-asset relationships are constant.

## Acceptance

Reliable cross-asset observations and evidence of incremental predictive or risk-management value.

---

# WP5 — Technical Trend and Market Structure Intelligence

**Priority: Critical**

Prioritise indicators available from existing cryptocurrency market data.

## 5.1 Moving-average indicators

Support relevant combinations of:

- SMA.
- EMA.
- Moving-average ribbons.
- Moving-average slopes.
- Price-to-average distance.
- Average-to-average distance.
- Trend alignment.
- Trend acceleration.
- Trend deceleration.

Initial candidate periods:

- 10.
- 20.
- 21.
- 50.
- 100.
- 111.
- 200.
- 350.

Use appropriate periods for each timeframe.

Do not exhaustively optimise arbitrary moving-average combinations.

## 5.2 Golden Cross and Death Cross

Canonical definitions:

- Golden Cross: 50-day SMA crosses above 200-day SMA.
- Death Cross: 50-day SMA crosses below 200-day SMA.

Capture:

- Crossover direction.
- Crossover timestamp.
- Confirmation status.
- Time since crossover.
- Distance between averages.
- Slope of each average.
- Price distance from each average.
- Pre-crossover return.
- Post-crossover return.
- Trend strength.
- Volatility.
- Market breadth.
- Derivatives positioning.

Research both conventional and contrarian interpretations.

Do not automatically short after a death cross or buy after a golden cross.

Investigate whether substantial price movement has already occurred before confirmation.

## 5.3 Long-term trend indicators

Support research into:

- Price above or below 200-day SMA.
- 200-day SMA breakdown.
- 200-day SMA reclaim.
- 20-week SMA.
- 21-week EMA.
- Bull Market Support Band.
- 200-week moving average.
- Long-term trend slope.
- Monthly trend alignment.
- Mayer Multiple.

The Mayer Multiple is price divided by the 200-day SMA.

## 5.4 Pi Cycle Top

Investigate the relationship between:

- 111-day SMA.
- Twice the 350-day SMA.

Capture crossover, proximity, slopes and historical subsequent returns.

Treat the indicator as a research hypothesis with very limited independent historical cycle evidence.

Do not assume it reliably predicts cycle peaks.

## 5.5 Market structure

Support:

- Higher highs.
- Higher lows.
- Lower highs.
- Lower lows.
- Confirmed swing highs.
- Confirmed swing lows.
- Break of structure.
- Change of character.
- Range breakout.
- Range breakdown.
- Failed breakout.
- Failed breakdown.
- Support reclaim.
- Resistance rejection.
- Trend persistence.

Define each event objectively and reproducibly.

Record when a structure becomes observable, not when it is retrospectively located.

## 5.6 Multi-timeframe context

Support suitable horizons including:

- 1 minute.
- 5 minutes.
- 15 minutes.
- 1 hour.
- 4 hours.
- Daily.
- Weekly.
- Monthly.

Use existing supported horizons where appropriate.

Long-term trend indicators should primarily provide regime context for short-term strategies.

Do not assume a daily death cross directly predicts the next one-minute return.

## 5.7 Trend research

Evaluate:

- Trend continuation.
- Trend exhaustion.
- Trend reversal.
- Crossover lag.
- Whipsaw frequency.
- Breakout reliability.
- Mean-reversion opportunities.
- Interaction with market breadth.
- Interaction with derivatives positioning.
- Interaction with macroeconomic events.

## Acceptance

Reusable technical trend and market structure features with empirically evaluated relevance across strategy horizons.

---

# WP6 — Momentum, Volatility and Reversal Intelligence

**Priority: High**

## 6.1 Momentum indicators

Investigate:

- RSI.
- RSI bullish divergence.
- RSI bearish divergence.
- MACD.
- MACD signal crossover.
- MACD zero-line crossover.
- Rate of change.
- Stochastic oscillator.
- Momentum acceleration.
- Momentum deceleration.
- Relative momentum.
- Multi-timeframe momentum alignment.

## 6.2 Trend-strength indicators

Investigate:

- ADX.
- Positive and negative directional indicators.
- Supertrend.
- Ichimoku cloud.
- Trend persistence.
- Directional efficiency.
- Moving-average ribbon expansion.

## 6.3 Volatility indicators

Investigate:

- ATR.
- Normalised ATR.
- Realised volatility.
- Bollinger Bands.
- Bollinger Band width.
- Bollinger Band squeeze.
- Keltner Channels.
- Volatility compression.
- Volatility expansion.
- Volatility percentile.
- Volatility regime transition.
- Upside versus downside volatility.

## 6.4 Breakout indicators

Investigate:

- Donchian Channel breakout.
- Prior-day high and low.
- Prior-week high and low.
- Rolling range breakout.
- Opening-range breakout where relevant.
- Volatility-adjusted breakout.
- Volume-confirmed breakout.
- Failed breakout.

## 6.5 Volume and flow indicators

Investigate:

- Relative volume.
- Volume expansion.
- Volume climax.
- Volume exhaustion.
- On-balance volume.
- OBV divergence.
- Cumulative volume delta.
- Price and volume divergence.
- Trade-flow imbalance.
- VWAP deviation.
- Anchored VWAP where supported.

## 6.6 Indicator event representation

For each indicator distinguish:

- Current value.
- State.
- State transition.
- Magnitude.
- Duration.
- Time since transition.
- Confirmation status.
- Historical percentile.
- Calculation horizon.
- Evidence quality.

Avoid reducing continuous indicators to binary signals prematurely.

## 6.7 Redundancy control

Many technical indicators measure similar underlying phenomena.

Investigate:

- Feature correlation.
- Redundant information.
- Incremental predictive value.
- Feature stability.
- Conditional importance.

Do not treat RSI, MACD and multiple momentum indicators as independent confirmations without supporting evidence.

## Acceptance

A compact, reusable technical feature library with validated incremental value and controlled feature redundancy.

---

# WP7 — Cryptocurrency Market Breadth and Relative Strength

**Priority: Critical**

## 7.1 Market breadth

Calculate where supported:

- Percentage of eligible assets above 20-day SMA.
- Percentage above 50-day SMA.
- Percentage above 200-day SMA.
- Percentage in positive momentum regimes.
- Percentage in negative momentum regimes.
- Advance/decline ratio.
- Advance/decline line.
- New highs versus new lows.
- Positive versus negative return participation.
- Volume participation.
- Equal-weight market return.
- Capitalisation-weighted market return.

## 7.2 Universe definitions

Maintain explicit versioned universes:

- All supported eligible cryptocurrencies.
- Liquid cryptocurrencies.
- Large-cap cryptocurrencies.
- Exchange-specific tradable assets.
- BTC-excluded universe.
- ETH-excluded universe where relevant.
- Strategy-eligible universe.

Exclude stablecoins, leveraged tokens and duplicated wrapped exposures where inappropriate.

Do not silently change historical denominators.

## 7.3 Bitcoin dominance

Investigate:

- BTC dominance.
- Change in BTC dominance.
- ETH dominance.
- Stablecoin dominance.
- Dominance trend.
- Dominance acceleration.
- Dominance versus total market return.

Distinguish genuine market-cap dominance from exchange-specific or tradable-universe proxies.

## 7.4 Relative strength

Investigate:

- ETH/BTC.
- Altcoin/BTC relative returns.
- Altcoin/ETH relative returns.
- Sector relative strength where reliable classifications exist.
- Relative momentum rankings.
- Cross-sectional return dispersion.
- Relative strength persistence.
- Relative strength reversal.

## 7.5 Capital rotation

Research:

- Bitcoin-led rallies.
- Ethereum-led rallies.
- Broad altcoin rallies.
- Defensive rotation towards BTC.
- Stablecoin rotation.
- Narrow market leadership.
- Broad market participation.
- Altcoin season conditions.

Do not rely solely on externally published Altcoin Season Index values where a transparent internal measure is possible.

## 7.6 Correlation and dispersion

Calculate:

- Cross-symbol correlation.
- BTC beta.
- ETH beta.
- Common market-factor exposure.
- Return dispersion.
- Correlation expansion.
- Correlation breakdown.
- Concentration of market leadership.

Use dependence-aware estimates and sufficient observations.

## Acceptance

Reliable market-wide features that distinguish broad market trends, narrow leadership, rotation and systemic correlated exposure.

---

# WP8 — Derivatives Positioning and Liquidation Intelligence

**Priority: Critical**

## 8.1 Funding rates

Capture:

- Current funding rate.
- Predicted funding where available.
- Historical funding.
- Funding interval.
- Funding changes.
- Funding persistence.
- Funding percentile.
- Cross-exchange funding divergence.
- Cumulative funding costs.

Normalise different exchange funding intervals.

Research crowded positioning without assuming extreme funding automatically predicts reversal.

## 8.2 Open interest

Capture:

- Current open interest.
- Open interest change.
- Open interest acceleration.
- Open interest percentile.
- Open interest relative to market size.
- Open interest relative to volume.
- Cross-exchange open interest.
- Price and open interest relationships.

Distinguish:

- Contract-denominated open interest.
- Asset-denominated open interest.
- Quote-currency notional.
- Changes caused by price movements.

Do not infer long or short positioning directly from open interest alone.

## 8.3 Price and positioning combinations

Investigate:

- Price rising and open interest rising.
- Price rising and open interest falling.
- Price falling and open interest rising.
- Price falling and open interest falling.
- Funding and price divergence.
- Funding and open interest divergence.
- Spot and derivatives divergence.

Treat interpretations as hypotheses.

## 8.4 Liquidation activity

Capture where available:

- Long liquidations.
- Short liquidations.
- Liquidation notional.
- Liquidation intensity.
- Liquidation acceleration.
- Liquidation clusters.
- Cross-exchange liquidation activity.
- Liquidation imbalance.
- Liquidation relative to market volume.
- Subsequent liquidity recovery.

Explicitly distinguish observed liquidations from estimated liquidation levels or heatmaps.

Do not treat incomplete exchange liquidation streams as comprehensive market coverage.

## 8.5 Liquidation cascade research

Investigate:

- Forced selling continuation.
- Forced buying continuation.
- Long squeeze.
- Short squeeze.
- Deleveraging.
- Capitulation.
- Post-liquidation reversal.
- Post-liquidation momentum.
- Liquidity recovery.

Evaluate execution feasibility during stressed liquidity conditions.

## 8.6 Futures basis

Investigate:

- Perpetual premium.
- Dated futures basis.
- Annualised basis.
- Basis expansion.
- Basis compression.
- Contango.
- Backwardation.
- Cross-exchange basis divergence.

Reuse existing funding and carry capabilities.

Do not automatically activate previously deferred carry or hedging strategies.

## 8.7 Spot versus derivatives activity

Investigate:

- Spot CVD.
- Perpetual CVD.
- Spot volume.
- Derivatives volume.
- Spot versus derivatives volume ratio.
- Spot-led price movement.
- Derivatives-led price movement.
- Spot/perpetual divergence.
- Coinbase premium or suitable venue premium.

Account for quote-currency differences, venue coverage and timestamp alignment.

## 8.8 Options intelligence

Where reliable data is available, investigate:

- Implied volatility.
- Realised versus implied volatility.
- Volatility term structure.
- Put/call ratio.
- Options open interest.
- Options volume.
- Volatility skew.
- Expiry concentration.

Do not infer dealer positioning or gamma exposure without a defensible methodology and appropriate evidence.

## Acceptance

Validated positioning, leverage and liquidation features with explicit exchange coverage, units, provenance and execution limitations.

---

# WP9 — On-Chain Valuation and Bitcoin Cycle Intelligence

**Priority: Medium to High, dependent on data feasibility**

On-chain indicators primarily provide slower-moving market context.

Do not treat daily on-chain observations as high-frequency trading signals.

## 9.1 Valuation indicators

Investigate:

- Market Value to Realised Value (MVRV).
- MVRV Z-Score.
- Realised price.
- Short-term holder realised price.
- Long-term holder realised price.
- Net Unrealised Profit/Loss (NUPL).
- Reserve Risk.
- RHODL Ratio.

Preserve provider-specific definitions and methodologies.

Do not combine differently calculated metrics as though they were equivalent.

## 9.2 Holder profitability

Investigate:

- Spent Output Profit Ratio (SOPR).
- Adjusted SOPR.
- Short-term holder SOPR.
- Long-term holder SOPR.
- Supply in profit.
- Supply in loss.
- Realised profit.
- Realised loss.
- Holder accumulation.
- Holder distribution.

## 9.3 Miner intelligence

Investigate:

- Hashrate.
- Hashrate trend.
- Hash Ribbons.
- Miner revenue.
- Puell Multiple.
- Miner selling pressure where reliably observable.
- Mining difficulty.
- Difficulty adjustments.

Hash Ribbons research should include the relationship between 30-day and 60-day hashrate averages.

Treat miner capitulation and recovery interpretations as hypotheses.

## 9.4 Bitcoin supply cycles

Investigate:

- Time since Bitcoin halving.
- Time until expected halving.
- Issuance rate.
- Historical cycle phase.
- Long-term trend context.
- Realised valuation.
- Holder profitability.
- Miner economics.

Do not assume Bitcoin follows a fixed four-year price cycle.

## 9.5 Exchange flows

Investigate where reliable:

- BTC exchange inflows.
- BTC exchange outflows.
- Net exchange flows.
- Exchange balance changes.
- Large transfer activity.
- Long-term accumulation proxies.

Account for:

- Exchange wallet reshuffling.
- Custody migrations.
- Address-label revisions.
- Internal transfers.
- Provider methodology changes.

Do not automatically interpret exchange inflows as selling or outflows as buying.

## 9.6 Statistical limitations

Bitcoin has experienced relatively few independent long-term market cycles.

Thousands of daily observations do not constitute thousands of independent cycle observations.

Do not promote cycle-specific trading rules solely because they identified several historical market tops or bottoms.

Use these indicators primarily as contextual features until sufficient independent evidence exists.

## Acceptance

Reliable on-chain and cycle context with explicit methodology, observation latency and statistical limitations.

---

# WP10 — Institutional Flows, Stablecoins and Sentiment

**Priority: High, dependent on source availability**

## 10.1 ETF flows

Investigate:

- Bitcoin ETF net flows.
- Ethereum ETF net flows.
- Aggregate daily flows.
- Rolling cumulative flows.
- Flow acceleration.
- Flow persistence.
- Flow relative to market volume.
- Price versus flow divergence.
- Assets under management.

Distinguish:

- Net subscriptions and redemptions.
- Trading volume.
- Assets under management changes caused by price movements.
- Estimated versus confirmed flows.

Respect actual publication and revision timestamps.

Do not use end-of-day ETF flow information for decisions made earlier that day.

## 10.2 Stablecoin liquidity

Investigate:

- Aggregate stablecoin supply.
- Stablecoin supply growth.
- Stablecoin issuance.
- Stablecoin redemptions.
- Stablecoin exchange inflows.
- Stablecoin exchange outflows.
- Stablecoin dominance.
- Stablecoin liquidity relative to cryptocurrency market size.
- Stablecoin peg deviations.

Account for:

- Chain migrations.
- Bridged assets.
- Treasury inventory.
- Double-counting.
- Changes in provider coverage.

Do not assume all stablecoin issuance represents immediate cryptocurrency buying demand.

## 10.3 Sentiment indicators

Investigate:

- Fear & Greed Index.
- Internally calculated market sentiment.
- Funding-based positioning sentiment.
- Market breadth sentiment.
- Volatility-based sentiment.
- Liquidation-based sentiment.
- Options-based sentiment where supported.

Preserve methodology and publication timestamps.

## 10.4 Composite sentiment

Investigate whether transparent combinations of existing features improve predictive value.

Avoid double-counting the same underlying information.

For example, an external sentiment index may already include momentum, volatility and market dominance.

Do not treat it as an independent confirmation without testing.

## 10.5 Scope limitations

Defer:

- Social media scraping.
- Twitter/X sentiment.
- Reddit sentiment.
- General headline interpretation.
- LLM-based news sentiment.
- Automated geopolitical interpretation.

These are separate future research programmes.

## Acceptance

Source-backed capital-flow and sentiment features with realistic publication latency and demonstrated incremental research value.

---

# WP11 — Unified Market Regime Classification

**Priority: Critical**

Combine supported intelligence domains into a coherent market regime framework.

## 11.1 Multi-dimensional regimes

Avoid reducing the entire market to one bullish or bearish label.

Maintain distinct regime dimensions.

### Trend regime

- Bullish.
- Bearish.
- Range-bound.
- Transitional.
- Uncertain.

### Momentum regime

- Accelerating.
- Persistent.
- Weakening.
- Reversing.
- Neutral.

### Volatility regime

- Compressed.
- Normal.
- Expanding.
- Elevated.
- Extreme.

### Liquidity regime

- Normal.
- Improving.
- Deteriorating.
- Thin.
- Stressed.
- Recovering.

### Positioning regime

- Balanced.
- Crowded long.
- Crowded short.
- Leveraging.
- Deleveraging.
- Liquidation-driven.

### Breadth regime

- Broad bullish participation.
- Broad bearish participation.
- Narrow leadership.
- Divergent.
- Rotational.

### Macroeconomic regime

- Tightening.
- Easing.
- Stable policy.
- Inflation acceleration.
- Inflation deceleration.
- Growth acceleration.
- Growth deterioration.
- Event-driven uncertainty.

### Cycle regime

- Potential accumulation.
- Expansion.
- Potential distribution.
- Capitulation.
- Recovery.
- Uncertain.

Cycle labels must remain explicitly provisional unless empirically validated.

## 11.2 Multi-horizon classification

Support different simultaneous regimes across horizons.

For example:

- Long-term bearish trend.
- Short-term bullish momentum.
- Elevated volatility.
- Deleveraging.
- Improving market breadth.
- Approaching macroeconomic announcement.

Do not force these conditions into a single directional score.

## 11.3 Regime transitions

Capture:

- Previous regime.
- Current regime.
- Transition timestamp.
- Transition magnitude.
- Time in regime.
- Confidence.
- Supporting features.
- Conflicting features.
- Applicable horizon.

Investigate whether transitions provide more predictive value than static regime classifications.

## 11.4 Regime stability

Prevent excessive state oscillation.

Investigate:

- Hysteresis.
- Confirmation periods.
- Minimum persistence.
- Transition uncertainty.
- Adaptive thresholds.

Do not introduce arbitrary universal thresholds without research.

## 11.5 Regime scope

Support:

- Global cryptocurrency market.
- BTC.
- ETH.
- Asset groups.
- Individual symbols.
- Exchange-specific liquidity and positioning.

Avoid unnecessary duplication of global regime calculations.

## 11.6 Composite regime research

Compare:

- Simple transparent rules.
- Statistical classification.
- Hierarchical classification.
- Probabilistic regime models where justified.

Prefer simpler models where performance is comparable.

Do not use future-smoothed regime classifications in historical trading decisions.

## Acceptance

Shared, explainable, versioned and point-in-time-correct multi-dimensional market regimes with demonstrated relevance to trading outcomes.

---

# WP12 — Event Studies and Indicator Research Framework

**Priority: Critical**

Extend existing historical replay and excursion infrastructure.

## 12.1 Research event types

Support:

- Economic announcements.
- Golden crosses.
- Death crosses.
- Moving-average reclaims.
- Trend transitions.
- Momentum reversals.
- RSI divergences.
- Volatility squeezes.
- Breakouts.
- Funding extremes.
- Open interest changes.
- Liquidation cascades.
- Breadth transitions.
- Dominance changes.
- Capital rotation.
- ETF flow extremes.
- Stablecoin liquidity changes.
- On-chain threshold crossings.
- Market regime transitions.

## 12.2 Macroeconomic research windows

Before release:

- 24 hours.
- 4 hours.
- 1 hour.
- 30 minutes.
- 15 minutes.
- 5 minutes.
- 1 minute.

After release:

- 1 second.
- 5 seconds.
- 10 seconds.
- 30 seconds.
- 1 minute.
- 5 minutes.
- 10 minutes.
- 15 minutes.
- 30 minutes.
- 1 hour.
- 4 hours.
- 24 hours.

Use only windows supported by available evidence.

## 12.3 Technical and positioning research windows

Investigate horizons appropriate to the originating signal:

- Seconds.
- Minutes.
- Hours.
- Days.
- Weeks.
- Months.

For daily golden and death crosses, investigate subsequent returns over suitable daily and weekly horizons rather than assuming immediate predictive value.

For liquidation cascades and liquidity events, investigate shorter executable horizons.

## 12.4 Research dimensions

Analyse by:

- Indicator.
- Event type.
- Event magnitude.
- Strategy.
- Symbol.
- Asset group.
- Direction.
- Exchange.
- Horizon.
- Market regime.
- Volatility regime.
- Liquidity regime.
- Positioning regime.
- Execution method.

Avoid exhaustive combinations.

## 12.5 Research metrics

- Gross return.
- Net return.
- Directional accuracy.
- Win rate.
- Profit factor.
- Net expectancy.
- Average winner.
- Average loser.
- Payoff ratio.
- Breakeven win rate.
- MFE.
- MAE.
- Time to MFE.
- Time to MAE.
- Profit capture.
- Giveback.
- R-multiples.
- Drawdown.
- Sharpe ratio.
- Sortino ratio.
- Calmar ratio.
- Spread expansion.
- Slippage.
- Fill probability.
- Liquidity recovery.
- Capital efficiency.
- Return per capital-hour.
- Effective independent evidence.
- Statistical uncertainty.

Use consistent net return series and appropriate denominators.

Do not annualise sparse per-trade returns indiscriminately.

## 12.6 Matched controls

Compare indicator-conditioned outcomes against matched baseline periods.

Match where practical for:

- Time of day.
- Day of week.
- Market volatility.
- Liquidity.
- Existing trend.
- Strategy.
- Symbol.
- Direction.
- Execution conditions.

## 12.7 Incremental feature value

Determine whether a feature adds information beyond existing indicators.

Compare:

1. Existing v35 strategy.
2. Existing strategy plus one intelligence domain.
3. Existing strategy plus selected additional domains.
4. Full supported market intelligence.

Use feature ablation and out-of-sample comparison.

Reject indicators that add complexity without incremental value.

## 12.8 Statistical safeguards

Account for:

- Cross-symbol correlation.
- Cross-strategy correlation.
- Overlapping holding periods.
- Overlapping event windows.
- Simultaneous announcements.
- Repeated observations of persistent regimes.
- Serial dependence.
- Multiple hypothesis testing.
- Selection bias.
- Survivorship bias.
- Structural market changes.
- Effective independent sample size.

Use dependence-aware uncertainty estimation.

## 12.9 Validation

Require:

- Chronological walk-forward evaluation.
- Untouched holdout periods.
- Announcement-group-aware validation.
- Leakage prevention.
- Experiment history.
- Multiple-testing controls.
- Robustness across market regimes.
- Sensitivity to execution assumptions.

## Acceptance

A reusable research framework that determines whether known indicators genuinely improve net trading outcomes.

---

# WP13 — Existing Strategy Enhancement

**Priority: Critical**

Improve existing strategies before introducing dedicated new strategies.

## 13.1 Strategy coverage

Investigate:

- Order-book imbalance.
- Trade-flow imbalance.
- VWAP mean reversion.
- VWAP dislocation.
- Breakout strategies.
- Short-term momentum.
- Longer-term momentum.
- Relative strength.
- Funding and basis strategies.
- Other existing strategies.

## 13.2 Candidate adaptations

Determine whether shared intelligence improves:

- Signal reliability.
- Entry thresholds.
- Required net edge.
- Directional confidence.
- Entry timing.
- Order type.
- Maximum order age.
- Position sizing.
- Stop distance.
- Take-profit distance.
- Trailing activation.
- Trailing distance.
- Breakeven protection.
- Profit protection.
- Maximum holding time.
- Exit timing.
- Abstention.

## 13.3 Initial research hypotheses

Examples:

- Mean reversion may perform differently during strong directional trends.
- Momentum may perform better when market breadth confirms the move.
- Order-book imbalance may become unreliable during liquidation shocks.
- Breakouts may benefit from volatility compression and subsequent expansion.
- Death crosses may provide either trend confirmation or contrarian opportunities.
- Funding extremes may alter short-term reversal probability.
- Cross-asset confirmation may reduce false signals.
- On-chain valuation may improve longer-horizon regime selection.
- Macro announcements may invalidate normal liquidity assumptions.

These are research hypotheses, not hardcoded trading rules.

## 13.4 Profile ownership

Reuse existing v35 profile architecture.

Entry and exit policies remain:

- Strategy-specific.
- Symbol-specific where supported.
- Versioned.
- Validated.
- Auditable.

Use broader validated fallback profiles where specialised evidence is insufficient.

Do not silently alter existing open-position policies.

## Acceptance

Demonstrated incremental out-of-sample net improvement over the completed v35 baseline.

---

# WP14 — Market-Aware Risk Management

**Priority: Critical**

## 14.1 Risk context

Extend existing risk decisions to incorporate:

- Upcoming macroeconomic events.
- Current market regime.
- Volatility expansion.
- Liquidity deterioration.
- Liquidation cascades.
- Correlated positioning.
- Breadth deterioration.
- Trend transitions.
- Cross-asset stress.
- Data degradation.

## 14.2 Candidate risk adaptations

Investigate:

- Reduced position sizing.
- Exposure reduction.
- Cancellation of vulnerable passive orders.
- Increased required net edge.
- Temporary entry abstention.
- Volatility-adjusted sizing.
- Correlation-aware exposure limits.
- Liquidity-adjusted risk.
- Event-conditioned capital concentration.
- Recovery-based entry resumption.

Learn appropriate policies rather than imposing universal restrictions.

## 14.3 Protective exit continuity

Market regime changes and macroeconomic announcements must not disable, delay or silently alter:

- Fixed stops.
- Trailing stops.
- Breakeven protection.
- Profit protection.
- Take-profit orders.
- Maximum holding periods.
- Strategy-managed exits.
- Emergency exits.

Do not automatically widen stops or reset accumulated profit protection.

Preserve immutable initial risk attribution.

Any permitted open-position policy changes must be validated and auditable.

## 14.4 Liquidity shock handling

Account for:

- Price gaps.
- Spread expansion.
- Stale quotes.
- Reduced executable depth.
- Partial fills.
- Rejected orders.
- Failed cancellations.
- Execution delays.

Do not assume protective orders guarantee trigger-price execution.

## 14.5 Data degradation matrix

### Missing consensus

Disable surprise-dependent features.

Retain event timing and observed market reaction.

### Delayed actual release

Remain in awaiting-release state.

Do not assume a neutral result.

### Calendar outage

Use last verified schedules only within validated freshness limits.

### Cross-asset market closed

Disable current cross-asset confirmation.

### Cross-asset price stale

Exclude affected current features.

### Technical indicator unavailable

Use validated fallback features or abstain from affected decisions.

### Insufficient indicator warm-up

Do not publish fully valid indicator states.

### Derivatives feed unavailable

Disable affected positioning and liquidation features.

### On-chain or flow data delayed

Retain explicitly timestamped historical context without representing it as current.

### Conflicting sources

Quarantine unresolved conflicting observations.

### Market-path observation gap

Exclude unsupported excursion and replay results.

### Market intelligence service unavailable

Use validated non-intelligence fallback profiles where appropriate.

Otherwise abstain from affected decisions.

Preserve existing protective exits.

## Acceptance

Validated market-aware risk policies that improve risk-adjusted outcomes without unnecessary trading suppression.

---

# WP15 — Market-Aware Execution

**Priority: High**

## 15.1 Execution research

Investigate execution quality during:

- Macroeconomic announcements.
- Trend transitions.
- Breakouts.
- Volatility expansion.
- Liquidation cascades.
- Funding extremes.
- Deleveraging.
- Liquidity recovery.
- Correlation shocks.

## 15.2 Metrics

- Maker fill probability.
- Passive-order adverse selection.
- Spread.
- Slippage.
- Market impact.
- Available depth.
- Partial fills.
- Cancellation frequency.
- Quote freshness.
- Execution latency.
- Liquidity recovery time.
- Trigger-to-execution price difference.
- Net execution costs.

## 15.3 Candidate adaptations

- Order lifetime.
- Passive entry distance.
- Maker versus taker selection.
- Maximum acceptable slippage.
- Minimum executable depth.
- Revalidation frequency.
- Entry delay.
- Order sizing.
- Cancellation behaviour.
- Execution abstention.

## 15.4 Execution realism

Account for:

- Actual bid and ask.
- Relevant executable depth.
- Queue position where supported.
- Feed latency.
- Submission latency.
- Cancellation latency.
- Partial fills.
- Fees.
- Adverse selection.
- Market impact assumptions.

A market touching an order price does not establish that the order would have filled.

## 15.5 Venue awareness

Evaluate venue-specific liquidity, fees, latency and execution behaviour.

Preserve distinct market-data and execution venue identities.

## Acceptance

Validated execution adaptations that improve net outcomes while preserving existing performance and emergency exit requirements.

---

# WP16 — Dedicated Market Intelligence Strategies

**Priority: High, dependent on empirical evidence**

All candidates begin as historical research or Scout strategies.

Do not implement every candidate merely because it appears in this plan.

## 16.1 Post-announcement momentum

Investigate sustained directional continuation following macroeconomic price discovery.

Potential inputs:

- Economic surprise.
- Initial BTC reaction.
- Cross-asset confirmation.
- Volume.
- Trade flow.
- Liquidity recovery.

## 16.2 Announcement overreaction

Investigate extreme initial market reactions followed by reversal.

## 16.3 BTC-to-altcoin lead-lag

Investigate delayed information absorption by less liquid cryptocurrencies.

Account for existing price movement, liquidity, correlation and execution costs.

## 16.4 Cross-asset confirmation and divergence

Investigate cryptocurrency opportunities supported by or diverging from Treasury yields, dollar and equity futures.

## 16.5 Pre-announcement positioning

Investigate systematic positioning behaviour before scheduled announcements.

Do not assume knowledge of unreleased results.

## 16.6 Monetary policy regime momentum

Investigate longer-horizon trends conditioned on inflation, employment, monetary policy and financial conditions.

## 16.7 Trend transition strategy

Investigate:

- Golden crosses.
- Death crosses.
- Long-term moving-average reclaims.
- Trend alignment.
- Confirmed market structure transitions.

Compare trend-following and contrarian variants.

## 16.8 Volatility compression breakout

Investigate volatility squeezes followed by executable directional expansion.

Use volume, breadth, liquidity and trend confirmation where validated.

## 16.9 Momentum divergence reversal

Investigate RSI, MACD, volume and price divergences.

Require point-in-time-correct confirmation.

## 16.10 Liquidation continuation

Investigate directional continuation during forced deleveraging.

## 16.11 Liquidation exhaustion reversal

Investigate reversal opportunities following liquidation intensity peaks and liquidity recovery.

## 16.12 Funding and positioning extremes

Investigate whether crowded derivatives positioning predicts continuation, reversal or asymmetric risk.

## 16.13 Spot versus derivatives divergence

Investigate discrepancies between spot demand and derivatives-driven price movement.

## 16.14 Market breadth confirmation

Investigate whether broad participation improves momentum and breakout reliability.

## 16.15 Market breadth divergence

Investigate narrow market leadership and deteriorating participation as potential reversal or abstention indicators.

## 16.16 Capital rotation and relative strength

Investigate:

- BTC-to-ETH rotation.
- BTC-to-altcoin rotation.
- Relative strength continuation.
- Relative strength reversal.
- Cross-sectional dispersion.

## 16.17 Liquidity recovery

Investigate opportunities as spreads, executable depth and fill quality recover after market shocks.

## 16.18 Institutional flow momentum

Investigate whether observable ETF flows or other institutional demand proxies provide incremental directional information.

## 16.19 On-chain regime strategy

Investigate longer-horizon strategies conditioned on realised valuation, holder profitability and miner economics.

Do not promote cycle-specific strategies without sufficient independent evidence.

## Acceptance

Only empirically supported candidates proceed to existing Learning, risk, financial and promotion gates.

Reject candidates without reliable incremental net profitability.

---

# WP17 — Autonomous Learning and Scout Intelligence

**Priority: Critical**

Extend existing Learning rather than creating a separate research system.

## 17.1 Autonomous research objectives

Learning should determine:

- Which indicators matter.
- Which indicators are redundant.
- Which indicators improve particular strategies.
- Which regimes favour particular strategies.
- Which regime transitions create opportunities.
- Which market conditions increase risk.
- Which features justify specialised profiles.
- Which strategies should abstain.
- Which execution policies perform best.
- Which exit policies perform best.
- Which relationships have deteriorated.

## 17.2 Scout evidence acquisition

Support:

- Existing strategy behaviour across market regimes.
- Alternative entry delays.
- Alternative exit policies.
- Alternative execution policies.
- Indicator-conditioned variants.
- Macro-conditioned variants.
- Cross-asset confirmation variants.
- Positioning-conditioned variants.
- Matched control periods.
- Historical replay.

Prioritise acquisition by expected information value.

Do not maximise raw Scout outcome counts.

## 17.3 Hierarchical Learning

Avoid exhaustive combinations of:

- Indicators.
- Event types.
- Symbols.
- Strategies.
- Directions.
- Horizons.
- Regimes.
- Exchanges.
- Execution variants.
- Exit-policy variants.

Start with broad relationships.

Progressively specialise:

1. Global market behaviour.
2. Intelligence-domain behaviour.
3. Event or regime category.
4. Strategy or asset-group behaviour.
5. Symbol and strategy-specific behaviour.
6. Additional specialised profiles only where justified.

Use validated broader profiles as fallbacks.

## 17.4 Experiment control

Bound:

- Concurrent experiments.
- Candidate variants.
- Profile growth.
- Scout capacity.
- Historical replay workload.
- Computational resource consumption.

Record:

- Research hypotheses.
- Parameters.
- Data versions.
- Feature versions.
- Validation periods.
- Rejected hypotheses.
- Failed experiments.
- Effective independent evidence.
- Promotion decisions.

Avoid repeatedly rediscovering rejected relationships.

## 17.5 Independent evidence units

Different indicator categories require different independence definitions.

### Macroeconomic events

Use independent announcement episodes.

### Technical crossovers

Use independent crossover and trend episodes.

### Persistent regimes

Use independent regime periods and transitions.

### Liquidations

Use distinct liquidation or deleveraging episodes.

### Market breadth

Use independent market-wide transitions rather than individual correlated symbol observations.

### On-chain cycles

Use genuinely independent cycle or regime evidence.

Do not count repeated observations of a persistent state as independent events.

## 17.6 Initial macroeconomic evidence thresholds

Provisional minimum research requirements:

- Hypothesis generation: 12 independent announcement episodes.
- Candidate research: 24 independent episodes.
- Formal validation: 36 independent episodes.
- Untouched holdout: 12 additional episodes.
- Subsequent natural paper validation: six additional episodes.

These are initial conservative research floors, not statistically proven universal thresholds.

Do not apply them mechanically to every indicator category.

## 17.7 Technical and regime evidence thresholds

Define category-specific evidence requirements based on:

- Effective independent sample size.
- Effect size.
- Statistical uncertainty.
- Serial dependence.
- Holding horizon.
- Number of tested variants.
- Market regime coverage.
- Execution sensitivity.
- Strategy complexity.

Do not promote a daily or weekly indicator simply because thousands of intraday observations occurred while it remained in the same state.

## 17.8 Promotion requirements

Require:

- Sufficient independent evidence.
- Chronological walk-forward validation.
- Untouched holdout evidence.
- Positive incremental out-of-sample net expectancy.
- Conservative uncertainty assessment.
- Realistic execution costs.
- Robustness across relevant regimes.
- Sensitivity analysis.
- Multiple-testing controls.
- Existing v35 promotion requirements.
- Subsequent natural paper evidence.

Synthetic fixtures establish correctness only.

Do not automatically promote candidates after a fixed sample count.

## 17.9 Continuous monitoring

Monitor promoted relationships for:

- Net performance deterioration.
- Changing regime sensitivity.
- Increased execution costs.
- Reduced liquidity.
- Feature degradation.
- Structural market changes.
- Loss of predictive value.

Use existing demotion and revalidation mechanisms.

## Acceptance

Autonomous, bounded and statistically defensible Learning that identifies useful market intelligence without uncontrolled profile fragmentation.

---

# WP18 — Market Intelligence Excursions and What-if Analysis

**Priority: High**

Reuse v35 excursion and chronological replay infrastructure.

## 18.1 Excursion research

Analyse:

- MFE and MAE by market regime.
- MFE and MAE around announcements.
- MFE and MAE around technical transitions.
- Post-liquidation excursions.
- Post-breakout excursions.
- Profit capture.
- Giveback.
- Time to favourable excursion.
- Time to adverse excursion.
- Stop-out frequency.
- Post-exit continuation.
- Post-exit reversal.

## 18.2 Winner and loser transitions

Support:

- Winner to winner.
- Winner to loser.
- Loser to winner.
- Loser to loser.
- Flat outcomes.
- Unknown outcomes.

Preserve correct denominators and evidence coverage.

## 18.3 Market-path requirements

Reuse:

- Event-driven MFE and MAE.
- Maximum one-second baseline path sampling.
- Intra-sample extrema.
- Chronological event ordering.
- Side-correct executable prices.
- Relevant liquidity.
- Protective threshold events.
- Trade-specific historical path retention.
- Sub-second markouts where supported.
- Post-exit observations.

Use a ten-minute general rolling market-data buffer where appropriate.

Do not apply this limit to retained trade-specific or research-event-specific evidence.

Preserve complete supported observation horizons.

Share market paths across overlapping AutoTrader and Scout positions.

## 18.4 What-if scenarios

Support:

- Delayed entry following macro announcements.
- Delayed entry following technical crossovers.
- Waiting for market breadth confirmation.
- Waiting for liquidity recovery.
- Requiring cross-asset confirmation.
- Avoiding crowded derivatives positioning.
- Trading only within selected regimes.
- Alternative stop distances.
- Alternative targets.
- Alternative trailing protection.
- Alternative breakeven protection.
- Alternative maximum holding periods.
- Alternative execution methods.
- Alternative position sizing.
- Ignoring individual intelligence domains entirely.

## 18.5 Counterfactual classifications

Distinguish:

- Directly observed outcomes.
- Deterministic rule replay.
- Modelled execution.
- Stochastic fill and queue simulation.
- Unsupported outcomes.

Do not represent hypothetical trades as actual executions.

## 18.6 Execution assumptions

Expose:

- Queue assumptions.
- Fill probability.
- Spread.
- Slippage.
- Fees.
- Available liquidity.
- Market impact.
- Entry latency.
- Cancellation latency.
- Data gaps.
- Threshold ordering uncertainty.

Evaluate sensitivity to alternative assumptions.

## 18.7 Comparison metrics

Compare original and alternative:

- Net P&L.
- Net expectancy.
- Profit factor.
- R-multiples.
- Win rate.
- Average winner.
- Average loser.
- Drawdown.
- Holding duration.
- Capital efficiency.
- Execution costs.
- MFE.
- MAE.
- Profit capture.
- Giveback.
- Evidence coverage.
- Uncertainty.

Preserve immutable initial risk attribution.

## Acceptance

Reliable comparative analysis of market-intelligence-conditioned decisions with explicit counterfactual assumptions.

---

# WP19 — Autonomous Capital Allocation

**Priority: High**

Extend existing allocation decisions using validated market intelligence.

## 19.1 Allocation inputs

Consider:

- Regime-conditioned net expectancy.
- Event-conditioned net expectancy.
- Effective independent evidence.
- Strategy confidence.
- Expected execution costs.
- Expected volatility.
- Available liquidity.
- Drawdown.
- Capital efficiency.
- Risk-adjusted performance.
- Current exposure.
- Correlated market exposure.
- Venue-specific available capital.

## 19.2 Correlated exposure

Account for:

- BTC-led market exposure.
- Common directional exposure.
- Common macroeconomic sensitivity.
- Common volatility exposure.
- Common liquidation risk.
- Common market regime.
- Correlated strategy signals.
- Shared announcement episodes.

Multiple cryptocurrencies do not necessarily represent independent diversification.

## 19.3 Allocation decisions

Determine whether capital is best deployed to:

- Existing non-conditioned strategies.
- Macro-enhanced strategies.
- Technical regime-enhanced strategies.
- Positioning-enhanced strategies.
- Dedicated market intelligence strategies.
- Unallocated capital.

Do not introduce fixed manual strategy or regime budgets.

Do not force capital deployment.

## 19.4 Venue constraints

Preserve:

- Execution venue identity.
- Market-data venue identity.
- Venue-specific balances.
- Collateral restrictions.
- Available capital.
- Exchange-specific execution costs.

Do not assume capital is freely transferable across venues.

## 19.5 Portfolio validation

Compare performance with and without intelligence-conditioned allocation.

Account for:

- Competing opportunities.
- Opportunity cost.
- Correlated exposure.
- Turnover.
- Drawdown.
- Capital utilisation.
- Net returns.

## Acceptance

Improved portfolio-level risk-adjusted allocation without excessive correlated exposure or duplicated capital budgets.

---

# WP20 — Dashboard and Operator Intelligence

**Priority: High**

Extend the existing configurable dashboard.

Preserve the compact v35 trading-terminal design.

Avoid creating separate widgets for every indicator category.

## 20.1 New widget — Economic Calendar

Provide:

- Upcoming announcements.
- Countdown.
- Importance.
- Previous value.
- Forecast.
- Actual value.
- Surprise.
- Revision.
- Release status.
- Freshness.
- Source.

Support:

- Upcoming and historical views.
- Country filtering.
- Event filtering.
- Importance filtering.
- Date range filtering.
- Announcement-group drill-down.
- Observed market reactions.

## 20.2 New widget — Market Intelligence

Provide configurable tabs:

- Overview.
- Technical.
- Breadth.
- Positioning.
- Macro.
- Cross-Asset.
- On-Chain.
- Flows.

### Overview

Display compact summaries of:

- BTC trend.
- ETH trend.
- Market breadth.
- Volatility.
- Liquidity.
- Positioning.
- Macro risk.
- Capital rotation.
- Relevant regime transitions.

### Technical

Display:

- Golden/death cross state.
- Time since crossover.
- Moving-average distances.
- Trend slopes.
- RSI.
- MACD.
- Momentum.
- Volatility.
- Breakouts.
- Market structure.

### Breadth

Display:

- Percentage above key moving averages.
- Advance/decline ratio.
- New highs/lows.
- BTC dominance.
- ETH/BTC.
- Relative strength.
- Return dispersion.

### Positioning

Display:

- Funding.
- Open interest.
- Funding extremes.
- Liquidation activity.
- Basis.
- Spot versus derivatives divergence.
- Options indicators where available.

### Macro

Display:

- Economic regime.
- Upcoming event risk.
- Recent releases.
- Surprise history.
- Event phase.

### Cross-Asset

Display:

- Dollar movement.
- Treasury yields.
- Equity futures.
- Gold.
- Volatility.
- Confirmation and divergence.
- Session status.

### On-Chain

Display:

- MVRV.
- NUPL.
- SOPR.
- Realised valuation.
- Holder cost basis.
- Hash Ribbons.
- Puell Multiple.
- Cycle context.

### Flows

Display:

- ETF flows.
- Stablecoin supply.
- Stablecoin flows.
- Exchange flows.
- Supported sentiment indicators.

Each tab must clearly identify unsupported, stale and unavailable observations.

## 20.3 New widget — Market Regime Monitor

Provide:

- Current multi-dimensional regimes.
- Regime transitions.
- Time in regime.
- Supporting features.
- Conflicting features.
- Confidence where supported.
- Historical regime timeline.
- Regime heatmap by symbol or asset group.
- Strategy performance by regime.

Support global, BTC, ETH, asset-group and symbol views.

Avoid reducing the market to a misleading single bullish/bearish score.

## 20.4 New widget — Market Intelligence Research

Provide configurable research views:

- Economic event studies.
- Technical crossover studies.
- Indicator effectiveness.
- Market regime performance.
- Positioning event studies.
- Liquidation studies.
- Cross-asset lead-lag.
- Feature correlation.
- Feature ablation.
- Incremental predictive value.
- Return distributions.
- Excursion distributions.
- Parameter sensitivity.
- What-if comparisons.
- Effective independent evidence.
- Validation and promotion status.

Support aggregate-to-record drill-down.

## 20.5 Extend AutoTrader Controls & Safety

Add:

- Market intelligence availability.
- Current event risk.
- Relevant market regime.
- Applicable validated risk policies.
- Degraded feature states.

Do not introduce unnecessary manual indicator controls.

## 20.6 Extend Decision Funnel

Add:

- Intelligence-related eligibility.
- Regime-related abstention.
- Missing feature evidence.
- Event-conditioned decisions.
- Relevant confirmation requirements.

Preserve separate AutoTrader and Scout populations.

## 20.7 Extend Orders & Positions

Add:

- Market regime at entry.
- Current market regime.
- Relevant event context.
- Relevant indicator transitions.
- Applicable conditioned profile.
- Entry and exit attribution.

Preserve compact position tables and net-versus-breakeven bars.

## 20.8 Extend Execution Quality

Add:

- Execution quality by regime.
- Execution quality around announcements.
- Execution quality during liquidations.
- Spread expansion.
- Fill probability comparison.
- Slippage comparison.
- Liquidity recovery.

## 20.9 Extend Portfolio & Reconciliation

Add:

- Regime-conditioned financial attribution.
- Event-period financial attribution.
- Strategy contribution.
- Regime-related drawdown.

Use authoritative reconciled financial evidence.

Do not include Scout simulated profits in actual portfolio totals.

## 20.10 Extend Autonomous Capital Allocation

Add:

- Correlated market exposure.
- Regime-conditioned allocation.
- Event-related capital constraints.
- Competing intelligence-conditioned opportunities.

## 20.11 Extend Strategy Performance

Add:

- Performance by regime.
- Performance by indicator condition.
- Original versus enhanced strategy.
- Feature-domain comparisons.
- Effective independent evidence.
- Risk-adjusted performance.

## 20.12 Extend Learning & Evidence Acquisition

Add:

- Market intelligence evidence gaps.
- Independent event counts.
- Independent regime counts.
- Historical coverage.
- Acquisition activity.
- Candidate validation.
- Promotion eligibility.
- Rejected hypotheses.
- Feature redundancy.
- Relationship deterioration.

## 20.13 Extend Excursions & Exit Analysis

Add:

- Event overlays.
- Indicator overlays.
- Regime overlays.
- Conditioned excursions.
- Alternative entry policies.
- Alternative exit policies.
- Winner/loser transitions.
- Execution assumption sensitivity.

## 20.14 UI architecture

Every view must have a corresponding tested Model.ts responsible for:

- Lifecycle.
- Subscriptions.
- Requests.
- Observable state.
- Transformation.
- Sorting.
- Filtering.
- Pagination.
- Drill-down.
- Configuration persistence.
- Error handling.
- Recovery.
- Cancellation.
- Cleanup.

TSX remains presentation-focused.

Generic components belong in ui-platform.

Trading-specific business logic remains in the application.

## 20.15 UI quality

Require:

- Compact trading-terminal presentation.
- Single widget headers.
- Consistent scrolling.
- Responsive resizing.
- Independent widget configuration.
- Saved layouts.
- Accessible expanded details.
- Shared tables and charts.
- Shared heatmaps.
- Shared status indicators.
- Explicit stale and unavailable states.
- No unnecessary duplicate subscriptions.
- No fabricated runtime data.
- No arbitrary fixed pixel requirements.

Review actual browser screenshots against approved designs.

Component tests alone do not establish visual acceptance.

## 20.16 Legacy consolidation

Audit existing indicator widgets and backend dependencies.

Consolidate overlapping capabilities into the approved intelligence widgets.

Remove verified obsolete:

- Widgets.
- Models.
- Routes.
- Subscriptions.
- Handlers.
- Aggregations.
- Polling.
- Caches.
- Background workers.

Verify all consumers before deletion.

Preserve shared authoritative data, operational monitoring and unrelated market widgets.

Migrate existing saved dashboard layouts safely.

## Acceptance

A compact, configurable and source-backed intelligence interface with verified runtime data, tested lifecycle behaviour and separate operator approval.

---

# WP21 — Performance, Reliability and Data Integrity

**Priority: Critical**

## 21.1 Hot-path isolation

- External ingestion remains outside the execution hot path.
- No synchronous external API requests during trading decisions.
- Shared global features calculated once.
- Incremental per-symbol calculations.
- Efficient feature distribution.
- Bounded queues.
- Bounded caches.
- Asynchronous persistence.
- Controlled background workloads.
- No unnecessary per-symbol macroeconomic polling.

## 21.2 Indicator calculation efficiency

Prefer incremental calculations where appropriate.

Avoid repeatedly recalculating complete historical indicator series.

Share:

- Candle histories.
- Moving averages.
- Volatility calculations.
- Market-wide breadth.
- Global regimes.
- Announcement context.
- Cross-asset observations.
- Market-path evidence.

Avoid duplicate calculations for multiple widget instances or Scouts.

## 21.3 Timestamp integrity

Maintain reliable clock synchronisation.

Record where supported:

- Source timestamp.
- Observation timestamp.
- Ingestion timestamp.
- Feature availability timestamp.
- Clock uncertainty.
- Source-to-system latency.
- Feature publication latency.

Do not claim sub-second accuracy where timestamp uncertainty prevents it.

## 21.4 Persistence and recovery

Require:

- Restart-safe ingestion.
- Idempotent processing.
- Duplicate prevention.
- Stable feature identities.
- Stable event identities.
- Revision-aware storage.
- Source provenance.
- Recovery of interrupted acquisition.
- Explicit observation gaps.
- Indicator warm-up recovery.
- Regime state recovery.

## 21.5 Operational monitoring

Use existing Grafana infrastructure.

Monitor:

- Source availability.
- Data freshness.
- Ingestion latency.
- Missing observations.
- Delayed releases.
- Conflicting sources.
- Indicator calculation latency.
- Regime calculation latency.
- Worker health.
- Queue depth.
- Persistence latency.
- Storage growth.
- Scout capacity.
- Research workload.

Do not duplicate operational monitoring inside trading widgets.

## 21.6 Performance acceptance

Preserve v35 execution targets.

Benchmark:

- p50 execution latency.
- p95 execution latency.
- p99 execution latency.
- Emergency exit latency.
- Sustained throughput.
- CPU.
- Memory.
- Database contention.
- Persistence latency.
- Network overhead.
- Feature calculation cost.

Use representative concurrent workloads:

- Market-data ingestion.
- Macroeconomic ingestion.
- Technical indicator calculation.
- Derivatives ingestion.
- Cross-asset ingestion.
- Insights.
- Scouts.
- Learning.
- AutoTrader.
- Position Management.
- Execution.
- UI subscriptions.

Include announcement bursts, liquidation cascades and broad market regime transitions.

## 21.7 Graceful degradation

Failure of market intelligence must not:

- Interrupt protective exits.
- Corrupt financial evidence.
- Block unrelated strategies.
- Generate fabricated neutral values.
- Cause uncontrolled retries.
- Create unbounded queues.
- Degrade execution latency beyond accepted requirements.

## Acceptance

Market intelligence improves trading capabilities without degrading existing execution performance, reliability or financial correctness.

---

# WP22 — Integrated Validation and Programme Acceptance

**Priority: Critical**

Validate the complete lifecycle:

Market observation or economic announcement → ingestion → feature generation → regime classification → Strategy → Scout → Learning → validated profile → AutoTrader → capital allocation → execution → Position Management → financial reconciliation → subsequent Learning.

## 22.1 Required comparisons

Compare:

- Completed v35 baseline.
- Calendar-only enhancement.
- Economic surprise enhancement.
- Cross-asset enhancement.
- Technical indicator enhancement.
- Market breadth enhancement.
- Positioning enhancement.
- On-chain enhancement.
- Capital-flow enhancement.
- Multi-dimensional regime enhancement.
- Full supported intelligence enhancement.

Use feature ablation to identify incremental value.

## 22.2 Portfolio comparisons

Compare:

- Existing risk versus market-aware risk.
- Existing execution versus market-aware execution.
- Existing allocation versus market-aware allocation.
- Existing strategies versus enhanced strategies.
- Dedicated strategies versus matched controls.
- Full portfolio with and without market intelligence.

## 22.3 Acceptance metrics

- Net profitability.
- Net expectancy.
- Profit factor.
- Sharpe ratio.
- Sortino ratio.
- Calmar ratio where meaningful.
- Drawdown.
- Win rate.
- Average winner.
- Average loser.
- Payoff ratio.
- Execution costs.
- Fill quality.
- Capital efficiency.
- Effective independent evidence.
- Out-of-sample performance.
- Robustness across regimes.
- Sensitivity to execution assumptions.

## 22.4 Evidence classification

Maintain separate classifications for:

- Synthetic fixtures.
- Historical observed evidence.
- Historical deterministic replay.
- Modelled execution.
- Stochastic execution simulation.
- Natural Scout evidence.
- Natural paper-trading evidence.
- Reconciled financial evidence.

Synthetic fixtures establish correctness, not profitability.

## 22.5 Acceptance states

Track separately:

- Implementation.
- Unit testing.
- Integration testing.
- Deployment.
- Runtime verification.
- Data integrity.
- Performance acceptance.
- Visual acceptance.
- Historical empirical validation.
- Natural paper validation.
- Financial reconciliation.
- Promotion eligibility.

Do not close work packages merely because code exists or tests pass.

## 22.6 Negative research results

A negative research result is valid.

Where an indicator or strategy fails to demonstrate incremental value:

- Record the result.
- Preserve relevant evidence.
- Reject or defer the candidate.
- Avoid unnecessary infrastructure.
- Avoid unnecessary profile growth.
- Do not manufacture profitability through repeated optimisation.

## 22.7 Operational restrictions

- Preserve Paper-only operation.
- No automatic live-trading activation.
- No weakened risk controls.
- No weakened Learning gates.
- No unauthorised trading setting changes.
- No historical database resets.
- No paid data dependencies without approval.

## Acceptance

Demonstrated incremental value over the completed v35 baseline with all existing financial, execution, risk and statistical safeguards preserved.

---

# 4. Delivery Sequence

## Wave 0 — Feasibility and Existing Capability Audit

Primary work:

- WP0.
- Initial WP21 requirements.

Deliver:

- Existing indicator inventory.
- Existing data-source inventory.
- Historical coverage assessment.
- Source capability matrix.
- Cost and licensing assessment.
- Reuse and consolidation plan.
- Prioritised research backlog.

Exit gate:

Reliable initial research inputs identified.

## Wave 1 — Existing-Data Intelligence Foundations

Primary work:

- WP5.
- WP6.
- WP7.
- Existing-data portions of WP8.
- Relevant WP2.
- WP21 foundations.

Deliver:

- Shared technical indicators.
- Golden and death cross detection.
- Multi-timeframe trend features.
- Momentum and volatility features.
- Market breadth.
- Relative strength.
- Existing derivatives features.
- Historical feature reconstruction.

Prioritise indicators that require no new external subscriptions.

Exit gate:

Reliable reusable technical and market-wide features with point-in-time correctness.

## Wave 2 — Macroeconomic Data Foundations

Primary work:

- WP1.
- WP2.
- Initial WP3.
- Minimal WP20 economic calendar.

Deliver:

- Economic calendar.
- Historical releases.
- Announcement grouping.
- Versioned observations.
- Basic macroeconomic features.
- Passive announcement collection.
- Minimal calendar UI.

Do not depend on unavailable cross-asset feeds.

Exit gate:

Reliable economic event and historical evidence foundations.

## Wave 3 — Initial Regime and Indicator Research

Primary work:

- Initial WP11.
- WP12.
- Initial WP17.
- Initial WP18.
- Minimal market intelligence UI.

Deliver:

- Initial multi-dimensional regimes.
- Technical event studies.
- Macroeconomic event studies.
- Matched controls.
- Independent evidence accounting.
- Initial Scout research.
- Feature redundancy analysis.

Primary research questions:

- Does technical market regime improve existing strategies?
- Does market breadth improve directional confidence?
- Does economic event timing improve risk-adjusted performance?
- Does existing derivatives positioning improve net outcomes?

Exit gate:

Statistically defensible initial findings.

## Wave 4 — Expanded Intelligence Sources

Primary work:

- WP4.
- Remaining WP8.
- WP9.
- WP10.
- Remaining WP3.
- Extended WP11.

Deliver where feasible:

- Cross-asset intelligence.
- Richer economic surprise features.
- Expanded derivatives positioning.
- Liquidation intelligence.
- On-chain intelligence.
- ETF flows.
- Stablecoin liquidity.
- Capital-flow intelligence.

Implement additional sources according to demonstrated research value and cost.

Exit gate:

Supported incremental information beyond existing-data intelligence.

## Wave 5 — Improve Existing Trading

Primary work:

- WP13.
- WP14.
- WP15.
- Extended WP17.
- Extended WP18.

Deliver:

- Intelligence-enhanced existing strategies.
- Regime-aware risk.
- Event-aware execution.
- Validated fallback policies.
- Comparative What-if research.
- Initial validated conditioned profiles.

Exit gate:

Supported incremental out-of-sample net improvements.

## Wave 6 — Dedicated Opportunities and Allocation

Primary work:

- WP16.
- WP19.
- Extended WP17.

Deliver:

- Empirically supported dedicated strategies.
- Market-aware capital allocation.
- Correlated exposure management.
- Continued natural paper evidence.

Exit gate:

Candidates satisfy applicable evidence and risk requirements.

## Wave 7 — Complete Integration and Acceptance

Primary work:

- Remaining WP20.
- Final WP21.
- WP22.

Deliver:

- Complete dashboard integration.
- Legacy consolidation.
- Full lifecycle validation.
- Performance acceptance.
- Financial reconciliation.
- Operator visual approval.
- Historical empirical acceptance.
- Natural paper validation where sufficient events have occurred.

Engineering completion and market-dependent empirical acceptance remain separately reported.

---

# 5. Parallel Delivery and Agent Management

Reuse the established v35 coordinator and specialist workflow.

Maintain two concurrent specialists where independent work exists.

## Coordinator

Responsible for:

- Architecture.
- Programme sequencing.
- Dependency management.
- Contract consistency.
- Integration.
- Acceptance.
- Resource allocation.
- Beads task management.
- Avoiding duplicated work.

Use medium reasoning where supported.

Escalate to high reasoning for complex architectural, statistical or financial decisions.

## Backend and research specialist

Responsible for:

- Ingestion.
- Feature generation.
- Point-in-time correctness.
- Statistical research.
- Regime classification.
- Learning.
- Strategy evaluation.
- Risk.
- Execution.
- Financial integrity.
- Performance.

Use high reasoning for complex trading, statistical and financial work.

## UI specialist

Responsible for:

- Dashboard integration.
- Shared ui-platform components.
- Visual fidelity.
- Responsive layouts.
- Scrolling.
- Model.ts lifecycle.
- Interaction testing.
- Legacy consolidation.

Use low reasoning for routine UI work.

Escalate to medium reasoning for complex lifecycle or interaction defects.

## Working practices

- Maintain authoritative progress in plan.md and Beads.
- Use bd remember for reusable repository navigation.
- Use RTK for bounded command outputs.
- Avoid repeatedly reading the entire master plan.
- Assign narrow independent specialist tasks.
- Avoid unnecessary full regression runs.
- Keep screenshot evidence on disk where practical.
- Maintain concise progress reports.
- Use fresh coordinator sessions at appropriate milestones.
- Preserve working-tree changes during handovers.
- Avoid multiple concurrent Cargo build owners.
- Do not introduce unnecessary tooling.

---

# 6. Mandatory Research Safeguards

## 6.1 No look-ahead bias

Never use:

- Economic values before observation.
- Future forecast revisions.
- Revised values as original releases.
- Future market reactions.
- Future cross-asset observations.
- Future liquidity information.
- Unfinished candle closes as confirmed observations.
- Future-confirmed market structure.
- Retrospectively identified divergences.
- Future-smoothed regime classifications.
- Future ETF flow information.
- Future on-chain revisions.
- Future profile validation outcomes.

## 6.2 No false independence

Do not treat:

- Multiple symbols reacting to one announcement.
- Multiple strategies observing one market shock.
- Repeated Scout outcomes.
- Simultaneous releases.
- Repeated observations of a persistent death cross.
- Multiple correlated technical indicators.
- Repeated observations of one liquidation cascade.
- Daily observations within one Bitcoin cycle.

as independent evidence without appropriate statistical justification.

## 6.3 No unrealistic execution

Do not assume:

- Every touched passive order fills.
- Stops execute at trigger prices.
- Candle extrema establish chronological paths.
- Liquidity remains constant.
- External data arrives instantaneously.
- Spread and slippage are negligible.
- Hypothetical trades are actual financial outcomes.

## 6.4 No uncontrolled experimentation

Prevent:

- Exhaustive feature combinations.
- Unbounded profile growth.
- Unbounded Scout growth.
- Repeated holdout optimisation.
- Repeated testing of rejected hypotheses.
- Selection of only successful results.
- Promotion based on raw sample count.

## 6.5 No unsupported indicator interpretations

Do not assume:

- Death crosses are always bearish.
- Golden crosses are always bullish.
- RSI oversold conditions predict immediate reversal.
- RSI overbought conditions predict immediate decline.
- Funding extremes always reverse.
- Rising open interest establishes bullish positioning.
- Liquidations always produce reversal.
- Higher BTC dominance is universally bearish for altcoins.
- ETF inflows always produce immediate price increases.
- Stablecoin issuance always represents buying demand.
- Exchange inflows always represent selling.
- Bitcoin follows a fixed four-year price cycle.
- Pi Cycle Top reliably predicts market peaks.
- Strong NFP is always bearish for crypto.
- Weak NFP is always bullish.
- Higher inflation always causes crypto prices to decline.
- Dollar strength always predicts crypto weakness.

Learn conditional relationships empirically.

## 6.6 No degraded protective behaviour

Market intelligence must not compromise:

- Existing stops.
- Trailing protection.
- Breakeven protection.
- Profit protection.
- Time exits.
- Emergency exits.
- Financial reconciliation.

---

# 7. Explicitly Deferred Scope

The following are not initial v36 requirements:

- General news sentiment.
- Social media scraping.
- Twitter/X sentiment.
- Reddit sentiment.
- LLM-based headline trading.
- Automated geopolitical interpretation.
- Institutional millisecond news trading.
- Unvalidated dealer gamma estimation.
- Automatic activation of deferred carry or hedging strategies.
- Additional exchange integrations solely for speculative research.
- Paid institutional data subscriptions without approval.
- Automated live-trading activation.

Do not allow these capabilities to expand v36 prematurely.

---

# 8. Programme Progress Reporting

Maintain a concise work-package completion matrix.

For each work package report:

- Implementation percentage.
- Testing status.
- Deployment status.
- Runtime verification.
- Historical coverage.
- Empirical validation.
- Natural paper evidence.
- Performance acceptance.
- Outstanding blockers.
- Next substantive action.

Report separately:

- Supported intelligence features.
- Features reused from existing infrastructure.
- Newly implemented features.
- Features rejected as redundant.
- Features rejected for insufficient value.
- Economic announcements collected.
- Independent announcement groups.
- Original release coverage.
- Consensus coverage.
- Cross-asset coverage.
- Technical event coverage.
- Derivatives coverage.
- On-chain coverage.
- Capital-flow coverage.
- Executable market-path coverage.
- Independent technical episodes.
- Independent liquidation episodes.
- Independent regime episodes.
- Accepted evidence.
- Effective independent evidence.
- Candidate profiles.
- Validated profiles.
- Rejected profiles.
- Promoted profiles.
- Intelligence-conditioned strict eligibility.

Distinguish engineering progress from market-dependent empirical validation.

Do not inflate progress using correlated Scout outcome counts.

---

# 9. Definition of Success

The completed system should autonomously:

1. Understand the current technical trend across multiple horizons.
2. Identify significant technical transitions such as golden and death crosses.
3. Understand market breadth and relative strength.
4. Identify capital rotation.
5. Understand derivatives positioning and leverage.
6. Detect liquidation and deleveraging episodes.
7. Understand relevant on-chain and cycle context.
8. Observe institutional and stablecoin capital flows.
9. Know when significant economic announcements are approaching.
10. Observe economic releases and supported surprises.
11. Observe cross-asset market reactions.
12. Identify changing volatility and liquidity conditions.
13. Maintain explainable multi-dimensional market regimes.
14. Determine which existing strategies benefit from each condition.
15. Determine when strategies should abstain.
16. Acquire missing evidence through Scouts and historical replay.
17. Validate relationships across independent market episodes.
18. Adapt entry, exit, sizing and execution policies where justified.
19. Identify additional profitable strategy opportunities.
20. Allocate capital according to validated net opportunity and correlated risk.
21. Reconcile resulting financial outcomes.
22. Continuously monitor whether learned relationships remain effective.
23. Reject indicators and strategies that fail to provide incremental value.

The system must remain operational when individual intelligence sources are unavailable, subject to validated fallback and abstention policies.

The programme succeeds only where market intelligence demonstrates measurable incremental value over the completed v35 baseline.

The objective is not to collect every possible indicator.

The objective is to understand the market environment and trade more intelligently, profitably and safely.

**End of v36 Master Plan.**