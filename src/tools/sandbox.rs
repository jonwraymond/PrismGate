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
#[allow(unused_variables, clippy::too_many_arguments)]
pub async fn handle_call_tool_chain(
    registry: &Arc<ToolRegistry>,
    manager: &Arc<BackendManager>,
    code: &str,
    timeout: Option<u64>,
    max_output_size: Option<usize>,
    sandbox_semaphore: &Semaphore,
    session_id: Option<u64>,
    intent: Option<&str>,
) -> Result<String> {
    let max_output = max_output_size.unwrap_or(200_000);

    // Try to parse as a direct tool call (fast path — no V8, no semaphore needed).
    // Pattern: `await manual_name.tool_name({...})` or JSON with tool_name + arguments
    if let Some(result) = try_direct_tool_call(registry, manager, code, session_id).await {
        return result.map(|v| {
            let filtered = if let Some(intent) = intent {
                filter_by_intent(&v, intent)
            } else {
                v
            };
            truncate_output(&filtered, max_output)
        });
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
            session_id,
        )
        .await?;
        let filtered = if let Some(intent) = intent {
            filter_by_intent(&result, intent)
        } else {
            result
        };
        return Ok(truncate_output(&filtered, max_output));
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
    session_id: Option<u64>,
) -> Option<Result<String>> {
    let code = code.trim();

    // Try JSON format first
    if let Ok(parsed) = serde_json::from_str::<Value>(code)
        && let Some(tool) = parsed.get("tool").and_then(|v| v.as_str())
    {
        let arguments = parsed.get("arguments").cloned();
        return Some(
            call_tool_by_dotted_name(registry, manager, tool, arguments, session_id).await,
        );
    }

    // Try to extract `backend.tool(args)` pattern from simple code
    // Look for pattern: `something.tool_name({...})`
    let code_clean = code
        .replace("const result = ", "")
        .replace("await ", "")
        .replace("return result;", "")
        .replace("return result", "")
        .trim()
        .trim_start_matches("return ")
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
            return Some(
                call_tool_by_dotted_name(registry, manager, &dotted, arguments, session_id).await,
            );
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
    session_id: Option<u64>,
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

    // Use call_tool_with_fallback to enable automatic failover on transient errors
    let result = manager
        .call_tool_with_fallback(
            &entry.backend_name,
            call_name,
            &entry.original_name,
            arguments.clone(),
            registry,
            session_id,
        )
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
                .call_tool(&entry.backend_name, call_name, arguments, session_id)
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

/// Threshold below which intent filtering is skipped (output is small enough to return raw).
const INTENT_SEARCH_THRESHOLD: usize = 5_000;

/// Filter output by intent relevance. When output exceeds 5KB and an intent is provided,
/// splits the output into chunks and returns only those matching the intent terms.
fn filter_by_intent(output: &str, intent: &str) -> String {
    if output.len() < INTENT_SEARCH_THRESHOLD {
        return output.to_string();
    }

    let intent_terms = crate::registry::tokenize(intent);
    if intent_terms.is_empty() {
        return output.to_string();
    }

    // Split into chunks: paragraphs (blank-line separated) or 10-line groups
    let chunks = split_into_chunks(output);
    if chunks.len() <= 1 {
        return output.to_string();
    }

    // Score each chunk by fraction of intent terms present
    let scored: Vec<(&str, f64)> = chunks
        .iter()
        .map(|chunk| {
            let chunk_terms: Vec<String> = crate::registry::tokenize(chunk);
            let hits = intent_terms
                .iter()
                .filter(|t| chunk_terms.contains(t))
                .count();
            (*chunk, hits as f64 / intent_terms.len().max(1) as f64)
        })
        .collect();

    // Keep chunks with score > 0.3 (at least 30% of intent terms present)
    let filtered: Vec<&str> = scored
        .iter()
        .filter(|(_, s)| *s > 0.3)
        .map(|(c, _)| *c)
        .collect();

    if filtered.is_empty() {
        // No matches — return original unchanged
        return output.to_string();
    }

    format!(
        "[Filtered by intent: '{}' — {}/{} sections]\n\n{}",
        intent,
        filtered.len(),
        chunks.len(),
        filtered.join("\n\n")
    )
}

/// Split text into chunks: paragraphs (blank-line separated) if there are enough,
/// otherwise 10-line groups.
fn split_into_chunks(text: &str) -> Vec<&str> {
    // Try paragraph splitting first
    let paragraphs: Vec<&str> = text
        .split("\n\n")
        .filter(|p| !p.trim().is_empty())
        .collect();
    if paragraphs.len() >= 3 {
        return paragraphs;
    }

    // Fall back to 10-line groups
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= 10 {
        return vec![text];
    }
    lines
        .chunks(10)
        .map(|chunk| {
            let start = chunk[0].as_ptr() as usize - text.as_ptr() as usize;
            let end = chunk
                .last()
                .map(|l| l.as_ptr() as usize + l.len() - text.as_ptr() as usize)
                .unwrap_or(start);
            &text[start..end]
        })
        .collect()
}

/// Smart truncation: keeps head 60% + tail 40% of output, snapped to line boundaries.
///
/// This preserves both the beginning (setup, context) and end (results, errors,
/// summaries) of tool output — the tail is often the most important part that
/// a simple head-only cutoff would lose.
fn truncate_output(s: &str, max_size: usize) -> String {
    if s.len() <= max_size {
        return s.to_string();
    }

    let lines: Vec<&str> = s.split_inclusive('\n').collect();

    // Single giant line (no newlines) — byte-level fallback
    if lines.len() <= 1 {
        let head_end = s.floor_char_boundary((max_size as f64 * 0.6) as usize);
        let tail_start =
            s.ceil_char_boundary(s.len().saturating_sub((max_size as f64 * 0.4) as usize));
        if tail_start <= head_end {
            // Overlap — just do a simple head cut
            let boundary = s.floor_char_boundary(max_size);
            return format!("{}\n... [output truncated]", &s[..boundary]);
        }
        return format!(
            "{}\n... [truncated middle — {:.1}KB omitted] ...\n{}",
            &s[..head_end],
            (tail_start - head_end) as f64 / 1024.0,
            &s[tail_start..]
        );
    }

    let head_budget = (max_size as f64 * 0.6) as usize;
    let tail_budget = max_size.saturating_sub(head_budget);

    // Walk forward for head lines
    let mut head_bytes = 0;
    let mut head_count = 0;
    for line in &lines {
        if head_bytes + line.len() > head_budget {
            break;
        }
        head_bytes += line.len();
        head_count += 1;
    }
    // Ensure at least 1 head line
    if head_count == 0 && !lines.is_empty() {
        head_bytes = lines[0].len();
        head_count = 1;
    }

    // Walk backward for tail lines
    let mut tail_bytes = 0;
    let mut tail_count = 0;
    for line in lines.iter().rev() {
        if tail_bytes + line.len() > tail_budget {
            break;
        }
        tail_bytes += line.len();
        tail_count += 1;
    }

    // Prevent overlap (head and tail selecting the same lines)
    if head_count + tail_count > lines.len() {
        tail_count = lines.len().saturating_sub(head_count);
    }

    let omitted_lines = lines.len() - head_count - tail_count;
    let omitted_bytes = s.len() - head_bytes - tail_bytes;

    if omitted_lines == 0 {
        // Nothing to omit — everything fits
        return s.to_string();
    }

    let head_text = &s[..head_bytes];
    let tail_start = s.len() - tail_bytes;
    let tail_text = &s[tail_start..];

    format!(
        "{}\n... [{} lines / {:.1}KB truncated — showing first {} + last {} lines] ...\n{}",
        head_text,
        omitted_lines,
        omitted_bytes as f64 / 1024.0,
        head_count,
        tail_count,
        tail_text
    )
}

#[cfg(test)]
mod truncation_tests {
    use super::*;

    #[test]
    fn test_truncate_preserves_small() {
        let small = "hello\nworld\n";
        assert_eq!(truncate_output(small, 100), small);
    }

    #[test]
    fn test_truncate_head_tail_split() {
        // 20 lines of 10 bytes each = 200 bytes total
        let lines: String = (1..=20).map(|i| format!("line {:04}\n", i)).collect();
        assert_eq!(lines.len(), 200);

        // Allow 100 bytes: head 60 bytes (~6 lines), tail 40 bytes (~4 lines)
        let result = truncate_output(&lines, 100);
        assert!(result.contains("line 0001"));
        assert!(result.contains("line 0020")); // tail preserved
        assert!(result.contains("truncated"));
        assert!(result.contains("showing first"));
        // Middle lines should be omitted
        assert!(!result.contains("line 0010"));
    }

    #[test]
    fn test_truncate_single_long_line() {
        let long = "x".repeat(1000);
        let result = truncate_output(&long, 200);
        assert!(result.contains("truncated middle"));
        // Head should be ~120 chars (60% of 200), tail ~80 chars (40% of 200)
        assert!(result.len() < 300); // head + tail + message
    }

    #[test]
    fn test_truncate_message_format() {
        let lines: String = (1..=100).map(|i| format!("line {:04}\n", i)).collect();
        let result = truncate_output(&lines, 200);
        // Should contain the descriptive truncation message
        assert!(result.contains("lines /"));
        assert!(result.contains("KB truncated"));
        assert!(result.contains("showing first"));
        assert!(result.contains("last"));
    }

    #[test]
    fn test_truncate_preserves_exact_fit() {
        let lines = "aaa\nbbb\nccc\n";
        // Exactly at limit — should return unchanged
        assert_eq!(truncate_output(lines, lines.len()), lines);
    }

    #[test]
    fn test_intent_small_output_passthrough() {
        let small = "This is a small output about errors.";
        assert!(small.len() < INTENT_SEARCH_THRESHOLD);
        assert_eq!(filter_by_intent(small, "errors"), small);
    }

    #[test]
    fn test_intent_filters_relevant_sections() {
        // Build output > 5KB with distinct paragraphs
        let mut output = String::new();
        for i in 0..20 {
            if i == 5 || i == 12 {
                output.push_str(&format!(
                    "Section {i}: This section discusses error handling and retry logic.\n\n"
                ));
            } else {
                output.push_str(&format!(
                    "Section {i}: {} unrelated content padding here.\n\n",
                    "lorem ipsum ".repeat(30)
                ));
            }
        }
        assert!(output.len() > INTENT_SEARCH_THRESHOLD);

        let result = filter_by_intent(&output, "error handling retry");
        assert!(result.contains("Filtered by intent"));
        assert!(result.contains("error handling"));
        // Should not contain all 20 sections
        assert!(result.len() < output.len());
    }

    #[test]
    fn test_intent_no_matches_returns_all() {
        let mut output = String::new();
        for i in 0..20 {
            output.push_str(&format!(
                "Section {i}: {} unrelated content here.\n\n",
                "lorem ipsum ".repeat(30)
            ));
        }
        assert!(output.len() > INTENT_SEARCH_THRESHOLD);

        // Intent with no matching terms — should return original
        let result = filter_by_intent(&output, "xyzzy quantum entanglement");
        assert!(!result.contains("Filtered by intent"));
        assert_eq!(result, output);
    }
}
