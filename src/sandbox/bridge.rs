use crate::registry::ToolEntry;
use std::collections::HashMap;

#[cfg(any(feature = "sandbox", test))]
pub use handles::DataHandleStore;

#[cfg(any(feature = "sandbox", test))]
mod handles {
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    const MAX_BYTES: usize = 64 * 1024 * 1024;
    const MAX_ENTRIES: usize = 256;
    const TTL: Duration = Duration::from_secs(3600);

    struct Entry {
        session: Option<u64>,
        value: Value,
        bytes: usize,
        created: Instant,
    }

    /// Process-local JSON handles: survive fresh V8 runtimes, not daemon restarts.
    /// Session-scoped, bounded to 64 MiB/256 entries with a one-hour lifetime.
    /// Capacity failures are explicit: live handles are never silently evicted.
    #[derive(Default)]
    pub struct DataHandleStore {
        entries: Mutex<HashMap<String, Entry>>,
    }

    impl DataHandleStore {
        pub fn store(&self, session: Option<u64>, value: Value) -> Result<String, String> {
            let bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?.len();
            let mut entries = self
                .entries
                .lock()
                .map_err(|_| "data store lock poisoned")?;
            entries.retain(|_, entry| entry.created.elapsed() < TTL);
            if entries.len() >= MAX_ENTRIES
                || bytes > MAX_BYTES.saturating_sub(entries.values().map(|e| e.bytes).sum())
            {
                return Err(
                    "data store capacity exceeded; remove handles before storing more".into(),
                );
            }
            let handle = loop {
                let candidate = format!("data_{:032x}", rand::random::<u128>());
                if !entries.contains_key(&candidate) {
                    break candidate;
                }
            };
            entries.insert(
                handle.clone(),
                Entry {
                    session,
                    value,
                    bytes,
                    created: Instant::now(),
                },
            );
            Ok(handle)
        }

        pub fn load(&self, session: Option<u64>, handle: &str) -> Result<Value, String> {
            self.query(session, handle, "", 0, usize::MAX)
        }

        /// Resolve an RFC 6901 JSON pointer, then page arrays without copying the
        /// rest of the document into V8. Scalars/objects are returned unchanged.
        pub fn query(
            &self,
            session: Option<u64>,
            handle: &str,
            pointer: &str,
            offset: usize,
            limit: usize,
        ) -> Result<Value, String> {
            let mut entries = self
                .entries
                .lock()
                .map_err(|_| "data store lock poisoned")?;
            entries.retain(|_, entry| entry.created.elapsed() < TTL);
            let entry = entries
                .get(handle)
                .filter(|e| e.session == session)
                .ok_or("data handle not found or expired in this session")?;
            if !pointer.is_empty() && !pointer.starts_with('/') {
                return Err("query pointer must be empty or start with '/' (RFC 6901)".into());
            }
            let value = entry
                .value
                .pointer(pointer)
                .ok_or("query pointer not found")?;
            Ok(match value.as_array() {
                Some(rows) => Value::Array(rows.iter().skip(offset).take(limit).cloned().collect()),
                None => value.clone(),
            })
        }

        pub fn remove(&self, session: Option<u64>, handle: &str) -> Result<(), String> {
            let mut entries = self
                .entries
                .lock()
                .map_err(|_| "data store lock poisoned")?;
            if !entries
                .get(handle)
                .is_some_and(|e| e.session == session && e.created.elapsed() < TTL)
            {
                return Err("data handle not found or expired in this session".into());
            }
            entries.remove(handle);
            Ok(())
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use serde_json::json;

        #[test]
        fn expiry_capacity_and_pointer_edges() {
            let store = DataHandleStore::default();
            let h = store
                .store(None, json!({"a/b": {"~key": [null, 1]}}))
                .unwrap();
            assert_eq!(
                store.query(None, &h, "/a~1b/~0key", 0, 1).unwrap(),
                json!([null])
            );
            assert_eq!(
                store.query(None, &h, "/a~1b/~0key", usize::MAX, 5).unwrap(),
                json!([])
            );
            assert!(store.query(None, &h, "a/b", 0, 1).is_err());
            assert!(store.remove(Some(1), &h).is_err());
            store.entries.lock().unwrap().get_mut(&h).unwrap().created = Instant::now() - TTL;
            assert!(store.load(None, &h).is_err());
            for _ in 0..MAX_ENTRIES {
                store.store(None, json!(null)).unwrap();
            }
            assert!(
                store
                    .store(None, json!(1))
                    .unwrap_err()
                    .contains("capacity")
            );
        }
    }
}

/// Validate capture before dispatch so invalid options cannot trigger side effects.
#[cfg(feature = "sandbox")]
pub fn capture_requested(options: Option<&serde_json::Value>) -> Result<bool, rustyscript::Error> {
    match options {
        None | Some(serde_json::Value::Null) => Ok(false),
        Some(serde_json::Value::Object(options)) if options.keys().all(|k| k == "capture") => {
            match options.get("capture") {
                None => Ok(false),
                Some(serde_json::Value::Bool(capture)) => Ok(*capture),
                _ => Err(rustyscript::Error::Runtime(
                    "capture must be a boolean".into(),
                )),
            }
        }
        _ => Err(rustyscript::Error::Runtime(
            "tool options must be {capture: boolean}".into(),
        )),
    }
}

/// Capture the raw successful response before it crosses the V8 boundary.
/// MCP operation errors remain inline; a storage failure never replays the call.
#[cfg(feature = "sandbox")]
pub fn capture_output(
    session: Option<u64>,
    capture: bool,
    value: serde_json::Value,
) -> Result<serde_json::Value, rustyscript::Error> {
    if !capture || value.get("isError").and_then(|v| v.as_bool()) == Some(true) {
        return Ok(value);
    }
    data_store()
        .store(session, value)
        .map(serde_json::Value::String)
        .map_err(|e| {
            rustyscript::Error::Runtime(format!(
                "tool completed but result capture failed: {e}; do not replay side effects"
            ))
        })
}

/// Shared host-side store; no backend dispatch is performed by data operations.
#[cfg(feature = "sandbox")]
pub fn data_store() -> &'static DataHandleStore {
    static STORE: std::sync::OnceLock<DataHandleStore> = std::sync::OnceLock::new();
    STORE.get_or_init(DataHandleStore::default)
}

#[cfg(feature = "sandbox")]
pub fn register_data_helpers(
    runtime: &mut rustyscript::Runtime,
    session: Option<u64>,
) -> Result<(), rustyscript::Error> {
    runtime.register_async_function("__data_op", move |args: Vec<serde_json::Value>| {
        Box::pin(async move {
            let result = (|| -> Result<serde_json::Value, String> {
                let string = |index: usize| {
                    args.get(index)
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| format!("data argument {index} must be a string"))
                };
                let store = data_store();
                match string(0)? {
                    "store" => store
                        .store(
                            session,
                            args.get(1).ok_or("store requires a JSON value")?.clone(),
                        )
                        .map(serde_json::Value::String),
                    "load" => store.load(session, string(1)?),
                    "remove" => store
                        .remove(session, string(1)?)
                        .map(|_| serde_json::Value::Bool(true)),
                    "query" => {
                        let number = |index: usize| {
                            args.get(index)
                                .and_then(|v| v.as_u64())
                                .and_then(|v| usize::try_from(v).ok())
                                .ok_or_else(|| {
                                    format!("data argument {index} must be a nonnegative integer")
                                })
                        };
                        store.query(session, string(1)?, string(2)?, number(3)?, number(4)?)
                    }
                    _ => Err("unknown data operation".into()),
                }
            })();
            result.map_err(rustyscript::Error::Runtime)
        })
    })
}

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
    preamble.push_str(r#"
// Handles survive executions in this session (one hour, daemon lifetime).
// query uses an RFC 6901 pointer plus optional array offset/limit.
const __data = Object.freeze({
  store: async (value) => await rustyscript.async_functions.__data_op('store', value),
  load: async (handle) => await rustyscript.async_functions.__data_op('load', handle),
  query: async (handle, pointer = '', offset = 0, limit = 100) => await rustyscript.async_functions.__data_op('query', handle, pointer, offset, limit),
  remove: async (handle) => await rustyscript.async_functions.__data_op('remove', handle),
  capture: async (backend, tool, args = {}) => await __ct(backend, tool, args, {capture: true}),
});
"#);

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

    // Generate __backends map for dynamic dispatch.
    // Maps both sanitized names ("my_backend") and original names ("my-backend")
    // to the const backend objects, enabling patterns like:
    //   const r = await __backends["auggie"].codebase_retrieval({...});
    //   const r = await __backends[backendName][toolName]({...});
    preamble.push_str("const __backends = {\n");
    for backend_name in by_backend.keys() {
        let js_name = sanitize_identifier(backend_name);
        // Always map the sanitized name
        preamble.push_str(&format!(
            "  {}: {},\n",
            serde_json::to_string(&js_name).unwrap_or_default(),
            js_name
        ));
        // Also map the original name if different (e.g., "my-backend" -> my_backend)
        if js_name != *backend_name {
            preamble.push_str(&format!(
                "  {}: {},\n",
                serde_json::to_string(backend_name).unwrap_or_default(),
                js_name
            ));
        }
    }
    preamble.push_str("};\n\n");

    // Mirror backends onto globalThis so `globalThis['auggie']` and
    // `globalThis.exa` work — LLMs frequently generate this pattern despite
    // instructions saying backends are top-level variables.
    for backend_name in by_backend.keys() {
        let js_name = sanitize_identifier(backend_name);
        preamble.push_str(&format!("globalThis['{}'] = {};\n", js_name, js_name));
        if js_name != *backend_name {
            preamble.push_str(&format!("globalThis['{}'] = {};\n", backend_name, js_name));
        }
    }
    preamble.push('\n');

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
            tags: Vec::new(),
        }
    }

    #[test]
    fn data_handle_lifecycle_and_query() {
        let store = DataHandleStore::default();
        let handle = store.store(Some(7), json!({"rows": [1, 2, 3]})).unwrap();
        assert_eq!(
            store.load(Some(7), &handle).unwrap(),
            json!({"rows": [1, 2, 3]})
        );
        assert!(store.load(Some(8), &handle).is_err());
        assert_eq!(
            store.query(Some(7), &handle, "/rows", 1, 1).unwrap(),
            json!([2])
        );
        assert!(store.query(Some(7), &handle, "/missing", 0, 10).is_err());
        store.remove(Some(7), &handle).unwrap();
        assert!(store.load(Some(7), &handle).is_err());
    }

    #[cfg(feature = "sandbox")]
    #[test]
    fn handles_persist_across_fresh_runtimes() {
        fn run(code: &str, session: Option<u64>) -> serde_json::Value {
            let mut runtime = rustyscript::Runtime::new(rustyscript::RuntimeOptions {
                default_entrypoint: Some("main".into()),
                ..Default::default()
            })
            .unwrap();
            register_data_helpers(&mut runtime, session).unwrap();
            let module = rustyscript::Module::new(
                "handles.js",
                format!(
                    "{}\nexport default async function main() {{ {code} }}",
                    generate_preamble(&[])
                ),
            );
            let module = runtime.load_module(&module).unwrap();
            runtime
                .call_entrypoint(&module, rustyscript::json_args!())
                .unwrap()
        }
        let handle = run(
            "return await __data.store({rows: [10, 20, 30]});",
            Some(901),
        );
        let code = format!("return await __data.query({handle}, '/rows', 1, 1);");
        assert_eq!(run(&code, Some(901)), json!([20]));
        assert_eq!(
            run(&format!("return await __data.load({handle});"), Some(901)),
            json!({"rows": [10,20,30]})
        );
        assert!(
            run(
                &format!("try {{ await __data.load({handle}); }} catch(e) {{ return e.message; }}"),
                Some(902)
            )
            .as_str()
            .unwrap()
            .contains("not found")
        );
        assert_eq!(
            run(&format!("return await __data.remove({handle});"), Some(901)),
            json!(true)
        );
    }

    #[cfg(feature = "sandbox")]
    #[test]
    fn capture_validates_options_and_preserves_backend_errors() {
        assert!(capture_requested(Some(&json!({"capture": "yes"}))).is_err());
        assert!(!capture_requested(None).unwrap());
        assert!(capture_requested(Some(&json!({"capture": true}))).unwrap());
        let failure = json!({"isError": true, "content": [{"text": "failed"}]});
        assert_eq!(
            capture_output(Some(991), true, failure.clone()).unwrap(),
            failure
        );
        let value = json!({"content": [{"text": "payload"}]});
        let handle = capture_output(Some(991), true, value.clone()).unwrap();
        assert_eq!(
            data_store()
                .load(Some(991), handle.as_str().unwrap())
                .unwrap(),
            value
        );
        data_store()
            .remove(Some(991), handle.as_str().unwrap())
            .unwrap();
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
    fn test_generate_preamble_backends_map() {
        let tools = vec![
            make_entry("web_search", "Search", "exa"),
            make_entry("search", "Search", "my-search-backend"),
        ];
        let preamble = generate_preamble(&tools);

        // Should contain __backends map
        assert!(preamble.contains("const __backends = {"));
        // Should map sanitized name to the const variable
        assert!(preamble.contains(r#""exa": exa"#));
        assert!(preamble.contains(r#""my_search_backend": my_search_backend"#));
        // Should also map original hyphenated name
        assert!(preamble.contains(r#""my-search-backend": my_search_backend"#));
    }

    #[test]
    fn test_generate_preamble_empty() {
        let preamble = generate_preamble(&[]);
        assert!(preamble.contains("const __interfaces = {"));
        assert!(preamble.contains("const __backends = {"));
        assert!(preamble.contains("function __getToolInterface"));
    }

    #[test]
    fn test_generate_preamble_globalthis_mirrors() {
        let tools = vec![
            make_entry("web_search", "Search", "exa"),
            make_entry("search", "Search", "my-search-backend"),
        ];
        let preamble = generate_preamble(&tools);

        // Sanitized names mirrored to globalThis
        assert!(preamble.contains("globalThis['exa'] = exa;"));
        assert!(preamble.contains("globalThis['my_search_backend'] = my_search_backend;"));
        // Original hyphenated name also mirrored
        assert!(preamble.contains("globalThis['my-search-backend'] = my_search_backend;"));
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
