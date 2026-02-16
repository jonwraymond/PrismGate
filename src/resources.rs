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
         Gatemini aggregates {} tools from {} backends into a single MCP server.\n\n\
         ## How to Discover Tools\n\n\
         1. **Quick overview**: Use @gatemini://backends to see all backends and tool counts\n\
         2. **Search by task**: Use search_tools(task_description=\"what you need\") — returns brief results by default\n\
         3. **Get full schema**: Use tool_info(tool_name=\"name\", detail=\"full\") only when you need the complete input schema\n\
         4. **Execute**: Use call_tool_chain to run TypeScript that calls backend tools\n\n\
         ## Tips\n\n\
         - search_tools defaults to brief=true (~60 tokens/result vs ~500)\n\
         - tool_info defaults to detail=\"brief\" (~200 tokens vs ~10k)\n\
         - Use @gatemini://tool/{{name}} resource to load full schema into context on-demand\n\
         - Use @gatemini://tools for a compact index of all {} tools\n",
        registry.tool_count(),
        registry.backend_count(),
        registry.tool_count(),
    )
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
