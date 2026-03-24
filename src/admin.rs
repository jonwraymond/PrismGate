//! Optional admin API (axum HTTP server) with embedded dashboard UI.
//! Feature-gated behind `admin` cargo feature.

#[cfg(feature = "admin")]
pub mod api {
    use axum::{Json, Router, extract::State, routing::get};
    use serde::Serialize;
    use serde_json::Value;
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio::sync::Notify;
    use tower_http::services::{ServeDir, ServeFile};
    use tracing::info;

    use crate::backend::BackendManager;
    use crate::registry::ToolRegistry;
    use crate::tracker::CallTracker;

    #[derive(Clone)]
    pub struct AdminState {
        pub registry: Arc<ToolRegistry>,
        pub backend_manager: Arc<BackendManager>,
        pub tracker: Arc<CallTracker>,
    }

    pub async fn start(
        state: AdminState,
        listen: &str,
        shutdown: Arc<Notify>,
    ) -> anyhow::Result<()> {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let assets_dir = manifest_dir.join("web/dist");
        let serve_dir = if assets_dir.exists() {
            // SPA: serve static files, fallback unmatched paths to index.html
            ServeDir::new(&assets_dir)
                .not_found_service(ServeFile::new(assets_dir.join("index.html")))
        } else {
            // web/dist not built — serve legacy single-file dashboard
            ServeDir::new(manifest_dir)
                .not_found_service(ServeFile::new(manifest_dir.join("web/dashboard.html")))
        };

        let app = Router::new()
            .route("/api/health", get(health))
            .route("/api/backends", get(backends))
            .route("/api/discovery", get(discovery))
            .route("/api/recent", get(recent))
            .route("/api/stats", get(stats))
            .route("/api/topology", get(topology))
            .fallback_service(serve_dir)
            .with_state(state);

        let listener = TcpListener::bind(listen).await?;
        info!(listen = %listen, "admin dashboard: http://{}", listen);
        axum::serve(listener, app)
            .with_graceful_shutdown(async move { shutdown.notified().await })
            .await?;
        info!("admin API stopped");
        Ok(())
    }

    // ── API endpoints ───────────────────────────────────────────────────

    #[derive(Serialize)]
    struct HealthResponse {
        status: &'static str,
        total_tools: usize,
        total_backends: usize,
        backends: Vec<crate::backend::BackendStatus>,
    }

    async fn health(State(state): State<AdminState>) -> Json<HealthResponse> {
        let statuses = state.backend_manager.get_all_status();
        let all_healthy = statuses.iter().all(|s| s.available);
        Json(HealthResponse {
            status: if all_healthy { "healthy" } else { "degraded" },
            total_tools: state.registry.tool_count(),
            total_backends: state.registry.backend_count(),
            backends: statuses,
        })
    }

    async fn backends(State(state): State<AdminState>) -> Json<Vec<BackendDetail>> {
        let statuses = state.backend_manager.get_all_status();
        let details: Vec<BackendDetail> = statuses
            .into_iter()
            .map(|s| {
                let tool_count = state.registry.get_by_backend(&s.name).len();
                let memory = state.backend_manager.get_memory_stats(&s.name);
                let stderr = state
                    .backend_manager
                    .get_backend_stderr(&s.name, 20)
                    .unwrap_or_default();
                let latency = state.tracker.latency_stats(&s.name);
                BackendDetail {
                    name: s.name,
                    state: format!("{:?}", s.state),
                    available: s.available,
                    tool_count,
                    pid: memory.as_ref().map(|m| m.pid),
                    rss_mb: memory.as_ref().map(|m| m.rss_kb / 1024),
                    peak_rss_mb: memory.as_ref().map(|m| m.peak_rss_kb / 1024),
                    p50_ms: latency.as_ref().map(|l| l.p50_ms),
                    p95_ms: latency.as_ref().map(|l| l.p95_ms),
                    calls: latency.as_ref().map(|l| l.sample_count).unwrap_or(0),
                    recent_stderr: stderr,
                }
            })
            .collect();
        Json(details)
    }

    #[derive(Serialize)]
    struct BackendDetail {
        name: String,
        state: String,
        available: bool,
        tool_count: usize,
        pid: Option<u32>,
        rss_mb: Option<u64>,
        peak_rss_mb: Option<u64>,
        p50_ms: Option<f64>,
        p95_ms: Option<f64>,
        calls: u64,
        recent_stderr: Vec<String>,
    }

    #[derive(Serialize)]
    struct DiscoveryEntry {
        name: String,
        description: String,
        backend: String,
        input_schema: Value,
    }

    async fn discovery(State(state): State<AdminState>) -> Json<Vec<DiscoveryEntry>> {
        let tools = state.registry.get_all();
        let entries: Vec<DiscoveryEntry> = tools
            .into_iter()
            .map(|t| DiscoveryEntry {
                name: t.name,
                description: t.description,
                backend: t.backend_name,
                input_schema: t.input_schema,
            })
            .collect();
        Json(entries)
    }

    async fn recent(
        State(state): State<AdminState>,
    ) -> Json<Vec<crate::tracker::CallEventSummary>> {
        Json(state.tracker.recent_calls(50))
    }

    async fn stats(State(state): State<AdminState>) -> Json<crate::tracker::SessionStats> {
        Json(state.tracker.session_stats())
    }

    /// Topology data for the animated diagram.
    #[derive(Serialize)]
    struct TopologyResponse {
        daemon: DaemonInfo,
        backends: Vec<TopologyBackend>,
        recent_calls: Vec<crate::tracker::CallEventSummary>,
    }

    #[derive(Serialize)]
    struct DaemonInfo {
        total_tools: usize,
        total_backends: usize,
        status: &'static str,
        uptime_seconds: f64,
    }

    #[derive(Serialize)]
    struct TopologyBackend {
        name: String,
        state: String,
        available: bool,
        tool_count: usize,
        rss_mb: Option<u64>,
        calls: u64,
    }

    async fn topology(State(state): State<AdminState>) -> Json<TopologyResponse> {
        let statuses = state.backend_manager.get_all_status();
        let all_healthy = statuses.iter().all(|s| s.available);
        let session_stats = state.tracker.session_stats();

        let backends: Vec<TopologyBackend> = statuses
            .into_iter()
            .map(|s| {
                let tool_count = state.registry.get_by_backend(&s.name).len();
                let memory = state.backend_manager.get_memory_stats(&s.name);
                let latency = state.tracker.latency_stats(&s.name);
                TopologyBackend {
                    name: s.name,
                    state: format!("{:?}", s.state),
                    available: s.available,
                    tool_count,
                    rss_mb: memory.map(|m| m.rss_kb / 1024),
                    calls: latency.map(|l| l.sample_count).unwrap_or(0),
                }
            })
            .collect();

        Json(TopologyResponse {
            daemon: DaemonInfo {
                total_tools: state.registry.tool_count(),
                total_backends: state.registry.backend_count(),
                status: if all_healthy { "healthy" } else { "degraded" },
                uptime_seconds: session_stats.uptime_seconds,
            },
            backends,
            recent_calls: state.tracker.recent_calls(20),
        })
    }
}
