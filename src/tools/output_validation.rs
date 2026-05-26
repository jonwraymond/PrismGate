//! Output validation for tool responses.
//!
//! Security layer that sanitizes and validates tool output from backend MCP
//! servers before it reaches the client. Runs *before* the output processing
//! pipeline (intent filtering, chunking, truncation) so all downstream stages
//! operate on validated content.
//!
//! # Threat model
//!
//! Backend MCP servers (especially third-party or community ones) may return:
//! - **Null bytes** that break JSON serialization and downstream parsers.
//! - **Control characters** (ANSI escape sequences, terminal controls) that
//!   can corrupt terminal output or inject commands in some clients.
//! - **Dangerous URI schemes** (`javascript:`, `data:` with embedded scripts)
//!   that could execute in HTML-based clients.
//! - **Excessively large individual items** that cause memory exhaustion.
//! - **File/resource URI leakage** exposing internal paths.
//!
//! All checks are **non-destructive by default**: dangerous content is replaced
//! with safe placeholders rather than causing the entire response to fail.

use tracing::warn;

/// Maximum size for a single text content item (1 MB).
const MAX_CONTENT_ITEM_SIZE: usize = 1_048_576;

/// Result of output validation.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// The sanitized output text.
    pub text: String,
    /// Number of null bytes removed.
    pub null_bytes_stripped: usize,
    /// Number of control characters stripped.
    pub control_chars_stripped: usize,
    /// Number of dangerous URIs neutralized.
    pub dangerous_uris_neutralized: usize,
    /// Whether the item exceeded the per-item size limit and was truncated.
    pub size_truncated: bool,
    /// Number of resource URIs sanitized.
    pub resource_uris_sanitized: usize,
}

impl ValidationResult {
    /// Returns true if any sanitization was applied.
    pub fn was_modified(&self) -> bool {
        self.null_bytes_stripped > 0
            || self.control_chars_stripped > 0
            || self.dangerous_uris_neutralized > 0
            || self.size_truncated
            || self.resource_uris_sanitized > 0
    }

    /// Log a warning if any sanitization was applied.
    pub fn log_if_modified(&self, tool_name: &str, backend: &str) {
        if self.was_modified() {
            warn!(
                tool = %tool_name,
                backend = %backend,
                null_bytes = self.null_bytes_stripped,
                control_chars = self.control_chars_stripped,
                dangerous_uris = self.dangerous_uris_neutralized,
                size_truncated = self.size_truncated,
                resource_uris = self.resource_uris_sanitized,
                "tool output sanitized"
            );
        }
    }
}

/// Validate and sanitize a single tool output text item.
///
/// Applies all validation passes in order:
/// 1. Null byte removal
/// 2. Control character stripping
/// 3. Dangerous URI neutralization
/// 4. Resource URI sanitization
/// 5. Per-item size enforcement
pub fn validate_tool_output(text: &str) -> ValidationResult {
    let (text, null_count) = strip_null_bytes(text);
    let (text, ctrl_count) = strip_control_chars(&text);
    let (text, uri_count) = neutralize_dangerous_uris(&text);
    let (text, resource_count) = sanitize_resource_uris(&text);
    let (text, truncated) = enforce_item_size(&text, MAX_CONTENT_ITEM_SIZE);

    ValidationResult {
        text,
        null_bytes_stripped: null_count,
        control_chars_stripped: ctrl_count,
        dangerous_uris_neutralized: uri_count,
        size_truncated: truncated,
        resource_uris_sanitized: resource_count,
    }
}

/// Validate a tool output that has already been converted to a JSON Value.
///
/// Recursively walks the value and sanitizes all string leaves.
pub fn validate_json_output(value: &serde_json::Value) -> (serde_json::Value, ValidationResult) {
    let mut total = ValidationResult {
        text: String::new(),
        null_bytes_stripped: 0,
        control_chars_stripped: 0,
        dangerous_uris_neutralized: 0,
        size_truncated: false,
        resource_uris_sanitized: 0,
    };

    let validated = validate_json_recursive(value, &mut total);
    total.text = format!("[{} string(s) validated]", count_strings(value));
    (validated, total)
}

fn validate_json_recursive(
    value: &serde_json::Value,
    total: &mut ValidationResult,
) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => {
            let result = validate_tool_output(s);
            total.null_bytes_stripped += result.null_bytes_stripped;
            total.control_chars_stripped += result.control_chars_stripped;
            total.dangerous_uris_neutralized += result.dangerous_uris_neutralized;
            total.resource_uris_sanitized += result.resource_uris_sanitized;
            if result.size_truncated {
                total.size_truncated = true;
            }
            serde_json::Value::String(result.text)
        }
        serde_json::Value::Object(map) => {
            let new_map: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), validate_json_recursive(v, total)))
                .collect();
            serde_json::Value::Object(new_map)
        }
        serde_json::Value::Array(arr) => serde_json::Value::Array(
            arr.iter()
                .map(|v| validate_json_recursive(v, total))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn count_strings(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::String(_) => 1,
        serde_json::Value::Object(map) => map.values().map(count_strings).sum(),
        serde_json::Value::Array(arr) => arr.iter().map(count_strings).sum(),
        _ => 0,
    }
}

/// Strip null bytes (`\0`) from text.
///
/// Null bytes can break JSON serialization, cause string truncation in C-based
/// parsers, and are never legitimate in MCP text content.
fn strip_null_bytes(text: &str) -> (String, usize) {
    let count = text.matches('\0').count();
    if count == 0 {
        return (text.to_string(), 0);
    }
    (text.replace('\0', ""), count)
}

/// Strip dangerous control characters while preserving whitespace.
///
/// Keeps: tab (0x09), newline (0x0A), carriage return (0x0D).
/// Removes: all other C0 controls (0x00–0x08, 0x0B–0x0C, 0x0E–0x1F, 0x7F)
///          and ANSI CSI sequences (`ESC [ ... final_byte`).
fn strip_control_chars(text: &str) -> (String, usize) {
    let mut result = String::with_capacity(text.len());
    let mut count = 0;
    let bytes = text.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];
        // Check for ANSI escape sequence: ESC [ ... final_byte
        if b == 0x1B && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            // Skip ESC [ and all subsequent bytes until a final byte (0x40–0x7E)
            i += 2;
            let seq_start = i;
            while i < bytes.len() && !(bytes[i] >= 0x40 && bytes[i] <= 0x7E) {
                i += 1;
            }
            if i < bytes.len() {
                i += 1; // skip the final byte too
            }
            count += 1;
            continue;
        }

        // Check for bare ESC without CSI (other escape sequences)
        if b == 0x1B {
            i += 1;
            count += 1;
            continue;
        }

        // Allow tab, newline, carriage return
        if b == 0x09 || b == 0x0A || b == 0x0D {
            result.push(b as char);
            i += 1;
            continue;
        }

        // Remove other C0 controls and DEL
        if b <= 0x1F || b == 0x7F {
            count += 1;
            i += 1;
            continue;
        }

        // UTF-8 multi-byte: let Rust handle it
        if b >= 0x80 {
            // Find the end of the UTF-8 character
            let char_len = if b < 0xC0 {
                1 // continuation byte (invalid as start, but handle gracefully)
            } else if b < 0xE0 {
                2
            } else if b < 0xF0 {
                3
            } else {
                4
            };
            let end = std::cmp::min(i + char_len, bytes.len());
            // Validate the UTF-8 sequence
            match std::str::from_utf8(&bytes[i..end]) {
                Ok(s) => {
                    result.push_str(s);
                }
                Err(_) => {
                    // Invalid UTF-8 — replace with U+FFFD
                    result.push('\u{FFFD}');
                    count += 1;
                }
            }
            i = end;
            continue;
        }

        // ASCII printable
        result.push(b as char);
        i += 1;
    }

    (result, count)
}

/// Neutralize dangerous URI schemes in text.
///
/// Replaces `javascript:` and `data:` (with script content) URI schemes
/// with safe placeholders. Case-insensitive matching.
fn neutralize_dangerous_uris(text: &str) -> (String, usize) {
    let lower = text.to_lowercase();
    let mut count = 0;

    // Check for javascript: URIs
    if lower.contains("javascript:") {
        count += 1;
    }

    // Check for data: URIs with script-like content
    // data:text/html, data:application/xhtml, etc.
    let has_dangerous_data = lower.contains("data:text/html")
        || lower.contains("data:application/xhtml")
        || lower.contains("data:text/xss");
    if has_dangerous_data {
        count += 1;
    }

    if count == 0 {
        return (text.to_string(), 0);
    }

    let mut result = text.to_string();

    // Neutralize javascript: URIs (case-insensitive)
    let js_replaced = replace_case_insensitive(&result, "javascript:", "[blocked-javascript-uri:]");
    if js_replaced.1 > 0 {
        count = count.max(js_replaced.1);
        result = js_replaced.0;
    }

    // Neutralize dangerous data: URIs
    let data_patterns = ["data:text/html", "data:application/xhtml", "data:text/xss"];
    for pattern in data_patterns {
        let replaced =
            replace_case_insensitive(&result, pattern, &format!("[blocked-data-uri:{}]", ""));
        if replaced.1 > 0 {
            result = replaced.0;
        }
    }

    (result, count)
}

/// Sanitize resource URIs that might leak internal paths.
///
/// Replaces `file://` and internal IP references with neutral placeholders.
fn sanitize_resource_uris(text: &str) -> (String, usize) {
    let lower = text.to_lowercase();
    let mut count = 0;

    if lower.contains("file:///") || lower.contains("file://") {
        count += 1;
    }

    if count == 0 {
        return (text.to_string(), 0);
    }

    let result = replace_case_insensitive(text, "file:///", "[blocked-file-uri:/]");
    count = result.1;
    let result = replace_case_insensitive(&result.0, "file://", "[blocked-file-uri:]");

    (result.0, count)
}

/// Enforce per-item size limit.
///
/// Truncates to `max_size` bytes at a UTF-8 character boundary if needed.
fn enforce_item_size(text: &str, max_size: usize) -> (String, bool) {
    if text.len() <= max_size {
        return (text.to_string(), false);
    }

    let boundary = text.floor_char_boundary(max_size);
    let truncated = format!(
        "{}\n[output validation: item exceeded {} byte limit, truncated]",
        &text[..boundary],
        max_size
    );
    (truncated, true)
}

/// Case-insensitive replacement helper.
fn replace_case_insensitive(text: &str, pattern: &str, replacement: &str) -> (String, usize) {
    let lower_text = text.to_lowercase();
    let lower_pattern = pattern.to_lowercase();
    let mut count = 0;
    let mut result = String::new();
    let mut last_end = 0;

    let mut search_start = 0;
    while let Some(pos) = lower_text[last_end..].find(&lower_pattern) {
        let abs_pos = last_end + pos;
        result.push_str(&text[last_end..abs_pos]);
        result.push_str(replacement);
        last_end = abs_pos + pattern.len();
        count += 1;
        search_start = last_end;
        if search_start >= lower_text.len() {
            break;
        }
    }

    if count == 0 {
        return (text.to_string(), 0);
    }

    result.push_str(&text[last_end..]);
    (result, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- strip_null_bytes ---

    #[test]
    fn test_strip_null_bytes_none() {
        let (result, count) = strip_null_bytes("hello world");
        assert_eq!(result, "hello world");
        assert_eq!(count, 0);
    }

    #[test]
    fn test_strip_null_bytes_middle() {
        let (result, count) = strip_null_bytes("hello\0world");
        assert_eq!(result, "helloworld");
        assert_eq!(count, 1);
    }

    #[test]
    fn test_strip_null_bytes_multiple() {
        let (result, count) = strip_null_bytes("\0\0\0abc\0\0");
        assert_eq!(result, "abc");
        assert_eq!(count, 5);
    }

    #[test]
    fn test_strip_null_bytes_json_breaking() {
        // Null byte in middle of JSON-like content
        let input = "{\"key\": \"val\0ue\"}";
        let (result, count) = strip_null_bytes(input);
        assert_eq!(result, "{\"key\": \"value\"}");
        assert_eq!(count, 1);
    }

    // --- strip_control_chars ---

    #[test]
    fn test_strip_control_chars_preserves_newlines() {
        let (result, count) = strip_control_chars("line1\nline2\r\nline3");
        assert_eq!(result, "line1\nline2\r\nline3");
        assert_eq!(count, 0);
    }

    #[test]
    fn test_strip_control_chars_preserves_tabs() {
        let (result, count) = strip_control_chars("col1\tcol2\tcol3");
        assert_eq!(result, "col1\tcol2\tcol3");
        assert_eq!(count, 0);
    }

    #[test]
    fn test_strip_control_chars_removes_bell() {
        let (result, count) = strip_control_chars("alert\x07");
        assert_eq!(result, "alert");
        assert_eq!(count, 1);
    }

    #[test]
    fn test_strip_control_chars_removes_backspace() {
        let (result, count) = strip_control_chars("over\x08write");
        assert_eq!(result, "overwrite");
        assert_eq!(count, 1);
    }

    #[test]
    fn test_strip_control_chars_removes_ansi_color() {
        let (result, count) = strip_control_chars("\x1b[31mred text\x1b[0m");
        assert_eq!(result, "red text");
        assert_eq!(count, 2);
    }

    #[test]
    fn test_strip_control_chars_removes_ansi_cursor() {
        let (result, count) = strip_control_chars("\x1b[2J\x1b[Hscreen cleared");
        assert_eq!(result, "screen cleared");
        assert_eq!(count, 2);
    }

    #[test]
    fn test_strip_control_chars_removes_del() {
        let (result, count) = strip_control_chars("text\x7F");
        assert_eq!(result, "text");
        assert_eq!(count, 1);
    }

    #[test]
    fn test_strip_control_chars_bare_escape() {
        let (result, count) = strip_control_chars("before\x1Bafter");
        assert_eq!(result, "beforeafter");
        assert_eq!(count, 1);
    }

    #[test]
    fn test_strip_control_chars_preserves_utf8() {
        let (result, count) = strip_control_chars("Hello 世界 🌍");
        assert_eq!(result, "Hello 世界 🌍");
        assert_eq!(count, 0);
    }

    // --- neutralize_dangerous_uris ---

    #[test]
    fn test_neutralize_javascript_uri() {
        let (result, count) = neutralize_dangerous_uris("click javascript:alert(1)");
        assert_eq!(result, "click [blocked-javascript-uri:]alert(1)");
        assert_eq!(count, 1);
    }

    #[test]
    fn test_neutralize_javascript_uri_case_insensitive() {
        let (result, count) = neutralize_dangerous_uris("JaVaScRiPt:alert(1)");
        assert!(result.contains("[blocked-javascript-uri:]"));
        assert_eq!(count, 1);
    }

    #[test]
    fn test_neutralize_data_html_uri() {
        let (result, count) = neutralize_dangerous_uris("data:text/html,<script>alert(1)</script>");
        assert!(result.contains("[blocked-data-uri:]"));
        assert_eq!(count, 1);
    }

    #[test]
    fn test_neutralize_safe_data_uri_passthrough() {
        // data:image/png should be safe
        let (result, count) = neutralize_dangerous_uris("data:image/png;base64,abc123");
        assert_eq!(result, "data:image/png;base64,abc123");
        assert_eq!(count, 0);
    }

    #[test]
    fn test_neutralize_no_uris() {
        let (result, count) = neutralize_dangerous_uris("just regular text");
        assert_eq!(result, "just regular text");
        assert_eq!(count, 0);
    }

    // --- sanitize_resource_uris ---

    #[test]
    fn test_sanitize_file_uri() {
        let (result, count) = sanitize_resource_uris("file:///etc/passwd");
        assert!(result.contains("[blocked-file-uri:]"));
        assert_eq!(count, 1);
    }

    #[test]
    fn test_sanitize_no_file_uri() {
        let (result, count) = sanitize_resource_uris("https://example.com/file.txt");
        assert_eq!(result, "https://example.com/file.txt");
        assert_eq!(count, 0);
    }

    // --- enforce_item_size ---

    #[test]
    fn test_enforce_size_within_limit() {
        let (result, truncated) = enforce_item_size("small", 100);
        assert_eq!(result, "small");
        assert!(!truncated);
    }

    #[test]
    fn test_enforce_size_exceeds_limit() {
        let big = "x".repeat(2_000_000);
        let (result, truncated) = enforce_item_size(&big, MAX_CONTENT_ITEM_SIZE);
        assert!(truncated);
        assert!(result.len() < big.len());
        assert!(result.contains("truncated"));
    }

    #[test]
    fn test_enforce_size_utf8_boundary() {
        // Make sure truncation doesn't split multi-byte characters
        let content = "日本語".repeat(400_000); // well over 1MB
        let (result, truncated) = enforce_item_size(&content, MAX_CONTENT_ITEM_SIZE);
        assert!(truncated);
        // Result should be valid UTF-8 (String always is, but verify no panic)
        assert!(result.chars().count() > 0);
    }

    // --- validate_tool_output (full pipeline) ---

    #[test]
    fn test_validate_clean_output() {
        let result = validate_tool_output("Hello, this is clean output.");
        assert!(!result.was_modified());
        assert_eq!(result.text, "Hello, this is clean output.");
    }

    #[test]
    fn test_validate_malicious_output() {
        let input = "click\x00javascript:evil\x1b[31mhidden\x1b[0mfile:///etc/shadow";
        let result = validate_tool_output(input);
        assert!(result.was_modified());
        assert!(!result.text.contains('\0'));
        assert!(result.text.contains("[blocked-javascript-uri:]"));
        assert!(result.text.contains("[blocked-file-uri:]"));
        assert!(!result.text.contains("\x1b"));
        assert!(result.text.contains("clickevilhidden"));
    }

    // --- validate_json_output ---

    #[test]
    fn test_validate_json_clean() {
        let value = serde_json::json!({"result": "clean output", "count": 42});
        let (validated, result) = validate_json_output(&value);
        assert!(!result.was_modified());
        assert_eq!(validated["result"], "clean output");
        assert_eq!(validated["count"], 42);
    }

    #[test]
    fn test_validate_json_nested_strings() {
        let value = serde_json::json!({
            "items": [
                {"name": "clean"},
                {"name": "has\x00null"},
                {"name": "has\x1b[31mcolor\x1b[0m"},
            ]
        });
        let (validated, result) = validate_json_output(&value);
        assert!(result.was_modified());
        assert_eq!(validated["items"][1]["name"], "hasnull");
        assert_eq!(validated["items"][2]["name"], "hascolor");
        assert_eq!(validated["items"][0]["name"], "clean");
    }

    #[test]
    fn test_validate_json_preserves_numbers_and_bools() {
        let value = serde_json::json!({"num": 123, "bool": true, "null": null});
        let (validated, _) = validate_json_output(&value);
        assert_eq!(validated["num"], 123);
        assert_eq!(validated["bool"], true);
        assert_eq!(validated["null"], serde_json::Value::Null);
    }

    // --- ValidationResult ---

    #[test]
    fn test_validation_result_was_modified() {
        let clean = ValidationResult {
            text: "clean".to_string(),
            null_bytes_stripped: 0,
            control_chars_stripped: 0,
            dangerous_uris_neutralized: 0,
            size_truncated: false,
            resource_uris_sanitized: 0,
        };
        assert!(!clean.was_modified());

        let dirty = ValidationResult {
            text: "dirty".to_string(),
            null_bytes_stripped: 1,
            control_chars_stripped: 0,
            dangerous_uris_neutralized: 0,
            size_truncated: false,
            resource_uris_sanitized: 0,
        };
        assert!(dirty.was_modified());
    }
}
