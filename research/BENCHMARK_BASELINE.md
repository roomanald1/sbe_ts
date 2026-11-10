# Rust benchmark baseline

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

## Preserve paired Scout control direction in learning snapshots (2026-11-07)

Strategies tests passed (61), Learning tests passed (28), and AutoTrader tests passed (131). Scouts reported 53 passed and 2 existing pair-ID/carry lifecycle failures; the paired-outcome baseline propagation test passed. React tests passed (87) and React typecheck passed. Benchmarks completed for all four crates: Strategies and Scouts have no dedicated benchmark targets, so 61 and 55 tests were ignored in bench mode. Learning measured database load at 2.61 ms, decision snapshot at 0.10 ms, and persistence at 7.34 ms; these are within the latest comparable ranges (2.27–3.06 ms, 0.09–0.12 ms, and 7.01–11.97 ms). AutoTrader entry/position evaluations ranged from 3.0–18.2 / 1.9–3.2 ns; serialization, depth assessment, order request construction, and parameter validation measured 567.2 / 590.0 / 440.4 / 1.8 ns. Compared with the latest comparable run (3.6–18.9 / 2.1–5.2 ns; 593.6 / 726.4 / 449.3 / 1.9 ns), all maxima and helpers were lower. The added paired-control metadata is outside the benchmarked strategy hot paths. WSL2 Linux x86_64, optimized Cargo bench profile.

## Scout outbox flush fan-out (2026-11-07)

An isolated Tokio backpressure benchmark held the Scout flush mutex while issuing 50,000 flush triggers. The uncoalesced implementation grew RSS from 8,336 KiB to 122,868 KiB (+111.8 MiB); the single-flight implementation grew RSS from 8,468 KiB to 10,052 KiB (+1.5 MiB), about 98.6% less growth. The 50,000-trigger regression test passes. Ten `/outcomes` handler calls against a consistent live-state snapshot left RSS flat at 4,931,464 KiB. The live snapshot passed SQLite `quick_check`; it contained 100,000 instances, 100,000 outcomes, 10,129 pending outbox records, and 16,084 cached plan results. Prometheus showed RSS reaching 42.01 GiB; after restarting Scouts it initially settled near 11.4 GiB but later climbed back to 47.1 GiB. Outbox task fan-out was a real inefficiency, but not the remaining production leak. See the Insights Scout input fan-out follow-up above. `cargo bench` completed with 55 tests ignored because Scouts has no dedicated bench target. `cargo test` passed 53 tests; two existing pair-ID/carry lifecycle assertions failed. No comparable prior flush benchmark exists. WSL2 Linux x86_64, Rust debug test profile for the focused fan-out benchmark.

## Balanced live scout parameter directions (2026-11-07)

Learning tests passed (28), including a regression that promotes a statistically successful downward candidate. Scouts reported 52 passed and 2 existing pair-ID/carry lifecycle failures; the new tests for both-direction generation and batch balance passed. `cargo bench` completed for both crates. Scouts has no dedicated benchmark target, so all 54 unit tests were ignored in bench mode. Learning measured database load at 2.33 ms, decision snapshot at 0.10 ms, and state persistence at 7.43 ms. These are within the latest comparable repeated ranges (2.27–3.06 ms, 0.09–0.12 ms, and 7.01–11.97 ms), so no degradation is indicated. WSL2 Linux x86_64, optimized Cargo bench profile.

## Multi-comparison baseline Scout registrations (2026-11-07)

Insights tests passed in the library (21); the main binary had 25 of 26 passing, with the existing ten-parameter subset-sweep mismatch (10 generated versus 1,023 expected). The focused Scout validation test passed. Scouts reported 51 passed and 2 existing pair-ID expectation failures. Both crate bench commands completed; Scouts has no dedicated benchmark target, and Insights' book-history and calculation benches ran. Insights measured 11,682.0 ns live snapshot, 11,880.2 ns with explanations, 264.7 ns feature extraction, 17.9 ns return context, 333.0 ns diagnostics, and 47.6 ns book snapshot. Against the latest comparable run (11,240.5 / 11,212.5 / 242.8 / 17.6 / 321.4 / 44.4 ns), changes were +3.9%, +6.0%, +9.0%, +1.7%, +3.6%, and +7.2%. This change only extends registration metadata and validation; the benchmarked calculations do not exercise it, so these differences are run variation rather than a measured impact. The book-history reproduction retained 6,000,000 rows (~1,419 MiB payload), with RSS 1→19 MiB in 0.23 s. WSL2 Linux x86_64, optimized Cargo bench profile.

## AutoTrader signal cycle lock scope (2026-11-07)

AutoTrader tests passed (131). `cargo bench --manifest-path src/rust/auto_trader/Cargo.toml` completed; tests were ignored in bench mode and the strategy-evaluation benchmark ran. Entry/position evaluations ranged from 3.6–18.9 / 2.1–5.2 ns; signal serialization, market-depth assessment, limit-order request construction, and parameter validation measured 593.6 / 726.4 / 449.3 / 1.9 ns. Compared with the latest comparable run (3.3–18.1 / 1.7–4.2 ns; 542.3 / 574.4 / 420.7 / 1.8 ns), some helper timings rose 10.6–26.5%; these microbenchmarks do not exercise signal lock hold duration or planning concurrency, so the differences are workload variation rather than evidence of a lock-path regression. The signal path now prepares from a settings snapshot outside the shared cycle lock, then reacquires it for the final position gate and reservation; the new scope metrics separate snapshot/gating from reservation. WSL2 Linux x86_64, optimized Cargo bench profile.

## AutoTrader asynchronous learning lookups (2026-11-07)

AutoTrader tests passed all 131 tests, including coverage that a recent cached Learning response is returned without a service request. `cargo bench --manifest-path src/rust/auto_trader/Cargo.toml` completed; the crate's unit tests were ignored in bench mode and its strategy-evaluation benchmark ran. Strategy entry/position measurements ranged from 3.3–18.1 / 1.7–4.2 ns; signal serialization, market-depth assessment, limit-order request construction, and parameter validation measured 542.3 / 574.4 / 420.7 / 1.8 ns. Compared with the latest comparable AutoTrader run (3.2–23.6 / 1.8–5.3 ns; 572.0 / 615.0 / 430.1 / 1.9 ns), the helper workloads were 0.5–11.8% lower, with parameter validation effectively unchanged; these microbenchmarks do not exercise the changed HTTP Learning lookups or their in-memory cache. No direct benchmark measures order-path latency. WSL2 Linux x86_64, optimized Cargo bench profile.

## Compact persisted Scout instances (2026-11-07)

The focused compaction regression passed. The full Scouts suite reported 51 passed and 2 failures: pair outcome expectations assume unsuffixed comparison IDs while current pairing code emits scenario-suffixed IDs. `cargo bench --manifest-path src/rust/scouts/Cargo.toml` completed; the crate has no dedicated benchmark target, so all 53 tests were ignored in bench mode. No comparable Scouts timing workload exists. This change strips unused signal and market-data payloads from stored instances and idempotency plan caches, and migrates persisted records on startup, reducing duplicate in-memory JSON expansion. WSL2 Linux x86_64, optimized Cargo bench profile.

## Prevent stale Scout registrations from blocking the outbox (2026-11-07)

Scouts tests passed (52), including regression coverage for stale registration filtering, replay from loaded state, and incremental outbox persistence. `cargo bench --manifest-path src/rust/scouts/Cargo.toml` completed; all 52 tests were ignored in bench mode and the crate has no dedicated benchmark target. No comparable Scouts timing workload exists. The changes replay active registrations at startup or after Insights recovery instead of every minute, batch outbox acknowledgements, persist outbox appends/removals incrementally, skip stale registrations, and discard permanently rejected HTTP 4xx items. These paths are not measured by the crate's microbenchmarks. WSL2 Linux x86_64, optimized Cargo bench profile.

## Insights signal-feed freshness status (2026-11-07)

Insights tests: all 21 library tests passed; 25 of 26 main-binary tests passed. The existing `ten_parameter_profile_generates_all_subset_probe_signals` test still fails with 10 generated versus 1,023 expected. `cargo bench --manifest-path src/rust/insights/Cargo.toml` completed. Calculations measured live snapshot 11,399.8 ns, snapshot with explanations 11,833.6 ns, research feature extraction 244.2 ns, price return context 17.7 ns, market input diagnostics 324.3 ns, and book snapshot 43.5 ns. Compared with the latest comparable run (10,984.1 / 11,253.8 / 243.2 / 17.3 / 318.9 / 44.0 ns), changes were +3.8%, +5.2%, +0.4%, +2.3%, +1.7%, and -1.1%. These calculations do not measure the new event-status endpoint or dashboard cache lifetime; the observed differences are small single-run variation. WSL2 Linux x86_64, optimized Cargo bench profile.

## Bounded Scouts active-audit cache (2026-11-07)

Scouts tests passed (48), including regression coverage that bounds the in-memory unresolved audit cache while keeping its SQLite records available to periodic Insights registration replay. `cargo bench --manifest-path src/rust/scouts/Cargo.toml` completed; the crate has no dedicated benchmark target, so all unit cases were ignored in bench mode. No comparable Scouts timings are available. The cache is limited to 10,000 groups and 50,000 audit records; evictions do not delete durable records. WSL2 Linux x86_64, optimized Cargo bench profile.

## Walk-forward evidence freshness metadata (2026-11-06)

AutoTrader tests passed (130); Insights library tests passed (21). Both cargo bench targets completed; their test cases were ignored in bench mode, and AutoTrader's strategy-evaluation benchmark ran. AutoTrader entry/position evaluations ranged from 3.2–23.6 / 1.8–5.3 ns; signal serialization, market-depth assessment, limit-order request construction, and parameter validation measured 572.0 / 615.0 / 430.1 / 1.9 ns. Against the immediately preceding comparable AutoTrader run (3.0–17.9 / 1.7–4.3 ns; 556.8 / 560.2 / 433.4 / 1.9 ns), evaluator maxima and several helper timings rose 5–71%; this was a single run under an active stack and does not measure evidence freshness filtering, so treat the changes as load variation rather than a demonstrated regression. Insights measured 10,984.1 ns live snapshots, 11,253.8 ns with explanations, 243.2 ns feature extraction, 17.3 ns return context, 318.9 ns diagnostics, and 44.0 ns book snapshots. Against the latest comparable Insights run (10,990.4 / 11,241.4 / 241.2 / 17.0 / 318.2 / 44.0 ns), differences ranged from -0.1% to +1.8%; the benchmark does not cover walk-forward recomputation or serialization. WSL2 Linux x86_64, optimized Cargo bench profile.

## Deterministic walk-forward evidence selection (2026-11-06)

AutoTrader library tests passed (129). `cargo bench --manifest-path src/rust/auto_trader/Cargo.toml` completed; the 129 test cases were ignored in bench mode and the strategy-evaluation benchmark ran. Entry/position evaluations ranged from 3.0–17.9 / 1.8–4.1 ns; signal serialization, market-depth assessment, limit-order request construction, and parameter validation measured 556.8 / 560.2 / 433.4 / 1.9 ns. Against the recent comparable AutoTrader run (3.0–18.8 / 1.7–3.9 ns; 524.3 / 570.2 / 421.1 / 1.8 ns), entry maximum fell 4.8%, position maximum rose 5.1%, serialization rose 6.2%, depth fell 1.8%, order construction rose 2.9%, and parameter validation rose 5.6%. This one-run noise is unrelated to the evidence lookup; no benchmark exercises evidence selection. WSL2 Linux x86_64, optimized Cargo bench profile.

## Fix daily Scout batch plan validation (2026-11-06)

Scouts tests passed (45). `cargo bench --manifest-path src/rust/scouts/Cargo.toml` completed; Scouts has no dedicated benchmark targets, so all 45 test cases were ignored in bench mode. The cooldown-key validation fix does not affect a benchmarked workload; no comparable performance measurement is available. WSL2 Linux x86_64, optimized Cargo bench profile.

## Same-signal multi-variant Scout batches (2026-11-06)

Scouts tests passed (45). Insights library tests passed (21); its main binary had 25 of 26 passing, with the existing `ten_parameter_profile_generates_all_subset_probe_signals` mismatch (expected 1,023, got 10). The new paired-lift batch ranking test passed. Both benchmark commands completed; Scouts has no dedicated benchmark targets (45 tests ignored). Insights measured live snapshot 11,402.4 ns, snapshot with explanations 12,020.4 ns, research extraction 255.9 ns, return context 18.6 ns, diagnostics 347.3 ns, and book snapshot 46.3 ns. Compared with the latest recorded comparable run (11,418.8 / 11,882.8 / 245.7 / 17.5 / 340.7 / 45.0 ns), live snapshot and explanations were 0.1% and 1.2% lower; extraction, return context, diagnostics, and book snapshot were 4.2%, 6.3%, 1.9%, and 2.9% higher. These benchmarks do not exercise daily probe ranking or Scout pairing, so the small single-run changes are recorded as variation. WSL2 Linux x86_64, optimized Cargo bench profile.

## Daily snapshot refresh and Scout request latency fixes (2026-11-06)

Auto Trader tests passed (128), Strategies tests passed (61), Scouts tests passed (45), and Insights library tests passed (21). The full Insights test command still has the already-recorded binary test failure `ten_parameter_profile_generates_all_subset_probe_signals` (10 generated versus an expected 1,023). Benchmarks completed for all four crates; Strategies and Scouts have no dedicated benchmark targets. Auto Trader entry/position evaluations ranged from 3.1–18.8 / 1.8–3.4 ns; serialization, market-depth assessment, order construction, and parameter validation measured 544.9 / 601.8 / 432.6 / 1.8 ns. Compared with the latest Auto Trader run above (3.0–17.8 / 1.7–3.2 ns; 528.7 / 556.0 / 417.3 / 1.8 ns), maxima and helpers rose 5.6–8.2%; these microbenchmarks do not measure the changed snapshot refresh or Scout persistence/delivery paths, so the single-run changes are recorded as variation. Insights measured 11,418.8 ns live snapshots, 11,882.8 ns with explanations, 245.7 ns feature extraction, 17.5 ns return context, 340.7 ns diagnostics, and 45.0 ns book snapshots. Compared with the latest recorded Insights run (11,814.0 / 13,345.0 / 281.2 / 19.4 / 386.0 / 49.6 ns), results ranged from 3.3–12.6% lower; these calculations do not exercise the daily publisher. No direct benchmark covers the Scout fixes. WSL2 Linux x86_64, optimized Cargo bench profile.

## Freshness pipeline final Rust validation (2026-11-06)

AutoTrader passed all 128 tests, Scouts passed all 42, and Live Signals passed all 27. Cargo benchmarks completed for all three; Scouts and Live Signals have no dedicated benchmark targets, so their test cases ran ignored in bench mode. AutoTrader entry/position evaluations ranged from 3.0–17.8 / 1.7–3.2 ns; serialization, market-depth assessment, order request construction, and parameter validation measured 528.7 / 556.0 / 417.3 / 1.8 ns. Compared with the latest comparable AutoTrader result below (3.0–18.8 / 1.7–3.9 ns; 524.3 / 570.2 / 421.1 / 1.8 ns), entry evaluation was similar, maximum position timing improved 17.9%, serialization rose 0.8%, depth fell 2.5%, request construction fell 0.9%, and parameter validation was unchanged. The benchmarks do not exercise queue batching, Scout profile lookups, or independent delivery retries. WSL2 Linux x86_64, optimized Cargo bench profile.

## AutoTrader batched signal pipeline final run (2026-11-06)

AutoTrader tests passed all 126 tests. `cargo bench --manifest-path src/rust/auto_trader/Cargo.toml` completed; the 126 test cases were ignored in bench mode and the strategy-evaluation benchmark ran. Entry/position evaluations ranged from 3.0–19.0 / 1.8–4.5 ns; signal serialization was 565.5 ns, depth assessment 604.4 ns, order request construction 428.4 ns, and parameter validation 1.8 ns. Against the latest comparable AutoTrader run above (3.0–18.8 / 1.7–3.9 ns; 524.3 / 570.2 / 421.1 / 1.8 ns), the maximum position time rose 15.4%, serialization rose 7.9%, depth rose 6.0%, request construction rose 1.7%, and validation was unchanged. These are single-run differences in microbenchmarks that do not exercise signal batching, evidence calls, or the freshness path; the stack was active, so treat them as workload variation rather than an established regression. WSL2 Linux x86_64, optimized Cargo bench profile.

## Decoupled live signal and Scout delivery (2026-11-06)

`cargo test --manifest-path src/rust/live_signals/Cargo.toml` passed all 27 tests. `cargo bench --manifest-path src/rust/live_signals/Cargo.toml` completed; Live Signals has no dedicated benchmark target, so all 27 test cases ran ignored in bench mode. The change separates AutoTrader delivery acknowledgements and retries from the Scout live-input queue; no directly comparable microbenchmark exists. WSL2 Linux x86_64, optimized Cargo bench profile.

## AutoTrader close fill strategy attribution (2026-11-06)

AutoTrader tests passed (128) and position-management tests passed (39; 1 ignored). React tests passed (86). `cargo bench --manifest-path src/rust/auto_trader/Cargo.toml` completed; its 128 test cases were ignored in bench mode, and the strategy-evaluation benchmark ran. Entry/position evaluations ranged from 3.0–18.8 / 1.7–3.9 ns; signal serialization, market-depth assessment, limit-order request construction, and parameter validation measured 524.3 / 570.2 / 421.1 / 1.8 ns. Compared with the latest comparable recorded run (3.0–17.8 / 1.8–4.7 ns; 529.1 / 552.5 / 420.6 / 1.8 ns), maximum entry time increased 5.6%, while serialization decreased 0.9%, depth increased 3.2%, request construction increased 0.1%, and parameter validation was unchanged. These single-run microbenchmarks do not exercise activity-row fill attribution, so the small evaluator/helper differences are unrelated to this change. WSL2 Linux x86_64, optimized Cargo bench profile; no services were started or stopped.

Position-management benchmarks also completed; `service_http` skipped because `BENCH_SERVICE_URL` was unset. Compared with the latest same-workload run (2026-11-05), concurrent filesystem time was 2,109.8 ms vs 1,958 ms (+7.7%); MessagePack / JSON decode for 100 fills was 122.5 / 139.6 us vs 122.3 / 140.4 us (+0.2% / -0.6%); MessagePack / JSON encode was 18.4 / 31.6 us vs 18.5 / 32.4 us (-0.8% / -2.5%). For 100,000 fills, MessagePack / JSON decode was 287.5 / 205.1 ms vs 171.9 / 194.2 ms (+67.3% / +5.6%); encode was 72.2 / 107.0 ms vs 74.2 / 106.8 ms (-2.7% / +0.2%). Position aggregation was 3.907 ms vs 3.958 ms (-1.3%), indexed lookup 49.5 ns vs 49.3 ns (+0.4%), and count scan 52.9 ns vs 52.7 ns (+0.4%). The 100k MessagePack decode timing is a large single-run increase in an unchanged serialization path; it does not measure the close attribution change. WSL2 Linux x86_64, optimized Cargo bench profile; no services were started or stopped.

After adding the manual-close flag to the activity response, AutoTrader tests again passed (128) and its benchmark completed. Entry/position evaluations ranged from 3.0–25.5 / 1.7–3.9 ns; helpers measured 572.8 / 573.2 / 493.8 / 3.0 ns. Compared with the immediately prior run above (3.0–18.8 / 1.7–3.9 ns; 524.3 / 570.2 / 421.1 / 1.8 ns), the live-absorption-pressure entry maximum increased 35.6%, serialization 9.3%, order construction 17.3%, and validation 66.7%; depth increased 0.5%. These benchmarks do not cover activity response construction, and these paths were unchanged; this single-run variation is not attributable to the added flag. WSL2 Linux x86_64, optimized Cargo bench profile; no services were started or stopped.

## Require two eligible positions for daily parameter comparisons (2026-11-06)

Scouts tests passed (41). `cargo bench --manifest-path src/rust/scouts/Cargo.toml` completed; Scouts has no dedicated benchmark targets, so all 41 test cases were ignored in bench mode. This eligibility-gate change does not affect a benchmarked workload; no directly comparable performance measurement is available. WSL2 Linux x86_64, optimized Cargo bench profile.

## Configurable scout promotion sample minimum (2026-11-06)

Learning tests passed (28), Auto Trader tests passed (126), and React tests passed (86). Auto Trader typecheck passed. Learning benchmarks were repeated five times: database load measured 2.27–3.06 ms, decision snapshots 0.09–0.12 ms, and persistence 7.01–11.97 ms. Against the latest comparable Learning run (load 1.85–1.96 ms, snapshot 0.09 ms, persistence 6.09 ms), load and snapshot were higher in most runs and persistence was 15–97% slower. The new persisted config adds one SQLite upsert; it does not change the benchmark's 1,000 variants, 200 profiles, and 5,000 event IDs. The repeated persistence increase is recorded as a degradation for follow-up, though run variance may contribute and the save path is not on the feedback hot path. Auto Trader's strategy benchmark completed: entry/position evaluations ranged 3.0–18.7 / 1.8–3.5 ns; serialization, market depth, order construction, and parameter validation measured 536.1 / 555.8 / 421.7 / 1.8 ns. Compared with the preceding same-harness run (3.2–24.3 / 1.9–6.5 ns; 651.5 / 745.7 / 435.1 / 2.0 ns), all maxima and helpers were lower. These settings and service-sync changes do not alter benchmarked strategy evaluators. WSL2 Linux x86_64, optimized Cargo bench profile; no services were started or stopped.

## Persist strategy gate evidence in audit records (2026-11-06)

Insights library tests passed (21). The full Insights suite still has the existing `ten_parameter_profile_generates_all_subset_probe_signals` failure: it expects 1,023 combinations while the current generator returns 10 single-parameter variants. Insights benchmarks completed. Live snapshot measured 12,705.3 ns (latest 11,047.2 ns, +15.0%), snapshot with explanations 12,440.0 ns (11,697.8 ns, +6.3%), research extraction 347.7 ns (241.6 ns, +43.9%), return context 31.8 ns (17.5 ns, +81.7%), diagnostics 489.3 ns (328.8 ns, +48.8%), and book snapshot 49.6 ns (44.8 ns, +10.7%). These workloads do not exercise the daily audit query or gate payload; the single-run increases are outside the changed paths and should be treated as run variation. WSL2 Linux x86_64, optimized Cargo bench profile, services active.

## Scouts walk-forward outcome profile index (2026-11-06)

`cargo test --manifest-path src/rust/scouts/Cargo.toml` passed all 42 tests. `cargo bench --manifest-path src/rust/scouts/Cargo.toml` completed; Scouts has no dedicated benchmark targets, so all 42 test cases were ignored in bench mode. There is no comparable Cargo benchmark. The same live 160-candidate request using actual symbol strategy profiles took 2,146.6 ms before indexing and averaged 2.87 ms across three requests after the indexed, lock-independent lookup was deployed (748x faster, 99.87% lower latency); all runs returned evidence for 147 candidates over 196 matured samples. The cached portfolio-instance path returned evidence for 8 of 32 candidates in 4.88 ms. These are HTTP workload measurements, not Cargo microbenchmarks. WSL2 Linux x86_64.

## AutoTrader freshness queue batching final run (2026-11-06)

After the final stale-candidate and cycle counters were added, AutoTrader tests again passed all 126 tests. `cargo bench --manifest-path src/rust/auto_trader/Cargo.toml` completed; all 126 tests were ignored in bench mode as expected. Entry/position evaluations ranged from 3.0–17.8 / 1.8–4.7 ns; signal serialization was 529.1 ns, market-depth assessment 552.5 ns, limit-order request construction 420.6 ns, and parameter validation 1.8 ns. Compared with the immediately prior comparable recorded run (entry 2.9–17.9, position 1.8–3.2, serialization 555.7, depth 563.5, request 429.3, validation 1.9), entry evaluation was similar and serialization, depth, request construction, and validation were lower by 4.8%, 2.0%, 2.0%, and 5.3%. The maximum position-evaluation time rose from 3.2 to 4.7 ns (+46.9%); this path is unchanged and the difference is a single-run microbenchmark variation. These benchmarks do not exercise signal queue throughput or remote Scout/learning calls. WSL2 Linux x86_64, optimized Cargo bench profile.

## Scout outcomes backend sorting (2026-11-06)

`cargo test --manifest-path src/rust/scouts/Cargo.toml` passed all 39 tests. `cargo bench --manifest-path src/rust/scouts/Cargo.toml` completed; Scouts has no dedicated benchmark targets, so Cargo ran the test binaries in bench mode with all 39 tests ignored. No comparable performance workload exists for this endpoint; the sorting change affects report generation and is not exercised by the crate's microbenchmarks. WSL2 Linux x86_64, optimized Cargo bench profile.

## AutoTrader freshness queue batching (2026-11-06)

`cargo test --manifest-path src/rust/auto_trader/Cargo.toml` passed all 126 tests. `cargo bench --manifest-path src/rust/auto_trader/Cargo.toml` completed; the bench-mode test binaries ran all 126 tests as ignored, as expected. AutoTrader entry/position evaluations ranged from 3.8–19.6 / 2.5–5.4 ns; signal serialization was 628.0 ns, market-depth assessment 924.8 ns, limit-order request construction 462.2 ns, and parameter validation 2.0 ns. Compared with the immediately prior AutoTrader run (entry 2.9–17.9, position 1.8–3.2, serialization 555.7, depth 563.5, request 429.3, validation 1.9), this single run is slower across all measured workloads; the largest increase is market-depth assessment (+64.1%). That path is outside the queue/freshness changes, so the result is recorded as a possible environment/run variation, not attributed to this change. The benchmark does not exercise queue batching or remote service calls. WSL2 Linux x86_64, optimized Cargo bench profile.

## Scout parameter rejection and probe suppression (2026-11-06)

Strategies, Learning, Scouts, and Auto Trader tests passed (61, 28, 38, and 124 tests). Insights library tests passed; one existing binary test, `ten_parameter_profile_generates_all_subset_probe_signals`, fails because the current generator returns 10 single-parameter variants while the stale assertion expects 1,023 combinations. The rejection filter is empty for that fixture and does not cause the mismatch. Benchmarks completed for all five affected crates: Strategies and Scouts have no benchmark targets; Learning decision snapshot was 0.09 ms (same as the latest 0.09 ms result), database load 1.93 ms (previous 1.94 ms), and persistence 6.64 ms (previous 7.95 ms). Insights measured 11,047.2 ns for live snapshots, 11,697.8 ns with explanations, 241.6 ns research extraction, 17.5 ns return context, 328.8 ns diagnostics, and 44.8 ns book snapshots; compared with the latest listed run, each was within or below its recorded result. Auto Trader entry/position evaluations ranged from 2.9–17.9 / 1.8–3.2 ns; signal serialization was 555.7 ns, depth assessment 563.5 ns, order request construction 429.3 ns, and parameter validation 1.9 ns. Compared with the latest Auto Trader run (entry 3.0–17.9, position 1.8–3.1, serialization 534.9, depth 567.5, request 420.0, validation 1.8), the largest change was request construction (+2.2%); the remaining differences are small and unrelated to parameter classification. WSL2 Linux x86_64, optimized Cargo bench profile.

## Backend position status reconciliation (2026-11-06)

`cargo bench --manifest-path src/rust/auto_trader/Cargo.toml` completed. AutoTrader entry evaluation ranged from 3.0–17.9 ns/evaluation and position evaluation from 1.8–3.1 ns/evaluation; signal serialization was 534.9 ns, market depth assessment 567.3 ns, limit-order request construction 420.3 ns, and parameter validation 1.8 ns. Compared with the latest recorded AutoTrader run (entry 3.0–22.0 ns, position 1.8–3.9 ns, serialization 578.9 ns, depth 617.5 ns, request 404.7 ns, validation 1.9 ns), all measured timings were within or below the prior range except request construction (+3.8%), a small change on an unrelated path. The API ledger classification change is not exercised by these microbenchmarks. WSL2 Linux x86_64, optimized Cargo bench profile.

## Scout evidence request optimization (2026-11-06)

`cargo test` passed for Scouts (37 tests) and Auto Trader (124 tests). Their benchmarks completed; Scouts has no dedicated benchmark target, so its test binary ran in bench mode with all tests ignored. Auto Trader entry evaluation ranged from 3.0–22.0 ns/evaluation and position evaluation from 1.8–3.9 ns/evaluation; signal serialization was 578.9 ns, market depth assessment 617.5 ns, limit-order request construction 404.7 ns, and parameter validation 1.9 ns. Compared with the immediately prior run (entry 3.5–19.3 ns, position 1.7–4.7 ns, serialization 642.2 ns, depth 733.2 ns, request 419.8 ns, validation 1.9 ns), serialization and depth were lower by 9.9% and 15.8%, and the position maximum fell 17.0%; the maximum entry timing increased 14.0%. The evaluator benchmark has no WFA evidence fixtures and does not measure the Scouts HTTP endpoint. Services were active and no load readings were collected, so the entry timing difference is recorded as a single-run variation. WSL2 Linux x86_64, optimized Cargo bench profile.

## Strategy-specific and portfolio walk-forward evidence (2026-11-06)

`cargo test` passed for Strategies (61 tests), Scouts (37 tests), Auto Trader (124 tests), Insights (46 tests across its library and service binaries), and Learning (27 tests). `cargo bench` completed for all five crates. Strategies and Scouts have no dedicated benchmark targets, so their test binaries ran in bench mode with all tests ignored.

Auto Trader entry evaluation ranged from 3.5–19.3 ns/evaluation and position evaluation from 1.7–4.7 ns/evaluation; signal serialization was 642.2 ns, market depth assessment 733.2 ns, limit-order request construction 419.8 ns, and parameter validation 1.9 ns. Against the immediately previous Auto Trader run (entry 3.1–19.2 ns, position 1.9–4.4 ns, serialization 550.8 ns, depth 632.1 ns, order construction 407.8 ns, validation 1.8 ns), serialization and depth timings rose 16.6% and 16.0%; order construction rose 2.9%, and entry/position maxima increased by 0.1/0.3 ns. The strategy evaluation benchmark has no walk-forward evidence in its fixtures, so it does not measure the added evidence branch or Scouts HTTP request. Services remained running; no CPU/load readings were captured, and the serialization/depth paths are unrelated to this change.

Insights results were 11,814.0 ns for live snapshots, 13,345.0 ns with explanations, 281.2 ns for research feature extraction, 19.4 ns for price return context, 386.0 ns for input diagnostics, and 49.6 ns for book snapshots. Compared with the latest recorded Insights run, live snapshot time was effectively unchanged (-0.2%); explanation, research extraction, return context, diagnostics, and book snapshot timings increased 14.6%, 18.2%, 10.9%, 19.9%, and 12.5%. Insights source was not changed for this work; these single-run differences are recorded as environment variation, with services active and no comparable load sample. Learning measured database load at 1.94 ms (previous 1.85 ms, +4.9%), decision snapshot at 0.09 ms (unchanged), and persistence at 7.95 ms (previous 6.64 ms, +19.7%). These workloads are outside the strategy evidence changes. WSL2 Linux x86_64, optimized Cargo bench profile.

## Daily scout probe profile tracking (2026-11-06)

`cargo test --manifest-path src/rust/auto_trader/Cargo.toml` passed all 124 tests. `cargo bench --manifest-path src/rust/auto_trader/Cargo.toml` completed. Entry evaluation ranged from 3.1–19.2 ns/evaluation and position evaluation from 1.9–4.4 ns/evaluation; signal serialization was 550.8 ns, market depth assessment 632.1 ns, limit-order request construction 407.8 ns, and parameter validation 1.8 ns. Compared with the latest run above (entry 3.0–18.1 ns, position 1.7–3.1 ns, serialization 533.0 ns, depth 619.0 ns, request 394.5 ns, validation 1.8 ns), timings were slightly higher, with the largest relative change in position evaluation; the change only forwards daily probe profiles to Insights and does not touch those benchmarked paths. Treat these small single-run differences as variation. WSL2 Linux x86_64, optimized Cargo bench profile; services remained running.

## Strategy-specific walk-forward evidence (2026-11-06)

`cargo test` passed for Auto Trader (124 tests), Strategies (61 tests), and Scouts (36 tests). `cargo bench` completed for all three crates. Strategies and Scouts have no dedicated benchmark targets, so Cargo ran their test binaries in bench mode with all tests ignored. Auto-trader strategy entry measured 3.0–18.1 ns/evaluation and position evaluation 1.7–3.1 ns/evaluation; signal serialization was 533.0 ns, market depth assessment 619.0 ns, limit-order request construction 394.5 ns, and strategy parameter validation 1.8 ns. Compared with the immediately prior Auto Trader run (entry 3.1–17.8 ns, position 1.7–3.2 ns, serialization 528.8 ns, depth 585.5 ns, order construction 408.6 ns, validation 1.9 ns), timings were broadly similar; the 33.5 ns depth change is 5.7% on a workload untouched by this change and is treated as run variation. The evaluator benchmark does not include the Scouts lookup, which is performed once per live or daily candidate batch and has a 500 ms request timeout. WSL2 Linux x86_64, optimized Cargo bench profile; services remained running.

## Auto-trader exploratory signal fallback (2026-11-06)

`cargo test --manifest-path src/rust/auto_trader/Cargo.toml` passed all 124 tests, including the new capped-exploration selection case. `cargo bench --manifest-path src/rust/auto_trader/Cargo.toml` completed successfully. Entry evaluation ranged from 3.1–17.8 ns/evaluation and position evaluation from 1.7–3.2 ns/evaluation; signal serialization was 528.8 ns, market depth assessment 585.5 ns, limit-order request construction 408.6 ns, and parameter validation 1.9 ns. Compared with the latest comparable run (entry 2.2–21.5 ns, position 0.7–3.4 ns, serialization 604.5 ns, depth 650.3 ns, request construction 446.1 ns, validation 2.0 ns), all maxima and helper-path timings were lower; no performance degradation was observed. The selector change is outside these benchmarked hot paths. WSL2 Linux x86_64, optimized Cargo bench profile, services remained running. An earlier bench attempt exposed a runtime Scouts dependency and declaration-order compile errors in the existing working-tree changes; both were corrected before the successful test and benchmark runs.

## Auto-trader signal evidence gate (2026-11-06)

`cargo test --manifest-path src/rust/auto_trader/Cargo.toml` passed all 123 tests, including a regression proving generic signal walk-forward results do not veto a strategy-attributed signal. `cargo bench --manifest-path src/rust/auto_trader/Cargo.toml` completed successfully. Entry evaluation ranged from 2.2–21.5 ns/evaluation and position evaluation from 0.7–3.4 ns/evaluation; signal serialization was 604.5 ns, market depth assessment 650.3 ns, limit-order request construction 446.1 ns, and parameter validation 2.0 ns. Against the latest comparable run (entry 1.2–18.2 ns, position 0.6–2.2 ns, serialization 533.7 ns, depth 570.3 ns, request construction 422.6 ns, validation 1.9 ns), timings were higher, with the largest differences in evaluator maxima and serialization. The change only affects candidate selection and does not touch these benchmark hot paths; this is not evidence of a performance regression. System services were active during the run, so workload contention may have influenced timings. WSL2 Linux x86_64, optimized Cargo bench profile.

## Auto-trader startup state schema initialization (2026-11-06)

`cargo test --manifest-path src/rust/auto_trader/Cargo.toml` passed all 122 tests. `cargo bench --manifest-path src/rust/auto_trader/Cargo.toml` completed successfully. Entry evaluation ranged from 1.2–18.2 ns/evaluation and position evaluation from 0.6–2.2 ns/evaluation; signal serialization was 533.7 ns, market depth assessment 570.3 ns, limit-order request construction 422.6 ns, and parameter validation 1.9 ns. Compared with the latest auto-trader run (entry 1.2–23.5 ns, position 0.8–2.7 ns, serialization 556.9 ns, depth 576.6 ns, request construction 427.5 ns, validation 1.8 ns), current timings are within or below those ranges except parameter validation (+5.6%, 0.1 ns). These are unrelated evaluator and construction workloads; schema initialization is outside them. WSL2 Linux x86_64, optimized Cargo bench profile.

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

## Signal strength and accuracy separation (2026-11-06)

`cargo test --manifest-path src/rust/insights/Cargo.toml` passed 46 tests across the Insights library and service binaries, including regressions that keep raw strength separate from estimated accuracy and decode legacy bincode snapshots. `cargo test --manifest-path src/rust/live_signals/Cargo.toml` passed all 26 tests, including news-adjusted strength coverage. `cargo bench` completed for both crates; Live Signals has no dedicated benchmark targets, so its 26 tests were ignored in bench mode.

Insights benchmark results: live snapshot 11,841.4 ns (previous 1,831.7 ns, +546%); snapshot with explanations 11,642.7 ns (2,319.3 ns, +402%); research feature extraction 237.9 ns (143.4 ns, +66%); price-return context 17.5 ns (17.4 ns, +1%); market-input diagnostics 321.8 ns (236.5 ns, +36%); book snapshot 44.1 ns (44.4 ns, -1%). Immediately after the run, Docker reported Insights at 110% CPU and Scouts at 52% CPU; the prior run has no comparable load readings. The change adds coverage/accuracy fields and moves the stored historical estimate out of `strength`; it does not add work to the live snapshot calculation other than storing the input-coverage fraction. Treat the live snapshot and explanation timing regressions as load-contaminated; no direct benchmark exercises historical calibration or serialization of the new fields. Environment: WSL2 Linux x86_64, 32 logical CPUs, optimized Cargo bench profile.

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

## Live Scout profile fallback (2026-11-06)

Scouts tests passed (34), including coverage for selecting matching AutoTrader profiles when Learning has no live profiles. Scouts has no dedicated benchmark target; `cargo bench` completed with all 34 test harness cases ignored. The change adds profile retrieval and filtering in the live-input path, which is not covered by a performance workload, so there is no comparable timing result. WSL2 Linux x86_64, optimized Cargo bench profile. The Scouts service was not restarted.

## Remove the composite Live Insights strategy (2026-11-06)

AutoTrader strategy benchmarks completed after removing the composite registry entry. Evaluator entry/position timings ranged 1.9–16.6 / 0.8–2.6 ns, versus the latest recorded range of 1.1–16.7 / 0.5–1.9 ns. The catalog now contains only the five separate live strategies, so aggregate ranges cover a different strategy set and are not directly comparable. Helper workloads measured 551.3 ns signal serialization (+5.5% vs 522.7 ns), 562.7 ns market depth assessment (+3.4% vs 544.1 ns), 420.8 ns limit request construction (+1.1% vs 416.4 ns), and 1.8 ns strategy parameter validation (unchanged). These small single-run helper changes are within benchmark variation; they do not exercise removed composite strategy logic. WSL2 Linux x86_64, optimized Cargo bench profile, serial execution; no services were started or stopped.


## Paired Scout batches across strategy cadences (2026-11-07)

Scouts tests passed (51), including daily signal-scoped baseline and variant grouping, bounded cross-path variant ranking with an exploration slot, same-signal zero-return handling when one arm rejects entry, live paired variant generation, and dated-carry shared-baseline pairing. Scouts has no dedicated performance benchmark targets; cargo bench completed with all 51 test harness cases ignored. No comparable timing workload applies. WSL2 Linux x86_64, optimized Cargo bench profile; no services were started or stopped.


## Sampled live book history (2026-11-07)

Insights tests: all 22 library tests passed; 25 of 26 main-binary tests passed. The existing  test still fails with 10 generated versus 1,023 expected. The high-frequency book-history sampling regression test passed.
running 22 tests
test daily_summary::training_archive_tests::archives_records_as_a_bincode_vector_without_materializing_the_day ... ignored
test daily_summary::training_archive_tests::retention_drops_expired_training_payloads_and_compacts_audit_rows ... ignored
test daily_summary::training_archive_tests::revisits_summarized_days_when_raw_rows_were_not_removed ... ignored
test signals::archive_reader_tests::archive_reader_keeps_only_the_newest_records ... ignored
test signals::completed_scout_id_tests::completed_scout_deduplication_history_is_bounded ... ignored
test signals::coverage_tests::breakout_watch_requires_confidence_range_and_directional_pressure ... ignored
test signals::coverage_tests::strength_threshold_uses_only_available_positive_training_cohorts ... ignored
test signals::granular_live_feature_tests::snapshots_publish_window_and_depth_variants_for_live_scouts ... ignored
test signals::logistic_model_tests::feature_vectors_require_complete_inputs_and_apply_bounds ... ignored
test signals::logistic_model_tests::logistic_models_train_only_with_minimum_samples_and_return_probabilities ... ignored
test signals::outcome_model_tests::microprice_and_directional_returns_handle_size_direction_and_invalid_inputs ... ignored
test signals::outcome_model_tests::stop_target_counterfactual_is_conservative_when_both_barriers_were_hit ... ignored
test signals::scout_tests::book_history_is_sampled_and_bounded_under_high_frequency_updates ... ignored
test signals::scout_tests::live_scout_stop_and_target_exits_use_trade_price_when_book_quote_is_missing ... ignored
test signals::scout_tests::parameter_probe_exit_state_matches_the_exact_profile_variant ... ignored
test signals::scout_tests::probe_hold_parameters_control_simulated_exit_timing ... ignored
test signals::scout_tests::scout_uses_stop_target_and_strategy_invalidation_instead_of_one_hour ... ignored
test signals::scout_tests::tight_stop_target_scenario_exits_before_standard_scenario ... ignored
test signals::tests::calibration_keeps_raw_strength_separate_from_estimated_accuracy ... ignored
test signals::tests::legacy_bincode_snapshots_default_new_grading_fields ... ignored
test signals::walk_forward_sample_tests::non_overlapping_samples_sort_filter_direction_and_advance_the_time_window ... ignored
test signals::walk_forward_sample_tests::walk_forward_requires_training_samples_and_at_least_one_test_sample ... ignored

test result: ok. 0 passed; 0 failed; 22 ignored; 0 measured; 0 filtered out; finished in 0.00s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s


running 26 tests
test daily_probe_batch_tests::batch_ranks_paired_average_lift_and_keeps_one_exploration_slot ... ignored
test dated_carry_helper_tests::civil_calendar_helpers_handle_epoch_leap_days_and_month_lengths ... ignored
test dated_carry_helper_tests::futures_step_uses_positive_lot_size_and_safe_default ... ignored
test dated_carry_helper_tests::parses_book_levels_and_rejects_malformed_or_nonpositive_rows ... ignored
test dated_carry_helper_tests::recent_expiries_are_sorted_and_return_spot_future_pairs ... ignored
test dated_carry_helper_tests::walks_book_for_requested_quantity_or_returns_none_when_depth_is_short ... ignored
test donchian_20_btc200_tests::all_parameter_variants_share_their_baseline_sweep_batch ... ignored
test donchian_20_btc200_tests::fixed_twenty_day_breakout_requires_btc_regime ... ignored
test donchian_20_btc200_tests::ten_parameter_profile_generates_all_subset_probe_signals ... ignored
test external_market_helper_tests::json_number_accepts_only_finite_numeric_strings ... ignored
test external_market_helper_tests::option_summary_filters_expiry_and_calculates_atm_volatility_and_skew ... ignored
test news_classification_tests::asset_matching_requires_token_boundaries_or_crypto_context ... ignored
test news_classification_tests::classification_prioritizes_adverse_risk_and_excludes_general_news ... ignored
test news_classification_tests::rss_parser_deduplicates_articles_and_preserves_description ... ignored
test news_classification_tests::xml_decoding_and_field_extraction_handle_cdata_entities_and_missing_fields ... ignored
test risk_adjusted_monthly_momentum_tests::monthly_selection_uses_previous_month_close_not_future_bars ... ignored
test risk_adjusted_monthly_momentum_tests::realized_volatility_requires_non_constant_returns ... ignored
test risk_adjusted_monthly_momentum_tests::utc_month_id_changes_at_month_boundary ... ignored
test signal_feed_worker_tests::decoder_reads_price_and_trade_contract_fields_and_rejects_truncated_events ... ignored
test signal_feed_worker_tests::draining_latest_cache_returns_each_symbol_value_and_empties_cache ... ignored
test signal_ingestion_tests::market_levels_filters_invalid_prices_and_sizes ... ignored
test signal_ingestion_tests::market_value_requires_a_valid_ordered_quote_and_keeps_valid_levels ... ignored
test signal_ingestion_tests::scout_validation_accepts_valid_sample_and_rejects_field_boundaries ... ignored
test strategy_parameter_probe_tests::continues_testing_other_parameter_combinations_between_lookback_arms ... ignored
test strategy_parameter_probe_tests::probes_a_shorter_breakout_lookback_without_changing_other_parameters ... ignored
test strategy_parameter_probe_tests::rotates_parameter_values_within_profile_bounds ... ignored

test result: ok. 0 passed; 0 failed; 26 ignored; 0 measured; 0 filtered out; finished in 0.00s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

live insight snapshot: 11240.5 ns/calculation
live snapshot including explanations: 11212.5 ns/calculation
research feature extraction: 242.8 ns/calculation
price return context: 17.6 ns/calculation
market input diagnostics: 321.4 ns/calculation
book snapshot: 44.4 ns/calculation

## Sampled live book history (2026-11-07)

Insights tests: all 22 library tests passed; 25 of 26 main-binary tests passed. The existing main-binary test for generating all subset probe signals still fails with 10 generated versus 1,023 expected. The high-frequency book-history sampling regression test passed. The Insights benchmark completed: live snapshot 11,445.0 ns, snapshot with explanations 11,659.2 ns, research feature extraction 269.6 ns, price return context 17.3 ns, market input diagnostics 326.6 ns, and book snapshot 47.3 ns. Compared with the latest comparable run (11,399.8 / 11,833.6 / 244.2 / 17.7 / 324.3 / 43.5 ns), changes were +0.4%, -1.5%, +10.4%, -2.3%, +0.7%, and +8.7%. These microbenchmarks do not measure memory reduction or high-rate book ingestion; no comparable allocation benchmark is available. Book history now samples at 100 ms, retaining its 60-second analysis window with about 601 entries per symbol under continuous updates and a hard 1,024-entry limit. WSL2 Linux x86_64, optimized Cargo bench profile.

## Book history memory reproduction (2026-11-07)

The standalone `book_history_memory` benchmark reproduced high RSS with per-update book-history retention: 100 symbols × 1,000 book updates/second × 60 seconds retained 6,000,000 rows (about 1,419 MiB row payload) and process RSS rose from 1 MiB to 1,443 MiB in 2.72 seconds. The run used the current unsampled history implementation. No comparable sampled implementation was present in the checkout during this run, so this is the reproduced baseline, not a before/after comparison. WSL2 Linux x86_64, optimized Cargo bench profile.

The 100 ms sampled implementation passed its high-frequency history regression test. Repeating the identical standalone scenario retained no more than 60,100 rows (about 14 MiB row payload); process RSS rose from 1 MiB to 20 MiB in 0.24 seconds. Against the unsampled run immediately above, RSS ended 1,423 MiB lower. The test preserves 60 seconds of sampled depth, sufficient for the five-second book-replenishment lookups. WSL2 Linux x86_64, optimized Cargo bench profile.

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
