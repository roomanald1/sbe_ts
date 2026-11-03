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
