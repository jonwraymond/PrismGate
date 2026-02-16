use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

#[cfg(feature = "semantic")]
use crate::embeddings::EmbeddingIndex;

/// A tool entry in the registry, linking a tool to its backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolEntry {
    /// The tool name as registered in the registry (may be namespaced: "github.get_repo").
    pub name: String,
    /// The original tool name from the backend MCP server ("get_repo").
    /// Used when dispatching calls to the backend (which doesn't know about namespacing).
    #[serde(default)]
    pub original_name: String,
    /// Description from the backend's tool definition.
    pub description: String,
    /// The backend that owns this tool.
    pub backend_name: String,
    /// The full JSON schema for the tool's input.
    pub input_schema: Value,
}

/// Concurrent tool registry aggregating tools from all backends.
///
/// Uses DashMap for lock-free concurrent reads. Backends register
/// tools concurrently at startup without contention.
///
/// ## Namespacing
///
/// Tools are always registered under their namespaced key (`backend.tool_name`).
/// If only one backend owns a given bare tool name, a bare alias is also registered
/// for backward compatibility. When a second backend registers the same bare name,
/// the alias is removed and a warning is logged — callers must then use the
/// `backend.tool_name` notation.
pub struct ToolRegistry {
    /// tool_name -> ToolEntry (contains both namespaced keys and bare-name aliases)
    tools: DashMap<String, ToolEntry>,
    /// backend_name -> list of tool keys registered in `tools`
    backend_tools: DashMap<String, Vec<String>>,
    /// bare_name -> list of (backend_name, namespace) pairs that own this tool name.
    /// Used for collision detection: len()==1 → bare alias OK, len()>1 → collision.
    bare_name_owners: DashMap<String, Vec<(String, String)>>,
    /// Optional semantic embedding index for hybrid search.
    #[cfg(feature = "semantic")]
    embedding_index: Option<EmbeddingIndex>,
}

impl ToolRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            tools: DashMap::new(),
            backend_tools: DashMap::new(),
            bare_name_owners: DashMap::new(),
            #[cfg(feature = "semantic")]
            embedding_index: None,
        })
    }

    /// Create a registry with an embedding index for hybrid BM25+semantic search.
    #[cfg(feature = "semantic")]
    pub fn new_with_embeddings(index: EmbeddingIndex) -> Arc<Self> {
        Arc::new(Self {
            tools: DashMap::new(),
            backend_tools: DashMap::new(),
            bare_name_owners: DashMap::new(),
            embedding_index: Some(index),
        })
    }

    /// Register tools discovered from a backend with automatic namespacing.
    ///
    /// Each tool is registered under its namespaced key (`namespace.original_name`).
    /// If no other backend owns the same bare name, a bare-name alias is also registered
    /// for backward compatibility. On collision (2+ backends with same bare name),
    /// the bare alias is removed and only namespaced keys remain.
    pub fn register_backend_tools(&self, backend_name: &str, tools: Vec<ToolEntry>) {
        self.register_backend_tools_namespaced(backend_name, backend_name, tools);
    }

    /// Register tools with an explicit namespace prefix.
    pub fn register_backend_tools_namespaced(
        &self,
        backend_name: &str,
        namespace: &str,
        tools: Vec<ToolEntry>,
    ) {
        // Clean up any existing entries for this backend (handles cache→live re-registration).
        // Remove from bare_name_owners first to prevent false collision detection.
        if self.backend_tools.contains_key(backend_name) {
            self.bare_name_owners.retain(|_bare_name, owners| {
                owners.retain(|(b, _)| b != backend_name);
                !owners.is_empty()
            });
            // Remove old tool entries from the tools map
            if let Some(old_keys) = self.backend_tools.get(backend_name) {
                for key in old_keys.value() {
                    self.tools.remove(key);
                }
            }
        }

        let mut registered_keys = Vec::new();

        // Collect entries for embedding before moving tools
        #[cfg(feature = "semantic")]
        let mut entries_for_embedding: Vec<ToolEntry> = Vec::new();

        for tool in tools {
            let original = if tool.original_name.is_empty() {
                tool.name.clone()
            } else {
                tool.original_name.clone()
            };

            // Always register the namespaced key
            let ns_key = format!("{}.{}", namespace, original);
            let ns_entry = ToolEntry {
                name: ns_key.clone(),
                original_name: original.clone(),
                description: tool.description.clone(),
                backend_name: backend_name.to_string(),
                input_schema: tool.input_schema.clone(),
            };
            self.tools.insert(ns_key.clone(), ns_entry.clone());
            registered_keys.push(ns_key);

            #[cfg(feature = "semantic")]
            entries_for_embedding.push(ns_entry);

            // Track bare name ownership for collision detection
            self.bare_name_owners
                .entry(original.clone())
                .or_default()
                .push((backend_name.to_string(), namespace.to_string()));

            let owners = self.bare_name_owners.get(&original).unwrap();
            if owners.len() == 1 {
                // No collision — also register bare name for backward compat
                let bare_entry = ToolEntry {
                    name: original.clone(),
                    original_name: original.clone(),
                    description: tool.description,
                    backend_name: backend_name.to_string(),
                    input_schema: tool.input_schema,
                };
                self.tools.insert(original.clone(), bare_entry);
                registered_keys.push(original);
            } else if owners.len() == 2 {
                // First collision — remove bare name alias, log warning
                self.tools.remove(&original);
                tracing::warn!(
                    tool = %original,
                    backends = ?owners.iter().map(|(b, _)| b.as_str()).collect::<Vec<_>>(),
                    "tool name collision detected, use backend.tool notation"
                );
            }
            // len > 2: bare alias already removed, nothing to do
        }

        // Update embeddings with namespaced entries
        #[cfg(feature = "semantic")]
        if let Some(ref index) = self.embedding_index {
            index.add_tools(&entries_for_embedding);
        }

        self.backend_tools
            .insert(backend_name.to_string(), registered_keys);
    }

    /// Remove all tools belonging to a backend.
    ///
    /// Also cleans up bare_name_owners and restores bare-name aliases if
    /// a collision resolves (goes from 2→1 owner).
    pub fn remove_backend_tools(&self, backend_name: &str) {
        if let Some((_, tool_names)) = self.backend_tools.remove(backend_name) {
            #[cfg(feature = "semantic")]
            if let Some(ref index) = self.embedding_index {
                index.remove_tools(&tool_names);
            }

            for name in &tool_names {
                self.tools.remove(name);
            }

            // Clean up bare_name_owners and restore bare aliases if collision resolves
            let mut to_restore: Vec<(String, String, String)> = Vec::new(); // (bare_name, remaining_backend, remaining_ns)
            self.bare_name_owners.retain(|bare_name, owners| {
                let had_collision = owners.len() > 1;
                owners.retain(|(b, _)| b != backend_name);
                if owners.is_empty() {
                    return false; // Remove entry entirely
                }
                // If collision just resolved (was >1, now ==1), restore bare alias
                if had_collision && owners.len() == 1 {
                    let (ref remaining_backend, ref remaining_ns) = owners[0];
                    to_restore.push((
                        bare_name.clone(),
                        remaining_backend.clone(),
                        remaining_ns.clone(),
                    ));
                }
                true
            });

            // Restore bare-name aliases for resolved collisions
            for (bare_name, remaining_backend, remaining_ns) in to_restore {
                let ns_key = format!("{}.{}", remaining_ns, bare_name);
                if let Some(ns_entry) = self.tools.get(&ns_key) {
                    let bare_entry = ToolEntry {
                        name: bare_name.clone(),
                        original_name: bare_name.clone(),
                        description: ns_entry.description.clone(),
                        backend_name: remaining_backend,
                        input_schema: ns_entry.input_schema.clone(),
                    };
                    self.tools.insert(bare_name, bare_entry);
                }
            }
        }
    }

    /// Get all tool entries.
    pub fn get_all(&self) -> Vec<ToolEntry> {
        self.tools.iter().map(|r| r.value().clone()).collect()
    }

    /// Get all tool names.
    pub fn get_all_names(&self) -> Vec<String> {
        self.tools.iter().map(|r| r.key().clone()).collect()
    }

    /// Look up a tool by exact name.
    ///
    /// Supports both bare names (`get_repo`) and namespaced names (`github.get_repo`).
    /// Bare names only resolve if there is no collision (single owner).
    pub fn get_by_name(&self, name: &str) -> Option<ToolEntry> {
        self.tools.get(name).map(|r| r.value().clone())
    }

    /// Get all tools for a specific backend.
    #[allow(dead_code)]
    pub fn get_by_backend(&self, backend_name: &str) -> Vec<ToolEntry> {
        if let Some(names) = self.backend_tools.get(backend_name) {
            names
                .iter()
                .filter_map(|n| self.tools.get(n).map(|r| r.value().clone()))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Search tools using BM25 ranking on name and description.
    ///
    /// BM25 parameters: k1=1.2 (term frequency saturation), b=0.75 (length normalization).
    /// Tool names are tokenized by splitting underscores/hyphens (e.g., "get_current_time"
    /// becomes ["get", "current", "time"]). Name tokens get a 2x boost over description tokens.
    pub fn search(&self, query: &str, limit: u32) -> Vec<ToolEntry> {
        let query_terms = tokenize(query);
        if query_terms.is_empty() {
            return Vec::new();
        }

        // Build corpus: collect (entry, doc_tokens) for all tools
        let corpus: Vec<(ToolEntry, Vec<String>)> = self
            .tools
            .iter()
            .map(|r| {
                let entry = r.value().clone();
                let mut tokens = tokenize(&entry.name);
                // Name tokens get 2x weight by appearing twice
                let name_tokens = tokens.clone();
                tokens.extend(name_tokens);
                tokens.extend(tokenize(&entry.description));
                (entry, tokens)
            })
            .collect();

        let n = corpus.len() as f64;
        if n == 0.0 {
            return Vec::new();
        }

        // Average document length
        let avgdl: f64 = corpus.iter().map(|(_, t)| t.len() as f64).sum::<f64>() / n;

        // Document frequency: how many docs contain each query term
        let mut df: HashMap<&str, f64> = HashMap::new();
        for term in &query_terms {
            let count = corpus
                .iter()
                .filter(|(_, tokens)| tokens.iter().any(|t| t == term))
                .count();
            df.insert(term.as_str(), count as f64);
        }

        // Score each document
        const K1: f64 = 1.2;
        const B: f64 = 0.75;

        let mut scored: Vec<(ToolEntry, f64)> = corpus
            .into_iter()
            .filter_map(|(entry, tokens)| {
                let dl = tokens.len() as f64;

                // Term frequencies in this doc
                let mut tf: HashMap<&str, f64> = HashMap::new();
                for term in &query_terms {
                    let count = tokens
                        .iter()
                        .filter(|t| t.as_str() == term.as_str())
                        .count();
                    tf.insert(term.as_str(), count as f64);
                }

                let mut score = 0.0f64;
                for term in &query_terms {
                    let term_freq = tf.get(term.as_str()).copied().unwrap_or(0.0);
                    if term_freq == 0.0 {
                        continue;
                    }
                    let doc_freq = df.get(term.as_str()).copied().unwrap_or(0.0);

                    // IDF: log((N - df + 0.5) / (df + 0.5) + 1)
                    let idf = ((n - doc_freq + 0.5) / (doc_freq + 0.5) + 1.0).ln();

                    // TF component with length normalization
                    let tf_norm =
                        (term_freq * (K1 + 1.0)) / (term_freq + K1 * (1.0 - B + B * dl / avgdl));

                    score += idf * tf_norm;
                }

                if score > 0.0 {
                    Some((entry, score))
                } else {
                    None
                }
            })
            .collect();

        // Sort by score descending, then by name for stability
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.name.cmp(&b.0.name))
        });

        scored.truncate(limit as usize);
        scored.into_iter().map(|(entry, _)| entry).collect()
    }

    /// Total number of registered tools.
    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    /// Number of registered backends.
    pub fn backend_count(&self) -> usize {
        self.backend_tools.len()
    }

    /// Get backend name for a given tool.
    #[allow(dead_code)]
    pub fn get_backend_for_tool(&self, tool_name: &str) -> Option<String> {
        self.tools
            .get(tool_name)
            .map(|r| r.value().backend_name.clone())
    }

    /// Get all backend names.
    #[allow(dead_code)]
    pub fn get_backend_names(&self) -> Vec<String> {
        self.backend_tools.iter().map(|r| r.key().clone()).collect()
    }

    /// Hybrid BM25 + semantic search using Reciprocal Rank Fusion (RRF).
    ///
    /// RRF combines ranked lists from different retrievers by scoring each result as
    /// `1 / (k + rank)` where k=60 is the standard IR constant. This normalizes
    /// the incomparable BM25 scores (0-15+) and cosine similarities (0-1) into a
    /// single ranking without hyperparameter tuning.
    #[cfg(feature = "semantic")]
    pub fn search_hybrid(&self, query: &str, limit: u32) -> Vec<ToolEntry> {
        let index = match &self.embedding_index {
            Some(idx) if !idx.is_empty() => idx,
            _ => return self.search(query, limit),
        };

        const RRF_K: f64 = 60.0;
        let fetch_limit = limit.max(30); // Fetch more candidates for fusion

        // Get BM25 ranked results
        let bm25_results = self.search(query, fetch_limit);

        // Get semantic ranked results
        let semantic_results = index.search(query, fetch_limit as usize);

        // RRF: accumulate 1/(k + rank) for each retriever
        let mut rrf_scores: HashMap<String, f64> = HashMap::new();

        for (rank, entry) in bm25_results.iter().enumerate() {
            *rrf_scores.entry(entry.name.clone()).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
        }

        for (rank, (name, _similarity)) in semantic_results.iter().enumerate() {
            *rrf_scores.entry(name.clone()).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
        }

        // Sort by combined RRF score descending
        let mut scored: Vec<(String, f64)> = rrf_scores.into_iter().collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        scored.truncate(limit as usize);

        // Look up full ToolEntry for each result
        scored
            .into_iter()
            .filter_map(|(name, _)| self.tools.get(&name).map(|r| r.value().clone()))
            .collect()
    }

    /// Export embedding vectors for cache persistence.
    #[cfg(feature = "semantic")]
    pub fn embedding_snapshot(&self) -> Option<HashMap<String, Vec<f32>>> {
        self.embedding_index.as_ref().map(|idx| idx.snapshot())
    }

    /// Restore embedding vectors from cache (avoids re-embedding on startup).
    #[cfg(feature = "semantic")]
    pub fn load_embeddings(&self, data: HashMap<String, Vec<f32>>) {
        if let Some(ref index) = self.embedding_index {
            index.load_snapshot(data);
        }
    }

    /// Export all tools grouped by backend name (for cache serialization).
    ///
    /// Only exports namespaced entries (entries where `name != original_name`),
    /// since bare-name aliases are recreated by `register_backend_tools` on load.
    /// Falls back to including all entries if no namespacing is active
    /// (pre-namespace caches or backends with no collisions).
    pub fn snapshot(&self) -> HashMap<String, Vec<ToolEntry>> {
        let mut result: HashMap<String, Vec<ToolEntry>> = HashMap::new();
        let mut seen: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        for entry in self.tools.iter() {
            let e = entry.value();
            let key = (e.backend_name.clone(), e.original_name.clone());
            // Deduplicate: only include one entry per (backend, original_name).
            // Prefer the namespaced entry (name != original_name) over the bare alias.
            if seen.contains(&key) {
                continue;
            }
            // Skip bare aliases — they share (backend, original_name) with the namespaced entry
            if !e.original_name.is_empty() && e.name == e.original_name {
                // Check if a namespaced version exists
                let has_namespaced = self.tools.iter().any(|other| {
                    let o = other.value();
                    o.backend_name == e.backend_name
                        && o.original_name == e.original_name
                        && o.name != o.original_name
                });
                if has_namespaced {
                    continue; // Skip bare alias, the namespaced entry will be included
                }
            }
            seen.insert(key);
            result
                .entry(e.backend_name.clone())
                .or_default()
                .push(e.clone());
        }
        result
    }
}

/// Tokenize text into lowercase terms, splitting on non-alphanumeric characters.
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
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
            input_schema: json!({"type": "object"}),
        }
    }

    #[test]
    fn test_register_and_get() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "exa",
            vec![
                make_entry("web_search", "Search the web", "exa"),
                make_entry("find_similar", "Find similar pages", "exa"),
            ],
        );

        // 2 namespaced + 2 bare aliases (no collisions)
        assert_eq!(reg.tool_count(), 4);
        assert_eq!(reg.backend_count(), 1);

        // Bare name resolves
        let tool = reg.get_by_name("web_search").unwrap();
        assert_eq!(tool.backend_name, "exa");
        assert_eq!(tool.description, "Search the web");

        // Namespaced name also resolves
        let tool = reg.get_by_name("exa.web_search").unwrap();
        assert_eq!(tool.backend_name, "exa");
    }

    #[test]
    fn test_remove_backend() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools("exa", vec![make_entry("web_search", "Search", "exa")]);
        reg.register_backend_tools(
            "tavily",
            vec![make_entry("tavily_search", "Search with Tavily", "tavily")],
        );

        // 2 namespaced + 2 bare (no collisions)
        assert_eq!(reg.tool_count(), 4);
        reg.remove_backend_tools("exa");
        // 1 namespaced + 1 bare
        assert_eq!(reg.tool_count(), 2);
        assert!(reg.get_by_name("web_search").is_none());
        assert!(reg.get_by_name("exa.web_search").is_none());
        assert!(reg.get_by_name("tavily_search").is_some());
        assert!(reg.get_by_name("tavily.tavily_search").is_some());
    }

    #[test]
    fn test_no_collision_bare_and_namespaced() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "exa",
            vec![make_entry("web_search", "Search via Exa", "exa")],
        );
        reg.register_backend_tools(
            "tavily",
            vec![make_entry("tavily_search", "Search via Tavily", "tavily")],
        );

        // No collision: both bare and namespaced resolve
        assert!(reg.get_by_name("web_search").is_some());
        assert!(reg.get_by_name("exa.web_search").is_some());
        assert!(reg.get_by_name("tavily_search").is_some());
        assert!(reg.get_by_name("tavily.tavily_search").is_some());
    }

    #[test]
    fn test_collision_removes_bare_name() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "github",
            vec![make_entry("get_repo", "Get GitHub repo", "github")],
        );
        reg.register_backend_tools(
            "vibe_kanban",
            vec![make_entry("get_repo", "Get Kanban repo", "vibe_kanban")],
        );

        // Bare name removed due to collision
        assert!(reg.get_by_name("get_repo").is_none());

        // Namespaced names resolve
        let gh = reg.get_by_name("github.get_repo").unwrap();
        assert_eq!(gh.backend_name, "github");
        let vk = reg.get_by_name("vibe_kanban.get_repo").unwrap();
        assert_eq!(vk.backend_name, "vibe_kanban");
    }

    #[test]
    fn test_custom_namespace() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools_namespaced(
            "github",
            "gh",
            vec![make_entry("get_repo", "Get repo", "github")],
        );

        // Custom namespace "gh" instead of "github"
        assert!(reg.get_by_name("gh.get_repo").is_some());
        assert!(reg.get_by_name("github.get_repo").is_none());
        // Bare name also works (no collision)
        assert!(reg.get_by_name("get_repo").is_some());
    }

    #[test]
    fn test_remove_backend_restores_bare_name() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "github",
            vec![make_entry("get_repo", "Get GitHub repo", "github")],
        );
        reg.register_backend_tools(
            "vibe_kanban",
            vec![make_entry("get_repo", "Get Kanban repo", "vibe_kanban")],
        );

        // Collision: bare name removed
        assert!(reg.get_by_name("get_repo").is_none());

        // Remove one backend — collision resolves
        reg.remove_backend_tools("vibe_kanban");

        // Bare name restored
        let entry = reg.get_by_name("get_repo").unwrap();
        assert_eq!(entry.backend_name, "github");
        assert!(reg.get_by_name("github.get_repo").is_some());
    }

    #[test]
    fn test_search_finds_namespaced_tools() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "github",
            vec![make_entry("get_repo", "Get a GitHub repository", "github")],
        );

        // Search for "get_repo" should find both the bare and namespaced entries
        let results = reg.search("get repo", 10);
        assert!(!results.is_empty(), "search should find namespaced tools");
        // Should find entries with "get" and "repo" tokens
        assert!(results.iter().any(|r| r.name.contains("get_repo")));
    }

    #[test]
    fn test_search() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "exa",
            vec![
                make_entry("web_search", "Search the web using Exa", "exa"),
                make_entry("find_similar", "Find similar content", "exa"),
            ],
        );
        reg.register_backend_tools(
            "tavily",
            vec![make_entry(
                "tavily_search",
                "Web search via Tavily",
                "tavily",
            )],
        );

        // "similar" only in find_similar entries
        let results = reg.search("similar", 10);
        assert!(!results.is_empty());
        assert!(results.iter().any(|r| r.original_name == "find_similar"));
    }

    #[test]
    fn test_search_limit() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "test",
            vec![
                make_entry("tool_a", "A tool", "test"),
                make_entry("tool_b", "B tool", "test"),
                make_entry("tool_c", "C tool", "test"),
            ],
        );

        let results = reg.search("tool", 2);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_bm25_ranking_name_boost() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "backend",
            vec![
                make_entry("list_files", "List all files in a directory", "backend"),
                make_entry("search_code", "Search through code for patterns", "backend"),
                make_entry(
                    "delete_file",
                    "Delete a file from the filesystem",
                    "backend",
                ),
            ],
        );

        // "file" exact token: delete_file has "file" (name match + desc match)
        let results = reg.search("file", 10);
        assert!(results.iter().any(|r| r.original_name == "delete_file"));

        // Multi-term: "delete file" — delete_file matches both terms in name
        let results = reg.search("delete file", 10);
        assert!(results[0].original_name == "delete_file");
    }

    #[test]
    fn test_bm25_multi_term_query() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "backend",
            vec![
                make_entry(
                    "get_current_time",
                    "Get current time in a specific timezone",
                    "backend",
                ),
                make_entry("convert_time", "Convert time between timezones", "backend"),
                make_entry(
                    "get_weather",
                    "Get current weather for a location",
                    "backend",
                ),
            ],
        );

        // Multi-term: "current time" should rank get_current_time highest
        let results = reg.search("current time", 10);
        assert!(results[0].original_name == "get_current_time");
    }

    #[test]
    fn test_bm25_no_match() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "backend",
            vec![make_entry("web_search", "Search the web", "backend")],
        );

        let results = reg.search("database", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_tokenize() {
        assert_eq!(
            super::tokenize("get_current_time"),
            vec!["get", "current", "time"]
        );
        assert_eq!(
            super::tokenize("streamable-http"),
            vec!["streamable", "http"]
        );
        assert_eq!(
            super::tokenize("Search the WEB"),
            vec!["search", "the", "web"]
        );
    }

    #[test]
    fn test_tool_entry_serde_default() {
        // Deserialize without original_name field (v1/v2 cache compat)
        let json = r#"{"name":"test","description":"desc","backend_name":"b","input_schema":{}}"#;
        let entry: ToolEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.original_name, ""); // default
        assert_eq!(entry.name, "test");
    }

    #[test]
    fn test_re_registration_preserves_bare_names() {
        // Simulates cache load → live discovery re-registration.
        // Bare names should still work after re-registration.
        let reg = ToolRegistry::new();

        // First registration (cache load)
        reg.register_backend_tools(
            "time",
            vec![make_entry("get_current_time", "Get current time", "time")],
        );
        assert!(
            reg.get_by_name("get_current_time").is_some(),
            "bare name after first registration"
        );
        assert!(
            reg.get_by_name("time.get_current_time").is_some(),
            "namespaced after first registration"
        );

        // Second registration (live discovery) — same backend, same tools
        reg.register_backend_tools(
            "time",
            vec![make_entry(
                "get_current_time",
                "Get current time in a timezone",
                "time",
            )],
        );
        assert!(
            reg.get_by_name("get_current_time").is_some(),
            "bare name after re-registration"
        );
        assert!(
            reg.get_by_name("time.get_current_time").is_some(),
            "namespaced after re-registration"
        );

        // Should be 2 entries (1 namespaced + 1 bare), not false collision
        assert_eq!(reg.tool_count(), 2);
    }

    #[test]
    fn test_re_registration_with_collision_still_detects() {
        // After re-registration, real collisions should still be detected.
        let reg = ToolRegistry::new();

        // Register time (from cache)
        reg.register_backend_tools(
            "time",
            vec![make_entry("get_current_time", "Get time", "time")],
        );

        // Register another backend with same tool name
        reg.register_backend_tools(
            "other",
            vec![make_entry("get_current_time", "Other time", "other")],
        );

        // Collision: bare name removed
        assert!(
            reg.get_by_name("get_current_time").is_none(),
            "bare name should be gone (collision)"
        );
        assert!(reg.get_by_name("time.get_current_time").is_some());
        assert!(reg.get_by_name("other.get_current_time").is_some());

        // Re-register time (live discovery) — collision should persist
        reg.register_backend_tools(
            "time",
            vec![make_entry("get_current_time", "Get time updated", "time")],
        );
        assert!(
            reg.get_by_name("get_current_time").is_none(),
            "bare name should still be gone after re-reg"
        );
        assert!(reg.get_by_name("time.get_current_time").is_some());
        assert!(reg.get_by_name("other.get_current_time").is_some());
    }
}
