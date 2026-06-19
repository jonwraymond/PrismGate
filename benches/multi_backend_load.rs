//! Multi-backend load benchmark.
//!
//! Spins up multiple mock backends with different `max_concurrent_calls` limits
//! and fires a sustained stream of calls across all of them to measure aggregate
//! throughput, fairness, and per-backend saturation behaviour.
//!
//! Run with: `cargo bench --bench multi_backend_load`

use std::sync::Arc;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use futures_util::future::join_all;
use serde_json::Value;

use gatemini::backend::BackendManager;
use gatemini::config::{BackendConfig, InstanceMode, PoolConfig, RetryConfig, Transport};
use gatemini::registry::ToolRegistry;
use gatemini::testutil::MockBackend;

fn build_manager_with_backends(
    specs: &[(&'static str, Duration, u32)],
) -> (Arc<BackendManager>, Vec<(&'static str, Arc<MockBackend>)>) {
    let manager = BackendManager::new();
    let mut mocks = Vec::new();

    for &(name, delay, max_conc) in specs {
        let mock = Arc::new(MockBackend::new(name, delay));

        let config = BackendConfig {
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
            max_concurrent_calls: Some(max_conc),
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

        manager.register_virtual_backend(name, Arc::clone(&mock) as Arc<dyn gatemini::backend::Backend>);
        {
            let mut configs = manager.configs.try_write_for(Duration::from_secs(1)).unwrap();
            configs.insert(name.to_string(), config);
        }
        if max_conc > 0 {
            manager.call_semaphores.insert(
                name.to_string(),
                Arc::new(tokio::sync::Semaphore::new(max_conc as usize)),
            );
        }
        mocks.push((name, mock));
    }

    (manager, mocks)
}

// ---- 3-backend mix: fast + slow + medium ----

fn bench_multi_backend_mix(c: &mut Criterion) {
    let (manager, _mocks) = build_manager_with_backends(&[
        ("fast-backend", Duration::from_millis(1), 100),
        ("slow-backend", Duration::from_millis(20), 50),
        ("medium-backend", Duration::from_millis(5), 75),
    ]);

    c.bench_function("multi_backend_mix_50_calls", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| {
                let mgr = Arc::clone(&manager);
                async move {
                    let futures = (0..50).map(|i| {
                        let m = Arc::clone(&mgr);
                        let backend = match i % 3 {
                            0 => "fast-backend",
                            1 => "slow-backend",
                            _ => "medium-backend",
                        };
                        async move {
                            let _ = m
                                .call_tool(backend, "echo_tool", Some(Value::Null), None)
                                .await;
                        }
                    });
                    join_all(futures).await;
                }
            })
    });
}

// ---- Saturate a backend with exactly its max_concurrent_calls ----

fn bench_saturation(c: &mut Criterion) {
    let (manager, mock) = build_manager_with_backends(&[("saturated", Duration::from_millis(10), 10)]);
    mock[0].1.reset(); // reset counters

    c.bench_function("saturation_10_permits_20_calls", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| {
                let mgr = Arc::clone(&manager);
                async move {
                    // 20 calls, only 10 permits — second batch queues
                    let futures = (0..20).map(|_| {
                        let m = Arc::clone(&mgr);
                        async move {
                            let _ = m
                                .call_tool("saturated", "echo_tool", Some(Value::Null), None)
                                .await;
                        }
                    });
                    join_all(futures).await;
                }
            })
    });
}

// ---- Parameterised throughput sweep ----

fn bench_throughput_sweep(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput_sweep");
    group.sample_size(10);

    for &num_calls in &[10, 50, 100, 200, 500] {
        let (manager, _) = build_manager_with_backends(&[
            ("t-fast", Duration::from_millis(1), num_calls),
            ("t-slow", Duration::from_millis(5), num_calls),
        ]);

        group.bench_with_input(
            BenchmarkId::new("calls", num_calls),
            &num_calls,
            |b, &n| {
                b.to_async(tokio::runtime::Runtime::new().unwrap())
                    .iter_batched(
                        || n,
                        |n_tasks| {
                            let mgr = Arc::clone(&manager);
                            async move {
                                let futures = (0..n_tasks).map(|i| {
                                    let m = Arc::clone(&mgr);
                                    let backend = if i % 2 == 0 { "t-fast" } else { "t-slow" };
                                    async move {
                                        let _ = m
                                            .call_tool(backend, "echo_tool", Some(Value::Null), None)
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

criterion_group!(
    benches,
    bench_multi_backend_mix,
    bench_saturation,
    bench_throughput_sweep,
);
criterion_main!(benches);
