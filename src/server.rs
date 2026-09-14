//! Public MCP server surface for the shared gateway.

use std::path::PathBuf;
use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars,
    service::RequestContext,
    tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::Value;

use crate::access::AccessControlConfig;
use crate::backend::BackendManager;
use crate::registry::ToolRegistry;

use tokio::sync::Semaphore;
use tracing::instrument;

// --- Parameter structs for each meta-tool ---

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadResultParams {
    /// Handle returned by call_tool_chain on this connection. Expires after 30 minutes or eviction.
    pub handle: String,
    /// UTF-8 byte offset (default 0).
    pub offset: Option<usize>,
    /// Page byte limit, clamped to 4..16384 (default 4096).
    pub limit: Option<usize>,
    /// Optional case-sensitive literal search starting at offset.
    pub query: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PurgeSessionParams {
    /// Must be true. Clears shared gateway tracker state for ALL clients.
    /// Does not erase client conversation context or backend-owned data.
    pub confirm: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RegisterManualParams {
    /// The call template for the manual backend endpoint.
    pub manual_call_template: Value,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeregisterManualParams {
    /// The name of the manual to deregister.
    pub manual_name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchToolsParams {
    /// A natural language description of the task.
    pub task_description: String,
    /// Stable agent/context ID when multiple agents share this connection. Reuse the same ID across search_tools and tool_info; omitted IDs share this session's budget.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Maximum number of results to return.
    #[serde(default = "default_limit")]
    pub limit: u32,
    /// Return brief results (name, backend, first sentence). Default: true. Set false for full descriptions.
    #[serde(default = "default_true")]
    pub brief: bool,
    /// Optional tag to filter results. Only tools with this tag are returned.
    #[serde(default)]
    pub tag: Option<String>,
}

fn default_limit() -> u32 {
    10
}

fn default_true() -> bool {
    true
}

fn default_page_size() -> u32 {
    50
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ToolInfoParams {
    /// Name of the tool to get information for.
    pub tool_name: String,
    /// Stable agent/context ID, matching search_tools. Omit for a session-local budget.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Detail level: "brief" returns name, backend, first-sentence description, parameter names (~200 tokens). "full" returns complete schema (~10k tokens). Default: "brief".
    #[serde(default = "default_detail")]
    pub detail: String,
}

fn default_detail() -> String {
    "brief".to_string()
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListToolsMetaParams {
    /// Pagination cursor from a previous response.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Number of tools per page (default: 50).
    #[serde(default = "default_page_size")]
    pub page_size: u32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RequiredKeysParams {
    /// Name of the tool to get required variables for.
    pub tool_name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CallToolChainParams {
    /// TypeScript code to execute with access to all registered tools.
    pub code: String,
    /// Optional timeout in milliseconds (default: 30000).
    pub timeout: Option<u64>,
    /// Optional maximum output size in UTF-8 bytes (default: 200000).
    pub max_output_size: Option<usize>,
    /// Single-query alias for structured retrieval (default 8 hits / 16000 bytes).
    /// No matches return []; explicit retrieval takes precedence.
    #[serde(default)]
    pub intent: Option<String>,
    /// Structured result search: queries, top_k, neighbors and max_bytes.
    /// Returns a bounded JSON evidence array; no matches return [].
    #[serde(default)]
    pub retrieval: Option<crate::tools::retrieval::RetrievalOptions>,
}

#[cfg(feature = "session-store")]
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SessionSearchParams {
    /// FTS5 match query over persisted session events (categories, tool names, payload summaries, outcomes). Empty query with resume=true returns the most recent events instead.
    #[serde(default)]
    pub query: Option<String>,
    /// Return the most recent events for this session instead of a ranked search.
    #[serde(default)]
    pub resume: bool,
    /// Compact resume card: open handles, recent tools, decisions, constraints.
    #[serde(default)]
    pub card: bool,
    /// Optional category filter for resume mode (tool, backend, file).
    #[serde(default)]
    pub category: Option<String>,
    /// Maximum hits to return (default 20, max 200).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[cfg(feature = "session-store")]
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SessionNoteParams {
    /// Kind of durable fact: decision, constraint, or note.
    pub kind: String,
    /// Short payload to persist. Keep it under a few hundred characters.
    pub text: String,
    /// Optional label for notes.
    #[serde(default)]
    pub name: Option<String>,
}

/// The MCP server exposed to Claude Code over stdio.
#[derive(Clone)]
pub struct PrismGateServer {
    pub registry: Arc<ToolRegistry>,
    pub backend_manager: Arc<BackendManager>,
    pub tracker: Arc<crate::tracker::CallTracker>,
    pub cache_path: PathBuf,
    pub allow_runtime_registration: bool,
    pub max_dynamic_backends: usize,
    /// Limits concurrent V8 sandbox executions to prevent OOM.
    pub sandbox_semaphore: Arc<Semaphore>,
    /// Session ID for dedicated instance pool routing. None for direct mode legacy.
    pub session_id: Option<u64>,
    /// Shared by server clones, never by unrelated connections.
    discovery_guard: Arc<crate::flood_guard::FloodGuard>,
    pub(crate) result_store: Arc<crate::result_store::ResultStore>,
    /// Output processing configuration (auto-chunking, smart truncation).
    pub output_config: crate::config::OutputConfig,
    pub session_store: crate::session::SessionEventStore,
    /// Access control configuration for tool-level RBAC.
    pub access_control: AccessControlConfig,
    /// TTL-based cache for generated catalog text (K2).
    pub catalog_cache: crate::resources::CatalogCache,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl PrismGateServer {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        registry: Arc<ToolRegistry>,
        backend_manager: Arc<BackendManager>,
        tracker: Arc<crate::tracker::CallTracker>,
        cache_path: PathBuf,
        allow_runtime_registration: bool,
        max_dynamic_backends: usize,
        sandbox_semaphore: Arc<Semaphore>,
        session_id: Option<u64>,
        output_config: crate::config::OutputConfig,
        access_control: AccessControlConfig,
    ) -> Self {
        Self {
            registry,
            backend_manager,
            tracker,
            cache_path,
            allow_runtime_registration,
            max_dynamic_backends,
            sandbox_semaphore,
            session_id,
            discovery_guard: Arc::new(crate::flood_guard::FloodGuard::default()),
            result_store: Arc::new(crate::result_store::ResultStore::default()),
            output_config,
            session_store: crate::session::SessionEventStore::default(),
            access_control,
            catalog_cache: crate::resources::CatalogCache::default(),
            tool_router: Self::tool_router(),
        }
    }
    #[cfg(feature = "session-store")]
    fn session_key(&self) -> String {
        self.session_id
            .map(|id| format!("session-{id}"))
            .unwrap_or_else(|| "direct".to_string())
    }

    /// Send a progress notification for long-running tool calls (R7).
    /// Reports execution progress without leaking raw backend payloads.
    #[allow(dead_code)]
    fn notify_progress(&self, tool_name: &str, message: &str) {
        tracing::info!(tool = %tool_name, progress = %message, "progress notification");
    }
}

#[tool_router]
impl PrismGateServer {
    #[tool(
        description = "Read or search a retained raw tool result by handle without rerunning its backend. Session-local; expires after 30 minutes or FIFO eviction. Returns a bounded UTF-8 page and next_offset. Literal query searches from offset."
    )]
    async fn read_result(
        &self,
        Parameters(params): Parameters<ReadResultParams>,
    ) -> Result<CallToolResult, McpError> {
        let offset = params.offset.unwrap_or(0);
        let limit = params.limit.unwrap_or(4096);
        let page = if let Some(query) = params.query.as_deref() {
            self.result_store
                .search(&params.handle, query, offset, limit)
        } else {
            self.result_store.read(&params.handle, offset, limit)
        };
        match page {
            Some(page) => Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string(&page)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?,
            )])),
            None => Ok(CallToolResult::error(vec![Content::text(
                "Result unavailable (expired, evicted, unknown handle, invalid offset, or no match).",
            )])),
        }
    }

    #[tool(
        description = "Read a file server-side and return a bounded page plus metadata. Raw file text is indexed for session_search instead of dumped into context. Use instead of host Read for analysis."
    )]
    async fn execute_file(
        &self,
        Parameters(params): Parameters<crate::tools::execute_file::ExecuteFileParams>,
    ) -> Result<CallToolResult, McpError> {
        let (session_store, session_key);
        #[cfg(feature = "session-store")]
        {
            session_store = Some(&self.session_store);
            let key = self.session_key();
            session_key = Some(key);
        }
        #[cfg(not(feature = "session-store"))]
        {
            session_store = None;
            session_key = None::<String>;
        }
        match crate::tools::execute_file::handle_execute_file(
            &params,
            &self.registry,
            &self.result_store,
            session_store,
            session_key.as_deref(),
        )
        .await
        {
            Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("{e:#}"))])),
        }
    }

    #[tool(
        description = "Fetch a URL server-side with SSRF protection, extract readable text, retain it behind a result handle, and index it for session_search. Never returns raw HTML."
    )]
    async fn fetch_and_index(
        &self,
        Parameters(params): Parameters<crate::tools::fetch::FetchAndIndexParams>,
    ) -> Result<CallToolResult, McpError> {
        let (session_store, session_key);
        #[cfg(feature = "session-store")]
        {
            session_store = Some(&self.session_store);
            let key = self.session_key();
            session_key = Some(key);
        }
        #[cfg(not(feature = "session-store"))]
        {
            session_store = None;
            session_key = None::<String>;
        }
        match crate::tools::fetch::handle_fetch_and_index(
            &params,
            &self.registry,
            &self.result_store,
            session_store,
            session_key.as_deref(),
        )
        .await
        {
            Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!("{e:#}"))])),
        }
    }

    #[tool(
        description = "Explicit clean slate: clears shared gateway call history, usage, latency and context statistics for ALL clients. Requires confirm=true. Preserves backend processes, health, tools and configuration. Does not erase client conversations or backend-owned memory. Calls completing afterwards count as new activity."
    )]
    async fn purge_session(
        &self,
        Parameters(params): Parameters<PurgeSessionParams>,
    ) -> Result<CallToolResult, McpError> {
        if !params.confirm {
            return Ok(CallToolResult::error(vec![Content::text(
                "Purge requires confirm=true; shared tracker state for all clients will be cleared.",
            )]));
        }
        self.tracker.reset();
        #[cfg(feature = "session-store")]
        {
            let key = self.session_key();
            let _ = self.session_store.purge_session(&key).await;
        }
        crate::cache::save(&self.cache_path, &self.registry, Some(&self.tracker)).await;
        Ok(CallToolResult::success(vec![Content::text(
            "Shared gateway history and statistics purged. Backend runtime and health preserved. Client conversation context and backend-owned memory are unchanged.",
        )]))
    }

    #[tool(description = "Registers a new tool provider by providing its call template.")]
    async fn register_manual(
        &self,
        Parameters(params): Parameters<RegisterManualParams>,
    ) -> Result<CallToolResult, McpError> {
        if !self.allow_runtime_registration {
            return Ok(CallToolResult::error(vec![Content::text(
                "Runtime registration is disabled (allow_runtime_registration: false in config).",
            )]));
        }

        let result = crate::tools::register::handle_register(
            &self.backend_manager,
            &self.registry,
            params.manual_call_template,
            self.max_dynamic_backends,
        )
        .await;

        match result {
            Ok(msg) => {
                // Save cache after adding a backend
                let reg = Arc::clone(&self.registry);
                let cp = self.cache_path.clone();
                tokio::spawn(async move { crate::cache::save(&cp, &reg, None).await });
                Ok(CallToolResult::success(vec![Content::text(msg)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "{:#}",
                e
            ))])),
        }
    }

    #[tool(description = "Deregisters a tool provider from the gateway.")]
    async fn deregister_manual(
        &self,
        Parameters(params): Parameters<DeregisterManualParams>,
    ) -> Result<CallToolResult, McpError> {
        if !self.allow_runtime_registration {
            return Ok(CallToolResult::error(vec![Content::text(
                "Runtime registration is disabled (allow_runtime_registration: false in config).",
            )]));
        }

        let result = crate::tools::register::handle_deregister(
            &self.backend_manager,
            &self.registry,
            &params.manual_name,
        )
        .await;

        match result {
            Ok(msg) => {
                // Save cache after removing a backend
                let reg = Arc::clone(&self.registry);
                let cp = self.cache_path.clone();
                tokio::spawn(async move { crate::cache::save(&cp, &reg, None).await });
                Ok(CallToolResult::success(vec![Content::text(msg)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "{:#}",
                e
            ))])),
        }
    }

    #[tool(
        description = "Searches for relevant tools based on a task description. Covers: web search (tavily, exa, zai), code intelligence (auggie, serena, octocode), browser automation (playwright, chrome-devtools), AI models (cerebras, pal, minimax), databases (supabase), file processing (repomix, firecrawl), docs (context7, deepwiki, ref), and more. Default: brief=true for compact results."
    )]
    #[instrument(skip(self, params), fields(query_len = params.task_description.len(), brief = params.brief, limit = params.limit))]
    async fn search_tools(
        &self,
        Parameters(params): Parameters<SearchToolsParams>,
    ) -> Result<CallToolResult, McpError> {
        let decision = self
            .discovery_guard
            .check(self.session_id.unwrap_or(0), params.agent_id.as_deref());
        if let crate::flood_guard::Decision::HardBlocked { .. } = decision {
            return Ok(CallToolResult::error(vec![Content::text(
                decision.notice().unwrap(),
            )]));
        }
        let filter_tags: Option<Vec<String>> = params.tag.map(|t| vec![t]);
        let filter_ref = filter_tags.as_deref();
        let tracker_ref = Some(self.tracker.as_ref());

        let json = if params.brief {
            let results = crate::tools::discovery::handle_search_brief(
                &self.registry,
                &params.task_description,
                params.limit,
                filter_ref,
                tracker_ref,
                decision,
            )
            .map_err(|_| McpError::internal_error("Discovery flood guard blocked", None))?;
            serde_json::to_string_pretty(&results)
        } else {
            let results = crate::tools::discovery::handle_search(
                &self.registry,
                &params.task_description,
                params.limit,
                filter_ref,
                tracker_ref,
                decision,
            )
            .map_err(|_| McpError::internal_error("Discovery flood guard blocked", None))?;
            serde_json::to_string_pretty(&results)
        }
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let mut content = vec![Content::text(json)];
        if let Some(notice) = decision.notice() {
            content.push(Content::text(notice));
        }
        Ok(CallToolResult::success(content))
    }

    #[tool(description = "Returns a list of all tool names currently registered.")]
    async fn list_tools_meta(
        &self,
        Parameters(params): Parameters<ListToolsMetaParams>,
    ) -> Result<CallToolResult, McpError> {
        let (names, next_cursor) = crate::tools::discovery::handle_list_paginated(
            &self.registry,
            params.cursor.as_deref(),
            params.page_size,
        );
        let result = serde_json::json!({
            "tools": names,
            "next_cursor": next_cursor,
        });
        let json = serde_json::to_string_pretty(&result)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Get complete information about a specific tool including its input schema."
    )]
    #[instrument(skip(self, params), fields(tool = %params.tool_name))]
    async fn tool_info(
        &self,
        Parameters(params): Parameters<ToolInfoParams>,
    ) -> Result<CallToolResult, McpError> {
        let decision = self
            .discovery_guard
            .check(self.session_id.unwrap_or(0), params.agent_id.as_deref());
        if let crate::flood_guard::Decision::HardBlocked { .. } = decision {
            return Ok(CallToolResult::error(vec![Content::text(
                decision.notice().unwrap(),
            )]));
        }
        let json = if params.detail == "full" && decision == crate::flood_guard::Decision::Normal {
            let result =
                crate::tools::discovery::handle_tool_info(&self.registry, &params.tool_name);
            match result {
                Some(info) => serde_json::to_string_pretty(&info)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?,
                None => {
                    return Ok(CallToolResult::error(vec![Content::text(format!(
                        "Tool '{}' not found",
                        params.tool_name
                    ))]));
                }
            }
        } else {
            // Default: brief mode
            let result =
                crate::tools::discovery::handle_tool_info_brief(&self.registry, &params.tool_name);
            match result {
                Some(info) => serde_json::to_string_pretty(&info)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?,
                None => {
                    return Ok(CallToolResult::error(vec![Content::text(format!(
                        "Tool '{}' not found. Use tool_info with detail=\"full\" for complete schema.",
                        params.tool_name
                    ))]));
                }
            }
        };
        let mut content = vec![Content::text(json)];
        if let Some(notice) = decision.notice() {
            content.push(Content::text(notice));
        }
        Ok(CallToolResult::success(content))
    }

    #[tool(description = "Get required environment variables for a registered tool.")]
    async fn get_required_keys_for_tool(
        &self,
        Parameters(params): Parameters<RequiredKeysParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = crate::tools::discovery::handle_required_keys_async(
            &self.registry,
            &self.backend_manager,
            &params.tool_name,
        )
        .await;
        match result {
            Some(keys) => {
                let json = serde_json::to_string_pretty(&keys)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            None => Ok(CallToolResult::error(vec![Content::text(format!(
                "Tool '{}' not found",
                params.tool_name
            ))])),
        }
    }

    #[cfg(feature = "session-store")]
    #[tool(
        description = "Search the persistent session event store (SQLite FTS5). Use query for ranked BM25 search. Use resume=true for recent events, card=true for a compact resume card (open handles, decisions, constraints). Survives context compaction. Fetch retained payloads with read_result."
    )]
    async fn session_search(
        &self,
        Parameters(params): Parameters<SessionSearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let limit = params.limit.unwrap_or(20).clamp(1, 200);
        let key = self.session_key();
        if params.card {
            return match self.session_store.resume_card(&key).await {
                Ok(card) => {
                    let json = serde_json::to_string_pretty(&card)
                        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                    Ok(CallToolResult::success(vec![Content::text(json)]))
                }
                Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                    "Resume card failed: {e:#}"
                ))])),
            };
        }
        let result = if params.resume {
            self.session_store
                .resume_summary(&key, params.category.as_deref(), limit)
                .await
        } else {
            let query = params.query.unwrap_or_default();
            if query.is_empty() {
                return Ok(CallToolResult::error(vec![Content::text(
                    "Provide a query, or set resume=true for recent events.",
                )]));
            }
            self.session_store.search(&query, limit).await
        };
        match result {
            Ok(hits) => {
                let json = serde_json::to_string_pretty(&hits)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Session search failed: {e:#}"
            ))])),
        }
    }

    #[cfg(feature = "session-store")]
    #[tool(
        description = "Persist a durable session fact that should survive compaction: kind=decision|constraint|note. Do not store raw tool output here — use result handles. After compact, call session_search(resume=true) or session_search(card=true)."
    )]
    async fn session_note(
        &self,
        Parameters(params): Parameters<SessionNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let key = self.session_key();
        let event = match params.kind.as_str() {
            "decision" => crate::session::extract::decision_event(&key, &params.text),
            "constraint" => crate::session::extract::constraint_event(&key, &params.text),
            "note" => crate::session::extract::note_event(
                &key,
                params.name.as_deref().unwrap_or("note"),
                &params.text,
            ),
            other => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Unknown kind '{other}'. Use decision, constraint, or note."
                ))]));
            }
        };
        match self.session_store.record(event).await {
            Ok(()) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Recorded {} for session {key}.",
                params.kind
            ))])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Failed to record session note: {e:#}"
            ))])),
        }
    }

    #[tool(
        description = "Execute TypeScript code with direct access to all registered tools as hierarchical functions (e.g., manual.tool()). IMPORTANT: the tool result is the value your code returns. `console.log(...)` output is not returned; if you do not return a value, the result is usually `null`."
    )]
    #[instrument(skip(self, params), fields(code_len = params.code.len(), has_intent = params.intent.is_some()))]
    async fn call_tool_chain(
        &self,
        Parameters(params): Parameters<CallToolChainParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = crate::tools::sandbox::handle_call_tool_chain_with_store(
            &self.registry,
            &self.backend_manager,
            &params.code,
            params.timeout,
            params.max_output_size,
            &self.sandbox_semaphore,
            self.session_id,
            params.intent.as_deref(),
            &self.output_config,
            Some(self.tracker.as_ref()),
            params.retrieval.as_ref(),
            Some(self.result_store.as_ref()),
            &self.access_control,
        )
        .await;

        match result {
            Ok(output) => {
                #[cfg(feature = "session-store")]
                {
                    let key = self.session_key();
                    let mut event = crate::session::extract::tool_call_event(
                        &key,
                        "call_tool_chain",
                        "sandbox",
                        "ok",
                    )
                    .with_bytes(output.len() as u64, output.len() as u64);
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&output)
                        && let Some(handle) = value.get("result_handle").and_then(|v| v.as_str())
                    {
                        let total = value
                            .get("total_bytes")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(output.len() as u64);
                        event = crate::session::extract::handle_event(
                            &key,
                            handle,
                            total,
                            "call_tool_chain",
                        );
                        if let Some(raw) = self.result_store.raw(handle) {
                            let indexed = crate::session::overflow::index_overflow(
                                &self.session_store,
                                &key,
                                handle,
                                &raw,
                                "call_tool_chain",
                                params.intent.as_deref(),
                            )
                            .await;
                            let mut pointer = value.clone();
                            pointer["sections"] = serde_json::json!(indexed.sections);
                            pointer["try_also"] = serde_json::json!(indexed.try_also);
                            pointer["preview"] = serde_json::json!(indexed.preview);
                            let serialized = serde_json::to_string(&pointer).unwrap_or(output);
                            let _ = self.session_store.record(event).await;
                            return Ok(CallToolResult::success(vec![Content::text(serialized)]));
                        }
                    }
                    let _ = self.session_store.record(event).await;
                }
                Ok(CallToolResult::success(vec![Content::text(output)]))
            }
            Err(e) => {
                #[cfg(feature = "session-store")]
                {
                    let event = crate::session::extract::tool_call_event(
                        &self.session_key(),
                        "call_tool_chain",
                        "sandbox",
                        "error",
                    );
                    let _ = self.session_store.record(event).await;
                }
                Ok(CallToolResult::error(vec![Content::text(format!(
                    "{:#}",
                    e
                ))]))
            }
        }
    }
}

#[cfg(test)]
mod flood_tests {
    use super::*;
    use crate::testutil::{MockBackend, insert_mock};
    use std::time::Duration;

    #[tokio::test]
    async fn discovery_budget_is_shared_across_tools_not_agents() {
        let registry = ToolRegistry::new();
        let manager = BackendManager::new();
        insert_mock(
            &manager,
            &registry,
            &MockBackend::new("guard", Duration::ZERO),
        )
        .await;
        let mut server = PrismGateServer::new(
            registry,
            manager,
            Arc::new(crate::tracker::CallTracker::new()),
            PathBuf::new(),
            false,
            0,
            Arc::new(Semaphore::new(1)),
            Some(7),
            Default::default(),
            Default::default(),
        );
        server.discovery_guard = Arc::new(crate::flood_guard::FloodGuard::new(
            1,
            3,
            Duration::from_secs(60),
        ));
        let search = |agent: &str, brief| {
            Parameters(SearchToolsParams {
                task_description: "returns".into(),
                agent_id: Some(agent.into()),
                limit: 10,
                brief,
                tag: None,
            })
        };
        let first = server.search_tools(search("a", true)).await.unwrap();
        assert_ne!(first.is_error, Some(true));
        let data: Value = serde_json::from_str(&first.content[0].as_text().unwrap().text).unwrap();
        assert!(data.as_array().unwrap().len() > 1);
        let second = server
            .clone()
            .search_tools(search("a", false))
            .await
            .unwrap();
        let data: Value = serde_json::from_str(&second.content[0].as_text().unwrap().text).unwrap();
        assert_eq!(data.as_array().unwrap().len(), 1);
        assert!(
            second.content[1]
                .as_text()
                .unwrap()
                .text
                .contains("soft cap")
        );
        let info = server
            .tool_info(Parameters(ToolInfoParams {
                tool_name: "echo_tool".into(),
                agent_id: Some("a".into()),
                detail: "full".into(),
            }))
            .await
            .unwrap();
        assert_ne!(info.is_error, Some(true));
        let data: Value = serde_json::from_str(&info.content[0].as_text().unwrap().text).unwrap();
        assert!(data.get("input_schema").is_none());
        assert!(info.content[1].as_text().unwrap().text.contains("brief"));
        let blocked = server.search_tools(search("a", true)).await.unwrap();
        assert_eq!(blocked.is_error, Some(true));
        assert!(
            blocked.content[0]
                .as_text()
                .unwrap()
                .text
                .contains("Retry after")
        );
        let blocked = server
            .tool_info(Parameters(ToolInfoParams {
                tool_name: "echo_tool".into(),
                agent_id: Some("a".into()),
                detail: "full".into(),
            }))
            .await
            .unwrap();
        assert_eq!(blocked.is_error, Some(true));
        let other = server.search_tools(search("b", true)).await.unwrap();
        assert_ne!(other.is_error, Some(true));
        assert_eq!(other.content.len(), 1);
    }
}

#[cfg(test)]
mod telemetry_tests {
    use super::*;
    use crate::testutil::{MockBackend, insert_mock};
    use crate::tracker::CallTracker;
    use std::time::Duration;

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn sandbox_records_output_bytes() {
        let tracker = CallTracker::new();
        let raw_value = serde_json::json!({"text": "é".repeat(2_000)});
        let output = crate::tools::sandbox::handle_call_tool_chain_with_tracker(
            &ToolRegistry::new(),
            &BackendManager::new(),
            "const text = 'é'.repeat(2000);\nreturn {text};",
            None,
            Some(100),
            &Semaphore::new(1),
            None,
            None,
            &Default::default(),
            Some(&tracker),
            &Default::default(),
        )
        .await
        .unwrap();
        let stats = tracker.session_stats();
        assert_eq!(stats.total_bytes_returned, output.len() as u64);
        // The sandbox serializes the returned value as pretty JSON.
        assert_eq!(
            stats.total_bytes_processed,
            serde_json::to_string_pretty(&raw_value).unwrap().len() as u64
        );
        assert!(stats.reduction_pct > 90.0);
    }

    #[tokio::test]
    async fn structured_retrieval_direct_call_respects_budget_and_misses() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("retrieval", Duration::ZERO);
        insert_mock(&manager, &registry, &mock).await;
        let tracker = CallTracker::new();
        let code = serde_json::json!({"tool": "retrieval.echo_tool", "arguments": {"alpha": "matched", "beta": "other"}}).to_string();
        for query in ["alpha", "absent"] {
            let options = crate::tools::retrieval::RetrievalOptions {
                queries: vec![query.into()],
                max_bytes: 150,
                ..Default::default()
            };
            let output = crate::tools::sandbox::handle_call_tool_chain_with_retrieval(
                &registry,
                &manager,
                &code,
                None,
                Some(100),
                &Semaphore::new(0),
                None,
                Some("ignored"),
                &Default::default(),
                Some(&tracker),
                Some(&options),
                &Default::default(),
            )
            .await
            .unwrap();
            assert!(output.len() <= 100);
            let results: Value = serde_json::from_str(&output).unwrap();
            assert_eq!(
                results.as_array().unwrap().len(),
                usize::from(query == "alpha")
            );
        }
        assert!(
            tracker.session_stats().total_bytes_processed
                > tracker.session_stats().total_bytes_returned
        );
    }

    #[tokio::test]
    async fn retained_result_can_be_read_across_server_clones() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("retained", Duration::ZERO);
        insert_mock(&manager, &registry, &mock).await;
        let server = PrismGateServer::new(
            registry,
            manager,
            Arc::new(CallTracker::new()),
            PathBuf::new(),
            false,
            0,
            Arc::new(Semaphore::new(0)),
            Some(42),
            crate::config::OutputConfig {
                result_handle_threshold: 100,
                ..Default::default()
            },
            Default::default(),
        );
        let arguments = serde_json::json!({"text": "needle".repeat(1000)});
        let raw = serde_json::to_string_pretty(&arguments).unwrap();
        let result = server
            .call_tool_chain(Parameters(CallToolChainParams {
                code: serde_json::json!({"tool": "retained.echo_tool", "arguments": arguments})
                    .to_string(),
                timeout: None,
                max_output_size: None,
                intent: None,
                retrieval: None,
            }))
            .await
            .unwrap();
        let reference: Value =
            serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap();
        let handle = reference["result_handle"].as_str().unwrap();
        let clone = server.clone();
        let mut recovered = String::new();
        let mut offset = 0;
        loop {
            let result = clone
                .read_result(Parameters(ReadResultParams {
                    handle: handle.into(),
                    offset: Some(offset),
                    limit: Some(1000),
                    query: None,
                }))
                .await
                .unwrap();
            assert_ne!(result.is_error, Some(true));
            let page: Value =
                serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap();
            recovered.push_str(page["text"].as_str().unwrap());
            match page["next_offset"].as_u64() {
                Some(next) => offset = next as usize,
                None => break,
            }
        }
        assert_eq!(recovered, raw);
        let result = clone
            .read_result(Parameters(ReadResultParams {
                handle: handle.into(),
                offset: None,
                limit: Some(40),
                query: Some("needle".into()),
            }))
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn call_tool_chain_records_processed_and_returned_bytes() {
        let tracker = Arc::new(CallTracker::new());
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("telemetry", Duration::ZERO);
        insert_mock(&manager, &registry, &mock).await;
        let server = PrismGateServer::new(
            registry,
            manager,
            Arc::clone(&tracker),
            PathBuf::new(),
            false,
            0,
            Arc::new(Semaphore::new(0)),
            None,
            crate::config::OutputConfig {
                smart_truncation: false,
                ..Default::default()
            },
            Default::default(),
        );
        let arguments = serde_json::json!({"text": "é".repeat(2_000)});
        let raw = serde_json::to_string_pretty(&arguments).unwrap();
        let result = server
            .call_tool_chain(Parameters(CallToolChainParams {
                code: serde_json::json!({"tool": "telemetry.echo_tool", "arguments": arguments})
                    .to_string(),
                timeout: None,
                max_output_size: Some(100),
                intent: None,
                retrieval: None,
            }))
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true));
        let output = &result.content[0].as_text().unwrap().text;
        assert!(output.contains("[Output:"));
        let stats = tracker.session_stats();
        assert_eq!(stats.total_bytes_processed, raw.len() as u64);
        assert_eq!(stats.total_bytes_returned, output.len() as u64);
        assert!(stats.reduction_pct > 90.0);
        assert_eq!(stats.per_tool[0].name, "call_tool_chain");
    }
}

#[tool_handler]
impl ServerHandler for PrismGateServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),
        )
        .with_protocol_version(ProtocolVersion::V_2025_11_25)
        .with_instructions(
                "prismgate is an MCP gateway that aggregates tools from multiple backend MCP servers.\n\n\
                 IMPORTANT: Backend tools (e.g. firecrawl_search, web_search_exa) are NOT direct MCP tools. \
                 Do NOT call them directly. They MUST be called via call_tool_chain.\n\n\
                 ## Discovery Workflow (use progressive disclosure to save context)\n\
                 1. search_tools(\"your task\") → brief results by default (~60 tokens/result)\n\
                 2. tool_info(\"name\") → brief: name, backend, description, param names (~200 tokens)\n\
                 3. tool_info(\"name\", detail=\"full\") → complete schema, ONLY when ready to call (~10k tokens)\n\
                 4. call_tool_chain(\"code\") → execute TypeScript: `const r = await backend.tool({params}); return r;`\n\n\
                 ## Key Tools\n\
                 - search_tools: BM25 search across all tools. brief=true (default) or brief=false for full descriptions\n\
                 - tool_info: Get tool details. detail=\"brief\" (default) or detail=\"full\" for complete input schema\n\
                 - list_tools_meta: Paginated tool list. cursor + page_size (default 50)\n\
                 - call_tool_chain: Execute TypeScript with tools as `backend.tool_name(args)`. Use __interfaces for introspection\n\n\
                 IMPORTANT: Never guess parameter names. Always call tool_info(detail=\"full\") or __getToolInterface(\"backend.tool\") before first use of any tool.\n\n\
                 IMPORTANT: Inside call_tool_chain you can ONLY call backend tools (e.g. `exa.web_search_exa`). \
                 You CANNOT call prismgate meta-tools (search_tools, tool_info, list_tools_meta) from inside the sandbox. \
                 Use meta-tools as separate MCP tool calls OUTSIDE of call_tool_chain.\n\n\
                 ## Resources (load on-demand via @ mention)\n\
                 - @prismgate://overview → gateway guide with live tool/backend counts\n\
                 - @prismgate://backends → all backends with status and tool counts\n\
                 - @prismgate://tools → compact index of ALL tools (~3k tokens vs ~40k for full schemas)\n\
                 - @prismgate://tool/{name} → full schema for one tool (on-demand)\n\
                 - @prismgate://backend/{name} → backend details + tool list\n\n\
                 ## Prompts\n\
                 - /mcp__prismgate__discover → guided progressive discovery walkthrough\n\
                 - /mcp__prismgate__find_tool → search + top match's full schema + execution example\n\
                 - /mcp__prismgate__backend_status → health/status table for all backends\n\n\
                 ## Naming Conventions\n\
                 - ALWAYS use qualified names: `backend.tool_name` (e.g. `exa.web_search_exa`)\n\
                 - Bare names (e.g. `web_search`) may not resolve if the backend is still starting\n\
                 - In call_tool_chain, hyphens become underscores: `codebase-retrieval` -> `codebase_retrieval`\n\
                 - Backend names too: `my-backend` -> `my_backend`\n\n\
                 ## call_tool_chain Sandbox\n\
                 - ES module sandbox (V8) — NO require(), import, fs, path, or Node.js APIs\n\
                 - Tools as functions: `const r = await backend.tool_name({params}); return r;`\n\
                 - Always return a value from the entrypoint. `console.log(...)` is for debugging only and is not surfaced as the tool result; omitting `return` usually yields `null`\n\
                 - Hyphens in names are auto-converted to underscores for valid JS identifiers\n\
                 - Introspection: `__getToolInterface('backend.tool')` returns schema\n\
                 - Backends are top-level `const` variables AND mirrored on `globalThis` — both `exa.tool()` and `globalThis['exa'].tool()` work\n\
                 - Dynamic dispatch via `__backends[name][tool]({args})` also works\n\
                 - For multi-tool loops, make separate call_tool_chain calls instead of dynamic resolution\n\
                 - Standard JS only: JSON, Math, Array, Object, Promise, async/await, console\n\
                 - If a backend is stopped, the tool call will auto-restart it\n\n\
                 ## Output Processing (automatic)\n\
                 - Large JSON responses (>10KB) are auto-chunked into path summaries\n\
                 - Uniform arrays (same-structure items) are collapsed: first 3 + identity summary\n\
                 - Use `intent` parameter to filter output by relevance (e.g., intent=\"error handling\")\n\
                 - Response metadata shows KB returned vs processed when reduction occurs\n\
                 - Smart truncation preserves head (60%) and tail (40%) of output at line boundaries\n\
                 - Search supports typos: three-tier fallback (BM25 → trigram → fuzzy Levenshtein)\n\n\
                 ## Example: Find and use a web search tool\n\
                 ```\n\
                 search_tools(\"web search\")           → [{name: \"web_search_exa\", backend: \"exa\", ...}]\n\
                 tool_info(\"web_search_exa\")           → {params: [\"query\", \"num_results\", ...]}\n\
                 tool_info(\"web_search_exa\", detail=\"full\") → {input_schema: {properties: {...}}}\n\
                 call_tool_chain(`const r = await exa.web_search_exa({query: \"MCP protocol\"}); return r;`)\n\
                 ```"
        )
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListResourcesResult {
            meta: None,
            next_cursor: None,
            resources: crate::resources::list_static_resources(),
        }))
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourceTemplatesResult, McpError>> + Send + '_
    {
        std::future::ready(Ok(ListResourceTemplatesResult {
            meta: None,
            next_cursor: None,
            resource_templates: crate::resources::list_resource_templates(),
        }))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ReadResourceResult, McpError>> + Send + '_ {
        let registry = Arc::clone(&self.registry);
        let backend_manager = Arc::clone(&self.backend_manager);
        let tracker = Arc::clone(&self.tracker);
        async move {
            crate::resources::read_resource(
                &request.uri,
                &registry,
                &backend_manager,
                &tracker,
                &self.catalog_cache,
            )
            .await
        }
    }

    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListPromptsResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListPromptsResult {
            meta: None,
            next_cursor: None,
            prompts: crate::prompts::list_prompts(),
        }))
    }

    fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<GetPromptResult, McpError>> + Send + '_ {
        let registry = Arc::clone(&self.registry);
        let backend_manager = Arc::clone(&self.backend_manager);
        let tracker = Arc::clone(&self.tracker);
        async move {
            crate::prompts::get_prompt(
                &request.name,
                request.arguments,
                &registry,
                &backend_manager,
                &tracker,
            )
            .await
        }
    }

    fn complete(
        &self,
        request: CompleteRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CompleteResult, McpError>> + Send + '_ {
        let registry = Arc::clone(&self.registry);
        async move { crate::resources::complete(&request, &registry) }
    }
}
