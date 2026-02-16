//! Optional admin API (axum HTTP server).
//! Feature-gated behind `admin` cargo feature.

#[cfg(feature = "admin")]
pub mod api {
    use axum::{extract::State, routing::get, Json, Router};
    use serde::Serialize;
    use serde_json::Value;
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio::sync::Notify;
    use tracing::info;

    use crate::backend::BackendManager;
    use crate::registry::ToolRegistry;

    #[derive(Clone)]
    pub struct AdminState {
        pub registry: Arc<ToolRegistry>,
        pub backend_manager: Arc<BackendManager>,
    }

    pub async fn start(state: AdminState, listen: &str, shutdown: Arc<Notify>) -> anyhow::Result<()> {
        let app = Router::new()
            .route("/api/health", get(health))
            .route("/api/backends", get(backends))
            .route("/api/discovery", get(discovery))
            .with_state(state);

        let listener = TcpListener::bind(listen).await?;
        info!(listen = %listen, "admin API started");
        axum::serve(listener, app)
            .with_graceful_shutdown(async move { shutdown.notified().await })
            .await?;
        info!("admin API stopped");
        Ok(())
    }

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

    async fn backends(
        State(state): State<AdminState>,
    ) -> Json<Vec<crate::backend::BackendStatus>> {
        Json(state.backend_manager.get_all_status())
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
}
