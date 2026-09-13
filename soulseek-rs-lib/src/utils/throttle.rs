//! A ceiling on transfer rate, one per direction, shared across transfers so
//! that raising the slot count does not raise the ceiling with it. Off by
//! default.
//!
//! Uploads are paced by holding back what is written. Downloads are paced by
//! holding back what is read, which leaves the bytes in the kernel's receive
//! buffer and lets TCP tell the sender to slow down — the only way a
//! downloader can limit anything, since it is not the one sending.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// How much idle time may be banked, so a quiet stretch cannot burst.
const MAX_BURST: Duration = Duration::from_secs(1);

/// The smallest slice worth waking for.
const MIN_SLICE: u64 = 1024;

struct Bucket {
    available: f64,
    refilled: Instant,
}

/// One direction's ceiling.
struct Limiter {
    /// Bytes per second. Zero is no limit.
    limit: AtomicU64,
    bucket: Mutex<Option<Bucket>>,
}

impl Limiter {
    const fn new() -> Self {
        Self {
            limit: AtomicU64::new(0),
            bucket: Mutex::new(None),
        }
    }

    fn set(&self, bytes_per_second: u64) {
        self.limit.store(bytes_per_second, Ordering::Relaxed);
        if let Ok(mut bucket) = self.bucket.lock() {
            *bucket = None;
        }
    }

    fn rate(&self) -> u64 {
        self.limit.load(Ordering::Relaxed)
    }

    /// Block until at least some of `want` may pass, and return how much.
    /// Returns `want` unchanged when there is no limit.
    fn take(&self, want: usize) -> usize {
        let limit = self.rate();
        if limit == 0 || want == 0 {
            return want;
        }
        let rate = limit as f64;
        let burst = rate * MAX_BURST.as_secs_f64();
        let wanted = want as f64;

        loop {
            let shortfall = {
                let Ok(mut guard) = self.bucket.lock() else {
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
}

static UPLOADS: Limiter = Limiter::new();
static DOWNLOADS: Limiter = Limiter::new();

/// Cap the total upload rate, in bytes per second. Zero removes the cap.
pub fn set_upload_rate_limit(bytes_per_second: u64) {
    UPLOADS.set(bytes_per_second);
}

/// The current upload cap in bytes per second, or zero when there is none.
#[must_use]
pub fn upload_rate_limit() -> u64 {
    UPLOADS.rate()
}

/// Block until at least some of `want` may be sent, and return how much.
pub fn take_upload_allowance(want: usize) -> usize {
    UPLOADS.take(want)
}

/// Cap the total download rate, in bytes per second. Zero removes the cap.
pub fn set_download_rate_limit(bytes_per_second: u64) {
    DOWNLOADS.set(bytes_per_second);
}

/// The current download cap in bytes per second, or zero when there is none.
#[must_use]
pub fn download_rate_limit() -> u64 {
    DOWNLOADS.rate()
}

/// Block until at least some of `want` may be read, and return how much.
pub fn take_download_allowance(want: usize) -> usize {
    DOWNLOADS.take(want)
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

    #[test]
    fn the_two_directions_do_not_share_a_ceiling() {
        let _guard = SERIAL.lock();
        set_upload_rate_limit(1024);
        set_download_rate_limit(0);

        assert_eq!(
            take_download_allowance(1 << 20),
            1 << 20,
            "an upload cap must not pace downloads"
        );
        set_upload_rate_limit(0);
    }

    #[test]
    fn a_download_limit_paces_what_is_read() {
        let _guard = SERIAL.lock();
        let rate = 20 * 1024;
        set_download_rate_limit(rate);

        let target = 40 * 1024;
        let started = Instant::now();
        let mut read = 0usize;
        while read < target {
            read += take_download_allowance(4096);
        }
        let elapsed = started.elapsed();

        assert!(read >= target);
        assert!(
            elapsed >= Duration::from_millis(1500),
            "{target} bytes at {rate}/s came in in {elapsed:?}"
        );
        set_download_rate_limit(0);
    }
}
