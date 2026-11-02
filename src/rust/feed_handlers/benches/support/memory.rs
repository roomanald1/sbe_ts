use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

struct CountingAllocator;
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static LIVE_BYTES: AtomicIsize = AtomicIsize::new(0);
static PEAK_LIVE_BYTES: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwards the allocation contract to the system allocator.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            account_alloc(layout.size());
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: pointer and layout are forwarded unchanged to the system allocator.
        unsafe { System.dealloc(pointer, layout) };
        LIVE_BYTES.fetch_sub(layout.size() as isize, Ordering::Relaxed);
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: forwards the reallocation contract to the system allocator.
        let next = unsafe { System.realloc(pointer, layout, size) };
        if !next.is_null() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            let live = LIVE_BYTES
                .fetch_add(size as isize - layout.size() as isize, Ordering::Relaxed)
                + size as isize
                - layout.size() as isize;
            PEAK_LIVE_BYTES.fetch_max(live, Ordering::Relaxed);
        }
        next
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

fn account_alloc(size: usize) {
    ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    let live = LIVE_BYTES.fetch_add(size as isize, Ordering::Relaxed) + size as isize;
    PEAK_LIVE_BYTES.fetch_max(live, Ordering::Relaxed);
}

#[derive(Clone, Copy)]
pub struct Snapshot {
    allocations: usize,
    live_bytes: isize,
    peak_live_bytes: isize,
}
pub fn snapshot() -> Snapshot {
    Snapshot {
        allocations: ALLOCATIONS.load(Ordering::Relaxed),
        live_bytes: LIVE_BYTES.load(Ordering::Relaxed),
        peak_live_bytes: PEAK_LIVE_BYTES.load(Ordering::Relaxed),
    }
}
pub fn report(label: &str, before: Snapshot) {
    let after = snapshot();
    let rss_kib = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<usize>().ok())
        });
    println!(
        "{label}: allocations={} peak_additional_heap={} KiB process_peak_rss={} KiB",
        after.allocations.saturating_sub(before.allocations),
        after
            .peak_live_bytes
            .saturating_sub(before.live_bytes)
            .max(0) as usize
            / 1024,
        rss_kib.map_or_else(|| "unavailable".to_owned(), |n| n.to_string())
    );
}
