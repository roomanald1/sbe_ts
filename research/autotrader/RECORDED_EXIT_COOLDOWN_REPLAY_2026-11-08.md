# Recorded exit and cooldown screen — 8 November 2026

## Result

Neither screen is sufficient to change the live stop or cooldown. The wider-stop results differ across days, and the cooldown sample is small. This is discovery evidence: both days have now been inspected and are not an untouched holdout.

The [replay](./recorded_exit_cooldown_replay.py) read the running Insights and Positions SQLite databases on 8 November at 19:42 UTC, with the latest included close at 19:00 UTC. The [saved results](./recorded_exit_cooldown_results_2026-11-08.json) include counts and exclusions. USDT and USDC are treated as USD at par.

## Wider stop screen

For the same stop-loss entries, compare the recorded exit with a 0.75% or 1.0% net stop and the recorded target. If neither alternative barrier is observed, exit at the last recorded quote about 30 minutes after the actual stop. Entry fees are the recorded fees; exit fees use the recorded stop fill's effective fee rate, applied to the alternative exit price. Longs exit at the sampled bid and shorts at the sampled ask. Each actual stop fill was matched by order ID, symbol, side, quantity, price and fee to the Positions ledger. The reconstructed original P&L and return must agree with the audit.

| Exit day UTC | Matched stops | Actual net | 0.75% net stop | 1.0% net stop |
| --- | ---: | ---: | ---: | ---: |
| 7 November | 15 | −$3.455 | −$3.499 | −$3.460 |
| 8 November | 38 | −$9.146 | −$7.740 | −$7.765 |

The 0.75% alternative kept these positions open an average of 24.6 and 20.1 additional minutes on the respective days, using about 16,034 and 36,830 additional dollar-minutes of notional. This is not peak concurrent capital or a portfolio simulation. The two databases are read at separate times while services are running, rather than from a cross-service atomic snapshot.

The screen excludes 21 stops without a post-exit record, 11 with overlapping same-strategy and symbol re-entry, and 7 with incomplete paths or insufficient fill/fee consistency. Quotes are sampled, and gaps up to two minutes are allowed. Stops or targets can be crossed between samples; queue position, market impact, latency, adverse selection, alternative strategy exits and funding are not reconstructed. The 30-minute forced exit is an assumed policy, not the current strategy exit. The replay uses closing audit P&L on matched single-quantity closes; it does not reconstruct a general partial-fill portfolio.

## Cooldown screen

For each retained closed trade, check whether its entry followed an observed losing stop for the same strategy and symbol within the candidate interval. Report that trade's actual net P&L as the P&L that would have been skipped; negative skipped P&L is an apparent saving. No observed trades fell inside five minutes. These are audit-based screens, not ledger-verified matched round trips or a recursive portfolio replay. In particular, the screen still uses a later observed stop even if the candidate policy would have skipped the trade that generated it. It does not model released capital, replacement trades, changed fills or portfolio limits.

| Interval after losing stop | 7 November skipped | 8 November skipped |
| --- | --- | --- |
| 15 minutes | 5 trades, −$0.447 combined; 2 wins | 2 trades, −$0.398 combined; 0 wins |
| 30 minutes | 10 trades, −$0.667 combined; 5 wins | 5 trades, −$0.500 combined; 2 wins |

Both longer intervals avoided net losses in this small, correlated observed sample, while also skipping winners. The sample cannot establish a reliable new cooldown. No strategy parameter or running service was changed.

## Reproduction

From the repository root, with the Docker volumes available, run:

```bash
docker run --rm --network none --user 0 \
  -v sbe-insights-data:/insights \
  -v sbe-position-management-data:/positions \
  -v "$PWD/research/autotrader:/research:ro" \
  sbe-infra-app-exporter:latest python3 /research/recorded_exit_cooldown_replay.py \
  --insights-db /insights/signals.sqlite --positions-db /positions/positions.sqlite
```

The volume mounts are writable so SQLite can access the live WAL and shared-memory files; both connections use read-only mode. Results can change as retained data changes. Evaluate any chosen policy on a later, untouched interval with denser quotes, explicit exposure and a recursively simulated cooldown before promotion.
