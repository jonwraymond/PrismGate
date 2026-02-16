use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use hdrhistogram::Histogram;
use serde::Serialize;

/// Default maximum number of recent call events to keep.
const DEFAULT_MAX_RECENT: usize = 500;

/// A single tool call event recorded by the tracker.
#[derive(Debug, Clone)]
pub struct CallEvent {
    pub tool_name: String,
    pub backend_name: String,
    pub timestamp: Instant,
    pub duration: Duration,
    pub success: bool,
}

/// Serializable summary of a call event for the recent-tools resource.
#[derive(Debug, Clone, Serialize)]
pub struct CallEventSummary {
    pub tool_name: String,
    pub backend_name: String,
    pub duration_ms: u64,
    pub success: bool,
    /// Seconds ago relative to the snapshot time.
    pub seconds_ago: f64,
}

/// Latency statistics for a backend.
#[derive(Debug, Clone, Serialize)]
pub struct LatencyStats {
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub avg_ms: f64,
    pub sample_count: u64,
}

/// Thread-safe call tracker that records tool invocations.
///
/// Provides three concurrent data structures:
/// - A bounded ring buffer of recent calls (Mutex<VecDeque>)
/// - Per-tool usage counters (DashMap<String, u64>)
/// - Per-backend HDR histograms for latency percentiles (DashMap<String, Mutex<Histogram>>)
pub struct CallTracker {
    /// Bounded FIFO of recent call events. Mutex held only for the push (~nanoseconds).
    recent: Mutex<VecDeque<CallEvent>>,
    /// Per-tool invocation counts for usage-weighted search.
    usage_counts: DashMap<String, u64>,
    /// Per-backend latency histograms. Inner Mutex because Histogram::record is &mut self.
    latency: DashMap<String, Mutex<Histogram<u64>>>,
    /// Maximum entries in the recent ring buffer.
    max_recent: usize,
}

impl CallTracker {
    /// Create a new tracker with default capacity (500 recent events).
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_MAX_RECENT)
    }

    /// Create a new tracker with a custom recent-events capacity.
    pub fn with_capacity(max_recent: usize) -> Self {
        Self {
            recent: Mutex::new(VecDeque::with_capacity(max_recent)),
            usage_counts: DashMap::new(),
            latency: DashMap::new(),
            max_recent,
        }
    }

    /// Record a completed tool call. Called from BackendManager::call_tool.
    pub fn record(&self, tool_name: &str, backend_name: &str, duration: Duration, success: bool) {
        let event = CallEvent {
            tool_name: tool_name.to_string(),
            backend_name: backend_name.to_string(),
            timestamp: Instant::now(),
            duration,
            success,
        };

        // Update recent ring buffer
        {
            let mut recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
            if recent.len() >= self.max_recent {
                recent.pop_front();
            }
            recent.push_back(event);
        }

        // Update usage count (lock-free via DashMap)
        self.usage_counts
            .entry(tool_name.to_string())
            .and_modify(|c| *c += 1)
            .or_insert(1);

        // Update latency histogram
        let duration_us = duration.as_micros() as u64;
        self.latency
            .entry(backend_name.to_string())
            .or_insert_with(|| {
                // Track latencies from 1µs to 10 minutes with 3 significant digits
                Mutex::new(
                    Histogram::<u64>::new_with_bounds(1, 600_000_000, 3)
                        .expect("valid histogram bounds"),
                )
            })
            .value()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .record(duration_us.max(1)) // clamp to min 1µs
            .ok(); // ignore out-of-range (>10min)
    }

    /// Get the total invocation count for a tool.
    pub fn usage_count(&self, tool_name: &str) -> u64 {
        self.usage_counts
            .get(tool_name)
            .map(|r| *r.value())
            .unwrap_or(0)
    }

    /// Snapshot all usage counts for cache persistence.
    pub fn snapshot_usage(&self) -> HashMap<String, u64> {
        self.usage_counts
            .iter()
            .map(|r| (r.key().clone(), *r.value()))
            .collect()
    }

    /// Load usage counts from cache (additive — merges with existing).
    pub fn load_usage(&self, counts: HashMap<String, u64>) {
        for (tool, count) in counts {
            self.usage_counts
                .entry(tool)
                .and_modify(|c| *c += count)
                .or_insert(count);
        }
    }

    /// Get latency statistics for a specific backend.
    pub fn latency_stats(&self, backend_name: &str) -> Option<LatencyStats> {
        let entry = self.latency.get(backend_name)?;
        let hist = entry.value().lock().unwrap_or_else(|e| e.into_inner());
        if hist.is_empty() {
            return None;
        }
        Some(LatencyStats {
            p50_ms: hist.value_at_quantile(0.50) as f64 / 1000.0,
            p95_ms: hist.value_at_quantile(0.95) as f64 / 1000.0,
            p99_ms: hist.value_at_quantile(0.99) as f64 / 1000.0,
            avg_ms: hist.mean() / 1000.0,
            sample_count: hist.len(),
        })
    }

    /// Get recent call events as serializable summaries.
    pub fn recent_calls(&self, limit: usize) -> Vec<CallEventSummary> {
        let now = Instant::now();
        let recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
        recent
            .iter()
            .rev() // most recent first
            .take(limit)
            .map(|e| CallEventSummary {
                tool_name: e.tool_name.clone(),
                backend_name: e.backend_name.clone(),
                duration_ms: e.duration.as_millis() as u64,
                success: e.success,
                seconds_ago: now.duration_since(e.timestamp).as_secs_f64(),
            })
            .collect()
    }

    /// Get all backend names that have latency data.
    pub fn backends_with_latency(&self) -> Vec<String> {
        self.latency
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_recent() {
        let tracker = CallTracker::new();

        tracker.record("tool_a", "backend1", Duration::from_millis(10), true);
        tracker.record("tool_b", "backend1", Duration::from_millis(20), false);
        tracker.record("tool_c", "backend2", Duration::from_millis(30), true);

        let recent = tracker.recent_calls(10);
        assert_eq!(recent.len(), 3);
        // Most recent first
        assert_eq!(recent[0].tool_name, "tool_c");
        assert_eq!(recent[1].tool_name, "tool_b");
        assert_eq!(recent[2].tool_name, "tool_a");
        // Check fields
        assert!(!recent[1].success);
        assert_eq!(recent[2].duration_ms, 10);
    }

    #[test]
    fn test_bounded_ring_buffer() {
        let tracker = CallTracker::with_capacity(5);

        for i in 0..10 {
            tracker.record(
                &format!("tool_{i}"),
                "backend",
                Duration::from_millis(1),
                true,
            );
        }

        let recent = tracker.recent_calls(100);
        assert_eq!(recent.len(), 5);
        // Should have tools 5-9 (oldest 0-4 evicted)
        assert_eq!(recent[0].tool_name, "tool_9");
        assert_eq!(recent[4].tool_name, "tool_5");
    }

    #[test]
    fn test_usage_counts() {
        let tracker = CallTracker::new();

        tracker.record("tool_a", "b1", Duration::from_millis(1), true);
        tracker.record("tool_a", "b1", Duration::from_millis(1), true);
        tracker.record("tool_a", "b1", Duration::from_millis(1), false);
        tracker.record("tool_b", "b1", Duration::from_millis(1), true);

        assert_eq!(tracker.usage_count("tool_a"), 3);
        assert_eq!(tracker.usage_count("tool_b"), 1);
        assert_eq!(tracker.usage_count("tool_c"), 0);

        // Snapshot
        let snap = tracker.snapshot_usage();
        assert_eq!(snap.get("tool_a"), Some(&3));
        assert_eq!(snap.get("tool_b"), Some(&1));
    }

    #[test]
    fn test_latency_recording() {
        let tracker = CallTracker::new();

        // Record known durations
        for i in 1..=100 {
            tracker.record("tool", "backend", Duration::from_millis(i), true);
        }

        let stats = tracker.latency_stats("backend").unwrap();
        assert_eq!(stats.sample_count, 100);
        // p50 should be around 50ms (±histogram quantization)
        assert!(
            stats.p50_ms > 40.0 && stats.p50_ms < 60.0,
            "p50={}",
            stats.p50_ms
        );
        // p95 should be around 95ms
        assert!(
            stats.p95_ms > 85.0 && stats.p95_ms < 105.0,
            "p95={}",
            stats.p95_ms
        );
        // No data for unknown backend
        assert!(tracker.latency_stats("unknown").is_none());
    }

    #[tokio::test]
    async fn test_concurrent_recording() {
        use std::sync::Arc;

        let tracker = Arc::new(CallTracker::new());
        let mut handles = Vec::new();

        for task_id in 0..10 {
            let t = Arc::clone(&tracker);
            handles.push(tokio::spawn(async move {
                for i in 0..50 {
                    t.record(
                        &format!("tool_{task_id}_{i}"),
                        &format!("backend_{task_id}"),
                        Duration::from_micros(100 + i),
                        true,
                    );
                }
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        // 10 tasks * 50 calls = 500 total
        let total_usage: u64 = tracker.snapshot_usage().values().sum();
        assert_eq!(total_usage, 500);

        // All 10 backends should have latency data
        assert_eq!(tracker.backends_with_latency().len(), 10);

        // Recent buffer should have 500 (matches capacity)
        let recent = tracker.recent_calls(1000);
        assert_eq!(recent.len(), 500);
    }

    #[test]
    fn test_load_usage() {
        let tracker = CallTracker::new();
        tracker.record("tool_a", "b", Duration::from_millis(1), true);

        let mut cached = HashMap::new();
        cached.insert("tool_a".to_string(), 10);
        cached.insert("tool_b".to_string(), 5);
        tracker.load_usage(cached);

        assert_eq!(tracker.usage_count("tool_a"), 11); // 1 + 10
        assert_eq!(tracker.usage_count("tool_b"), 5);
    }

    #[test]
    fn test_recent_calls_limit() {
        let tracker = CallTracker::new();
        for i in 0..10 {
            tracker.record(&format!("tool_{i}"), "b", Duration::from_millis(1), true);
        }

        let recent = tracker.recent_calls(3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].tool_name, "tool_9");
    }

    // --- Phase 3: Latency stats and recent calls tests ---

    #[test]
    fn test_latency_stats_empty() {
        let tracker = CallTracker::new();
        // No calls recorded — latency should be None
        assert!(tracker.latency_stats("nonexistent").is_none());
    }

    #[test]
    fn test_latency_stats_populated() {
        let tracker = CallTracker::new();
        // Record 100 calls with durations 1ms..100ms
        for i in 1..=100u64 {
            tracker.record("tool", "my_backend", Duration::from_millis(i), true);
        }

        let stats = tracker.latency_stats("my_backend").unwrap();
        assert_eq!(stats.sample_count, 100);
        // p50 should be around 50ms (±histogram quantization)
        assert!(
            (45.0..=55.0).contains(&stats.p50_ms),
            "p50 should be ~50ms, got {:.1}",
            stats.p50_ms
        );
        // p95 should be around 95ms
        assert!(
            (90.0..=100.0).contains(&stats.p95_ms),
            "p95 should be ~95ms, got {:.1}",
            stats.p95_ms
        );
        // p99 should be around 99-100ms
        assert!(
            (95.0..=105.0).contains(&stats.p99_ms),
            "p99 should be ~99ms, got {:.1}",
            stats.p99_ms
        );
    }

    #[test]
    fn test_recent_calls_structure() {
        let tracker = CallTracker::new();
        tracker.record("my_tool", "my_backend", Duration::from_millis(42), true);
        tracker.record("other_tool", "my_backend", Duration::from_millis(10), false);

        let recent = tracker.recent_calls(10);
        assert_eq!(recent.len(), 2);
        // Most recent first
        assert_eq!(recent[0].tool_name, "other_tool");
        assert!(!recent[0].success);
        assert_eq!(recent[1].tool_name, "my_tool");
        assert!(recent[1].success);
        // Duration should be recorded
        assert!(recent[1].duration_ms > 0);
    }

    #[test]
    fn test_backends_with_latency() {
        let tracker = CallTracker::new();
        tracker.record("tool_a", "backend_1", Duration::from_millis(10), true);
        tracker.record("tool_b", "backend_2", Duration::from_millis(20), true);

        let backends = tracker.backends_with_latency();
        assert_eq!(backends.len(), 2);
        assert!(backends.contains(&"backend_1".to_string()));
        assert!(backends.contains(&"backend_2".to_string()));
    }
}
