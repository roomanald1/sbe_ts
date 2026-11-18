use std::hint::black_box;

fn main() {
    const ITERATIONS: usize = 1_000_000;
    for (strategy, entry_ns, position_ns) in
        auto_trader::auto_trader::benchmark_strategy_evaluation(ITERATIONS)
    {
        black_box((&strategy, entry_ns, position_ns));
        println!(
            "{strategy}: entry {entry_ns:.1} ns/evaluation, position {position_ns:.1} ns/evaluation"
        );
    }
    for (path, ns) in auto_trader::auto_trader::benchmark_autotrader_paths(ITERATIONS) {
        black_box((&path, ns));
        println!("{path}: {ns:.1} ns/operation");
    }
    const SNAPSHOT_ITERATIONS: usize = 250;
    for (path, us) in
        auto_trader::auto_trader::benchmark_signal_settings_snapshot_path(SNAPSHOT_ITERATIONS)
    {
        black_box((&path, us));
        println!("{path}: {us:.3} us/operation ({SNAPSHOT_ITERATIONS} iterations)");
    }
    const DECISION_PARAMETERS_ITERATIONS: usize = 100;
    for retained_variants in [0, 20_000] {
        let (snapshot_us, profiles_us, json_us, response_bytes, returned_variants) =
            auto_trader::auto_trader::benchmark_decision_parameters_path(
                DECISION_PARAMETERS_ITERATIONS,
                retained_variants,
            );
        println!(
            "decision parameters ({retained_variants} retained Scout variants): snapshot {snapshot_us:.3} us, profile build {profiles_us:.3} us, JSON {json_us:.3} us ({response_bytes} response bytes, {returned_variants} returned variants; {DECISION_PARAMETERS_ITERATIONS} iterations, 20,000 walk-forward evidence entries)"
        );
    }
}
