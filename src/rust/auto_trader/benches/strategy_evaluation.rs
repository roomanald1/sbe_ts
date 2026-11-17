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
}
