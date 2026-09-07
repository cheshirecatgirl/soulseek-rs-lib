//! A ceiling on upload rate, shared across transfers so that raising the slot
//! count does not raise the ceiling with it. Off by default.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Bytes per second across all uploads. Zero is no limit.
static LIMIT: AtomicU64 = AtomicU64::new(0);

/// How much idle time may be banked, so a quiet stretch cannot burst.
const MAX_BURST: Duration = Duration::from_secs(1);

/// The smallest slice worth waking for.
const MIN_SLICE: u64 = 1024;

struct Bucket {
    available: f64,
    refilled: Instant,
}

static BUCKET: Mutex<Option<Bucket>> = Mutex::new(None);

/// Cap the total upload rate, in bytes per second. Zero removes the cap.
pub fn set_upload_rate_limit(bytes_per_second: u64) {
    LIMIT.store(bytes_per_second, Ordering::Relaxed);
    if let Ok(mut bucket) = BUCKET.lock() {
        *bucket = None;
    }
}

/// The current cap in bytes per second, or zero when there is none.
#[must_use]
pub fn upload_rate_limit() -> u64 {
    LIMIT.load(Ordering::Relaxed)
}

/// Block until at least some of `want` may be sent, and return how much.
/// Returns `want` unchanged when there is no limit.
pub fn take_upload_allowance(want: usize) -> usize {
    let limit = upload_rate_limit();
    if limit == 0 || want == 0 {
        return want;
    }
    let rate = limit as f64;
    let burst = rate * MAX_BURST.as_secs_f64();
    let wanted = want as f64;

    loop {
        let shortfall = {
            let Ok(mut guard) = BUCKET.lock() else {
                return want; // a poisoned bucket must not stall transfers
            };
            let now = Instant::now();
            let bucket = guard.get_or_insert(Bucket {
                available: 0.0,
                refilled: now,
            });
            let elapsed = now.saturating_duration_since(bucket.refilled);
            bucket.available = elapsed
                .as_secs_f64()
                .mul_add(rate, bucket.available)
                .min(burst);
            bucket.refilled = now;

            let grant = bucket.available.min(wanted);
            if grant >= MIN_SLICE as f64 || grant >= wanted {
                bucket.available -= grant;
                return grant as usize;
            }
            Duration::from_secs_f64(
                ((MIN_SLICE as f64).min(wanted) - grant) / rate,
            )
        };
        std::thread::sleep(shortfall.min(MAX_BURST));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The limit is process-wide, so these cannot run in parallel.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn no_limit_hands_back_everything_asked_for() {
        let _guard = SERIAL.lock();
        set_upload_rate_limit(0);
        assert_eq!(take_upload_allowance(1 << 20), 1 << 20);
    }

    #[test]
    fn a_limit_actually_paces_the_bytes_it_hands_out() {
        let _guard = SERIAL.lock();
        let rate = 20 * 1024;
        set_upload_rate_limit(rate);

        let target = 40 * 1024;
        let started = Instant::now();
        let mut sent = 0usize;
        while sent < target {
            sent += take_upload_allowance(4096);
        }
        let elapsed = started.elapsed();

        assert!(sent >= target);
        // An upper bound alone would pass with no throttling at all.
        assert!(
            elapsed >= Duration::from_millis(1500),
            "{target} bytes at {rate}/s went out in {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(6),
            "throttling should pace, not stall: took {elapsed:?}"
        );
        set_upload_rate_limit(0);
    }

    #[test]
    fn a_slow_limit_still_makes_progress_on_a_large_ask() {
        let _guard = SERIAL.lock();
        set_upload_rate_limit(4 * 1024);

        let granted = take_upload_allowance(1 << 20);
        assert!(granted > 0);
        assert!(granted <= 1 << 20);
        set_upload_rate_limit(0);
    }

    #[test]
    fn changing_the_limit_does_not_carry_the_old_allowance_over() {
        let _guard = SERIAL.lock();
        set_upload_rate_limit(1 << 20);
        std::thread::sleep(Duration::from_millis(20));
        set_upload_rate_limit(8 * 1024);

        let granted = take_upload_allowance(1 << 20);
        assert!(
            granted <= 8 * 1024,
            "a fresh limit granted {granted} at once"
        );
        set_upload_rate_limit(0);
    }
}
