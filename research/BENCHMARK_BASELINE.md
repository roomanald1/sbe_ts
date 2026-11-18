# Rust benchmark baseline

## Require ten samples before Scout parameter promotion (2026-11-18)

Learning passed 65 tests. The Learning service benchmark was repeated twice with
the existing workload (1,000 variants, 200 active profiles, 5,000 event IDs):
database load measured 31.92–35.79 ms, decision snapshots 0.21–0.31 ms, and
state persistence 18.72–19.12 ms. Compared with the latest same-workload result
on 2026-11-11 (2.32 / 0.11 / 6.38 ms), each metric was slower in both runs
(load +1,276–1,443%, snapshot +91–182%, persistence +193–200%). The benchmark
has no Scout outcome history and does not exercise sample qualification or
promotion, so these differences cannot be attributed to this change. The
repeated high readings indicate an environment or runtime difference worth
investigating; no target failed or was skipped. WSL2 Linux x86_64, optimized
Cargo bench profile.

## Require minimum strength for live strategy entries (2026-11-18)

Strategy passed 172 service-feature library tests. The Scout registration replay
benchmark measured 16,364 us for 10,000 active registrations, 60 replay
intervals, and 500 transient failures (10,500 encoded requests), 13.0% below the
latest comparable run on 2026-11-14 (18,803 us). This benchmark does not exercise
live strategy evaluation or the signal-strength gate, so the timing change is
not attributable to this work. WSL2 Linux x86_64, optimized Cargo profile.

## Scale entry notional by signal strength (2026-11-16)

The new bounded strength-to-size test passed. The full AutoTrader suite ran 168
library tests: 165 passed and the same 3 existing tests failed in live-scout
direction, signal-execution budget, and strategy-pause snapshot behavior. The
strategy-evaluation benchmark measured maximum entry / position evaluation at
18.1 / 5.4 ns; signal serialization / market-depth / order-request / parameter
validation at 545.6 / 626.6 / 422.9 / 2.4 ns; and settings snapshots at 0 / 256
/ 10,000 retained orders at 13.186 / 75.891 / 2,957.708 us. Compared with the
preceding same-day run (18.4 / 4.4 ns; helpers 544.9 / 588.1 / 423.3 / 2.0 ns;
snapshots 9.526 / 66.734 / 2,871.748 us), position evaluation increased
22.7%, market-depth assessment 6.5%, parameter validation 20.0%, and settings
snapshots 3.0% to 38.4%; other measurements changed by -1.6% to +0.1%. This
benchmark does not exercise the strength-to-size calculation, so the increases
cannot be attributed to it. Workloads were the same; runtime load can affect
these microbenchmarks. WSL2 Linux x86_64, optimized Cargo profile.

## Audit the final outcome of selected signal entries (2026-11-16)

The new AutoTrader regression test for an exploratory selection followed by a
downstream skip passed. The full AutoTrader suite ran 167 library tests: 164
passed and 3 unrelated existing tests failed in live-scout direction,
signal-execution budget, and strategy-pause snapshot behavior. The strategy-
evaluation benchmark measured maximum entry / position evaluation at 18.4 / 4.4
ns; signal serialization / market-depth / order-request / parameter validation
at 544.9 / 588.1 / 423.3 / 2.0 ns; and settings snapshots at 0 / 256 / 10,000
retained orders at 9.526 / 66.734 / 2,871.748 us. Compared with the latest
comparable run (21.7 / 4.8 ns; helpers 575.2 / 677.1 / 433.4 / 2.0 ns;
snapshots 11.038 / 72.192 / 3,479.812 us), all measurements were unchanged or
lower by 2.3% to 17.5%. This benchmark does not exercise audit persistence or
decision payload construction; results are environment-sensitive. WSL2 Linux
x86_64, optimized Cargo profile.

## Stream scout assignments and cache Learning updates in workers (2026-11-14)

Strategy passed 172 service-feature library tests; Learning passed 62 library
tests and 1 binary test. The Strategy registration replay benchmark measured
26,652 us for 10,000 active registrations, 60 replay intervals, and 500
transient failures (10,500 encoded requests), 41.7% above the latest comparable
run (18,803 us). The benchmark does not exercise Learning broadcasts or Scout
subscriptions and ran while Learning tests were compiling/running, so this
contended result cannot be attributed to the stream changes. WSL2 Linux x86_64,
optimized Cargo profile.

## Attach a reason to every strategy rejection (2026-11-14)

Strategy passed 71 library tests; AutoTrader passed 168 library and 8 binary
tests. The Strategy service-feature bench completed with all 172 harness tests
ignored and measured Scout registration replay at 18,803 us for 10,000 active
registrations, 60 intervals, and 500 transient failures (10,500 encoded
requests), 6.3% above the latest comparable run (17,684 us). This workload does
not exercise rejection explanations. The AutoTrader strategy-evaluation bench
measured maximum entry / position evaluation at 21.7 / 4.8 ns; serialization /
depth / order construction / parameter validation at 575.2 / 677.1 / 433.4 /
2.0 ns; and settings snapshots at 0 / 256 / 10,000 retained orders at 11.038 /
72.192 / 3,479.812 us. Compared with the latest comparable run (21.8 / 5.4 ns;
helpers 642.2 / 752.1 / 467.5 / 2.5 ns; snapshots 14.970 / 94.447 / 4,056.418
us), all measurements decreased by 0.5% to 26.3%. These workloads do not
exercise reason generation; results are environment-sensitive. WSL2 Linux
x86_64, optimized Cargo profiles.

## Report worker route failures and back off during AutoTrader timeouts (2026-11-13)

Strategy passed 169 service-feature library tests; AutoTrader passed 166 library
and 8 binary tests. The Scout registration replay benchmark measured 17,684 us
for 10,000 registrations, 60 intervals, and 500 transient failures (10,500
encoded requests), 8.6% above the latest comparable run (16,283 us). The replay
workload does not exercise live decision callbacks or profile-query backoff;
this active-stack run overlapped repeated AutoTrader route timeouts and is
environment-contended, so the timing cannot be attributed to the change.
WSL2 Linux x86_64, optimized Cargo profiles.

The AutoTrader strategy-evaluation benchmark measured entry / position maxima
of 21.8 / 5.4 ns; signal serialization / market-depth / order-request /
parameter validation measured 642.2 / 752.1 / 467.5 / 2.5 ns; settings
snapshots at 0 / 256 / 10,000 orders measured 14.970 / 94.447 / 4,056.418 us.
Compared with the latest same-day run (20.0 / 6.2 ns; helpers 569.0 / 633.7 /
440.4 / 2.2 ns; snapshots 16.446 / 72.259 / 2,877.191 us), values changed
+9.0% / -12.9%, helpers +12.9% / +18.7% / +6.2% / +13.6%, and snapshots -9.0%
/ +30.7% / +41.0%. This microbenchmark does not cover persistence coalescing;
results ran during active strategy-worker route timeouts and are
environment-contended. WSL2 Linux x86_64, optimized Cargo profile.

## Back off failed live worker routes and expose report health (2026-11-13)

Strategy passed 169 service-feature library tests; AutoTrader passed 166 library
and 8 binary tests. The Scout registration replay benchmark measured 14,725 us
for 10,000 registrations, 60 intervals, and 500 transient failures (10,500
encoded requests), 35.8% above the latest comparable run (10,843 us). This
synthetic Scout workload does not exercise live Insights report retries or
profile refresh; the stack was active and AutoTrader/Insights were resource
contended, so the movement is not attributable to the live-worker changes.
WSL2 Linux x86_64, optimized Cargo profiles.

The AutoTrader strategy-evaluation benchmark measured entry / position maxima
of 20.0 / 6.2 ns; signal serialization / market-depth / order-request /
parameter validation measured 569.0 / 633.7 / 440.4 / 2.2 ns; settings
snapshots at 0 / 256 / 10,000 orders measured 16.446 / 72.259 / 2,877.191 us.
Compared with the preceding same-day run (25.0 / 6.1 ns; helpers 639.1 / 727.5
/ 465.0 / 2.8 ns; snapshots 15.745 / 82.913 / 3,347.611 us), values changed
-20.0% / +1.6%, helpers -11.0% / -12.9% / -5.3% / -21.4%, and snapshots +4.5%
/ -12.8% / -14.1%. This microbenchmark does not exercise persistence
coalescing; run under active system load, so timing differences remain
environment-sensitive. WSL2 Linux x86_64, optimized Cargo profile.

## Count live strategy rejections in the Reasons summary (2026-11-13)

AutoTrader passed 166 library tests and 8 binary tests. The strategy-evaluation
benchmark measured entry / position maxima of 25.0 / 6.1 ns; signal
serialization / market-depth / order-request / parameter validation measured
639.1 / 727.5 / 465.0 / 2.8 ns; settings snapshots at 0 / 256 / 10,000
orders measured 15.745 / 82.913 / 3,347.611 us. Compared with the latest
same-day run (20.3 / 5.4 ns; helpers 604.7 / 696.1 / 447.7 / 2.3 ns;
snapshots 14.071 / 89.538 / 3,517.862 us), values changed +23.2% / +13.0%,
helpers +5.7% / +4.5% / +3.9% / +21.7%, and snapshots +11.9% / -7.4% / -4.8%.
This benchmark does not exercise worker-rejection accounting or the signal
queue; results ran under active stack load and are environment-sensitive.
WSL2 Linux x86_64, optimized Cargo profile.

## Coalesce signal reports before bounded parallel cycle evaluation (2026-11-13)

AutoTrader passed 166 library tests and 8 binary tests. The strategy-evaluation
benchmark measured entry / position maxima of 20.3 / 5.4 ns; signal
serialization / market-depth / order-request / parameter validation measured
604.7 / 696.1 / 447.7 / 2.3 ns; settings snapshots at 0 / 256 / 10,000
orders measured 14.071 / 89.538 / 3,517.862 us. Compared with the latest
comparable same-day run (18.4 / 3.6 ns; helpers 547.4 / 586.9 / 454.3 / 2.1
ns; snapshots 10.007 / 71.846 / 2,538.766 us), values changed +10.3% / +50.0%,
helpers +10.5% / +18.6% / -1.5% / +9.5%, and snapshots +40.6% / +24.6% /
+38.6%. These microbenchmarks do not exercise signal batching or cycle
scheduling. They ran with the active stack under heavy AutoTrader and Insights
load, so the increases are environment-contended and are not evidence of a
batching regression. WSL2 Linux x86_64, optimized Cargo profile.

## Dequeue only after live-worker capacity is available (2026-11-13)

Strategy passed 168 service-feature library tests. The Scout registration replay
benchmark measured a 10,843 us median for 10,000 registrations, 60 intervals,
and 500 transient failures (10,500 encoded requests), 10.1% below the prior
same-workload run (12,065 us). This change bounds live-worker update processing;
the benchmark does not exercise that path, so the movement is environment
variation rather than a measured effect. WSL2 Linux x86_64, optimized Cargo
profile, active stack.

## Match failed Learning evidence cooldown to the evidence refresh interval (2026-11-13)

Strategy passed 168 service-feature library tests. The Scout registration replay
benchmark measured a 12,065 us median for 10,000 registrations, 60 intervals,
and 500 transient failures (10,500 encoded requests), 16.1% below the preceding
same-workload run (14,386 us). The evidence cooldown does not affect registration
replay; this result is environment-sensitive and does not measure live request
volume. WSL2 Linux x86_64, optimized Cargo profile, active stack.

## Align live signal queue expiry with the entry freshness window (2026-11-13)

AutoTrader passed 166 library tests and 8 binary tests. The strategy-evaluation
benchmark completed: evaluator maxima were 18.4 / 3.6 ns; serialization / depth
/ order / parameter helpers measured 547.4 / 586.9 / 454.3 / 2.1 ns; settings
snapshots at 0 / 256 / 10,000 retained orders measured 10.007 / 71.846 /
2,538.766 us. Compared with the preceding same-day run, evaluator maxima
changed -16.4% / -33.3%, helpers -2.1% / -1.5% / +6.5% / +5.0%, and snapshots
+1.6% / +6.6% / -7.3%. These microbenchmarks do not exercise the live queue
expiry or cycle backlog; changes are environment-sensitive. WSL2 Linux x86_64,
optimized Cargo profile, active stack.

## Keep latest per-symbol worker inputs without stream backpressure (2026-11-13)

Strategy passed 168 service-feature library tests. The Scout registration replay
benchmark measured a 14,386 us median for 10,000 registrations, 60 intervals,
and 500 transient failures (10,500 encoded requests), 11.9% above the previous
same-workload run (12,862 us). This benchmark does not exercise the live
Insights worker stream or the pending-update map; the difference is unrelated
and environment-sensitive. WSL2 Linux x86_64, optimized Cargo profile, active
stack.

## Bound repeated failures while loading live profile evidence (2026-11-13)

Strategy passed 168 service-feature library tests. The Scout registration replay
benchmark measured a 12,862 us median for 10,000 registrations, 60 intervals,
and 500 transient failures (10,500 encoded requests), 9.4% below the previous
same-workload run (14,193 us). The live evidence retry cooldown does not affect
registration replay; this result is environment-sensitive and does not measure
evidence request volume. WSL2 Linux x86_64, optimized Cargo profile, active
stack.

## Back off repeated Learning evidence failures (2026-11-13)

Strategy passed 168 service-feature library tests. The Scout registration replay
benchmark measured a 14,193 us median for 10,000 registrations, 60 intervals,
and 500 transient failures (10,500 encoded requests), 3.1% above the prior
same-workload run (13,769 us). This change affects live evidence lookup
backoff, not registration replay; the small difference is environment-sensitive
and the benchmark does not measure evidence request volume. WSL2 Linux x86_64,
optimized Cargo profile, active stack.

## Coalesce queued live Insights snapshots by symbol (2026-11-13)

Strategy passed 168 service-feature library tests. The Scout registration replay
benchmark completed with a 13,769 us median for 10,000 registrations, 60
intervals, and 500 transient failures (10,500 encoded requests). Compared with
the preceding same-workload run (14,986 us), this is 8.1% faster. The queue
coalescing change does not affect the timed registration replay workload; the
result is environment-sensitive and no live stream timing is measured by this
benchmark. WSL2 Linux x86_64, optimized Cargo profile, active stack.

## Gate live Learning evidence lookups behind the local entry rule (2026-11-13)

Strategy passed 167 service-feature library tests. `cargo bench --features
service` completed with all 167 harness tests ignored and ran the Scout
registration replay workload: 10,000 registrations, 60 intervals, and 500
transient failures encoded 10,500 requests in a 14,986 us median. Compared with
the latest same-workload result (13,349 us), this is 12.3% slower; this change
does not touch registration replay, and the run overlapped active service
traffic. The benchmark does not measure live-worker evidence lookup frequency.
WSL2 Linux x86_64, optimized Cargo profile, active stack.

## Index shared exit IDs once during AutoTrader position reconciliation (2026-11-13)

AutoTrader passed 165 library tests and 8 binary tests. The strategy-evaluation
benchmark completed: evaluator maxima were 22.0 / 5.4 ns; serialization / depth
/ order / parameter helpers measured 559.0 / 595.6 / 426.6 / 2.0 ns; settings
snapshots at 0 / 256 / 10,000 retained orders measured 9.849 / 67.412 /
2,740.156 us. Compared with the latest comparable same-day run, evaluator maxima
changed +16.4% / +12.5%, helpers +2.3% / -1.1% / +0.7% / +5.3%, and snapshots
+2.5% / +5.6% / +4.1%. These microbenchmarks do not exercise position
reconciliation; the change avoids rescanning retained orders once per externally
closed position, but runtime lock timings require rebuilding the running
AutoTrader. No services were restarted. WSL2 Linux x86_64, optimized Cargo
profile, active stack.

## Allow normal live-signal processing latency (2026-11-13)

AutoTrader passed 165 library and 8 binary tests. The Live Signal freshness regression test accepts a 6-second-old live signal, rejects one older than 15 seconds, and still rejects an explicitly stale source. The AutoTrader benchmark completed with all 173 harness tests ignored. Entry / position evaluator maxima were 18.9 / 4.8 ns; signal serialization / market depth / order construction / parameter validation measured 546.5 / 602.4 / 423.5 / 1.9 ns. Settings snapshots at 0 / 256 / 10,000 orders measured 9.607 / 63.820 / 2,631.034 us. Compared with the latest comparable same-day run (18.1 / 3.6 ns; helpers 543.4 / 628.2 / 420.6 / 2.2 ns; snapshots 12.030 / 97.194 / 2,448.180 us), evaluator maxima changed +4.4% / +33.3%, helpers +0.6% / -4.1% / +0.7% / -13.6%, and snapshots -20.1% / -34.3% / +7.5%. This age-window change does not affect benchmarked evaluator or snapshot work; measurements ran with the active stack and are environment-sensitive. The freshness window changed from 5 to 15 seconds after runtime diagnostics showed live candidates rejected at 6 seconds while AutoTrader cycles took about 4 seconds. A pre-existing test assertion for daily-strategy priority was corrected to expect a non-exploratory selection. No services were restarted during these benchmark runs.

Follow-up after moving cycle-slot acquisition ahead of live-signal dequeue: AutoTrader passed 165 library and 8 binary tests. Entry / position evaluator maxima were 21.4 / 5.4 ns; serialization / depth / order / parameter helpers measured 582.1 / 621.2 / 431.3 / 2.1 ns; settings snapshots at 0 / 256 / 10,000 orders measured 10.168 / 75.374 / 2,746.931 us. Compared with the immediately preceding run, evaluator maxima changed +13.2% / +12.5%, helpers +6.5% / +3.1% / +1.8% / +10.5%, and snapshots +5.8% / +18.1% / +4.4%. This queue-ordering change does not affect these workloads; the run overlapped active stack traffic, so the movements are environment-sensitive. No services were restarted during this benchmark run.

Follow-up after expiring stale queued live dashboards: AutoTrader passed 166 library and 8 binary tests. The strategy-evaluation benchmark measured entry / position maxima of 19.0 / 5.6 ns; serialization / depth / order / parameter helpers measured 547.0 / 609.5 / 442.8 / 1.9 ns; settings snapshots at 0 / 256 / 10,000 orders measured 9.970 / 69.195 / 3,120.645 us. Compared with the latest same-day run (18.1 / 3.6 ns; helpers 543.4 / 628.2 / 420.6 / 2.2 ns; snapshots 12.030 / 97.194 / 2,448.180 us), maxima changed +5.0% / +55.6%, helpers +0.7% / -3.0% / +5.3% / -13.6%, and snapshots -17.1% / -28.8% / +27.5%. The queue-expiry change does not affect timed evaluator or settings-snapshot workloads; the 10,000-order snapshot spread was collected under active stack traffic and is environment-sensitive. No services were restarted during this benchmark run.

Follow-up benchmark after limiting queued live reports to an eight-second age: evaluator maxima were 23.4 / 4.8 ns; helpers 588.9 / 617.4 / 441.8 / 1.9 ns; settings snapshots 10.034 / 80.575 / 2,742.079 us. Compared with the prior queue-expiry run (19.0 / 5.6 ns; helpers 547.0 / 609.5 / 442.8 / 1.9 ns; snapshots 9.970 / 69.195 / 3,120.645 us), values changed +23.2% / -14.3%, helpers +7.7% / +1.3% / -0.2% / 0%, and snapshots +0.6% / +16.4% / -12.1%. These unchanged evaluator and snapshot workloads are highly variable under active stack traffic; this freshness filter adds no work to them.

Final follow-up after reserving cycle capacity before dequeue and limiting the live queue to six seconds: AutoTrader passed 166 library and 8 binary tests. Entry / position evaluator maxima were 19.0 / 3.9 ns; helpers 559.0 / 601.4 / 427.6 / 2.1 ns; settings snapshots 10.383 / 74.549 / 2,891.388 us. Compared with the preceding same-day run (23.4 / 4.8 ns; helpers 588.9 / 617.4 / 441.8 / 1.9 ns; snapshots 10.034 / 80.575 / 2,742.079 us), evaluator maxima changed -18.8% / -18.8%, helpers -5.1% / -2.6% / -3.2% / +10.5%, and snapshots +3.5% / -7.5% / +5.4%. The 15-second age window and queue policy do not affect these timed workloads; measurements remain environment-sensitive under active stack traffic.

After tightening the stale-live queue window to six seconds, AutoTrader again passed 166 library and 8 binary tests. The focused benchmark measured entry / position maxima of 17.8 / 3.5 ns; helpers 554.6 / 612.2 / 430.1 / 1.9 ns; settings snapshots 10.827 / 64.700 / 2,821.486 us. Compared with the preceding queue-window run (19.0 / 3.9 ns; helpers 559.0 / 601.4 / 427.6 / 2.1 ns; snapshots 10.383 / 74.549 / 2,891.388 us), values changed -6.3% / -10.3%, helpers -0.8% / +1.8% / +0.6% / -9.5%, and snapshots +4.3% / -13.2% / -2.4%. These measurements do not exercise queue expiry and vary under active stack traffic.

Final measured run after retaining queued live reports for at most two seconds and allowing 20 seconds end-to-end freshness: AutoTrader passed 166 library and 8 binary tests. Entry / position evaluator maxima were 23.6 / 6.5 ns; helpers 606.0 / 640.1 / 469.2 / 2.2 ns; settings snapshots 11.857 / 72.608 / 2,512.450 us. Compared with the prior six-second queue run (17.8 / 3.5 ns; helpers 554.6 / 612.2 / 430.1 / 1.9 ns; snapshots 10.827 / 64.700 / 2,821.486 us), maxima changed +32.6% / +85.7%, helpers +9.4% / +4.6% / +9.1% / +15.8%, and snapshots +9.5% / +12.4% / -11.0%. These workloads do not exercise live queue expiry or freshness behavior; the run overlapped active stack traffic and is highly variable.

Focused benchmark after rechecking live queue age once a cycle slot is available: AutoTrader passed 166 library and 8 binary tests. Entry / position evaluator maxima were 64.0 / 23.8 ns; helpers 804.9 / 1,078.4 / 462.0 / 2.3 ns; settings snapshots 13.627 / 96.201 / 6,248.014 us. Compared with the latest comparable same-day run (18.1 / 3.6 ns; helpers 543.4 / 628.2 / 420.6 / 2.2 ns; snapshots 12.030 / 97.194 / 2,448.180 us), entry / position maxima were +254% / +561%, helpers +48.2% / +71.7% / +9.8% / +4.5%, and snapshots +13.3% / -1.0% / +155%. This unchanged microbenchmark ran under heavy active stack load (AutoTrader sampled at 120% CPU and Learning near 4 GiB RSS) and does not exercise queue expiry; results are severely environment-contended. Do not attribute the increase to the queue policy without a controlled rerun.

## Route Scout markouts by their strategy (2026-11-13)

Insights passed 40 library and 46 binary tests; Learning passed 61 library and 1 binary test. Insights calculations measured 11,262.0 / 11,877.3 / 251.0 / 19.6 / 323.5 / 43.0 ns for live snapshot, explained snapshot, research features, return context, diagnostics, and book snapshot, compared with the latest comparable 13,423.1 / 13,051.6 / 256.1 / 21.1 / 351.0 / 45.3 ns. Changes were -16.1% / -9.0% / -2.0% / -7.1% / -7.8% / -5.1%. Feed-event measurements for on_price / on_trade / on_book / snapshot were 10,689.8 / 7.2 / 38.3 / 39,792.0 ns, versus 12,377.5 / 10.3 / 47.2 / 53,294.0 ns (-13.6% / -30.1% / -18.9% / -25.3%). Hot-path book encoding measured 6,400.5 ns and 192 allocations for legacy JSON, versus 225.9 ns and 6.714 allocations for compact caching. Learning load / decision snapshot / persistence measured 22.01 / 0.19 / 15.32 ms versus latest comparable 2.32 / 0.11 / 6.38 ms (+849% / +73% / +140%); this single run overlapped an active Insights markout backlog and is environment-contended, not attributable to routing metadata. Before the route fix, live diagnosis recorded 29,950 live Scout observations but no live markout deliveries, while 213 unrelated daily markouts delivered. After targeting each callback to its strategy and replaying active registrations, Insights logged more than 434 live markout deliveries and Prometheus aggregate closed count rose from 179,286 to 180,324 (+1,038). Live-trade-flow increased by 94 in the first short sample; other live strategy counts had not yet moved at that sample time. Homepage liveness passed 17/17 after the Insights and Scout worker restarts. These benchmarks do not measure RSocket fanout. WSL2 Linux x86_64, optimized Cargo profiles, active stack. No services were restarted during these benchmark runs.

## Reconnect idle live Scout streams and expose input freshness (2026-11-13)

Strategy passed 166 service-feature library tests. The service-feature bench command completed with all 166 harness tests ignored and ran the Scout registration replay workload: 10,000 registrations, 60 intervals, and 500 transient failures encoded 10,500 requests in a 13,349 us median. Compared with the latest same-workload result (21,275 us), this is 37.3% lower encoding time; request counts and workload are unchanged. This benchmark does not exercise the live Insights stream timeout. WSL2 Linux x86_64, optimized Cargo profile, with the live stack active. Prometheus showed all five live Scout input-freshness values stale during diagnosis. After the focused Scout worker rebuild and restart, homepage liveness passed 17/17 checks and all five live streams delivered fresh updates; the completed count was unchanged over the first 30 seconds, below the 15-minute minimum live Scout hold.

## Avoid pausing entries for replaceable audit evictions (2026-11-12)

AutoTrader passed 164 library tests and 7 binary tests. Entry / position
evaluator maxima measured 18.1 / 3.6 ns; serialization / depth / order /
parameter helpers measured 543.4 / 628.2 / 420.6 / 2.2 ns. Settings snapshots
at 0 / 256 / 10,000 retained orders measured 12.030 / 97.194 / 2,448.180 us.
Compared with the latest same-day AutoTrader run, evaluator maxima changed
+0.6% / 0%; helper timings changed +3.1% / -8.5% / +0.1% / +15.8%; snapshots
changed +23.1% / +46.5% / -0.1%. This audit queue policy change does not alter
those workloads; snapshot measurements varied under active stack traffic.
WSL2 Linux x86_64, optimized Cargo bench profile. No services were restarted.

## Verify the paired-baseline Learning outbox path (2026-11-12)

Strategy passed 164 service-feature library tests. `cargo bench --features
service` completed with all 164 harness tests ignored; Strategy has no timed
benchmark target. The new handler-level test confirms a paired baseline queues
its absolute outcome for the exact baseline profile, while paired comparison
feedback remains separate. No timed performance delta is available for this
Scout feedback change. WSL2 Linux x86_64, optimized Cargo bench profile. Scout
workers will be rebuilt and restarted; homepage liveness will be checked
afterward.

## Preserve paired-baseline outcomes for exact-profile Learning (2026-11-12)

Strategy passed 163 service-feature library tests. `cargo bench --features
service` completed with 163 harness tests ignored; Strategy has no timed
benchmark target. This change adds one independent Learning event for the
paired baseline's main strategy scenario while preserving the separate paired
delta event on the tested variant. It does not change evaluator hot paths, and
the available Strategy command provides no timing data for Scout feedback
dispatch. WSL2 Linux x86_64, optimized Cargo bench profile. Scout worker images
will be rebuilt and restarted; homepage liveness will be checked afterward.

## Log Learning profile registration failures (2026-11-12)

AutoTrader passed 164 library tests and 7 binary tests. Entry / position
evaluator maxima measured 18.0 / 3.6 ns; serialization / depth / order /
parameter helpers measured 527.1 / 686.8 / 420.1 / 1.9 ns. Settings snapshots
at 0 / 256 / 10,000 retained orders measured 9.772 / 66.358 / 2,451.252 us.
Compared with the preceding same-day run, evaluator maxima changed -5.8% / -34.5%;
helper values changed -13.6% to +8.5%; snapshots changed -8.5% / -6.8% / -9.5%.
The benchmark does not exercise profile registration or its new diagnostics;
single-run variation occurred under active stack traffic. WSL2 Linux x86_64,
optimized Cargo bench profile. Only AutoTrader will be rebuilt and restarted;
Learning is already running the updated route allowlist. Homepage liveness will
be checked after the restart.

## Allow profile registration over Learning RSocket (2026-11-12)

Learning passed 60 library tests and 1 binary test. The unchanged 1,000-variant /
200-profile / 5,000-event benchmark measured database load / decision snapshot /
state persistence at 19.28 / 0.17 / 13.58 ms. Compared with the immediately
preceding run (22.06 / 0.27 / 14.19 ms), these changed -12.6% / -37.0% / -4.3%.
Both runs used the live stack; the earlier run overlapped Docker builds, so the
timing movement is host-contention-sensitive and not attributable to the route
allowlist. This fixture does not measure RSocket route dispatch or registration.
WSL2 Linux x86_64, optimized Cargo bench profile. Learning will be restarted;
AutoTrader already runs the snapshot and queue changes. Homepage liveness will
be checked after the restart.

## Prevent missed-opportunity backlog from starving profile registration (2026-11-12)

AutoTrader passed 164 library tests and 7 binary tests. Entry / position
evaluator maxima measured 19.1 / 5.5 ns; serialization / depth / order /
parameter helpers measured 561.9 / 632.8 / 420.8 / 2.2 ns. Settings snapshots
at 0 / 256 / 10,000 retained orders measured 10.677 / 71.196 / 2,708.528 us.
Compared with the preceding same-day run, evaluator maxima changed -3.0% / +3.8%;
helpers were within 4.8%; snapshots changed +9.6% / -26.9% / +0.9%. The run was
under active stack load and does not benchmark feedback/registration queue
fairness. WSL2 Linux x86_64, optimized Cargo bench profile. Only AutoTrader was
restarted for this worker change; Learning was already running the updated
snapshot contract. Homepage liveness had passed 17/17 checks after the preceding
Learning and AutoTrader restarts.

## Register active Learning profiles for evidence collection (2026-11-12)

Strategy passed 162 service-feature library tests; Learning passed 59 library
and 1 binary test; AutoTrader passed 163 library and 7 binary tests. Learning's
1,000-variant / 200-profile / 5,000-event fixture measured database load /
decision snapshot / persistence at 22.06 / 0.27 / 14.19 ms. Versus the latest
same-workload result (8.32 / 0.12 / 6.78 ms), these were +165% / +125% / +109%.
This run had the live 50-container stack active and overlapped a focused Docker
rebuild, so the timing increase is environment-contended and not attributable
to the snapshot-contract change. AutoTrader entry / position evaluator maxima
were 19.7 / 5.3 ns; serialization / depth / order / parameter helpers measured
562.5 / 625.5 / 431.1 / 2.1 ns. Settings snapshots at 0 / 256 / 10,000
retained orders measured 9.737 / 97.377 / 2,685.412 us. Compared with the
immediately preceding same-day run, evaluator maxima changed -0.5% / -14.5%,
helpers were within 5%, and settings snapshots changed -7.1% / +44.1% / -10.6%.
The 256-order snapshot result was collected under the same active-stack load;
these microbenchmarks do not measure profile registration or evidence routing.
Strategy has no dedicated timing benchmark. WSL2 Linux x86_64, optimized Cargo
bench profiles. AutoTrader and Learning were restarted; homepage liveness passed
17/17 checks.

## Apply changed Learning profile identities (2026-11-12)

AutoTrader passed 161 library tests and 7 binary tests. The strategy evaluation
benchmark measured entry / position evaluation at 3.0–19.8 / 1.8–6.2 ns;
signal serialization / market depth / limit-order construction / parameter
validation measured 563.4 / 628.5 / 429.1 / 2.0 ns. Settings snapshots at 0 /
256 / 10,000 retained orders measured 10.482 / 67.581 / 3,002.987 us. Compared
with the latest same-day comparable run, evaluator maxima changed +4.8% / +12.7%;
helper timings were within 6.3%; settings snapshots changed -3.3% / -1.5% /
+19.8%. The benchmark does not exercise profile snapshot synchronization. It
ran with the local system stack active, so the largest retained-order snapshot
variation is environment-sensitive. WSL2 Linux x86_64, optimized Cargo bench
profile. No services were restarted.

## Bounded parallel insight cycles (2026-11-12)

AutoTrader passed 160 library tests and 7 binary tests. The strategy evaluation
benchmark measured live strategy entry at 10.6–18.9 ns and position evaluation
at 2.7–5.5 ns; signal serialization / market depth / limit-order construction /
parameter validation measured 550.2 / 589.1 / 421.2 / 2.0 ns. Settings snapshots
at 0 / 256 / 10,000 retained orders measured 10.838 / 68.637 / 2,507.392 us.
Compared with the latest same-day comparable run (entry 3.8–21.5 ns, position
2.0–3.9 ns, helpers 565.5 / 700.2 / 467.9 / 2.1 ns, and snapshots 12.829 /
86.293 / 3,874.467 us), helper timings improved 2.7–15.9%, settings snapshots
improved 20.4–35.3%, and common live-strategy position evaluations were within
1.3 ns; the slowest live-momentum case was 1.6 ns higher. These microbenchmarks
do not measure cycle queue latency or concurrent execution. WSL2 Linux x86_64,
optimized Cargo bench profile. No services were restarted.

## Prioritize trade learning feedback (2026-11-12)

AutoTrader passed 160 library tests and 7 binary tests. The strategy evaluation
benchmark measured entry evaluation at 3.8–21.5 ns and position evaluation at
2.0–3.9 ns; signal serialization / market depth / limit-order construction /
parameter validation measured 565.5 / 700.2 / 467.9 / 2.1 ns. Settings
snapshots at 0 / 256 / 10,000 retained orders measured 12.829 / 86.293 /
3,874.467 us. Compared with the latest comparable recorded run (entry 3.1–18.4
ns, position 1.8–4.3 ns, helpers 577.4 / 653.9 / 449.9 / 2.0 ns, and settings
snapshots 16.719 / 71.929 / 2,965.218 us), evaluator maxima changed +16.8% /
-9.3%, helper measurements varied by -2.1% to +7.1%, and settings snapshots
varied by -23.3% to +30.7%. These measurements do not exercise feedback queue
selection; the run occurred with the system stack active, so the unrelated
settings snapshot variation is environment-sensitive. WSL2 Linux x86_64,
optimized Cargo bench profile. No services were restarted.

## Insights hot-path benchmark methodology (2026-11-12)

Insights tests passed (40 library, 45 binary). The calculation harness now uses
one warmup and nine timed samples, reports median and range, uses Linux process
CPU time rather than wall time, and separately measures allocation calls and
requested bytes per operation. The hot-path harness compares per-book nested
JSON construction with compact numeric caching and JSON construction once per
14 updates. In the latest run, per-book JSON measured 6,819.9 ns/update, 192
allocations, and 40,607.9 requested bytes; compact caching measured 240.2
ns/update, 6.714 allocations, and 1,415.5 requested bytes. Across four focused runs,
compact caching was 25–30× faster in this microbenchmark and used about 96.5%
fewer allocations and requested bytes. The cache benchmark models the hot-path
representation and snapshot cadence; it is not an end-to-end service
measurement.

The calculation medians were 13,423.1 / 13,051.6 / 256.1 / 21.1 / 351.0 /
45.3 ns for live snapshot, explained snapshot, research features, return
context, diagnostics, and book snapshot. Their nine-sample ranges remained
wide under concurrent host load, so compare these CPU-time measurements only
with later runs using this updated harness and similar conditions. A price
update measured 15.3 / 2,376.4 / 4,532.6 / 9,079.4 / 18,087.7 ns with
0 / 100 / 200 / 400 / 800 pending Scout outcomes. The slope is about 22.6 ns
per pending outcome. A live three-second sample showed 477 price events/s, so
even 800 pending outcomes on every price event would account for about 8.6 ms
of CPU per second (0.009 core). This scan scales linearly but is too small to
explain the multi-core CPU chart. The six-million-row book-history reproduction
retained 20 MiB RSS in 0.33 s. WSL2 Linux x86_64, optimized Cargo profile. Live
`perf` profiling remains unavailable without matching WSL2 kernel tools. No
services were restarted.

## Defer cross-asset returns to research samples (2026-11-12)

Insights tests passed (40 library, 45 binary). Feed ingestion now calculates
BTC/ETH return context only when the current event produces a research sample;
same-symbol context still comes from that symbol's locked engine, and other
reference engines remain nonblocking. This removes two engine-map lookups and
return-history scans from ordinary ticker, trade, and book events. Book events
cache numeric levels and build the JSON book only when a signal snapshot needs
it; unused JSON wrappers and clones were removed. Price updates reuse the
pending-outcome remainder queue. The initial single-sample calculation run
measured 20,291.3 / 22,937.6 / 345.8 / 44.6 / 539.9 / 49.2 ns for live
snapshot, explained snapshot, research features, return context, diagnostics,
and book snapshot. A focused pending-outcome run measured 21,783.3 ns/update.
Repeated runs varied sharply on unchanged calculations under concurrent host
load; these initial timings are superseded by the measurement method in the
section above. The 6-million-row book-history reproduction retained 20 MiB RSS
in 0.47 s. WSL2 Linux x86_64, optimized Cargo profile. Live profiling was
unavailable because matching `linux-tools` for the WSL2 kernel are not
installed. No services were restarted.

## Restore missed-opportunity bands browser path (2026-11-12)

AutoTrader passed 159 library tests and 7 binary tests; React passed 110 tests.
The AutoTrader benchmark completed on WSL2 Linux x86_64 with the optimized Cargo
profile. Entry / position evaluators ranged from 3.1–18.4 / 1.8–4.3 ns, and
serialization / depth / order / parameter helpers measured 577.4 / 653.9 /
449.9 / 2.0 ns. The 0 / 256 / 10,000-order settings snapshots measured 16.719 /
71.929 / 2,965.218 us versus the latest recorded 11.856 / 84.808 / 3,903.786
us. The band retrieval endpoint is not exercised by these microbenchmarks.

## Rename the Paper Exchange source crate (2026-11-12)

After moving the crate to `src/rust/paper_exchange`, Position Management passed
29 tests (1 ignored), AutoTrader 159 library / 7 binary, and Paper Exchange 10
library / 4 binary. The rerun produced 2,093.0 ms for the concurrent file
workload, 100k-fill decode/encode of 216.7 / 234.4 / 77.8 / 103.6 ms, and
aggregation / indexed lookup / count scan of 4.58 ms / 55.3 ns / 60.8 ns. These
unchanged ledger workloads were 4.8–29.1% above the latest archived comparable
baseline, with the concurrent file workload +12%; recent same-day runs were
also elevated. Treat the changes as noisy under active stack load, not an effect
of the source rename. AutoTrader and Paper Exchange benchmark workloads were
within 10% of the preceding same-day run except the 10,000-order settings
snapshot (+29%), an unrelated existing workload. Optional service HTTP bench
was skipped; no services were restarted.

## Position Management mark-cache naming follow-up (2026-11-12)

Position Management passed 29 tests (1 ignored). After renaming its market
pollers to reflect mark-only caching, the concurrent file workload measured
2,878.1 ms, 100-fill MessagePack/JSON decode/encode 147.7 / 162.1 / 19.5 / 33.4
us, and 100k-fill decode/encode 224.6 / 245.9 / 82.7 / 112.0 ms. Aggregation /
indexed lookup / count scan measured 4.66 ms / 70.3 ns / 76.9 ns. Compared with
the immediately prior same-day run, the 100k decodes were within 3%, while the
file workload, encodes, aggregation, and lookups were 7–29% higher. The run was
under active host load, and the rename does not touch these code paths; treat
the increases as noisy observations pending a controlled repeat. The optional
HTTP benchmark remained skipped because `BENCH_SERVICE_URL` was unset.

## Paper Exchange boundary change repeat benchmarks (2026-11-12)

The repeat run completed after renaming the Rust package; Position Management
passed 29 tests (1 ignored), AutoTrader 159 library / 7 binary, and Paper
Exchange 10 library / 4 binary. React passed 110 tests and typecheck; exporter
passed 7 tests. Compose validation and `git diff --check` passed. No services
were restarted.

Position Management's concurrent file workload measured 2,405.3 ms (+29.2%
versus 1,861.7 ms). 100-fill decode/encode measured 158.8 / 165.8 / 18.8 / 33.4
us; 100k-fill decode/encode measured 221.8 / 239.8 / 76.1 / 105.1 ms. Aggregation
/ indexed lookup / count scan measured 4.27 ms / 54.7 ns / 63.5 ns. Relative to
the latest comparable recorded run, most unchanged persistence workloads were
12–33% higher; the previous run in this section was also elevated. These
single-run results were collected with the stack active and do not measure the
new Paper Exchange RPC path. The optional HTTP bench was skipped because
`BENCH_SERVICE_URL` was unset.

AutoTrader evaluator ranges were 3.0–22.9 ns entry and 2.5–6.4 ns position;
signal serialization / depth / order / parameter helpers measured 594.9 / 680.1
/ 454.5 / 2.1 ns. Settings snapshot at 0 / 256 / 10,000 orders measured 12.55 /
80.43 / 2,892.3 us. These workloads do not measure hedge-history retrieval or
the candidate cache. Paper Exchange scheduler / manager construction measured
122.2 / 252.6 ns; child calculation and multi-symbol workload measured 5.4
ns/call and 5.9 ns/event. Against the immediately prior run, construction was
within 4% / 12%; the multi-symbol workload changed 3.5%. The current benchmark
does not exercise market-data subscription or order matching RPCs. WSL2 Linux
x86_64, optimized Cargo profiles.

## Move paper market data and matching to Paper Exchange (2026-11-12)

Position Management (29 passed, 1 ignored), AutoTrader (159 library, 7 binary),
and Paper Exchange (10 library, 4 binary) tests passed. Paper Exchange now owns
shared FeedHandlers book/ticker subscriptions and paper matching; Position
Management consumes compact marks and persists returned fills. Hedge candidates
are calculated by AutoTrader. No service was restarted.

Position Management benches measured the 100k-row concurrent file workload at
1,945.1 ms (+4.5% versus 1,861.7 ms), 100-fill MessagePack/JSON decode and
encode at 136.1 / 154.0 / 19.9 / 32.8 us (+4.8% to +14.6%), and 100k-fill
decode/encode at 208.5 / 227.5 / 78.0 / 108.7 ms (+10.0% to +26.4%). Aggregation
/ indexed lookup / count scan measured 4.32 ms / 54.8 ns / 55.8 ns (+12.0% to
14.9%). These unchanged ledger workloads were higher in this single run;
the full stack was active and no serialization changes explain the increase.
The optional HTTP benchmark was skipped because `BENCH_SERVICE_URL` was unset.

AutoTrader entry/position evaluator maxima were 20.3 / 5.5 ns versus 18.4 / 3.1
ns in the latest same-day recorded run; signal serialization / depth / order /
parameter helpers measured 584.0 / 655.4 / 455.0 / 2.1 ns versus 547.6 / 571.4
/ 417.0 / 1.8 ns. These microbenchmarks do not measure the new market-history
RPC or hedge-candidate cache. Paper Exchange VWAP scheduler / manager
construction measured 117.8 / 225.6 ns versus 106.8 / 200.1 ns in the latest
recorded run; the new market-data decode and bounded-history paths have no
standalone timing benchmark. The multi-symbol VWAP workload measured 5.7
ns/event and is not directly comparable to the earlier signal-processing event
workload. WSL2 Linux x86_64, optimized Cargo profiles.

## Bound AutoTrader audit queues and active signal context (2026-11-12)

AutoTrader tests passed (160 library, 7 binary). Audit queues now cap at
16,384 decision / 4,096 execution records and 8 MiB estimated retained payload
each. Replaceable decision rejections coalesce by symbol and may be evicted;
execution records are not evicted to admit another record, and saturation
continues to trigger the existing audit-health latch and new-entry pause. The
latest strategy signal map now retains one sample only for active position
strategy keys, expires them after 10 seconds, and is therefore bounded by the
configured 20 open-position limit. Existing benchmark workloads do not exercise
audit admission or signal-cache retention. The latest repeated run measured
entry 3.0–18.4 ns, position 1.7–3.1 ns, and serialization/depth/order/parameter
helpers at 547.6 / 571.4 / 417.4 / 1.8 ns. Against the previous same-day run
(3.0–18.3 / 1.7–4.8 ns; 538.5 / 764.3 / 435.5 / 1.8 ns), helpers were within
25.3%; the depth sample fell 25.2%. Compared with the preceding audit-change
run (529.6 / 568.9 / 429.3 / 1.8 ns), helper results were within 3.4%, with
depth +0.4%; the prior +34.3% depth result was a single-run outlier. WSL2 Linux
x86_64, optimized Cargo profile.

## Bound AutoTrader working caches (2026-11-11)

AutoTrader tests passed (157 library, 7 binary). Latest Strategy signal
retention now keeps one sample per symbol/strategy, expires entries after 3
minutes, and caps at 20,480 keys; the book-prewarm map caps at 4,096 symbols
and 5 minutes; the Learning response cache caps at 128 entries and 30 seconds.
The bounds do not include live positions, orders, or durable feedback outboxes.
The existing benchmark does not exercise cache admission/expiry or measure RSS.
Entry evaluation measured 3.0–17.3 ns, position evaluation 1.7–3.5 ns, and
serialization/depth/order/parameter helpers 529.6 / 568.9 / 429.3 / 1.8 ns.
Against the latest comparable post-cutover run (2.9–17.9 / 1.7–3.1 ns;
524.8 / 570.1 / 416.9 / 1.8 ns), results were within 3.0% except position
maximum (+12.9%); these standalone microbenchmarks do not isolate cache
changes and cannot establish their memory savings. WSL2 Linux x86_64, optimized
Cargo profile.

## Post-cutover strategy workers and replay verification (2026-11-11)

AutoTrader tests passed (155 library, 7 binary), Learning passed (53 library,
1 binary), Insights passed (39), FeedHandlers passed (63), and Strategy
passed (155 with the service feature). React tests passed (110); homepage
typecheck/build passed; exporter tests passed (6). The full rebuilt system ran
with 44 required system containers and 8 infrastructure containers. The
homepage liveness check passed all 17 checks, including feed ingestion and
Prometheus scrape targets.

AutoTrader measured 2.9–17.9 ns for entry and 1.7–3.1 ns for position
evaluation; serialization, depth assessment, order construction, and
parameter validation measured 524.8 / 570.1 / 416.9 / 1.8 ns. Versus the
latest same-day results (3.0–18.0 / 1.7–3.4 ns; 531.9 / 564.2 / 419.0 / 1.8
ns), changes were within 1.1% except depth assessment (+1.1%).

Learning load / decision snapshot / persistence measured 8.32 / 0.12 / 6.78
ms. Against the latest comparable baseline run (2.67 / 0.12 / 7.14 ms), load
was +211%, snapshot unchanged, and persistence -5%. Two earlier runs with the
same fixture measured 8.46 / 8.38 ms for load. This fixture has no replay
history; the new measurement ran with the rebuilt market/Scout stack active,
so the load increase remains a host-contention or runtime-working-set question
and is not attributed to replay processing.

Insights measured 11,281 / 11,598 / 283 / 20 / 325 / 44 ns for live snapshot,
explained snapshot, research extraction, return context, diagnostics, and book
snapshot. Compared with the same-day repeat (11,376 / 11,461 / 239 / 19.8 /
321 / 43.7 ns), the feature-extraction sample was +18.5%; other results were
within 1.2%. Against the previous baseline, research extraction was +16.8%.
This calculation-only workload does not exercise active-symbol eviction or
the new historical query.

FeedHandlers measured 252 ns trade decode, 1.329 / 6.618 / 49.388 us for 7 /
50 / 400-level snapshots, and 1.159 us incremental updates. Versus the latest
same-day results (260 ns; 1.274 / 6.745 / 48.632 us; 1.129 us), these changed
-3.0%, +4.3%, -1.9%, +1.6%, and +2.7%, respectively. Order-book reads, price
parsing, writes, updates, best-level restore, and sparse snapshots were 0.84 /
7.57 / 4.53 ns, 179 ns, and 5.819 / 6.830 us, within 2.5% of the latest
comparable results. SBE allocation/encode and reused-buffer encode measured
30.1 / 11.5 ns; 1 / 4 / 16-client separate encoding was 29.3 / 104.7 / 416.1
ns, while shared immutable payload was 40.6 / 76.9 / 216.4 ns. Broadcast to
200 / 400 subscribers measured 4.035 / 8.154 us per event. Feed-to-subscriber
measured 0.1 / 0.2 / 0.1 us per event for 1 / 16 / overloaded 16 subscribers;
the intentionally overloaded case reported 674,301 lagged deliveries. These
SBE/fanout results stayed within recent single-run variation. Strategy has
no timed benchmark target; its bench command completed with all 155 harness
tests ignored. WSL2 Linux x86_64, optimized Cargo profiles; all timing results
are standalone microbenchmarks, not end-to-end RSocket latency.

The first post-cutover Scout RSS sample reached 873–1,019 MiB for the five
live-strategy Scout workers, with four close to their 1 GiB cap. The Scout
worker now hydrates at most 4,096 outcomes and 4,096 portfolio evidence rows
into RAM while keeping older records in SQLite. Rebuild and remeasure after
this bounded-hydration change before treating the initial RSS as steady state.

## Learning-owned multi-horizon strategy replay (2026-11-11)

Learning tests passed (48 library, 1 binary), AutoTrader tests passed (155 library, 7 binary), Strategy tests passed (153 with the `service` feature), and Insights tests passed (39). Learning's unchanged load fixture (1,000 variants, 200 profiles, 5,000 event IDs) measured 8.46 / 8.38 ms across two runs, decision snapshot 0.13 / 0.11 ms, and state persistence 6.73 / 6.66 ms. Versus the latest same-workload Learning run (2.67 / 0.12 / 7.14 ms), load was +217% / +214%; snapshot varied +8% / -8%; persistence was -6% / -7%. The fixture contains no replay jobs or history, so this is not a measurement of replay processing. These runs occurred while the old duplicate containers were still active and WSL2 host load averaged about 4.5; treat the Learning load result as environment-contended and repeat after runtime cutover before attributing the increase.

AutoTrader strategy evaluation measured entry 3.0–18.0 ns, position 1.7–3.4 ns, and serialization / depth / order / parameter helpers at 531.9 / 564.2 / 419.0 / 1.8 ns. Against the latest comparable run (3.0–17.9 / 1.8–3.2 ns; 530.1 / 561.5 / 419.5 / 1.8 ns), evaluator maxima changed +0.6% / +6.3%; helpers changed +0.3% / +0.5% / -0.1% / unchanged. These measurements do not exercise the new close-feedback payload or replay path and were collected under the same container contention, so repeat after cutover.

Insights tests passed (39). Calculation benchmarks measured 11,110 / 11,334 / 242.3 / 20.8 / 340.5 / 43.3 ns for live snapshot, explained snapshot, research features, return context, diagnostics, and book snapshot; a focused repeat measured 11,376 / 11,461 / 239.0 / 19.8 / 321.0 / 43.7 ns. Compared with the latest same-day run (10,871.7 / 11,638.6 / 242.5 / 32.4 / 348.5 / 45.0 ns), results were within recent single-run variation except that return-context varied materially across runs. The six-million-row book-history reproduction retained 19 MiB RSS in 0.22 s. Strategy has no timed benchmark target; `cargo bench --features service` completed with all 153 harness tests ignored. WSL2 Linux x86_64, optimized Cargo profiles. No services were started or restarted; repeat the Learning, AutoTrader, and Insights measurements after the duplicate runtime cutover.

## Bound Insights active-symbol runtime state (2026-11-11)

Insights tests passed (39). Insights now caps active symbol state at 4,096 by default, evicts only symbols idle for the configurable 60-second default TTL when their engine is unlocked, and rejects new symbols with HTTP 429 when capacity is full and no idle symbol can be safely removed. Per-symbol feed shard queues and the compact one-minute book history remain unchanged. Symbol-keyed engine, signal-push, Scout-input, market, UI, diagnostic, latest-price, and rolling-volatility state is removed together; pending news-learning votes are limited to 60 seconds and 65,536 entries. The final calculation benchmark measured 10,871.7 / 11,638.6 / 242.5 / 32.4 / 348.5 / 45.0 ns for live snapshot, explained snapshot, research feature extraction, return context, diagnostics, and book snapshot. Versus the preceding run of this change (11,401.6 / 11,253.1 / 253.5 / 20.5 / 322.6 / 46.4 ns), changes were -4.6% / +3.4% / -4.3% / +58.0% / +8.0% / -3.0%; the return-context result varied materially between runs, and these calculation workloads do not exercise active-symbol admission or eviction. The six-million-row book-history reproduction retained 19 MiB RSS in 0.22 s versus 19 MiB in 0.24 s previously. WSL2 Linux x86_64, optimized Cargo bench profile. No services were restarted.

## Remove unused one-hour quote-path retention (2026-11-11)

Insights tests passed (35). The one-hour, one-second counterfactual book ring and its per-evaluation quote-path generation/storage were removed; the compact 100 ms, 60-second book history remains for live and research features. Legacy kind-6 quote-path records still decode, but new evaluation writes emit only the evaluation record. Production-only generic stop/target and strategy-invalidation counterfactual calculations were removed. `cargo bench --manifest-path src/rust/insights/Cargo.toml` completed; its test cases were ignored in bench mode. The six-million-row book-history reproduction retained 19 MiB RSS in 0.25 s. Calculation benchmarks measured 11,590.2 / 14,285.4 / 254.6 / 22.3 / 329.7 / 44.2 ns for live snapshot, explained snapshot, research feature extraction, return context, diagnostics, and book snapshot. Versus the latest same-day comparable run (11,382.6 / 12,915.1 / 276.7 / 21.0 / 359.1 / 45.7 ns), single-run changes were +1.8% / +10.6% / -8.0% / +6.2% / -8.2% / -3.3%. These calculations do not measure retained memory under production feed traffic; the book-history reproduction covers only the retained compact history. WSL2 Linux x86_64, optimized Cargo bench profile. No services were restarted.

## Keep learned threshold validation in Learning (2026-11-11)

AutoTrader tests passed (154 library, 7 binary); Learning passed (43 library, 1 binary). AutoTrader no longer retains per-trade threshold sample arrays or recomputes train/holdout walk-forward thresholds. Learning remains the sole learned threshold source through its active profile snapshot; valid user overrides win, and Scout candidate parameters are applied last for the evaluation with a profile ID recomputed from the exact parameters. Threshold observations still flow from trade feedback to Learning. AutoTrader entry evaluation measured 3.0–17.9 ns, position evaluation 1.8–3.2 ns, and serialization/depth/order/parameter helpers 530.1 / 561.5 / 419.5 / 1.8 ns. Compared with the latest same-day run (3.0–18.1 / 1.7–3.3 ns; 555.3 / 580.1 / 421.5 / 1.9 ns), maxima changed -1.1% / -3.0% and helpers varied by -5.3% to -0.5%. Learning load / decision snapshot / persistence measured 2.67 / 0.12 / 7.14 ms versus 2.60 / 0.11 / 6.87 ms (+2.7% / +9.1% / +3.9%). These microbenchmarks do not exercise threshold profile adaptation or RPC dispatch. WSL2 Linux x86_64, optimized Cargo bench profile. No services were restarted.

## Remove Insights-owned signal validation and missed-opportunity evidence (2026-11-11)

Insights tests passed (35); Learning tests passed (43 library, 1 binary); AutoTrader passed (159 library, 7 binary); Strategy passed (152 library with `service`); React passed (110). Insights no longer calculates generic signal walk-forward or missed-opportunity bands. AutoTrader no longer consumes Insights generic walk-forward fallbacks; strategy/profile evidence attached by Learning remains. Learning exposes an empty persisted missed-opportunity snapshot until attributed skipped-candidate and quote-path evidence exists; no bands are inferred from raw signals. Insights benchmarks measured 11,211.9 / 11,697.6 / 253.1 / 20.6 / 323.4 / 45.1 ns for live snapshot, explained snapshot, research feature extraction, return context, diagnostics, and book snapshot. The bounded book-history reproduction retained 21 MiB RSS over six million rows and completed in 0.22 s. Compared with the prior same-day run, calculation measurements were within 6.5%; these workloads do not exercise the removed historical scans. Learning load / decision snapshot / persistence measured 2.76 / 0.12 / 6.94 ms, compared with 2.60 / 0.11 / 6.87 ms (+6.2% / +9.1% / +1.0%); the benchmarks do not exercise missed-opportunity evidence handling. Repeated AutoTrader evaluator runs measured entry 3.1–17.8 ns, position 1.8–3.4 ns, and serialization/depth/order/parameter helpers 557.4 / 581.5 / 420.6 / 1.8 ns. Compared with the latest same-day run (3.0–18.1 / 1.7–3.3 ns; 555.3 / 580.1 / 421.5 / 1.9 ns), maximum evaluator values changed -1.7% / +3.0%, and helper values varied by -5.3% to +0.4%. Those microbenchmarks do not exercise RSocket dispatch, timeouts, or concurrency. Strategy and Learning have no additional timed worker/replay workload. WSL2 Linux x86_64, optimized Cargo bench profile. No services were restarted.

## Show a blocker for every strategy that does not submit an order (2026-11-11)

AutoTrader tests passed (165 library, 7 binary). Benchmarks measured entry evaluation 3.0–18.1 ns, position evaluation 1.7–3.3 ns, and serialization/depth/order/parameter helpers 555.3 / 580.1 / 421.5 / 1.9 ns. Compared with the previous same-day run (entry 3.8–18.7 ns, position 1.8–3.9 ns, helpers 530.5 / 565.9 / 430.9 / 1.8 ns), evaluator ranges were within 22% at the minimum and 4% at the maximum; helpers varied by +4.7% / +2.5% / -2.2% / +5.6%. The evaluation microbenchmarks do not measure candidate-selection blocker reporting or RSocket worker routing. These runs used the 44-container Strategy deployment, unlike earlier 28-container runs. WSL2 Linux x86_64, optimized Cargo bench profile.

## Route AutoTrader strategy decisions to Strategy (2026-11-11)

AutoTrader tests passed (165 library, 7 binary). Benchmarks measured entry evaluation at 3.1–18.3 ns, position evaluation at 2.1–3.8 ns, and serialization/depth/order/parameter helpers at 540.5 / 584.5 / 434.1 / 1.9 ns. Compared with the latest same-day AutoTrader run (entry 3.0–18.3 ns, position 1.8–3.4 ns, helpers 534.5 / 571.3 / 440.7 / 1.8 ns), entry was stable, position maximum was 11.8% higher, and helper values varied by -1.5% to +5.6%. These evaluator/helper microbenchmarks do not exercise RSocket routing, timeout, or concurrency behavior; no latency claim is made for the new remote decision path. WSL2 Linux x86_64, optimized Cargo bench profile.

## Remove raw-signal walk-forward from Insights (2026-11-11)

Insights tests passed (43 library, 35 binary) and React tests passed (110). `cargo bench --manifest-path src/rust/insights/Cargo.toml` completed; its 43 library and 35 binary tests were ignored in bench mode. The bounded book-history reproduction retained about 21 MiB RSS. Calculation benchmarks measured 10,957.5 / 10,986.0 / 242.1 / 20.2 / 335.7 / 45.0 ns for live snapshot, explained snapshot, research feature extraction, return context, diagnostics, and book snapshot. Compared with the latest same-day run (11,382.6 / 12,915.1 / 276.7 / 21.0 / 359.1 / 45.7 ns), these single-run readings varied -3.7% / -14.9% / -12.5% / -3.8% / -6.5% / -1.5%. The calculation workloads do not call the removed generic historical walk-forward scan, so they cannot establish its CPU or memory savings. WSL2 Linux x86_64, optimized Cargo bench profile.

## Add bounded historical research-feature paging for Learning replay (2026-11-11)

Insights tests passed (48 library, 35 binary). The new internal RSocket route pages symbol-filtered `SignalResearchFeatures` across inclusive time bounds using a chronological timestamp/row-ID cursor, with a 512-row page cap, 4,096-row scan cap, 8 MiB compressed scan cap, 128 KiB decoded-record cap, and 512 KiB JSON response cap. Tests cover symbol/time filtering, page continuation across other symbols, inclusive lower/upper bounds, bounded-scan incomplete coverage, and archived-history coverage reporting. `cargo bench --manifest-path src/rust/insights/Cargo.toml` completed; the 6,000,000-row book-history reproduction retained about 21 MiB RSS and took 0.24 s. Calculation benchmarks measured 11,382.6 / 12,915.1 / 276.7 / 21.0 / 359.1 / 45.7 ns for live snapshot, explained snapshot, feature extraction, return context, diagnostics, and book snapshot. Compared with the latest same-day readings (10,921.0 / 11,190.2 / 238.9 / 19.8 / 319.8 / 45.0 ns), these calculation microbenchmarks varied higher by 1.6–15.8%; they do not exercise the SQLite paging route and the single-run differences cannot be attributed to it. The API reports overlapping archived research days as unscanned or unavailable and marks history incomplete instead of implying full coverage. WSL2 Linux x86_64, optimized Cargo bench profile.

## Add stateless Strategy live-evaluation worker protocol (2026-11-11)

Strategy tests passed: 152 library tests with the `service` feature and 0 binary tests; `cargo bench` completed with 152 unit tests ignored and no timed benchmark workloads. The new live worker exposes a versioned RSocket request-response contract for entry, open-position, dated-carry entry, and settlement evaluation. It validates strategy routing, symbol/profile identity, parameter names/bounds/steps, payload size, and configured in-flight capacity; responses echo request ID and exact profile ID/version. A local WebSocket/RSocket integration test verified correlated profile-attributed response delivery. No timing comparison is available because this crate has no timed benchmark target; its existing evaluator-only tests do not measure RSocket latency, queue behavior, or process memory. WSL2 Linux x86_64.

## Bound Learning evidence-only response fan-out (2026-11-11)

Learning tests passed (43 library, 1 binary). Benchmarks measured state load / decision snapshot / persistence at 2.60 / 0.11 / 6.87 ms, compared with the latest Learning run (2.56 / 0.12 / 7.09 ms): +1.6% / -8.3% / -3.1%. The evidence-only RSocket path now selects the exact requested parameter profile, calculates evidence locally without dispatching to a Scout worker, and returns a compact candidate summary instead of cloning the full market signal for each profile. The microbenchmarks do not exercise this RSocket path, so they do not measure its memory reduction. WSL2 Linux x86_64, optimized Cargo bench profile.

## Record downstream candidate-selection blockers (2026-11-11)

AutoTrader tests passed (165 library, 7 binary). Benchmarks measured entry evaluation 3.8–18.7 ns, position evaluation 1.8–3.9 ns, and serialization/depth/order/parameter helpers 530.5 / 565.9 / 430.9 / 1.8 ns. Compared with the latest same-day run (entry 3.1–18.3 ns, position 2.1–3.8 ns, helpers 540.5 / 584.5 / 434.1 / 1.9 ns), entry maxima and position maxima were within 3%; the minimum entry timing varied +22.6%, while helper timings were 0.7–5.3% lower. These benchmarks ran with the new worker fleet active and do not measure the per-strategy status reporting path. The status now records downstream strength, score, evidence, freshness, pullback, and order-submission reasons after a strategy accepts its signal but no order is placed; enabled daily strategies also report when Insights sent no candidate. WSL2 Linux x86_64, optimized Cargo bench profile.

## Report the exact missing live-strategy feature (2026-11-11)

AutoTrader tests passed (159 library, 7 binary). Benchmarks measured entry evaluation 3.1–18.2 ns, position evaluation 1.8–3.2 ns, and serialization/depth/order/parameter helpers 543.4 / 599.4 / 427.5 / 2.6 ns. Compared with the latest same-day run (entry 3.0–17.4 ns, position 1.7–3.0 ns, helpers 529.2 / 585.3 / 422.1 / 1.8 ns), entry and position timings were within 5%; helper timings varied +2.7% / +2.4% / +1.3% / +44.4%. The parameter-validation difference is 0.8 ns absolute and these helper benchmarks do not measure live feature selection or the new error formatting, so it is not evidence of material production degradation. The change replaces one generic blocker with the missing parameter/window, warm-up value, or price-move limit that actually prevented feature preparation. WSL2 Linux x86_64, optimized Cargo bench profile.

## Keep Insights live and research cadences independent (2026-11-11)

Insights tests passed (44 library, 34 binary). `cargo bench` completed; no timed tests failed. The bounded book-history reproduction retained 100 symbols × 1,000 updates/s × 60s at approximately 21 MiB RSS. Insights calculations measured 10,921.0 / 11,190.2 / 238.9 / 19.8 / 319.8 / 45.0 ns for live snapshot, explained snapshot, research feature extraction, return context, diagnostics, and book snapshot. Compared with the latest same-day recorded run (16,462.5 / 12,452.7 / 275.4 / 20.1 / 326.4 / 45.4 ns), these single-run measurements were lower by 33.6% / 10.1% / 13.3% / 1.5% / 2.0% / 0.9%. The workloads do not exercise live RSocket delivery or research-queue behavior, so they do not establish a latency/RSS change from the mode removal. WSL2 Linux x86_64, optimized Cargo bench profile.

## Distinguish non-directional live signals in strategy blockers (2026-11-11)

Strategies tests passed (70); AutoTrader tests passed (157 library, 7 binary). The Strategies bench target completed with all 70 unit tests ignored and no timed workloads. AutoTrader measured entry evaluation 3.0–17.4 ns, position evaluation 1.7–3.0 ns, and serialization/depth/order/parameter helpers 529.2 / 585.3 / 422.1 / 1.8 ns. Compared with the latest same-day run (entry 3.0–17.5 ns, position 1.7–3.7 ns, helpers 525.3 / 567.4 / 409.9 / 1.8 ns), helpers varied +0.7% / +3.1% / +3.0% / 0%; no material regression is indicated. This change only improves blocker diagnostics by distinguishing an explicitly neutral signal, a missing/empty direction field, and an unsupported direction value. WSL2 Linux x86_64.

## Pool all Strategies RSocket request helpers (2026-11-11)

Strategies tests passed (69), Insights passed (43 library), AutoTrader passed (157 library, 7 binary), Scouts passed (80), and Live Signals passed (25). Strategies, Scouts, and Live Signals have no timed benchmark workloads; their bench runs completed with tests ignored. Insights calculations measured 16,462.5 / 12,452.7 / 275.4 / 20.1 / 326.4 / 45.4 ns for live snapshot, explained snapshot, feature extraction, return context, diagnostics, and book snapshot. Compared with the prior same-day run (11,829.1 / 12,019.2 / 246.0 / 20.2 / 327.9 / 45.2 ns), snapshot and extraction measurements varied +39.2% / +3.6% / +12.0%; the other workloads were within 0.5%. These calculation workloads do not exercise RSocket pooling, so no performance change is attributed to the transport pool. AutoTrader measured entry 3.0–17.5 ns, position 1.7–3.7 ns, and serialization/depth/order/parameter helpers 525.3 / 567.4 / 409.9 / 1.8 ns, within 6% of the prior same-day run. WSL2 Linux x86_64, optimized Cargo bench profiles.

## SQLite-backed Scout instance history (2026-11-11)

Scouts tests passed (80) and Strategies tests passed (66). Both Cargo bench commands completed; these crates have no timed benchmark workloads. Scout workers now keep at most 4,096 full instance JSON values in their runtime cache (previously up to 100,000), a maximum 95.9% reduction in resident full-instance count. All rows remain in SQLite; report requests read the complete existing seven-day window, point lookups serve markouts and outcomes, and the portfolio evidence index is rebuilt by streaming the database. The cache-bound regression test verifies 4,113 records remain queryable after only 4,096 are resident. This is a record-count bound rather than an RSS measurement; service memory still needs observation after rebuild/start. WSL2 Linux x86_64.

## Bound AutoTrader's pending live-signal queue (2026-11-11)

AutoTrader tests passed (156 library, 7 binary). Evaluator measurements ranged 3.1–19.4 ns for entry and 2.0–4.4 ns for position evaluation; serialization, market-depth assessment, order request construction, and parameter validation measured 591.4 / 604.0 / 432.6 / 1.8 ns. Compared with the prior same-day run (maximum 18.1 / 5.8 ns and helpers 539.3 / 591.9 / 440.4 / 1.8 ns), position max was lower and helper timings were within 9.7%. This evaluator benchmark does not exercise queue retention or RSocket dispatch. WSL2 Linux x86_64, optimized Cargo bench profile.

## Reuse AutoTrader service RPC connections (2026-11-11)

AutoTrader tests passed (157 library, 7 binary). Evaluator results ranged 3.1–19.1 ns for entry and 1.7–4.1 ns for position; serialization, market-depth assessment, order request construction, and parameter validation measured 543.6 / 583.6 / 430.0 / 1.8 ns. Compared with the prior queue-bound run (maximum 19.4 / 4.4 ns and helpers 591.4 / 604.0 / 432.6 / 1.8 ns), all were within 8.1%. These workloads do not exercise RSocket connection reuse. WSL2 Linux x86_64, optimized Cargo bench profile.

## Share missed-opportunity evidence in context-threshold requests (2026-11-11)

Strategies tests passed (67) with no timed benchmark workloads; Learning passed 42 tests and AutoTrader passed 157 library / 7 binary tests. Learning measurements were 2.56 / 0.12 / 7.09 ms for state load, decision snapshot, and persistence, versus the prior run's 2.31 / 0.11 / 6.42 ms (+10.8% / +9.1% / +10.4%). These single-run results do not exercise context-threshold payload sizing. AutoTrader measured entry 3.2–24.4 ns, position 1.8–5.6 ns, and helpers 558.3 / 591.8 / 427.0 / 2.0 ns; no material attribution to the request contract. WSL2 Linux x86_64, optimized Cargo bench profile.

## Share Insights evidence across live symbol delivery (2026-11-11)

AutoTrader tests passed (152 library, 7 binary), Live Signals (24), and Insights (43 library, 34 binary). Live Signals has no timing workload. Insights calculations measured 11,227.6 / 11,734.0 / 257.7 / 21.5 / 324.6 / 45.4 ns for live snapshot, explained snapshot, feature extraction, return context, diagnostics, and book snapshot. Compared with the previous same-day run (10,880.9 / 11,171.3 / 263.9 / 20.5 / 316.1 / 45.2 ns), differences were +3.2% / +5.0% / -2.3% / +4.9% / +2.7% / +0.4%. The new RSocket context path has no dedicated benchmark.

The subsequent Insights run measured 12,085.4 / 12,037.6 / 259.4 / 20.4 / 330.8 / 48.0 ns; compared with the prior run above, changes were +7.6% / +2.6% / +0.7% / -5.1% / +1.9% / +5.7%. These single-run calculation differences do not exercise the RSocket context path. WSL2 Linux x86_64, optimized Cargo bench profile.

## Per-symbol live signal payloads (2026-11-11)

Live Signals tests passed (24) and its benchmark command completed, but the crate has no timed benchmark workloads. The live-to-AutoTrader payload now carries only the current symbol's signal and top-of-book data. AutoTrader separately fetches and shares Insights walk-forward and missed-opportunity evidence, rather than receiving duplicate copies in every symbol request. Runtime memory must still be observed after deployment.

## Scout state partition and per-strategy workers (2026-11-11)

Scouts tests passed (74) and Strategies tests passed (66). Both Cargo bench commands completed; every test was ignored in bench mode and neither crate defines timing workloads, so there is no comparable runtime measurement. The one-off importer and shared legacy JSON state were removed after verified migration of the Docker volume into 17 per-strategy SQLite databases. Source reconciliation covered 341,275 rows including WAL data; all destinations passed SQLite integrity and strategy-partition checks. WSL2 Linux x86_64.

## End-to-end RSocket migration and operations-only HTTP surfaces (2026-11-11)

Final verification passed: FeedHandlers 63 tests; Binance Feed 1; Position Management 45 (1 ignored); Execution 4; Insights 43 library + 34 binary; Live Signals 24; Learning 41 library + 1 binary; Scouts 74; Strategies 66; AutoTrader 152 library + 7 binary; React 110, typecheck and Prettier; monitoring exporter 3 Python tests; news sentiment 3 Python tests. Services were not restarted during these tests.

Cargo benchmarks ran serially on WSL2 Linux x86_64 with optimized profiles. Learning load / decision snapshot / persistence measured 2.31 / 0.11 / 6.42 ms versus the latest comparable 2.32 / 0.11 / 6.38 ms (-0.4% / unchanged / +0.6%). Scouts, Strategies and Live Signals have no dedicated timing workloads. Execution scheduler/manager construction measured 106.8 / 200.1 ns versus 106.5 / 213.3 ns; 64-symbol signal processing was 4.9 ns/event versus 5.0 ns/event. These workloads do not exercise RSocket dispatch.

FeedHandlers measured 265.4 ns trade decode/encode, 1.215 / 6.404 / 47.385 us for 7 / 50 / 400-level snapshots, and 1.117 us incremental update. Against the latest comparable same-day run (243.1 ns, 1.223 / 6.363 / 47.241 us, 1.120 us), trade decode was 9.2% slower while snapshot/update workloads were within 0.7%. The trade path is unchanged and the single run is recorded as variation. Insights calculations measured 10,880.9 / 11,171.3 / 263.9 / 20.5 / 316.1 / 45.2 ns for live snapshot, explained snapshot, feature extraction, return context, diagnostics, and book snapshot. Compared with the latest comparable run (11,176.8 / 11,479.2 / 244.6 / 17.5 / 324.4 / 44.5 ns), changes were -2.6% / -2.7% / +7.9% / +17.1% / -2.6% / +1.6%; these calculation workloads do not exercise the new RSocket paths.

Position Management's concurrent file workload was 1,861.7 ms versus 1,887.8 ms (-1.4%). 100-fill MessagePack/JSON decode and encode measured 119.3 / 134.4 / 18.1 / 31.3 us, broadly stable. At 100k fills, MessagePack/JSON decode was 176.4 / 206.9 ms (latest comparable 181.2 / 176.0 ms; JSON decode +17.5%); encode was 61.7 / 93.7 ms (60.7 / 92.4 ms). Aggregation / indexed lookup / count scan measured 3.77 ms / 47.7 ns / 49.8 ns versus 3.86 ms / 49.9 ns / 51.9 ns. The JSON decode increase is a single-run result on an unchanged serialization path; the optional service HTTP benchmark was skipped because `BENCH_SERVICE_URL` was unset. AutoTrader's evaluator ranged 3.0–17.2 ns entry and 1.7–5.7 ns position; serialization/depth/order/parameter helpers were 533.6 / 574.0 / 417.0 / 1.8 ns. Compared with the latest comparable maxima (17.2 / 2.9 ns) and helpers (508.0 / 541.8 / 403.1 / 1.8 ns), position maximum and the first three helpers were higher; these single-run evaluator workloads do not measure RSocket requests. The new RSocket endpoints, Python classifier client, and metrics exporter have no representative end-to-end benchmark workloads.

## FeedHandlers instrument-list RSocket request-response (2026-11-11)

FeedHandlers tests passed (63); all FeedHandlers benchmarks completed. The new instrument-list RSocket request-response handler has no dedicated benchmark, so no performance change is attributed to it. Feed-processing workloads measured 243.1 ns trade decode/encode, 1.223 / 6.363 / 47.241 us for 7 / 50 / 400-level snapshots, and 1.120 us incremental update. Compared with the latest same-day comparable run (247.8 ns, 1.236 / 6.466 / 47.529 us, and 1.111 us), the first four workloads were 0.6–1.9% faster; incremental update was 0.8% slower. Order-book measurements were 0.79 ns direct reads, 7.37 ns decimal parse, 4.35 ns writes, 171.38 ns update, 5.464 us restore, and 6.467 us sparse snapshot. SBE encoding measured 28.9 / 11.9 ns (allocate/reused buffer), 29.1 / 38.4 ns (1 client encode/share), 104.9 / 72.8 ns (4 clients), and 401.9 / 208.1 ns (16 clients). Broadcast to 200 / 400 subscribers measured 3.825 / 7.642 us per event. Compared with the most recent comparable run above, order-book and fan-out results were broadly stable; decimal parsing was 2% slower. WSL2 Linux x86_64, optimized Cargo bench profile.

## Move Binance daily candles from internal HTTP to RSocket (2026-11-11)

Binance Feed contract tests passed (1); FeedHandlers tests passed (62); Insights tests passed (38 library and 33 binary). `cargo bench` completed for Binance Feed, FeedHandlers, and Insights. Binance Feed has no dedicated benchmark workloads. FeedHandlers measured 247.8 ns trade decode/encode, 1.236 / 6.466 / 47.529 us for 7 / 50 / 400-level snapshots, and 1.111 us incremental update. Order-book workloads measured 0.79 ns direct reads, 7.34 ns decimal parse, 4.41 ns writes, 170.48 ns update, 5.441 us restore, and 6.436 us sparse snapshot. SBE encode/fan-out measured 29.2 / 10.6 ns (allocate/reused buffer), 27.6 / 39.5 ns (one client encode/share), 99.5 / 72.8 ns (four clients), and 412.3 / 207.3 ns (16 clients). Broadcast to 200 / 400 subscribers measured 3.855 / 7.645 us per event. Versus the latest comparable FeedHandlers run above, feed and order-book workloads were within 2.1%; 16-client encode-allocation was 4.6% slower, while shared-buffer and broadcast workloads were broadly stable or faster. Insights calculations measured 11,176.8 / 11,479.2 / 244.6 / 17.5 / 324.4 / 44.5 ns for live snapshot, explained snapshot, feature extraction, price-return context, diagnostics, and book snapshot. Compared with the immediately prior run above, these unrelated calculation workloads were 1.8–8.0% slower. No benchmark exercises the new Binance RSocket request-response path, so no performance change is attributed to this migration. WSL2 Linux x86_64, optimized Cargo bench profile; no services were restarted.

## Coordinate Scout candidate evaluation/evidence through Learning RSocket (2026-11-11)

Learning tests passed (40), Scouts tests passed (72), Strategies tests passed (66), and AutoTrader tests passed (150). Scouts and Strategies have no dedicated benchmark workloads, so their tests were ignored in bench mode. Learning measured database load at 2.39 ms, decision snapshot at 0.11 ms, and state persistence at 6.09 ms. Compared with the latest comparable Learning run (3.39 / 0.14 / 8.06 ms), these were 29.5%, 21.4%, and 24.4% lower. AutoTrader measured strategy entry/position evaluation at 2.9–17.2 / 1.7–2.9 ns and signal serialization, market-depth assessment, order construction, and parameter validation at 508.0 / 541.8 / 403.1 / 1.8 ns. Compared with its latest comparable run (2.9–18.5 / 1.7–4.2 ns; 526.6 / 547.5 / 413.8 / 1.8 ns), entry and position maxima were 7.0% and 31.0% lower and helper costs were 0.0–3.5% lower. These microbenchmarks do not exercise RSocket dispatch, worker CPU/memory, or the still-unported Scout lifecycle, so no performance improvement is attributed to the worker path. WSL2 Linux x86_64, optimized Cargo bench profile. No services were restarted.

## Move rolling price volatility from FeedHandlers to Insights (2026-11-11)

FeedHandlers tests passed (62), Insights tests passed (38 library and 32 binary), and Live Signals tests passed (27). React app tests passed (93), with typecheck successful. `cargo bench` completed for all three Rust crates; Live Signals has no benchmark targets, so its 27 tests were ignored in bench mode. The FeedHandlers feed-processing workloads measured 245.0 ns for trade decode/encode, 1.197 / 6.274 / 46.365 us for 7 / 50 / 400-level snapshots, and 1.117 us for incremental updates. Order-book results were 0.78 ns direct reads, 7.69 ns decimal parse, 4.33 ns writes, 167.60 ns update, 5.348 us best-level restore, and 6.430 us sparse snapshot. SBE encoding measured 29.1 ns allocation/encode, 10.6 ns reused-buffer encode, and per-client/share costs of 28.1/38.3 ns (1), 100.5/75.0 ns (4), and 397.4/206.6 ns (16); broadcast to 200/400 subscribers measured 3.785/7.486 us/event. Compared with latest comparable FeedHandlers results above, feed processing and most order-book numbers were comparable or modestly faster; decimal parsing was 4.2% slower. Standalone broadcast throughput varied, with 12.5% lower single-subscriber throughput, 2% higher 16-subscriber throughput, and 6.5% higher slow-consumer throughput than the last responder run. Insights calculations measured 10,686.8 / 10,996.4 / 232.7 / 16.6 / 316.7 / 45.0 ns for live snapshot, explained snapshot, feature extraction, price-return context, diagnostics, and book snapshot, 3.6–12.9% below the latest comparable run. These microbenchmarks do not measure the new raw-ticker-to-enriched-RSocket pipeline or rolling-volatility throughput, so no performance change is attributed to relocation. WSL2 Linux x86_64, optimized Cargo bench profile.

## Typed enriched-price SBE template and O(1) rolling counts (2026-11-11)

FeedHandlers tests passed (62), Insights tests passed (38 library and 33 binary), Live Signals tests passed (27), and React tests passed (107) with typecheck successful. The wire format now distinguishes raw `SymbolPrice` template 1 (48-byte body: symbol, price, timestamp) from `EnrichedSymbolPrice` template 4 (64-byte body: symbol, price, timestamp, 1m volatility, 5m volatility), schema version 9. Insights rolling-window result counts now use already-pruned deque lengths, keeping each price update O(1); cutoff boundary tests cover both windows. `cargo bench` completed for all three crates. Live Signals has no benchmark targets, so its tests were ignored in bench mode. Final serial FeedHandlers measurements were 247.4 ns trade decode/encode, 1.210 / 6.380 / 48.084 us for 7 / 50 / 400-level snapshots, 1.124 us incremental update, and order-book timings of 0.79 ns direct reads, 7.22 ns decimal parse, 4.32 ns writes, 169.21 ns update, 5.368 us restore, and 6.428 us sparse snapshot. Against the latest same-day comparable run above, feed workloads were 0.7–3.7% slower; order-book results were essentially stable, with decimal parsing faster. SBE encode/fan-out measurements were 28.7 / 10.5 ns (allocate/reused buffer), 27.3/38.1 ns (1 client encode/share), 98.8/72.1 ns (4 clients), and 394.3/206.1 ns (16 clients). Broadcast throughput was 17.3M/97.5M/15.8M deliveries/s for 1 subscriber, 16 subscribers, and the slow-consumer case; versus the most recent comparable run, the single-subscriber result was 13% lower and the other two varied by 1% and 6%. An initial simultaneous benchmark run showed broad slowdowns; the reported follow-up reran the workloads serially. Insights calculation timings were 10,632.6 / 10,943.2 / 232.2 / 16.2 / 305.8 / 43.7 ns for live snapshot, explained snapshot, feature extraction, price-return context, diagnostics, and book snapshot, within 0.2–3.4% of the latest comparable run. The calculation benchmarks do not exercise the new SBE codec, rolling volatility, or RSocket pipeline. WSL2 Linux x86_64, optimized Cargo bench profile.

## Explicit Scout comparison metadata in feedback (2026-11-11)

Strategies tests passed (64), Learning tests passed (38), Scouts tests passed (67), and AutoTrader library tests passed (150). Full Learning, Scouts, and AutoTrader Cargo targets have pre-existing dangling trailing documentation comments in their `src/main.rs` files; tests were run with a temporary trailing item appended during each command and the files were restored unchanged afterward. Benchmarks completed for Strategies, Learning, Scouts, and AutoTrader; the first three have no dedicated benchmark workloads, so their tests were ignored in bench mode. AutoTrader measured strategy entry/position evaluation at 2.9–18.5 / 1.7–4.2 ns and signal serialization, market-depth assessment, order construction, and parameter validation at 526.6 / 547.5 / 413.8 / 1.8 ns. Compared with the latest comparable run (2.9–19.3 / 1.7–5.0 ns; 576.3 / 612.6 / 447.1 / 2.0 ns), these aggregate timings were lower by 4.1–16.0% for maxima and 7.4–10.6% for helpers. The benchmark does not exercise Scout feedback serialization or Learning comparison grouping, so no performance change is attributed to this contract update. WSL2 Linux x86_64, optimized Cargo bench profile; no services were started or stopped.

## Isolate Binance candle retrieval in its own service (2026-11-11)

FeedHandlers tests passed (65), Insights tests passed (38 library and 27 binary), and the new Binance Feed crate compiled and its test target passed (0 tests). FeedHandlers benchmarks ran after moving daily-candle retrieval behind an internal HTTP proxy; no benchmark exercises that route or the new Binance service. Feed-processing results were 247.5 ns trade decode/encode, 1.217 / 6.456 / 47.880 us for 7 / 50 / 400-level snapshots, and 1.131 us for incremental updates. Order-book results were 0.83 ns direct reads, 8.07 ns decimal parse, 5.08 ns writes, 180.21 ns update, 5.685 us best-level restore, and 6.612 us sparse snapshot. SBE results were 35.8 ns allocate/encode, 9.9 ns reused buffer, and per-client/share costs of 32.3/45.9 ns (1), 120.4/80.3 ns (4), and 459.8/218.8 ns (16); broadcast to 200/400 subscribers measured 3.922/7.728 us per event. Feed-to-subscriber measured 16.5M deliveries/s (1 subscriber), 70.9M (16), and 10.1M in the slow-consumer case; reported lag was 703,300 deliveries. Versus the latest comparable measurements, serialization/feed-processing and order-book timings were broadly comparable; subscriber throughput varied 17–40% lower in these standalone noisy workloads. No change is attributed to this service split. WSL2 Linux x86_64, optimized Cargo bench profile.

## Learning feedback handler readability refactor (2026-11-11)

Learning library tests passed (36). The full Learning crate test and benchmark commands could not compile the existing binary because the existing learning/src/main.rs:58 has a dangling documentation comment; the source is outside this refactor and was left unchanged. No comparable benchmark result is available. The refactor changes organization of the feedback handler, not its algorithm or persistence workload. WSL2 Linux x86_64, optimized Cargo bench profile.

## Unsupported RSocket interaction handling (2026-11-11)

FeedHandlers tests passed (65). `cargo bench --manifest-path src/rust/feed_handlers/Cargo.toml` completed; benchmarks do not exercise the responder interaction handlers changed here. Feed-processing results were 248.7 ns trade decode/encode, 1.230 / 6.456 / 47.397 us for 7 / 50 / 400-level snapshots, and 1.120 us for incremental updates. Order-book results were 0.81 ns direct reads, 7.38 ns decimal parse, 4.39 ns writes, 170.81 ns update, 5.485 us best-level restore, and 6.555 us sparse snapshot. SBE results were 34.6 ns allocate/encode, 9.5 ns reused buffer, and per-client/share costs of 34.8/48.2 ns (1), 126.3/77.1 ns (4), and 453.3/217.3 ns (16); broadcast to 200/400 subscribers measured 3.908/7.726 us per event. Feed-to-subscriber measured 20.0M deliveries/s (1 subscriber), 98.5M (16), and 16.8M in the slow-consumer case; reported lag was 680,756 deliveries. Compared with the baseline, results are broadly comparable; feed-to-subscriber throughput differs and is a standalone noisy workload, unrelated to this responder change. WSL2 Linux x86_64, optimized Cargo bench profile.

## Historical Insights snapshot compatibility (2026-11-09)

Insights tests passed (38 library, 27 binary), including regression coverage for historical kind-0 snapshots with scalar-only components and differing range-field layouts; AutoTrader tests passed (150). Four serial Insights calculation runs measured 11,685.4 / 12,842.3 / 245.2 / 17.4 / 330.6 / 45.9 ns, 11,989.4 / 12,072.9 / 243.7 / 19.3 / 337.4 / 45.7 ns, 12,262.4 / 12,305.1 / 245.6 / 17.3 / 322.7 / 47.9 ns, and 12,263.8 / 12,250.5 / 241.5 / 17.6 / 331.4 / 48.4 ns for live snapshot, explained snapshot, feature extraction, price-return context, diagnostics, and book snapshot. Compared with the latest comparable run (12,494.0 / 12,004.4 / 318.3 / 17.8 / 333.2 / 48.0 ns), the final run was 1.8% faster / 2.1% slower / 24.1% faster / 1.1% faster / 0.5% faster / 0.8% slower, respectively. These calculations do not exercise snapshot decoding or maintenance archiving, so no change is attributed to the decoder. The same 100 symbols × 1,000 updates/s × 60s book-history workload used 21 MiB RSS (1→21 MiB) and 0.25–0.26s versus 21 MiB and 0.22s in the latest comparable run (0.23–0.24s in the preceding compatibility runs); the modest single-run timing increase is recorded without attributing it to the decoder. WSL2 Linux x86_64, optimized Cargo bench profile.

## Service-reported strategy blockers (2026-11-09)

AutoTrader tests passed (150). The AutoTrader benchmark measured 3.0–19.3 ns for strategy entry evaluation and 1.7–5.0 ns for position evaluation; signal serialization, market-depth assessment, order-request construction, and parameter validation measured 576.3 / 612.6 / 447.1 / 2.0 ns. Compared with the latest comparable run (2.9–17.8 / 1.7–5.3 ns; 529.6 / 572.3 / 426.3 / 1.8 ns), maximum entry timing rose 8.4%, helper timings rose 4.9–11.1%, and maximum position timing fell 5.7%. These microbenchmarks do not cover the new status-response blocker generation or serialization path, so the differences are recorded as single-run variation rather than attributed to this change. WSL2 Linux x86_64, optimized Cargo bench profile.

## Precise live strategy rejection diagnostics (2026-11-09)

Strategies tests passed (64) and AutoTrader tests passed (150). The final AutoTrader benchmark measured live-strategy entry evaluation at 10.7–25.0 ns per strategy (momentum 16.2, trade flow 10.7, order book 13.0, VWAP dislocation 13.3, absorption pressure 25.0), position evaluation at 1.8–4.5 ns, and serialization/depth/order/parameter helpers at 574.5 / 618.4 / 454.9 / 1.8 ns. Keeping reason formatting out of the evaluator restored the live-entry workload from the initial diagnostic-in-evaluator result of roughly 99–179 ns to the expected range. The Strategies crate has no dedicated benchmark target; its 64 tests were ignored in bench mode. Results are single-run microbenchmarks, and the final live-only entry workload is not directly comparable to the aggregate strategy-entry range above. WSL2 Linux x86_64, optimized Cargo bench profile; no services were started or stopped.

## Multi-horizon Scout outcomes (2026-11-09)

Insights tests passed (32 library, 27 binary); Scouts tests passed (66). React app tests and typecheck passed. Two Insights calculation runs measured 13,193.9 / 15,268.6 / 273.6 / 18.5 / 418.8 / 51.5 ns, then 13,854.9 / 12,699.1 / 308.5 / 32.5 / 446.0 / 51.1 ns for live snapshot, explained snapshot, feature extraction, price-return context, diagnostics, and book snapshot. The latest run is 1.4–80.6% above the latest comparable baseline (11,398.7 / 12,519.7 / 266.1 / 18.0 / 343.0 / 44.3 ns), but repeated runs varied materially, especially price-return context (18.5–32.5 ns). The book-history reproduction used 1→21 MiB RSS and 0.24 s versus 1→20 MiB and 0.23 s. These workloads do not exercise Scout registration or multi-horizon tracking, so they cannot attribute those differences to this change; record them as noisy run variation, not a Scout-path regression. Scouts has no dedicated benchmark target. WSL2 Linux x86_64, optimized Cargo bench profile.

## Quote-path execution and adaptive opportunity policies (2026-11-09)

Insights tests passed (33 library, 27 binary); AutoTrader tests passed (150); React tests passed (88), and the React TypeScript build passed. After the legacy record compatibility and diagnostic-context changes, Insights calculation benchmarks measured 12,494.0 / 12,004.4 / 318.3 / 17.8 / 333.2 / 48.0 ns for live snapshot, explained snapshot, feature extraction, price-return context, diagnostics, and book snapshot. Compared with the latest recorded Insights run (13,193.9 / 15,268.6 / 273.6 / 18.5 / 418.8 / 51.5 ns), five workloads were 3.8–21.4% faster and feature extraction was 16.3% slower. Earlier repetitions varied materially; these single-run results do not establish a stable regression, and the workloads do not exercise legacy decoding. Book-history reproduction used the same 100 symbols × 1,000 updates/s × 60s workload and retained 21 MiB RSS (1→21 MiB) in 0.22s versus 0.24s previously.

AutoTrader entry/position timings ranged 3.1–19.7 / 1.8–5.0 ns/evaluation across two runs versus the latest comparable 3.0–19.3 / 1.7–5.0 ns. Serialization, market-depth assessment, order construction, and parameter validation measured 570.1 / 608.5 / 436.9 / 2.3 ns, then 545.7 / 614.7 / 443.6 / 2.4 ns, versus 576.3 / 612.6 / 447.1 / 2.0 ns. Results were mixed; parameter validation was 15–20% slower but only 0.3–0.4 ns/operation in absolute terms. These microbenchmarks do not cover the new quote-path simulation or cohort adaptation. WSL2 Linux x86_64, optimized Cargo bench profile.

## Missed-opportunity ingestion and adaptive bands (2026-11-09)

Insights tests passed (25 library, 27 binary); AutoTrader tests passed (147). Insights calculation benches measured 11,398.7 / 12,519.7 / 266.1 / 18.0 / 343.0 / 44.3 ns for live snapshot, explained snapshot, feature extraction, price-return context, diagnostics, and book snapshot. Versus the latest comparable run (13,834.6 / 13,729.8 / 350.5 / 27.9 / 475.7 / 48.4 ns), all workloads were faster by 8.5–35.5%; these benchmarks do not exercise audit scanning or missed-band aggregation. Book-history RSS was 1→20 MiB and elapsed time 0.23 s versus 1→19 MiB and 0.34 s. AutoTrader entry/position ranges were 2.9–17.8 / 1.7–5.3 ns versus 4.2–19.7 / 2.2–5.6 ns; serialization, market-depth assessment, order construction, and parameter validation measured 529.6 / 572.3 / 426.3 / 1.8 ns versus 558.1 / 629.7 / 445.9 / 2.0 ns. No comparable workload regressed. WSL2 Linux x86_64, optimized Cargo bench profile.

## Share the live strategy signal stream with Scout replays (2026-11-08)

AutoTrader tests passed (136), Scouts tests passed (59), Insights tests passed (22 library, 27 binary), and Strategies tests passed (61). Scouts has no dedicated benchmark target; its 59 tests were ignored in bench mode. AutoTrader measurements were 4.2–19.7 ns for entry and 2.2–5.6 ns for position evaluation; serialization, market depth, order construction, and parameter validation were 558.1 / 629.7 / 445.9 / 2.0 ns. Compared with the preceding run (3.9–18.9 / 2.8–6.1 ns; 561.2 / 601.9 / 431.0 / 1.8 ns), maxima varied by -8.2% to +11.1%; these microbenchmarks do not cover the new live-signal cache or Scout replay. Insights calculation times were 13,834.6 / 13,729.8 / 350.5 / 27.9 / 475.7 / 48.4 ns; compared with the latest comparable run (11,350.7 / 12,149.1 / 336.4 / 30.0 / 460.9 / 46.8 ns), snapshot timings rose 13.0–21.9%, extraction 4.2%, diagnostics 3.2%, and book snapshots 3.4%, while return context fell 7.0%. The Insights change only forwards the configured fee rate; these calculation workloads do not exercise it, so the differences are recorded as single-run variation. Book-history RSS stayed at 1→19 MiB and elapsed time was 0.34 s. WSL2 Linux x86_64, optimized Cargo bench profile.

## Replay thesis-failure candidates from live Scout signal history (2026-11-08)

Scouts tests passed (59); Insights tests passed (22 library, 27 binary); AutoTrader tests passed (136). Scouts has no dedicated benchmark target, so all 59 tests were ignored in bench mode. Insights calculation benches measured 11,350.7 / 12,149.1 / 336.4 / 30.0 / 460.9 / 46.8 ns for live snapshot, snapshot with explanations, feature extraction, price-return context, diagnostics, and book snapshot. Compared with the latest comparable run (15,211.6 / 15,913.8 / 314.1 / 22.2 / 386.1 / 47.4 ns), live snapshot and explanations were 23.7–25.4% lower, while feature extraction, return context, and diagnostics were 7.1–35.1% higher; book snapshot was 1.3% lower. The Insights change only forwards its configured taker-fee rate to Scouts; the benchmarked calculations do not cover that field or Scout replay, so these one-run differences are recorded as variation. Book-history RSS stayed at 1→19 MiB and elapsed time was 0.30 s. WSL2 Linux x86_64, optimized Cargo bench profile.

## Ignore scout probes when confirming opposing signals (2026-11-08)

AutoTrader tests passed (136). Benchmarks measured 3.9–18.9 ns for entry and 2.8–6.1 ns for position evaluation; serialization, market-depth assessment, limit-order construction, and parameter validation measured 561.2 / 601.9 / 431.0 / 1.8 ns. Compared with the prior run (3.6–19.8 / 1.8–5.1 ns; 580.7 / 621.9 / 435.6 / 1.8 ns), maximum entry time was 4.5% lower, maximum position time was 19.6% higher due to one strategy measurement, and helper timings were within 1%. The change excludes Scout probe and scout-only signals from the opposing-signal exit trigger; the benchmark does not exercise that exit-maintenance path, so the position outlier is recorded as run variation. WSL2 Linux x86_64, optimized Cargo bench profile.

## Count distinct opposing-signal observations (2026-11-08)

AutoTrader tests passed (136). The strategy-evaluation benchmark measured 3.6–19.8 ns for entry and 1.8–5.1 ns for position evaluation; serialization, market-depth assessment, limit-order construction, and parameter validation measured 580.7 / 621.9 / 435.6 / 1.8 ns. Compared with the prior run (3.1–18.2 / 1.8–5.8 ns; 565.0 / 595.3 / 435.7 / 1.9 ns), maxima were within ±9%, helpers were within ±5%, and order construction was unchanged. This refines the confirmation key to the signal timestamp so repeated polling of one signal cannot count as multiple confirmations; the benchmark does not exercise the exit-maintenance path. WSL2 Linux x86_64, optimized Cargo bench profile.

## Require a fresh opposing signal before a loss-triggered exit (2026-11-08)

AutoTrader tests passed (136), Strategies tests passed (61), and Scouts tests passed (57). AutoTrader entry and position evaluation ranged from 3.1–18.2 ns and 1.8–5.8 ns; serialization, market-depth assessment, limit-order construction, and parameter validation measured 565.0 / 595.3 / 435.7 / 1.9 ns. Compared with the immediately prior run (3.0–18.4 / 1.8–3.4 ns; 560.8 / 587.3 / 440.0 / 1.8 ns), entry timing was similar; helpers were within 1.4%, while one strategy position-evaluation measurement rose from 3.2 to 5.8 ns. These benchmarks do not exercise position maintenance or signal-confirmation processing, and the single outlier is recorded as variation. Strategies and Scouts have no dedicated bench targets; all 61 and 57 test cases were ignored in bench mode. WSL2 Linux x86_64, optimized Cargo bench profile.

## Confirmed thesis-failure exits (2026-11-08)

AutoTrader tests passed (135), Strategies tests passed (61), and Scouts tests passed (57). AutoTrader benchmarks ranged from 3.0–18.4 ns for entry and 1.8–3.4 ns for position evaluation; serialization, market-depth assessment, limit-order construction, and parameter validation measured 560.8 / 587.3 / 440.0 / 1.8 ns. Compared with the latest comparable run (4.1–22.9 / 1.9–4.6 ns; 634.8 / 754.0 / 480.6 / 2.2 ns), maximum timings were 19.7–26.1% lower and helper timings were 8.4–22.1% lower. Strategies and Scouts have no dedicated benchmark targets; their bench commands completed with 61 and 57 tests ignored. The exit checks run in position maintenance, outside the benchmarked strategy evaluation loops. WSL2 Linux x86_64, optimized Cargo bench profile.

## Validate Insights-to-Scouts probe and scenario attribution (2026-11-08)

Insights tests passed (22 library, 27 binary); Scouts tests passed (56). The Insights test expectations now cover its bounded, single-parameter probes and tolerate the extra generated baseline signal; Scout pair expectations include the exit-scenario key so outcomes remain separate. Scouts has no dedicated benchmark target; `cargo bench --manifest-path src/rust/scouts/Cargo.toml` completed with 56 tests ignored. Insights benchmarks measured 15,211.6 / 15,913.8 / 314.1 / 22.2 / 386.1 / 47.4 ns for live snapshot, snapshot with explanations, feature extraction, return context, diagnostics, and book snapshot. Compared with the latest comparable run (11,689.5 / 11,600.2 / 241.8 / 17.4 / 322.2 / 43.9 ns), timings were 8.0–37.2% higher; the book-history reproduction remained at 1→19 MiB RSS and took 0.28 s. This change only updates stale tests, not these calculation paths, so the differences are recorded as single-run variation. WSL2 Linux x86_64, optimized Cargo bench profile.

## Send attributable closed-trade returns to Learning (2026-11-08)

AutoTrader tests passed (132). `cargo bench --manifest-path src/rust/auto_trader/Cargo.toml` completed. Entry evaluations ranged from 4.1–22.9 ns and position evaluations from 1.9–4.6 ns; serialization, market-depth assessment, limit-order construction, and parameter validation measured 634.8 / 754.0 / 480.6 / 2.2 ns. Compared with the latest comparable run (3.3–18.1 / 1.7–4.2 ns; 542.3 / 574.4 / 420.7 / 1.8 ns), maxima rose 26.5% / 9.5%, and helpers rose 14.2–31.3%. The change now passes each completed trade's return from its matched entry notional into the Learning outbox and drops symbol-level reconciliation deltas that cannot be attributed to a complete trade. These benchmarks do not exercise the close/feedback path; the measured increases are recorded but cannot be attributed to this change. WSL2 Linux x86_64, optimized Cargo bench profile.

## Distinguish paper Scout fills and paired comparisons (2026-11-08)

Learning tests passed (31). `cargo bench --manifest-path src/rust/learning/Cargo.toml` completed: database load measured 4.79 ms, decision snapshot 0.12 ms, and persistence 9.82 ms. Compared with the prior same-change run (4.24 / 0.13 / 8.58 ms), load and persistence were 13.0% and 14.5% slower while snapshot was 7.7% faster. The benchmark workloads do not isolate the new outcome-source fields and remain small samples; differences are recorded as run variation. WSL2 Linux x86_64, optimized Cargo bench profile.

## Separate trade and Scout learning evidence (2026-11-08)

Learning tests passed (31). `cargo bench --manifest-path src/rust/learning/Cargo.toml` completed: database load measured 4.24 ms, decision snapshot 0.13 ms, and persistence 8.58 ms. Compared with the latest comparable run (3.52 / 0.10 / 9.84 ms), load and snapshot were 20.5% and 30.0% slower, while persistence was 12.8% faster. This change tags trade, Scout, paired Scout, and unclassified legacy outcomes; profile statistics and strategy multipliers now use closed-trade evidence only. Snapshot remains a small workload at 0.13 ms for 1,000 variants and 200 profiles. These are single-run measurements; no benchmark isolates event-source aggregation. WSL2 Linux x86_64, optimized Cargo bench profile.

## Symbol-scoped Scout parameter evidence (2026-11-08)

Learning tests passed (29). `cargo bench --manifest-path src/rust/learning/Cargo.toml` completed: database load measured 3.52 ms, decision snapshot 0.10 ms, and persistence 9.84 ms. Compared with the latest comparable run (2.61 / 0.10 / 7.34 ms), load and persistence were 34.9% and 34.1% slower while snapshot time was unchanged. The snapshot benchmark covers 1,000 variants and 200 profiles; the change now keeps per-symbol parameter outcome statistics separate, while profile promotion also requires symbol-specific paired evidence. These single-run database differences are recorded as variation; the benchmark does not isolate the new symbol-key aggregation. WSL2 Linux x86_64, optimized Cargo bench profile.

## Incremental dated-carry lifecycle persistence (2026-11-08)

The route-level probe identified one repeating 3.7–4.0 GiB RSS jump: `/dated-carry-inputs` persisted full `outcomes`, `pairs`, and `scout_instances` maps even when few lifecycle keys changed. On that build, one call increased RSS by 3,999,068,160 bytes in 12.282 seconds. Lifecycle persistence now reports changed record keys and updates/deletes only those records. The first instrumented post-fix call increased RSS by 266,240 bytes in 1.881 seconds, confirming that this route-level allocation was removed. Later clean restarts still showed high Scouts RSS, so this fix alone did not solve the overall memory growth; repeated `/outcomes` report construction was then isolated and measured separately below. Regression coverage verifies record-level persistence keys. Full Scouts tests: 54 passed and 2 existing paired-outcome assertions failed. `cargo bench` completed with 56 tests ignored (no dedicated Scouts benchmark). WSL2 Linux x86_64.

## Cache repeated Scout outcomes reports (2026-11-08)

The `/outcomes` handler rebuilt JSON rows for all 100,000 Scout instances on every request, even for a one-row page. On the live dataset, a cold `source=all&status=open&page=0&page_size=1&learning_symbol=BTC-USDT` request took 2.732 s, returned 89,645 bytes, and coincided with RSS rising from 6.243 to 6.707 GiB (+0.464 GiB). Nine same-key requests then returned the same 89,645-byte response in 1.37–2.35 ms each. For the unfiltered 8.24 MB report, repeated same-key requests took 3.0–5.1 ms. Scouts now caches up to four query-specific serialized responses for 90 seconds and holds one async cache lock across misses, so concurrent duplicate report requests share one rebuild. The cache stores reference-counted response bytes and returns them without cloning the full JSON value. After the final clean restart at 09:22:37 UTC, Prometheus RSS was 5.20 GiB at restart, 7.21 GiB within the first minute, then 7.30 GiB at 09:26:30 and 7.44 GiB at 09:27:50 across the next 5-minute scan; there was no 3.7–4.0 GiB scan-sized jump. Scouts health passed. Homepage liveness was 23/26: the other failures were an Insights signal-pipeline timeout, AutoTrader decision-audit health, and the existing Insights maintenance enum decode error. The Scouts test suite had 54 passes and 2 existing paired-outcome assertion failures; `cargo bench` completed with all 56 test cases ignored (no dedicated Scouts benchmark). WSL2 Linux x86_64; live request benchmark used the existing 100,000-instance state.

## Bound Insights-to-Scouts live-input delivery (2026-11-08)

Follow-up monitoring disproved the earlier claim that coalesced outbox flushes fixed the production leak: after the Scouts restart, RSS climbed back to 47.1 GiB while `/data/scouts_state.sqlite` remained 1.2 GiB. The remaining source was an unbounded `tokio::spawn` on every Insights signal batch, cloning complete dashboard snapshots and issuing one Scout request per symbol. Insights now keeps one slim latest dashboard per symbol and uses one worker, preserving per-symbol Scout requests to avoid expanding many strategy profiles in one request. A stress test replaced the same symbol 50,000 times, retained one 146-byte payload, and grew RSS from 1,432 KiB to 2,488 KiB (+1.0 MiB), despite a 20 MiB bulky first dashboard. After rebuilding/restarting Insights, while Scouts stayed running, the homepage liveness check passed 25/26; the only failure was the existing Insights-maintenance enum error. Scouts RSS remained between 17.42 and 17.49 GiB for about four minutes after startup, rather than continuing toward 47 GiB. This is encouraging live evidence, but still a short observation window. Insights tests: 25 passed, 2 existing daily-strategy assertions failed (20 vs. 1,023 probes; 9 vs. 8 candidates). `cargo bench` completed; book-history reproduction retained 6,000,000 rows with 20 MiB RSS. Calculation timings were 11,689.5 / 11,600.2 / 241.8 / 17.4 / 322.2 / 43.9 ns, within variance from the latest comparable run. WSL2 Linux x86_64; focused RSS test used debug profile; standard benches used optimized profile.

## Historical runs through 2026-11-05 (compacted)

Dated detail for runs before 2026-11-06 is condensed here; the original persisted baseline and methodology are retained verbatim later in this file. Historical Rust benchmarks used WSL2 Linux x86_64 and optimized Cargo profiles. Results are single-run unless noted, so only matching workloads and environments should be compared.

Latest comparable readings before 2026-11-06:
- Feed handling (2026-11-04 serial rerun): OKX trade decode/parse/SBE encode 285.0 ns/message; 7/50/400-level snapshots 1.395 / 7.278 / 52.915 us; 5-level update 1.194 us; dedicated fanout 0.1 / 0.2 us/event for 1 / 16 subscribers. Earlier concurrent feed runs were 3.3-9.9% slower than the immediately preceding readings; workload contention and the synthetic capacity-64 slow-consumer case limit attribution to production behavior.
- AutoTrader (2026-11-05): entry evaluation 1.1-16.7 ns and position evaluation 0.5-1.9 ns; serialization/depth/order/parameter helpers 522.7 / 544.1 / 416.4 / 1.8 ns. These evaluator/helper timings do not measure Scout processing.
- Insights (2026-11-05): live snapshot, explained snapshot, feature extraction, return context, input diagnostics, and book snapshot measured 11,200.0 / 11,602.5 / 242.2 / 17.2 / 342.8 / 44.6 ns. Repeated snapshots varied across runs; no stable regression was established.
- Position management (2026-11-05): 100-fill MessagePack/JSON decode 122.3 / 140.4 us and encode 18.5 / 32.4 us; 100k-fill decode 171.9 / 194.2 ms and encode 74.2 / 106.8 ms; concurrent file workload 1,958 ms; aggregation proxy 3.958 ms; indexed lookup/count 49.3 / 52.7 ns. Versus the immediately prior single run, 100k MessagePack and JSON encode increased 24.3% and 15.8%, while decode and the concurrent file workload improved. These standalone serialization/filesystem results do not directly measure the production persistence path.
- Learning (2026-11-05): database load / decision snapshot / state persistence 2.19 / 0.09 / 9.61 ms per iteration; load and persistence were 18.4% and 44.7% above the preceding single run, while decision snapshot was unchanged.
- Persistence comparisons: replacing a 74.5 MB full-state JSON snapshot with normalized SQLite measured 642.30 vs 486.62 ms load and 1,171.91 vs 212.35 ms full write (SQLite write 5.5x slower). A separate targeted SQLite row update averaged 22.31 ms versus 232.50 ms for rewriting JSON, but this compares different scopes and is not a full-save comparison.

The original report remains the source for execution/VWAP results where no newer comparable run was recorded. Scouts and Live Signals had no dedicated performance benchmark targets; their unit tests ran ignored in bench mode. Optional live HTTP/service benchmarks were skipped when BENCH_SERVICE_URL was unset. Early AutoTrader and Insights benchmark attempts encountered compile-time API/initializer errors; later runs completed, so those failures are historical blockers rather than current benchmark results.

## Historical runs from 2026-11-06 to 2026-11-07 (compacted)

Runs used WSL2 Linux x86_64 and optimized Cargo profiles; results were single-run and several were captured with services active. The 2026-11-06 to 2026-11-07 Insights binary repeatedly had one stale subset-probe assertion (10 generated variants versus 1,023 expected); Scouts had two pair-ID/carry lifecycle expectation failures on some runs. These later changed and are historical, not current blockers. Scouts and Strategies have no dedicated timing targets; their bench-mode unit tests were ignored.

Notable measurements: a 50,000-trigger Scout outbox test reduced additional RSS growth from 111.8 MiB to 1.5 MiB (98.6%), but the live Scouts process still grew from about 42 GiB to 47 GiB after a restart, so outbox fan-out was not the production memory leak. Sampling live book history at 100 ms reduced measured process RSS from 1,443 MiB to about 20 MiB, about 1,423 MiB lower, while retaining a 60-second window. The unsampled workload had no comparable sampled implementation until the follow-up.

Last comparable calculations before 2026-11-08: Insights live/explained snapshot, feature extraction, return context, diagnostics, and book snapshot were 11,445 / 11,659 / 269.6 / 17.3 / 326.6 / 47.3 ns. AutoTrader entry/position evaluation was 3.3–18.1 / 1.7–4.2 ns, with serialization/depth/order/parameter helpers at 542.3 / 574.4 / 420.7 / 1.8 ns. Learning load/snapshot/persistence was 2.61 / 0.10 / 7.34 ms. The original persisted baseline below remains the reference for workloads with no later comparable run; newer dated sections preserve subsequent comparable results.

## Follow-up run (2026-11-05)

### Learning persistence (learning)

| Target | Result |
|---|---|
| `cargo bench --manifest-path src/rust/learning/Cargo.toml` | Completed successfully; the crate has no dedicated benchmark targets. Cargo ran the unit-test binaries in bench mode with all 24 tests ignored. |

No comparable Learning benchmark was present in the prior baseline. The persistence change was validated with the 24-test debug test suite, including the legacy JSON migration test; no service was started or stopped.

**Recorded:** 2026-11-02 14:27 UTC
**Status:** All benchmark targets completed successfully. The optional live HTTP benchmark compiled and was skipped because BENCH_SERVICE_URL was unset.

This is the first persisted comparison baseline. Values are single-run microbenchmarks; compare only the same workloads, toolchain, and similar hardware/load. Timings are not CI thresholds. RSS is Linux process high-water RSS. Allocation measurements use a counting global allocator in each benchmark process.

## Environment

- WSL2 Linux x86_64, kernel 5.15.167.4-microsoft-standard-WSL2
- rustc 1.96.0 (ac68faa20 2025-05-25)
- 32 logical CPUs reported by nproc
- Optimized Cargo bench profile

## Results

### Feed handling (feed_handlers)

| Workload | Result |
|---|---:|
| OKX trade JSON decode, parse, SBE encode | 260.2 ns/message; 3.84M messages/s |
| 7-level snapshot decode/load/top-seven/encode | 1.274 us/message |
| 50-level snapshot | 6.745 us/message |
| 400-level snapshot | 48.632 us/message |
| 5-level incremental update on 400-level book | 1.129 us/message |
| Direct two-tick read / decimal parse / two-tick write | 0.81 / 7.59 / 4.53 ns; zero measured allocations |
| Book update (8 writes and parses) | 175.16 ns; zero measured allocations |
| Best-level restore / sparse snapshot over 1,000,001 slots | 5.700 / 6.743 us |
| Allocate + SBE encode + payload | 35.4 ns/update |
| Reused-buffer SBE encode | 10.0 ns/update |
| Per-client encoding, 1 / 4 / 16 subscribers | 33.2 / 116.5 / 463.0 ns/update |
| Shared immutable payload, 1 / 4 / 16 subscribers | 44.3 / 79.9 / 229.8 ns/update |
| Tokio broadcast, 200 / 400 subscribers | 3.960 / 7.830 us/event; 50.5M / 51.1M deliveries/s |
| Dedicated fanout, 1 subscriber | 0.1 us/event; 0 lag; 7 allocations; 5,120 KiB peak additional heap; 7,844 KiB process peak RSS |
| Dedicated fanout, 16 subscribers | 0.3 us/event; 0 lag; 30 allocations; 5,125 KiB peak additional heap; 8,092 KiB process peak RSS |
| Slow-consumer case, capacity 64 | 700,181 lagged deliveries; 19 allocations; 5,124 KiB peak additional heap; 8,092 KiB process peak RSS |

### Insights (insights)

| Workload | Result |
|---|---:|
| Live insight snapshot | 1,831.7 ns/calculation |
| Snapshot including explanations | 2,319.3 ns/calculation |
| Research feature extraction | 143.4 ns/calculation |
| Price return context | 17.4 ns/calculation |
| Market input diagnostics | 236.5 ns/calculation |
| Book snapshot | 44.4 ns/calculation |

### Strategy and auto-trader (auto_trader)

Entry evaluation ranged 1.2-5.2 ns/evaluation; position evaluation ranged 1.1-3.1 ns/evaluation across registered strategies. Signal serialization: 504.3 ns; market depth assessment: 557.1 ns; limit-order request construction: 399.7 ns; parameter validation: 1.9 ns.

### VWAP (execution)

| Workload | Result |
|---|---:|
| Scheduler / order-manager construction | 106.5 / 213.3 ns |
| Child-order calculation | 2.4 ns |
| Market trade: no action / new / modify / cancel | 6.3 / 6.4 / 11.7 / 11.4 ns |
| Fill processing | 9.2 ns |
| Full 78-bin run with fills | 8.4 ns/iteration |
| 64-symbol arrival/partial-fill workload | 1.28M events; 5.0 ns/event; 128k partial-fill events |

### Position management (position_management)

| Workload | Result |
|---|---:|
| Ledger, 100 fills: MessagePack / JSON decode | 121.7 / 133.7 us |
| Ledger, 100 fills: MessagePack / JSON encode | 19.6 / 32.0 us |
| Ledger, 100 fills allocator / memory | 1,117,238 allocations; 490 KiB peak additional heap; 2,916 KiB process peak RSS |
| Ledger, 100,000 fills: MessagePack / JSON decode | 175.0 / 189.0 ms |
| Ledger, 100,000 fills: MessagePack / JSON encode | 64.5 / 102.3 ms |
| Ledger, 100,000 fills allocator / memory | 13,500,237 allocations; 491,302 KiB peak additional heap; 587,544 KiB process peak RSS |
| Concurrent atomic file workload, 100k rows, 4 readers and 1 writer | 1,869 ms; 587,115,500 bytes transferred; 0 invalid reader snapshots |
| 100k-fill position aggregation proxy | 4.385 ms/replay; 2,000,321 allocations; 9 KiB peak additional heap; 11,372 KiB process peak RSS |
| Indexed lookup / position count scan | 58.4 / 53.0 ns |
| Live HTTP health endpoint | Not run; requires BENCH_SERVICE_URL pointing to an already-running service |

## Comparison notes and limits

- This is the first persisted report; future runs should compare against these values. Earlier chat-only figures are not a stable baseline.
- Ledger memory includes fixtures, JSON and MessagePack buffers, and generic serde_json::Value decode outputs. It is not a direct measurement of production typed-ledger RSS.
- Fanout is in-process Tokio broadcast. Lag depends on ring capacity and consumer scheduling; the slow-consumer scenario intentionally overloads a 64-slot ring.
- The concurrent ledger benchmark models atomic file replacement but does not measure the production ledger mutex, filesystem durability, or power-loss behavior.
- Strategy timings measure evaluator speed, not trading returns, fill quality, or robustness across regimes.
- The HTTP benchmark is opt-in and was not run. No services were started or stopped.
- All five cargo bench --manifest-path invocations exited successfully. Crate test binaries also ran in bench mode; tests marked ignored remained ignored.

## Pause strategies with negative net trade evidence (2026-11-08)

Learning tests passed (32 tests). The learning_service benchmark measured 2.74 ms to load 1,000 variants, 200 active profiles, and 5,000 event IDs; 0.12 ms for a decision snapshot; and 8.00 ms to persist the benchmark profile. The latest comparable readings were 1.85–1.96 / 0.09 / 6.09 ms. These single runs are +40–48% / +33% / +31%; load and persistence do not touch the changed snapshot calculation, and the snapshot fixture contains no trade outcomes, so this does not measure the additional outcome aggregation. Snapshot time adds about 0.03 ms per 1,000-variant poll; this is a small absolute cost but a higher relative result that merits a repeated comparison. No targets failed or were skipped. WSL2 Linux x86_64, optimized Cargo bench profile; no services were started or stopped.

## Strategy entry evidence and allocation pause (2026-11-08)

AutoTrader tests passed (138). The strategy_evaluation benchmark measured entry 3.0–17.8 ns, position 1.7–3.7 ns, and serialization/depth/order/parameter helpers 546.3 / 581.3 / 432.9 / 1.8 ns. The latest comparable run measured 4.2–19.7 / 2.2–5.6 ns and 558.1 / 629.7 / 445.9 / 2.0 ns; all reported maxima and helpers were lower. These evaluator microbenchmarks do not exercise the new entry evidence and sizing checks. WSL2 Linux x86_64, optimized Cargo bench profile. No service was restarted.

After the reservation-time pause recheck and position-reducing rebalance handling, AutoTrader tests again passed (138). The repeated benchmark measured entry 3.3–19.1 ns and position 1.7–4.7 ns, with helpers 547.7 / 588.5 / 429.0 / 1.8 ns. Against the preceding run immediately above, entry and position maxima were 7.3% and 27.0% higher; helpers changed +0.3%, +1.2%, -0.9%, and 0%. The benchmark does not exercise the changed reservation or order path, and these are single-run measurements. No services were restarted.

## Persisted strategy pause latch (2026-11-08)

AutoTrader tests passed (139). Strategy evaluation benchmark entry measured 3.9–18.8 ns, position 2.3–5.5 ns, and serialization/depth/order/parameter helpers 560.3 / 603.9 / 428.5 / 1.8 ns. Compared with the prior 3.3–19.1 / 1.7–4.7 ns and 547.7 / 588.5 / 429.0 / 1.8 ns, entry maximum fell 1.6%, position maximum rose 17.0%, and helpers changed +2.3%, +2.6%, -0.1%, and 0%. These single-run microbenchmarks do not cover the changed snapshot merge or persistence behavior. WSL2 Linux x86_64, optimized Cargo bench profile.

## Scout outbox acknowledgement and retry (2026-11-08)

Scouts tests passed (62), including durable acknowledgement during a continuing queue, acknowledgement before a downstream error, and rollback on persistence failure. Cargo bench completed with 62 harness tests ignored; Scouts has no dedicated benchmark workload. The latest comparable Scout bench likewise had no timing measurements, so throughput and persistence overhead cannot be compared. The outbox now persists delivered prefixes in batches of 64 and retries pending work after startup and downstream failures. WSL2 Linux x86_64, optimized Cargo bench profile. The running Scouts service was not restarted.

## Profile and context loss response (2026-11-08)

Learning tests passed (35), AutoTrader tests passed (140), and Strategies tests passed (61). Learning database load / snapshot / persistence measured 3.00 / 0.17 / 8.29 ms versus the latest comparable 2.74 / 0.12 / 8.00 ms: +9.5%, +41.7%, and +3.6%. The snapshot increase is 0.05 ms for 1,000 variants; the fixture has no trade outcomes, so it does not measure aggregation of real context histories. AutoTrader entry / position evaluation ranged 3.0–18.8 / 1.7–3.8 ns; serialization / depth / order / parameter helpers measured 583.9 / 579.5 / 430.8 / 1.8 ns versus 560.3 / 603.9 / 428.5 / 1.8 ns: +4.2%, -4.0%, +0.5%, and unchanged. Those evaluator workloads do not exercise the risk snapshot or order path. Strategies has no dedicated benchmark; its 61 harness tests were ignored in bench mode. Single runs, WSL2 Linux x86_64, optimized Cargo bench profile. No services were restarted.

After restricting named-strategy stops to their learned profile and prioritizing feedback over unrelated registrations, AutoTrader tests passed (142). The repeat benchmark measured entry 3.0–18.3 ns, position 1.7–4.0 ns, and helpers 542.9 / 608.2 / 436.5 / 1.8 ns. Compared with the immediate preceding run (3.0–18.3 / 1.7–3.3 ns; 533.4 / 602.6 / 427.4 / 1.8 ns), position maximum rose 21.2%; helpers changed +1.8%, +0.9%, +2.1%, and 0%. These single-run evaluator benchmarks do not exercise snapshot delivery, order gating or stop selection. WSL2 Linux x86_64, optimized Cargo bench profile. No services were restarted.

After requiring the paired event marker and exempting portfolio reductions from the entry-risk multiplier, Learning tests passed (35) and AutoTrader tests passed (142). Learning load / snapshot / persistence measured 2.70 / 0.12 / 7.51 ms against the preceding 3.00 / 0.17 / 8.29 ms (-10.0%, -29.4%, -9.4%); the fixture has no trade outcomes. AutoTrader entry / position evaluation ranged 3.0–18.9 / 1.7–5.4 ns, with serialization / depth / order / parameter helpers 593.5 / 568.3 / 426.6 / 2.0 ns versus 542.9 / 608.2 / 436.5 / 1.8 ns; entry and position maxima rose 3.3% and 35.0%, while helpers changed +9.3%, -6.6%, -2.3%, and +11.1%. These single-run evaluator measurements do not cover the modified feedback, risk sizing or order path. WSL2 Linux x86_64, optimized Cargo bench profile. No services were restarted.

## Post-exit evidence preservation (2026-11-08)

AutoTrader tests passed (142). The strategy evaluation benchmark measured entry 3.8–18.5 ns and position 1.7–4.5 ns, with serialization / depth / order / parameter helpers at 578.0 / 622.6 / 440.2 / 1.8 ns. Against the latest comparable run (3.0–18.9 / 1.7–5.4 ns; 593.5 / 568.3 / 426.6 / 2.0 ns), maximum entry and position timings fell 2.1% and 16.7%; helper timings changed -2.6%, +9.6%, +3.2%, and -10.0%. These single-run evaluator benchmarks do not exercise post-exit observation collection or audit serialization. WSL2 Linux x86_64, optimized Cargo bench profile. Services were restarted before this source change.

## Scout paired opportunity validation (2026-11-08)

Scouts tests passed (63). Cargo bench completed with 63 harness tests ignored; Scouts has no dedicated benchmark workload. The latest comparable run also had no timing measurements, so performance cannot be compared. The changed pairing validation is not measured. WSL2 Linux x86_64, optimized Cargo bench profile. The Scout service was not restarted after this change.

## Contextual stop and cooldown profiles (2026-11-08)

Learning tests passed (35), AutoTrader tests passed (143), Strategies tests passed (62), and Scouts tests passed (63). Learning load / snapshot / persistence measured 2.82 / 0.13 / 8.54 ms against the prior 2.70 / 0.12 / 7.51 ms, changes of +4.4%, +8.3%, and +13.7%. The benchmark fixture has no retained exit observations, so it does not measure contextual stop replay work. AutoTrader entry / position evaluation ranged 3.0–18.5 / 1.7–5.3 ns; serialization / depth / order / parameter helpers measured 567.0 / 606.3 / 431.7 / 1.9 ns versus 593.5 / 568.3 / 426.6 / 2.0 ns: -4.5%, +6.6%, +1.2%, and -5.0%. The evaluator benchmark does not execute the post-exit replay or cooldown gate. Scouts and Strategies have no dedicated timing benchmark; all 63 and 62 harness tests were ignored in bench mode. WSL2 Linux x86_64, optimized Cargo bench profile. Services will be restarted after this change.

## Cooldown lineage and evidence-backed mixed exploration (2026-11-09)

Scouts tests passed (64) and AutoTrader tests passed (144). Scouts has no dedicated timing target; cargo bench completed with all 64 harness tests ignored. AutoTrader strategy evaluation measured entry 3.3–24.1 ns and position 1.8–5.5 ns. The maximum entry time rose 30.3% from 18.5 ns and position maximum rose 3.8% from 5.3 ns in the latest comparable run, driven by the existing live-absorption-pressure evaluator and normal single-run variation; the changed profile-lineage and signal-selection paths are not exercised by this evaluator benchmark. Serialization / depth / order / parameter helpers measured 561.4 / 603.6 / 438.3 / 2.0 ns versus 567.0 / 606.3 / 431.7 / 1.9 ns: -1.0%, -0.4%, +1.5%, and +5.3%. The change retains evidence across cooldown-only profile variants and permits a 5%-notional mixed-regime exploration only after positive directional walk-forward evidence; those service paths have no representative benchmark workload. WSL2 Linux x86_64, optimized Cargo bench profile. No services were restarted.

## Validated strategy strength-floor relaxation (2026-11-09)

AutoTrader tests passed (145). The strategy evaluation benchmark measured entry 3.7–18.2 ns and position 1.8–4.4 ns, with serialization / depth / order / parameter helpers at 583.0 / 632.8 / 438.7 / 1.8 ns. Compared with the preceding run (3.3–24.1 / 1.8–5.5 ns; 561.4 / 603.6 / 438.3 / 2.0 ns), entry and position maxima fell 24.5% and 20.0%; helpers changed +3.8%, +4.8%, +0.1%, and -10.0%. The new validated-profile selection path is not exercised by this evaluator microbenchmark. WSL2 Linux x86_64, optimized Cargo bench profile. AutoTrader will be rebuilt and restarted after this change.

## Short-hold evidence and expanded Scout probes (2026-11-09)

AutoTrader tests passed (146) and Scouts tests passed (64). The strategy evaluation benchmark measured entry 3.4–18.2 ns and position 1.7–4.4 ns, with serialization / depth / order / parameter helpers at 544.3 / 579.7 / 426.8 / 1.8 ns. Compared with the preceding 3.7–18.2 / 1.8–4.4 ns and 583.0 / 632.8 / 438.7 / 1.8 ns, entry and position maxima changed -0.0% and -0.0%; helpers changed -6.6%, -8.4%, -2.7%, and 0%. These evaluator microbenchmarks do not measure horizon selection or Scout planning. Insights already emits 60-second outcomes; AutoTrader now prefers that horizon for short-hold live strategies, with a 300-second fallback. Scout live batches now select up to five bounded parameter variants and two distinct profiles per strategy/symbol/direction while preserving symbol-scoped attribution. WSL2 Linux x86_64, optimized Cargo bench profile. Services were not restarted.

## Learning threshold refresh backoff (2026-11-09)

AutoTrader tests passed (146). The strategy evaluation benchmark measured entry 3.1–21.3 ns and position 1.8–5.5 ns, with serialization / depth / order / parameter helpers at 573.2 / 628.3 / 443.6 / 1.9 ns. Compared with the preceding 3.4–18.2 / 1.7–4.4 ns and 544.3 / 579.7 / 426.8 / 1.8 ns, evaluator maxima changed within single-run variance; these workloads do not exercise Learning refreshes. AutoTrader now gives Learning context-threshold requests a 2-second timeout, refreshes each cached context no more often than every 5 seconds, and retains successful responses for 60 seconds to prevent request storms. WSL2 Linux x86_64, optimized Cargo bench profile.

## Scout horizon feedback into Learning (2026-11-09)

Learning tests passed (36) and Scouts tests passed (67). The Learning service benchmark measured database load at 3.39 ms/iteration (1,000 variants, 200 active profiles, 5,000 event IDs), decision snapshot at 0.14 ms/iteration (1,000 variants, 200 active profiles), and state persistence at 8.06 ms/iteration (1,000 variants, 5,000 event IDs). No comparable result for this Learning benchmark was recorded, so no performance delta can be claimed. Its fixtures contain no outcome history and do not exercise horizon-group aggregation or paired promotion. Scouts has no dedicated benchmark target. WSL2 Linux x86_64, optimized Cargo bench profile.

## Learning effectiveness summary (2026-11-09)

Learning tests passed (36). The Learning service benchmark measured database load at 3.70 ms/iteration (1,000 variants, 200 active profiles, 5,000 event IDs), decision snapshot at 0.16 ms/iteration (1,000 variants, 200 active profiles), and state persistence at 8.26 ms/iteration (1,000 variants, 5,000 event IDs). Compared with the latest same-workload run (3.39 / 0.14 / 8.06 ms), these single-run measurements were 9.1% / 14.3% / 2.5% slower; benchmark fixtures do not include outcome history and do not isolate the new active-profile trade summary. WSL2 Linux x86_64, optimized Cargo bench profile.

## Learning Scout RSocket coordinator and per-strategy Scout partitions (2026-11-11)

Learning tests passed (41), Scouts tests passed (74), and React tests passed (108). The Learning service benchmark measured database load at 2.32 ms/iteration (1,000 variants, 200 profiles, 5,000 event IDs), decision snapshot at 0.11 ms/iteration (1,000 variants, 200 profiles), and state persistence at 6.38 ms/iteration (1,000 variants, 5,000 event IDs). Compared with the latest same-workload run (3.70 / 0.16 / 8.26 ms), these single-run readings were 37.3% / 31.3% / 22.8% lower; these workloads do not exercise Scout assignment routing or outcomes aggregation. Scouts has no dedicated timing benchmark; `cargo bench` completed with all 74 unit tests ignored. Insights verification was blocked by the independent `NewsFeedSnapshot` import compile error in `signal_engine/store/browser_rsocket.rs`. WSL2 Linux x86_64, optimized Cargo bench profile. No services were started or restarted.

## Learning owns strategy threshold walk-forward (2026-11-11)

Learning tests passed (43 library, 1 binary); AutoTrader tests passed (154 library, 7 binary). Learning database load / decision snapshot / state persistence measured 2.93 / 0.14 / 7.00 ms against the latest same-workload run (2.32 / 0.11 / 6.38 ms): +26.3% / +27.3% / +9.7%. These single-run workloads contain no threshold outcome series and do not measure Learning's walk-forward scorer. AutoTrader strategy evaluation measured entry 3.2–23.7 ns and position 1.8–5.1 ns, with serialization / depth / order / parameter helpers at 596.4 / 646.9 / 429.7 / 2.1 ns. Compared with the latest comparable AutoTrader run (3.1–21.3 / 1.8–5.5 ns; 573.2 / 628.3 / 443.6 / 1.9 ns), entry maximum rose 11.3%, position maximum fell 7.3%, and helpers changed +4.0%, +3.0%, -3.1%, and +10.5%. These microbenchmarks do not exercise profile refresh or threshold-history handling; the refactor removes that duplicate AutoTrader history path, while threshold observations still go to Learning. Single-run measurements, WSL2 Linux x86_64, optimized Cargo bench profile. No services were started or restarted.

## Strategy worker process metrics (2026-11-11)

Strategy service-feature tests passed (154). Cargo bench completed with all 154 unit tests ignored and no dedicated timing workload, so request instrumentation overhead is not measured and there is no comparable timing result. The change adds a separate internal HTTP listener and atomic counters/histogram to each live and Scout worker; RSS and CPU are read only when the metrics endpoint is scraped. Python exporter tests passed (6), and the Grafana dashboard JSON parsed successfully. WSL2 Linux x86_64, optimized Cargo bench profile. No services were started or restarted.

## Strategy worker extraction and bounded memory follow-up (2026-11-12)

Verification after the crate rename and memory-bound changes: AutoTrader tests passed (162 library, 7 binary), Learning (58 library, 1 binary), Insights (40 library, 40 binary), FeedHandlers (63), Strategy (160 with service), and exporter tests (7). The full Compose stack was rebuilt against the existing named state volumes; homepage liveness passed 17/17. Prometheus reported 34/34 Strategy worker metric series.

The current benchmark run is not directly comparable to isolated baselines: the full system and infrastructure stacks were active with live feed/worker traffic, and the first five benchmark commands also overlapped each other. AutoTrader measured entry 2.9–17.9 ns and position 1.7–3.0 ns, with serialization/depth/order/parameter helpers at 530.3 / 556.7 / 410.3 / 1.8 ns. Learning's initial overlapping run measured load/snapshot/persistence at 33.79 / 0.12 / 18.60 ms. A serial rerun while the live stack remained active measured 26.53 / 0.13 / 16.47 ms versus the latest prior full-stack run of 8.32 / 0.12 / 6.78 ms. The load and persistence differences are material and remain an open benchmark follow-up; service CPU usage and active traffic make attribution to the retention changes unreliable. Insights measured live snapshot / explained snapshot / research extraction / return context / diagnostics / book snapshot at 12,763.7 / 13,002.0 / 242.9 / 20.1 / 321.5 / 44.1 ns. FeedHandlers measured trade processing 485.8 ns/message, 7/50/400-level snapshots 2.567 / 15.372 / 111.494 us, and incremental update 1.885 us; the active workloads and build contention make these single runs unsuitable for regression claims. Strategy has no dedicated timing target; its 158 service tests were ignored in bench mode. WSL2 Linux x86_64, optimized Cargo profile.

Early post-rebuild memory snapshots were AutoTrader 594 MiB, Insights 2.64 GiB, and Learning 635 MiB. The maximum sampled Scout worker RSS was 1.08 GiB against the updated 1.5 GiB cap (72%). These are startup/short-window measurements, not peak-load acceptance results. Re-run comparable benchmarks and collect longer steady-state and Scout/replay peak RSS without disrupting the running stack.

After observing several Scout workers near 1 GiB RSS with only a few hundred MiB of persisted records, the serialized `/outcomes` response cache was found to have a count cap but no byte cap. It now caches responses up to 4 MiB each, 8 MiB in total, for 30 seconds, and serves larger responses without caching. Strategy tests passed (159 at that deployment), including byte/entry-cap coverage. After rebuilding and restarting the dependency graph, a short 90-second sample showed AutoTrader 1.4 GiB, Insights 1.16 GiB, Learning 226 MiB, and maximum Scout worker RSS 1.06 GiB; several Scout workers were below 150 MiB. Startup and active feed traffic were still changing, so this is not a stable before/after attribution. Homepage liveness passed 17/17 after the restart. WSL2 Linux x86_64, optimized Cargo profile for benchmarks.

The same review found the Strategy worker histogram exporter was adding cumulative bucket counters a second time. It now emits the already-cumulative buckets directly, with regression assertions for known observation counts. Strategy tests passed (159 at that deployment) and the 34 live/Scout workers were rebuilt with `--no-deps`; homepage liveness passed 17/17. A read-only 85-request smoke check across the 17 live worker endpoints measured a 0.527 ms median and 8.212 ms p95 round trip, including connection setup. The corrected worker-side live p95 reached 0.95 ms; Scout p95 was 1.0 s under real Scout work. The previous p95 Prometheus samples are invalid because they include the old bucket encoding. No end-to-end comparison with the old in-process path is available.

AutoTrader memory follow-up: its pending signal queue retained the full multi-symbol `markets` map on every per-symbol queued dashboard. Queue admission now strips each item down to the matching symbol book, merges those books when forming a processing batch, and lowers the default unique-symbol backlog from 4,096 to 256 (the active feed universe is 100). AutoTrader tests passed (161 library, 7 binary). After deployment with feed traffic continuing, process RSS was 347 MiB shortly after startup and 381 MiB about a minute later, versus 2.91 GiB before the change. The reduction strongly implicates duplicated book maps as the main AutoTrader RSS source; longer burst/steady-state monitoring is still needed. The AutoTrader evaluator benchmark measured entry 3.3–18.9 ns, position 2.0–3.6 ns, and serialization/depth/order/parameter helpers 572.8 / 584.0 / 428.1 / 1.8 ns. Compared with the previous active-stack run (entry 2.9–17.9 ns, position 1.7–3.0 ns; helpers 530.3 / 556.7 / 410.3 / 1.8 ns), maxima changed by +5.6% / +20%; helper changes were +8.0% / +4.9% / +4.3% / unchanged. This single microbenchmark does not exercise the pending dashboard queue and ran under live feed load. WSL2 Linux x86_64, optimized Cargo profile.

Exporter follow-up: monitoring no longer calls Learning's `/outcomes` route and paginates all Scout reports on each scrape. Each Scout worker exports open/closed counts from a cached, read-only SQLite aggregate refreshed once a minute. This removes repeated full report materialization from Learning and all Scout workers. Worker metric tests passed (160 Strategy library tests total); exporter tests passed (7). After deploying the change and restarting Learning against the same durable volume, its RSS was 63 MiB after one minute and 85 MiB after 13 minutes under active feed/worker traffic, versus 5.28 GiB before the change. Current AutoTrader RSS is 636 MiB after a minute of signal ingress, Insights 1.16 GiB, and maximum Scout RSS 1.03 GiB. These are stable enough to show a large improvement, but burst/replay peak and disk-retention checks remain. Homepage liveness passed 17/17 and Prometheus showed 34 worker series. No dedicated timing benchmark covers the metrics SQL query.

The pending signal queue now strips each dashboard to its signal's market, caps each queued signal/book estimate at 256 KiB, and defaults to 256 distinct symbols, giving a 64 MiB aggregate payload bound. The new queue tests passed (162 AutoTrader library, 7 binary). Its microbenchmark measured entry 2.9–17.8 ns and position 1.7–3.1 ns, with serialization/depth/order/parameter helpers 535.9 / 566.7 / 420.2 / 1.8 ns. These are evaluator helpers; the benchmark does not measure queue admission or per-event serialization. Compared with the prior active-stack run (entry 3.3–18.9 ns, position 2.0–3.6 ns; helpers 572.8 / 584.0 / 428.1 / 1.8 ns), all maxima and helpers were equal or lower. WSL2 Linux x86_64, optimized Cargo profile.

## Improve RSS news outcome classification (2026-11-12)

Insights tests passed (40 library, 44 binary). The classifier recognizes Zcash/ZEC and Kraken-related crypto ecosystem coverage, treats concrete product launches as positive despite incidental or earlier objections, flags material operating revenue declines as bearish despite buyback wording, leaves treasury proposals with offsetting supply and funding effects neutral, and treats crypto underperformance ahead of inflation/tighter-rate news as bearish. The latest Insights calculation run measured 11,275.5 / 11,478.6 / 248.2 / 26.1 / 327.8 / 45.9 ns for live snapshot, explained snapshot, research feature extraction, return context, diagnostics, and book snapshot. Compared with the immediately prior repeat (11,645.2 / 11,570.8 / 253.7 / 20.3 / 323.5 / 44.0 ns), these changed -3.2% / -0.8% / -2.2% / +28.6% / +1.3% / +4.3%. Compared with the latest full-stack result (12,763.7 / 13,002.0 / 242.9 / 20.1 / 321.5 / 44.1 ns), return context was +29.9%; the other workloads were -11.7% / -11.7% / +2.2% / +2.0% / +4.1%. The return-context measurement varied between repeats and the workload does not exercise RSS classification, so this is recorded as a benchmark variation, not attributed to the change. Book-history reproduction retained 6,000,000 rows at approximately 19 MiB RSS in 0.23 s. WSL2 Linux x86_64, optimized Cargo profile.

## AutoTrader signal settings snapshot benchmark (2026-11-12)

AutoTrader tests passed (162 library, 7 binary). The new `signal settings snapshot` benchmark executes the production settings-mutex/gate/profile/snapshot helper with fixed ledgers of 0, 256, and 10,000 retained orders (250 operations each). It measured 9.683 / 63.383 / 2,313.149 us per operation, respectively. This is an uncontended settings-lock benchmark, not a cycle-lock contention test; it includes the settings lock wait sample recording and settings clone. Its synthetic ledger contains compact filled signal orders and does not reproduce other large runtime histories. `cargo bench --manifest-path src/rust/auto_trader/Cargo.toml` completed; all 162 library and 7 binary tests were ignored in bench mode. The existing entry / position evaluator measured 3.0–19.0 / 1.7–3.6 ns; serialization / depth / order / parameter helpers measured 529.0 / 568.8 / 421.9 / 2.0 ns. These evaluator/helper workloads do not exercise the snapshot path. WSL2 Linux x86_64, optimized Cargo profile with the full stack active. No services were restarted.

## Strategy workers consume Insights updates (2026-11-12)

AutoTrader tests passed (158 library, 7 binary), Insights (45), Learning (59 library, 1 binary), Strategy (160 with `service`), React (110), and exporter tests (7). The new daily snapshot status propagation test confirms that evaluated active and inactive symbols are distinguished and unevaluated symbols remain unknown. Compose configuration, Grafana dashboard JSON, `git diff --check`, and Rust formatting passed.

Benchmarks ran with the full service stack active and benchmark commands launched concurrently, so treat them as noisy single-run observations. AutoTrader entry / position evaluation measured 3.4–21.5 / 1.8–5.2 ns; serialization / depth / order / parameter helpers were 534.2 / 663.7 / 427.9 / 1.9 ns. Against the latest active-stack result (3.3–18.9 / 2.0–3.6 ns; helpers 572.8 / 584.0 / 428.1 / 1.8 ns), entry and position maxima rose 13.8% / 44.4%, and helpers changed -6.7% / +13.6% / -0.0% / +5.6%. The current benchmark does not exercise worker dispatch or changed daily-snapshot behavior; repeat under controlled load before attributing these evaluator variations. Signal-settings snapshot results for 0 / 256 / 10,000 orders were 9.659 / 70.306 / 2,403.324 us, respectively, versus 9.683 / 63.383 / 2,313.149 us in the previous run.

Insights measured live snapshot / explained snapshot / research extraction / return context / diagnostics / book snapshot at 13,758.6 / 12,156.0 / 257.3 / 20.9 / 337.3 / 45.6 ns. Against the latest full-stack comparison (12,763.7 / 13,002.0 / 242.9 / 20.1 / 321.5 / 44.1 ns), changes were +7.8% / -6.5% / +5.9% / +4.0% / +4.9% / +3.4%. These workloads do not cover daily snapshot status publication. The book-history reproduction retained six million rows at about 19 MiB RSS in 0.25 s, consistent with the prior 19 MiB / 0.23 s result. Learning load / decision snapshot / persistence measured 21.12 / 0.12 / 13.39 ms; versus the latest full-stack run (26.53 / 0.13 / 16.47 ms), changes were -20.4% / -7.7% / -18.7%. Strategy has no dedicated timing target; its 160 service tests were ignored in bench mode. WSL2 Linux x86_64, optimized Cargo profile. Services remained running during benchmark collection.

Post-cutover callback correction: live Strategy worker decisions now use an unpartitioned AutoTrader RSocket route envelope; the strategy and profile identities remain in the decision payload. AutoTrader rejects strategy-partitioned route envelopes, which prevented live decision callbacks. Strategy tests passed (160), and `cargo bench --manifest-path src/rust/strategy/Cargo.toml --features service` completed with no timing target (160 tests ignored in bench mode). The routing correction has no measured benchmark impact. WSL2 Linux x86_64, optimized Cargo profile.

Worker metric scrape follow-up: all 34 live and Scout metrics endpoints responded during a parallel probe, but several Scout endpoints took 1–4 seconds under their CPU limit while the exporter timeout was 3 seconds. The scrape timeout is now 8 seconds and covered by the exporter unit test. Exporter tests passed (7); after rebuild, Prometheus reported 34/34 worker endpoints up and connected. This is an availability/scrape check, not a timed benchmark.

Scout Insights backpressure isolation: each Scout worker now handles its Insights subscription serially through a lane separate from Learning's one-in-flight assignment/lifecycle capacity. This avoids dropping a market update merely because an assignment is active, while keeping assignment payloads out of an unbounded worker queue. Strategy tests passed (161); `cargo bench --manifest-path src/rust/strategy/Cargo.toml --features service` completed with no timed workload (161 tests ignored). This concurrency change has no benchmark measurement. WSL2 Linux x86_64, optimized Cargo profile.

Scout Insights route latency telemetry (2026-11-12): Strategy tests passed (162), including histogram bucket/count coverage. The Scout worker now exports `app_strategy_worker_insight_route_duration_seconds`, measuring serialization and dispatch through the local handler. The Grafana panel reports pooled p50/p95 in milliseconds; it explicitly excludes Insights stream transport and upstream event delivery. The first live 5-minute window measured 0.56 ms p50 and 77.9 ms p95 across the Scout pool (24,491 route samples at query time); treat this as an initial live sample, not a steady-state baseline. Strategy has no timed bench target; `cargo bench --manifest-path src/rust/strategy/Cargo.toml --features service` completed with all 162 unit tests ignored and no timing measurements. WSL2 Linux x86_64, optimized Cargo profile.

## Avoid repeated Insights Scout registration replay (2026-11-12)

Strategy tests passed (166). The isolated `cargo bench --manifest-path src/rust/strategy/Cargo.toml --features service --bench scout_registration_replay` benchmark exercises the same `RegistrationReplayQueue` used by service replay. With 10,000 synthetic active registrations, 60 intervals, and 500 transient first-pass failures, the repeated-full-replay policy encoded 600,000 request payloads in a 599,599 us median; retry-only encoded 10,500 payloads in 21,275 us, a 98.25% request reduction and 28.2x lower payload-encoding time. Five repetitions; registration set creation/cloning excluded. The benchmark isolates JSON payload encoding and queue policy; it excludes SQLite scans, RSocket transport, Insights handler work, and live-service scheduling. Live diagnosis found 28,378 duplicate Scout registration log lines in a 30-second Insights log window while Insights used about 398% CPU and 1.8 GiB RSS. A 65-second observation of the still-running old implementation sampled Insights at 443–570% CPU and counted a further 27,448 duplicate-registration lines. After the Scout worker rebuild/restart, duplicate-registration logs stopped (zero in a 30-second window), but a paired 30-second Insights CPU sample remained at 541–608%; the replay fix removed the periodic burst but did not lower the overall CPU load. WSL2 Linux x86_64, optimized Cargo profile. Homepage liveness passed 17/17 after the Scout worker restart.

## Route Insights updates only to workers that consume them (2026-11-12)

Insights passed 46 tests and Strategy passed 166 service-feature tests. The Strategy bench suite completed with all 166 tests ignored and no timed Strategy workload. A new Insights feed-event benchmark models a 12,000-price-sample history and 1,000 pending Scout outcomes: `SignalEngine::on_price` / `on_trade` / `on_book` / `snapshot_with_reasons` measured 12,377.5 ns / 10.3 ns / 47.2 ns / 53,294.0 ns per operation. Compared with the immediately preceding same-fixture run (14,186.7 / 10.3 / 45.3 / 53,065.7 ns), values changed -12.8% / 0.0% / +4.2% / +0.4%; these are engine-only calculations and do not exercise RSocket fanout. The stream protocol sends high-frequency market updates to the five live-data evaluators and their five Scouts, instead of all 34 Strategy workers: 70.6% fewer market-update stream recipients. Daily and dated-carry updates stay on the maintenance stream, with initial-state replay and a legacy stream retained.

Post-rebuild observation against the old-stream sample: before the change, Insights used 541.0 / 607.5 / 580.3% process CPU across three 10-second windows, with 11,469 price, 828 trade, and 11,191 book events total. After the change, it used 13.8 / 12.4 / 16.2%, with 11,838 price, 722 trade, and 14,468 book events over the same duration. Mean sampled CPU fell from 576.3% to 14.1% (97.5%); total event volume was 15.1% higher in the after sample. Docker stats reported 15.98% CPU and 292.4 MiB RSS after restart, versus the preceding 1.98 GiB RSS sample. Duplicate Scout registration logs remained at zero in the last 60 seconds. Homepage liveness passed 17/17; all 35 restarted Insights/Strategy containers are healthy. This is a short live sample under active feed traffic, not a long-run baseline. WSL2 Linux x86_64, optimized Cargo profile.

## Provide service-owned fallback entry explanations (2026-11-12)

AutoTrader passed 164 library tests and 7 binary tests. The benchmark measured
entry / position evaluation maxima of 20.9 / 5.6 ns; serialization / depth /
order / parameter helpers measured 605.3 / 669.8 / 444.8 / 2.0 ns. Settings
snapshots at 0 / 256 / 10,000 retained orders measured 11.468 / 79.549 /
3,161.279 us. Compared with the latest comparable same-day run (18.0 / 3.6 ns
entry / position maxima, helpers 527.1 / 686.8 / 420.1 / 1.9 ns, snapshots
9.772 / 66.358 / 2,451.252 us), evaluator, helper and snapshot measurements
varied; this change affects status response construction and the benchmark does
not exercise status serialization. WSL2 Linux x86_64, optimized Cargo bench
profile.

## Reduce live profile refresh contention (2026-11-13)

AutoTrader passed 167 library and 8 binary tests; Strategy passed 169 service-feature tests. The AutoTrader benchmark measured entry / position maxima of 22.2 / 5.9 ns; serialization / market-depth / order-request / parameter-validation helpers measured 623.5 / 739.0 / 467.3 / 2.1 ns; settings snapshots at 0 / 256 / 10,000 orders measured 14.617 / 90.762 / 3,807.465 us. Compared with the immediately preceding same-day run (21.8 / 5.4 ns; helpers 642.2 / 752.1 / 467.5 / 2.5 ns; snapshots 14.970 / 94.447 / 4,056.418 us), values changed +1.8% / +9.3%, helpers -2.9% / -1.7% / -0.0% / -16.0%, and snapshots -2.4% / -3.9% / -6.1%. The benchmark does not measure profile route generation or lock contention.

The Strategy Scout registration replay benchmark measured 18,869 us for 10,000 registrations, 60 intervals, and 500 transient failures (10,500 encoded requests), 6.7% above the previous 17,684 us result. This unrelated synthetic benchmark ran with active, resource-contended services; neither result measures live profile refresh or callback throughput, so the movement is not attributed to this change. WSL2 Linux x86_64, optimized Cargo profiles.

## Push live profiles and decisions over typed RSocket operations (2026-11-14)

AutoTrader passed 168 library and 8 binary tests; Strategy passed 172 service-feature tests. AutoTrader's evaluator benchmark measured entry / position maxima of 22.0 / 4.8 ns; serialization / market-depth / order-request / parameter-validation helpers measured 623.9 / 774.6 / 490.5 / 2.1 ns; settings snapshots at 0 / 256 / 10,000 orders measured 16.963 / 100.632 / 3,700.363 us. Against the immediately preceding same-day run (22.2 / 5.9 ns; helpers 623.5 / 739.0 / 467.3 / 2.1 ns; snapshots 14.617 / 90.762 / 3,807.465 us), values changed -0.9% / -18.6%, helpers +0.1% / +4.8% / +5.0% / unchanged, and snapshots +16.0% / +10.9% / -2.8%. These microbenchmarks do not exercise profile publication or socket decision routing; the stack was active.

The Strategy Scout registration replay benchmark measured 15,041 us for 10,000 registrations, 60 intervals, and 500 transient failures (10,500 encoded requests), 20.3% faster than the latest same-workload result (18,869 us). This benchmark does not exercise live profile pushes or decision reports, so the improvement is not attributed to this change. WSL2 Linux x86_64, optimized Cargo profiles.
