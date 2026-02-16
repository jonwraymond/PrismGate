use anyhow::{Context, Result};
use serde_json::Value;
use std::sync::Arc;
use tracing::{debug, warn};

use crate::backend::BackendManager;
use crate::registry::ToolRegistry;

/// Handle call_tool_chain: execute TypeScript code that can call backend tools.
///
/// Strategy:
/// 1. Try direct tool call parsing (fast path for simple `backend.tool(args)` patterns)
/// 2. If that fails and the sandbox feature is enabled, execute in the V8 sandbox
/// 3. If sandbox is not available, return an error
#[allow(unused_variables)]
pub async fn handle_call_tool_chain(
    registry: &Arc<ToolRegistry>,
    manager: &Arc<BackendManager>,
    code: &str,
    timeout: Option<u64>,
    max_output_size: Option<usize>,
) -> Result<String> {
    let max_output = max_output_size.unwrap_or(200_000);

    // Try to parse as a direct tool call (fast path).
    // Pattern: `await manual_name.tool_name({...})` or JSON with tool_name + arguments
    if let Some(result) = try_direct_tool_call(registry, manager, code).await {
        return result.map(|v| truncate_output(&v, max_output));
    }

    // Fall back to full TypeScript sandbox
    #[cfg(feature = "sandbox")]
    #[allow(clippy::needless_return)]
    {
        let timeout_dur = std::time::Duration::from_millis(timeout.unwrap_or(30_000));
        let result = crate::sandbox::execute(
            registry,
            manager,
            code,
            timeout_dur,
            None, // use default V8 heap size (50MB)
        )
        .await?;
        return Ok(truncate_output(&result, max_output));
    }

    #[cfg(not(feature = "sandbox"))]
    anyhow::bail!(
        "TypeScript sandbox not available (compile with 'sandbox' feature). \
         For direct tool calls, use the pattern: \
         `const result = await backend_name.tool_name({{\"param\": \"value\"}}); return result;`"
    )
}

/// Try to parse the code as a simple direct tool call.
///
/// Supports patterns like:
/// - JSON: `{"tool": "backend.tool_name", "arguments": {...}}`
/// - Simple: `backend_name.tool_name({"arg": "val"})`
async fn try_direct_tool_call(
    registry: &Arc<ToolRegistry>,
    manager: &Arc<BackendManager>,
    code: &str,
) -> Option<Result<String>> {
    let code = code.trim();

    // Try JSON format first
    if let Ok(parsed) = serde_json::from_str::<Value>(code)
        && let Some(tool) = parsed.get("tool").and_then(|v| v.as_str())
    {
        let arguments = parsed.get("arguments").cloned();
        return Some(call_tool_by_dotted_name(registry, manager, tool, arguments).await);
    }

    // Try to extract `backend.tool(args)` pattern from simple code
    // Look for pattern: `something.tool_name({...})`
    let code_clean = code
        .replace("const result = ", "")
        .replace("await ", "")
        .replace("return result;", "")
        .replace("return result", "")
        .trim()
        .to_string();

    // Match: `name.tool({...})` or `name.tool({...});`
    if let Some(dot_pos) = code_clean.find('.') {
        let backend_name = code_clean[..dot_pos].trim();
        let rest = &code_clean[dot_pos + 1..];

        if let Some(paren_pos) = rest.find('(') {
            let tool_name = rest[..paren_pos].trim();
            let args_str = rest[paren_pos + 1..].trim_end_matches([')', ';']);
            let args_str = args_str.trim();

            let arguments = if args_str.is_empty() {
                None
            } else {
                match serde_json::from_str::<Value>(args_str) {
                    Ok(v) => Some(v),
                    Err(_) => return None, // Not a simple call, needs real TS execution
                }
            };

            let dotted = format!("{}.{}", backend_name, tool_name);
            debug!(pattern = %dotted, "parsed direct tool call from code");
            return Some(call_tool_by_dotted_name(registry, manager, &dotted, arguments).await);
        }
    }

    None
}

/// Call a tool using "backend.tool_name" dotted notation, or just "tool_name".
async fn call_tool_by_dotted_name(
    registry: &Arc<ToolRegistry>,
    manager: &Arc<BackendManager>,
    dotted_name: &str,
    arguments: Option<Value>,
) -> Result<String> {
    // Resolve the tool name and backend
    let (backend_name, tool_name) = if let Some(dot_pos) = dotted_name.find('.') {
        let bn = &dotted_name[..dot_pos];
        let tn = &dotted_name[dot_pos + 1..];
        (bn.to_string(), tn.to_string())
    } else {
        // Just a tool name — look up the backend from registry
        let entry = registry
            .get_by_name(dotted_name)
            .ok_or_else(|| anyhow::anyhow!("tool '{}' not found", dotted_name))?;
        (entry.backend_name.clone(), dotted_name.to_string())
    };

    // Look up tool in registry to verify it exists
    let entry = registry
        .get_by_name(&tool_name)
        .ok_or_else(|| anyhow::anyhow!("tool '{}' not found in registry", tool_name))?;

    // Use the actual backend from the registry entry (it's authoritative)
    if entry.backend_name != backend_name {
        warn!(
            expected_backend = %backend_name,
            actual_backend = %entry.backend_name,
            tool = %tool_name,
            "backend mismatch in dotted name, using actual backend"
        );
    }

    let result = manager
        .call_tool(&entry.backend_name, &tool_name, arguments.clone())
        .await;

    let value = match result {
        Ok(v) => v,
        Err(e) if e.to_string().contains("not available") && e.to_string().contains("Stopped") => {
            // Attempt on-demand restart and retry once
            debug!(backend = %entry.backend_name, tool = %tool_name, "attempting on-demand restart for stopped backend");
            manager
                .restart_backend(&entry.backend_name, registry)
                .await
                .with_context(|| format!("on-demand restart of '{}' failed", entry.backend_name))?;
            manager
                .call_tool(&entry.backend_name, &tool_name, arguments)
                .await
                .with_context(|| {
                    format!(
                        "retry after restart: tool '{}' on '{}'",
                        tool_name, entry.backend_name
                    )
                })?
        }
        Err(e) => {
            return Err(e).with_context(|| {
                format!("tool '{}' on backend '{}'", tool_name, entry.backend_name)
            });
        }
    };

    serde_json::to_string_pretty(&value)
        .map_err(|e| anyhow::anyhow!("failed to serialize tool result: {e}"))
}

fn truncate_output(s: &str, max_size: usize) -> String {
    if s.len() <= max_size {
        s.to_string()
    } else {
        let boundary = s.floor_char_boundary(max_size);
        let mut truncated = s[..boundary].to_string();
        truncated.push_str("\n... [output truncated]");
        truncated
    }
}
