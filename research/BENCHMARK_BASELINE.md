# Rust benchmark baseline

## Daily snapshot refresh and Scout request latency fixes (2026-11-06)

Auto Trader tests passed (128), Strategies tests passed (61), Scouts tests passed (44), and Insights library tests passed (21). The full Insights test command still has the already-recorded binary test failure `ten_parameter_profile_generates_all_subset_probe_signals` (10 generated versus an expected 1,023). Benchmarks completed for all four crates; Strategies and Scouts have no dedicated benchmark targets. Auto Trader entry/position evaluations ranged from 3.1–18.8 / 1.8–3.4 ns; serialization, market-depth assessment, order construction, and parameter validation measured 544.9 / 601.8 / 432.6 / 1.8 ns. Compared with the latest Auto Trader run above (3.0–17.8 / 1.7–3.2 ns; 528.7 / 556.0 / 417.3 / 1.8 ns), maxima and helpers rose 5.6–8.2%; these microbenchmarks do not measure the changed snapshot refresh or Scout persistence/delivery paths, so the single-run changes are recorded as variation. Insights measured 11,418.8 ns live snapshots, 11,882.8 ns with explanations, 245.7 ns feature extraction, 17.5 ns return context, 340.7 ns diagnostics, and 45.0 ns book snapshots. Compared with the latest recorded Insights run (11,814.0 / 13,345.0 / 281.2 / 19.4 / 386.0 / 49.6 ns), results ranged from 3.3–12.6% lower; these calculations do not exercise the daily publisher. No direct benchmark covers the Scout fixes. WSL2 Linux x86_64, optimized Cargo bench profile.

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

## Live Signals Scout batch delivery (2026-11-05)

`cargo test --manifest-path src/rust/live_signals/Cargo.toml` passed all 26 tests, including regression coverage for combining symbol updates into batches of at most 20 and for a Scout handler taking longer than the previous 300 ms timeout. `cargo bench --manifest-path src/rust/live_signals/Cargo.toml` completed; the crate has no dedicated benchmark targets, so Cargo ran the 26 unit tests in bench mode with all ignored. No comparable Live Signals benchmark is recorded. WSL2 Linux x86_64, optimized Cargo bench profile. The change reduces Scout requests from one per eligible symbol to one per 20 symbols while leaving AutoTrader delivery per symbol.

## Signal strength and accuracy separation (2026-11-06)

`cargo test --manifest-path src/rust/insights/Cargo.toml` passed 46 tests across the Insights library and service binaries, including regressions that keep raw strength separate from estimated accuracy and decode legacy bincode snapshots. `cargo test --manifest-path src/rust/live_signals/Cargo.toml` passed all 26 tests, including news-adjusted strength coverage. `cargo bench` completed for both crates; Live Signals has no dedicated benchmark targets, so its 26 tests were ignored in bench mode.

Insights benchmark results: live snapshot 11,841.4 ns (previous 1,831.7 ns, +546%); snapshot with explanations 11,642.7 ns (2,319.3 ns, +402%); research feature extraction 237.9 ns (143.4 ns, +66%); price-return context 17.5 ns (17.4 ns, +1%); market-input diagnostics 321.8 ns (236.5 ns, +36%); book snapshot 44.1 ns (44.4 ns, -1%). Immediately after the run, Docker reported Insights at 110% CPU and Scouts at 52% CPU; the prior run has no comparable load readings. The change adds coverage/accuracy fields and moves the stored historical estimate out of `strength`; it does not add work to the live snapshot calculation other than storing the input-coverage fraction. Treat the live snapshot and explanation timing regressions as load-contaminated; no direct benchmark exercises historical calibration or serialization of the new fields. Environment: WSL2 Linux x86_64, 32 logical CPUs, optimized Cargo bench profile.

## Auto-trader paired scout variants in decision profiles (2026-11-05)

`cargo test --manifest-path src/rust/auto_trader/Cargo.toml` passed all 122 tests, including the regression test for forwarding Learning's paired scout variants into strategy parameter profiles. `cargo bench --manifest-path src/rust/auto_trader/Cargo.toml` completed. Strategy entry measurements ranged from 1.2–23.5 ns/evaluation, position measurements from 0.8–2.7 ns/evaluation, signal serialization was 556.9 ns, market depth assessment 576.6 ns, limit request construction 427.5 ns, and parameter validation 1.8 ns. Compared with the prior auto-trader benchmark (entry 1.2–16.6 ns, position 0.6–2.2 ns, serialization 524.0 ns, depth 557.7 ns, request construction 397.5 ns, validation 1.8 ns), these unrelated existing workloads varied upward in this run; the profile forwarding change is outside the measured hot paths, so this is not evidence of a regression. WSL2 Linux x86_64.

## Scout targeted SQLite writes (2026-11-05)

After changing persistence to write only caller-selected sections (and a single row for a new plan result), a three-run optimized Rust measurement on WSL2 Linux x86_64 averaged **22.31 ms per SQLite row update** versus **232.50 ms to serialize and write the full JSON state**. The SQLite write opens the 77 MB database, ensures WAL mode with full synchronous durability, and commits one plan-result row; the JSON path rewrites the 74.5 MB state file. This is a targeted-update comparison, not a full-state save comparison. The first SQLite write on a process may take longer when it switches the existing database into WAL mode.

## Auto-trader sampled price-path checkpointing (2026-11-05)

`cargo test --manifest-path src/rust/auto_trader/Cargo.toml` passed all 121 tests. `cargo bench --manifest-path src/rust/auto_trader/Cargo.toml` could not complete because the existing Scouts dependency has eight `E0061` call-site errors: callers pass a third `persisted_snapshot` argument to the two-argument `persist_scout_state` function. This is outside the auto-trader change. No comparable persistence-throughput benchmark exists; the auto-trader benchmark target does not measure settings snapshot writes.

The change caps price-path-triggered settings snapshot requests at one per minute while retaining five-second in-memory samples. Other state changes still request immediate snapshots. This caps the price-path-only request rate at 1/12 its prior rate; actual write reduction depends on other state changes that also trigger saves. WSL2 Linux x86_64.

## Scout normalized SQLite full-state storage (2026-11-05)

Three-run average against the same 74.5 MB state snapshot, using the optimized Rust test profile on WSL2 Linux x86_64:

| Operation | JSON | Normalized SQLite | Change |
|---|---:|---:|---:|
| Full-state load | 486.62 ms | 642.30 ms | +32.0% |
| Full-state write | 212.35 ms | 1,171.91 ms | 5.5x slower |
| Stored file size | 74,475,890 bytes | 77,115,392 bytes | +3.5% |

The read comparison includes reading and deserializing JSON versus opening SQLite and reconstructing all records. The write comparison serializes/writes JSON versus transactionally replacing all 13,148 normalized SQLite rows. No existing Scout persistence benchmark was available for historical comparison. The SQLite writer currently rewrites every row for every state save, which is the main source of the write cost; this workload measures full-snapshot saves rather than targeted record updates.

## Process metrics endpoints (2026-11-05)

`cargo test --manifest-path src/rust/learning/Cargo.toml --lib` passed all 27 tests. `cargo check --manifest-path src/rust/scouts/Cargo.toml --lib` passed. The Scouts library test suite and benchmark could not compile because the existing `src/scouts/tests/scout_plan.rs` test initializer omits the `legacy_path` field from `ScoutState`.

`cargo bench --manifest-path src/rust/learning/Cargo.toml` completed successfully: database load was 2.19 ms/iteration, decision snapshot 0.09 ms/iteration, and state persistence 9.61 ms/iteration. Against the latest comparable Learning run (1.85 ms, 0.09 ms, and 6.64 ms), load increased 18.4% and persistence 44.7%, while decision snapshot was unchanged. These endpoint additions are outside those benchmark workloads; this is a single-run difference and not evidence of a performance regression. WSL2 Linux x86_64, optimized Cargo bench profile. No services were started or stopped.

## Learning context-threshold unit tests (2026-11-05)

`cargo test --manifest-path src/rust/learning/Cargo.toml --lib` passed all 27 tests. `cargo llvm-cov --manifest-path src/rust/learning/Cargo.toml --lib --summary-only` reported 86.76% line coverage (85.84% regions), up from 81.81% lines (81.01% regions) before the tests. Context coverage increased from 59.64% to 97.00% lines.

`cargo bench --manifest-path src/rust/learning/Cargo.toml` completed successfully. The benchmark harness measured database load at 1.85 ms/iteration, decision snapshot at 0.09 ms/iteration, and state persistence at 6.64 ms/iteration. Compared with the latest recorded learning run, no directly comparable timings were recorded there; this run adds the first timing readings for these workloads. WSL2 Linux x86_64, optimized Cargo bench profile. No services were started or stopped.

## Follow-up run (2026-11-05)

### Auto-trader settings table persistence

| Target | Result |
|---|---|
| `cargo test --manifest-path src/rust/auto_trader/Cargo.toml` | Passed: 138 tests, including normalized settings round-trip and legacy blob migration. |
| `cargo bench --manifest-path src/rust/auto_trader/Cargo.toml` | Completed successfully. Strategy entry timings were 1.2–16.6 ns/evaluation and position timings 0.6–2.2 ns/evaluation across current strategies; signal serialization 524.0 ns, market depth 557.7 ns, limit request construction 397.5 ns, and parameter validation 1.8 ns. |

Compared with the recorded evaluator range, several current strategy entry measurements exceed the old 1.2–5.2 ns range. The current run includes additional registered strategies and is not a like-for-like comparison; the persistence change is outside those evaluator workloads. This benchmark does not measure SQLite persistence throughput or database size.

**Recorded:** 2026-11-05. All benchmark targets completed successfully; tests run in bench mode are ignored as expected.

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

## Follow-up run (2026-11-02 14:28 UTC)

### USD-rate order-path lookup microbenchmark

| Workload | Result |
|---|---:|
| Merge + clone 6-currency cached/stream fixtures | 547.8 ns/op |
| Load premerged 6-currency snapshot | 15.5 ns/op |
| Snapshot lookup speedup | 35.3x |

This workload was run once earlier in the session (522.3 / 15.2 ns/op); the latest run differs by +4.9% / +2.0%, respectively. Treat this as normal single-run variation, not a regression. These figures isolate local rate-map preparation and atomic snapshot access; they do not include the order-cycle HTTP round trip or capacity/account work. No prior persisted result existed for this workload.

## Follow-up run (2026-11-02)

### Auto-trader strategy evaluation

| Workload | Previous baseline | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation | 1.2–5.2 ns/evaluation | 1.2–5.1 ns/evaluation | Within baseline range |
| Strategy position evaluation | 1.1–3.1 ns/evaluation | 1.1–1.9 ns/evaluation | Within baseline range |
| Signal serialization | 504.3 ns/operation | 501.9 ns/operation | -0.5% |
| Market depth assessment | 557.1 ns/operation | 562.0 ns/operation | +0.9% |
| Limit-order request construction | 399.7 ns/operation | 408.5 ns/operation | +2.2% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |

Same auto-trader benchmark target and optimized Cargo bench profile. This change affects shadow sample selection and does not alter benchmarked strategy evaluation paths. Single-run timing differences are within normal variation.

## Follow-up run (2026-11-02)

### Auto-trader strategy evaluation

| Workload | Previous baseline | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation | 1.2–5.2 ns/evaluation | 1.7–5.0 ns/evaluation | Within baseline range |
| Strategy position evaluation | 1.1–3.1 ns/evaluation | 1.1–1.9 ns/evaluation | Within baseline range |
| Signal serialization | 504.3 ns/operation | 501.2 ns/operation | -0.6% |
| Market depth assessment | 557.1 ns/operation | 565.4 ns/operation | +1.5% |
| Limit-order request construction | 399.7 ns/operation | 397.8 ns/operation | -0.5% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |

Same auto-trader benchmark target and optimized Cargo bench profile. This change is in daily signal deduplication and does not alter benchmarked strategy evaluation paths. Single-run timing differences are within normal variation.

## Follow-up run (2026-11-02)

### Auto-trader strategy evaluation

| Workload | Previous baseline | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation | 1.2–5.2 ns/evaluation | 1.2–5.2 ns/evaluation | Within baseline range |
| Strategy position evaluation | 1.1–3.1 ns/evaluation | 1.1–2.0 ns/evaluation | Within baseline range |
| Signal serialization | 501.2 ns/operation | 497.4 ns/operation | -0.8% |
| Market depth assessment | 565.4 ns/operation | 568.6 ns/operation | +0.6% |
| Limit-order request construction | 397.8 ns/operation | 403.0 ns/operation | +1.3% |
| Strategy parameter validation | 1.9 ns/operation | 2.7 ns/operation | +42.1%; single-run variance, benchmark path unaffected |

Same auto-trader benchmark target and optimized Cargo bench profile. This change adjusts scout selection and outcome learning; it does not change strategy evaluator performance. Single-run parameter validation variation is not representative of the changed code path.

## Follow-up run (2026-11-02)

### Auto-trader and Insights

| Workload | Previous baseline | Current run | Change |
|---|---:|---:|---:|
| Auto-trader entry evaluation | 1.2–5.2 ns/evaluation | 1.1–5.5 ns/evaluation | Within single-run variation |
| Auto-trader position evaluation | 1.1–3.1 ns/evaluation | 1.1–2.5 ns/evaluation | Within baseline range |
| Signal serialization | 504.3 ns/operation | 510.3 ns/operation | +1.2% |
| Market depth assessment | 557.1 ns/operation | 569.1 ns/operation | +2.2% |
| Limit-order request construction | 399.7 ns/operation | 399.2 ns/operation | -0.1% |
| Strategy parameter validation | 1.9 ns/operation | 2.0 ns/operation | +0.1 ns |
| Insights live signal snapshot | 1,831.7 ns/calculation | 1,855.7 ns/calculation | +1.3% |
| Snapshot including explanations | 2,319.3 ns/calculation | 2,371.8 ns/calculation | +2.3% |
| Research feature extraction | 143.4 ns/calculation | 147.7 ns/calculation | +3.0% |
| Price return context | 17.4 ns/calculation | 17.6 ns/calculation | +1.1% |
| Market input diagnostics | 236.5 ns/calculation | 229.8 ns/calculation | -2.8% |
| Book snapshot | 44.4 ns/calculation | 46.8 ns/calculation | +5.4% |

Same benchmark targets and optimized Cargo bench profile as the recorded baseline. The change adds scout outcome registration and learning bookkeeping outside these measured calculation loops. Results remain within expected single-run timing variation. No services were started or stopped.

## Follow-up run (2026-11-02)

### Insights after scout outcome tracking

| Workload | Previous baseline | Current run | Change |
|---|---:|---:|---:|
| Live signal snapshot | 1,831.7 ns/calculation | 1,753.6 ns/calculation | -4.3% |
| Snapshot including explanations | 2,319.3 ns/calculation | 2,280.1 ns/calculation | -1.7% |
| Research feature extraction | 143.4 ns/calculation | 141.2 ns/calculation | -1.5% |
| Price return context | 17.4 ns/calculation | 17.4 ns/calculation | No change |
| Market input diagnostics | 236.5 ns/calculation | 221.8 ns/calculation | -6.2% |
| Book snapshot | 44.4 ns/calculation | 47.8 ns/calculation | +7.7% |

Same Insights benchmark target and optimized Cargo bench profile. Variations are within single-run timing noise; the new scout registration work executes outside these benchmarked calculations.

## Final follow-up run (2026-11-02)

### Scout-learning changes, final measurements

| Workload | Previous baseline | Final run | Change |
|---|---:|---:|---:|
| Auto-trader entry evaluation | 1.2–5.2 ns/evaluation | 1.1–5.0 ns/evaluation | Within baseline range |
| Auto-trader position evaluation | 1.1–3.1 ns/evaluation | 1.1–1.9 ns/evaluation | Within baseline range |
| Signal serialization | 504.3 ns/operation | 493.4 ns/operation | -2.2% |
| Market depth assessment | 557.1 ns/operation | 550.4 ns/operation | -1.2% |
| Limit-order request construction | 399.7 ns/operation | 398.4 ns/operation | -0.3% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |
| Insights live signal snapshot | 1,831.7 ns/calculation | 1,823.2 ns/calculation | -0.5% |
| Snapshot including explanations | 2,319.3 ns/calculation | 2,258.8 ns/calculation | -2.6% |
| Research feature extraction | 143.4 ns/calculation | 146.4 ns/calculation | +2.1% |
| Price return context | 17.4 ns/calculation | 18.2 ns/calculation | +4.6% |
| Market input diagnostics | 236.5 ns/calculation | 231.2 ns/calculation | -2.2% |
| Book snapshot | 44.4 ns/calculation | 45.7 ns/calculation | +2.9% |

Same workloads and optimized Cargo bench profiles as the first baseline. This run includes startup replay and idempotent completion tracking; no measured path shows a meaningful regression. No services were started or stopped.

## Follow-up run (2026-11-02)

### Auto-trader hourly scout refresh

| Workload | Previous baseline | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation | 1.2–5.2 ns/evaluation | 1.1–5.0 ns/evaluation | Within baseline range |
| Strategy position evaluation | 1.1–3.1 ns/evaluation | 1.1–1.9 ns/evaluation | Within baseline range |
| Signal serialization | 504.3 ns/operation | 493.5 ns/operation | -2.1% |
| Market depth assessment | 557.1 ns/operation | 554.2 ns/operation | -0.5% |
| Limit-order request construction | 399.7 ns/operation | 390.6 ns/operation | -2.3% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |

Same auto-trader benchmark target and optimized Cargo bench profile. The cooldown change is outside the measured evaluator paths; no meaningful performance regression was observed.

## Follow-up run (2026-11-02)

### Strategy-exit scout scenarios

| Workload | Previous baseline | Current serial run | Change |
|---|---:|---:|---:|
| Auto-trader entry evaluation | 1.2–5.0 ns/evaluation | 1.2–5.6 ns/evaluation | Upper outlier +9.8%; one-run variance |
| Auto-trader position evaluation | 1.1–1.9 ns/evaluation | 1.1–2.3 ns/evaluation | Upper outlier +21%; one-run variance |
| Signal serialization | 493.4 ns/operation | 500.4 ns/operation | +1.4% |
| Market depth assessment | 550.4 ns/operation | 562.9 ns/operation | +2.3% |
| Limit-order request construction | 398.4 ns/operation | 396.6 ns/operation | -0.5% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |
| Insights live signal snapshot | 1,823.2 ns/calculation | 1,838.4 ns/calculation | +0.8% |
| Snapshot including explanations | 2,258.8 ns/calculation | 2,254.7 ns/calculation | -0.2% |
| Research feature extraction | 146.4 ns/calculation | 149.6 ns/calculation | +2.2% |
| Price return context | 18.2 ns/calculation | 17.4 ns/calculation | -4.4% |
| Market input diagnostics | 231.2 ns/calculation | 232.4 ns/calculation | +0.5% |
| Book snapshot | 45.7 ns/calculation | 45.1 ns/calculation | -1.3% |

Both crates ran serially in the same WSL2 environment and optimized Cargo bench profile as the earlier measurements. The two elevated strategy-evaluation maxima were isolated single-run outliers; repeated path benchmarks and Insights calculations remained near baseline. The exit-policy work is outside the measured hot loops.


## Follow-up run (2026-11-02 19:49 UTC)

### Strategy parameter profile and daily signal calculations

| Workload | Previous baseline | Current run | Change |
|---|---:|---:|---:|
| Auto-trader entry evaluation | 1.2–5.0 ns/evaluation | 1.2–5.2 ns/evaluation | Within baseline range |
| Auto-trader position evaluation | 1.1–1.9 ns/evaluation | 1.1–1.9 ns/evaluation | Within baseline range |
| Signal serialization | 493.4 ns/operation | 505.8 ns/operation | +2.5%; small single-run increase |
| Market depth assessment | 550.4 ns/operation | 555.4 ns/operation | +0.9% |
| Limit-order request construction | 398.4 ns/operation | 394.7 ns/operation | -0.9% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |
| Insights live signal snapshot | 1,823.2 ns/calculation | 1,766.9 ns/calculation | -3.1% |
| Snapshot including explanations | 2,258.8 ns/calculation | 2,236.6 ns/calculation | -1.0% |
| Research feature extraction | 146.4 ns/calculation | 145.4 ns/calculation | -0.7% |
| Price return context | 18.2 ns/calculation | 17.0 ns/calculation | -6.6% |
| Market input diagnostics | 231.2 ns/calculation | 225.0 ns/calculation | -2.7% |
| Book snapshot | 45.7 ns/calculation | 44.8 ns/calculation | -2.0% |

Both crate targets used the same optimized Cargo bench profile as the persisted baseline and ran serially. The modified daily strategy calculations are not directly measured by the existing Insights microbenchmarks. No meaningful regression appeared in the measured paths. No services were started or stopped.

## Follow-up run (2026-11-02 19:51 UTC)

### Scout strategy-exit scenarios

| Workload | Latest comparable run (19:49 UTC) | Current serial run | Change |
|---|---:|---:|---:|
| Auto-trader entry evaluation | 1.2–5.2 ns/evaluation | 1.2–5.5 ns/evaluation | +5.8% upper bound; single-run variation |
| Auto-trader position evaluation | 1.1–1.9 ns/evaluation | 1.1–2.0 ns/evaluation | +5.3% upper bound; single-run variation |
| Signal serialization | 505.8 ns/operation | 495.2 ns/operation | -2.1% |
| Market depth assessment | 555.4 ns/operation | 554.7 ns/operation | -0.1% |
| Limit-order request construction | 394.7 ns/operation | 396.6 ns/operation | +0.5% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |
| Insights live signal snapshot | 1,766.9 ns/calculation | 1,770.9 ns/calculation | +0.2% |
| Snapshot including explanations | 2,236.6 ns/calculation | 2,247.0 ns/calculation | +0.5% |
| Research feature extraction | 145.4 ns/calculation | 143.2 ns/calculation | -1.5% |
| Price return context | 17.0 ns/calculation | 16.9 ns/calculation | -0.6% |
| Market input diagnostics | 225.0 ns/calculation | 235.3 ns/calculation | +4.6% |
| Book snapshot | 44.8 ns/calculation | 44.3 ns/calculation | -1.1% |

Both crates ran serially in the same WSL2 environment and optimized bench profiles. The upper evaluator values are small single-run changes; the remaining workloads are near baseline. The new exit simulation runs outside the benchmarked evaluator and calculation loops.

## Follow-up run (2026-11-02 19:53 UTC)

### Scout exit-policy comparison and learning

| Workload | Latest comparable run (19:51 UTC) | Current serial run | Change |
|---|---:|---:|---:|
| Auto-trader entry evaluation | 1.2–5.5 ns/evaluation | 1.1–5.1 ns/evaluation | Within baseline range |
| Auto-trader position evaluation | 1.1–2.0 ns/evaluation | 1.1–2.5 ns/evaluation | Isolated dated-carry upper outlier; other strategies within range |
| Signal serialization | 495.2 ns/operation | 490.7 ns/operation | -0.9% |
| Market depth assessment | 554.7 ns/operation | 554.3 ns/operation | -0.1% |
| Limit-order request construction | 396.6 ns/operation | 394.2 ns/operation | -0.6% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |
| Insights live signal snapshot | 1,770.9 ns/calculation | 1,750.5 ns/calculation | -1.2% |
| Snapshot including explanations | 2,247.0 ns/calculation | 2,212.6 ns/calculation | -1.5% |
| Research feature extraction | 143.2 ns/calculation | 146.5 ns/calculation | +2.3% |
| Price return context | 16.9 ns/calculation | 17.2 ns/calculation | +1.8% |
| Market input diagnostics | 235.3 ns/calculation | 232.7 ns/calculation | -1.1% |
| Book snapshot | 44.3 ns/calculation | 44.5 ns/calculation | +0.5% |

Same workloads, WSL2 environment, and optimized Cargo bench profiles. The isolated dated-carry position timing varied between runs; the strategy-exit work is outside that evaluator. No consistent regression appeared in repeated measurements. No services were started or stopped.

## Follow-up run (2026-11-02 19:58 UTC)

### Scout exit outcomes and strategy feedback

| Workload | Latest comparable run (19:53 UTC) | Current serial run | Change |
|---|---:|---:|---:|
| Auto-trader entry evaluation | 1.1–5.5 ns/evaluation | 1.1–5.0 ns/evaluation | Within baseline range |
| Auto-trader position evaluation | 1.1–2.5 ns/evaluation | 1.1–1.9 ns/evaluation | Within baseline range |
| Signal serialization | 490.7 ns/operation | 519.3 ns/operation | +5.8%; repeated single-run variability |
| Market depth assessment | 554.3 ns/operation | 583.3 ns/operation | +5.2%; repeated single-run variability |
| Limit-order request construction | 394.2 ns/operation | 396.5 ns/operation | +0.6% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |
| Insights live signal snapshot | 1,750.5 ns/calculation | 1,809.6 ns/calculation | +3.4% |
| Snapshot including explanations | 2,212.6 ns/calculation | 2,255.8 ns/calculation | +2.0% |
| Research feature extraction | 143.2 ns/calculation | 145.4 ns/calculation | +1.5% |
| Price return context | 16.9 ns/calculation | 17.5 ns/calculation | +3.6% |
| Market input diagnostics | 232.7 ns/calculation | 228.2 ns/calculation | -1.9% |
| Book snapshot | 44.5 ns/calculation | 44.9 ns/calculation | +0.9% |

Same workloads, serial execution, WSL2 environment, and optimized Cargo profiles. Strategy evaluation ranges improved or stayed within baseline; non-evaluator calculations varied by 0–6% across consecutive single runs, consistent with local timing noise. The new exit tracking and audit writes are outside the benchmarked calculation loops.

## Follow-up run (2026-11-02 20:02 UTC)

### Persistent scout outcome comparison

| Workload | Latest comparable run (19:58 UTC) | Current serial run | Change |
|---|---:|---:|---:|
| Auto-trader entry evaluation | 1.1–5.5 ns/evaluation | 1.1–5.1 ns/evaluation | Within baseline range |
| Auto-trader position evaluation | 1.1–2.5 ns/evaluation | 1.1–1.9 ns/evaluation | Within baseline range |
| Signal serialization | 519.3 ns/operation | 499.5 ns/operation | -3.8% |
| Market depth assessment | 583.3 ns/operation | 552.8 ns/operation | -5.2% |
| Limit-order request construction | 396.5 ns/operation | 397.1 ns/operation | +0.2% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |
| Insights live signal snapshot | 1,809.6 ns/calculation | 1,770.5 ns/calculation | -2.2% |
| Snapshot including explanations | 2,255.8 ns/calculation | 2,237.5 ns/calculation | -0.8% |
| Research feature extraction | 145.4 ns/calculation | 142.6 ns/calculation | -1.9% |
| Price return context | 17.5 ns/calculation | 17.2 ns/calculation | -1.7% |
| Market input diagnostics | 228.2 ns/calculation | 228.7 ns/calculation | +0.2% |
| Book snapshot | 44.9 ns/calculation | 44.7 ns/calculation | -0.4% |

Same workloads, WSL2 environment, and optimized Cargo bench profiles. No meaningful regression appeared in the measured paths; the exit scenario work runs outside the microbenchmark loops. No services were started or stopped.


## Follow-up run (2026-11-02 20:02 UTC)

### Per-parameter scout probes and feedback

| Workload | Latest comparable run (19:58 UTC) | Current run | Change |
|---|---:|---:|---:|
| Auto-trader entry evaluation | 1.1–5.0 ns/evaluation | 1.1–6.2 ns/evaluation | Upper outlier +24%; repeated serial run ranged to 8.0 ns |
| Auto-trader position evaluation | 1.1–1.9 ns/evaluation | 1.1–2.1 ns/evaluation | Upper outlier +11%; repeated serial run ranged to 4.3 ns |
| Signal serialization | 519.3 ns/operation | 488.7 ns/operation | -5.9% |
| Market depth assessment | 583.3 ns/operation | 567.6 ns/operation | -2.7% |
| Limit-order request construction | 396.5 ns/operation | 396.2 ns/operation | -0.1% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |
| Insights live signal snapshot | 1,750.5 ns/calculation | 1,799.1 ns/calculation | +2.8% |
| Snapshot including explanations | 2,212.6 ns/calculation | 2,399.4 ns/calculation | +8.4% |
| Research feature extraction | 146.5 ns/calculation | 148.8 ns/calculation | +1.6% |
| Price return context | 17.2 ns/calculation | 17.1 ns/calculation | -0.6% |
| Market input diagnostics | 232.7 ns/calculation | 224.8 ns/calculation | -3.4% |
| Book snapshot | 44.5 ns/calculation | 46.0 ns/calculation | +3.4% |

Same WSL2 environment and optimized Cargo bench targets as the persisted comparisons. Two consecutive auto-trader runs varied substantially in evaluator maxima (Donchian 55: 3.3–6.2 ns; RSI2: 2.8–5.5 ns; dated carry: 1.9–4.2 ns), indicating high single-run noise. Parameter probing and outcome bookkeeping are outside the measured evaluator loops. The Insights calculations show a one-run increase of 0–8.4%; repeat measurements are needed before attributing it to the new code. No services were started or stopped.

## Follow-up run (2026-11-02 20:07 UTC)

### Scout profile-probe outcome wiring

| Workload | Latest comparable run (20:02 UTC) | Current serial run | Change |
|---|---:|---:|---:|
| Insights live signal snapshot | 1,770.5 ns/calculation | 1,789.0 ns/calculation | +1.0% |
| Snapshot including explanations | 2,237.5 ns/calculation | 2,217.1 ns/calculation | -0.9% |
| Research feature extraction | 142.6 ns/calculation | 139.8 ns/calculation | -2.0% |
| Price return context | 17.2 ns/calculation | 17.1 ns/calculation | -0.6% |
| Market input diagnostics | 228.7 ns/calculation | 218.7 ns/calculation | -4.4% |
| Book snapshot | 44.7 ns/calculation | 44.9 ns/calculation | +0.4% |

Same Insights workloads, WSL2 environment, and optimized Cargo bench profile. No meaningful regression appeared in the measured calculations; the new scout exit and probe callbacks are outside these hot loops.


## Follow-up run (2026-11-02 20:07 UTC)

### Parameter-specific scout feedback and exit tracking

| Workload | Latest comparable run (20:02 UTC) | Current run | Change |
|---|---:|---:|---:|
| Auto-trader entry evaluation | 1.1–6.2 ns/evaluation | 1.1–5.0 ns/evaluation | Within recent run range |
| Auto-trader position evaluation | 1.1–2.1 ns/evaluation | 1.1–1.8 ns/evaluation | Within baseline range |
| Signal serialization | 488.7 ns/operation | 485.2 ns/operation | -0.7% |
| Market depth assessment | 567.6 ns/operation | 550.8 ns/operation | -3.0% |
| Limit-order request construction | 396.2 ns/operation | 394.3 ns/operation | -0.5% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |
| Insights live signal snapshot | 1,799.1 ns/calculation | 1,747.6 ns/calculation | -2.9% |
| Snapshot including explanations | 2,399.4 ns/calculation | 2,214.2 ns/calculation | -7.7% |
| Research feature extraction | 148.8 ns/calculation | 147.6 ns/calculation | -0.8% |
| Price return context | 17.1 ns/calculation | 17.5 ns/calculation | +2.3% |
| Market input diagnostics | 224.8 ns/calculation | 231.0 ns/calculation | +2.8% |
| Book snapshot | 46.0 ns/calculation | 45.5 ns/calculation | -1.1% |

Both crates ran serially in the same WSL2 environment and optimized Cargo bench profiles. The earlier 20:02 strategy maxima varied across back-to-back runs; the current evaluator range is back within the persisted baseline. Parameter probing and probe-specific exit handling do not run in these benchmark loops. The Insights calculations also returned near or below the previous run. No services were started or stopped.

## Follow-up run (2026-11-02 20:09 UTC)

### Scout outcome persistence and exit variants

| Workload | Latest comparable run (20:07 UTC) | Current serial run | Change |
|---|---:|---:|---:|
| Auto-trader entry evaluation | 1.1–6.2 ns/evaluation | 1.1–5.1 ns/evaluation | Within prior run range |
| Auto-trader position evaluation | 1.1–2.1 ns/evaluation | 1.1–1.9 ns/evaluation | Within baseline range |
| Signal serialization | 488.7 ns/operation | 487.7 ns/operation | -0.2% |
| Market depth assessment | 567.6 ns/operation | 555.1 ns/operation | -2.2% |
| Limit-order request construction | 396.2 ns/operation | 393.8 ns/operation | -0.6% |
| Strategy parameter validation | 1.9 ns/operation | 2.0 ns/operation | +0.1 ns |
| Insights live signal snapshot | 1,799.1 ns/calculation | 1,861.5 ns/calculation | +3.5% |
| Snapshot including explanations | 2,399.4 ns/calculation | 2,336.7 ns/calculation | -2.6% |
| Research feature extraction | 148.8 ns/calculation | 145.7 ns/calculation | -2.1% |
| Price return context | 17.1 ns/calculation | 17.5 ns/calculation | +2.3% |
| Market input diagnostics | 224.8 ns/calculation | 227.0 ns/calculation | +1.0% |
| Book snapshot | 46.0 ns/calculation | 46.3 ns/calculation | +0.7% |

Same workloads and serial WSL2 optimized bench profiles. Results stayed close to the latest comparable run and within the recorded baseline range, with no meaningful regression in these benchmarked paths.


## Follow-up run (2026-11-02 20:09 UTC)

### Variant-specific scout status and hold outcomes

| Workload | Latest comparable run (20:07 UTC) | Current run | Change |
|---|---:|---:|---:|
| Auto-trader entry evaluation | 1.1–5.0 ns/evaluation | 1.1–5.1 ns/evaluation | Within baseline range |
| Auto-trader position evaluation | 1.1–1.8 ns/evaluation | 1.1–1.9 ns/evaluation | Within baseline range |
| Signal serialization | 485.2 ns/operation | 492.4 ns/operation | +1.5% |
| Market depth assessment | 550.8 ns/operation | 608.6 ns/operation | +10.5%; isolated single-run increase |
| Limit-order request construction | 394.3 ns/operation | 394.7 ns/operation | +0.1% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |
| Insights live signal snapshot | 1,747.6 ns/calculation | 1,761.2 ns/calculation | +0.8% |
| Snapshot including explanations | 2,214.2 ns/calculation | 2,211.0 ns/calculation | -0.1% |
| Research feature extraction | 147.6 ns/calculation | 143.0 ns/calculation | -3.1% |
| Price return context | 17.5 ns/calculation | 17.3 ns/calculation | -1.1% |
| Market input diagnostics | 231.0 ns/calculation | 226.0 ns/calculation | -2.2% |
| Book snapshot | 45.5 ns/calculation | 49.6 ns/calculation | +9.0%; isolated single-run increase |

Same benchmark targets, serial execution, WSL2 environment, and optimized Cargo profiles. Evaluator and Insights snapshot paths remain near baseline. Market depth and book snapshot were elevated in this run while adjacent measurements remained near prior values; treat these as unconfirmed single-run outliers. Parameter-probe status matching and hold deadlines are outside the measured loops. No services were started or stopped.


## Final follow-up run (2026-11-02 20:16 UTC)

### Daily, live, and carry parameter probes

| Workload | Latest comparable run (20:09 UTC) | Current run | Change |
|---|---:|---:|---:|
| Auto-trader entry evaluation | 1.1–5.1 ns/evaluation | 1.2–5.1 ns/evaluation | Within baseline range |
| Auto-trader position evaluation | 1.1–1.9 ns/evaluation | 1.1–1.9 ns/evaluation | Within baseline range |
| Signal serialization | 492.4 ns/operation | 495.7 ns/operation | +0.7% |
| Market depth assessment | 608.6 ns/operation | 554.3 ns/operation | -8.9% |
| Limit-order request construction | 394.7 ns/operation | 392.8 ns/operation | -0.5% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |
| Insights live signal snapshot | 1,761.2 ns/calculation | 1,742.8 ns/calculation | -1.0% |
| Snapshot including explanations | 2,211.0 ns/calculation | 2,280.0 ns/calculation | +3.1% |
| Research feature extraction | 143.0 ns/calculation | 142.6 ns/calculation | -0.3% |
| Price return context | 17.3 ns/calculation | 17.1 ns/calculation | -1.2% |
| Market input diagnostics | 226.0 ns/calculation | 224.1 ns/calculation | -0.8% |
| Book snapshot | 49.6 ns/calculation | 45.3 ns/calculation | -8.7% |

Same serial benchmark targets, WSL2 environment, and optimized Cargo profiles. Auto-trader evaluator results remained within the recorded range. The Insights explanation snapshot rose 3.1% in this single run; adjacent calculation workloads were unchanged or faster, so this does not show a consistent regression. Probe generation, feedback persistence, and shadow exit tracking are outside the measured evaluator and snapshot loops. No services were started or stopped.


## Final follow-up run (2026-11-02 20:17 UTC)

### Live and dated-carry parameter probes

| Workload | Latest comparable run (20:16 UTC) | Current run | Change |
|---|---:|---:|---:|
| Auto-trader entry evaluation | 1.2–5.1 ns/evaluation | 1.1–5.0 ns/evaluation | Within baseline range |
| Auto-trader position evaluation | 1.1–1.9 ns/evaluation | 1.1–1.8 ns/evaluation | Within baseline range |
| Signal serialization | 495.7 ns/operation | 490.6 ns/operation | -1.0% |
| Market depth assessment | 554.3 ns/operation | 548.7 ns/operation | -1.0% |
| Limit-order request construction | 392.8 ns/operation | 392.4 ns/operation | -0.1% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |
| Insights live signal snapshot | 1,742.8 ns/calculation | 1,772.1 ns/calculation | +1.7% |
| Snapshot including explanations | 2,280.0 ns/calculation | 2,177.4 ns/calculation | -4.5% |
| Research feature extraction | 142.6 ns/calculation | 142.4 ns/calculation | -0.1% |
| Price return context | 17.1 ns/calculation | 16.8 ns/calculation | -1.8% |
| Market input diagnostics | 224.1 ns/calculation | 225.2 ns/calculation | +0.5% |
| Book snapshot | 45.3 ns/calculation | 44.6 ns/calculation | -1.5% |

Same serial benchmark targets, WSL2 environment, and optimized Cargo profiles. All measurements are within the baseline or recent-run variation. The parameter probe generation, outcome attribution, and dated-carry settlement updates are outside the measured hot loops. No services were started or stopped.


## Final follow-up run (2026-11-02 20:20 UTC)

### Full strategy parameter scouting

| Workload | Latest comparable run (20:17 UTC) | Current run | Change |
|---|---:|---:|---:|
| Auto-trader entry evaluation | 1.1–5.0 ns/evaluation | 1.1–5.1 ns/evaluation | Within baseline range |
| Auto-trader position evaluation | 1.1–1.8 ns/evaluation | 1.1–1.9 ns/evaluation | Within baseline range |
| Signal serialization | 490.6 ns/operation | 524.6 ns/operation | +6.9%; single-run increase |
| Market depth assessment | 548.7 ns/operation | 557.4 ns/operation | +1.6% |
| Limit-order request construction | 392.4 ns/operation | 402.5 ns/operation | +2.6% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |
| Insights live signal snapshot | 1,772.1 ns/calculation | 1,757.6 ns/calculation | -0.8% |
| Snapshot including explanations | 2,177.4 ns/calculation | 2,225.9 ns/calculation | +2.2% |
| Research feature extraction | 142.4 ns/calculation | 146.3 ns/calculation | +2.7% |
| Price return context | 16.8 ns/calculation | 16.9 ns/calculation | +0.6% |
| Market input diagnostics | 225.2 ns/calculation | 222.4 ns/calculation | -1.2% |
| Book snapshot | 44.6 ns/calculation | 44.5 ns/calculation | -0.2% |

Same serial workloads, WSL2 environment, and optimized Cargo profiles. Evaluator and Insights snapshot paths remain near baseline. The signal serialization result rose 6.9% in one run; this does not measure the added scout serialization or outcome persistence path, so treat it as an unconfirmed timing outlier. No services were started or stopped.


## Final follow-up run (2026-11-02 21:57 UTC)

### Scout fallback guard

| Workload | Latest comparable run (20:20 UTC) | Current run | Change |
|---|---:|---:|---:|
| Auto-trader entry evaluation | 1.1–5.1 ns/evaluation | 1.1–5.0 ns/evaluation | Within baseline range |
| Auto-trader position evaluation | 1.1–1.9 ns/evaluation | 1.1–1.9 ns/evaluation | Within baseline range |
| Signal serialization | 524.6 ns/operation | 487.2 ns/operation | -7.1% |
| Market depth assessment | 557.4 ns/operation | 544.3 ns/operation | -2.4% |
| Limit-order request construction | 402.5 ns/operation | 387.4 ns/operation | -3.7% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |

Same auto-trader bench target, WSL2 environment, and optimized Cargo profile. This change is in candidate routing and shadow-only guards, outside the measured hot loops. All measured workloads remained within recent run variation. No services were started or stopped.

## Follow-up run (2026-11-03 09:16 UTC)

### Scout shadow outcome aggregation

| Workload | Latest comparable run (2026-11-02 21:57 UTC) | Current run | Change |
|---|---:|---:|---:|
| Auto-trader entry evaluation | 1.1–5.0 ns/evaluation | 1.3–5.0 ns/evaluation | Within baseline range |
| Auto-trader position evaluation | 1.1–1.9 ns/evaluation | 1.1–1.8 ns/evaluation | Within baseline range |
| Signal serialization | 487.2 ns/operation | 494.0 ns/operation | +1.4% |
| Market depth assessment | 544.3 ns/operation | 588.2 ns/operation | +8.1% |
| Limit-order request construction | 387.4 ns/operation | 403.8 ns/operation | +4.2% |
| Strategy parameter validation | 1.9 ns/operation | 2.0 ns/operation | +5.3% |

Same auto-trader benchmark target, WSL2 environment, and optimized Cargo profile. The change adds shadow learning outcomes to strategy sizing aggregation, outside the measured evaluator and request-construction loops. Single-run increases in market depth and request construction are not on the changed aggregation path and are unconfirmed timing variations. No service was started or stopped.

## Follow-up run (2026-11-03)

### Auto-trader crate after configurable daily Scout selection

| Workload | Result | Baseline comparison |
|---|---:|---|
| Strategy entry evaluation | 1.7–5.1 ns/evaluation | Baseline 1.2–5.2 ns; within range |
| Strategy position evaluation | 1.1–1.9 ns/evaluation | Baseline 1.1–3.1 ns; within range |
| Signal serialization | 507.8 ns/operation | +0.7% vs 504.3 ns |
| Market depth assessment | 553.6 ns/operation | -0.5% vs 557.1 ns |
| Limit order request construction | 402.2 ns/operation | +0.8% vs 399.7 ns |
| Strategy parameter validation | 1.9 ns/operation | unchanged vs 1.9 ns |

Same WSL2 environment and optimized Cargo bench profile as the baseline. Results are single-run microbenchmarks; no meaningful performance degradation was observed. The benchmark target's embedded test binary ran with all 112 tests ignored, as expected in bench mode; cargo test separately passed all 112 tests. The optional live HTTP benchmark was not applicable to this crate.

## Follow-up run (2026-11-03)

### Auto-trader after enabling Scout selection for live strategies

| Workload | Result | Baseline comparison |
|---|---:|---|
| Strategy entry evaluation | 1.1–5.0 ns/evaluation | Baseline 1.2–5.2 ns; within range |
| Strategy position evaluation | 1.1–1.9 ns/evaluation | Baseline 1.1–3.1 ns; within range |
| Signal serialization | 510.0 ns/operation | +1.1% vs 504.3 ns |
| Market depth assessment | 566.2 ns/operation | +1.6% vs 557.1 ns |
| Limit order request construction | 399.3 ns/operation | -0.1% vs 399.7 ns |
| Strategy parameter validation | 1.9 ns/operation | unchanged vs 1.9 ns |

The auto-trader benchmark ran in the same WSL2 optimized Cargo profile as the baseline. Results are single-run microbenchmarks; measured workloads were within baseline ranges, with no meaningful performance degradation.


## Follow-up run (2026-11-03 09:45 UTC)

### Daily strategy exit handling and eligibility snapshots

| Workload | Latest comparable run | This change: run 1 / repeat | Comparison |
|---|---:|---:|---|
| Strategy entry evaluation | 1.1–5.0 ns/evaluation | 1.2–5.6 / 1.2–5.3 ns/evaluation | First run had a 5.6 ns upper outlier; repeat was close to recent range |
| Strategy position evaluation | 1.1–1.9 ns/evaluation | 1.1–2.4 / 1.1–1.9 ns/evaluation | First run had a 2.4 ns outlier; repeat matched recent range |
| Signal serialization | 510.0 ns/operation | 503.3 / 522.2 ns/operation | Within recent single-run variation |
| Market depth assessment | 566.2 ns/operation | 580.9 / 580.1 ns/operation | +2.5% vs latest; +4.1% vs original baseline, outside the changed path |
| Limit order request construction | 399.3 ns/operation | 393.3 / 404.9 ns/operation | Within recent variation |
| Strategy parameter validation | 1.9 ns/operation | 1.9 / 2.0 ns/operation | Within measurement variation |

Both runs used the same auto-trader benchmark target, WSL2 environment, and optimized Cargo profile. The changes add tri-state daily eligibility handling, exit audit context, and a pending-exit entry guard; they do not change market-depth assessment. The first-run entry and position maxima did not repeat. Market-depth assessment was consistently about 2.5% above the latest comparable run, but remains a single-workload, single-host measurement and is outside the modified path; treat it as an observed timing difference rather than a confirmed regression. The auto-trader test suite passed separately. No services were started or stopped.

## Follow-up run (2026-11-03)

### Execution crate test coverage

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Scheduler / order-manager construction | 106.5 / 213.3 ns | 105.7 / 209.6 ns | -0.8% / -1.7% |
| Child-order calculation | 2.4 ns | 2.5 ns | +4.2% |
| Market trade: no action / new / modify / cancel | 6.3 / 6.4 / 11.7 / 11.4 ns | 6.2 / 6.3 / 11.1 / 11.3 ns | -1.6% / -1.6% / -5.1% / -0.9% |
| Fill processing | 9.2 ns | 9.1 ns | -1.1% |
| Full 78-bin run with fills | 8.4 ns | 8.4 ns | No change |
| 64-symbol arrival/partial-fill workload | 5.0 ns/event | 4.9 ns/event | -2.0% |

Same WSL2 optimized Cargo bench profile. All matching execution workloads were unchanged or slightly faster; no performance degradation was observed.  passed all 10 unit tests. Bench mode ran both benchmark targets successfully and ignored the 10 test cases as expected. No services were started or stopped.

## Follow-up run (2026-11-03)

### Position management test coverage additions

| Workload | Previous baseline | Current run | Change |
|---|---:|---:|---:|
| Ledger, 100 fills: MessagePack / JSON decode | 121.7 / 133.7 us | 120.9 / 144.2 us | -0.7% / +7.9% |
| Ledger, 100 fills: MessagePack / JSON encode | 19.6 / 32.0 us | 18.3 / 32.1 us | -6.6% / +0.4% |
| Ledger, 100,000 fills: MessagePack / JSON decode | 175.0 / 189.0 ms | 208.8 / 253.8 ms | +19.3% / +34.3% |
| Ledger, 100,000 fills: MessagePack / JSON encode | 64.5 / 102.3 ms | 75.0 / 110.5 ms | +16.2% / +8.0% |
| 100k-fill position aggregation proxy | 4.385 ms/replay | 3.937 ms/replay | -10.2% |
| Indexed lookup / position count scan | 58.4 / 53.0 ns | 49.1 / 52.6 ns | -15.9% / -0.8% |
| Concurrent atomic file workload | 1,869 ms | 1,938 ms | +3.7% |

Same position-management benchmark targets and optimized Cargo bench profile as the recorded baseline. The changes add unit tests only; they do not alter benchmarked production paths. Large-ledger single-run timings varied upward, while the smaller workloads were close to baseline. Treat these results as environmental/run variation rather than a code regression. The live HTTP benchmark was skipped because BENCH_SERVICE_URL was unset. No services were started or stopped.

## Follow-up run (2026-11-03)

### Auto-trader contract and utility tests

| Workload | Previous run | Current run | Change |
|---|---:|---:|---:|
| Auto-trader entry evaluation | 1.1–5.0 ns/evaluation | 1.9–5.1 ns/evaluation | Within baseline range |
| Auto-trader position evaluation | 1.1–1.9 ns/evaluation | 1.1–1.8 ns/evaluation | Within baseline range |
| Signal serialization | 493.4 ns/operation | 494.6 ns/operation | +0.2% |
| Market depth assessment | 550.4 ns/operation | 555.9 ns/operation | +1.0% |
| Limit-order request construction | 398.4 ns/operation | 401.3 ns/operation | +0.7% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |

Same auto_trader benchmark target and optimized Cargo bench profile as the previous run. These single-run variations are small and within the recorded range; the change adds tests and a guarded diagnostic classification without changing benchmarked strategy evaluation paths.

## Follow-up run (2026-11-03)

### Auto-trader strategy evaluation

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation | 1.1–5.0 ns/evaluation | 1.2–5.5 ns/evaluation | Maximum +10.0% |
| Strategy position evaluation | 1.1–1.9 ns/evaluation | 1.2–2.4 ns/evaluation | Maximum +26.3% |
| Signal serialization | 494.6 ns/operation | 524.9 ns/operation | +6.1% |
| Market depth assessment | 555.9 ns/operation | 643.6 ns/operation | +15.8% |
| Limit-order request construction | 401.3 ns/operation | 419.4 ns/operation | +4.5% |
| Strategy parameter validation | 1.9 ns/operation | 2.0 ns/operation | +5.3% |

Same auto-trader benchmark target and optimized Cargo profile on WSL2. This change adjusts daily Scout candidate filtering and shadow recording; it does not change the benchmarked evaluator, serialization, depth, order construction, or parameter-validation code. Single-run timings were higher for several workloads, including market depth, which is outside the changed path. Treat these as observed run variation, not a confirmed regression. Auto-trader tests passed (124 tests). No services were started or stopped.

## Follow-up run (2026-11-03, repeated after adding diagnostic coverage)

### Auto-trader benchmark comparison

| Workload | Previous run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation, per registered strategy | 1.9–5.1 ns/evaluation | 1.9–5.8 ns/evaluation | Upper end +13.7%, above prior range |
| Strategy position evaluation, per registered strategy | 1.1–1.8 ns/evaluation | 1.1–2.1 ns/evaluation | Upper end +16.7%, above prior range |
| Signal serialization | 494.6 ns/operation | 530.7 ns/operation | +7.3% |
| Market depth assessment | 555.9 ns/operation | 648.0 ns/operation | +16.6% |
| Limit-order request construction | 401.3 ns/operation | 431.8 ns/operation | +7.6% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |

This used the same auto_trader benchmark target and optimized Cargo bench profile, but remains a single run with workload and environment noise. The benchmarked production paths were not changed by the added tests or diagnostic classifier guard, so these increases are recorded for follow-up rather than attributed to this change. No service processes were started or stopped.

## Follow-up run (2026-11-03, feed-handler test fix)

### Feed-handler benchmark comparison

| Workload | Previous run | Current run | Change |
|---|---:|---:|---:|
| Direct two-tick read | 0.81 ns | 0.81 ns | No change |
| Decimal price parse | 7.59 ns | 7.76 ns | +2.2% |
| Two direct tick writes | 4.53 ns | 4.49 ns | -0.9% |
| Book update (8 writes and parses) | 175.16 ns | 177.30 ns | +1.2% |
| Best-level restore, 1,000,001 slots | 5.700 us | 5.855 us | +2.7% |
| Sparse snapshot, 1,000,001 slots | 6.743 us | 6.698 us | -0.7% |
| Allocate + SBE encode + payload | 35.4 ns | 35.7 ns | +0.8% |
| Reused-buffer SBE encode | 10.0 ns | 9.7 ns | -3.0% |
| Per-client encode, 1 / 4 / 16 subscribers | 33.2 / 116.5 / 463.0 ns | 34.4 / 117.8 / 464.0 ns | +3.6% / +1.1% / +0.2% |
| Shared immutable payload, 1 / 4 / 16 subscribers | 44.3 / 79.9 / 229.8 ns | 44.3 / 79.8 / 224.1 ns | No change / -0.1% / -2.5% |
| Tokio broadcast, 200 / 400 subscribers | 3.960 / 7.830 us | 4.049 / 7.991 us | +2.2% / +2.1% |

Same feed_handlers benchmark targets and optimized Cargo bench profile as the previous recorded run. This change only corrects a test expectation for the 8-byte SBE header plus 56-byte block; measured workloads are within ordinary single-run variation. No services were started or stopped.

## Follow-up run (2026-11-03, expanded strategy learning window)

### Auto-trader strategy evaluation

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation, per registered strategy | 1.9–5.8 ns/evaluation | 1.8–5.2 ns/evaluation | Within or below prior range |
| Strategy position evaluation, per registered strategy | 1.1–2.4 ns/evaluation | 1.1–1.8 ns/evaluation | Within or below prior range |
| Signal serialization | 530.7 ns/operation | 491.7 ns/operation | -7.3% |
| Market depth assessment | 648.0 ns/operation | 548.3 ns/operation | -15.4% |
| Limit-order request construction | 431.8 ns/operation | 403.8 ns/operation | -6.5% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |

Same auto_trader strategy evaluation benchmark and optimized Cargo profile. The change increases the retained outcome window from 20 to 500; it does not alter the benchmarked paths. All measurements were within or below the previous recorded range. Auto-trader tests passed (124 tests). No services were started or stopped.

## Follow-up run (2026-11-03, 5,000-outcome strategy learning window)

### Auto-trader strategy evaluation

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation, per registered strategy | 1.8–5.2 ns/evaluation | 1.1–5.2 ns/evaluation | Within or below prior range |
| Strategy position evaluation, per registered strategy | 1.1–1.8 ns/evaluation | 1.1–2.2 ns/evaluation | Upper end +22.2% |
| Signal serialization | 491.7 ns/operation | 521.9 ns/operation | +6.1% |
| Market depth assessment | 548.3 ns/operation | 591.3 ns/operation | +7.8% |
| Limit-order request construction | 403.8 ns/operation | 407.8 ns/operation | +1.0% |
| Strategy parameter validation | 1.9 ns/operation | 2.0 ns/operation | +5.3% |

Same auto_trader strategy evaluation benchmark and optimized Cargo profile. The change raises the retained outcome window from 500 to 5,000; it does not alter the benchmarked paths. Position evaluation, serialization, market depth, and parameter validation showed modest single-run increases; these paths are unaffected by the window change and the results do not indicate a regression from this change. Auto-trader tests passed (124 tests). No services were started or stopped.

## Follow-up run (2026-11-03, scout parameter promotion evidence)

### Auto-trader strategy evaluation

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation, per registered strategy | 1.1–5.2 ns/evaluation | 1.2–5.2 ns/evaluation | Within prior range |
| Strategy position evaluation, per registered strategy | 1.1–2.2 ns/evaluation | 1.1–1.8 ns/evaluation | Within or below prior range |
| Signal serialization | 521.9 ns/operation | 494.2 ns/operation | -5.3% |
| Market depth assessment | 591.3 ns/operation | 550.6 ns/operation | -6.9% |
| Limit-order request construction | 407.8 ns/operation | 385.5 ns/operation | -5.5% |
| Strategy parameter validation | 2.0 ns/operation | 2.0 ns/operation | No change |

Same auto_trader strategy evaluation benchmark and optimized Cargo profile. The change tightens scout parameter promotion and is outside benchmarked evaluation paths; all workloads are within or below the latest comparable run. The full test suite ran 144 tests: 140 passed and 4 unrelated existing diagnostic expectation tests failed. The focused scout parameter promotion test passed. No services were started or stopped.

## Follow-up run (2026-11-03, coverage threshold work)

### Auto-trader benchmark comparison

| Workload | Previous run | Current run | Change |
|---|---:|---:|---:|
| Signal serialization | 530.7 ns/operation | 498.1 ns/operation | -6.1% |
| Market depth assessment | 648.0 ns/operation | 564.2 ns/operation | -12.9% |
| Limit-order request construction | 431.8 ns/operation | 395.4 ns/operation | -8.4% |
| Strategy parameter validation | 1.9 ns/operation | 2.0 ns/operation | +0.1 ns |

The strategy evaluation workloads remained within the previously observed single-run range. This used the same auto_trader benchmark target and optimized Cargo bench profile; test additions and diagnostic parsing changes do not alter these benchmarked hot paths. Results moved back toward the November 2 baseline, consistent with single-run noise. No services were started or stopped.

## Follow-up run (2026-11-03, learned Donchian entry distance)

### Auto-trader strategy evaluation

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Signal serialization | 498.1 ns/operation | 494.7 ns/operation | -0.7% |
| Market depth assessment | 564.2 ns/operation | 554.4 ns/operation | -1.7% |
| Limit-order request construction | 395.4 ns/operation | 407.6 ns/operation | +3.1% |
| Strategy parameter validation | 2.0 ns/operation | 1.9 ns/operation | -5.0% |

The strategy-evaluation workload now supplies each strategy’s registered default parameter map; prior runs used an empty map, so those evaluator timings are not directly comparable. The comparable request and validation workloads were effectively unchanged. The first bench attempt exposed missing registered defaults in its fixture; the fixture was corrected and the target then completed successfully. Auto-trader tests passed (159 tests). No services were started or stopped.

## Follow-up run (2026-11-03, learned breakout entry distance)

### Auto-trader strategy evaluation

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Signal serialization | 498.1 ns/operation | 488.8 ns/operation | -1.9% |
| Market depth assessment | 564.2 ns/operation | 550.3 ns/operation | -2.5% |
| Limit-order request construction | 395.4 ns/operation | 395.7 ns/operation | +0.1% |
| Strategy parameter validation | 2.0 ns/operation | 1.9 ns/operation | -5.0% |

The entry-evaluation benchmark now constructs each strategy’s registered default parameter map; earlier runs used an empty map, so those timings are not comparable. The four workloads above retained the same benchmark inputs and remained within normal single-run variation. Auto-trader tests passed (160 tests). No services were started or stopped.

## Follow-up run (2026-11-03, probe-specific breakout samples)

### Auto-trader strategy evaluation

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Signal serialization | 498.1 ns/operation | 490.2 ns/operation | -1.6% |
| Market depth assessment | 564.2 ns/operation | 567.5 ns/operation | +0.6% |
| Limit-order request construction | 395.4 ns/operation | 401.8 ns/operation | +1.6% |
| Strategy parameter validation | 2.0 ns/operation | 1.9 ns/operation | -5.0% |

Strategy entry-evaluation inputs include registered default parameter maps, unlike the older empty-map benchmark, so those timings remain non-comparable. The four matching workloads were within small single-run variation. Auto-trader tests passed (160 tests). No services were started or stopped.

## Follow-up run (2026-11-03, simultaneous live and daily strategies)

### Auto-trader strategy evaluation

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Signal serialization | 490.2 ns/operation | 498.7 ns/operation | +1.7% |
| Market depth assessment | 567.5 ns/operation | 551.3 ns/operation | -2.9% |
| Limit-order request construction | 401.8 ns/operation | 392.4 ns/operation | -2.3% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |

Strategy entry-evaluation inputs use registered default parameter maps, so older empty-map timings are not comparable. The comparable workloads were unchanged within normal single-run variation. Auto-trader tests passed (160 tests). No services were started or stopped.

## Follow-up run (2026-11-03, keep live Scouts alongside daily signals)

### Auto-trader strategy evaluation

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Signal serialization | 498.7 ns/operation | 490.6 ns/operation | -1.6% |
| Market depth assessment | 551.3 ns/operation | 552.9 ns/operation | +0.3% |
| Limit-order request construction | 392.4 ns/operation | 398.9 ns/operation | +1.7% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |

Strategy entry-evaluation workloads remain non-comparable with older empty-parameter-map runs. Comparable workloads were within normal single-run variation. Auto-trader tests passed (161 tests). No services were started or stopped.

## Follow-up run (2026-11-03)

### Auto-trader live Scout ingress

The live Scout subscription gate is not part of the strategy evaluation benchmark workload. The cargo bench strategy_evaluation target completed successfully; evaluator results ranged 1.1–10.7 ns/entry and 1.1–1.9 ns/position. Signal serialization measured 498.3 ns/operation, market depth assessment 558.6 ns/operation, limit order construction 398.8 ns/operation, and parameter validation 1.9 ns/operation. Against the latest comparable run, these remain within prior recorded ranges and show no meaningful regression. The changed ingress routing path is not represented by this microbenchmark.


## Follow-up run (2026-11-03, Scout learning thresholds)

### Auto-trader strategy evaluation

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Signal serialization | 490.6 ns/operation | 492.3 ns/operation | +0.35% |
| Market depth assessment | 552.9 ns/operation | 556.0 ns/operation | +0.56% |
| Limit-order request construction | 398.9 ns/operation | 392.1 ns/operation | -1.7% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |

Same AutoTrader benchmark target and optimized Cargo profile. Strategy evaluator timings were recorded but are not directly compared because the benchmark fixture now supplies registered parameter profiles; older empty-map runs are not comparable. The matched workloads show no meaningful regression. AutoTrader tests passed (163 tests). No services were started or stopped.


## Follow-up run (2026-11-03, learned Scout candidate gating)

### Auto-trader strategy evaluation

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Signal serialization | 492.3 ns/operation | 499.3 ns/operation | +1.42% |
| Market depth assessment | 556.0 ns/operation | 565.9 ns/operation | +1.78% |
| Limit-order request construction | 392.1 ns/operation | 401.7 ns/operation | +2.45% |
| Strategy parameter validation | 1.9 ns/operation | 2.0 ns/operation | +0.1 ns |

Same AutoTrader benchmark target and optimized Cargo profile. Strategy evaluator timings were recorded but are not directly compared because the benchmark fixture supplies registered parameter profiles; older empty-map runs are not comparable. Matched workloads remain within prior single-run variation; the parameter validation change is 0.1 ns/operation. AutoTrader tests passed (163 tests). No services were started or stopped.

## Follow-up run (2026-11-03, scoutable dated-carry expiry and edge gates)

### Auto-trader strategy evaluation

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation | 1.1–10.7 ns/evaluation | 1.1–10.7 ns/evaluation | Within run range |
| Strategy position evaluation | 1.1–2.0 ns/evaluation | 1.1–2.0 ns/evaluation | Within run range |
| Signal serialization | 499.3 ns/operation | 500.8 ns/operation | +0.3% |
| Market depth assessment | 565.9 ns/operation | 647.2 ns/operation | +14.4% |
| Limit-order request construction | 401.7 ns/operation | 438.4 ns/operation | +9.1% |
| Strategy parameter validation | 2.0 ns/operation | 2.1 ns/operation | +5.0% |

### Insights calculations

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Live signal snapshot | 1,742.8 ns/calculation | 1,804.1 ns/calculation | +3.5% |
| Snapshot including explanations | 2,280.0 ns/calculation | 2,296.4 ns/calculation | +0.7% |
| Research feature extraction | 142.6 ns/calculation | 146.5 ns/calculation | +2.7% |
| Price return context | 17.1 ns/calculation | 17.5 ns/calculation | +2.3% |
| Market input diagnostics | 224.1 ns/calculation | 225.5 ns/calculation | +0.6% |
| Book snapshot | 45.3 ns/calculation | 46.6 ns/calculation | +2.9% |

The same benchmark targets and optimized Cargo profiles ran serially under WSL2. Strategy entry and position workloads stayed within their recorded ranges. Several unrelated AutoTrader microbenchmarks rose in this single run, especially market depth and order construction; treat those as unconfirmed timing outliers. Insights calculations were within small single-run variation. The changed carry eligibility paths are not isolated by these benchmarks. AutoTrader tests passed (164 tests); Insights tests passed (21 tests, one ignored). No services were started or stopped.

## Follow-up run (2026-11-03, dated-carry candidate shadow learning)

### Auto-trader strategy evaluation

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation | 1.1–10.7 ns/evaluation | 1.1–11.2 ns/evaluation | Within measurement range |
| Strategy position evaluation | 1.1–2.0 ns/evaluation | 1.1–1.8 ns/evaluation | Within measurement range |
| Signal serialization | 500.8 ns/operation | 526.5 ns/operation | +5.1% |
| Market depth assessment | 647.2 ns/operation | 579.7 ns/operation | -10.4% |
| Limit-order request construction | 438.4 ns/operation | 430.2 ns/operation | -1.9% |
| Strategy parameter validation | 2.1 ns/operation | 1.9 ns/operation | -9.5% |

### Insights calculations

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Live signal snapshot | 1,804.1 ns/calculation | 1,854.3 ns/calculation | +2.8% |
| Snapshot including explanations | 2,296.4 ns/calculation | 2,292.2 ns/calculation | -0.2% |
| Research feature extraction | 146.5 ns/calculation | 140.4 ns/calculation | -4.2% |
| Price return context | 17.5 ns/calculation | 17.4 ns/calculation | -0.6% |
| Market input diagnostics | 225.5 ns/calculation | 227.5 ns/calculation | +0.9% |
| Book snapshot | 46.6 ns/calculation | 44.9 ns/calculation | -3.6% |

The same benchmark targets and optimized Cargo profiles ran serially under WSL2. Strategy evaluator workloads remained within the recorded range. Signal serialization rose 5.1% in this single run; the changed candidate scanning and shadow-outcome paths are outside the evaluator benchmark and were not isolated. Insights calculations stayed within small single-run variation. AutoTrader tests passed (167 tests); Insights tests passed (21 tests, one ignored). No services were started or stopped.

## Follow-up run (2026-11-03, repeated dated-carry shadow benchmark)

A second serial run followed an isolated AutoTrader evaluator outlier in the first run. Results returned to the recent ranges.

### Auto-trader strategy evaluation

| Workload | Latest comparable run | Repeated run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation | 1.1–11.2 ns/evaluation | 1.1–11.6 ns/evaluation | Within run range |
| Strategy position evaluation | 1.1–1.8 ns/evaluation | 1.1–1.9 ns/evaluation | Within run range |
| Signal serialization | 507.3 ns/operation | 513.7 ns/operation | +1.3% |
| Market depth assessment | 579.7 ns/operation | 593.8 ns/operation | +2.4% |
| Limit-order request construction | 430.2 ns/operation | 419.1 ns/operation | -2.6% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |

### Insights calculations

| Workload | Latest comparable run | Repeated run | Change |
|---|---:|---:|---:|
| Live signal snapshot | 1,784.0 ns/calculation | 1,809.5 ns/calculation | +1.4% |
| Snapshot including explanations | 2,238.3 ns/calculation | 2,332.5 ns/calculation | +4.2% |
| Research feature extraction | 146.4 ns/calculation | 145.1 ns/calculation | -0.9% |
| Price return context | 19.2 ns/calculation | 17.0 ns/calculation | -11.5% |
| Market input diagnostics | 262.2 ns/calculation | 226.9 ns/calculation | -13.5% |
| Book snapshot | 50.2 ns/calculation | 45.4 ns/calculation | -9.6% |

Same benchmark targets, optimized Cargo profiles, and serial WSL2 execution. The first run's Donchian evaluator values (9.5–10.5 ns) returned to 5.7–6.3 ns in the repeat. Insights explanation timing varied +4.2% in this repeat while other measurements were flat or lower; no consistent regression is indicated. Candidate streaming and one-day shadow scoring are not represented by these microbenchmarks. AutoTrader tests passed (168 tests); Insights tests passed (21 tests, one ignored). No services were started or stopped.

## Follow-up run (2026-11-03, independent live scout direction)

### Auto-trader strategy evaluation

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation | 1.1–11.6 ns/evaluation | 1.1–11.4 ns/evaluation | Within recorded range |
| Strategy position evaluation | 1.1–1.9 ns/evaluation | 1.1–1.9 ns/evaluation | Within recorded range |
| Signal serialization | 513.7 ns/operation | 513.7 ns/operation | No change |
| Market depth assessment | 593.8 ns/operation | 600.0 ns/operation | +1.0% |
| Limit-order request construction | 419.1 ns/operation | 407.8 ns/operation | -2.7% |
| Strategy parameter validation | 1.9 ns/operation | 2.4 ns/operation | +26.3% |

The same AutoTrader benchmark target and optimized Cargo profile ran serially under WSL2. Strategy evaluator workloads stayed within the latest recorded range. Parameter validation increased 0.5 ns/operation in this single run; candidate direction construction and scout selection are not isolated by this benchmark. No services were started or stopped.

## Follow-up run (2026-11-03, live scout shadow exit tracking)

### Auto-trader strategy evaluation

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation | 1.1–11.4 ns/evaluation | 1.2–10.8 ns/evaluation | Within recorded range |
| Strategy position evaluation | 1.1–1.9 ns/evaluation | 1.2–4.1 ns/evaluation | One run showed higher values |
| Signal serialization | 513.7 ns/operation | 565.8 ns/operation | +10.2% |
| Market depth assessment | 600.0 ns/operation | 584.7 ns/operation | -2.6% |
| Limit-order request construction | 407.8 ns/operation | 412.1 ns/operation | +1.1% |
| Strategy parameter validation | 2.4 ns/operation | 2.0 ns/operation | -16.7% |

The same AutoTrader benchmark target and optimized Cargo profile ran serially under WSL2. Entry evaluations stayed within the recorded range. The position-evaluation range rose due to a single 4.1 ns dated-carry result; serialization rose 10.2% in this run. These timing-only workloads do not exercise fresh market-book retrieval for shadow samples, which is the changed path. Treat the position and serialization results as single-run variation pending repeat. AutoTrader tests passed (171 tests); React app tests passed (85 tests). No services were started or stopped.

## Follow-up run (2026-11-03, live scout exit without synchronized book)

### Insights calculations

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Live signal snapshot | 1,809.5 ns/calculation | 1,794.0 ns/calculation | -0.9% |
| Snapshot including explanations | 2,332.5 ns/calculation | 2,287.4 ns/calculation | -1.9% |
| Research feature extraction | 145.1 ns/calculation | 144.4 ns/calculation | -0.5% |
| Price return context | 17.0 ns/calculation | 17.5 ns/calculation | +2.9% |
| Market input diagnostics | 226.9 ns/calculation | 231.1 ns/calculation | +1.9% |
| Book snapshot | 45.4 ns/calculation | 45.6 ns/calculation | +0.4% |

The same Insights benchmark target and optimized Cargo profile ran under WSL2. All calculations were within 3% of the latest comparable run. The changed scout exit path is not isolated by this benchmark. AutoTrader tests passed (171 tests), Insights tests passed (22 tests, one intentionally ignored), and React app tests passed (85 tests). No services were started or stopped.

## Follow-up run (2026-11-03, pair-based strategy parameter scouts)

### Auto-trader strategy evaluation

| Workload | Latest recorded run | Current serial run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation | 1.1–11.4 ns/evaluation | 1.3–10.3 ns/evaluation | Within recorded range |
| Strategy position evaluation | 1.1–4.1 ns/evaluation | 1.1–2.0 ns/evaluation | Within recorded range |
| Signal serialization | 565.8 ns/operation | 482.5 ns/operation | -14.7% |
| Market depth assessment | 584.7 ns/operation | 548.5 ns/operation | -6.2% |
| Limit-order request construction | 412.1 ns/operation | 400.8 ns/operation | -2.7% |
| Strategy parameter validation | 2.0 ns/operation | 1.9 ns/operation | -5.0% |

### Insights calculations

| Workload | Latest recorded run | Current serial run | Change |
|---|---:|---:|---:|
| Live insight snapshot | 1,794.0 ns/calculation | 1,783.9 ns/calculation | -0.6% |
| Snapshot including explanations | 2,287.4 ns/calculation | 2,247.5 ns/calculation | -1.7% |
| Research feature extraction | 144.4 ns/calculation | 140.7 ns/calculation | -2.6% |
| Price return context | 17.5 ns/calculation | 17.3 ns/calculation | -1.1% |
| Market input diagnostics | 231.1 ns/calculation | 226.9 ns/calculation | -1.8% |
| Book snapshot | 45.6 ns/calculation | 45.0 ns/calculation | -1.3% |

The same AutoTrader and Insights bench targets ran serially under WSL2 with optimized Cargo profiles. All measured workloads were within the latest recorded ranges or faster; no performance degradation was observed. The pair-selection and outcome-attribution work is not isolated by these microbenchmarks. AutoTrader tests passed (173 tests); Insights tests passed (16 tests, one intentionally ignored). No services were started or stopped. No benchmark target was skipped or failed.

### Repeated serial run (2026-11-03)

This repeat used the same bench targets and optimized Cargo profiles, after the first serial run above.

| AutoTrader workload | First pair-learning run | Repeat | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.3–10.3 ns/evaluation | 1.5–13.7 ns/evaluation | One evaluator reached 13.7 ns |
| Strategy position evaluation range | 1.1–2.0 ns/evaluation | 1.1–2.6 ns/evaluation | Within recorded range |
| Signal serialization | 482.5 ns/operation | 500.4 ns/operation | +3.7% |
| Market depth assessment | 548.5 ns/operation | 584.6 ns/operation | +6.6% |
| Limit-order request construction | 400.8 ns/operation | 413.7 ns/operation | +3.2% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |

| Insights workload | First pair-learning run | Repeat | Change |
|---|---:|---:|---:|
| Live insight snapshot | 1,783.9 ns/calculation | 1,774.6 ns/calculation | -0.5% |
| Snapshot including explanations | 2,247.5 ns/calculation | 2,364.2 ns/calculation | +5.2% |
| Research feature extraction | 140.7 ns/calculation | 150.6 ns/calculation | +7.0% |
| Price return context | 17.3 ns/calculation | 18.6 ns/calculation | +7.5% |
| Market input diagnostics | 226.9 ns/calculation | 230.9 ns/calculation | +1.8% |
| Book snapshot | 45.0 ns/calculation | 46.5 ns/calculation | +3.3% |

The live-momentum entry evaluator reached 13.7 ns in the repeat, above the latest recorded 11.4 ns maximum; the prior pair-learning run measured 11.4 ns for it. Evaluator code was not changed by this work, and the variation was isolated to that measurement. Market depth and several Insights timings moved by 5–8% between single runs. Treat these as noisy microbenchmark changes; the pair-selection and profile-attribution paths are not represented by these workloads. AutoTrader tests passed (173 tests); Insights tests passed (16 tests, one intentionally ignored). No targets were skipped or failed, and no services were started or stopped.

### Final serial repeat (2026-11-03)

| AutoTrader workload | Previous serial repeat | Final repeat | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.5–13.7 ns/evaluation | 1.2–7.1 ns/evaluation | Back within recent range |
| Strategy position evaluation range | 1.1–2.6 ns/evaluation | 1.1–1.9 ns/evaluation | Within recorded range |
| Signal serialization | 500.4 ns/operation | 500.2 ns/operation | No material change |
| Market depth assessment | 584.6 ns/operation | 561.3 ns/operation | -4.0% |
| Limit-order request construction | 413.7 ns/operation | 393.7 ns/operation | -4.8% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |

| Insights workload | Previous serial repeat | Final repeat | Change |
|---|---:|---:|---:|
| Live insight snapshot | 1,774.6 ns/calculation | 1,774.4 ns/calculation | No material change |
| Snapshot including explanations | 2,364.2 ns/calculation | 2,232.4 ns/calculation | -5.6% |
| Research feature extraction | 150.6 ns/calculation | 141.2 ns/calculation | -6.2% |
| Price return context | 18.6 ns/calculation | 17.4 ns/calculation | -6.5% |
| Market input diagnostics | 230.9 ns/calculation | 225.2 ns/calculation | -2.5% |
| Book snapshot | 46.5 ns/calculation | 44.5 ns/calculation | -4.3% |

All final workloads returned to or stayed within recent ranges. The earlier 13.7 ns live-momentum evaluator result did not repeat; evaluator code remains outside this change. The pair scouting paths are not isolated by these benchmarks. AutoTrader tests passed (174 tests); Insights tests passed (16 tests, one intentionally ignored). No targets were skipped or failed, and no services were started or stopped.

## Follow-up run (2026-11-03, learned live insight windows and depth)

### Auto-trader strategy evaluation

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation | 1.2–16.4 ns/evaluation | 1.1–18.0 ns/evaluation | Live absorption upper reading +9.8% |
| Strategy position evaluation | 1.1–2.4 ns/evaluation | 1.1–2.4 ns/evaluation | Within run range |
| Signal serialization | 507.5 ns/operation | 509.1 ns/operation | +0.3% |
| Market depth assessment | 584.7 ns/operation | 582.6 ns/operation | -0.4% |
| Limit-order request construction | 412.1 ns/operation | 415.5 ns/operation | +0.8% |
| Strategy parameter validation | 2.0 ns/operation | 2.0 ns/operation | No change |

### Insights calculations

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Live signal snapshot | 1,794.0 ns/calculation | 11,516.6 ns/calculation | +542% |
| Snapshot including explanations | 2,287.4 ns/calculation | 11,778.3 ns/calculation | +415% |
| Research feature extraction | 144.4 ns/calculation | 315.1 ns/calculation | +118% |
| Price return context | 17.5 ns/calculation | 28.9 ns/calculation | +65% |
| Market input diagnostics | 231.1 ns/calculation | 465.8 ns/calculation | +102% |
| Book snapshot | 45.6 ns/calculation | 48.2 ns/calculation | +5.7% |

The same optimized Cargo benchmark targets ran serially under WSL2. A repeat Insights run measured 11,861 ns for snapshots and 12,259 ns with explanations, confirming the added feature calculations account for the increase. The absolute cost is about 12 microseconds per snapshot; the benchmark is a single-symbol micro-workload and does not represent whole-service throughput. The larger research and diagnostic readings may include the same added component work and remain unisolated. AutoTrader tests passed (175 tests); Insights tests passed (23 tests, one intentionally ignored). No services were started or stopped.

## Repeat run (2026-11-03, learned live insight windows and depth)

A repeat serial run measured the granular snapshot workload close to the prior run.

| Workload | Prior run | Repeat run | Change |
|---|---:|---:|---:|
| Live signal snapshot | 11,516.6 ns/calculation | 12,489.1 ns/calculation | +8.4% |
| Snapshot including explanations | 11,778.3 ns/calculation | 13,226.9 ns/calculation | +12.3% |
| Research feature extraction | 315.1 ns/calculation | 259.9 ns/calculation | -17.5% |
| Price return context | 28.9 ns/calculation | 17.6 ns/calculation | -39.1% |
| Market input diagnostics | 338.9 ns/calculation | 415.6 ns/calculation | +22.6% |
| Book snapshot | 47.2 ns/calculation | 53.0 ns/calculation | +12.3% |

Same optimized Insights benchmark target and serial WSL2 environment. Snapshot timings repeat within roughly 12%; standalone helper timings vary more, indicating microbenchmark noise. The snapshot increase versus the old scalar-only baseline remains substantial (about 7x); in absolute terms it is 12–13 microseconds per symbol snapshot. No service was started or stopped.

## Repeat run (2026-11-03, granular live strategy parameter selection)

### Auto-trader live strategy evaluation

| Strategy | Prior run | Repeat run | Change |
|---|---:|---:|---:|
| Live momentum | 13.6 ns/evaluation | 11.0 ns/evaluation | -19.1% |
| Live trade flow | 8.3 ns/evaluation | 6.9 ns/evaluation | -16.9% |
| Live order book | 10.4 ns/evaluation | 10.4 ns/evaluation | No change |
| Live VWAP dislocation | 10.5 ns/evaluation | 11.3 ns/evaluation | +7.6% |
| Live absorption pressure | 18.0 ns/evaluation | 16.6 ns/evaluation | -7.8% |
| Signal serialization | 509.1 ns/operation | 509.1 ns/operation | No change |

The same optimized strategy benchmark ran serially under WSL2. Live strategy evaluator results repeated within the prior run's range. Feature selection from the newly transmitted granular vectors is not isolated by this evaluator microbenchmark.

## Follow-up run (2026-11-03)

### Auto-trader and Insights after Scout exit boundary expansion

| Workload | Previous recorded run | Current run | Change |
|---|---:|---:|---:|
| Auto-trader strategy entry evaluation | 1.2–5.2 ns/evaluation | 1.2–17.9 ns/evaluation | Higher upper range; noisy single-run result |
| Auto-trader strategy position evaluation | 1.1–3.1 ns/evaluation | 1.1–3.0 ns/evaluation | Within baseline range |
| Signal serialization | 497.4 ns/operation | 528.2 ns/operation | +6.2% |
| Market depth assessment | 568.6 ns/operation | 652.6 ns/operation | +14.8% |
| Limit-order request construction | 397.8 ns/operation | 404.3 ns/operation | +1.6% |
| Strategy parameter validation | 1.9 ns/operation | 2.0 ns/operation | +5.3% |
| Insights live signal snapshot | 1,753.6 ns/calculation | 11,744.7 ns/calculation | +570% |
| Snapshot including explanations | 2,280.1 ns/calculation | 12,339.6 ns/calculation | +441% |
| Research feature extraction | 141.2 ns/calculation | 266.2 ns/calculation | +89% |
| Price return context | 17.4 ns/calculation | 17.3 ns/calculation | -0.6% |
| Market input diagnostics | 221.8 ns/calculation | 338.2 ns/calculation | +52% |
| Book snapshot | 47.8 ns/calculation | 48.4 ns/calculation | +1.3% |

Same named benchmark targets and optimized Cargo bench profile. The measured code paths do not include the changed Scout exit-scenario selection. Insights calculation results are substantially slower than the latest recorded run and repeated at similar values on an immediate rerun (live snapshot 11,903.9 ns; snapshot with explanations 11,981.4 ns; research extraction 268.0 ns; diagnostics 332.8 ns). Treat these as a real environment/workload discrepancy requiring a comparable rerun before attributing them to code; no Scout computation was added to these benchmarked calculations. No services were started or stopped.


## Follow-up run (2026-11-03, Scout isolation from real entry limits)

### AutoTrader strategy evaluation

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation | 1.2–17.9 ns/evaluation | 1.1–16.3 ns/evaluation | Within latest range |
| Strategy position evaluation | 1.1–3.0 ns/evaluation | 1.1–1.8 ns/evaluation | Within latest range |
| Signal serialization | 528.2 ns/operation | 497.9 ns/operation | -5.7% |
| Market depth assessment | 652.6 ns/operation | 553.2 ns/operation | -15.2% |
| Limit-order request construction | 404.3 ns/operation | 394.5 ns/operation | -2.4% |
| Strategy parameter validation | 2.0 ns/operation | 1.9 ns/operation | -5.0% |

Same strategy_evaluation target and optimized Cargo profile under WSL2. The changed Scout-position filtering and entry reservation paths are not isolated by this benchmark. The first compile attempt was terminated by SIGTERM; the retry completed successfully. AutoTrader tests passed (179 tests). No benchmark target was skipped or left failed, and no services were started or stopped.


## Follow-up run (2026-11-03)

### Insights calculation benchmark

| Workload | Latest baseline (2026-11-02) | Runs on 2026-11-03 |
|---|---:|---:|
| Live insight snapshot | 1,831.7 ns/calculation | 11,179.2 / 12,004.0 ns/calculation |
| Snapshot including explanations | 2,319.3 ns/calculation | 12,110.2 / 11,923.0 ns/calculation |
| Research feature extraction | 143.4 ns/calculation | 248.8 / 249.3 ns/calculation |
| Price return context | 17.4 ns/calculation | 17.6 / 17.3 ns/calculation |
| Market input diagnostics | 236.5 ns/calculation | 337.1 / 315.9 ns/calculation |
| Book snapshot | 44.4 ns/calculation | 45.3 / 43.5 ns/calculation |

The Insights service and other local trading services were already running during both benchmark runs, so CPU contention differs from the baseline environment. Snapshot and research extraction timings were substantially slower, while price-return and book-snapshot workloads were effectively unchanged. Treat the slower figures as a workload/environment difference, not as an attributed regression from the parameter-probe change; the benchmark does not exercise the daily strategy probe path. No services were started, stopped, or restarted.


## Repeat run (2026-11-03, shadow-cap regression test)

### AutoTrader strategy evaluation

| Workload | Previous run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation | 1.1–16.3 ns/evaluation | 1.4–16.5 ns/evaluation | Upper reading +1.2% |
| Strategy position evaluation | 1.1–1.8 ns/evaluation | 1.1–1.8 ns/evaluation | No range change |
| Signal serialization | 497.9 ns/operation | 550.9 ns/operation | +10.6% |
| Market depth assessment | 553.2 ns/operation | 579.1 ns/operation | +4.7% |
| Limit-order request construction | 394.5 ns/operation | 413.8 ns/operation | +4.9% |
| Strategy parameter validation | 1.9 ns/operation | 2.2 ns/operation | +15.8% (+0.3 ns) |

Same optimized strategy_evaluation target and WSL2 environment. The changed shadow-cap predicate is not exercised by these evaluator microbenchmarks. The small single-run increases in helper timings are not on the modified path and should be treated as measurement variation. AutoTrader tests passed (180 tests). No benchmark targets were skipped or failed, and no services were started or stopped.


## Follow-up run (2026-11-03, protective-stop attribution)

### Auto-trader strategy evaluation

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation | 1.2–17.9 ns/evaluation | 1.2–17.1 ns/evaluation | Within latest range |
| Strategy position evaluation | 1.1–3.0 ns/evaluation | 1.2–2.0 ns/evaluation | Within latest range |
| Signal serialization | 528.2 ns/operation | 506.2 ns/operation | -4.2% |
| Market depth assessment | 652.6 ns/operation | 579.7 ns/operation | -11.2% |
| Limit-order request construction | 404.3 ns/operation | 408.8 ns/operation | +1.1% |
| Strategy parameter validation | 2.0 ns/operation | 1.9 ns/operation | -5.0% |

Same optimized `strategy_evaluation` target and WSL2 environment. The protective-stop attribution changes do not affect these benchmarked paths. No benchmark target was skipped or failed; no services were started or stopped.


## Follow-up run (2026-11-03, pooled learned entry thresholds)

### AutoTrader strategy evaluation

| Workload | Latest comparable run | Current runs |
|---|---:|---:|
| Strategy entry evaluation | 1.2–17.1 ns/evaluation | 1.2–19.2 / 1.2–16.9 ns/evaluation |
| Strategy position evaluation | 1.1–2.0 ns/evaluation | 1.1–2.5 / 1.1–2.0 ns/evaluation |
| Signal serialization | 506.2 ns/operation | 544.2 / 516.4 ns/operation |
| Market depth assessment | 579.7 ns/operation | 577.0 / 662.0 ns/operation |
| Limit-order request construction | 408.8 ns/operation | 430.1 / 413.1 ns/operation |
| Strategy parameter validation | 1.9 ns/operation | 2.0 / 2.0 ns/operation |

Same strategy_evaluation benchmark target and optimized Cargo profile. The strategy evaluator timings straddled the latest baseline on repeat; helper timings varied between repeat runs, including market depth (+14.2% on the second run). The modified pooled threshold-profile calculation is not exercised by these evaluator microbenchmarks. These measurements ran while local services were active, so attribute no regression from the parameter-learning change; use a quiet, comparable run for finer timing conclusions. All benchmark targets completed; no services were started or stopped.


## Follow-up run (2026-11-03, learning history UI fields)

### AutoTrader strategy evaluation

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation | 1.2–19.2 ns/evaluation | 1.1–17.3 ns/evaluation | Within previous range |
| Strategy position evaluation | 1.1–2.5 ns/evaluation | 1.1–2.6 ns/evaluation | Within previous range |
| Signal serialization | 516.4 ns/operation | 506.3 ns/operation | -2.0% |
| Market depth assessment | 662.0 ns/operation | 560.1 ns/operation | -15.4% |
| Limit order request construction | 413.1 ns/operation | 405.7 ns/operation | -1.8% |
| Strategy parameter validation | 2.0 ns/operation | 1.9 ns/operation | -5.0% |

Same optimized strategy_evaluation target and WSL2 environment. The added learning P&L profile field is not exercised by these evaluator microbenchmarks. All benchmark workloads completed; no services were started or stopped.


## Follow-up run (2026-11-03, learning history UI fields)

### AutoTrader strategy evaluation

| Workload | Latest comparable run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation | 1.2–19.2 ns/evaluation | 1.1–17.3 ns/evaluation | Within previous range |
| Strategy position evaluation | 1.1–2.5 ns/evaluation | 1.1–2.6 ns/evaluation | Within previous range |
| Signal serialization | 516.4 ns/operation | 506.3 ns/operation | -2.0% |
| Market depth assessment | 662.0 ns/operation | 560.1 ns/operation | -15.4% |
| Limit order request construction | 413.1 ns/operation | 405.7 ns/operation | -1.8% |
| Strategy parameter validation | 2.0 ns/operation | 1.9 ns/operation | -5.0% |

Same optimized strategy_evaluation target and WSL2 environment. The added learning P&L profile field is not exercised by these evaluator microbenchmarks. All benchmark workloads completed; no services were started or stopped.


## Follow-up run (2026-11-03)

### Strategy and auto-trader (auto_trader)

| Workload | Result | 2026-11-02 baseline | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.2–17.9 ns/evaluation | 1.2–19.2 ns/evaluation | Within latest range. |
| Position evaluation range | 1.1–2.5 ns/evaluation | 1.1–2.5 ns/evaluation | Within latest range. |
| Signal serialization | 521.4 ns/operation | 516.4 ns/operation | +1.0%. |
| Market depth assessment | 596.2 ns/operation | 662.0 ns/operation | -9.9%. |
| Limit order request construction | 409.7 ns/operation | 413.1 ns/operation | -0.8%. |
| Strategy parameter validation | 2.1 ns/operation | 2.0 ns/operation | +5.0%; single-run variation. |

All listed workloads completed. The live HTTP benchmark was skipped because BENCH_SERVICE_URL was unset. Compared with the latest prior auto_trader run. Entry and position evaluation remained within their prior ranges.


## Follow-up run (2026-11-03, independent Scout evaluation)

### Auto-trader strategy evaluation

| Workload | Latest prior run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.2–19.2 ns/evaluation | 1.1–21.5 ns/evaluation | Upper reading +12.0% |
| Strategy position evaluation range | 1.1–2.5 ns/evaluation | 1.1–2.6 ns/evaluation | Upper reading +4.0% |
| Signal serialization | 521.4 ns/operation | 501.9 ns/operation | -3.7% |
| Market depth assessment | 596.2 ns/operation | 551.0 ns/operation | -7.6% |
| Limit order request construction | 409.7 ns/operation | 393.7 ns/operation | -3.9% |
| Strategy parameter validation | 2.1 ns/operation | 1.9 ns/operation | -9.5% |

Same optimized Cargo bench target and WSL2 environment. The Scout candidate generation and shadow registration added here are outside these evaluator microbenchmarks. The upper evaluator readings were slightly above the previous single run while the other measured operations were faster; local trading services were active, so treat the evaluator maxima as unconfirmed timing variation rather than an attributable regression. AutoTrader tests passed (183 tests). No benchmark targets were skipped or failed; no services were started or stopped during measurement.


## Repeat run (2026-11-03, independent Scout evaluation)

### Auto-trader strategy evaluation

| Workload | Previous run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.1–21.5 ns/evaluation | 1.2–16.7 ns/evaluation | Within previous range |
| Strategy position evaluation range | 1.1–2.6 ns/evaluation | 1.1–1.9 ns/evaluation | Within previous range |
| Signal serialization | 501.9 ns/operation | 506.7 ns/operation | +1.0% |
| Market depth assessment | 551.0 ns/operation | 579.3 ns/operation | +5.1% |
| Limit order request construction | 393.7 ns/operation | 407.0 ns/operation | +3.4% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |

Same optimized Cargo bench target and WSL2 environment. Local services remained active. Evaluator maxima returned within the prior run's range on repeat; helper timings moved by 1–5%, consistent with single-run variation. The Scout candidate and shadow registration paths are outside the measured loops. AutoTrader tests passed (183 tests). No benchmark targets were skipped or failed; no services were started or stopped during measurement.


## Follow-up run (2026-11-03, Insights Scout comparison metadata)

### Insights calculation benchmark

| Workload | Latest prior run | Current run | Change |
|---|---:|---:|---:|
| Live insight snapshot | 11,179.2 / 12,004.0 ns/calculation | 10,760.7 ns/calculation | Below prior readings |
| Snapshot including explanations | 12,110.2 / 11,923.0 ns/calculation | 11,035.8 ns/calculation | Below prior readings |
| Research feature extraction | 248.8 / 249.3 ns/calculation | 229.5 ns/calculation | Below prior readings |
| Price return context | 17.6 / 17.3 ns/calculation | 16.7 ns/calculation | Within recent range |
| Market input diagnostics | 337.1 / 315.9 ns/calculation | 312.5 ns/calculation | Within recent range |
| Book snapshot | 45.3 / 43.5 ns/calculation | 42.1 ns/calculation | Within recent range |

Same optimized Insights benchmark target and WSL2 environment with local services active. All workloads were within or below the most recent readings; no performance degradation was observed. Insights tests passed (25 passed, 1 intentionally ignored). No benchmark targets were skipped or failed.


## Repeat run (2026-11-03, Scout market-book fallback)

### Auto-trader strategy evaluation

| Workload | Previous run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.2–16.7 ns/evaluation | 1.1–16.3 ns/evaluation | Within previous range |
| Strategy position evaluation range | 1.1–1.9 ns/evaluation | 1.1–1.9 ns/evaluation | No range change |
| Signal serialization | 506.7 ns/operation | 515.1 ns/operation | +1.7% |
| Market depth assessment | 579.3 ns/operation | 578.5 ns/operation | -0.1% |
| Limit order request construction | 407.0 ns/operation | 427.8 ns/operation | +5.1% |
| Strategy parameter validation | 1.9 ns/operation | 2.1 ns/operation | +10.5% (+0.2 ns) |

Same optimized Cargo bench target and WSL2 environment with local services active. Evaluator ranges were unchanged or slightly faster. Signal helper timings varied by 0–5%; these are single-run measurements outside the changed Scout market-fetch path. AutoTrader tests passed (184 tests). No benchmark targets were skipped or failed.


## Follow-up run (2026-11-03, live Scout diagnostics)

### Auto-trader strategy evaluation

| Workload | Previous run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.1–16.3 ns/evaluation | 1.2–16.6 ns/evaluation | Within previous range |
| Strategy position evaluation range | 1.1–1.9 ns/evaluation | 1.1–2.9 ns/evaluation | Upper reading +52.6% |
| Signal serialization | 515.1 ns/operation | 505.4 ns/operation | -1.9% |
| Market depth assessment | 578.5 ns/operation | 573.0 ns/operation | -1.0% |
| Limit order request construction | 427.8 ns/operation | 423.3 ns/operation | -1.1% |
| Strategy parameter validation | 2.1 ns/operation | 1.9 ns/operation | -9.5% |

Same optimized Cargo bench target and WSL2 environment with local services active. The position evaluation maximum rose on one strategy while the other measurements were stable or faster; the new diagnostics are outside the evaluator benchmark. Treat this isolated maximum as unconfirmed single-run variation. AutoTrader tests passed (184 tests). No benchmark targets were skipped or failed.


## Follow-up run (2026-11-03, active Scout API counters)

### Auto-trader strategy evaluation

| Workload | Previous run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.2–16.6 ns/evaluation | 1.2–16.2 ns/evaluation | Within previous range |
| Strategy position evaluation range | 1.1–2.9 ns/evaluation | 1.1–1.9 ns/evaluation | Within previous range |
| Signal serialization | 505.4 ns/operation | 526.8 ns/operation | +4.2% |
| Market depth assessment | 573.0 ns/operation | 570.5 ns/operation | -0.4% |
| Limit order request construction | 423.3 ns/operation | 431.9 ns/operation | +2.0% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |

Same optimized Cargo bench target and WSL2 environment with local services active. Entry and position evaluation ranges improved; helper timings varied by up to 4.2%. The API count and diagnostic changes are outside the measured evaluator paths. AutoTrader tests passed (184 tests). No benchmark targets were skipped or failed.


## Follow-up run (2026-11-03, paired Scout parameter evaluation)

### Auto-trader strategy evaluation

| Workload | Latest prior run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.2–16.6 ns/evaluation | 1.5–17.0 ns/evaluation | Upper reading +2.4% |
| Strategy position evaluation range | 1.2–2.9 ns/evaluation | 1.1–1.8 ns/evaluation | Within prior range |
| Signal serialization | 505.4 ns/operation | 512.4 ns/operation | +1.4% |
| Market depth assessment | 573.0 ns/operation | 571.8 ns/operation | -0.2% |
| Limit order request construction | 423.3 ns/operation | 430.1 ns/operation | +1.6% |
| Strategy parameter validation | 1.9 ns/operation | 2.0 ns/operation | +5.3% (+0.1 ns) |

### Insights calculation benchmark

| Workload | Latest prior run | Current repeat run | Change |
|---|---:|---:|---:|
| Live insight snapshot | 10,760.7 ns/calculation | 11,671.6 ns/calculation | +8.5% |
| Snapshot including explanations | 11,035.8 ns/calculation | 12,020.1 ns/calculation | +8.9% |
| Research feature extraction | 229.5 ns/calculation | 243.2 ns/calculation | +6.0% |
| Price return context | 16.7 ns/calculation | 17.6 ns/calculation | +5.4% |
| Market input diagnostics | 312.5 ns/calculation | 368.2 ns/calculation | +17.8% |
| Book snapshot | 42.1 ns/calculation | 46.2 ns/calculation | +9.7% |

AutoTrader timings remained close to the previous run. Insights snapshots were about 9% slower in both runs, while price-return context returned to its prior range after one noisy 32.3 ns reading. The Insights repeat ran with a Vite server left active by the React test process; benchmark results are environment-contaminated and should be repeated when the workspace is idle. AutoTrader tests passed (184); Insights tests passed (25, 1 intentionally ignored). Both benchmark targets completed; no benchmark targets were skipped. No services were started or stopped for measurement.


## Follow-up run (2026-11-03, shared daily Scout cap)

### Auto-trader strategy evaluation

| Workload | Previous run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.2–16.2 ns/evaluation | 1.4–17.8 ns/evaluation | Upper reading +7.2% |
| Strategy position evaluation range | 1.1–1.9 ns/evaluation | 1.1–2.4 ns/evaluation | Upper reading +26.3% |
| Signal serialization | 526.8 ns/operation | 535.4 ns/operation | +1.6% |
| Market depth assessment | 570.5 ns/operation | 624.5 ns/operation | +9.5% |
| Limit order request construction | 431.9 ns/operation | 442.9 ns/operation | +2.5% |
| Strategy parameter validation | 1.9 ns/operation | 2.1 ns/operation | +10.5% (+0.2 ns) |

Same optimized Cargo bench target and WSL2 environment with local services active. The upper evaluator and helper readings increased in this single run; the shared-cap bookkeeping change is outside the evaluator microbenchmark. Treat these as unconfirmed local timing variation and compare again in a quiet run before attributing them to code. AutoTrader tests passed (184 tests). No benchmark targets were skipped or failed.


## Repeat run (2026-11-03, paired Scout comparison with test server active)

| AutoTrader workload | Latest recorded run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.2–16.6 ns/evaluation | 1.4–18.3 ns/evaluation | Upper reading +10.2% |
| Strategy position evaluation range | 1.2–2.9 ns/evaluation | 1.2–4.5 ns/evaluation | Upper reading +55.2% |
| Signal serialization | 505.4 ns/operation | 539.6 ns/operation | +6.8% |
| Market depth assessment | 573.0 ns/operation | 597.4 ns/operation | +4.3% |
| Limit order request construction | 423.3 ns/operation | 444.5 ns/operation | +5.0% |
| Strategy parameter validation | 1.9 ns/operation | 2.0 ns/operation | +5.3% (+0.1 ns) |

The npm test process was interrupted after its LiveModels test remained open for over ten minutes; its Vite child was still running during this benchmark and may have contended for CPU. Treat the evaluator maxima and helper timings as an environment-contaminated reading, not a code regression. The service was left running in accordance with the workspace instruction not to stop services. Re-run the benchmark when the workspace is idle before drawing a performance conclusion.


## Follow-up run (2026-11-04, daily Scout resampling cooldown)

### Auto-trader strategy evaluation

| Workload | Previous run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.2–16.2 ns/evaluation | 1.4–17.3 ns/evaluation | Upper reading +6.8% |
| Strategy position evaluation range | 1.1–2.4 ns/evaluation | 1.1–2.0 ns/evaluation | Within previous range |
| Signal serialization | 535.4 ns/operation | 494.0 ns/operation | -7.7% |
| Market depth assessment | 624.5 ns/operation | 574.8 ns/operation | -8.0% |
| Limit order request construction | 442.9 ns/operation | 444.6 ns/operation | +0.4% |
| Strategy parameter validation | 2.1 ns/operation | 2.0 ns/operation | -4.8% |

Same optimized Cargo bench target and WSL2 environment with local services active. The upper strategy evaluator reading was slightly above the previous single run; all helper timings were unchanged or faster. The daily Scout cooldown adjustment is outside these microbenchmarks, so treat the evaluator maximum as unconfirmed timing variation. AutoTrader tests passed (185 tests). No benchmark targets were skipped or failed.


## Follow-up run (2026-11-04)

### Auto-trader strategy parameter and post-only expiry

| Workload | Previous recorded result | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.4–17.3 ns/evaluation | 1.1–16.7 ns/evaluation | Within prior range |
| Strategy position evaluation range | 1.1–2.0 ns/evaluation | 1.1–2.1 ns/evaluation | +0.1 ns at upper end |
| Signal serialization | 494.0 ns/operation | 518.6 ns/operation | +5.0% |
| Market depth assessment | 574.8 ns/operation | 605.8 ns/operation | +5.4% |
| Limit order request construction | 444.6 ns/operation | 439.6 ns/operation | -1.1% |
| Strategy parameter validation | 2.0 ns/operation | 2.0 ns/operation | No change |

Same auto-trader benchmark target and optimized Cargo bench profile. The change adds a bounded post-only timeout parameter to each strategy profile and uses that value to calculate order expiry. This is a single run; the measured changes are not necessarily caused by the parameter change. Tests: 189 passed. Auto-trader benchmark completed; bench-mode tests were ignored as expected. No services were started or stopped.
running 187 tests
test auto_trader::alternative_strategies::tests::default_accounts_disabled_and_independent ... ok
test auto_trader::alternative_strategies::tests::covered_call_enforces_published_minimum_and_quantity_step ... ok
test auto_trader::alternative_strategies::tests::covered_call_cash_equity_and_expiry_payoff ... ok
test auto_trader::alternative_strategies::api::tests::api_rejects_historical_observations ... ok
test auto_trader::alternative_strategies::tests::held_options_require_exact_contract_and_official_settlement ... ok
test auto_trader::alternative_strategies::tests::disabled_paused_or_unsimulated_cannot_open ... ok
test auto_trader::alternative_strategies::tests::delayed_redemption_does_not_realize_projected_profit ... ok
test auto_trader::alternative_strategies::tests::liquidation_net_costs_and_duplicate_opportunity ... ok
test auto_trader::alternative_strategies::api::tests::load_error_prevents_overwrite ... ok
test auto_trader::alternative_strategies::tests::liquidity_fees_accrue_once_and_exit_preserves_costs ... ok
test auto_trader::alternative_strategies::api::tests::invalid_observation_cannot_partially_reset_daily_ledger ... ok
test auto_trader::alternative_strategies::api::tests::stale_or_duplicate_native_status_cannot_replace_current_book ... ok
test auto_trader::alternative_strategies::tests::liquidity_rejects_fee_growth_from_another_range ... ok
test auto_trader::alternative_strategies::tests::loss_limit_resets_only_on_next_utc_day ... ok
test auto_trader::alternative_strategies::tests::lp_amounts_outside_range_are_one_sided ... ok
test auto_trader::alternative_strategies::tests::liquidity_uses_exact_initialized_tick_bounds_and_rejects_inconsistent_ticks ... ok
test auto_trader::alternative_strategies::tests::residual_rejects_scores_that_disagree_with_causal_history ... ok
test auto_trader::alternative_strategies::tests::observation_wire_roundtrip ... ok
test auto_trader::alternative_strategies::api::tests::failed_persistence_does_not_change_in_memory_settings ... ok
test auto_trader::alternative_strategies::tests::residual_portfolio_balances_and_borrow_is_charged_without_rebalance ... ok
test auto_trader::alternative_strategies::api::tests::native_batch_isolates_bad_strategy_and_persists_healthy_opportunity ... ok
test auto_trader::alternative_strategies::tests::stale_quotes_and_out_of_order_events_are_rejected ... ok
test auto_trader::alternative_strategies::api::tests::fresh_observation_is_idempotent_and_persists_across_restart ... ok
test auto_trader::alternative_strategies::tests::residual_rejects_unsynchronized_or_future_history ... ok
test auto_trader::alternative_strategies::api::tests::native_readiness_is_required_to_enable_and_disable_remains_available ... ok
test auto_trader::dated_carry_tests::entry_rejects_prices_that_do_not_match_the_published_basis ... ok
test auto_trader::candidate_diagnostic_tests::supported_candidate_passes_strict_and_scout_checks_and_audit_omits_rejections ... ok
test auto_trader::dated_carry_tests::entry_requires_executable_spot_future_edge_above_cost_and_seven_days_to_expiry ... ok
test auto_trader::dated_carry_tests::exit_requires_a_valid_settlement_signal_for_an_expired_contract ... ok
test auto_trader::cycle_result_helper_tests::skipped_results_preserve_signal_identity_and_reason ... ok
test auto_trader::deep_book_assessment_tests::deep_book_assessment_handles_buy_sell_capacity_impact_and_invalid_markets ... ok
test auto_trader::closed_trade_metrics_migration_tests::clears_legacy_outcomes_without_changing_ledger_pnl ... ok
test auto_trader::candidate_evidence_format_tests::renders_present_and_missing_walk_forward_metrics ... ok
test auto_trader::dated_carry_tests::scout_shadow_markout_records_positive_and_negative_net_returns ... ok
test auto_trader::deep_book_assessment_tests::filters_invalid_book_levels_and_computes_each_side_depth_inside_band ... ok
test auto_trader::daily_strategy_allocator_tests::daily_budget_and_position_slots_are_split_across_selected_daily_strategies ... ok
test auto_trader::candidate_diagnostic_tests::rejected_candidate_diagnostic_and_audit_explain_strength_score_and_scout_inventory ... ok
test auto_trader::exit_policy_learning_tests::extended_hold_requires_enough_positive_outcomes_and_respects_reward_risk ... ok
test auto_trader::dated_carry_tests::lower_expiry_probe_can_shadow_an_expiry_filtered_by_the_baseline ... ok
test auto_trader::dated_carry_shadow_probe_learning_tests::promotion_uses_one_day_shadow_markouts_for_dated_carry_variants ... ok
test auto_trader::donchian_20_btc200_registry_tests::selected_breakout_exits_only_on_daily_invalidation ... ok
test auto_trader::execution_stage_timings_tests::extracts_valid_stage_times_and_clamps_transport_remainder ... ok
test auto_trader::execution_stage_timings_tests::missing_handler_keeps_submit_and_response_timings_only ... ok
test auto_trader::dated_carry_tests::shadow_probe_dedupes_maturities_for_one_asset_and_parameter_variant ... ok
test auto_trader::candidate_evidence_tests::selects_matching_entry_horizon_and_checks_directional_evidence ... ok
test auto_trader::external_close_audit_tests::close_quantity_detects_partial_full_and_reversing_closes_without_counting_adds ... ok
test auto_trader::dated_carry_shadow_probe_learning_tests::dated_carry_shadow_markouts_promote_the_tested_pair_in_its_profile_context ... ok
test auto_trader::external_close_audit_tests::external_close_payload_captures_entry_and_path_context ... ok
test auto_trader::exit_policy_learning_tests::learned_exit_policy_requires_validated_positive_evidence_for_matching_side ... ok
test auto_trader::gate_pass_distance_tests::computes_normalized_distance_for_signal_candidate_and_market_gates ... ok
test auto_trader::external_close_audit_tests::legacy_manual_close_without_strategy_metadata_still_gets_an_audit_order ... ok
test auto_trader::live_scout_direction_tests::learned_book_depth_parameter_selects_the_matching_insight ... ok
test auto_trader::live_scout_direction_tests::live_parameter_probe_can_sample_a_direction_rejected_by_baseline ... ok
test auto_trader::gap_reason_category_tests::summarizes_common_entry_gap_reasons ... ok
test auto_trader::gate_pass_distance_tests::strict_sample_and_capacity_gaps_do_not_report_negative_distance ... ok
test auto_trader::loss_limit_probability_tests::missing_fx_error_distinguishes_uncached_and_cached_quote_rates ... ok
test auto_trader::loss_limit_probability_tests::normal_cdf_is_symmetric_and_has_expected_center_and_tails ... ok
test auto_trader::inactive_execution_order_tests::removes_unfilled_and_partially_filled_orders_from_pending_state ... ok
test auto_trader::inactive_execution_order_tests::scheduled_orders_are_not_checked_against_the_resting_order_ledger ... ok
test auto_trader::live_scout_direction_tests::conflicting_live_momentum_inputs_do_not_create_two_opposing_scouts ... ok
test auto_trader::exit_policy_learning_tests::risk_and_volatility_adjustments_stay_within_policy_bounds ... ok
test auto_trader::external_close_audit_tests::partial_manual_close_is_allocated_to_the_strategy_and_persisted_as_an_order ... ok
test auto_trader::live_scout_direction_tests::neutral_aggregate_signal_can_produce_independent_live_scout_directions ... ok
test auto_trader::loss_limit_probability_tests::probability_fails_closed_for_missing_depth_invalid_levels_short_history_or_zero_variance ... ok
test auto_trader::maker_reconciliation_tests::maker_fills_are_recovered_after_order_disappears ... ok
test auto_trader::loss_limit_probability_tests::profitable_close_probability_uses_bid_for_longs_ask_for_shorts_and_price_history ... ok
test auto_trader::missed_opportunity_learning_behavior_tests::adaptive_floor_uses_only_validated_short_horizon_stop_target_bands ... ok
test auto_trader::order_request_tests::builds_basic_and_remaining_order_variants ... ok
test auto_trader::missed_opportunity_learning_behavior_tests::longer_horizon_multiplier_requires_positive_stop_target_and_timely_profit_evidence ... ok
test auto_trader::order_request_tests::builds_scheduled_order_variants ... ok
test auto_trader::order_request_tests::builds_price_and_trigger_order_variants ... ok
test auto_trader::order_request_tests::post_only_expiry_uses_configured_timeout ... ok
test auto_trader::order_type_selection_behavior_tests::order_type_adaptation_requires_validated_fill_and_cost_advantage ... ok
test auto_trader::order_type_selection_behavior_tests::exploration_assignment_is_stable_and_only_uses_parameterized_order_types ... ok
test auto_trader::order_type_selection_behavior_tests::parameter_profiles_explore_unobserved_cohorts_then_select_lowest_validated_cost ... ok
test auto_trader::paired_scout_parameter_tests::live_parameter_promotion_requires_positive_paired_improvement ... ok
test auto_trader::parallel_daily_candidate_tests::daily_scouts_are_shadow_only ... ok
test auto_trader::order_type_selection_behavior_tests::order_selection_uses_aggressive_rules_then_passive_fallback ... ok
test auto_trader::parallel_daily_candidate_tests::same_side_daily_strategy_is_not_blocked_by_another_strategy_lot ... ok
test auto_trader::parallel_daily_cycle_tests::daily_enter_candidates_are_shadowed_when_selected_as_scouts ... ok
test auto_trader::parallel_daily_cycle_tests::daily_scout_can_be_sampled_again_after_five_minutes ... ok
test auto_trader::parallel_daily_candidate_tests::live_signal_sources_remain_available_alongside_daily_candidates ... ok
test auto_trader::alternative_strategies::tests::fixture_replays_all_five_accounts_without_starting_services ... ok
test auto_trader::order_type_selection_behavior_tests::schedule_threshold_learning_requires_paired_samples_and_moves_by_cost_and_fill_quality ... ok
test auto_trader::parallel_daily_cycle_tests::daily_strategy_candidates_outside_donchian_can_become_scouts ... ok
test auto_trader::parallel_daily_cycle_tests::batch_selects_one_highest_priority_strategy_candidate_per_symbol ... ok
test auto_trader::parallel_daily_cycle_tests::full_shadow_sample_limit_does_not_block_real_exploratory_entries ... ok
test auto_trader::parallel_daily_cycle_tests::daily_strategy_scout_is_kept_in_the_candidate_batch ... ok
test auto_trader::parallel_daily_candidate_tests::failed_rsi_daily_candidate_cannot_fall_back_to_strict_selection ... ok
test auto_trader::pending_signal_queue_tests::daily_and_live_signals_use_independent_queues ... ok
test auto_trader::parallel_daily_cycle_tests::parameter_probe_audit_preserves_the_exact_value_and_entry_result ... ok
test auto_trader::parallel_daily_cycle_tests::live_parameter_probe_cooldown_remains_one_hour ... ok
test auto_trader::persistence_allocation_tests::allocated_symbol_exposure_counts_open_lots_and_only_incremental_pending_risk ... ok
test auto_trader::parallel_daily_cycle_tests::scout_shadow_sample_has_the_contract_required_for_learning_outcomes ... ok
test auto_trader::persistence_allocation_tests::pending_entries_are_unique_and_existing_strategy_positions_are_excluded ... ok
test auto_trader::parallel_daily_cycle_tests::selected_live_strategy_is_shadowed_even_when_enabled_for_trading ... ok
test auto_trader::portfolio_hedge_order_tests::portfolio_hedge_order_has_expected_identity_and_reservation ... ok
test auto_trader::position_exit_order_tests::position_exit_order_preserves_strategy_and_entry_decision ... ok
test auto_trader::persistence_allocation_tests::category_limits_and_quote_conversion_use_safe_fallbacks ... ok
test auto_trader::position_exit_order_tests::position_exit_order_uses_live_insights_fallback ... ok
test auto_trader::pending_exit_tests::blocks_reentry_while_strategy_exit_is_pending ... ok
test auto_trader::position_exit_profit_protection_tests::legacy_price_path_seeds_long_and_short_profit_peaks_and_ignores_bad_marks ... ok
test auto_trader::persistence_allocation_tests::strategy_budgets_allocate_configured_remainder_and_exposure ... ok
test auto_trader::persistence_allocation_tests::category_allocation_and_open_counts_support_strategy_lots_and_legacy_positions ... ok
test auto_trader::protective_stop_audit_tests::audit_payload_records_barrier_fill_and_duration ... ok
test auto_trader::protective_stop_order_tests::protective_stop_retains_entry_attribution_and_barrier_id ... ok
test auto_trader::persistence_allocation_tests::scout_positions_do_not_consume_real_open_position_caps ... ok
test auto_trader::position_exit_profit_protection_tests::peak_profit_is_raised_and_giveback_crosses_the_minimum_lock_floor ... ok
test auto_trader::reason_gate_hits_tests::maps_strategy_and_position_failures_without_duplicate_gates ... ok
test auto_trader::position_exit_profit_protection_tests::strategy_parameters_and_legacy_aliases_are_bounded ... ok
test auto_trader::registered_strategy_contract_tests::every_registered_strategy_has_a_complete_contract ... ok
test auto_trader::reason_symbol_gate_hits_tests::extracts_early_candidate_and_strategy_gate_details_with_distances ... ok
test auto_trader::risk_adjusted_monthly_momentum_sizing_tests::strategy_uses_volatility_multiplier_and_fails_closed_when_missing ... ok
test auto_trader::risk_adjusted_monthly_momentum_tests::same_symbol_rebalances_only_when_month_anchor_advances ... ok
test auto_trader::scout_parameter_combination_tests::validated_pair_outcome_promotes_both_values_together ... ok
test auto_trader::risk_adjusted_monthly_momentum_sizing_tests::recent_strategy_learning_includes_scout_shadow_outcomes_across_symbols ... ok
test auto_trader::reason_gate_hits_tests::maps_combined_candidate_reasons_to_distinct_gates ... ok
test auto_trader::scout_parameter_scheduling_tests::live_strategy_probe_rotates_one_bounded_parameter_value ... ok
test auto_trader::scheduled_order_reconciliation_tests::recovers_scheduled_orders_misclassified_by_resting_order_reconciliation ... ok
test auto_trader::scout_parameter_variant_tests::breakout_entry_distance_promotes_per_symbol_after_confident_probe_outcomes ... ok
test auto_trader::scout_parameter_variant_tests::feedback_selects_the_best_probed_value_without_pooling_it_with_baseline_samples ... ok
test auto_trader::risk_adjusted_monthly_momentum_registry_tests::strategy_is_registered_and_uses_signal_exit_without_fixed_stops ... ok
test auto_trader::scout_parameter_variant_tests::probe_does_not_replace_the_profile_before_minimum_evidence ... ok
test auto_trader::scout_parameter_combination_tests::pair_probes_change_two_bounded_parameters_and_have_stable_keys ... ok
test auto_trader::scout_shadow_parameter_learning_tests::scout_outcomes_contribute_to_strategy_parameter_learning ... ok
test auto_trader::signal_execution_budget_tests::configured_budgets_bypass_signal_weighting_and_order_multipliers_validate_contracts ... ok
test auto_trader::signal_execution_budget_tests::recent_net_edge_and_learned_size_multiplier_handle_empty_profitable_and_losing_samples ... ok
test auto_trader::stablecoin_pair_tests::recognizes_stablecoin_pairs_case_insensitively_and_rejects_other_symbols ... ok
test auto_trader::signal_execution_budget_tests::signal_weighted_daily_budget_keeps_a_floor_and_rewards_eligible_signals ... ok
test auto_trader::strategy_dated_futures_carry::evaluate::tests::settlement_exits_only_with_valid_expired_position_data ... ok
test auto_trader::strategy_dated_futures_carry::evaluate::tests::entry_rejects_invalid_expiry_market_and_stale_metrics ... ok
test auto_trader::strategy_donchian_10_btc200::evaluate::parameter_bounds_donchian_10_btc200::parameter_bounds_donchian_10_btc200 ... ok
test auto_trader::strategy_donchian_10_btc200::evaluate::tests::accepts_only_its_own_published_candidate_signal ... ok
test auto_trader::strategy_dated_futures_carry::evaluate::tests::entry_uses_profile_edge_gate_and_rejects_missing_parameters ... ok
test auto_trader::strategy_donchian_55_btc200::evaluate::tests::accepts_only_its_own_published_candidate_signal ... ok
test auto_trader::strategy_donchian_20_btc200_hold3::evaluate::parameter_bounds_donchian_20_btc200_hold3::parameter_bounds_donchian_20_btc200_hold3 ... ok
test auto_trader::strategy_live_momentum::evaluate::tests::entry_requires_signal_aligned_momentum_or_trend ... ok
test auto_trader::strategy_live_momentum::evaluate::parameter_bounds_live_momentum::parameter_bounds_live_momentum ... ok
test auto_trader::strategy_dated_futures_carry::evaluate::tests::entry_requires_a_consistent_eligible_candidate ... ok
test auto_trader::strategy_dated_futures_carry::evaluate::tests::lifecycle_context_holds_or_rejects_mismatched_signals ... ok
test auto_trader::strategy_donchian_20_btc200::evaluate::tests::accepts_only_its_own_published_candidate_signal ... ok
test auto_trader::strategy_donchian_20_btc200_hold3::evaluate::tests::accepts_only_its_own_published_candidate_signal ... ok
test auto_trader::strategy_lot_pnl_tests::closing_return_uses_the_strategy_lot_basis ... ok
test auto_trader::strategy_donchian_20_sma200::evaluate::tests::accepts_only_its_own_published_candidate_signal ... ok
test auto_trader::strategy_lot_pnl_tests::real_fill_turns_scout_only_aggregate_into_real_position ... ok
test auto_trader::strategy_parameter_bounds_tests::daily_rule_inputs_are_registered_for_learning_profiles ... ok
test auto_trader::strategy_donchian_55_btc200::evaluate::parameter_bounds_donchian_55_btc200::parameter_bounds_donchian_55_btc200 ... ok
test auto_trader::strategy_relative_strength_90_sma200::evaluate::tests::accepts_only_its_own_published_candidate_signal ... ok
test auto_trader::strategy_donchian_20_sma200::evaluate::parameter_bounds_donchian_20_sma200::parameter_bounds_donchian_20_sma200 ... ok
test auto_trader::strategy_donchian_20_btc200::evaluate::parameter_bounds_donchian_20_btc200::parameter_bounds_donchian_20_btc200 ... ok
test auto_trader::strategy_risk_adjusted_monthly_momentum::evaluate::parameter_bounds_risk_adjusted_monthly_momentum::parameter_bounds_risk_adjusted_monthly_momentum ... ok
test auto_trader::strategy_risk_adjusted_monthly_momentum::evaluate::tests::accepts_only_its_own_published_candidate_signal ... ok
test auto_trader::strategy_relative_strength_90_sma200::evaluate::parameter_bounds_relative_strength_90_sma200::parameter_bounds_relative_strength_90_sma200 ... ok
test auto_trader::strategy_risk_adjusted_trend_portfolio::evaluate::tests::accepts_only_its_own_published_candidate_signal ... ok
test auto_trader::strategy_risk_adjusted_trend_portfolio::evaluate::parameter_bounds_risk_adjusted_trend_portfolio::parameter_bounds_risk_adjusted_trend_portfolio ... ok
test auto_trader::strategy_rsi2_oversold_rebound::evaluate::tests::accepts_only_its_own_published_candidate_signal ... ok
test auto_trader::strategy_rsi2_oversold_rebound::evaluate::parameter_bounds_rsi2_oversold_rebound::parameter_bounds_rsi2_oversold_rebound ... ok
test auto_trader::strategy_live_trade_flow::evaluate::parameter_bounds_live_trade_flow::parameter_bounds_live_trade_flow ... ok
test auto_trader::strategy_signal_contract_tests::every_live_strategy_evaluates_its_signal_and_rejects_missing_signal ... ok
test auto_trader::strategy_signal_contract_tests::every_registered_strategy_returns_an_explicit_position_decision ... ok
test auto_trader::strategy_signal_contract_tests::every_daily_strategy_enters_only_from_its_own_benchmark_signal ... ok
test auto_trader::strategy_trend_50_200::evaluate::tests::accepts_only_its_own_published_candidate_signal ... ok
test auto_trader::strategy_signal_contract_tests::legacy_live_insights_parameter_bounds_are_enforced ... ok
test auto_trader::strategy_trend_sleeves_btc200::evaluate::tests::accepts_only_its_own_published_candidate_signal ... ok
test auto_trader::strategy_trend_50_200::evaluate::parameter_bounds_trend_50_200::parameter_bounds_trend_50_200 ... ok
test auto_trader::strategy_trend_sleeves_btc200::evaluate::parameter_bounds_trend_sleeves_btc200::parameter_bounds_trend_sleeves_btc200 ... ok
test auto_trader::strategy_live_order_book::evaluate::parameter_bounds_live_order_book::parameter_bounds_live_order_book ... ok
test auto_trader::tests::position_exit_audit_handles_zero_quantity_metrics ... ok
test auto_trader::tests::position_exit_audit_includes_execution_and_entry_context ... ok
test auto_trader::threshold_walk_forward_tests::extracts_the_signal_feature_for_each_learned_threshold ... ok
test auto_trader::strategy_live_absorption_pressure::evaluate::parameter_bounds_live_absorption_pressure::parameter_bounds_live_absorption_pressure ... ok
test auto_trader::strategy_live_vwap_dislocation::evaluate::parameter_bounds_live_vwap_dislocation::parameter_bounds_live_vwap_dislocation ... ok
test auto_trader::threshold_walk_forward_tests::leaves_threshold_unchanged_without_enough_holdout_samples ... ok
test auto_trader::threshold_walk_forward_tests::negative_pooled_live_results_tighten_parameter_when_no_threshold_validates ... ok
test auto_trader::threshold_walk_forward_tests::parameter_profile_uses_validated_walk_forward_threshold ... ok
test auto_trader::threshold_walk_forward_tests::profitable_scout_outcomes_relax_scout_walk_forward_thresholds_within_bounds ... ok
test auto_trader::threshold_walk_forward_tests::rsi_entry_ceiling_uses_daily_rsi_feature_without_directional_wire_data ... ok
test auto_trader::threshold_walk_forward_tests::tightens_a_threshold_only_when_later_holdout_edge_improves ... ok
test auto_trader::strategy_parameter_bounds_tests::every_registered_strategy_accepts_bounds_and_rejects_values_outside_them ... ok
test auto_trader::signal_selection_behavior_tests::selects_strict_evidence_before_exploration_and_marks_unvalidated_signals_exploratory ... ok
test auto_trader::signal_selection_behavior_tests::scout_can_probe_pullback_strategy_without_pullback_confirmation ... ok
test auto_trader::signal_selection_behavior_tests::live_scout_uses_its_own_entry_rule_without_aggregate_score_floors ... ok
test auto_trader::signal_selection_behavior_tests::daily_strategy_candidates_take_priority_and_pullback_strategies_require_counter_momentum ... ok
test auto_trader::signal_selection_behavior_tests::live_scout_walk_forward_thresholds_are_strategy_parameters ... ok
test auto_trader::signal_selection_behavior_tests::scout_fallback_respects_inventory_and_measured_negative_history ... ok
test auto_trader::maker_reconciliation_tests::recovers_true_strategy_without_double_counting_mirrored_inventory ... ok
test auto_trader::strategy_cleanup_integration_tests::every_registered_strategy_cancels_its_protective_orders_on_exit ... ok

test result: ok. 187 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s passed (189 tests). The benchmark target completed successfully; its test binaries ran in bench mode with tests ignored as expected. No services were started or stopped.


## Follow-up run (2026-11-04, causal Scout parameter probes)

### Auto-trader strategy evaluation

| Workload | Previous run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.4–17.3 ns/evaluation | 1.2–16.8 ns/evaluation | Within previous range |
| Strategy position evaluation range | 1.1–2.0 ns/evaluation | 1.1–1.8 ns/evaluation | Within previous range |
| Signal serialization | 494.0 ns/operation | 520.1 ns/operation | +5.3% |
| Market depth assessment | 574.8 ns/operation | 609.1 ns/operation | +6.0% |
| Limit order request construction | 444.6 ns/operation | 427.4 ns/operation | -3.9% |
| Strategy parameter validation | 2.0 ns/operation | 1.9 ns/operation | -5.0% |

Same optimized Cargo bench target and WSL2 environment with local services active. Evaluator ranges improved; helper timings varied by up to 6%. The paired-probe and audit display changes are outside these microbenchmarks, so treat helper variation as single-run noise. AutoTrader tests passed (189 tests); React tests passed (88 tests). No benchmark targets were skipped or failed.


## Follow-up run (2026-11-04, paired Scout probes and outcome deltas)

### Auto-trader strategy evaluation

| Workload | Previous run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.4–17.3 ns/evaluation | 1.2–16.6 ns/evaluation | Within previous range |
| Strategy position evaluation range | 1.1–2.0 ns/evaluation | 1.1–2.2 ns/evaluation | Upper reading +10.0% |
| Signal serialization | 520.1 ns/operation | 527.4 ns/operation | +1.4% |
| Market depth assessment | 609.1 ns/operation | 614.3 ns/operation | +0.9% |
| Limit order request construction | 427.4 ns/operation | 443.6 ns/operation | +3.8% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |

Same optimized Cargo benchmark target and WSL2 environment with local services active. Entry evaluation and helper measurements remained near the prior run; the position-evaluation maximum was slightly higher in this single run. The paired Scout changes are outside the microbenchmarked paths. AutoTrader tests passed (189 tests), and React tests passed (88 tests). No benchmark targets were skipped or failed.


## Follow-up run (2026-11-04, Scout probe eligibility and paired delta display)

### Auto-trader strategy evaluation

| Workload | Previous run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.4–17.3 ns/evaluation | 1.1–16.0 ns/evaluation | Within previous range |
| Strategy position evaluation range | 1.1–2.0 ns/evaluation | 1.1–1.7 ns/evaluation | Within previous range |
| Signal serialization | 527.4 ns/operation | 486.2 ns/operation | -7.8% |
| Market depth assessment | 614.3 ns/operation | 549.0 ns/operation | -10.6% |
| Limit order request construction | 443.6 ns/operation | 413.6 ns/operation | -6.8% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |

Same optimized Cargo benchmark target and WSL2 environment. Strategy evaluator ranges improved or stayed within the previous run's range; helper timings were lower in this single run. The Scout probe changes are outside the microbenchmarked paths. AutoTrader tests passed (189 tests), and React tests passed (88 tests). No benchmark targets were skipped or failed.

## Follow-up run (2026-11-04 08:45 UTC, strategy/scout/learning crate contracts)

### Auto-trader strategy evaluation

| Workload | Previous run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.1–16.0 ns/evaluation | 1.1–16.9 ns/evaluation | Upper reading +5.6% |
| Strategy position evaluation range | 1.1–1.7 ns/evaluation | 1.1–3.9 ns/evaluation | Upper reading +129.4% |
| Signal serialization | 486.2 ns/operation | 523.7 ns/operation | +7.7% |
| Market depth assessment | 549.0 ns/operation | 616.8 ns/operation | +12.3% |
| Limit order request construction | 413.6 ns/operation | 431.5 ns/operation | +4.3% |
| Strategy parameter validation | 1.9 ns/operation | 2.2 ns/operation | +15.8% |

Same optimized `strategy_evaluation` target and WSL2 environment. This was a single run; the upper position-evaluation reading and small helper changes need a repeat before attributing them to code. The new contract, scout, and learning crate workloads are not included in this auto-trader benchmark. AutoTrader tests passed (189 tests); all new-crate tests passed. No benchmark target was skipped or failed.

### Repeat run (2026-11-04 08:47 UTC)

| Workload | Previous run | Repeat | Change vs 2026-11-04 08:45 |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.1–16.0 ns/evaluation | 1.2–19.0 ns/evaluation | Upper reading +18.8% |
| Strategy position evaluation range | 1.1–1.7 ns/evaluation | 1.1–4.1 ns/evaluation | Upper reading +141.2% |
| Signal serialization | 486.2 ns/operation | 522.7 ns/operation | +7.5% |
| Market depth assessment | 549.0 ns/operation | 592.1 ns/operation | +7.8% |
| Limit order request construction | 413.6 ns/operation | 429.9 ns/operation | +3.9% |
| Strategy parameter validation | 1.9 ns/operation | 2.0 ns/operation | +5.3% |

Same benchmark target and environment. Elevated strategy range maxima persisted across two runs, though the two new crates do not participate in these evaluator functions; treat this as an observed benchmark degradation requiring investigation before the split is considered performance-neutral. AutoTrader tests passed (189 tests). No benchmark target was skipped or failed.

## Follow-up run (2026-11-04 10:38 UTC, external profiles and Scouts crate)

### Auto-trader strategy evaluation

| Workload | Previous recorded run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.2–19.0 ns/evaluation | 1.1–18.1 ns/evaluation | Upper reading -4.7% |
| Strategy position evaluation range | 1.1–4.1 ns/evaluation | 1.1–2.1 ns/evaluation | Upper reading -48.8% |
| Signal serialization | 522.7 ns/operation | 522.4 ns/operation | -0.1% |
| Market depth assessment | 592.1 ns/operation | 599.0 ns/operation | +1.2% |
| Limit order request construction | 429.9 ns/operation | 439.9 ns/operation | +2.3% |
| Strategy parameter validation | 2.0 ns/operation | 2.3 ns/operation | +15.0% |

Same optimized `strategy_evaluation` target and WSL2 environment. The decision-path max readings moved back toward the previous recorded range; the small helper timings remain noisy single-run measurements. Scouts' parameter-probe generation now runs through the shared Scouts crate API, but the benchmark measures only AutoTrader decision helpers. All Rust tests for strategy_core, scouts, learning, auto_trader (192 tests), and insights passed; one existing public-provider connectivity test was ignored. No benchmark target was skipped or failed.

## Repeat run (2026-11-04 10:42 UTC, learner profile registration and strategy contract extraction)

### Auto-trader strategy evaluation

| Workload | Previous run | Repeat | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.1–18.1 ns/evaluation | 1.2–17.6 ns/evaluation | Upper reading -2.8% |
| Strategy position evaluation range | 1.1–2.1 ns/evaluation | 1.2–2.7 ns/evaluation | Upper reading +28.6% |
| Signal serialization | 522.4 ns/operation | 548.0 ns/operation | +4.9% |
| Market depth assessment | 599.0 ns/operation | 598.9 ns/operation | No change |
| Limit order request construction | 439.9 ns/operation | 435.7 ns/operation | -1.0% |
| Strategy parameter validation | 2.3 ns/operation | 2.0 ns/operation | -13.0% |

Same optimized target and WSL2 environment. Evaluator range maxima vary across single runs; the position maximum rose on this pass while helper measurements stayed close. No measurable work was added to these benchmarked functions by profile registration or the Scouts library extraction. All Rust tests passed before the benchmark; the ignored provider test remains an explicit external-connectivity test. No benchmark target was skipped or failed.

### New-crate benchmark targets

`cargo bench` completed for `strategy_core`, `scouts`, and `learning`. These crates currently define no benchmark workloads, so their unit-test harnesses ran with all tests ignored in bench mode; no throughput or latency numbers were produced. The real tests for those crates passed in test mode.

## Follow-up run (2026-11-04 10:47 UTC, final crate wiring)

### Auto-trader strategy evaluation

| Workload | Previous run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.2–17.6 ns/evaluation | 1.2–20.4 ns/evaluation | Upper reading +15.9% |
| Strategy position evaluation range | 1.2–2.7 ns/evaluation | 1.1–3.5 ns/evaluation | Upper reading +29.6% |
| Signal serialization | 548.0 ns/operation | 525.5 ns/operation | -4.1% |
| Market depth assessment | 598.9 ns/operation | 581.9 ns/operation | -2.8% |
| Limit order request construction | 435.7 ns/operation | 434.5 ns/operation | -0.3% |
| Strategy parameter validation | 2.0 ns/operation | 2.0 ns/operation | No change |

Same optimized target and WSL2 environment. Single-run evaluator maxima remain variable, while the helper workloads stayed level or improved. This benchmark does not exercise the new asynchronous feedback/profile exchange or probe generator. All affected Rust tests passed; the existing external-provider test was ignored. `cargo bench` also completed for each new crate, which has no dedicated benchmark workload yet; test harnesses ran with tests ignored in bench mode.

## Follow-up run (2026-11-04 11:08 UTC, scout audit visibility and maker fill updates)

### Auto-trader strategy evaluation

| Workload | Latest prior run (10:47 UTC) | Current serial run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.2–20.4 ns/evaluation | 1.2–16.8 ns/evaluation | Upper reading -17.6% |
| Strategy position evaluation range | 1.1–3.5 ns/evaluation | 1.1–2.2 ns/evaluation | Upper reading -37.1% |
| Signal serialization | 525.5 ns/operation | 528.1 ns/operation | +0.5% |
| Market depth assessment | 581.9 ns/operation | 573.0 ns/operation | -1.5% |
| Limit order request construction | 434.5 ns/operation | 400.5 ns/operation | -7.8% |
| Strategy parameter validation | 2.0 ns/operation | 1.9 ns/operation | -5.0% |

### Insights calculations

| Workload | Latest recorded run | Current serial run | Change |
|---|---:|---:|---:|
| Live insight snapshot | 11,671.6 ns/calculation | 11,647.7 ns/calculation | -0.2% |
| Snapshot including explanations | 12,020.1 ns/calculation | 11,788.9 ns/calculation | -1.9% |
| Research feature extraction | 243.2 ns/calculation | 249.3 ns/calculation | +2.5% |
| Price return context | 17.6 ns/calculation | 17.0 ns/calculation | -3.4% |
| Market input diagnostics | 368.2 ns/calculation | 326.2 ns/calculation | -11.4% |
| Book snapshot | 46.2 ns/calculation | 43.8 ns/calculation | -5.2% |

Both optimized benchmark targets completed serially under WSL2 (rustc 1.96.0, 32 reported logical CPUs). The first overlapping run was discarded because build/measurement contention made it unsuitable for comparison. The current serial results show no meaningful degradation against the latest recorded comparable run. These microbenchmarks do not measure the new maker-fill audit write or dynamic profile response construction. AutoTrader tests passed (194); Insights tests passed (25, one external-provider test intentionally ignored); React tests passed (88). No benchmark targets were skipped or failed, and no services were started or stopped.

## Follow-up run (2026-11-04, unique auto-trader order IDs)

### Auto-trader strategy evaluation

| Workload | Latest recorded run | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.2–16.8 ns/evaluation | 1.1–16.7 ns/evaluation | Within run variation |
| Strategy position evaluation range | 1.1–2.2 ns/evaluation | 1.2–1.9 ns/evaluation | Within run variation |
| Signal serialization | 528.1 ns/operation | 495.7 ns/operation | -6.1% |
| Market depth assessment | 573.0 ns/operation | 555.4 ns/operation | -3.1% |
| Limit-order request construction | 400.5 ns/operation | 396.7 ns/operation | -0.9% |
| Strategy parameter validation | 1.9 ns/operation | 2.0 ns/operation | +0.1 ns; single-run variation |

Same optimized auto-trader benchmark target, WSL2, rustc 1.96.0, and 32 reported logical CPUs. The change makes signal and position-exit order timestamps unique; it does not affect the measured evaluator paths. No meaningful performance degradation was observed. `cargo test` passed all 194 auto-trader tests. `cargo fmt` completed. No services were started or stopped.

## Follow-up run (2026-11-04 11:30 UTC, Scout-to-entry learning gate)

### Auto-trader strategy evaluation

| Workload | Latest prior run (11:08 UTC) | Current run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.2–16.8 ns/evaluation | 1.1–16.8 ns/evaluation | Same upper reading |
| Strategy position evaluation range | 1.1–2.2 ns/evaluation | 1.1–2.4 ns/evaluation | Upper reading +9.1% |
| Signal serialization | 528.1 ns/operation | 521.1 ns/operation | -1.3% |
| Market depth assessment | 573.0 ns/operation | 614.1 ns/operation | +7.2% |
| Limit order request construction | 400.5 ns/operation | 408.5 ns/operation | +2.0% |
| Strategy parameter validation | 1.9 ns/operation | 1.9 ns/operation | No change |

Same optimized benchmark target and WSL2 environment. This single run shows a higher market-depth and position maximum; those paths were not changed by Scout-to-entry learning. Treat these small microbenchmark variations as unconfirmed timing noise. The benchmark does not exercise the adaptive entry-selection path changed here. AutoTrader tests passed (196); no benchmark targets were skipped or failed, and no services were started or stopped.

## Repeat run (2026-11-04 11:33 UTC, Scout outcome entry gating)

### Auto-trader strategy evaluation

| Workload | Latest prior run (11:30 UTC) | First run (11:33 UTC) | Serial repeat (11:33 UTC) |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.1–16.8 ns/evaluation | 1.2–19.3 ns/evaluation | 1.1–16.2 ns/evaluation |
| Strategy position evaluation range | 1.1–2.4 ns/evaluation | 1.2–2.5 ns/evaluation | 1.1–1.8 ns/evaluation |
| Signal serialization | 521.1 ns/operation | 497.7 ns/operation | 512.4 ns/operation |
| Market depth assessment | 614.1 ns/operation | 561.5 ns/operation | 550.3 ns/operation |
| Limit order request construction | 408.5 ns/operation | 413.1 ns/operation | 397.3 ns/operation |
| Strategy parameter validation | 1.9 ns/operation | 2.0 ns/operation | 1.9 ns/operation |

The first run's higher evaluator maxima did not repeat; the serial repeat was within or below the prior evaluator range and showed no meaningful degradation. Helper timings varied between single runs. The benchmark does not exercise the adaptive entry-selection path changed here. AutoTrader tests passed (197); React tests passed (88). No benchmark targets were skipped or failed, and no services were started or stopped.

## Follow-up run (2026-11-04 11:43 UTC, all Scout variants feed parameter learning)

### Auto-trader strategy evaluation

| Workload | Latest recorded repeat (11:33 UTC) | Current serial repeat | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.1–16.2 ns/evaluation | 1.2–16.6 ns/evaluation | Upper reading +2.5% |
| Strategy position evaluation range | 1.1–1.8 ns/evaluation | 1.2–2.6 ns/evaluation | Upper reading +44.4% (+0.8 ns) |
| Signal serialization | 512.4 ns/operation | 507.4 ns/operation | -1.0% |
| Market depth assessment | 550.3 ns/operation | 587.0 ns/operation | +6.7% |
| Limit order request construction | 397.3 ns/operation | 395.5 ns/operation | -0.5% |
| Strategy parameter validation | 1.9 ns/operation | 2.0 ns/operation | +0.1 ns |

An earlier run in this repeat pair had larger maxima and did not persist on the serial repeat. The position maximum and market-depth helper are higher than the last saved repeat, but those functions were not changed; treat these single-run deltas as observed timing noise pending a quiet comparison. The benchmark does not measure the changed Scout feedback/entry-selection paths. AutoTrader tests passed (198); Learning tests passed (4). Both benchmark targets completed; no benchmark target was skipped or failed, and no services were started or stopped.

### Learning crate benchmark status (2026-11-04 11:44 UTC)

Cargo bench for the Learning crate completed successfully. No benchmark workload is defined, so Cargo ran its four test harness cases in bench mode with all four ignored; there are no performance figures to compare. The four Learning tests passed in test mode.

## Follow-up run (2026-11-04 12:38 UTC, exhaustive one-step Scout subsets)

### Auto-trader strategy evaluation

| Workload | Latest recorded repeat (11:43 UTC) | First current run | Serial repeat |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.2–16.6 ns/evaluation | 1.2–21.6 ns/evaluation | 1.2–16.6 ns/evaluation |
| Strategy position evaluation range | 1.2–2.6 ns/evaluation | 1.2–3.5 ns/evaluation | 1.1–2.0 ns/evaluation |
| Signal serialization | 507.4 ns/operation | 523.5 ns/operation | 507.9 ns/operation |
| Market depth assessment | 587.0 ns/operation | 561.4 ns/operation | 623.7 ns/operation |
| Limit order request construction | 395.5 ns/operation | 404.9 ns/operation | 398.8 ns/operation |
| Strategy parameter validation | 2.0 ns/operation | 1.9 ns/operation | 1.9 ns/operation |

The larger evaluator maxima from the first run did not repeat; the serial repeat is within the previous entry range and below its position maximum. Market-depth timing was 6.3% higher than the last recorded repeat, while the other helpers were close. These microbenchmarks do not exercise the new subset generation or batch scheduling path.

### Insights calculations

| Workload | Latest comparable run (11:08 UTC) | Current serial run | Change |
|---|---:|---:|---:|
| Live insight snapshot | 11,647.7 ns/calculation | 11,178.8 ns/calculation | -4.0% |
| Snapshot including explanations | 11,788.9 ns/calculation | 11,612.5 ns/calculation | -1.5% |
| Research feature extraction | 249.3 ns/calculation | 244.1 ns/calculation | -2.1% |
| Price return context | 17.0 ns/calculation | 17.1 ns/calculation | +0.6% |
| Market input diagnostics | 326.2 ns/calculation | 321.4 ns/calculation | -1.5% |
| Book snapshot | 43.8 ns/calculation | 43.7 ns/calculation | -0.2% |

AutoTrader tests passed (199), Insights tests passed (19; one external-provider test intentionally ignored), and Scouts tests passed (5). Cargo bench completed for Scouts, which has no benchmark workload; its five test harness cases were ignored in bench mode. Both optimized benchmark targets completed, with no targets skipped or failed. The measurements ran serially under WSL2; no services were started or stopped.

## Follow-up run (2026-11-04 13:08 UTC, feed drop and Insights warning fixes)

### Feed handling and Insights calculations

| Workload | Latest comparable baseline | Current run | Change |
|---|---:|---:|---:|
| OKX trade JSON decode, parse, SBE encode | 260.2 ns/message | 275.9 ns/message | +6.0% |
| 7-level snapshot decode/load/top-seven/encode | 1.274 us/message | 1.388 us/message | +9.0% |
| 50-level snapshot | 6.745 us/message | 6.991 us/message | +3.6% |
| 400-level snapshot | 48.632 us/message | 50.908 us/message | +4.7% |
| 5-level incremental update | 1.129 us/message | 1.241 us/message | +9.9% |
| Live insight snapshot | 11,647.7 ns/calculation | 11,772.5 ns/calculation | +1.1% |
| Snapshot including explanations | 11,788.9 ns/calculation | 12,602.8 ns/calculation | +6.9% |
| Research feature extraction | 249.3 ns/calculation | 244.1 ns/calculation | -2.1% |
| Price return context | 17.0 ns/calculation | 17.0 ns/calculation | No change |
| Market input diagnostics | 326.2 ns/calculation | 317.8 ns/calculation | -2.6% |
| Book snapshot | 43.8 ns/calculation | 43.5 ns/calculation | -0.7% |

The Insights comparison is from a serial rerun; the first simultaneous run was slower (12,109.0 / 12,357.5 ns for the two snapshot workloads), consistent with CPU contention. The +6.9% explanation-enabled result is a single-run increase and does not exercise the changed delivery limiter or log level; treat it as unconfirmed timing variation. Feed benchmark targets ran concurrently with Insights and the local stack; observed feed decode results were 3.6–9.9% slower than the first baseline, while broadcast fanout stayed at 0.1/0.2 us per event for 1/16 subscribers. The slow-consumer synthetic benchmark still used capacity 64 (not the changed production capacity 512), so it does not directly measure production lag behavior. All benchmark targets completed; test harness cases in bench mode were ignored as configured. No services were started or stopped.

## Follow-up run (2026-11-04, removal of alternative paper strategies)

### Auto-trader strategy evaluation

| Workload | Latest serial run (12:38 UTC) | Current serial run | Change |
|---|---:|---:|---:|
| Strategy entry evaluation range | 1.2–16.6 ns/evaluation | 1.1–17.7 ns/evaluation | Upper reading +6.6% |
| Strategy position evaluation range | 1.1–2.0 ns/evaluation | 1.1–2.5 ns/evaluation | Upper reading +25.0% |
| Signal serialization | 507.9 ns/operation | 532.8 ns/operation | +4.9% |
| Market depth assessment | 623.7 ns/operation | 680.6 ns/operation | +9.1% |
| Limit order request construction | 398.8 ns/operation | 424.3 ns/operation | +6.4% |
| Strategy parameter validation | 1.9 ns/operation | 2.3 ns/operation | +21.1% |

These single-run evaluator maxima and helper timings were not changed by deleting the separate alternative-paper subsystem. Treat the small deltas as unconfirmed timing variation; repeat a quiet comparison if performance attribution is needed.

### Feed handling and Insights calculations

| Workload | Latest comparable run (13:08 UTC) | Current serial run | Change |
|---|---:|---:|---:|
| OKX trade JSON decode, parse, SBE encode | 275.9 ns/message | 285.0 ns/message | +3.3% |
| 7-level snapshot decode/load/top-seven/encode | 1.388 us/message | 1.395 us/message | +0.5% |
| 50-level snapshot | 6.991 us/message | 7.278 us/message | +4.1% |
| 400-level snapshot | 50.908 us/message | 52.915 us/message | +3.9% |
| 5-level incremental update | 1.241 us/message | 1.194 us/message | -3.8% |
| Live insight snapshot | 11,772.5 ns/calculation | 11,880.5 ns/calculation | +0.9% |
| Snapshot including explanations | 12,602.8 ns/calculation | 12,175.3 ns/calculation | -3.4% |
| Research feature extraction | 244.1 ns/calculation | 245.1 ns/calculation | +0.4% |
| Price return context | 17.0 ns/calculation | 17.7 ns/calculation | +4.1% |
| Market input diagnostics | 317.8 ns/calculation | 337.9 ns/calculation | +6.3% |
| Book snapshot | 43.5 ns/calculation | 43.9 ns/calculation | +0.9% |

The first benchmarks were run concurrently and were discarded; the figures above are serial reruns, although the React test process remained active during measurement. Feed, Insights, and AutoTrader tests passed. All optimized benchmark targets completed; no targets were skipped or failed. No services were started or stopped.



## Auto-trader follow-up run (2026-11-04)

The auto-trader Cargo benchmark completed successfully. Exact host load and workload equivalence to the 2026-11-02 run are unknown, so treat these single-run differences as signals for follow-up rather than regressions attributable to the Scout scheduling change.

| Workload | 2026-11-02 baseline | 2026-11-04 run | Difference |
|---|---:|---:|---:|
| Strategy evaluation, entry/position range | 1.1–17.7 / 1.1–2.5 ns/evaluation | 2.2–24.6 / 1.5–5.7 ns/evaluation | Current maxima are higher |
| Signal serialization | 532.8 ns/operation | 572.9 ns/operation | +7.5% |
| Market depth assessment | 680.6 ns/operation | 837.2 ns/operation | +23.0% |
| Limit-order request construction | 424.3 ns/operation | 434.5 ns/operation | +2.4% |
| Strategy parameter validation | 2.3 ns/operation | 2.1 ns/operation | -8.7% |

The benchmark-mode test binaries reported 174 ignored tests; the separate cargo test run passed all 174 tests.

## Insights Scout record cap (2026-11-04)

`cargo test --manifest-path src/rust/insights/Cargo.toml` passed (19 tests). The Insights `calculations` benchmark was attempted but failed to compile because `SignalEngine` in `src/signals.rs` is missing the `completed_scout_shadow_order` field in an initializer. No benchmark workloads ran, so there is no performance comparison for this change.

## Follow-up run (2026-11-04)

### Position ledger snapshot batching change

| Workload | Previous recorded result | Current run | Change |
|---|---:|---:|---:|
| 100,000-fill MessagePack decode | 175.0 ms | 224.9 ms | +28.5% |
| 100,000-fill MessagePack encode | 64.5 ms | 77.6 ms | +20.3% |
| Concurrent atomic file workload, 100k rows, 4 readers and 1 writer | 1,869 ms | 2,531 ms | +35.4% |
| 100-fill MessagePack encode | 19.6 us | 19.5 us | -0.3% |

These generic serialization and filesystem benchmarks do not exercise the production snapshot_position_ledger copier, so they do not measure the batching change. Large-workload results were slower in this single run; treat them as a possible environmental/load difference and investigate with repeat runs before attributing them to this change. The change reduces copy-loop lock reacquisitions/yields from about 782 to 98 for 100,000 trades. The existing append-consistency test passed. WSL2 Linux x86_64, optimized Cargo bench profile; benchmarks were run sequentially. No services were started or stopped.

## Strategy learned early-exit floor (2026-11-04)

The auto-trader strategy evaluator benchmark completed successfully. Risk-adjusted trend portfolio measured 3.7 ns/entry evaluation and 2.3 ns/position evaluation. The recorded baseline gives a cross-strategy position-evaluation range of 1.1–3.1 ns; this strategy remains inside that range. The baseline does not record a per-strategy historical value, so this is not a direct same-workload regression comparison. WSL2 Linux x86_64, optimized Cargo bench profile. No services were started or stopped.

## Insights retention policy (2026-11-04)

The Insights calculations benchmark completed successfully. Compared with the 2026-11-02 Insights baseline, snapshot calculation workloads were slower in this single run: live insight snapshot 12,287.6 ns vs 1,831.7 ns (+570.9%); snapshot with explanations 13,511.5 ns vs 2,319.3 ns (+482.6%); research feature extraction 258.6 ns vs 143.4 ns (+80.3%); price return context 21.0 ns vs 17.4 ns (+20.7%); market input diagnostics 363.9 ns vs 236.5 ns (+53.7%); book snapshot 45.1 ns vs 44.4 ns (+1.6%). This retention change does not touch these calculation paths, and host load/workload equivalence are unknown, so treat the differences as environmental signals rather than regressions pending a repeat run. WSL2 Linux x86_64, optimized Cargo bench profile. The benchmark emitted an existing dead-code warning for the unused anchor_close_time_ms field. No services were started or stopped.

## Scout and learning service migration (2026-11-04)

The AutoTrader and Insights benchmark targets completed successfully. AutoTrader compared with the latest recorded 2026-11-04 run improved from 572.9 to 530.8 ns for signal serialization (-7.3%), 837.2 to 620.1 ns for depth assessment (-26.0%), 434.5 to 408.7 ns for order request construction (-5.9%), and 2.1 to 1.9 ns for parameter validation (-9.5%). Evaluator ranges were 1.2–21.5 / 1.1–3.2 ns for entry/position, versus 2.2–24.6 / 1.5–5.7 ns in that run.

Insights was run twice serially. The first result was slower and was discarded; the repeat measured live snapshot 12,577.5 ns, snapshot with explanations 12,776.5 ns, feature extraction 248.9 ns, return context 18.5 ns, input diagnostics 379.0 ns, and book snapshot 47.0 ns. Compared with the latest comparable 2026-11-04 entry (11,880.5 / 12,175.3 / 245.1 / 17.7 / 337.9 / 43.9 ns), changes are +5.9%, +4.9%, +1.6%, +4.5%, +12.2%, and +7.1%. Treat these single-run differences as signals for follow-up; the service migration does not alter these calculation loops. The bench-mode test harness reported all 178 AutoTrader and 22 Insights tests ignored, as expected; their normal test suites passed separately. No benchmark targets failed or were skipped. WSL2 Linux x86_64, optimized Cargo bench profile. No services were started or stopped.

## Insights maintenance service (2026-11-04)

The Insights calculations benchmark completed successfully. Compared with the latest recorded Insights run on 2026-11-04, all six workloads were faster: live snapshot 11,353.8 vs 12,577.5 ns (-9.7%); snapshot with explanations 11,805.5 vs 12,776.5 ns (-7.6%); research feature extraction 242.2 vs 248.9 ns (-2.7%); price return context 17.7 vs 18.5 ns (-4.3%); market input diagnostics 323.8 vs 379.0 ns (-14.6%); book snapshot 44.1 vs 47.0 ns (-6.2%). The dedicated maintenance process does not change these calculation paths; treat single-run movement as workload variation. The Insights test suite passed (22 tests). No benchmark targets failed or were skipped. WSL2 Linux x86_64, optimized Cargo bench profile. No services were started or stopped.

## AutoTrader scout promotion confidence (2026-11-04)

The AutoTrader library test target and `strategy_evaluation` benchmark were attempted but did not compile. Both stopped before running because `src/auto_trader/dated_carry.rs:201` initializes `ShadowPlanCandidate` without its required `audit_templates` field. No benchmark workloads ran, so there is no performance comparison for this change. No services were started or stopped.

## Insights maintenance Grafana telemetry (2026-11-04)

The Insights calculations benchmark completed successfully. Compared with the latest recorded Insights run on 2026-11-04, live snapshot measured 11,668.8 vs 11,353.8 ns (+2.8%); snapshot with explanations 11,880.9 vs 11,805.5 ns (+0.6%); research feature extraction 269.7 vs 242.2 ns (+11.4%); price return context 26.3 vs 17.7 ns (+48.6%); market input diagnostics 328.7 vs 323.8 ns (+1.5%); and book snapshot 44.2 vs 44.1 ns (+0.2%). This change adds maintenance status reporting outside those calculation paths; host load and other workspace changes may affect this single run, so the differences are not attributed to the monitoring change. Insights tests passed (22 tests). No benchmark targets failed or were skipped. WSL2 Linux x86_64, optimized Cargo bench profile. No services were started or stopped.

## Completed Scout and Learning migration (2026-11-04)

The final serial AutoTrader benchmark completed. Compared with the latest comparable completed run in this log (530.8 / 620.1 / 408.7 / 1.9 ns for serialization, market depth, order request construction, and parameter validation), the final run measured 506.2 / 562.9 / 397.3 / 1.9 ns (-4.6%, -9.2%, -2.8%, and no change). Entry/position evaluation ranged 1.1–16.6 / 1.1–2.0 ns, versus 1.2–21.5 / 1.1–3.2 ns. The range maxima are not tied to a single strategy, so treat these as single-run comparisons.

The final serial Insights calculations benchmark measured 11,263.9 ns live snapshot, 11,635.1 ns with explanations, 246.1 ns feature extraction, 17.5 ns return context, 320.6 ns input diagnostics, and 56.3 ns book snapshot. Compared with the latest completed 2026-11-04 Insights run (11,668.8 / 11,880.9 / 269.7 / 26.3 / 328.7 / 44.2 ns), differences are -3.5%, -2.1%, -8.8%, -33.5%, -2.5%, and +27.4%. Book snapshot was slower in this single run; workload/environment equivalence is unknown, and the Scout service-boundary changes do not alter this calculation path, so repeat under a quiet host before attributing it to the migration.

AutoTrader and Insights optimized benchmark targets completed. Their benchmark-mode test harness cases were ignored by design and ran normally in separate `cargo test` runs. Scouts, Learning, and Strategy Core have no benchmark harnesses. No benchmark target failed or was skipped. WSL2 Linux x86_64, optimized Cargo bench profile; targets were run serially. No services were started or stopped.

## Scout outcomes API ownership (2026-11-04)

The Insights calculations benchmark completed. Compared with the latest recorded Insights run above (11,263.9 / 11,635.1 / 246.1 / 17.5 / 320.6 / 56.3 ns), this run measured 11,174.0 / 11,534.7 / 247.4 / 17.2 / 323.7 / 43.6 ns for live snapshot, snapshot with explanations, feature extraction, return context, market input diagnostics, and book snapshot (-0.8%, -0.9%, +0.5%, -1.7%, +1.0%, and -22.6%). The Scout report route change does not alter these calculation paths; treat these single-run differences as environment variation. Insights tests passed (13 library and 27 binary tests). Scouts tests passed (22 tests). Scouts has no benchmark harness. No benchmark targets failed or were skipped. No services were started or stopped.

## Learning SQLite normalization (2026-11-05)

Learning tests passed (24 tests). Initial `learning_service` measurements were 1.82 ms to load 1,000 variants, 200 active profiles, and 5,000 event IDs; 0.09 ms to generate a decision snapshot from 1,000 variants and 200 profiles; and 18.02 ms to persist that state. After reusing prepared SQLite statements in the persistence path, three runs measured 14.09, 12.79, and 13.18 ms for persistence (-21.8%, -29.0%, and -26.9% against the initial run); load measured 1.82, 1.83, and 1.87 ms and snapshots 0.09, 0.08, and 0.08 ms. The save improvement held across the three samples, though these are single-process microbenchmark runs. No directly comparable prior workload exists. No benchmark targets failed or were skipped, and no services were started or stopped.

## Learning incremental persistence (2026-11-05)

Learning tests passed (24 tests), including a new regression check that removed deduplication IDs are deleted from SQLite. Persistence now uses conditional upserts and removes only stale records. Three runs measured 6.93, 6.15, and 6.09 ms for the same 1,000-variant, 5,000-event workload (about 46–57% below the prepared-statement-only results and 62–66% below the original full-rewrite run). Load measured 1.85–1.96 ms and decision snapshots 0.09 ms. No benchmark target failed or was skipped, and no services were started or stopped.

## Reusable strategy and crate organization (2026-11-04)

The FeedHandlers benchmark targets completed serially. Compared with the latest recorded feed-processing run above, trade JSON decode/parse/SBE encode measured 257.0 vs 285.0 ns/message (-9.8%); 7-level snapshots 1,251.6 vs 1,395 ns (-10.3%); 50-level snapshots 6,807.5 vs 7,278 ns (-6.5%); 400-level snapshots 48,971.3 vs 52,915 ns (-7.4%); and 5-level incremental updates 1,157.9 vs 1,194 ns (-3.0%). The order-book, SBE encoding, and broadcast subscriber targets also completed; their detailed current readings are in the bench output and do not have matching entries in this baseline.

The Insights calculations benchmark measured 11,207.4 ns live snapshot, 11,563.9 ns with explanations, 243.5 ns feature extraction, 17.2 ns return context, 331.9 ns input diagnostics, and 43.6 ns book snapshot. Compared with the latest recorded Insights run (11,174.0 / 11,534.7 / 247.4 / 17.2 / 323.7 / 43.6 ns), changes were +0.3%, +0.3%, -1.6%, no change, +2.5%, and no change. Treat these small single-run differences as timing variation while broader changes are evaluated.

Position-management benchmarks completed except `service_http`, which explicitly skipped because `BENCH_SERVICE_URL` was unset; no service was started. The current 100,000-fill MessagePack decode/encode measured 178.7/62.9 ms, versus the latest recorded 224.9/77.6 ms; the concurrent filesystem workload measured 2,053 ms versus 2,531 ms; and 100-fill MessagePack encode measured 18.5 us versus 19.5 us. The main runtime suites passed separately. WSL2 Linux x86_64, optimized Cargo bench profile; targets were run serially. No services were started or stopped.

## Final reorganization and coverage repeat (2026-11-04)

FeedHandlers benchmarks were rerun after the instrument catalog parser tests and helpers. Trade decode/parse/SBE encode measured 254.5 ns/message versus 285.0 ns/message in the latest comparable recorded run (-10.7%); 7-level snapshots measured 1,240.0 ns (-11.1%), 50-level snapshots 6,595.8 ns (-9.4%), 400-level snapshots 48,599.0 ns (-8.2%), and 5-level incremental updates 1,149.0 ns (-3.8%). All feed benchmark targets completed.

The Insights calculations repeat measured 11,308.8 / 11,654.3 / 244.1 / 17.5 / 327.1 / 44.6 ns for live snapshot, snapshot with explanations, feature extraction, return context, input diagnostics, and book snapshot. Relative to the immediately prior readings (11,207.4 / 11,563.9 / 243.5 / 17.2 / 331.9 / 43.6 ns), differences were +0.9%, +0.8%, +0.2%, +1.7%, -1.4%, and +2.3%; treat as normal single-run variation.

Position-management repeat measured 176.1 ms MessagePack decode and 61.4 ms encode for 100,000 fills, 18.4 us encode for 100 fills, 3.85 ms position aggregation, 49.5 ns indexed lookup, and 51.8 ns count scan. `service_http` again skipped because `BENCH_SERVICE_URL` was unset; no services were started. Against the latest comparable ledger readings (224.9 / 77.6 ms, 19.5 us), decode/encode and 100-fill encode were faster. WSL2 Linux x86_64, optimized Cargo bench profile; targets were run serially. No benchmark target failed.

AutoTrader's strategy-evaluation benchmark was repeated after replacing the per-call owned adapter with a borrowed view. Entry evaluations measured 1.2–17.4 ns and position evaluations 0.6–3.1 ns, versus the latest comparable 1.1–16.6 / 1.1–2.0 ns range. The earlier owned adapter measured hundreds of nanoseconds and was discarded after its deep-copy cost was removed. Signal serialization, market-depth assessment, order-request construction, and parameter validation measured 511.3 / 567.4 / 423.8 / 2.0 ns; versus the latest recorded 506.2 / 562.9 / 397.3 / 1.9 ns, differences are +1.0%, +0.8%, +6.7%, and +5.3%. These are single-run measurements; no benchmark target failed. WSL2 Linux x86_64, optimized Cargo bench profile. No services were started or stopped.

## Strategy-core consolidation (2026-11-05)

AutoTrader's same-harness strategy benchmark after moving core contracts, daily models, and parameter policy into the Strategies crate measured entry evaluation at 1.3–17.5 ns and position evaluation at 0.5–3.1 ns, within the latest recorded 1.2–17.4 / 0.6–3.1 ns ranges. Signal serialization, market-depth assessment, order-request construction, and parameter validation measured 500.5 / 571.8 / 432.4 / 2.0 ns, versus the latest 506.2 / 562.9 / 397.3 / 1.9 ns (-1.1%, +1.6%, +8.8%, +5.3%). These are single-run readings; the strategy consolidation did not change the helper workloads. The benchmark completed without failures. WSL2 Linux x86_64, optimized Cargo bench profile. No services were started or stopped.

After moving the evaluator benchmark helper into `strategies/src/benchmarks.rs`, a repeat measured entry evaluations at 2.4–17.9 ns and positions at 0.8–3.5 ns. Against the prior 1.3–17.5 / 0.5–3.1 ns run, the maxima shifted slightly; these remain within the earlier historical ranges. Serialization/depth/order/parameter helpers measured 521.2 / 597.3 / 445.8 / 2.0 ns, changes of +4.1%, +4.5%, +3.1%, and no change from the immediately prior readings. Treat this single run as timing variation; no benchmark target failed. No services were started or stopped.

## Learning-owned paired promotion and walk-forward thresholds (2026-11-05)

The AutoTrader strategy-evaluation benchmark completed. Compared with the latest recorded same-harness run (entry 2.4–17.9 ns; position 0.8–3.5 ns; serialization/depth/order/parameter helpers 521.2 / 597.3 / 445.8 / 2.0 ns), this run measured entry 1.3–16.4 ns, position 0.5–2.0 ns, and helpers 505.7 / 568.1 / 426.7 / 2.0 ns. The helper changes are -3.0%, -4.9%, -4.3%, and no change. Range maxima are not tied to a single strategy, so treat them as workload-dependent readings. The benchmark does not measure Learning service promotion or threshold fitting. It completed without failures. No services were started or stopped.

## Learning feedback observation wiring repeat (2026-11-05)

AutoTrader's strategy-evaluation benchmark completed. Compared with the immediately previous reading (entry 1.3–16.4 ns; position 0.5–2.0 ns; serialization/depth/order/parameter helpers 505.7 / 568.1 / 426.7 / 2.0 ns), this run measured entry 1.3–16.5 ns, position 0.6–3.0 ns, and helpers 510.3 / 573.4 / 426.7 / 1.9 ns (+0.9%, +0.9%, no change, and -5.0%). These single-run variations do not exercise the new feedback/threshold sample path; treat them as normal workload variation. No benchmark target failed. No services were started or stopped.

## AutoTrader SQLite state persistence (2026-11-05)

AutoTrader's strategy-evaluation benchmark completed after moving saved state persistence to SQLite. Entry evaluations measured 1.2–16.5 ns and position evaluations 0.6–1.9 ns. Signal serialization, market-depth assessment, order-request construction, and parameter validation measured 503.0 / 618.3 / 443.2 / 2.0 ns versus 510.3 / 573.4 / 426.7 / 1.9 ns in the previous same-harness run (-1.4%, +7.8%, +3.9%, +5.3%). These workloads do not measure SQLite read/write latency; no comparison is available for persistence I/O. No benchmark target failed. WSL2 Linux x86_64, optimized Cargo bench profile. No services were started or stopped.

After removing the runtime legacy-file fallback, a repeat measured entry evaluations at 1.3–17.3 ns and position evaluations at 0.6–4.3 ns. Signal serialization, market-depth assessment, order-request construction, and parameter validation measured 549.0 / 615.1 / 428.3 / 2.0 ns versus 503.0 / 618.3 / 443.2 / 2.0 ns in the preceding run (+9.1%, -0.5%, -3.4%, no change). Several position-evaluation readings were elevated (up to 4.3 ns versus 1.9 ns), while the benchmark still does not measure SQLite persistence. The same-harness run completed; no services were started or stopped.

## Scout terminology consolidation (2026-11-05)

AutoTrader's strategy-evaluation benchmark completed after renaming the Scout flow, API contracts, and audit fields. Compared with the latest SQLite persistence run (entry 1.2–16.5 ns, position 0.6–1.9 ns; helpers 503.0 / 618.3 / 443.2 / 2.0 ns), this run measured entry 1.2–16.4 ns and position 0.6–2.6 ns; helpers 502.5 / 568.4 / 430.0 / 1.9 ns (-0.1%, -8.1%, -3.0%, -5.0%). These workloads do not measure Scout API behavior. The Insights calculations benchmark measured 11,948.9 / 12,022.7 / 247.9 / 17.5 / 335.3 / 44.2 ns for live snapshot, snapshot with explanations, feature extraction, return context, input diagnostics, and book snapshot. Compared with the latest comparable recorded readings (11,308.8 / 11,654.3 / 244.1 / 17.5 / 327.1 / 44.6 ns), changes are +5.7%, +3.2%, +1.6%, no change, +2.5%, and -0.9%. These are single-run variations; the rename does not change these calculation paths. AutoTrader tests passed (136), Insights tests passed (18 library and 29 binary), Scouts tests passed (27), and React tests passed (84). WSL2 Linux x86_64, optimized Cargo bench profile. No services were started or stopped.

## Scout outcome proxy removal repeat (2026-11-05)

AutoTrader's strategy-evaluation benchmark completed after removing the obsolete AutoTrader-to-Scouts forwarding proxy. Compared with the immediately preceding Scout terminology run (entry 1.2–16.4 ns, position 0.6–2.6 ns; serialization/depth/order/parameter helpers 502.5 / 568.4 / 430.0 / 1.9 ns), this run measured entry 1.2–16.7 ns, position 0.6–4.2 ns, and helpers 514.0 / 586.6 / 411.9 / 2.0 ns (+2.3%, +3.2%, -4.2%, +5.3%). The position maximum widened in this single run; timing noise and per-strategy variation cannot be separated from this aggregate range. The proxy removal does not alter the evaluator or helper workloads. AutoTrader tests passed (136). No benchmark targets failed. WSL2 Linux x86_64, optimized Cargo bench profile. No services were started or stopped.

## Live Signals service split (2026-11-05)

The Insights calculations benchmark completed twice with the same optimized Cargo bench target. The second run measured 11,250.7 ns live snapshot, 11,536.4 ns with explanations, 247.1 ns research feature extraction, 17.6 ns return context, 321.9 ns market input diagnostics, and 44.2 ns book snapshot. Compared with the latest same-workload readings above (11,948.9 / 12,022.7 / 247.9 / 17.5 / 335.3 / 44.2 ns), changes were -5.8%, -4.0%, -0.3%, +0.6%, -4.0%, and no change. This split leaves the measured engine calculations unchanged; these results show no material regression. WSL2 Linux x86_64, 32 logical CPUs, optimized Cargo bench profile; load average was about 4.9 during the run. No benchmark target failed. Services were not started during benchmarking.

The final Insights calculations repeat measured 11,091.4 ns live snapshot, 12,049.4 ns with explanations, 241.4 ns research feature extraction, 17.7 ns return context, 318.9 ns input diagnostics, and 44.5 ns book snapshot. Against the preceding same-harness run (11,250.7 / 11,536.4 / 247.1 / 17.6 / 321.9 / 44.2 ns), differences were -1.4%, +4.4%, -2.3%, +0.6%, -0.9%, and +0.7%. The explanation variant moved +4.4% in this single run; the other paths were within 2.3%. No benchmark target failed. WSL2 Linux x86_64, optimized Cargo bench profile. Services were not started during benchmarking.

## Dated-carry normalized SQLite storage (2026-11-05)

The AutoTrader `strategy_evaluation` benchmark completed with the optimized Cargo bench profile. Entry evaluation ranged from 1.3–16.8 ns (previous run: 2.2–24.6 ns), and position evaluation ranged from 0.6–2.6 ns (previous: 1.5–5.7 ns). Signal serialization measured 509.3 ns (previous 572.9 ns, -11.1%), market-depth assessment 574.3 ns (837.2 ns, -31.4%), limit-order request construction 406.4 ns (434.5 ns, -6.5%), and parameter validation 2.3 ns (2.1 ns, +9.5%). These single-run changes are unrelated to the persistence schema and remain within noisy microbenchmark variation; the strategy benchmark does not measure SQLite persistence. WSL2 Linux x86_64, optimized Cargo bench profile. No benchmark targets failed. Services were not started or stopped.

## Follow-up run (2026-11-05 09:24 UTC)

### Position management float round-trip verification and benchmarks

Enabled `serde_json`'s `float_roundtrip` feature after comparison exposed tiny `f64` differences when SQLite JSON payloads were loaded. Regenerated SQLite from the original MessagePack file and verified exact equality across trades, topups, resting orders, barrier orders, scheduled runs, and account metadata. Temporary verification code was removed after the check.

| Workload | 2026-11-02 baseline | Current run | Change |
|---|---:|---:|---:|
| 100-fill MessagePack decode | 121.7 us | 119.8 us | -1.6% |
| 100-fill JSON decode | 133.7 us | 136.5 us | +2.1% |
| 100-fill MessagePack encode | 19.6 us | 19.6 us | Within run variation |
| 100-fill JSON encode | 32.0 us | 31.6 us | -1.4% |
| 100,000-fill MessagePack decode | 175.0 ms | 192.4 ms | +9.9% |
| 100,000-fill JSON decode | 189.0 ms | 181.3 ms | -4.1% |
| 100,000-fill MessagePack encode | 64.5 ms | 59.7 ms | -7.4% |
| 100,000-fill JSON encode | 102.3 ms | 92.3 ms | -9.8% |
| Concurrent atomic file workload | 1,869 ms | 1,974 ms | +5.6% |
| Position aggregation proxy | 4.385 ms | 3.772 ms | -14.0% |
| Indexed lookup / count scan | 58.4 / 53.0 ns | 48.1 / 49.5 ns | -17.6% / -6.6% |

The 100k MessagePack decode and concurrent file workload were slower in this single run; the file benchmark remains a JSON rename workload and does not represent SQLite persistence. No SQLite write benchmark exists yet, so these results do not measure production database save latency. All available crate benchmark targets completed; the optional HTTP benchmark was skipped because `BENCH_SERVICE_URL` was unset. Environment and workload match the recorded WSL2 baseline; single-run variance applies.

## Live Signals function-level tests (2026-11-05)

`cargo test --manifest-path src/rust/live_signals/Cargo.toml` passed all 23 tests covering market-event decoding, news filtering and weighting, context refresh, signal state/enqueue, sharded event processing, delivery success/failure and acknowledgement, and API handlers. The crate has no benchmark targets. `cargo bench --manifest-path src/rust/live_signals/Cargo.toml` completed successfully; Cargo ran the 23 test harness cases in bench mode with all ignored, so no performance workloads were available. No comparable Live Signals benchmark baseline exists.

## Position-management function coverage tests (2026-11-05)

`cargo bench` completed all four position-management benchmark targets. The optional `service_http` workload was skipped because `BENCH_SERVICE_URL` was unset; no services were started. The test and coverage additions do not change the measured hot paths.

| Workload | Latest comparable run (2026-11-05) | Current | Change |
|---|---:|---:|---:|
| 100-fill MessagePack / JSON decode | 119.8 / 136.5 us | 122.3 / 140.4 us | +2.1% / +2.9% |
| 100-fill MessagePack / JSON encode | 19.6 / 31.6 us | 18.5 / 32.4 us | -5.8% / +2.7% |
| 100,000-fill MessagePack / JSON decode | 192.4 / 181.3 ms | 171.9 / 194.2 ms | -10.7% / +7.1% |
| 100,000-fill MessagePack / JSON encode | 59.7 / 92.3 ms | 74.2 / 106.8 ms | +24.3% / +15.8% |
| Concurrent atomic file workload | 1,974 ms | 1,958 ms | -0.8% |
| Position aggregation proxy | 3.772 ms | 3.958 ms | +4.9% |
| Indexed lookup / count scan | 48.1 / 49.5 ns | 49.3 / 52.7 ns | +2.5% / +6.5% |

The 100k-fill JSON and MessagePack encode timings increased versus the immediately preceding single run, while 100k decode and the concurrent file workload improved. These are standalone serialization and filesystem workloads; no serialization or filesystem implementation changed in this task, so the variation does not indicate a measured effect from the coverage tests. Compared with the original persisted position-management baseline, the 100k MessagePack decode remained slightly faster (171.9 vs 175.0 ms) and the aggregation proxy improved (3.958 vs 4.385 ms); other observed differences remain single-run noise. Environment: WSL2 Linux x86_64, optimized Cargo bench profile, targets run serially. No benchmark target failed.

## Scout configuration ownership (2026-11-05)

AutoTrader and Insights benchmark targets completed serially after the settings ownership changes. AutoTrader entry evaluation ranged 1.2–16.1 ns and position evaluation 0.6–1.9 ns, versus the latest recorded 1.3–16.8 / 0.6–2.6 ns. Signal serialization measured 521.0 ns (+2.3% vs 509.3 ns), market-depth assessment 542.0 ns (-5.6% vs 574.3 ns), limit request construction 386.5 ns (-4.9% vs 406.4 ns), and parameter validation 1.8 ns (-21.7% vs 2.3 ns). No material degradation was observed.

Insights measured 10,662.7 ns live snapshot, 11,024.3 ns with explanations, 240.9 ns feature extraction, 17.4 ns return context, 345.6 ns input diagnostics, and 44.2 ns book snapshot. Compared with the latest recorded 11,091.4 / 12,049.4 / 241.4 / 17.7 / 318.9 / 44.5 ns, changes were -3.9%, -8.5%, -0.2%, -1.7%, +8.4%, and -0.7%. Input diagnostics was the only higher measurement; its code path was not changed by this task, and this single run is not enough to attribute a regression. Scouts has no performance benchmark target; `cargo bench` ran its 28 tests in bench mode with all ignored. WSL2 Linux x86_64, optimized Cargo bench profile; no services were started or stopped.

## Scout execution ownership split (2026-11-05)

AutoTrader tests passed (121), Scouts tests passed (28), Insights tests passed (29), and React tests passed (84). The AutoTrader strategy benchmark measured entry evaluation at 1.1–16.0 ns and position evaluation at 0.5–2.7 ns, compared with the latest 1.3–16.8 / 0.6–2.6 ns range. Signal serialization was 513.6 ns (+0.8% vs 509.3), market-depth assessment 544.6 ns (-5.2% vs 574.3), limit request construction 411.9 ns (+1.4% vs 406.4), and parameter validation 1.8 ns (-21.7% vs 2.3). The highest position timing increased by 0.1 ns in this single run; the benchmark does not exercise Scout processing.

Insights measured 10,771.3 / 11,047.0 / 234.3 / 16.4 / 325.1 / 42.6 ns for live snapshot, explained snapshot, feature extraction, return context, input diagnostics, and book snapshot. Compared with the latest prior run (11,091.4 / 12,049.4 / 241.4 / 17.7 / 318.9 / 44.5 ns), changes were -2.9%, -8.3%, -2.9%, -7.3%, +1.9%, and -4.3%. Scouts has no dedicated benchmark target; `cargo bench` completed with its 28 unit tests ignored. WSL2 Linux x86_64, optimized Cargo bench profile; no services were started or stopped.

## Independent Scout run pipeline (2026-11-05)

AutoTrader benchmark entry timings were 1.1–16.1 ns and position timings 0.5–2.0 ns, compared with the preceding run's 1.1–16.0 / 0.5–2.7 ns. Signal serialization measured 524.5 ns (+2.1% vs 513.6), market depth 549.1 ns (+0.8% vs 544.6), limit request construction 415.0 ns (+0.8% vs 411.9), and parameter validation 1.8 ns (unchanged). These small helper differences are within single-run variation; no measured regression in the decision workload.

Insights measured 10,687.7 / 11,169.7 / 238.0 / 16.8 / 315.5 / 43.3 ns for live snapshot, explained snapshot, feature extraction, return context, input diagnostics, and book snapshot. Against the preceding run (10,771.3 / 11,047.0 / 234.3 / 16.4 / 325.1 / 42.6 ns), changes were -0.8%, +1.1%, +1.6%, +2.4%, -3.0%, and +1.6%, within single-run variation. Scouts has no benchmark target; its 28 unit-test harness cases ran ignored in bench mode. WSL2 Linux x86_64, optimized Cargo bench profile; targets ran serially, with no services started or stopped.

## Scout run ownership final repeat (2026-11-05)

The same AutoTrader and Insights benchmark targets completed serially. AutoTrader entry timings were 1.1–16.1 ns; positions were 0.5–2.4 ns. Against the preceding run (1.1–16.1 / 0.5–2.0 ns), the high position timing increased 0.4 ns in the RSI2 evaluator; other entries and positions were within the recorded range. Signal serialization measured 521.3 ns (-0.6% vs 524.5), market depth 546.5 ns (-0.5% vs 549.1), limit request construction 416.8 ns (+0.4% vs 415.0), and parameter validation 1.8 ns (unchanged). These decision microbenchmarks do not exercise the moved Scout processing.

Insights measured 10,687.7 / 11,169.7 / 238.0 / 16.8 / 315.5 / 43.3 ns for live snapshot, explained snapshot, feature extraction, return context, input diagnostics, and book snapshot, matching the previous single run within displayed precision. Scouts has no dedicated benchmark target; its 28 test harness cases ran ignored in bench mode. WSL2 Linux x86_64, optimized Cargo bench profile, serial runs; no services were started or stopped.

## Scouts process decoupling repeat (2026-11-05)

AutoTrader entry/position evaluation ranged 1.1–16.7 / 0.5–1.9 ns versus 1.1–16.1 / 0.5–2.4 ns in the prior run. The highest entry timing increased 0.6 ns (+3.7%) for live absorption pressure; other evaluator timings were within range. Signal serialization measured 522.7 ns (+0.3%), market-depth assessment 544.1 ns (-0.4%), limit request construction 416.4 ns (-0.1%), and parameter validation 1.8 ns (unchanged). These single-run helper differences are within timing variation.

Insights measured 10,730.9 / 11,238.6 / 236.1 / 16.6 / 313.2 / 42.4 ns for live snapshot, explained snapshot, feature extraction, return context, input diagnostics, and book snapshot. Compared with the preceding run (10,687.7 / 11,169.7 / 238.0 / 16.8 / 315.5 / 43.3 ns), changes were +0.4%, +0.6%, -0.8%, -1.2%, -0.7%, and -2.1%. Scouts has no benchmark target; its 28 test harness cases ran ignored in bench mode. WSL2 Linux x86_64, optimized Cargo bench profile, serial execution; no services were started or stopped.

## Scout state/report ownership in Scouts (2026-11-05)

Scouts has no dedicated benchmark target; `cargo bench` completed with all 31 unit-test cases ignored. The Insights calculations benchmark was repeated twice. The final run measured 10,600.5 / 10,900.7 / 237.2 / 16.4 / 315.8 / 42.1 ns for live snapshot, explained snapshot, feature extraction, return context, input diagnostics, and book snapshot. Compared with the latest same-workload run (10,730.9 / 11,238.6 / 236.1 / 16.6 / 313.2 / 42.4 ns), changes were -1.2%, -3.0%, +0.5%, -1.2%, +0.8%, and -0.7%; no meaningful regression was observed. Tests passed: Scouts 31, Insights 18 library and 25 binary. WSL2 Linux x86_64, 32 logical CPUs, optimized Cargo bench profile, load average 1.86; services were stopped during benchmarks.

## Live Signals to Scouts input delivery (2026-11-05)

Live Signals has no dedicated benchmark target; `cargo bench` completed with all 24 unit-test cases ignored. The new Scout delivery runs alongside AutoTrader delivery, with a 300 ms Scout request timeout. Tests passed (24). No comparable Live Signals benchmark workload is recorded, so there is no timing regression comparison. WSL2 Linux x86_64, optimized Cargo bench profile; services were not running during the benchmark.

## Live Scout candidate attribution fix (2026-11-05)

Scouts tests passed (32). Scouts has no dedicated benchmark target; `cargo bench` completed with all 32 unit-test cases ignored. The fix adds the strategy identity to generated live candidates so plan registration can accept them; it does not change benchmarked strategy or market-data calculations. No comparable performance workload applies. WSL2 Linux x86_64, optimized Cargo bench profile.

## Walk-forward history coverage (2026-11-05)

Insights tests passed (44 across library and binary). The calculations benchmark completed. Against the previous recorded run, live snapshot measured 11,200.0 ns (+5.7%), explained snapshot 11,602.5 ns (+6.4%), research feature extraction 242.2 ns (+2.1%), price return 17.2 ns (+4.9%), market input diagnostics 342.8 ns (+8.5%), and book snapshot 44.6 ns (+5.9%). These single-run changes are within expected variation; the benchmark does not exercise walk-forward database scanning, which is the changed workload. No walk-forward benchmark fixture exists. WSL2 Linux x86_64, optimized Cargo bench profile.

## Scout settings persistence and limits (2026-11-05)

Scouts tests passed (33). Scouts has no dedicated benchmark target; `cargo bench` completed with all 33 test harness cases ignored. This change expands the configured Scout position limit and updates settings validation; it does not alter a benchmarked calculation. No comparable performance workload applies. WSL2 Linux x86_64, optimized Cargo bench profile.

## Live Scout profile fallback (2026-11-06)

Scouts tests passed (34), including coverage for selecting matching AutoTrader profiles when Learning has no live profiles. Scouts has no dedicated benchmark target; `cargo bench` completed with all 34 test harness cases ignored. The change adds profile retrieval and filtering in the live-input path, which is not covered by a performance workload, so there is no comparable timing result. WSL2 Linux x86_64, optimized Cargo bench profile. The Scouts service was not restarted.

## Remove the composite Live Insights strategy (2026-11-06)

AutoTrader strategy benchmarks completed after removing the composite registry entry. Evaluator entry/position timings ranged 1.9–16.6 / 0.8–2.6 ns, versus the latest recorded range of 1.1–16.7 / 0.5–1.9 ns. The catalog now contains only the five separate live strategies, so aggregate ranges cover a different strategy set and are not directly comparable. Helper workloads measured 551.3 ns signal serialization (+5.5% vs 522.7 ns), 562.7 ns market depth assessment (+3.4% vs 544.1 ns), 420.8 ns limit request construction (+1.1% vs 416.4 ns), and 1.8 ns strategy parameter validation (unchanged). These small single-run helper changes are within benchmark variation; they do not exercise removed composite strategy logic. WSL2 Linux x86_64, optimized Cargo bench profile, serial execution; no services were started or stopped.
