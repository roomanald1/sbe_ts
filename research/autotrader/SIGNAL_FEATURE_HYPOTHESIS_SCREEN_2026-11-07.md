# Signal Feature Hypothesis Screen — 2026-11-07

## Result

Neither candidate showed positive net returns in this screen. Keep both as research hypotheses only; do not promote them to active strategies based on these results.

| Candidate / control | Horizon | Samples | Mean net return | Positive net rate |
| --- | ---: | ---: | ---: | ---: |
| Existing directional signal (baseline) | 60 s | 3,024 | −0.2555% | 1.2% |
| BTC 5m return confirms signal direction | 60 s | 1,574 | −0.2550% | 1.3% |
| BTC 5m return disagrees (control) | 60 s | 1,324 | −0.2555% | 0.9% |
| Existing directional signal (baseline) | 300 s | 1,046 | −0.2464% | 4.0% |
| BTC 5m return confirms signal direction | 300 s | 549 | −0.2567% | 4.7% |
| BTC 5m return disagrees (control) | 300 s | 459 | −0.2344% | 2.8% |
| Existing directional signal (baseline) | 900 s | 414 | −0.2306% | 10.6% |
| BTC 5m return confirms signal direction | 900 s | 204 | −0.2019% | 12.3% |
| BTC 5m return disagrees (control) | 900 s | 196 | −0.2713% | 8.2% |
| Perp crowding squeeze, BTC/ETH | 60 s | 18 | −0.1950% | 0.0% |
| Perp crowding squeeze, BTC/ETH | 300 s | 7 | −0.1918% | 0.0% |
| Perp crowding squeeze, BTC/ETH | 900 s | 3 | −0.1782% | 0.0% |

The BTC confirmation filter is approximately flat against the baseline at 60 seconds, weaker at 300 seconds, and less negative than its disagreement control at 900 seconds. That 900-second separation is a lead for a longer study, not a demonstrated edge: the measured mean remains negative, the period is short, and many symbols share the same BTC market moves. The perp squeeze sample is too small to judge and was negative at every horizon.

## Method

The reproducible screen is [signal_feature_hypothesis_screen.py](./signal_feature_hypothesis_screen.py). It reads the Insights `GET` endpoints only and writes [signal_feature_hypothesis_screen_results.json](./signal_feature_hypothesis_screen_results.json).

- Data window: **2026-11-07 07:32:36–08:55:19 UTC** signal timestamps.
- Fetched 134,576 evaluations, 50,000 research-feature rows, and 34,742 external-market rows.
- Evaluated 4,484 directionally actionable, non-overlapping samples, spaced by each symbol and horizon before strategy filtering.
- Outcomes use the database's long/short net-return labels, including its recorded quote and taker-fee assumptions.
- Feature joins use the latest record at or before the signal time, with maximum ages of 30 seconds for research features and 120 seconds for external-market features.
- BTC confirmation follows the existing evaluation signal only when BTC's 5-minute return has the same sign. The disagreement cohort is included as a control.
- Perp squeeze is restricted to BTC-USDT and ETH-USDT. It fades when funding and perp basis have the same nonzero sign while open interest is increasing.

## Caveats and next step

This is a short, recent window rather than a market-cycle validation. Samples remain cross-correlated across symbols, and database execution labels may not match live fills. The perp feature feed covers only BTC and ETH, yielding just 28 squeeze observations total. The positive-net confidence intervals and detailed horizon breakdowns are in the JSON output.

The next useful test is a multi-regime replay with non-overlapping portfolio-level exposure, fees and spread retained, and a held-out time period. The BTC confirmation idea is the better one to carry forward, limited to testing the 900-second horizon; the perp squeeze hypothesis needs substantially more observations before further tuning.
