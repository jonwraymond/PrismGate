mod admin;
mod backend;
mod cache;
mod cli;
mod config;
#[cfg(feature = "semantic")]
mod embeddings;
#[cfg(test)]
mod integration_inventory;
mod ipc;
#[cfg(test)]
mod mcp_compliance_tests;
mod prompts;
mod registry;
mod resources;
mod sandbox;
mod secrets;
mod server;
#[cfg(test)]
mod testutil;
mod tools;

use anyhow::Result;
use clap::Parser;
use rmcp::{ServiceExt, transport::stdio};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

/// Everything produced by shared initialization, ready for either direct or daemon mode.
pub struct InitializedGateway {
    pub registry: Arc<registry::ToolRegistry>,
    pub backend_manager: Arc<backend::BackendManager>,
    pub cache_path: PathBuf,
    pub config: config::Config,
    pub shutdown_notify: Arc<tokio::sync::Notify>,
}

/// Shared initialization: config, tracing, secrets, registry, backends, health, watcher, admin.
///
/// This is extracted from the original monolithic main() so both direct mode and daemon mode
/// can reuse it without duplication.
pub async fn initialize(config_path: &Path) -> Result<InitializedGateway> {
    // Load ~/.env into process environment (once, before any concurrent work).
    config::load_dotenv();

    // Ensure ~/.prismgate directory exists
    let prismgate_home = cli::prismgate_home();
    if !prismgate_home.exists() {
        std::fs::create_dir_all(&prismgate_home)?;
        // Note: tracing not initialized yet, so use eprintln
        eprintln!(
            "created prismgate home directory: {}",
            prismgate_home.display()
        );
    }

    // Load config (env var expansion + YAML parse)
    let mut config = config::Config::load(config_path)?;

    // Initialize tracing (logs to stderr so stdio transport is clean)
    let filter = EnvFilter::try_new(&config.log_level).unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    // Resolve secrets (requires tokio runtime + network for BWS SDK)
    config.resolve_secrets_async().await?;

    info!(
        config_path = %config_path.display(),
        backends = config.backends.len(),
        "gatemini starting"
    );

    // Create shared state
    let registry = {
        #[cfg(feature = "semantic")]
        {
            // Direct HuggingFace model downloads to ~/.prismgate/models/
            let models_dir = config
                .semantic
                .as_ref()
                .and_then(|s| s.cache_dir.clone())
                .unwrap_or_else(|| cli::prismgate_home().join("models"));
            if !models_dir.exists() {
                std::fs::create_dir_all(&models_dir)?;
            }
            // SAFETY: No concurrent env reads at this point — tokio worker threads
            // exist but no user tasks have been spawned yet.
            unsafe { std::env::set_var("HF_HOME", &models_dir) };

            let model_path = config
                .semantic
                .as_ref()
                .map(|s| s.model_path.as_str())
                .unwrap_or("minishlab/potion-base-8M");

            match embeddings::EmbeddingIndex::new(model_path) {
                Ok(index) => {
                    info!("semantic search enabled");
                    registry::ToolRegistry::new_with_embeddings(index)
                }
                Err(e) => {
                    warn!(error = %e, "failed to load embedding model, falling back to BM25-only");
                    registry::ToolRegistry::new()
                }
            }
        }
        #[cfg(not(feature = "semantic"))]
        {
            registry::ToolRegistry::new()
        }
    };
    let backend_manager = backend::BackendManager::new_with_config(&config.health);

    // Load tool cache for instant availability before backends connect
    let cache_path = config
        .cache_path
        .clone()
        .unwrap_or_else(cache::default_cache_path);
    let config_backend_names: Vec<String> = config.backends.keys().cloned().collect();
    let cached = cache::load(&cache_path, &registry, &config_backend_names).await;
    if cached > 0 {
        info!(tools = cached, "tools available from cache");
    }

    // Start all backends in the background
    {
        let manager = Arc::clone(&backend_manager);
        let reg = Arc::clone(&registry);
        let cfg = config.clone();
        let cp = cache_path.clone();
        tokio::spawn(async move {
            if let Err(e) = manager.start_all(&cfg, &reg).await {
                tracing::error!(error = %e, "backend startup failed");
            }
            info!(
                tools = reg.tool_count(),
                backends = reg.backend_count(),
                "tool discovery complete"
            );
            cache::save(&cp, &reg).await;
        });
    }

    // Shared config for hot-reload
    let shared_config = Arc::new(arc_swap::ArcSwap::from_pointee(config.clone()));

    // Start health checker in background
    let shutdown_notify = Arc::new(tokio::sync::Notify::new());
    {
        let manager = Arc::clone(&backend_manager);
        let reg = Arc::clone(&registry);
        let health_config = config.health.clone();
        let shutdown = Arc::clone(&shutdown_notify);
        let cp = cache_path.clone();
        tokio::spawn(async move {
            backend::health::run_health_checker(manager, reg, health_config, shutdown, cp).await;
        });
    }

    // Start config file watcher in background
    {
        let config_path = config_path.to_path_buf();
        let shared = Arc::clone(&shared_config);
        let mgr = Arc::clone(&backend_manager);
        let reg = Arc::clone(&registry);
        let cp = cache_path.clone();
        let shutdown = Arc::clone(&shutdown_notify);
        tokio::spawn(async move {
            config::watch_config(config_path, shared, mgr, reg, cp, shutdown).await;
        });
    }

    // Start admin API in background (if enabled)
    #[cfg(feature = "admin")]
    if config.admin.enabled {
        let admin_state = admin::api::AdminState {
            registry: Arc::clone(&registry),
            backend_manager: Arc::clone(&backend_manager),
        };
        let listen = config.admin.listen.clone();
        let shutdown_admin = Arc::clone(&shutdown_notify);
        tokio::spawn(async move {
            if let Err(e) = admin::api::start(admin_state, &listen, shutdown_admin).await {
                tracing::error!(error = %e, "admin API failed");
            }
        });
    }

    Ok(InitializedGateway {
        registry,
        backend_manager,
        cache_path,
        config,
        shutdown_notify,
    })
}

/// Run in direct (legacy) mode: single Claude Code session over stdio.
async fn run_direct(gw: InitializedGateway) -> Result<()> {
    let sandbox_semaphore = Arc::new(tokio::sync::Semaphore::new(
        gw.config.sandbox.max_concurrent_sandboxes as usize,
    ));
    let server = server::GateminiServer::new(
        Arc::clone(&gw.registry),
        Arc::clone(&gw.backend_manager),
        gw.cache_path.clone(),
        gw.config.allow_runtime_registration,
        gw.config.max_dynamic_backends,
        sandbox_semaphore,
    );

    info!("starting MCP stdio server (direct mode)");
    let service = server.serve(stdio()).await?;

    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;

        tokio::select! {
            result = service.waiting() => {
                if let Err(e) = result {
                    warn!(error = %e, "MCP service exited with error");
                }
            }
            _ = sigterm.recv() => {
                info!("received SIGTERM");
            }
            _ = sigint.recv() => {
                info!("received SIGINT");
            }
        }
    }

    #[cfg(not(unix))]
    {
        service.waiting().await?;
    }

    info!("shutting down");
    gw.shutdown_notify.notify_waiters();
    gw.backend_manager.stop_all().await;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = cli::Cli::parse();

    match (&cli.command, cli.direct) {
        // Explicit direct mode
        (None, true) => {
            let gw = initialize(&cli.config).await?;
            run_direct(gw).await
        }

        // Daemon mode: gatemini serve
        (Some(cli::Command::Serve { socket }), _) => {
            let gw = initialize(&cli.config).await?;
            ipc::daemon::run(gw, socket.clone()).await
        }

        // Status check
        (Some(cli::Command::Status), _) => ipc::status::run(),

        // Stop daemon
        (Some(cli::Command::Stop), _) => ipc::stop::run(),

        // Default: proxy mode (auto-start daemon if needed)
        (None, false) => ipc::proxy::run(&cli.config).await,
    }
}
