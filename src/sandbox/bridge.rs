use crate::registry::ToolEntry;
use std::collections::HashMap;

/// Generate a JavaScript preamble that creates tool accessor objects and
/// introspection APIs matching the sandbox call interface.
///
/// For each backend, generates an object with async methods:
/// ```js
/// const exa = {
///     web_search: async (args) => await rustyscript.async_functions['__call_tool']('exa', 'web_search', args || {}),
/// };
/// ```
///
/// Also generates `__interfaces` and `__getToolInterface()` for discovery.
pub fn generate_preamble(tools: &[ToolEntry]) -> String {
    // Group tools by backend
    let mut by_backend: HashMap<&str, Vec<&ToolEntry>> = HashMap::new();
    for tool in tools {
        by_backend.entry(&tool.backend_name).or_default().push(tool);
    }

    let mut preamble = String::with_capacity(4096);

    // Helper alias
    preamble.push_str("const __ct = rustyscript.async_functions['__call_tool'];\n\n");

    // Node.js compatibility shim — catch common LLM mistakes with helpful errors
    preamble.push_str(
        "function require(module) {\n\
         \x20 throw new Error(\n\
         \x20   `require('${module}') is not available. This is an ES module sandbox, not Node.js.\\n` +\n\
         \x20   `Available in this sandbox:\\n` +\n\
         \x20   `  - Backend tools: const r = await backend_name.tool_name({args}); return r;\\n` +\n\
         \x20   `  - Introspection: __getToolInterface('backend.tool')\\n` +\n\
         \x20   `  - Standard JS: JSON, Math, Array, Object, Promise, async/await, console\\n` +\n\
         \x20   `  - NO require(), import, fs, path, child_process, or network access`\n\
         \x20 );\n\
         }\n\
         const process = undefined;\n\
         const module = undefined;\n\
         const exports = undefined;\n\
         const Buffer = undefined;\n\n",
    );

    // Generate backend accessor objects
    for (backend_name, backend_tools) in &by_backend {
        // Sanitize backend name for use as JS identifier
        let js_name = sanitize_identifier(backend_name);

        preamble.push_str(&format!("const {} = {{\n", js_name));
        for tool in backend_tools {
            // Use original_name for JS method names and __ct calls
            // (backends don't know about namespacing)
            let orig = if tool.original_name.is_empty() {
                &tool.name
            } else {
                &tool.original_name
            };
            let tool_js_name = sanitize_identifier(orig);
            preamble.push_str(&format!(
                "  {}: async (args) => await __ct({}, {}, args || {{}}),\n",
                tool_js_name,
                serde_json::to_string(backend_name).unwrap_or_default(),
                serde_json::to_string(orig).unwrap_or_default()
            ));
        }
        preamble.push_str("};\n\n");
    }

    // Generate __interfaces object
    preamble.push_str("const __interfaces = {\n");
    for (backend_name, backend_tools) in &by_backend {
        let js_name = sanitize_identifier(backend_name);
        preamble.push_str(&format!("  \"{}\": {{\n", js_name));
        for tool in backend_tools {
            let orig = if tool.original_name.is_empty() {
                &tool.name
            } else {
                &tool.original_name
            };
            let schema_json = serde_json::to_string(&tool.input_schema).unwrap_or_default();
            let desc_json = serde_json::to_string(&tool.description).unwrap_or_default();
            let name_json = serde_json::to_string(orig).unwrap_or_default();
            preamble.push_str(&format!(
                "    {}: {{ name: {}, description: {}, input_schema: {} }},\n",
                name_json, name_json, desc_json, schema_json
            ));
        }
        preamble.push_str("  },\n");
    }
    preamble.push_str("};\n\n");

    // Generate __getToolInterface function
    preamble.push_str(
        "function __getToolInterface(dotted_name) {\n\
         \x20 const parts = dotted_name.split('.');\n\
         \x20 if (parts.length === 2) {\n\
         \x20   return __interfaces[parts[0]]?.[parts[1]] || null;\n\
         \x20 }\n\
         \x20 // Search all backends for tool name\n\
         \x20 for (const backend of Object.values(__interfaces)) {\n\
         \x20   if (backend[dotted_name]) return backend[dotted_name];\n\
         \x20 }\n\
         \x20 return null;\n\
         }\n\n",
    );

    preamble
}

/// Sanitize a string into a valid JavaScript identifier.
/// Replaces hyphens and other invalid chars with underscores.
pub(crate) fn sanitize_identifier(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
            result.push(c);
        } else {
            result.push('_');
        }
    }
    // Ensure doesn't start with a digit
    if result.starts_with(|c: char| c.is_ascii_digit()) {
        result.insert(0, '_');
    }
    if result.is_empty() {
        result.push_str("_unnamed");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_entry(name: &str, desc: &str, backend: &str) -> ToolEntry {
        ToolEntry {
            name: name.to_string(),
            original_name: name.to_string(),
            description: desc.to_string(),
            backend_name: backend.to_string(),
            input_schema: json!({"type": "object", "properties": {"query": {"type": "string"}}}),
        }
    }

    #[test]
    fn test_sanitize_identifier() {
        assert_eq!(sanitize_identifier("exa"), "exa");
        assert_eq!(sanitize_identifier("my-backend"), "my_backend");
        assert_eq!(sanitize_identifier("123start"), "_123start");
        assert_eq!(sanitize_identifier("valid_name"), "valid_name");
        assert_eq!(sanitize_identifier("has.dots"), "has_dots");
    }

    #[test]
    fn test_generate_preamble_basic() {
        let tools = vec![
            make_entry("web_search", "Search the web", "exa"),
            make_entry("find_similar", "Find similar pages", "exa"),
            make_entry("tavily_search", "Search with Tavily", "tavily"),
        ];

        let preamble = generate_preamble(&tools);

        // Should contain backend accessor objects
        assert!(preamble.contains("const exa = {"));
        assert!(preamble.contains("const tavily = {"));

        // Should contain tool methods
        assert!(preamble.contains("web_search: async (args)"));
        assert!(preamble.contains("find_similar: async (args)"));
        assert!(preamble.contains("tavily_search: async (args)"));

        // Should contain __call_tool references with JSON-escaped strings
        assert!(preamble.contains(r#"__ct("exa", "web_search""#));
        assert!(preamble.contains(r#"__ct("tavily", "tavily_search""#));

        // Should contain __interfaces
        assert!(preamble.contains("const __interfaces = {"));

        // Should contain __getToolInterface
        assert!(preamble.contains("function __getToolInterface(dotted_name)"));
    }

    #[test]
    fn test_generate_preamble_hyphenated_backend() {
        let tools = vec![make_entry("search", "Search", "my-search-backend")];

        let preamble = generate_preamble(&tools);

        // Hyphenated name should be sanitized
        assert!(preamble.contains("const my_search_backend = {"));
        // But the __call_tool should use the original name (JSON-escaped)
        assert!(preamble.contains(r#"__ct("my-search-backend", "search""#));
    }

    #[test]
    fn test_generate_preamble_escapes_special_chars() {
        // Tool names come from MCP backends and could contain quotes/backslashes
        let tools = vec![
            make_entry("tool'quote", "A tool with quote", "normal"),
            make_entry("tool\\slash", "A tool with backslash", "normal"),
        ];

        let preamble = generate_preamble(&tools);

        // Backend/tool names in __ct() must be JSON-escaped strings, not raw interpolation
        assert!(
            !preamble.contains("__ct('normal', 'tool'quote'"),
            "raw single-quoted name with embedded quote creates invalid JS"
        );

        // After fix: names should appear as JSON strings (double-quoted, escaped)
        assert!(
            preamble.contains(r#"__ct("normal", "tool'quote""#),
            "names should be JSON-escaped strings"
        );
        assert!(
            preamble.contains(r#"__ct("normal", "tool\\slash""#),
            "backslashes should be escaped"
        );
    }

    #[test]
    fn test_generate_preamble_empty() {
        let preamble = generate_preamble(&[]);
        assert!(preamble.contains("const __interfaces = {"));
        assert!(preamble.contains("function __getToolInterface"));
    }

    #[test]
    fn test_generate_preamble_contains_require_shim() {
        let preamble = generate_preamble(&[]);
        assert!(
            preamble.contains("function require(module)"),
            "preamble should contain require() shim"
        );
        assert!(
            preamble.contains("not available"),
            "require shim should explain the error"
        );
        assert!(
            preamble.contains("const process = undefined;"),
            "preamble should neutralize Node.js globals"
        );
        assert!(
            preamble.contains("const Buffer = undefined;"),
            "preamble should neutralize Buffer"
        );
    }
}
