# Edge search: retained signal outcomes

**Assessment date:** 22 October 2026  
**Scope:** Read-only replay of recorded executable signal evaluations. No trading rules or runtime state were changed.

## Finding

No repeatable profitable rule is supported by the retained data. The clearest candidate worth another paper replay is a 15-minute fade of stronger directional scores, but it loses on the first session and is inconclusive on the next session after accounting for outliers. Do not promote it to live use on this evidence.

## Data and method

Decoded evaluation payloads from the SQLite signal_records table and used their recorded long/short net-return fields, which include the stored spread/fee assumptions. Evaluations were grouped by 60, 300 and 900 second horizons. To reduce overlap, retained at most one evaluation per symbol per horizon interval, then split chronologically at UTC midnight: 21 October for the earlier session and 22 October for the later session. The extracted evaluation signal timestamps ran from 21 October 19:40:04.999 through 22 October 00:14:11.258 UTC; although the database contains more recent feed records, later outcome labels were not available in this evaluation extract.

For the score-fade candidate, a positive score means short and a negative score means long; score magnitude floors were fixed at 0, 0.2 and 0.4. These are descriptive checks, not a broad search over optimized thresholds. Returns below are arithmetic mean and median of recorded net percent returns per selected evaluation. Tail sensitivity reports the mean after removing the single best observation in that slice.

## Results: fade the score

| Horizon | Session | Score floor | N | Mean net return | Median | Positive outcomes | Mean without best |
|---|---|---:|---:|---:|---:|---:|---:|
| 1 min | 21 Oct | 0.0 | 23,141 | -0.282% | -0.235% | 2.3% | -0.282% |
| 1 min | 21 Oct | 0.2 | 4,854 | -0.255% | -0.227% | 3.6% | -0.257% |
| 1 min | 22 Oct | 0.2 | 292 | -0.265% | -0.242% | 5.8% | -0.270% |
| 5 min | 21 Oct | 0.2 | 1,027 | -0.250% | -0.224% | 12.3% | -0.256% |
| 5 min | 22 Oct | 0.2 | 60 | -0.209% | -0.262% | 15.0% | -0.264% |
| 15 min | 21 Oct | 0.2 | 398 | -0.229% | -0.227% | 20.4% | -0.246% |
| 15 min | 22 Oct | 0.2 | 22 | -0.033% | +0.066% | 54.5% | -0.090% |
| 15 min | 21 Oct | 0.4 | 114 | -0.226% | -0.215% | 27.2% | -0.287% |
| 15 min | 22 Oct | 0.4 | 2 | +0.117% | +0.117% | 100.0% | +0.007% |

The apparent improvement for 15-minute fades on 22 October is not robust: the 0.2 slice has only 22 observations and is negative after dropping its best observation; the 0.4 slice has only two observations. The earlier session is negative at both thresholds. All tested 1- and 5-minute slices are negative in both sessions.

## Interpretation and next evidence needed

Recorded costs erase the small directional returns in these slices. The 15-minute score fade is a hypothesis for continued shadow collection only, not a validated edge. The prior independent rule review also found no profitable chronological holdout, and the external-market joins from the 21 October evening window showed mixed, non-replicating relationships; see [REPORT.md](./REPORT.md) and [YESTERDAY_EXTERNAL_TEST.md](./YESTERDAY_EXTERNAL_TEST.md).

To test a real candidate, retain more days of synchronized, as-of external features and signal labels, then freeze one rule before a later untouched evaluation period. Require non-overlapping trades, recorded execution costs, adequate symbol and day diversity, and positive net returns across multiple chronological holdouts. This dataset currently does not provide enough independent sessions to establish that.

## Expanded sample note

A later extraction added labeled evaluations through 22 October 06:02 UTC. It included a paper-motivated BTC-to-altcoin cross-return proxy; that test and further research sources are documented in [RESEARCH_LEADS.md](./RESEARCH_LEADS.md). The tables above remain the original, shorter score-fade scan.
