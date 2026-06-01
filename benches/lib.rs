//! Shared utilities for benchmarks and load tests.
//!
//! This module is compiled only for benches (via `#[cfg(bench)]`), but the
//! types defined here (e.g. `MockBackend`, `bench_call_tool`) are designed
//! to mirror the real backend interface so benchmarks exercise the same code
//! paths as production (BackendManager::call_tool, semaphores, rate limiting,
//! retry loop, etc.).

#![allow(unused_imports)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::time::timeout;
use tracing::{info, warn};

/// A simple mock backend whose `call_tool` introduces a configurable delay.
///
/// Tracks the maximum number of concurrently in-flight calls seen during the
/// benchmark run — this is what assertions like "max_seen_concurrent <= N" use.
pub struct MockBackend {
    name: &'static str,
    delay: Duration,
    // Atomic counters: total calls so far + current in-flight (for max tracking)
    total_calls: AtomicUsize,
    in_flight: AtomicUsize,
    max_seen_concurrent: AtomicUsize,
    force_error: std::sync::atomic::AtomicBool,
}

impl MockBackend {
    /// Create a new mock backend with the given name and per-call processing delay.
    pub fn new(name: &'static str, delay: Duration) -> Self {
        Self {
            name,
            delay,
            total_calls: AtomicUsize::new(0),
            in_flight: AtomicUsize::new(0),
            max_seen_concurrent: AtomicUsize::new(0),
            force_error: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Total number of calls received since construction.
    pub fn call_count(&self) -> usize {
        self.total_calls.load(Ordering::SeqCst)
    }

    /// Maximum concurrent calls observed simultaneously.
    pub fn max_seen_concurrent(&self) -> usize {
        self.max_seen_concurrent.load(Ordering::SeqCst)
    }

    /// Reset counters between benchmark iterations.
    pub fn reset(&self) {
        self.total_calls.store(0, Ordering::SeqCst);
        self.in_flight.store(0, Ordering::SeqCst);
        self.max_seen_concurrent.store(0, Ordering::SeqCst);
    }

    /// Set whether future calls return an error.
    pub fn set_force_error(&self, force: bool) {
        self.force_error.store(force, Ordering::SeqCst);
    }

    /// Simulate processing a call: increments counters, sleeps, decrements.
    pub async fn process_call(&self) -> Result<serde_json::Value, String> {
        let total = self.total_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let inflight = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;

        // Update max_seen_concurrent
        loop {
            let current = self.max_seen_concurrent.load(Ordering::SeqCst);
            if inflight <= current {
                break;
            }
            if self
                .max_seen_concurrent
                .compare_exchange(current, inflight, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                break;
            }
        }

        if self.force_error.load(Ordering::SeqCst) {
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            return Err("forced error".to_string());
        }

        // Simulate backend processing time
        tokio::time::sleep(self.delay).await;

        self.in_flight.fetch_sub(1, Ordering::SeqCst);

        Ok(serde_json::json!({
            "backend": self.name,
            "call": total,
            "concurrent": inflight.saturating_sub(1),
        }))
    }
}

/// Benchmark helper: fire `num_calls` concurrent tasks, each invoking `f`,
/// and return the elapsed wall time.
pub async fn bench_concurrent_calls<F, Fut>(num_calls: usize, f: F) -> Duration
where
    F: Fn(usize) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = ()> + Send,
{
    let start = std::time::Instant::now();

    let mut handles = Vec::with_capacity(num_calls);
    for i in 0..num_calls {
        handles.push(tokio::spawn(f(i)));
    }
    for h in handles {
        h.await.expect("task panicked");
    }

    start.elapsed()
}

/// Wrapper that records latency distribution for a single call.
pub async fn timed_call<F, Fut>(f: F) -> (Duration, ())
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let start = std::time::Instant::now();
    f().await;
    (start.elapsed(), ())
}
