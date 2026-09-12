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
    /// Distinctive terms from this backend for follow-up searches.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub try_also: Vec<String>,
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

/// Strip non-essential description components per arxiv 2602.14878.
///
/// Preserves the `Limitations:` block because it contains high-leverage
/// operational cues. Removes `Parameters:`, `Key features:`, `You should:`,
/// `When to use:`, `Examples:`, and `Guidelines:` sections.
///
/// The result is clamped to 240 chars at a sentence boundary.
fn minimize_description(text: &str) -> String {
    let non_essential = [
        "parameters:",
        "key features:",
        "you should:",
        "when to use:",
        "examples:",
        "guidelines:",
    ];

    let mut preserved = Vec::new();
    let mut limitations = Vec::new();

    for para in text.split("\n\n") {
        let trimmed = para.trim();
        if trimmed.is_empty() {
            continue;
        }
        let first = trimmed.lines().next().unwrap_or("").trim().to_lowercase();

        if first.starts_with("limitations:") {
            limitations.push(trimmed);
        } else if non_essential.iter().any(|pat| first.starts_with(pat)) {
            // skip non-essential section
        } else {
            preserved.push(trimmed);
        }
    }

    let mut result = preserved.join(" ");
    if !limitations.is_empty() {
        if !result.is_empty() {
            result.push(' ');
        }
        result.push_str(&limitations.join(" "));
    }

    // Clamp to 240 chars at a sentence boundary.
    if result.len() > 240 {
        if let Some(idx) = result[..240].rfind(". ") {
            result = result[..=idx].to_string();
        } else if let Some(idx) = result[..240].rfind(".\n") {
            result = result[..=idx].to_string();
        } else {
            result = format!("{}...", &result[..237]);
        }
    }

    result
}

/// Prune per-parameter `description` fields in a JSON schema to their first sentence.
///
/// This reduces token usage while preserving the high-leverage operational cues
/// identified in arxiv 2602.14878 RQ-3.
fn prune_schema_descriptions(schema: &mut Value) {
    let Some(obj) = schema.as_object_mut() else {
        return;
    };
    let Some(properties) = obj.get_mut("properties").and_then(|p| p.as_object_mut()) else {
        return;
    };

    for (_, value) in properties.iter_mut() {
        let Some(prop_obj) = value.as_object_mut() else {
            continue;
        };
        if let Some(desc) = prop_obj.get("description").and_then(|d| d.as_str()) {
            let minimized = first_sentence(desc);
            if !minimized.is_empty() {
                prop_obj.insert("description".to_string(), Value::String(minimized));
            }
        }
    }

    // Recurse into nested object properties if present
    for (_, value) in properties.iter_mut() {
        prune_schema_descriptions(value);
    }

    // Also recurse into `items` for array schemas
    if let Some(items) = obj.get_mut("items") {
        prune_schema_descriptions(items);
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
fn search_tools(
    registry: &ToolRegistry,
    query: &str,
    limit: u32,
    filter_tags: Option<&[String]>,
    tracker: Option<&crate::tracker::CallTracker>,
) -> Vec<ToolEntry> {
    #[cfg(feature = "semantic")]
    {
        registry.search_hybrid(query, limit, filter_tags, tracker)
    }
    #[cfg(not(feature = "semantic"))]
    {
        registry.search(query, limit, filter_tags, tracker)
    }
}

/// Handle search_tools: BM25 (or hybrid) search across names and descriptions.
pub fn handle_search(
    registry: &ToolRegistry,
    query: &str,
    limit: u32,
    filter_tags: Option<&[String]>,
    tracker: Option<&crate::tracker::CallTracker>,
    decision: crate::flood_guard::Decision,
) -> Result<Vec<SearchResult>, crate::flood_guard::Decision> {
    let limit = decision.limit(limit)?;
    Ok(search_tools(registry, query, limit, filter_tags, tracker)
        .into_iter()
        .map(|e| SearchResult {
            name: e.name,
            description: e.description,
            backend: e.backend_name,
        })
        .collect())
}

/// Handle search_tools with brief=true: returns compact results.
pub fn handle_search_brief(
    registry: &ToolRegistry,
    query: &str,
    limit: u32,
    filter_tags: Option<&[String]>,
    tracker: Option<&crate::tracker::CallTracker>,
    decision: crate::flood_guard::Decision,
) -> Result<Vec<BriefSearchResult>, crate::flood_guard::Decision> {
    let limit = decision.limit(limit)?;
    Ok(search_tools(registry, query, limit, filter_tags, tracker)
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
            let try_also = registry.get_distinctive_terms(&e.backend_name, 3);
            BriefSearchResult {
                name: e.name,
                backend: e.backend_name,
                description: minimize_description(&e.description),
                call,
                try_also,
            }
        })
        .collect())
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
    registry.get_by_name(tool_name).map(|e| {
        let mut input_schema = e.input_schema;
        prune_schema_descriptions(&mut input_schema);
        ToolInfoResult {
            name: e.name,
            description: e.description,
            backend: e.backend_name,
            input_schema,
        }
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
            description: minimize_description(&e.description),
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

    #[test]
    fn test_minimize_description_strips_sections() {
        let input = "Search the web.\n\nParameters:\n- query (string): the search terms\n- limit (integer): max results\n\nKey features:\n- returns JSON\n\nLimitations:\n- requires API key";
        let result = minimize_description(input);
        assert!(result.contains("Search the web"));
        assert!(!result.contains("Parameters:"));
        assert!(!result.contains("Key features:"));
        assert!(result.contains("Limitations"));
    }

    #[test]
    fn test_minimize_description_clamps_long() {
        let input = "A very long description. ".repeat(20);
        let result = minimize_description(&input);
        assert!(result.len() <= 240, "got len {}", result.len());
    }

    #[test]
    fn test_minimize_description_preserves_short() {
        let input = "Short description.";
        let result = minimize_description(input);
        assert_eq!(result, "Short description.");
    }

    #[test]
    fn test_prune_schema_descriptions() {
        let mut schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query. Should be concise."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return."
                }
            }
        });
        prune_schema_descriptions(&mut schema);
        let query_desc = schema["properties"]["query"]["description"]
            .as_str()
            .unwrap();
        let limit_desc = schema["properties"]["limit"]["description"]
            .as_str()
            .unwrap();
        assert_eq!(query_desc, "The search query.");
        assert_eq!(limit_desc, "Maximum number of results to return.");
    }
}
