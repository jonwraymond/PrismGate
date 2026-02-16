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

use crate::backend::BackendManager;
use crate::registry::ToolRegistry;

// --- Parameter structs for each meta-tool ---

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
    /// Maximum number of results to return.
    #[serde(default = "default_limit")]
    pub limit: u32,
    /// Return brief results (name, backend, first sentence). Default: true. Set false for full descriptions.
    #[serde(default = "default_true")]
    pub brief: bool,
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
    /// Optional maximum output size in characters (default: 200000).
    pub max_output_size: Option<usize>,
}

/// The MCP server exposed to Claude Code over stdio.
#[derive(Clone)]
pub struct GateminiServer {
    pub registry: Arc<ToolRegistry>,
    pub backend_manager: Arc<BackendManager>,
    pub cache_path: PathBuf,
    pub allow_runtime_registration: bool,
    pub max_dynamic_backends: usize,
    tool_router: ToolRouter<Self>,
}

impl GateminiServer {
    pub fn new(
        registry: Arc<ToolRegistry>,
        backend_manager: Arc<BackendManager>,
        cache_path: PathBuf,
        allow_runtime_registration: bool,
        max_dynamic_backends: usize,
    ) -> Self {
        Self {
            registry,
            backend_manager,
            cache_path,
            allow_runtime_registration,
            max_dynamic_backends,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl GateminiServer {
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
                tokio::spawn(async move { crate::cache::save(&cp, &reg).await });
                Ok(CallToolResult::success(vec![Content::text(msg)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
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
                tokio::spawn(async move { crate::cache::save(&cp, &reg).await });
                Ok(CallToolResult::success(vec![Content::text(msg)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    #[tool(description = "Searches for relevant tools based on a task description. Covers: web search (tavily, exa, zai), code intelligence (auggie, serena, octocode), browser automation (playwright, chrome-devtools), AI models (cerebras, pal, minimax), databases (supabase), file processing (repomix, firecrawl), docs (context7, deepwiki, ref), and more. Default: brief=true for compact results.")]
    async fn search_tools(
        &self,
        Parameters(params): Parameters<SearchToolsParams>,
    ) -> Result<CallToolResult, McpError> {
        let json = if params.brief {
            let results = crate::tools::discovery::handle_search_brief(
                &self.registry,
                &params.task_description,
                params.limit,
            );
            serde_json::to_string_pretty(&results)
        } else {
            let results = crate::tools::discovery::handle_search(
                &self.registry,
                &params.task_description,
                params.limit,
            );
            serde_json::to_string_pretty(&results)
        }
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
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

    #[tool(description = "Get complete information about a specific tool including its input schema.")]
    async fn tool_info(
        &self,
        Parameters(params): Parameters<ToolInfoParams>,
    ) -> Result<CallToolResult, McpError> {
        let json = if params.detail == "full" {
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
        Ok(CallToolResult::success(vec![Content::text(json)]))
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

    #[tool(description = "Execute TypeScript code with direct access to all registered tools as hierarchical functions (e.g., manual.tool()).")]
    async fn call_tool_chain(
        &self,
        Parameters(params): Parameters<CallToolChainParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = crate::tools::sandbox::handle_call_tool_chain(
            &self.registry,
            &self.backend_manager,
            &params.code,
            params.timeout,
            params.max_output_size,
        )
        .await;

        match result {
            Ok(output) => Ok(CallToolResult::success(vec![Content::text(output)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }
}

#[tool_handler]
impl ServerHandler for GateminiServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2025_06_18,
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(
                "gatemini is an MCP gateway that aggregates tools from multiple backend MCP servers.\n\n\
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
                 ## Resources (load on-demand via @ mention)\n\
                 - @gatemini://overview → gateway guide with live tool/backend counts\n\
                 - @gatemini://backends → all backends with status and tool counts\n\
                 - @gatemini://tools → compact index of ALL tools (~3k tokens vs ~40k for full schemas)\n\
                 - @gatemini://tool/{name} → full schema for one tool (on-demand)\n\
                 - @gatemini://backend/{name} → backend details + tool list\n\n\
                 ## Prompts\n\
                 - /mcp__gatemini__discover → guided progressive discovery walkthrough\n\
                 - /mcp__gatemini__find_tool → search + top match's full schema + execution example\n\
                 - /mcp__gatemini__backend_status → health/status table for all backends\n\n\
                 ## call_tool_chain Sandbox\n\
                 - ES module sandbox (V8) — NO require(), import, fs, path, or Node.js APIs\n\
                 - Tools as functions: `const r = await backend.tool_name({params}); return r;`\n\
                 - Introspection: `__getToolInterface('backend.tool')` returns schema\n\
                 - Standard JS only: JSON, Math, Array, Object, Promise, async/await, console\n\
                 - If a backend is stopped, the tool call will auto-restart it\n\n\
                 ## Example: Find and use a web search tool\n\
                 ```\n\
                 search_tools(\"web search\")           → [{name: \"web_search_exa\", backend: \"exa\", ...}]\n\
                 tool_info(\"web_search_exa\")           → {params: [\"query\", \"num_results\", ...]}\n\
                 tool_info(\"web_search_exa\", detail=\"full\") → {input_schema: {properties: {...}}}\n\
                 call_tool_chain(`const r = await exa.web_search_exa({query: \"MCP protocol\"}); return r;`)\n\
                 ```"
                    .into(),
            ),
        }
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
        async move {
            crate::resources::read_resource(&request.uri, &registry, &backend_manager).await
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
        async move {
            crate::prompts::get_prompt(&request.name, request.arguments, &registry, &backend_manager)
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
