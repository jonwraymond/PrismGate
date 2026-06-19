//! Concurrent tool-call benchmarks for `BackendManager`.
//!
//! Exercises the full async code path: semaphore acquire, rate-limit check,
//! retry loop, state check, and per-backend `call_tool`.
//!
//! Run with: `cargo bench --bench concurrent_calls`

use std::sync::Arc;
use std::time::Duration;

use criterion::{black_box, BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use futures_util::future::join_all;
use serde_json::Value;

use gatemini::backend::{BackendManager, BackendState, STATE_HEALTHY};
use gatemini::config::{BackendConfig, InstanceMode, PoolConfig, RetryConfig, Transport};
use gatemini::registry::ToolRegistry;
use gatemini::testutil::MockBackend;

fn manager_with_mock_backend(
    name: &'static str,
    delay: Duration,
    max_concurrent: u32,
) -> (Arc<BackendManager>, Arc<MockBackend>) {
    let mock = Arc::new(MockBackend::new(name, delay));

    let mut config = BackendConfig {
        transport: Transport::Stdio,
        namespace: None,
        command: None,
        args: Vec::new(),
        env: std::collections::HashMap::new(),
        cwd: None,
        url: None,
        headers: std::collections::HashMap::new(),
        timeout: Duration::from_secs(10),
        required_keys: Vec::new(),
        max_concurrent_calls: Some(max_concurrent),
        semaphore_timeout: Duration::from_secs(30),
        retry: RetryConfig {
            max_retries: 1,
            initial_delay: Duration::from_millis(10),
            backoff_multiplier: 1.5,
            max_delay: Duration::from_millis(50),
        },
        prerequisite: None,
        rate_limit: None,
        tags: Vec::new(),
        fallback_chain: Vec::new(),
        tools: None,
        adapter_file: None,
        health_check: None,
        instance_mode: InstanceMode::Shared,
        pool: PoolConfig::default(),
        shutdown_grace_period: Duration::from_secs(5),
        max_memory_mb: None,
    };

    let manager = BackendManager::new();
    let registry = Arc::new(ToolRegistry::new());

    // Register mock backend directly (skip child process)
    manager.register_virtual_backend(name, Arc::clone(&mock) as Arc<dyn gatemini::backend::Backend>);

    // Also store config so semaphores and retry configs are set up correctly
    {
        let mut configs = manager.configs.try_write_for(Duration::from_secs(1)).unwrap();
        configs.insert(name.to_string(), config);
    }
    // Manually install semaphore (add_backend would do this, but we're direct-registering)
    if max_concurrent > 0 {
        manager
            .call_semaphores
            .insert(name.to_string(), Arc::new(tokio::sync::Semaphore::new(max_concurrent as usize)));
    }

    (manager, mock)
}

// ---- Single-backend latency benchmarks ----

fn bench_single_call(c: &mut Criterion) {
    let (manager, mock) = manager_with_mock_backend("fast-backend", Duration::from_millis(1), 100);
    mock.reset();

    c.bench_function("call_tool_single_1ms", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| {
                let mgr = Arc::clone(&manager);
                async {
                    let _ = mgr
                        .call_tool("fast-backend", "echo_tool", Some(Value::Null), None)
                        .await;
                }
            })
    });
}

fn bench_10_concurrent(c: &mut Criterion) {
    let (manager, mock) = manager_with_mock_backend("10-concurrent", Duration::from_millis(10), 100);
    mock.reset();

    c.bench_function("call_tool_10_concurrent_10ms", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| {
                let mgr = Arc::clone(&manager);
                async move {
                    let futures = (0..10).map(|_| {
                        let m = Arc::clone(&mgr);
                        async move {
                            let _ = m
                                .call_tool("10-concurrent", "echo_tool", Some(Value::Null), None)
                                .await;
                        }
                    });
                    join_all(futures).await;
                }
            })
    });
}

fn bench_100_concurrent(c: &mut Criterion) {
    let (manager, mock) = manager_with_mock_backend("100-concurrent", Duration::from_millis(5), 200);
    mock.reset();

    c.bench_function("call_tool_100_concurrent_5ms", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| {
                let mgr = Arc::clone(&manager);
                async move {
                    let futures = (0..100).map(|_| {
                        let m = Arc::clone(&mgr);
                        async move {
                            let _ = m
                                .call_tool("100-concurrent", "echo_tool", Some(Value::Null), None)
                                .await;
                        }
                    });
                    join_all(futures).await;
                }
            })
    });
}

// ---- Parameterised concurrency sweep ----

fn bench_concurrency_sweep(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrency_sweep");
    group.sample_size(20); // fewer samples for heavier loads

    for &concurrency in &[1, 10, 50, 100, 200, 500] {
        let max_caps = concurrency; // no bottleneck
        let (manager, mock) = manager_with_mock_backend(
            &format!("sweep-{}", concurrency),
            Duration::from_millis(5),
            max_caps,
        );
        mock.reset();

        group.bench_with_input(
            BenchmarkId::from_parameter(concurrency),
            &concurrency,
            |b, &n| {
                b.to_async(tokio::runtime::Runtime::new().unwrap())
                    .iter_batched(
                        || n, // batch input: number of concurrent tasks
                        |n_tasks| {
                            let mgr = Arc::clone(&manager);
                            async move {
                                let futures = (0..n_tasks).map(|_| {
                                    let m = Arc::clone(&mgr);
                                    async move {
                                        let _ = m
                                            .call_tool(
                                                "sweep",
                                                "echo_tool",
                                                Some(Value::Null),
                                                None,
                                            )
                                            .await;
                                    }
                                });
                                join_all(futures).await;
                            }
                        },
                        BatchSize::LargeInput,
                    )
            },
        );
    }

    group.finish();
}

// ---- Semaphore backpressure benchmark ----

fn bench_semaphore_backpressure(c: &mut Criterion) {
    // Only 2 permits available, but we fire 20 concurrent calls.
    // 18 should queue (and eventually time out or wait).
    let (manager, mock) = manager_with_mock_backend(
        "backpressure",
        Duration::from_millis(50),
        2, // only 2 concurrent calls allowed
    );
    mock.reset();

    c.bench_function("call_tool_semaphore_2_permits_20_calls", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| {
                let mgr = Arc::clone(&manager);
                async move {
                    let futures = (0..20).map(|_| {
                        let m = Arc::clone(&mgr);
                        async move {
                            // We expect some to timeout (semaphore_timeout is 30s), but
                            // with only 2 permits and 50ms processing, all should succeed
                            // because processing is fast enough that permits free up.
                            let _ = m
                                .call_tool("backpressure", "echo_tool", Some(Value::Null), None)
                                .await;
                        }
                    });
                    join_all(futures).await;
                }
            })
    });
}

// ---- Error-path benchmark ----

fn bench_error_path(c: &mut Criterion) {
    let (manager, mock) =
        manager_with_mock_backend("error-backend", Duration::from_millis(10), 100);
    mock.set_force_error(true);

    c.bench_function("call_tool_all_errors", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| {
                let mgr = Arc::clone(&manager);
                async move {
                    let futures = (0..10).map(|_| {
                        let m = Arc::clone(&mgr);
                        async move {
                            let _ = m
                                .call_tool("error-backend", "error_tool", Some(Value::Null), None)
                                .await;
                        }
                    });
                    join_all(futures).await;
                }
            })
    });
}

criterion_group!(
    benches,
    bench_single_call,
    bench_10_concurrent,
    bench_100_concurrent,
    bench_concurrency_sweep,
    bench_semaphore_backpressure,
    bench_error_path,
);
criterion_main!(benches);
