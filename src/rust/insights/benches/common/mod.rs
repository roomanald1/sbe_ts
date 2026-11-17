use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

struct CountingAllocator;

static ALLOCATION_CALLS: AtomicU64 = AtomicU64::new(0);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);
static TRACK_ALLOCATIONS: AtomicBool = AtomicBool::new(false);

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

// SAFETY: Every operation delegates to the system allocator with the original
// pointer and layout; the counters do not affect allocation ownership.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            if TRACK_ALLOCATIONS.load(Ordering::Relaxed) {
                ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
                ALLOCATED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
            }
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_pointer = unsafe { System.realloc(pointer, layout, new_size) };
        if !new_pointer.is_null() {
            if TRACK_ALLOCATIONS.load(Ordering::Relaxed) {
                ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
                ALLOCATED_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
            }
        }
        new_pointer
    }
}

#[cfg(target_os = "linux")]
fn cpu_time_ns() -> u128 {
    let mut time = std::mem::MaybeUninit::<libc::timespec>::uninit();
    // SAFETY: `time` points to writable storage for `timespec`.
    let result = unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, time.as_mut_ptr()) };
    if result != 0 {
        return 0;
    }
    // SAFETY: clock_gettime initialized the structure after returning success.
    let time = unsafe { time.assume_init() };
    (time.tv_sec as u128) * 1_000_000_000 + time.tv_nsec as u128
}

pub fn measure<T>(
    name: &str,
    iterations: usize,
    mut setup: impl FnMut() -> T,
    mut operation: impl FnMut(&mut T, usize),
) {
    const SAMPLES: usize = 9;
    let warmup_iterations = (iterations / 10).clamp(1, 10_000);
    let mut warmup = setup();
    operation(&mut warmup, warmup_iterations);
    drop(warmup);

    let mut times = Vec::with_capacity(SAMPLES);
    let mut allocation_calls = 0_u128;
    let mut allocated_bytes = 0_u128;
    for _ in 0..SAMPLES {
        let mut state = setup();

        #[cfg(target_os = "linux")]
        let started = cpu_time_ns();
        #[cfg(not(target_os = "linux"))]
        let started = std::time::Instant::now();

        operation(&mut state, iterations);

        #[cfg(target_os = "linux")]
        let elapsed_ns = cpu_time_ns().saturating_sub(started);
        #[cfg(not(target_os = "linux"))]
        let elapsed_ns = started.elapsed().as_nanos();

        times.push(elapsed_ns as f64 / iterations as f64);
        drop(state);
    }
    const ALLOCATION_SAMPLES: usize = 3;
    for _ in 0..ALLOCATION_SAMPLES {
        let mut state = setup();
        ALLOCATION_CALLS.store(0, Ordering::Relaxed);
        ALLOCATED_BYTES.store(0, Ordering::Relaxed);
        TRACK_ALLOCATIONS.store(true, Ordering::Relaxed);
        operation(&mut state, iterations);
        TRACK_ALLOCATIONS.store(false, Ordering::Relaxed);
        allocation_calls += ALLOCATION_CALLS.load(Ordering::Relaxed) as u128;
        allocated_bytes += ALLOCATED_BYTES.load(Ordering::Relaxed) as u128;
        drop(state);
    }
    times.sort_by(f64::total_cmp);
    let median = times[times.len() / 2];
    let allocation_operations = (ALLOCATION_SAMPLES * iterations) as f64;
    let allocations_per_operation = allocation_calls as f64 / allocation_operations;
    let bytes_per_operation = allocated_bytes as f64 / allocation_operations;
    println!(
        "{name}: median={median:.1} ns/op range={:.1}..{:.1} samples={SAMPLES} allocations={allocations_per_operation:.4}/op requested_bytes={bytes_per_operation:.1} B/op",
        times[0],
        times[times.len() - 1],
    );
}
