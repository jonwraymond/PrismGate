//! In-memory tracking for recent tool calls, usage counts, and backend latency.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// Completed backend calls and failures since startup/reset.
    backend_counts: DashMap<String, (u64, u64)>,
    /// Per-backend input/output token estimates. (input, output).
    backend_tokens: DashMap<String, (u64, u64)>,
    /// Per-backend latency histograms. Inner Mutex because Histogram::record is &mut self.
    latency: DashMap<String, Mutex<Histogram<u64>>>,
    /// Maximum entries in the recent ring buffer.
    max_recent: usize,
    /// Per-tool bytes returned to context (after truncation/filtering).
    bytes_returned: DashMap<String, u64>,
    /// Total raw bytes processed before truncation/filtering.
    bytes_processed: AtomicU64,
    /// Session start time for uptime calculation.
    session_start: Mutex<Instant>,
    /// Serializes reset against complete tracker mutations.
    reset_gate: Mutex<()>,
    /// Estimated USD cost in micros ($1 = 1_000_000). Derived from processed bytes.
    estimated_cost_usd_micros: AtomicU64,
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
            backend_counts: DashMap::new(),
            backend_tokens: DashMap::new(),
            latency: DashMap::new(),
            max_recent,
            bytes_returned: DashMap::new(),
            bytes_processed: AtomicU64::new(0),
            session_start: Mutex::new(Instant::now()),
            reset_gate: Mutex::new(()),
            estimated_cost_usd_micros: AtomicU64::new(0),
        }
    }

    /// Clear shared gateway history and statistics without touching backend runtime.
    /// Calls completing after this boundary are recorded as new activity.
    pub fn reset(&self) {
        let _gate = self.reset_gate.lock().unwrap_or_else(|e| e.into_inner());
        self.recent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.usage_counts.clear();
        self.backend_counts.clear();
        self.backend_tokens.clear();
        self.latency.clear();
        self.bytes_returned.clear();
        self.bytes_processed.store(0, Ordering::Relaxed);
        self.estimated_cost_usd_micros.store(0, Ordering::Relaxed);
        *self.session_start.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    /// Record a completed tool call. Called from BackendManager::call_tool.
    pub fn record(&self, tool_name: &str, backend_name: &str, duration: Duration, success: bool) {
        let _gate = self.reset_gate.lock().unwrap_or_else(|e| e.into_inner());
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

        {
            let mut counts = self
                .backend_counts
                .entry(backend_name.to_owned())
                .or_default();
            counts.0 += 1;
            counts.1 += u64::from(!success);
        }

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

    /// Record a completed call with estimated input/output token counts.
    /// Estimates use the ~4 bytes/token heuristic and are labeled as estimates.
    pub fn record_with_tokens(
        &self,
        tool_name: &str,
        backend_name: &str,
        duration: Duration,
        success: bool,
        input_tokens: u64,
        output_tokens: u64,
    ) {
        self.record(tool_name, backend_name, duration, success);
        let _gate = self.reset_gate.lock().unwrap_or_else(|e| e.into_inner());
        self.backend_tokens
            .entry(backend_name.to_string())
            .and_modify(|t| {
                t.0 += input_tokens;
                t.1 += output_tokens;
            })
            .or_insert((input_tokens, output_tokens));
    }

    /// Aggregate metadata only; no arguments or backend error bodies.
    pub fn profile(&self) -> serde_json::Value {
        let _gate = self.reset_gate.lock().unwrap_or_else(|e| e.into_inner());
        let mut names: Vec<String> = self
            .backend_counts
            .iter()
            .map(|e| e.key().clone())
            .collect();
        names.sort();
        let backends: Vec<_> = names
            .iter()
            .map(|name| {
                let counts = self
                    .backend_counts
                    .get(name)
                    .expect("snapshot under reset gate");
                let (in_tok, out_tok) = self
                    .backend_tokens
                    .get(name)
                    .map(|r| *r.value())
                    .unwrap_or((0, 0));
                serde_json::json!({"backend": name, "calls": counts.0, "errors": counts.1,
                "error_rate": if counts.0 == 0 { 0.0 } else { counts.1 as f64 / counts.0 as f64 },
                "latency": self.latency_stats(name),
                "input_tokens_est": in_tok, "output_tokens_est": out_tok,
                "input_bytes_est": in_tok * 4, "output_bytes_est": out_tok * 4})
            })
            .collect();
        serde_json::json!({"scope": "process_since_reset", "backends": backends})
    }

    #[cfg(test)]
    fn assert_profile_contract() {
        let tracker = Self::with_capacity(1);
        tracker.record(
            "private_tool_name",
            "backend",
            Duration::from_millis(10),
            true,
        );
        tracker.record(
            "private_tool_name",
            "backend",
            Duration::from_millis(20),
            false,
        );
        let snapshot = tracker.profile();
        assert_eq!(snapshot["backends"][0]["calls"], 2);
        assert_eq!(snapshot["backends"][0]["errors"], 1);
        assert_eq!(snapshot["backends"][0]["error_rate"], 0.5);
        assert!(!snapshot.to_string().contains("private_tool_name"));
        tracker.reset();
        assert_eq!(tracker.profile()["backends"], serde_json::json!([]));
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
        let _gate = self.reset_gate.lock().unwrap_or_else(|e| e.into_inner());
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

    /// Record bytes for a tool call (after truncation/filtering).
    ///
    /// `returned` = bytes actually sent to context (after truncation).
    /// `processed` = raw bytes before truncation.
    pub fn record_bytes(&self, tool_name: &str, returned: u64, processed: u64) {
        let _gate = self.reset_gate.lock().unwrap_or_else(|e| e.into_inner());
        self.bytes_returned
            .entry(tool_name.to_string())
            .and_modify(|b| *b += returned)
            .or_insert(returned);
        self.bytes_processed.fetch_add(processed, Ordering::Relaxed);
        // Heuristic: ~4 bytes/token at $3 / 1M tokens → 3 micros USD per token.
        let tokens = processed / 4;
        self.estimated_cost_usd_micros
            .fetch_add(tokens.saturating_mul(3), Ordering::Relaxed);
    }

    /// Get session-level statistics for context savings tracking.
    pub fn session_stats(&self) -> SessionStats {
        let total_calls: u64 = self.usage_counts.iter().map(|r| *r.value()).sum();
        let total_bytes_returned: u64 = self.bytes_returned.iter().map(|r| *r.value()).sum();
        let total_bytes_processed = self.bytes_processed.load(Ordering::Relaxed);
        let uptime = self
            .session_start
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .elapsed()
            .as_secs_f64();

        let savings_ratio = if total_bytes_returned > 0 {
            total_bytes_processed as f64 / total_bytes_returned as f64
        } else {
            1.0
        };
        let reduction_pct = if total_bytes_processed > 0 {
            (1.0 - total_bytes_returned as f64 / total_bytes_processed as f64) * 100.0
        } else {
            0.0
        };

        let mut per_tool: Vec<ToolByteStat> = self
            .bytes_returned
            .iter()
            .map(|r| ToolByteStat {
                name: r.key().clone(),
                calls: self.usage_count(r.key()),
                bytes_returned: *r.value(),
            })
            .collect();
        per_tool.sort_by_key(|stat| std::cmp::Reverse(stat.bytes_returned));

        SessionStats {
            uptime_seconds: uptime,
            total_calls,
            total_bytes_returned,
            total_bytes_processed,
            savings_ratio,
            reduction_pct,
            estimated_tokens_saved: (total_bytes_processed.saturating_sub(total_bytes_returned))
                / 4,
            estimated_cost_usd: self.estimated_cost_usd_micros.load(Ordering::Relaxed) as f64
                / 1_000_000.0,
            per_tool,
        }
    }
}

/// Session-level statistics for context savings tracking.
#[derive(Debug, Clone, Serialize)]
pub struct SessionStats {
    pub uptime_seconds: f64,
    pub total_calls: u64,
    pub total_bytes_returned: u64,
    pub total_bytes_processed: u64,
    pub savings_ratio: f64,
    pub reduction_pct: f64,
    pub estimated_tokens_saved: u64,
    /// Estimated USD spent on processed tokens. Unknown-model heuristic, not a bill.
    pub estimated_cost_usd: f64,
    pub per_tool: Vec<ToolByteStat>,
}

/// Per-tool byte tracking statistics.
#[derive(Debug, Clone, Serialize)]
pub struct ToolByteStat {
    pub name: String,
    pub calls: u64,
    pub bytes_returned: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_counts_survive_ring_eviction_and_reset() {
        CallTracker::assert_profile_contract();
    }

    #[test]
    fn token_accounting_tracks_input_output_per_backend() {
        let tracker = CallTracker::with_capacity(1);
        // Simulate calls with estimated token counts (4 bytes/token heuristic)
        tracker.record_with_tokens("a.b", "backend", Duration::from_millis(5), true, 120, 480);
        tracker.record_with_tokens("a.b", "backend", Duration::from_millis(6), true, 80, 320);
        let snapshot = tracker.profile();
        let be = &snapshot["backends"][0];
        assert_eq!(be["calls"], 2);
        assert_eq!(be["input_tokens_est"], 200); // 120 + 80
        assert_eq!(be["output_tokens_est"], 800); // 480 + 320
        assert_eq!(be["input_bytes_est"], 800); // 200 * 4
        assert_eq!(be["output_bytes_est"], 3200); // 800 * 4
    }

    #[test]
    fn reset_clears_history_usage_bytes_and_latency() {
        let tracker = CallTracker::new();
        tracker.record("old", "healthy_backend", Duration::from_millis(5), true);
        tracker.record_bytes("old", 10, 100);
        let before = tracker.session_stats().uptime_seconds;
        tracker.reset();
        assert!(tracker.recent_calls(10).is_empty());
        assert!(tracker.snapshot_usage().is_empty());
        assert!(tracker.backends_with_latency().is_empty());
        let stats = tracker.session_stats();
        assert_eq!(stats.total_calls, 0);
        assert_eq!(stats.total_bytes_returned, 0);
        assert_eq!(stats.total_bytes_processed, 0);
        assert_eq!(stats.estimated_cost_usd, 0.0);
        assert!(stats.per_tool.is_empty());
        assert!(stats.uptime_seconds < before + 0.1);
        tracker.reset(); // idempotent, still accepts new calls
        tracker.record("new", "healthy_backend", Duration::from_millis(1), true);
        assert_eq!(tracker.usage_count("new"), 1);
        assert_eq!(tracker.recent_calls(10).len(), 1);
    }

    #[test]
    fn estimated_cost_usd_from_processed_bytes() {
        let tracker = CallTracker::new();
        tracker.record_bytes("tool", 100, 4_000);
        let stats = tracker.session_stats();
        // 4000 bytes / 4 = 1000 tokens * 3 micros = 3000 micros = $0.003
        assert!((stats.estimated_cost_usd - 0.003).abs() < 1e-9);
        tracker.reset();
        assert_eq!(tracker.session_stats().estimated_cost_usd, 0.0);
    }

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

    #[test]
    fn test_record_bytes_per_tool() {
        let tracker = CallTracker::new();
        tracker.record_bytes("tool_a", 100, 500);
        tracker.record_bytes("tool_a", 200, 1000);
        tracker.record_bytes("tool_b", 50, 300);

        let stats = tracker.session_stats();
        assert_eq!(stats.total_bytes_returned, 350); // 100+200+50
        assert_eq!(stats.total_bytes_processed, 1800); // 500+1000+300

        let tool_a = stats.per_tool.iter().find(|t| t.name == "tool_a").unwrap();
        assert_eq!(tool_a.bytes_returned, 300);
    }

    #[test]
    fn test_session_stats_savings() {
        let tracker = CallTracker::new();
        tracker.record("tool_a", "b1", Duration::from_millis(1), true);
        tracker.record_bytes("tool_a", 100, 10_000); // 100 bytes returned from 10KB raw

        let stats = tracker.session_stats();
        assert_eq!(stats.total_calls, 1);
        assert_eq!(stats.total_bytes_returned, 100);
        assert_eq!(stats.total_bytes_processed, 10_000);
        // Savings ratio: 10000/100 = 100x
        assert!((stats.savings_ratio - 100.0).abs() < 0.1);
        // Reduction: (1 - 100/10000) * 100 = 99%
        assert!((stats.reduction_pct - 99.0).abs() < 0.1);
        // Tokens saved: (10000 - 100) / 4 = 2475
        assert_eq!(stats.estimated_tokens_saved, 2475);
    }

    #[test]
    fn test_session_stats_zero_returned() {
        let tracker = CallTracker::new();
        tracker.record_bytes("call_tool_chain", 0, 10_000);
        let stats = tracker.session_stats();
        assert_eq!(stats.total_calls, 0); // Byte accounting does not invent backend calls.
        assert_eq!(stats.total_bytes_returned, 0);
        assert_eq!(stats.total_bytes_processed, 10_000);
        assert_eq!(stats.savings_ratio, 1.0); // Finite fallback when division is undefined.
        assert_eq!(stats.reduction_pct, 100.0);
        assert_eq!(stats.estimated_tokens_saved, 2_500);
        assert_eq!(stats.per_tool[0].calls, 0);
        assert!(serde_json::to_string(&stats).is_ok());
    }

    #[test]
    fn test_session_stats_large_reduction_ratio() {
        let tracker = CallTracker::new();
        tracker.record_bytes("call_tool_chain", 1, 1_000_000_000);
        let stats = tracker.session_stats();
        assert_eq!(stats.savings_ratio, 1_000_000_000.0);
        assert!((stats.reduction_pct - 99.9999999).abs() < 1e-9);
        assert_eq!(stats.estimated_tokens_saved, 249_999_999);
    }

    #[test]
    fn test_session_stats_output_expansion() {
        let tracker = CallTracker::new();
        tracker.record_bytes("call_tool_chain", 200, 100);
        let stats = tracker.session_stats();
        assert_eq!(stats.savings_ratio, 0.5);
        assert_eq!(stats.reduction_pct, -100.0);
        assert_eq!(stats.estimated_tokens_saved, 0);
    }

    #[test]
    fn test_session_stats_zero_byte_record() {
        let tracker = CallTracker::new();
        tracker.record_bytes("call_tool_chain", 0, 0);
        let stats = tracker.session_stats();
        assert_eq!(stats.total_bytes_processed, 0);
        assert_eq!(stats.total_bytes_returned, 0);
        assert_eq!(stats.savings_ratio, 1.0);
        assert_eq!(stats.reduction_pct, 0.0);
        assert_eq!(stats.estimated_tokens_saved, 0);
        assert_eq!(stats.per_tool.len(), 1);
    }

    #[test]
    fn test_session_stats_no_data() {
        let tracker = CallTracker::new();
        let stats = tracker.session_stats();
        assert_eq!(stats.total_calls, 0);
        assert_eq!(stats.total_bytes_returned, 0);
        assert_eq!(stats.savings_ratio, 1.0);
        assert_eq!(stats.reduction_pct, 0.0);
    }
}
