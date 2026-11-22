//! jemalloc as the global allocator, wrapped so heap profiling can tell that it is in use.

use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicBool, Ordering};

/// Set by the first allocation [`Jemalloc`] serves.
static IN_USE: AtomicBool = AtomicBool::new(false);

/// jemalloc with heap profiling support, to declare as the binary's global allocator:
///
/// ```no_run
/// #[cfg(target_os = "linux")]
/// #[global_allocator]
/// static ALLOCATOR: neon_observability::profiling::Jemalloc =
///     neon_observability::profiling::Jemalloc;
/// # fn main() {}
/// ```
///
/// It forwards every call to [`tikv_jemallocator::Jemalloc`] and records that it served an
/// allocation. jemalloc is linked into every Linux build, so without that record a binary on
/// another allocator would get valid but empty heap profiles; with it the heap endpoint
/// answers `501` and the heap push agent is skipped instead.
#[derive(Debug, Clone, Copy, Default)]
pub struct Jemalloc;

impl Jemalloc {
    /// Whether this allocator has served an allocation, which means the binary uses it as
    /// its global allocator. It turns true with the process's first allocation, long before
    /// telemetry starts: reading the configuration alone allocates.
    #[must_use]
    pub fn in_use() -> bool {
        IN_USE.load(Ordering::Relaxed)
    }

    #[inline]
    fn record_use() {
        // Reading first keeps the flag's cache line shared once it is set, so allocations on
        // many threads do not contend for it.
        if !IN_USE.load(Ordering::Relaxed) {
            IN_USE.store(true, Ordering::Relaxed);
        }
    }
}

// SAFETY: every method passes its arguments unchanged to `tikv_jemallocator::Jemalloc`, whose
// implementation upholds the `GlobalAlloc` contract, and returns its result unchanged. The
// only addition is an atomic flag, which neither allocates nor panics.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Jemalloc {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        Self::record_use();
        // SAFETY: the caller's guarantees for `alloc` are passed on as they are.
        unsafe { tikv_jemallocator::Jemalloc.alloc(layout) }
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        Self::record_use();
        // SAFETY: the caller's guarantees for `alloc_zeroed` are passed on as they are.
        unsafe { tikv_jemallocator::Jemalloc.alloc_zeroed(layout) }
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from this allocator, so from `tikv_jemallocator::Jemalloc`.
        unsafe { tikv_jemallocator::Jemalloc.dealloc(ptr, layout) }
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: `ptr` came from this allocator, so from `tikv_jemallocator::Jemalloc`, and
        // the caller's other guarantees for `realloc` are passed on as they are.
        unsafe { tikv_jemallocator::Jemalloc.realloc(ptr, layout, new_size) }
    }
}
