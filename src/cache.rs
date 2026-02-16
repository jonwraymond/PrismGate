use crate::registry::{ToolEntry, ToolRegistry};
use crate::tracker::CallTracker;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Current cache version. Bump when adding new persisted fields.
const CACHE_VERSION: u32 = 4;

#[derive(Serialize, Deserialize)]
struct ToolCache {
    version: u32,
    backends: HashMap<String, Vec<ToolEntry>>,
    /// Embedding vectors keyed by tool name. Only present in version 2+ caches.
    #[serde(default)]
    embeddings: Option<HashMap<String, Vec<f32>>>,
    /// Per-tool usage counts. Only present in version 4+ caches.
    #[serde(default)]
    usage_stats: Option<HashMap<String, u64>>,
}

/// Default cache path: platform cache directory
pub fn default_cache_path() -> PathBuf {
    crate::cli::prismgate_cache_home().join("cache.json")
}

/// Derive cache path from config path (legacy, kept for backward compatibility).
/// e.g. config/gatemini.yaml -> config/.gatemini.cache.json
#[cfg(test)]
pub fn cache_path_from_config(config_path: &Path) -> PathBuf {
    let dir = config_path.parent().unwrap_or(Path::new("."));
    let stem = config_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("gatemini");
    dir.join(format!(".{stem}.cache.json"))
}

/// Load cached tools into the registry and restore usage stats to tracker.
/// Only loads backends that exist in the current config.
pub async fn load(
    path: &Path,
    registry: &ToolRegistry,
    config_backend_names: &[String],
    tracker: Option<&CallTracker>,
) -> usize {
    let data = match tokio::fs::read_to_string(path).await {
        Ok(d) => d,
        Err(_) => return 0, // no cache file yet
    };

    let mut cache: ToolCache = match serde_json::from_str::<ToolCache>(&data) {
        Ok(c) if c.version >= 1 && c.version <= CACHE_VERSION => c,
        Ok(c) => {
            warn!(
                version = c.version,
                "incompatible tool cache version, skipping"
            );
            return 0;
        }
        Err(e) => {
            warn!(error = %e, "invalid tool cache, skipping");
            return 0;
        }
    };

    // Migration: v1/v2 caches don't have original_name — populate from name
    if cache.version < 3 {
        for tools in cache.backends.values_mut() {
            for tool in tools.iter_mut() {
                if tool.original_name.is_empty() {
                    tool.original_name = tool.name.clone();
                }
            }
        }
    }

    let mut total = 0;
    for (backend_name, tools) in &cache.backends {
        if config_backend_names.contains(backend_name) {
            total += tools.len();
            registry.register_backend_tools(backend_name, tools.clone());
        }
    }

    // Restore cached embeddings (semantic feature only)
    #[cfg(feature = "semantic")]
    if let Some(embeddings) = cache.embeddings
        && !embeddings.is_empty()
    {
        info!(count = embeddings.len(), "restoring cached embeddings");
        registry.load_embeddings(embeddings);
    }

    // Restore usage stats (version 4+ caches)
    if let Some(tracker) = tracker
        && let Some(usage) = cache.usage_stats
        && !usage.is_empty()
    {
        info!(count = usage.len(), "restoring cached usage stats");
        tracker.load_usage(usage);
    }

    info!(tools = total, path = %path.display(), "loaded tools from cache");
    total
}

/// Save the current registry to the cache file (atomic write via temp + rename).
pub async fn save(path: &Path, registry: &ToolRegistry, tracker: Option<&CallTracker>) {
    let snapshot = registry.snapshot();

    #[cfg(feature = "semantic")]
    let embeddings = registry.embedding_snapshot();
    #[cfg(not(feature = "semantic"))]
    let embeddings: Option<HashMap<String, Vec<f32>>> = None;

    let usage_stats = tracker.map(|t| t.snapshot_usage());

    let cache = ToolCache {
        version: CACHE_VERSION,
        backends: snapshot,
        embeddings,
        usage_stats,
    };

    let json = match serde_json::to_string_pretty(&cache) {
        Ok(j) => j,
        Err(e) => {
            warn!(error = %e, "failed to serialize tool cache");
            return;
        }
    };

    // Atomic write: write to temp file, then rename
    let tmp = path.with_extension("cache.tmp");
    if let Err(e) = tokio::fs::write(&tmp, &json).await {
        warn!(error = %e, "failed to write tool cache temp file");
        return;
    }
    if let Err(e) = tokio::fs::rename(&tmp, path).await {
        warn!(error = %e, "failed to rename tool cache file");
        return;
    }

    debug!(path = %path.display(), tools = cache.backends.values().map(|v| v.len()).sum::<usize>(), "tool cache saved");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_entry(name: &str, backend: &str) -> ToolEntry {
        ToolEntry {
            name: name.to_string(),
            original_name: name.to_string(),
            description: format!("{name} description"),
            backend_name: backend.to_string(),
            input_schema: json!({"type": "object"}),
            tags: Vec::new(),
        }
    }

    #[test]
    fn test_cache_path_from_config() {
        let path = cache_path_from_config(Path::new("config/gatemini.yaml"));
        assert_eq!(path, PathBuf::from("config/.gatemini.cache.json"));

        let path = cache_path_from_config(Path::new("/etc/myapp.yml"));
        assert_eq!(path, PathBuf::from("/etc/.myapp.cache.json"));
    }

    #[tokio::test]
    async fn test_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join(".test.cache.json");

        let registry = ToolRegistry::new();
        registry.register_backend_tools(
            "exa",
            vec![
                make_entry("web_search", "exa"),
                make_entry("find_similar", "exa"),
            ],
        );
        registry.register_backend_tools("tavily", vec![make_entry("tavily_search", "tavily")]);

        // Save
        save(&cache_path, &registry, None).await;
        assert!(cache_path.exists());

        // Load into a fresh registry
        let registry2 = ToolRegistry::new();
        let config_names = vec!["exa".to_string(), "tavily".to_string()];
        let loaded = load(&cache_path, &registry2, &config_names, None).await;
        // Snapshot saves all entries (bare + namespaced), loaded count matches what was saved
        assert!(loaded > 0);
        // Both bare and namespaced should resolve
        assert!(registry2.get_by_name("web_search").is_some());
        assert!(registry2.get_by_name("tavily_search").is_some());
    }

    #[tokio::test]
    async fn test_load_filters_by_config() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join(".test.cache.json");

        let registry = ToolRegistry::new();
        registry.register_backend_tools("exa", vec![make_entry("web_search", "exa")]);
        registry.register_backend_tools(
            "removed_backend",
            vec![make_entry("old_tool", "removed_backend")],
        );

        save(&cache_path, &registry, None).await;

        // Only load "exa" (removed_backend no longer in config)
        let registry2 = ToolRegistry::new();
        let config_names = vec!["exa".to_string()];
        let loaded = load(&cache_path, &registry2, &config_names, None).await;
        assert!(loaded > 0);
        assert!(registry2.get_by_name("web_search").is_some());
        assert!(registry2.get_by_name("old_tool").is_none());
    }

    #[tokio::test]
    async fn test_load_missing_file() {
        let registry = ToolRegistry::new();
        let loaded = load(Path::new("/nonexistent/cache.json"), &registry, &[], None).await;
        assert_eq!(loaded, 0);
    }

    #[tokio::test]
    async fn test_load_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join(".test.cache.json");
        tokio::fs::write(&cache_path, "not json").await.unwrap();

        let registry = ToolRegistry::new();
        let loaded = load(&cache_path, &registry, &[], None).await;
        assert_eq!(loaded, 0);
    }

    #[tokio::test]
    async fn test_load_wrong_version() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join(".test.cache.json");
        let bad = serde_json::json!({"version": 99, "backends": {}});
        tokio::fs::write(&cache_path, bad.to_string())
            .await
            .unwrap();

        let registry = ToolRegistry::new();
        let loaded = load(&cache_path, &registry, &[], None).await;
        assert_eq!(loaded, 0);
    }

    #[tokio::test]
    async fn test_cache_v2_migration() {
        // Simulate a v2 cache without original_name fields
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join(".test.cache.json");
        let v2_cache = json!({
            "version": 2,
            "backends": {
                "exa": [
                    {"name": "web_search", "description": "Search", "backend_name": "exa", "input_schema": {"type": "object"}}
                ]
            }
        });
        tokio::fs::write(&cache_path, v2_cache.to_string())
            .await
            .unwrap();

        let registry = ToolRegistry::new();
        let loaded = load(&cache_path, &registry, &["exa".to_string()], None).await;
        assert_eq!(loaded, 1);

        // original_name should be populated from name during migration
        let entry = registry.get_by_name("web_search").unwrap();
        assert_eq!(entry.original_name, "web_search");
    }

    #[tokio::test]
    async fn test_cache_v4_usage_stats_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join(".test.cache.json");

        let registry = ToolRegistry::new();
        registry.register_backend_tools("exa", vec![make_entry("web_search", "exa")]);

        // Create tracker with some usage
        let tracker = CallTracker::new();
        tracker.record(
            "exa.web_search",
            "exa",
            std::time::Duration::from_millis(10),
            true,
        );
        tracker.record(
            "exa.web_search",
            "exa",
            std::time::Duration::from_millis(20),
            true,
        );

        // Save with tracker
        save(&cache_path, &registry, Some(&tracker)).await;

        // Load into fresh registry + tracker
        let registry2 = ToolRegistry::new();
        let tracker2 = CallTracker::new();
        let loaded = load(
            &cache_path,
            &registry2,
            &["exa".to_string()],
            Some(&tracker2),
        )
        .await;
        assert!(loaded > 0);

        // Usage should be restored
        assert_eq!(tracker2.usage_count("exa.web_search"), 2);
    }

    #[tokio::test]
    async fn test_cache_v3_migration_to_v4() {
        // v3 cache without usage_stats should load fine, defaulting to no usage
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join(".test.cache.json");
        let v3_cache = json!({
            "version": 3,
            "backends": {
                "exa": [
                    {
                        "name": "exa.web_search",
                        "original_name": "web_search",
                        "description": "Search",
                        "backend_name": "exa",
                        "input_schema": {"type": "object"}
                    }
                ]
            }
        });
        tokio::fs::write(&cache_path, v3_cache.to_string())
            .await
            .unwrap();

        let registry = ToolRegistry::new();
        let tracker = CallTracker::new();
        let loaded = load(&cache_path, &registry, &["exa".to_string()], Some(&tracker)).await;
        assert_eq!(loaded, 1);

        // No usage stats in v3 cache, so tracker should be empty
        assert_eq!(tracker.usage_count("exa.web_search"), 0);
    }
}
