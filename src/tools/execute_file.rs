//! Execute file: server-side file read that returns only structured output.
//!
//! Instead of dumping file contents into the conversation, this tool:
//! 1. Reads the file server-side
//! 2. Extracts metadata (size, lines, language hint)
//! 3. Returns a bounded summary + offset/limit for paged reads
//! 4. Indexes the content into session FTS5 for later search
//!
//! Use this instead of Read for analysis/exploration.

use anyhow::{Context as _, Result};
use rmcp::schemars;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;

use crate::registry::ToolRegistry;
use crate::result_store::ResultStore;
use crate::session::SessionEventStore;
use crate::session::overflow;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExecuteFileParams {
    /// Path to the file. Must be within the working directory or an allowed path.
    pub path: String,
    /// Optional offset in lines (default 0).
    #[serde(default)]
    pub offset: Option<usize>,
    /// Optional line limit (default 50, max 500).
    #[serde(default)]
    pub limit: Option<usize>,
    /// Optional source label for indexing (e.g. "config", "source", "docs").
    #[serde(default)]
    pub source: Option<String>,
}

/// Allowed path prefixes for execute_file (prevents path traversal).
const ALLOWED_PREFIXES: &[&str] = &[".", "/home", "/usr", "/opt", "/tmp", "/var", "/root"];

/// Validate that a path is safe to read.
fn validate_path(path: &str) -> Result<PathBuf> {
    let path = PathBuf::from(path);

    // Reject absolute paths outside allowed prefixes
    if path.is_absolute() {
        let prefix_ok = ALLOWED_PREFIXES.iter().any(|p| path.starts_with(p));
        if !prefix_ok {
            anyhow::bail!(
                "Path '{}' is outside allowed directories. Only reading within project or standard system paths is permitted.",
                path.display()
            );
        }
    }

    // Reject path traversal (e.g., ../etc/passwd)
    if path.components().any(|c| c.as_os_str() == "..") {
        anyhow::bail!("Path '{}' contains path traversal (..)", path.display());
    }

    Ok(path)
}

/// Handle execute_file: read file server-side, return summary + lines, index content.
pub async fn handle_execute_file(
    params: &ExecuteFileParams,
    _registry: &Arc<ToolRegistry>,
    _result_store: &Arc<ResultStore>,
    session_store: Option<&SessionEventStore>,
    session_key: Option<&str>,
) -> Result<String> {
    let path = validate_path(&params.path)?;

    // Read file
    let content = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("Failed to read '{}'", path.display()))?;

    let total_lines = content.lines().count();
    let bytes = content.len();

    // Extract requested lines
    let offset = params.offset.unwrap_or(0);
    let limit = params.limit.unwrap_or(50).min(500);

    let lines: Vec<String> = content
        .lines()
        .skip(offset)
        .take(limit)
        .map(|l| l.to_string())
        .collect();

    // Index full content into session FTS5
    if let (Some(store), Some(key)) = (session_store, session_key) {
        let _ = overflow::index_overflow(
            store,
            key,
            &format!("file:{}", params.path),
            &content,
            params.source.as_deref().unwrap_or("file"),
            None,
        )
        .await;
    }

    // Detect language hint from extension
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let language = match ext {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "py" => "python",
        "go" => "go",
        "yaml" | "yml" => "yaml",
        "json" => "json",
        "md" => "markdown",
        "toml" => "toml",
        _ => "text",
    };

    let result = serde_json::json!({
        "path": params.path,
        "language": language,
        "total_lines": total_lines,
        "total_bytes": bytes,
        "offset": offset,
        "limit": limit,
        "lines": lines,
        "lookup": "execute_file(path, offset, limit) for more lines",
        "search": "session_search(query, source=\"file\") for relevant sections",
        "note": "File content is indexed for search. Use session_search(query) to find relevant sections without reading the whole file."
    });

    Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_path_blocks_traversal() {
        assert!(validate_path("../../etc/passwd").is_err());
        assert!(validate_path("/etc/shadow").is_err());
    }

    #[test]
    fn validate_path_allows_relative() {
        assert!(validate_path("./src/main.rs").is_ok());
        assert!(validate_path("src/main.rs").is_ok());
        assert!(validate_path("Cargo.toml").is_ok());
    }

    #[test]
    fn validate_path_allows_absolute() {
        assert!(validate_path("/home/user/project/src/main.rs").is_ok());
        assert!(validate_path("/usr/local/bin/cargo").is_ok());
        assert!(validate_path("/tmp/file.txt").is_ok());
    }
}
