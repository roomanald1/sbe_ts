# Passive-entry paper replay

**Capture window:** 23 October 2026, approximately 09:50–10:59 UTC for labeled entries  
**Data source:** active paper Insights database and its live evaluation/book-snapshot API  
**Scope:** directional maker-entry scenarios; no orders were placed

## Result

Passive entry reduced the simulated losses versus taker entry, but did not produce positive net returns. Even the optimistic scenario that assumes every resting order fills lost on each horizon's chronological holdout. Requiring the observed price path to trade through the resting price also stayed negative.

## Method

For each directional evaluation, post a buy at the recorded best bid for a bullish signal or a sell at the best ask for a bearish signal. Exit at the recorded horizon using the exit bid (long) or exit ask (short), which assumes a taker exit. Use the exact entry and exit quotes stored in each evaluation. Keep at most one overlapping entry per symbol and horizon. Split the chronological sample 70/30 per horizon.

Fees are a scenario: 0.08% maker on entry plus 0.10% taker on exit, the published OKX regular spot rates for standard pairs. The account-specific rate can differ by tier and pair; OKX directs users to the authenticated fee-rate endpoint or account fee display for their applicable rate. [OKX fee rules](https://www.okx.com/en-us/help/trading-fee-rules-faq) · [OKX fee-rate API](https://www.okx.com/docs-v5/)

Two fill assumptions were measured:

- **Assume filled:** every resting best-price order fills. This is an optimistic upper bound on fill frequency, not a realistic fill model.
- **Trade-through proxy:** count a long fill only when the recorded path low moves strictly below the posted bid, or a short fill when the path high moves strictly above the posted ask. Path extrema don't reveal queue position, cancellation order, or exact trade sequence, so this remains a proxy rather than proof of execution.

## Results

Returns are mean net percent per filled trade. Holdout is the final 30% of each horizon's timestamp-ordered, non-overlapping sample.

| Hold | Taker baseline, holdout (N / mean) | Maker assumes fill, holdout (N / mean) | Maker trade-through proxy, holdout (fills / mean) |
|---:|---:|---:|---:|
| 1 min | 122 / −0.269% | 122 / −0.168% | 70 / −0.209% |
| 5 min | 51 / −0.234% | 51 / −0.153% | 33 / −0.192% |
| 15 min | 28 / −0.400% | 28 / −0.307% | 20 / −0.413% |
| 60 min | 27 / −0.215% | 27 / −0.121% | 25 / −0.192% |

The optimistic maker assumption improves the taker baseline by about 8–9 bp per trade, largely from entering at the passive side of the spread and paying a lower modeled entry fee. The observed trade-through fills are worse than the fill-all scenario, consistent with passive orders being filled during adverse price movement. Neither approach covers queue priority, partial fills, cancellation latency, or a passive exit.

## Decision

This capture does not support activating maker execution as a profitable strategy. A full market-making test needs sequenced trade and L2 updates, account-specific maker fees, queue-aware partial-fill simulation, inventory limits, and marked-to-market risk while inventory is open. The current paper capture's snapshots are too sparse (roughly 10-second intervals per symbol) to reconstruct queue events reliably.

No AutoTrader settings or live order behavior were changed. The experiment used the paper-only runtime database at research/autotrader/runtime/cross_asset_paper/signals.sqlite; its parent collector remains separate from live trading.
