//! JSON key-path chunking for large tool outputs.
//!
//! Recursively walks a JSON value, producing chunks with hierarchical path
//! titles (e.g., "results > items > [0-4]"). Arrays are batched by accumulated
//! byte size. Identity fields (id, name, title, slug, key, label) are extracted
//! from objects to create meaningful chunk labels.

use serde::Serialize;
use serde_json::Value;

/// Target byte size for each chunk. Chunks may exceed this for atomic values.
const CHUNK_TARGET: usize = 4096;

/// Fields checked (in order) to extract a human-readable identity from objects.
const IDENTITY_FIELDS: &[&str] = &["id", "name", "title", "slug", "key", "label"];

/// A chunk of a JSON document with its path in the tree.
#[derive(Debug, Clone, Serialize)]
pub struct JsonChunk {
    /// Hierarchical path: "results > items > [0-4]"
    pub path: String,
    /// Identity extracted from object fields: "id=123, name=foo"
    #[serde(skip_serializing_if = "String::is_empty")]
    pub identity: String,
    /// Serialized JSON fragment.
    pub content: String,
    /// Byte size of the content.
    pub byte_size: usize,
}

/// Chunk a JSON value into path-labeled fragments for summarization.
pub fn chunk_json(value: &Value, path_prefix: &str) -> Vec<JsonChunk> {
    let mut chunks = Vec::new();
    walk(value, path_prefix, &mut chunks);
    chunks
}

/// Produce a compact summary of chunks: paths, identities, and sizes.
pub fn chunk_summary(chunks: &[JsonChunk]) -> String {
    let total_bytes: usize = chunks.iter().map(|c| c.byte_size).sum();
    let mut summary = format!(
        "[JSON chunked: {} chunks, {:.1}KB total]\n\n",
        chunks.len(),
        total_bytes as f64 / 1024.0
    );
    for chunk in chunks {
        let identity = if chunk.identity.is_empty() {
            String::new()
        } else {
            format!(" ({})", chunk.identity)
        };
        summary.push_str(&format!(
            "  {} — {:.1}KB{}\n",
            chunk.path,
            chunk.byte_size as f64 / 1024.0,
            identity,
        ));
    }
    summary
}

fn walk(value: &Value, path: &str, chunks: &mut Vec<JsonChunk>) {
    match value {
        Value::Object(map) => {
            // Check if the whole object is small enough to be one chunk
            let serialized = serde_json::to_string(value).unwrap_or_default();
            if serialized.len() <= CHUNK_TARGET && !has_nested_containers(value) {
                chunks.push(JsonChunk {
                    path: path.to_string(),
                    identity: extract_identity(value),
                    content: serialized.clone(),
                    byte_size: serialized.len(),
                });
                return;
            }

            // Recurse into each key
            for (key, val) in map {
                let child_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{} > {}", path, key)
                };
                walk(val, &child_path, chunks);
            }
        }
        Value::Array(arr) => {
            if arr.is_empty() {
                let content = "[]".to_string();
                chunks.push(JsonChunk {
                    path: path.to_string(),
                    identity: String::new(),
                    content,
                    byte_size: 2,
                });
                return;
            }

            // Batch consecutive items by accumulated size
            let mut batch_start = 0;
            let mut batch_bytes = 0;
            let mut batch_items: Vec<String> = Vec::new();

            for (i, item) in arr.iter().enumerate() {
                let item_str = serde_json::to_string(item).unwrap_or_default();
                let item_len = item_str.len();

                if !batch_items.is_empty() && batch_bytes + item_len > CHUNK_TARGET {
                    // Flush current batch
                    flush_batch(
                        path,
                        batch_start,
                        i - 1,
                        &batch_items,
                        batch_bytes,
                        arr,
                        chunks,
                    );
                    batch_start = i;
                    batch_bytes = 0;
                    batch_items.clear();
                }

                batch_bytes += item_len;
                batch_items.push(item_str);
            }

            // Flush remaining batch
            if !batch_items.is_empty() {
                flush_batch(
                    path,
                    batch_start,
                    arr.len() - 1,
                    &batch_items,
                    batch_bytes,
                    arr,
                    chunks,
                );
            }
        }
        // Scalar values — emit as single chunk
        _ => {
            let content = serde_json::to_string(value).unwrap_or_default();
            let byte_size = content.len();
            chunks.push(JsonChunk {
                path: path.to_string(),
                identity: String::new(),
                content,
                byte_size,
            });
        }
    }
}

fn flush_batch(
    path: &str,
    start: usize,
    end: usize,
    items: &[String],
    byte_size: usize,
    arr: &[Value],
    chunks: &mut Vec<JsonChunk>,
) {
    let range_label = if start == end {
        format!("[{}]", start)
    } else {
        format!("[{}-{}]", start, end)
    };

    let chunk_path = if path.is_empty() {
        range_label
    } else {
        format!("{} > {}", path, range_label)
    };

    // Extract identity from first item if it's an object
    let identity = arr.get(start).map(extract_identity).unwrap_or_default();

    let content = format!("[{}]", items.join(","));
    chunks.push(JsonChunk {
        path: chunk_path,
        identity,
        content,
        byte_size,
    });
}

/// Extract identity fields from a JSON object for human-readable labels.
fn extract_identity(value: &Value) -> String {
    let obj = match value.as_object() {
        Some(o) => o,
        None => return String::new(),
    };

    let mut parts = Vec::new();
    for field in IDENTITY_FIELDS {
        if let Some(val) = obj.get(*field) {
            let s = match val {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                _ => continue,
            };
            parts.push(format!("{}={}", field, s));
            if parts.len() >= 2 {
                break; // At most 2 identity fields
            }
        }
    }
    parts.join(", ")
}

/// Check if a value contains nested objects or arrays.
fn has_nested_containers(value: &Value) -> bool {
    match value {
        Value::Object(map) => map
            .values()
            .any(|v| matches!(v, Value::Object(_) | Value::Array(_))),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_chunk_flat_object() {
        let value = json!({"id": 1, "name": "test", "status": "ok"});
        let chunks = chunk_json(&value, "root");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].path, "root");
        assert!(chunks[0].identity.contains("id=1"));
        assert!(chunks[0].identity.contains("name=test"));
    }

    #[test]
    fn test_chunk_nested_array() {
        // Create an array of 100 items, each ~50 bytes → ~5KB total
        let items: Vec<Value> = (0..100)
            .map(|i| json!({"id": i, "name": format!("item-{}", i), "data": "x".repeat(20)}))
            .collect();
        let value = json!({"results": items});
        let chunks = chunk_json(&value, "");
        // Should be multiple chunks (100 * ~50 bytes / 4096 ≈ ~2 chunks)
        assert!(
            chunks.len() > 1,
            "expected multiple chunks, got {}",
            chunks.len()
        );
        // First chunk path should include array range
        assert!(chunks[0].path.contains("results >"));
    }

    #[test]
    fn test_chunk_identity_extraction() {
        let value = json!({"id": 42, "name": "example", "title": "My Title"});
        let identity = extract_identity(&value);
        assert!(identity.contains("id=42"));
        assert!(identity.contains("name=example"));
        // Should stop at 2 fields
        assert!(!identity.contains("title"));
    }

    #[test]
    fn test_chunk_path_format() {
        let value = json!({"data": {"users": [{"id": 1}, {"id": 2}]}});
        let chunks = chunk_json(&value, "");
        // Should have paths like "data > users > [0-1]"
        let paths: Vec<&str> = chunks.iter().map(|c| c.path.as_str()).collect();
        assert!(
            paths.iter().any(|p| p.contains(" > ")),
            "paths should use ' > ' separator: {:?}",
            paths
        );
    }

    #[test]
    fn test_chunk_empty_array() {
        let value = json!({"items": []});
        let chunks = chunk_json(&value, "root");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, "[]");
    }

    #[test]
    fn test_chunk_summary_format() {
        let value = json!({"a": "hello", "b": [1, 2, 3]});
        let chunks = chunk_json(&value, "root");
        let summary = chunk_summary(&chunks);
        assert!(summary.contains("JSON chunked:"));
        assert!(summary.contains("chunks"));
        assert!(summary.contains("KB"));
    }

    #[test]
    fn test_chunk_scalar() {
        let value = json!("just a string");
        let chunks = chunk_json(&value, "val");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].path, "val");
    }
}
