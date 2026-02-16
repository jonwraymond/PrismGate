use anyhow::{Context, Result};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::debug;

use crate::backend::BackendManager;
use crate::registry::ToolRegistry;

/// Handle call_tool_chain: execute TypeScript code that can call backend tools.
///
/// Strategy:
/// 1. Try direct tool call parsing (fast path — bypasses sandbox semaphore)
/// 2. If that fails and the sandbox feature is enabled, acquire sandbox semaphore
///    and execute in the V8 sandbox
/// 3. If sandbox is not available, return an error
#[allow(unused_variables)]
pub async fn handle_call_tool_chain(
    registry: &Arc<ToolRegistry>,
    manager: &Arc<BackendManager>,
    code: &str,
    timeout: Option<u64>,
    max_output_size: Option<usize>,
    sandbox_semaphore: &Semaphore,
) -> Result<String> {
    let max_output = max_output_size.unwrap_or(200_000);

    // Try to parse as a direct tool call (fast path — no V8, no semaphore needed).
    // Pattern: `await manual_name.tool_name({...})` or JSON with tool_name + arguments
    if let Some(result) = try_direct_tool_call(registry, manager, code).await {
        return result.map(|v| truncate_output(&v, max_output));
    }

    // Fall back to full TypeScript sandbox — acquire semaphore first
    #[cfg(feature = "sandbox")]
    #[allow(clippy::needless_return)]
    {
        let timeout_dur = std::time::Duration::from_millis(timeout.unwrap_or(30_000));

        // Acquire sandbox semaphore to limit concurrent V8 isolates.
        // Use the call's own timeout — no point waiting longer for a permit.
        let _permit = match tokio::time::timeout(timeout_dur, sandbox_semaphore.acquire()).await {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) => anyhow::bail!("sandbox semaphore closed"),
            Err(_) => anyhow::bail!(
                "sandbox concurrency limit reached. All V8 isolates are busy. \
                 Try again shortly or increase max_concurrent_sandboxes in config."
            ),
        };
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
///
/// After namespacing, tools may exist under both `backend.tool_name` and bare `tool_name`
/// (if no collision). This function resolves both forms and always passes the
/// `original_name` to the backend MCP server (which doesn't know about namespacing).
async fn call_tool_by_dotted_name(
    registry: &Arc<ToolRegistry>,
    manager: &Arc<BackendManager>,
    dotted_name: &str,
    arguments: Option<Value>,
) -> Result<String> {
    // Resolve: try looking up the full dotted name first (handles both namespaced and bare)
    let entry = if let Some(e) = registry.get_by_name(dotted_name) {
        e
    } else if let Some(dot_pos) = dotted_name.find('.') {
        // Try interpreting as backend.tool — look up the namespaced key
        let bn = &dotted_name[..dot_pos];
        let tn = &dotted_name[dot_pos + 1..];
        // Fall back: the tool might be registered under the bare name with this backend
        registry
            .get_by_name(&format!("{}.{}", bn, tn))
            .or_else(|| {
                // Maybe the bare name resolves and belongs to this backend
                registry.get_by_name(tn).filter(|e| e.backend_name == bn)
            })
            .ok_or_else(|| anyhow::anyhow!("tool '{}' not found in registry", dotted_name))?
    } else {
        return Err(anyhow::anyhow!("tool '{}' not found", dotted_name));
    };

    // CRITICAL: pass original_name to backend, not the namespaced registry key
    let call_name = if entry.original_name.is_empty() {
        &entry.name
    } else {
        &entry.original_name
    };

    let result = manager
        .call_tool(&entry.backend_name, call_name, arguments.clone())
        .await;

    let value = match result {
        Ok(v) => v,
        Err(e) if e.to_string().contains("not available") && e.to_string().contains("Stopped") => {
            // Attempt on-demand restart and retry once
            debug!(backend = %entry.backend_name, tool = %call_name, "attempting on-demand restart for stopped backend");
            manager
                .restart_backend(&entry.backend_name, registry)
                .await
                .with_context(|| format!("on-demand restart of '{}' failed", entry.backend_name))?;
            manager
                .call_tool(&entry.backend_name, call_name, arguments)
                .await
                .with_context(|| {
                    format!(
                        "retry after restart: tool '{}' on '{}'",
                        call_name, entry.backend_name
                    )
                })?
        }
        Err(e) => {
            return Err(e).with_context(|| {
                format!("tool '{}' on backend '{}'", call_name, entry.backend_name)
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
