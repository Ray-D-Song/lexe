use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
    time::{Instant, SystemTime},
};

static TIME_ORIGIN: AtomicU64 = AtomicU64::new(0);
static ORIGIN_INSTANT: OnceLock<Instant> = OnceLock::new();

/// Get the current time in nanoseconds.
///
/// # Safety
/// - Good until the year 2554
/// - Always use a checked substraction since this can return 0
pub fn now_nanos() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

/// Get the current time in millis.
///
/// # Safety
/// - Good until the year 2554
/// - Always use a checked substraction since this can return 0
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Get the origin time in nanoseconds.
///
/// # Safety
/// - Good until the year 2554
/// - Always use a checked substraction since this can return 0
pub fn origin_nanos() -> u64 {
    let origin = TIME_ORIGIN.load(Ordering::Relaxed);
    if origin != 0 {
        return origin;
    }
    init();
    TIME_ORIGIN.load(Ordering::Relaxed)
}

/// Monotonic nanoseconds elapsed since VM time origin initialization.
pub fn monotonic_elapsed_nanos() -> u64 {
    ORIGIN_INSTANT
        .get_or_init(Instant::now)
        .elapsed()
        .as_nanos() as u64
}

/// Monotonic milliseconds elapsed since VM time origin initialization.
pub fn monotonic_elapsed_ms() -> f64 {
    monotonic_elapsed_nanos() as f64 / 1e6
}

// For accuracy reasons, this function should be executed when the vm is initialized
pub fn init() {
    let _ = ORIGIN_INSTANT.get_or_init(Instant::now);
    if TIME_ORIGIN.load(Ordering::Relaxed) == 0 {
        let time_origin = now_nanos();
        let _ = TIME_ORIGIN.compare_exchange(0, time_origin, Ordering::Relaxed, Ordering::Relaxed);
    }
}
