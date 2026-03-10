//! MCP resources and resource-template handling for compact discovery views.

use std::sync::Arc;

use rmcp::{ErrorData as McpError, model::*};
use serde::Serialize;

use crate::backend::BackendManager;
use crate::registry::ToolRegistry;
use crate::tracker::CallTracker;

/// Return the static resources available for @-mention discovery.
pub fn list_static_resources() -> Vec<Resource> {
    vec![
        Resource {
            raw: RawResource {
                uri: "gatemini://overview".to_string(),
                name: "overview".to_string(),
                title: Some("Gatemini Overview".to_string()),
                description: Some(
                    "Gateway guide: what gatemini is, how to discover tools, when to use resources vs tools"
                        .to_string(),
                ),
                mime_type: Some("text/plain".to_string()),
                size: None,
                icons: None,
                meta: None,
            },
            annotations: None,
        },
        Resource {
            raw: RawResource {
                uri: "gatemini://backends".to_string(),
                name: "backends".to_string(),
                title: Some("Backend List".to_string()),
                description: Some(
                    "JSON list of all backends with name, tool count, and status".to_string(),
                ),
                mime_type: Some("application/json".to_string()),
                size: None,
                icons: None,
                meta: None,
            },
            annotations: None,
        },
        Resource {
            raw: RawResource {
                uri: "gatemini://tools".to_string(),
                name: "tools".to_string(),
                title: Some("Compact Tool Index".to_string()),
                description: Some(
                    "All tools with name, backend, and one-line description (~3k tokens vs ~40k for full schemas)"
                        .to_string(),
                ),
                mime_type: Some("application/json".to_string()),
                size: None,
                icons: None,
                meta: None,
            },
            annotations: None,
        },
        Resource {
            raw: RawResource {
                uri: "gatemini://recent".to_string(),
                name: "recent".to_string(),
                title: Some("Recent Tool Calls".to_string()),
                description: Some(
                    "Last 50 tool calls with tool name, backend, duration, and success/failure"
                        .to_string(),
                ),
                mime_type: Some("application/json".to_string()),
                size: None,
                icons: None,
                meta: None,
            },
            annotations: None,
        },
        Resource {
            raw: RawResource {
                uri: "gatemini://call_tool_chain".to_string(),
                name: "call_tool_chain".to_string(),
                title: Some("call_tool_chain Guide".to_string()),
                description: Some(
                    "Execution contract, return semantics, and examples for sandboxed TypeScript tool calls"
                        .to_string(),
                ),
                mime_type: Some("text/plain".to_string()),
                size: None,
                icons: None,
                meta: None,
            },
            annotations: None,
        },
    ]
}

/// Return resource templates for on-demand tool/backend lookups.
pub fn list_resource_templates() -> Vec<ResourceTemplate> {
    vec![
        ResourceTemplate {
            raw: RawResourceTemplate {
                uri_template: "gatemini://tool/{tool_name}".to_string(),
                name: "tool".to_string(),
                title: Some("Tool Schema".to_string()),
                description: Some(
                    "Full schema + description for a specific tool (on-demand, ~200-10k tokens)"
                        .to_string(),
                ),
                mime_type: Some("application/json".to_string()),
                icons: None,
            },
            annotations: None,
        },
        ResourceTemplate {
            raw: RawResourceTemplate {
                uri_template: "gatemini://backend/{backend_name}".to_string(),
                name: "backend".to_string(),
                title: Some("Backend Details".to_string()),
                description: Some(
                    "Backend details: name, status, tool count, list of tool names".to_string(),
                ),
                mime_type: Some("application/json".to_string()),
                icons: None,
            },
            annotations: None,
        },
        ResourceTemplate {
            raw: RawResourceTemplate {
                uri_template: "gatemini://backend/{backend_name}/tools".to_string(),
                name: "backend-tools".to_string(),
                title: Some("Backend Tools".to_string()),
                description: Some(
                    "All tools for a specific backend with brief descriptions".to_string(),
                ),
                mime_type: Some("application/json".to_string()),
                icons: None,
            },
            annotations: None,
        },
        ResourceTemplate {
            raw: RawResourceTemplate {
                uri_template: "gatemini://recent/{limit}".to_string(),
                name: "recent-limited".to_string(),
                title: Some("Recent Tool Calls (Custom Limit)".to_string()),
                description: Some(
                    "Last N tool calls (customizable limit) with tool name, backend, duration, success"
                        .to_string(),
                ),
                mime_type: Some("application/json".to_string()),
                icons: None,
            },
            annotations: None,
        },
        ResourceTemplate {
            raw: RawResourceTemplate {
                uri_template: "gatemini://guide/{topic}".to_string(),
                name: "guide".to_string(),
                title: Some("Guide".to_string()),
                description: Some(
                    "Focused guidance for gateway concepts such as call_tool_chain return semantics and discovery workflow"
                        .to_string(),
                ),
                mime_type: Some("text/plain".to_string()),
                icons: None,
            },
            annotations: None,
        },
    ]
}

/// Compact tool entry for the gatemini://tools resource.
#[derive(Debug, Serialize)]
struct CompactToolEntry {
    name: String,
    backend: String,
    description: String,
}

/// Backend info entry for the gatemini://backends resource.
#[derive(Debug, Serialize)]
struct BackendInfo {
    name: String,
    tool_count: usize,
    status: String,
    available: bool,
}

/// Backend detail for the gatemini://backend/{name} template.
#[derive(Debug, Serialize)]
struct BackendDetail {
    name: String,
    status: String,
    available: bool,
    tool_count: usize,
    tools: Vec<String>,
}

/// Extract the first sentence from a description.
fn first_sentence(text: &str) -> String {
    if let Some(idx) = text.find(". ") {
        text[..=idx].to_string()
    } else if let Some(idx) = text.find(".\n") {
        text[..=idx].to_string()
    } else if text.ends_with('.') {
        text.to_string()
    } else if text.len() > 120 {
        format!("{}...", &text[..120])
    } else {
        text.to_string()
    }
}

/// Handle read_resource for all gatemini:// URIs.
pub async fn read_resource(
    uri: &str,
    registry: &Arc<ToolRegistry>,
    backend_manager: &Arc<BackendManager>,
    tracker: &Arc<CallTracker>,
) -> Result<ReadResourceResult, McpError> {
    // Parse the URI
    let path = uri
        .strip_prefix("gatemini://")
        .ok_or_else(|| McpError::invalid_params(format!("Unknown URI scheme: {uri}"), None))?;

    match path {
        "overview" => Ok(text_resource(uri, &overview_text(registry))),
        "call_tool_chain" => Ok(text_resource(uri, &call_tool_chain_guide_text())),
        "backends" => {
            let statuses = backend_manager.get_all_status();
            let infos: Vec<BackendInfo> = statuses
                .into_iter()
                .map(|s| {
                    let tool_count = registry.get_by_backend(&s.name).len();
                    BackendInfo {
                        name: s.name,
                        tool_count,
                        status: format!("{:?}", s.state),
                        available: s.available,
                    }
                })
                .collect();
            let json = serde_json::to_string_pretty(&infos)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            Ok(text_resource(uri, &json))
        }
        "recent" => {
            let calls = tracker.recent_calls(50);
            let json = serde_json::to_string_pretty(&calls)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            Ok(text_resource(uri, &json))
        }
        "tools" => {
            let tools: Vec<CompactToolEntry> = registry
                .get_all()
                .into_iter()
                .map(|e| CompactToolEntry {
                    name: e.name,
                    backend: e.backend_name,
                    description: first_sentence(&e.description),
                })
                .collect();
            let json = serde_json::to_string_pretty(&tools)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            Ok(text_resource(uri, &json))
        }
        _ => {
            // Try template matching
            if let Some(limit_str) = path.strip_prefix("recent/") {
                // gatemini://recent/{limit}
                let limit: usize = limit_str.parse().map_err(|_| {
                    McpError::invalid_params(
                        format!("Invalid limit '{limit_str}': must be a positive integer"),
                        None,
                    )
                })?;
                let calls = tracker.recent_calls(limit);
                let json = serde_json::to_string_pretty(&calls)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                Ok(text_resource(uri, &json))
            } else if let Some(topic) = path.strip_prefix("guide/") {
                match topic {
                    "call_tool_chain" => Ok(text_resource(uri, &call_tool_chain_guide_text())),
                    "discovery" => Ok(text_resource(uri, &overview_text(registry))),
                    _ => Err(McpError::invalid_params(
                        format!(
                            "Unknown guide topic '{topic}'. Available topics: call_tool_chain, discovery"
                        ),
                        None,
                    )),
                }
            } else if let Some(tool_name) = path.strip_prefix("tool/") {
                // gatemini://tool/{tool_name}
                let entry = registry.get_by_name(tool_name).ok_or_else(|| {
                    McpError::invalid_params(format!("Tool '{tool_name}' not found"), None)
                })?;
                let json = serde_json::to_string_pretty(&entry)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                Ok(text_resource(uri, &json))
            } else if let Some(rest) = path.strip_prefix("backend/") {
                if let Some(backend_name) = rest.strip_suffix("/tools") {
                    // gatemini://backend/{name}/tools
                    let tools = registry.get_by_backend(backend_name);
                    if tools.is_empty() {
                        return Err(McpError::invalid_params(
                            format!("Backend '{backend_name}' not found or has no tools"),
                            None,
                        ));
                    }
                    let brief: Vec<CompactToolEntry> = tools
                        .into_iter()
                        .map(|e| CompactToolEntry {
                            name: e.name,
                            backend: e.backend_name,
                            description: first_sentence(&e.description),
                        })
                        .collect();
                    let json = serde_json::to_string_pretty(&brief)
                        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                    Ok(text_resource(uri, &json))
                } else {
                    // gatemini://backend/{name}
                    let backend_name = rest;
                    let tools = registry.get_by_backend(backend_name);
                    let status = backend_manager
                        .get_all_status()
                        .into_iter()
                        .find(|s| s.name == backend_name);
                    let detail = BackendDetail {
                        name: backend_name.to_string(),
                        status: status
                            .as_ref()
                            .map(|s| format!("{:?}", s.state))
                            .unwrap_or_else(|| "Unknown".to_string()),
                        available: status.as_ref().is_some_and(|s| s.available),
                        tool_count: tools.len(),
                        tools: tools.into_iter().map(|t| t.name).collect(),
                    };
                    let json = serde_json::to_string_pretty(&detail)
                        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                    Ok(text_resource(uri, &json))
                }
            } else {
                Err(McpError::invalid_params(
                    format!("Unknown resource URI: {uri}"),
                    None,
                ))
            }
        }
    }
}

/// Handle completion/complete for resource template arguments.
pub fn complete(
    request: &CompleteRequestParams,
    registry: &Arc<ToolRegistry>,
) -> Result<CompleteResult, McpError> {
    match &request.r#ref {
        Reference::Resource(resource_ref) => {
            let uri = &resource_ref.uri;
            if uri.contains("tool/{tool_name}") || uri.contains("tool/") {
                // Complete tool names
                let prefix = &request.argument.value;
                let values: Vec<String> = registry
                    .get_all_names()
                    .into_iter()
                    .filter(|n| n.starts_with(prefix))
                    .take(CompletionInfo::MAX_VALUES)
                    .collect();
                let total = values.len() as u32;
                Ok(CompleteResult {
                    completion: CompletionInfo {
                        values,
                        total: Some(total),
                        has_more: Some(false),
                    },
                })
            } else if uri.contains("backend/{backend_name}") || uri.contains("backend/") {
                // Complete backend names
                let prefix = &request.argument.value;
                let values: Vec<String> = registry
                    .get_backend_names()
                    .into_iter()
                    .filter(|n| n.starts_with(prefix))
                    .take(CompletionInfo::MAX_VALUES)
                    .collect();
                let total = values.len() as u32;
                Ok(CompleteResult {
                    completion: CompletionInfo {
                        values,
                        total: Some(total),
                        has_more: Some(false),
                    },
                })
            } else if uri.contains("guide/{topic}") || uri.contains("guide/") {
                let prefix = &request.argument.value;
                let values: Vec<String> = ["call_tool_chain", "discovery"]
                    .into_iter()
                    .filter(|topic| topic.starts_with(prefix))
                    .map(str::to_string)
                    .collect();
                let total = values.len() as u32;
                Ok(CompleteResult {
                    completion: CompletionInfo {
                        values,
                        total: Some(total),
                        has_more: Some(false),
                    },
                })
            } else {
                Ok(CompleteResult::default())
            }
        }
        Reference::Prompt(_) => Ok(CompleteResult::default()),
    }
}

fn overview_text(registry: &ToolRegistry) -> String {
    format!(
        "# Gatemini MCP Gateway\n\n\
         You are connected to gatemini, an MCP gateway that aggregates {} tools from {} backends          into a single interface. You interact with it through 7 meta-tools — never call backend          tools directly as MCP tools.\n\n\
         ## Discovery\n\n\
         1. `search_tools(task_description=\"what you need\")` — brief results (~60 tokens each)\n\
         2. `tool_info(tool_name=\"backend.tool_name\")` — parameter names (~200 tokens)\n\
         3. `tool_info(tool_name=\"...\", detail=\"full\")` — complete input schema (only when ready to call)\n\
         4. `list_tools_meta` — paginated browsing of all {} tools\n\n\
         ## Execution\n\n\
         Call backend tools via `call_tool_chain` with TypeScript:\n\
         ```typescript\n\
         const r = await exa.web_search_exa({{ query: \"...\" }});\n\
         return r;\n\
         ```\n\
         You MUST explicitly `return` a value. `console.log()` output is not the result; omitting `return` yields `null`.\n\n\
         For multi-tool loops, use `__backends` for dynamic dispatch:\n\
         ```typescript\n\
         for (const q of queries) {{\n\
           results.push(await __backends[q.backend][q.tool](q.args));\n\
         }}\n\
         return results;\n\
         ```\n\n\
         ## Naming Rules\n\n\
         - ALWAYS use qualified names: `backend.tool_name` (e.g. `exa.web_search_exa`)\n\
         - Hyphens become underscores in sandbox: `my-backend` -> `my_backend`\n\
         - `__backends` maps both original and sanitized names\n\
         - Bare names may not resolve if the backend is still starting\n\n\
         ## Resources\n\n\
         - `@gatemini://tools` — compact index of all tools (~3k tokens)\n\
         - `@gatemini://backends` — backend health status and tool counts\n\
         - `@gatemini://tool/{{name}}` — full schema for one tool\n\
         - `@gatemini://call_tool_chain` — execution contract and examples\n\n\
         ## Prompts\n\n\
         - `/mcp__gatemini__discover` — guided discovery walkthrough\n\
         - `/mcp__gatemini__find_tool` — search + top match schema\n\
         - `/mcp__gatemini__backend_status` — health dashboard\n\n\
         ## Runtime Management\n\n\
         - `register_manual` / `deregister_manual` — add/remove backends dynamically\n\
         - `get_required_keys_for_tool` — check env vars a backend needs\n",
        registry.tool_count(),
        registry.backend_count(),
        registry.tool_count(),
    )
}

fn call_tool_chain_guide_text() -> String {
    "# call_tool_chain Guide\n\n\
     ## Execution Contract\n\n\
     - `call_tool_chain` returns the value your sandbox entrypoint explicitly `return`s\n\
     - `console.log(...)` output is for debugging only and is not surfaced as the tool result\n\
     - If you do not `return` a value, the result is usually `null`\n\n\
     ## Recommended Pattern\n\n\
     ```typescript\n\
     const lib = await context7.resolve_library_id({\n\
       query: \"Tailscale official documentation\",\n\
       libraryName: \"Tailscale\",\n\
     });\n\
     const refSearch = await ref.ref_search_documentation({\n\
       query: \"site:tailscale.com/kb subnet routers exit nodes\",\n\
     });\n\
     return { lib, refSearch };\n\
     ```\n\n\
     ## Avoid This\n\n\
     ```typescript\n\
     const lib = await context7.resolve_library_id({\n\
       query: \"Tailscale official documentation\",\n\
       libraryName: \"Tailscale\",\n\
     });\n\
     console.log(JSON.stringify(lib, null, 2));\n\
     // No explicit return -> call_tool_chain result is usually null\n\
     ```\n\n\
     ## Dynamic Dispatch\n\n\
     Backends are top-level variables, NOT properties of `globalThis`.\n\
     `globalThis.exa` and bracket notation like `globalThis[name]` are `undefined`.\n\n\
     ```typescript\n\
     // WRONG: dynamic dispatch via globalThis\n\
     const result = await globalThis[backendName][toolName]({...}); // TypeError\n\n\
     // CORRECT: use backend variables directly\n\
     const result = await auggie.codebase_retrieval({...});\n\
     return result;\n\
     ```\n\n\
     For loops over multiple tools, make separate call_tool_chain calls\n\
     or list tool calls sequentially using direct backend references.\n\n\
     ## Quick Rules\n\n\
     - Always return a compact object or summary from multi-step chains\n\
     - Use `console.log(...)` only for debugging while developing the chain\n\
     - Prefer qualified names such as `backend.tool_name(...)`\n\
     - Hyphens become underscores in sandbox identifiers\n\
     - Backends are top-level variables, NOT on globalThis\n"
        .to_string()
}

fn text_resource(uri: &str, text: &str) -> ReadResourceResult {
    ReadResourceResult {
        contents: vec![ResourceContents::TextResourceContents {
            uri: uri.to_string(),
            mime_type: Some("text/plain".to_string()),
            text: text.to_string(),
            meta: None,
        }],
    }
}
