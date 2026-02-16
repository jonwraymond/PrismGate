use serde::Serialize;
use serde_json::Value;

use crate::backend::BackendManager;
use crate::registry::{ToolEntry, ToolRegistry};

/// Search result returned by search_tools (full mode).
#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub name: String,
    pub description: String,
    pub backend: String,
}

/// Brief search result — name, backend, first sentence of description, call example.
#[derive(Debug, Serialize)]
pub struct BriefSearchResult {
    pub name: String,
    pub backend: String,
    pub description: String,
    /// How to call this tool (backend tools are NOT direct MCP tools).
    pub call: String,
}

/// Full tool info returned by tool_info (full mode).
#[derive(Debug, Serialize)]
pub struct ToolInfoResult {
    pub name: String,
    pub description: String,
    pub backend: String,
    pub input_schema: Value,
}

/// Brief tool info — name, backend, first sentence of description, parameter names, call example.
#[derive(Debug, Serialize)]
pub struct BriefToolInfoResult {
    pub name: String,
    pub backend: String,
    pub description: String,
    pub parameters: Vec<String>,
    /// How to call this tool (backend tools are NOT direct MCP tools).
    pub call: String,
}

/// Sanitize a name for use in a JS call example.
/// Delegates to the shared `sanitize_identifier` which handles leading digits and empty strings.
fn sanitize_js_name(name: &str) -> String {
    crate::sandbox::bridge::sanitize_identifier(name)
}

/// Extract the first sentence from a description string.
fn first_sentence(text: &str) -> String {
    // Find first period followed by space or end of string
    if let Some(idx) = text.find(". ") {
        text[..=idx].to_string()
    } else if let Some(idx) = text.find(".\n") {
        text[..=idx].to_string()
    } else if text.ends_with('.') {
        text.to_string()
    } else if text.len() > 200 {
        // Truncate long descriptions without sentence boundary
        format!("{}...", &text[..200])
    } else {
        text.to_string()
    }
}

/// Extract parameter names from a JSON schema's `properties` object.
fn extract_param_names(schema: &Value) -> Vec<String> {
    schema
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default()
}

/// Search the registry, using hybrid BM25+semantic when the semantic feature is active.
fn search_tools(registry: &ToolRegistry, query: &str, limit: u32) -> Vec<ToolEntry> {
    #[cfg(feature = "semantic")]
    {
        registry.search_hybrid(query, limit)
    }
    #[cfg(not(feature = "semantic"))]
    {
        registry.search(query, limit)
    }
}

/// Handle search_tools: BM25 (or hybrid) search across names and descriptions.
pub fn handle_search(registry: &ToolRegistry, query: &str, limit: u32) -> Vec<SearchResult> {
    search_tools(registry, query, limit)
        .into_iter()
        .map(|e| SearchResult {
            name: e.name,
            description: e.description,
            backend: e.backend_name,
        })
        .collect()
}

/// Handle search_tools with brief=true: returns compact results.
pub fn handle_search_brief(
    registry: &ToolRegistry,
    query: &str,
    limit: u32,
) -> Vec<BriefSearchResult> {
    search_tools(registry, query, limit)
        .into_iter()
        .map(|e| {
            let orig = if e.original_name.is_empty() {
                &e.name
            } else {
                &e.original_name
            };
            let call = format!(
                "call_tool_chain: await {}.{}({{...}})",
                sanitize_js_name(&e.backend_name),
                sanitize_js_name(orig)
            );
            BriefSearchResult {
                name: e.name,
                backend: e.backend_name,
                description: first_sentence(&e.description),
                call,
            }
        })
        .collect()
}

/// Handle list_tools with pagination.
pub fn handle_list_paginated(
    registry: &ToolRegistry,
    cursor: Option<&str>,
    page_size: u32,
) -> (Vec<String>, Option<String>) {
    let mut names = registry.get_all_names();
    names.sort(); // Stable ordering for pagination

    let start = cursor.and_then(|c| c.parse::<usize>().ok()).unwrap_or(0);

    let page: Vec<String> = names
        .into_iter()
        .skip(start)
        .take(page_size as usize)
        .collect();

    let next_cursor = if page.len() == page_size as usize {
        Some((start + page_size as usize).to_string())
    } else {
        None
    };

    (page, next_cursor)
}

/// Handle tool_info: return full schema for a specific tool.
pub fn handle_tool_info(registry: &ToolRegistry, tool_name: &str) -> Option<ToolInfoResult> {
    registry.get_by_name(tool_name).map(|e| ToolInfoResult {
        name: e.name,
        description: e.description,
        backend: e.backend_name,
        input_schema: e.input_schema,
    })
}

/// Handle tool_info with brief mode: returns compact info.
pub fn handle_tool_info_brief(
    registry: &ToolRegistry,
    tool_name: &str,
) -> Option<BriefToolInfoResult> {
    registry.get_by_name(tool_name).map(|e| {
        let orig = if e.original_name.is_empty() {
            &e.name
        } else {
            &e.original_name
        };
        let parameters = extract_param_names(&e.input_schema);
        let call = format!(
            "call_tool_chain: await {}.{}({{...}})",
            sanitize_js_name(&e.backend_name),
            sanitize_js_name(orig)
        );
        BriefToolInfoResult {
            name: e.name,
            backend: e.backend_name,
            description: first_sentence(&e.description),
            parameters,
            call,
        }
    })
}

/// Handle get_required_keys_for_tool: return env var keys the backend needs.
pub async fn handle_required_keys_async(
    registry: &ToolRegistry,
    manager: &BackendManager,
    tool_name: &str,
) -> Option<Vec<String>> {
    let entry = registry.get_by_name(tool_name)?;
    let config = manager.get_backend_config(&entry.backend_name).await?;

    // Return the env var keys from the backend config + explicit required_keys
    let mut keys: Vec<String> = config.env.keys().cloned().collect();
    for k in &config.required_keys {
        if !keys.contains(k) {
            keys.push(k.clone());
        }
    }
    Some(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_sentence() {
        assert_eq!(
            first_sentence("Search the web. Returns results in JSON format."),
            "Search the web."
        );
        assert_eq!(first_sentence("Search the web"), "Search the web");
        assert_eq!(first_sentence("Search the web."), "Search the web.");
        assert_eq!(first_sentence("Search.\nMore info here."), "Search.");
        // Long text without period
        let long = "a".repeat(250);
        let result = first_sentence(&long);
        assert_eq!(result.len(), 203); // 200 + "..."
    }

    #[test]
    fn test_extract_param_names() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "limit": {"type": "integer"}
            }
        });
        let mut names = extract_param_names(&schema);
        names.sort();
        assert_eq!(names, vec!["limit", "query"]);
    }

    #[test]
    fn test_extract_param_names_empty() {
        let schema = serde_json::json!({"type": "object"});
        assert_eq!(extract_param_names(&schema), Vec::<String>::new());
    }

    #[test]
    fn test_sanitize_js_name_basic() {
        assert_eq!(sanitize_js_name("exa"), "exa");
        assert_eq!(sanitize_js_name("web_search"), "web_search");
    }

    #[test]
    fn test_sanitize_js_name_hyphens() {
        assert_eq!(sanitize_js_name("my-backend"), "my_backend");
        assert_eq!(sanitize_js_name("web-search-exa"), "web_search_exa");
    }

    #[test]
    fn test_sanitize_js_name_leading_digit() {
        // Must prefix with underscore to be valid JS identifier
        assert_eq!(sanitize_js_name("123api"), "_123api");
    }

    #[test]
    fn test_sanitize_js_name_dots_and_special() {
        assert_eq!(sanitize_js_name("my.tool"), "my_tool");
        assert_eq!(sanitize_js_name("tool@v2"), "tool_v2");
    }

    #[test]
    fn test_sanitize_js_name_empty() {
        assert_eq!(sanitize_js_name(""), "_unnamed");
    }
}
