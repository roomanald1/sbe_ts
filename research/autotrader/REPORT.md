# Historical model review — 19 October 2026

## Result

No profitable model was validated in this search. Some rules made money in training but failed in validation. The selected rule also lost money in the final holdout. This does not prove every possible model would fail.

No live strategy changes or service restarts were made. The five-minute review loop remains stopped.

## Data and method

Today contains 92,850 recorded outcomes. Only 11,842 include execution costs; 11,787 also pass snapshot/quality checks, across 31 symbols. The 57,402 neutral and 17,764 ranging outcomes have no recorded costs. Another 5,674 directional outcomes lack costs.

London-time partitions: 09:04:44–15:08:16 training, 15:08:16–18:10:01 validation, 18:10:01–21:11:47 final evaluation. Outcomes crossing boundaries are excluded. These are elapsed-time splits, not equal sample counts.

The search considered 1,032 parameter combinations across trend, momentum, flow, book imbalance, volatility, VWAP deviation, pressure and aggressor cost; following and fading; and 1-, 5-, 15- and 60-minute holds. Some combinations duplicate each other. Only 304 met the minimum training sample size. Training selected a shortlist; validation selected one frozen rule for the final evaluation.

Only one position per symbol is permitted. Portfolio capital limits are not simulated. Same-direction net returns are recorded; opposite-direction costs are approximated. See [method and limitations](README.md).

## Selected candidate: fade the signal for 15 minutes

Take the opposite side when absolute signal score is at least 0.20009. This is the best validation-ranked member of the training shortlist, not a recommendation. Validation was itself slightly negative.

| Period | Trades | Win rate | Mean net return | Hypothetical P&L, USD at 100 per entry |
|---|---:|---:|---:|---:|
| Training | 59 | 23.7% | -0.1266% | -7.47 |
| Validation | 28 | 53.6% | -0.0009% | -0.02 |
| Final holdout | 217 | 20.3% | -0.1895% | -41.13 |

On those same 217 final-period entries, following the original signal returned -56.03 USD; fading returned -41.13 USD. Improvement is not profitability. All 217 reversed entries use a cost proxy. Hypothetical drawdown was 41.19 USD at 100 per entry, before capital constraints. These dollars are not comparable directly with actual Auto Trader P&L.

## Training profits that failed

| Rule | Training trades | Training mean net | Validation trades | Validation mean net |
|---|---:|---:|---:|---:|
| Short for 15 minutes when volatility exceeds approximately 0.0499% | 40 | +0.0602% | 23 | -0.8403% |
| Fade five-minute trend, hold five minutes, volatility at least 0.1% | 32 | +0.0652% | 52 | -0.2380% |

These illustrate why choosing rules from their best historical result would be misleading.

## Were profitable moves available?

Yes, in hindsight. The diagnostic below assumes perfect direction selection and compares absolute market return with a nonnegative cost proxy on a fixed nonoverlapping sample. It is not an achievable trading strategy or an exact executable-profit bound.

| Horizon | Sample size | Median absolute move | Median recorded cost | Moves exceeding cost |
|---|---:|---:|---:|---:|
| 1 minute | 3,192 | 0.050% | 0.210% | 7.8% |
| 5 minutes | 1,056 | 0.104% | 0.209% | 27.4% |
| 15 minutes | 336 | 0.146% | 0.208% | 35.4% |
| 60 minutes | 41 | 0.140% | 0.212% | 29.3% |

Typical short-horizon moves were smaller than transaction costs. Longer holds offer more room, but 60-minute evidence is sparse: only 41 nonoverlapping observations and zero eligible validation trades in the baseline split. That cannot support model selection.

## Data issues and next model changes

1. Record executable long AND short outcomes for every snapshot, including neutral and ranging cases. Most of today's opportunity universe cannot currently be compared with recorded costs.
2. Store entry/exit bid and ask, fee rates and conventional entry-notional P&L separately. The current short formula uses entry/exit minus one; it differs from conventional short P&L and distorts derived execution costs. There are 55 negative derived costs in the matched sample. Opposite-side replay remains approximate.
3. Train a multivariate model to predict expected return after costs, with a no-trade choice. Predict move size as well as direction; win rate alone is insufficient.
4. Evaluate 15–60 minute decisions over additional days with realistic capital constraints. Use an untouched future period; this holdout has now been inspected and must not become a target for repeated tuning.
5. Record price paths and deep-book features at decision time before testing alternative stops, targets or liquidity-wall signals. These cannot be reconstructed reliably from existing snapshots.

These are proposed changes, not deployed changes. This study covers interpretable rule families, not all possible models. It supplies no evidence to justify increasing risk.

## Files

- [Method and reproduction](README.md)
- [Replay implementation](replay.py)
- [Replay results and source SHA-256](results.json)
- [Coverage audit](coverage.json)
