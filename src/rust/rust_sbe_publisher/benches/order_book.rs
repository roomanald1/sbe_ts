#[path = "../src/okx/book.rs"]
mod book;

use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Instant,
};

use book::{Book, Price, Snapshot, Update};

const ITERATIONS: usize = 2_000_000;
const BEST_LEVEL_ITERATIONS: usize = 20_000;
const SNAPSHOT_ITERATIONS: usize = 5_000;

struct CountingAllocator;

static COUNT_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static ALLOCATION_COUNT: AtomicUsize = AtomicUsize::new(0);

// SAFETY: Allocation and deallocation are forwarded unchanged to the system allocator.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: The caller upholds GlobalAlloc's layout requirements.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: The pointer and layout are forwarded unchanged to the system allocator.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: The caller upholds GlobalAlloc's pointer and layout requirements.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;

fn report(name: &str, elapsed_ns: u128, operations: usize) {
    let seconds = elapsed_ns as f64 / 1_000_000_000.0;
    println!(
        "{name}: {:.2} ns/op, {:.2} M ops/s ({operations} operations)",
        elapsed_ns as f64 / operations as f64,
        operations as f64 / seconds / 1_000_000.0,
    );
}

fn new_book() -> Book {
    Book::new(
        Price::from_decimal("0.01").unwrap(),
        Price::from_decimal("100").unwrap(),
        16,
    )
    .unwrap()
}

fn main() {
    let mut book = new_book();
    book.update_from_snapshot(&Snapshot {
        asks: vec![
            vec!["50.01", "1.0"],
            vec!["50.02", "1.0"],
            vec!["50.03", "1.0"],
            vec!["50.04", "1.0"],
        ],
        bids: vec![
            vec!["49.99", "1.0"],
            vec!["49.98", "1.0"],
            vec!["49.97", "1.0"],
            vec!["49.96", "1.0"],
        ],
        ts: "0",
        seq: 1,
        prev_seq: 0,
    })
    .unwrap();

    let read_iterations = ITERATIONS * 4;
    ALLOCATION_COUNT.store(0, Ordering::SeqCst);
    COUNT_ALLOCATIONS.store(true, Ordering::SeqCst);
    let started = Instant::now();
    for index in 0..read_iterations {
        let tick = 4_996 + (index & 3);
        black_box(book.bid_size_at_tick(black_box(tick)));
        black_box(book.ask_size_at_tick(black_box(tick + 5)));
    }
    let elapsed_ns = started.elapsed().as_nanos();
    COUNT_ALLOCATIONS.store(false, Ordering::SeqCst);
    let read_allocations = ALLOCATION_COUNT.load(Ordering::SeqCst);
    report("two direct tick reads", elapsed_ns, read_iterations);
    println!("direct-read heap allocations: {read_allocations}");
    assert_eq!(read_allocations, 0, "direct tick reads allocated");

    ALLOCATION_COUNT.store(0, Ordering::SeqCst);
    COUNT_ALLOCATIONS.store(true, Ordering::SeqCst);
    let started = Instant::now();
    for _ in 0..ITERATIONS {
        black_box(Price::from_decimal(black_box("50.01")).unwrap());
    }
    let elapsed_ns = started.elapsed().as_nanos();
    COUNT_ALLOCATIONS.store(false, Ordering::SeqCst);
    let parse_allocations = ALLOCATION_COUNT.load(Ordering::SeqCst);
    report("decimal price parse", elapsed_ns, ITERATIONS);
    println!("price-parse heap allocations: {parse_allocations}");
    assert_eq!(parse_allocations, 0, "decimal price parsing allocated");

    ALLOCATION_COUNT.store(0, Ordering::SeqCst);
    COUNT_ALLOCATIONS.store(true, Ordering::SeqCst);
    let started = Instant::now();
    for index in 0..ITERATIONS {
        let tick = 4_990 + (index & 15);
        book.set_bid_size_at_tick(black_box(tick), black_box(2.0))
            .unwrap();
        book.set_ask_size_at_tick(black_box(tick), black_box(3.0))
            .unwrap();
    }
    let elapsed_ns = started.elapsed().as_nanos();
    COUNT_ALLOCATIONS.store(false, Ordering::SeqCst);
    let write_allocations = ALLOCATION_COUNT.load(Ordering::SeqCst);

    report(
        "two direct tick writes (ns/write is half this figure)",
        elapsed_ns,
        ITERATIONS,
    );
    println!(
        "direct-write heap allocations: {write_allocations} \
         ({:.6} allocations/pair)",
        write_allocations as f64 / ITERATIONS as f64
    );
    assert_eq!(write_allocations, 0, "direct tick writes allocated");
    assert_eq!(book.bid_size_at_tick(5_005), Some(2.0));
    assert_eq!(book.ask_size_at_tick(5_005), Some(3.0));

    let mut update = Update {
        bids: vec![
            vec!["49.99", "2.0"],
            vec!["49.98", "2.0"],
            vec!["49.97", "2.0"],
            vec!["49.96", "2.0"],
        ],
        asks: vec![
            vec!["50.01", "2.0"],
            vec!["50.02", "2.0"],
            vec!["50.03", "2.0"],
            vec!["50.04", "2.0"],
        ],
        ts: "1",
        seq: 2,
        prev_seq: 1,
    };

    for _ in 0..10_000 {
        book.update(black_box(&update)).unwrap();
        update.prev_seq = update.seq;
        update.seq += 1;
    }

    ALLOCATION_COUNT.store(0, Ordering::SeqCst);
    COUNT_ALLOCATIONS.store(true, Ordering::SeqCst);
    let started = Instant::now();
    for _ in 0..ITERATIONS {
        book.update(black_box(&update)).unwrap();
        update.prev_seq = update.seq;
        update.seq += 1;
    }
    let elapsed_ns = started.elapsed().as_nanos();
    COUNT_ALLOCATIONS.store(false, Ordering::SeqCst);
    let allocations = ALLOCATION_COUNT.load(Ordering::SeqCst);

    report(
        "update (8 level writes + 8 price/size parses)",
        elapsed_ns,
        ITERATIONS,
    );
    println!(
        "measured heap allocations: {allocations} ({:.6} allocations/update)",
        allocations as f64 / ITERATIONS as f64
    );
    println!(
        "update throughput: {:.2} M updates/s, {:.2} M level rows/s",
        ITERATIONS as f64 / (elapsed_ns as f64 / 1_000_000_000.0) / 1_000_000.0,
        ITERATIONS as f64 * 8.0 / (elapsed_ns as f64 / 1_000_000_000.0) / 1_000_000.0
    );
    assert_eq!(allocations, 0, "book update loop allocated");

    let mut wide_book = Book::new(
        Price::from_decimal("0.01").unwrap(),
        Price::from_decimal("10000").unwrap(),
        16,
    )
    .unwrap();
    wide_book.set_bid_size_at_tick(10, 1.0).unwrap();
    wide_book.set_bid_size_at_tick(900_000, 1.0).unwrap();
    wide_book.set_ask_size_at_tick(10, 1.0).unwrap();
    wide_book.set_ask_size_at_tick(900_000, 1.0).unwrap();

    ALLOCATION_COUNT.store(0, Ordering::SeqCst);
    COUNT_ALLOCATIONS.store(true, Ordering::SeqCst);
    let started = Instant::now();
    for _ in 0..BEST_LEVEL_ITERATIONS {
        wide_book
            .set_bid_size_at_tick(900_000, black_box(0.0))
            .unwrap();
        wide_book.set_ask_size_at_tick(10, black_box(0.0)).unwrap();
        wide_book
            .set_bid_size_at_tick(900_000, black_box(1.0))
            .unwrap();
        wide_book.set_ask_size_at_tick(10, black_box(1.0)).unwrap();
    }
    let elapsed_ns = started.elapsed().as_nanos();
    COUNT_ALLOCATIONS.store(false, Ordering::SeqCst);
    let best_level_allocations = ALLOCATION_COUNT.load(Ordering::SeqCst);
    report(
        "delete and restore best bid/ask on 1,000,001 slots",
        elapsed_ns,
        BEST_LEVEL_ITERATIONS,
    );
    println!("best-level heap allocations: {best_level_allocations}");
    assert_eq!(best_level_allocations, 0, "best-level updates allocated");
    assert_eq!(
        wide_book.best_bid().unwrap().price.units(),
        9_000_000_000_000_000_000_000
    );
    assert_eq!(
        wide_book.best_ask().unwrap().price.units(),
        100_000_000_000_000_000
    );

    let mut snapshot_book = Book::new(
        Price::from_decimal("0.01").unwrap(),
        Price::from_decimal("10000").unwrap(),
        16,
    )
    .unwrap();
    let mut snapshot = Snapshot {
        asks: vec![
            vec!["50.01", "1.0"],
            vec!["50.02", "1.0"],
            vec!["50.03", "1.0"],
            vec!["50.04", "1.0"],
        ],
        bids: vec![
            vec!["49.99", "1.0"],
            vec!["49.98", "1.0"],
            vec!["49.97", "1.0"],
            vec!["49.96", "1.0"],
        ],
        ts: "2",
        seq: 1,
        prev_seq: 0,
    };
    snapshot_book.update_from_snapshot(&snapshot).unwrap();
    for _ in 0..100 {
        snapshot.seq += 1;
        snapshot_book
            .update_from_snapshot(black_box(&snapshot))
            .unwrap();
    }

    ALLOCATION_COUNT.store(0, Ordering::SeqCst);
    COUNT_ALLOCATIONS.store(true, Ordering::SeqCst);
    let started = Instant::now();
    for _ in 0..SNAPSHOT_ITERATIONS {
        snapshot.seq += 1;
        snapshot_book
            .update_from_snapshot(black_box(&snapshot))
            .unwrap();
    }
    let elapsed_ns = started.elapsed().as_nanos();
    COUNT_ALLOCATIONS.store(false, Ordering::SeqCst);
    let snapshot_allocations = ALLOCATION_COUNT.load(Ordering::SeqCst);
    report(
        "sparse snapshot on 1,000,001 slots (8 levels)",
        elapsed_ns,
        SNAPSHOT_ITERATIONS,
    );
    println!("sparse-snapshot heap allocations: {snapshot_allocations}");
    assert_eq!(snapshot_allocations, 0, "snapshot application allocated");
}
