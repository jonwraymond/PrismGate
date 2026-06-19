//! Registry search benchmarks — BM25, trigram, and fuzzy tiers.
//!
//! Run with: cargo bench --bench registry_search

use gatemini::registry::Registry;
use std::sync::OnceLock;

static REG: OnceLock<Registry> = OnceLock::new();

fn registry() -> &'static Registry {
    REG.get_or_init(|| {
        let mut r = Registry::default();
        // Seed realistic tool entries (simulates 50 registered tools)
        let tools = [
            ("search_tools", "search and filter MCP tools by name or description"),
            ("list_tools_meta", "list all tools with metadata including descriptions and schemas"),
            ("tool_info", "get detailed information about a specific tool including its parameters"),
            ("get_required_keys_for_tool", "get the required authentication keys for a tool"),
            ("call_tool_chain", "execute multiple tools in sequence with shared context"),
            ("register_manual", "manually register a tool with name and description"),
            ("deregister_manual", "remove a manually registered tool from the registry"),
            ("file_read", "read contents of a file from the filesystem"),
            ("file_write", "write content to a file in the filesystem"),
            ("http_get", "perform HTTP GET request to a specified URL"),
            ("http_post", "perform HTTP POST request with JSON body to a URL"),
            ("database_query", "execute a SQL query against the configured database"),
            ("redis_get", "get a value from Redis by key"),
            ("redis_set", "set a key-value pair in Redis with optional TTL"),
            ("memory_store", "store a value in the agent's persistent memory"),
            ("memory_recall", "recall a previously stored value from memory"),
            ("eval_python", "execute Python code in a sandboxed environment"),
            ("run_shell", "run a shell command and return its stdout/stderr"),
            ("git_clone", "clone a git repository to a specified directory"),
            ("git_commit", "create a git commit with the given message"),
            ("docker_ps", "list running Docker containers"),
            ("docker_logs", "fetch logs from a running Docker container"),
            ("k8s_pods", "list Kubernetes pods in a namespace"),
            ("k8s_scale", "scale a Kubernetes deployment to a specified replica count"),
            ("aws_s3_list", "list objects in an S3 bucket with optional prefix filter"),
            ("aws_s3_put", "upload a file or object to an S3 bucket"),
            ("slack_post", "post a message to a Slack channel via webhook or API"),
            ("send_email", "send an email via the configured SMTP server"),
            ("postgres_query", "execute a read-only SQL query against PostgreSQL"),
            ("mongo_find", "find documents in MongoDB matching a filter"),
            ("graphql_query", "execute a GraphQL query against a configured endpoint"),
            ("rate_limit_check", "check if a rate limit has been exceeded for a key"),
            ("cache_invalidate", "invalidate cached entries matching a pattern"),
            ("session_create", "create a new user session with TTL"),
            ("session_get", "retrieve session data by session ID"),
            ("config_get", "get a configuration value by key from config store"),
            ("config_set", "set a configuration value in the config store"),
            ("health_check", "check health status of all registered backends"),
            ("metrics_get", "retrieve current metrics from the gateway"),
            ("logs_tail", "tail recent log entries from the gateway"),
            ("trace_get", "get a trace by its ID from the tracing backend"),
            ("user_authenticate", "authenticate a user and issue a session token"),
            ("user_logout", "invalidate the current user session"),
            ("webhook_register", "register a webhook URL for event notifications"),
            ("queue_publish", "publish a message to a message queue topic"),
            ("queue_consume", "consume messages from a queue consumer group"),
            ("lock_acquire", "acquire a distributed lock with a TTL"),
            ("lock_release", "release a previously acquired distributed lock"),
            ("feature_flag_get", "get the value of a feature flag by name"),
        ];
        for (name, desc) in tools {
            r.register(name, desc, Default::default());
        }
        r
    })
}

// --- BM25 tier ---

fn bm25_term() -> impl Send + Fn() {
    let reg = registry();
    move || {
        let _ = reg.search("rate limit", Default::default());
    }
}

fn bm25_two_terms() -> impl Send + Fn() {
    let reg = registry();
    move || {
        let _ = reg.search("docker container", Default::default());
    }
}

fn bm25_no_match() -> impl Send + Fn() {
    let reg = registry();
    move || {
        let _ = reg.search("xyzzy plume", Default::default());
    }
}

// --- Trigram tier ---

fn trigram_typo() -> impl Send + Fn() {
    let reg = registry();
    move || {
        let _ = reg.search("rediz_get", Default::default());
    }
}

// --- Fuzzy tier ---

fn fuzzy_deep_miss() -> impl Send + Fn() {
    let reg = registry();
    move || {
        let _ = reg.search("postgrs_quety", Default::default());
    }
}

// --- Combinator ---

fn search_mixed() -> impl Send + Fn() {
    let reg = registry();
    move || {
        let _ = reg.search("k8s", Default::default());
    }
}

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};

pub fn bench_registry(c: &mut Criterion) {
    let mut group = c.benchmark_group("registry_search");

    group.bench_function("bm25_single_term", |b| b.iter(bm25_term()));
    group.bench_function("bm25_two_terms", |b| b.iter(bm25_two_terms()));
    group.bench_function("bm25_no_match", |b| b.iter(bm25_no_match()));
    group.bench_function("trigram_typo", |b| b.iter(trigram_typo()));
    group.bench_function("fuzzy_deep_miss", |b| b.iter(fuzzy_deep_miss()));
    group.bench_function("search_mixed", |b| b.iter(search_mixed()));

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(200);
    targets = bench_registry
}
criterion_main!(benches);
