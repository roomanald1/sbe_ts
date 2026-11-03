# Rust benchmark baseline

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
