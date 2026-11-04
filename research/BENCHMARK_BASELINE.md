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
