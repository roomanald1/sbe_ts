# Offline Auto Trader study — 19 October 2026

## Reproduction

Run python3 research/autotrader/replay.py and python3 research/autotrader/coverage.py from the repository root. No app or services are needed.

The replay reads the input once, decodes gzip/bincode, validates every record length, and records its SHA-256 in [results.json](results.json). Reproduction requires the same input hash; an appended recording is a different dataset. Research date: 19 October 2026, Europe/London.

## Method

Join snapshots and outcomes by symbol and timestamp. Exclude stale snapshots, missing costs/returns and nonfinite results. Split elapsed recorded time into 50% training, 25% validation and 25% final evaluation; purge outcomes crossing partition boundaries.

Search 1-, 5-, 15- and 60-minute holding periods, 11 features, follow/fade directions, training-derived magnitude thresholds and volatility gates. Some combinations duplicate each other. Require 30 training trades. Rank by mean minus descriptive standard error; shortlist 20 rules using training alone. Choose one using validation alone, requiring 15 trades. Evaluate that frozen choice on the final period. Standard errors are ranking heuristics, not confidence bounds; symbols and market periods are correlated.

Only one simultaneous trade per symbol is allowed, closing at the recorded evaluation time. Capital limits and maximum portfolio positions are not imposed. This is an entry-rule study, not an executable portfolio backtest. Dollars at 00 per entry equal the sum of percentage returns, without compounding, and cannot be directly compared to actual Auto Trader P&L.

Same-direction results use recorded net returns. Reversed trades reuse the recorded execution-cost estimate and are approximate. Short returns in the signal engine use entry/exit minus one, differing from conventional entry-notional short P&L. This also affects derived cost estimates; some are negative. Historical costs are retained rather than silently changed.

## Limits

The search covers interpretable single-feature rules with a volatility filter, not every multivariate model. Failure does not prove no profitable model exists. One day cannot establish a durable edge.

Many outcomes lack executable net returns/costs. Neutral signals therefore cannot be assessed with the same cost evidence. Snapshots lack full entry/exit bid/ask and continuous price paths. Alternative stop/target touch order, queue position, partial fills, market impact, borrowing and financing cannot be reliably reconstructed. Longer horizons have sparse, uneven temporal coverage. Late-added deep-book telemetry cannot establish historical predictive value.

The signal cost code defaults to 0.1% per side, plus bid/ask execution effects. Historical fee settings are not stored, so the analysis uses recorded costs rather than asserting that default applied throughout.
