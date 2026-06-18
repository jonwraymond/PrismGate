//! Tool groups: named collections of tools for scoped discovery and multi-agent isolation.
//!
//! Groups aggregate tools from multiple backends using three membership types:
//! - `explicit`: enumerate specific tool names
//! - `by_tag`: all tools from backends that carry a given tag
//! - `by_backend`: all tools from a specific backend
//!
//! Groups are configured statically in `Config.tool_groups`. Membership is resolved
//! at lookup time against the live `ToolRegistry` — no caching of the member set is
//! needed since the registry already handles cache invalidation.
//!
//! ## Future RBAC integration
//!
//! `GroupPermission.allow/deny` are currently informational. Full enforcement requires
//! external middleware or per-session principal injection. The `permissions()` method
//! exposes the config so future middleware can inspect it without re-parsing YAML.

use std::collections::HashMap;
use std::sync::Arc;

use crate::config::{GroupMembership, GroupPermission, ToolGroupConfig};
use crate::registry::{ToolEntry, ToolRegistry};

/// Live state for all tool groups. Built once at startup from Config and
/// cheaply cloned into each GateminiServer instance.
///
/// TODO: wire into GateminiServer for group-scoped discovery.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct ToolGroups {
    /// group_name -> config
    configs: HashMap<String, ToolGroupConfig>,
}

#[allow(dead_code)]
impl ToolGroups {
    /// Build from a config map. Groups with `enabled: false` are stored but
    /// return empty member sets from all query methods.
    pub fn from_config(config: HashMap<String, ToolGroupConfig>) -> Arc<Self> {
        Arc::new(Self { configs: config })
    }

    /// Return metadata for all groups (names, descriptions, membership counts).
    /// Excludes disabled groups.
    pub fn list_groups(&self, registry: &ToolRegistry) -> Vec<GroupMeta> {
        self.configs
            .iter()
            .filter(|(_, cfg)| cfg.enabled)
            .map(|(name, cfg)| {
                let members = self.resolve_members(name, registry);
                GroupMeta {
                    name: name.clone(),
                    description: cfg.description.clone(),
                    tool_count: members.len(),
                    tags: cfg.tags.clone(),
                }
            })
            .collect()
    }

    /// Return full config for one group (informational — used by group_info).
    pub fn get_config(&self, name: &str) -> Option<&ToolGroupConfig> {
        self.configs.get(name)
    }

    /// Return the resolved tool names that belong to `group_name`.
    /// Returns an empty vec if the group is unknown or disabled.
    pub fn tools_in_group(&self, group_name: &str, registry: &ToolRegistry) -> Vec<String> {
        let cfg = match self.configs.get(group_name) {
            Some(c) if c.enabled => c,
            _ => return Vec::new(),
        };
        let members = resolve_membership(&cfg.members, registry);
        members.into_iter().map(|e| e.name).collect()
    }

    /// Return the resolved ToolEntries that belong to `group_name`.
    /// Returns an empty vec if the group is unknown or disabled.
    pub fn entries_in_group(&self, group_name: &str, registry: &ToolRegistry) -> Vec<ToolEntry> {
        if !self
            .configs
            .get(group_name)
            .map(|c| c.enabled)
            .unwrap_or(false)
        {
            return Vec::new();
        }
        resolve_membership(&self.configs[group_name].members, registry)
    }

    /// Search tools within a specific group using BM25.
    /// Returns results ranked by relevance, filtered to only tools in the group.
    pub fn search_in_group(
        &self,
        group_name: &str,
        registry: &ToolRegistry,
        query: &str,
        limit: u32,
        tracker: Option<&crate::tracker::CallTracker>,
    ) -> Vec<ToolEntry> {
        if !self
            .configs
            .get(group_name)
            .map(|c| c.enabled)
            .unwrap_or(false)
        {
            return Vec::new();
        }
        let member_names: std::collections::HashSet<_> = self
            .tools_in_group(group_name, registry)
            .into_iter()
            .collect();

        registry
            .search(query, limit, None, tracker)
            .into_iter()
            .filter(|e| member_names.contains(&e.name))
            .collect()
    }

    /// Return a summary of the access-control entries for RBAC middleware.
    pub fn permissions(&self) -> HashMap<String, &GroupPermission> {
        self.configs
            .iter()
            .map(|(k, v)| (k.clone(), &v.permissions))
            .collect()
    }

    /// Resolve the raw member list from config against the live registry.
    fn resolve_members(&self, group_name: &str, registry: &ToolRegistry) -> Vec<ToolEntry> {
        match self.configs.get(group_name) {
            Some(cfg) if cfg.enabled => resolve_membership(&cfg.members, registry),
            _ => Vec::new(),
        }
    }
}

/// Shared logic for converting a membership spec into actual ToolEntries.
#[allow(dead_code)]
fn resolve_membership(members: &[GroupMembership], registry: &ToolRegistry) -> Vec<ToolEntry> {
    let mut seen = std::collections::HashSet::new();
    let mut entries = Vec::new();

    for m in members {
        match m {
            GroupMembership::Explicit { tools } => {
                for name in tools {
                    if let Some(entry) = registry.get_by_name(name)
                        && seen.insert(entry.name.clone())
                    {
                        entries.push(entry);
                    }
                }
            }
            GroupMembership::ByTag { tag } => {
                // All tools whose backend carries this tag (entry.tags is inherited from backend config).
                // Only consider namespaced entries — bare-name aliases are duplicates.
                for entry in registry.get_all() {
                    if !entry.name.contains('.') {
                        continue;
                    }
                    if seen.contains(&entry.name) {
                        continue;
                    }
                    if entry.tags.contains(tag) {
                        seen.insert(entry.name.clone());
                        entries.push(entry);
                    }
                }
            }
            GroupMembership::ByBackend { backend } => {
                for entry in registry.get_by_backend(backend) {
                    if seen.insert(entry.name.clone()) {
                        entries.push(entry);
                    }
                }
            }
        }
    }
    entries
}

/// Group metadata returned by list_groups.
#[derive(Debug, serde::Serialize)]
#[allow(dead_code)]
pub struct GroupMeta {
    pub name: String,
    pub description: String,
    pub tool_count: usize,
    /// Inherited tags for this group (from config).
    pub tags: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GroupMembership, GroupPermission, ToolGroupConfig};
    use crate::registry::{ToolEntry, ToolRegistry};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_entry(name: &str, desc: &str, backend: &str) -> ToolEntry {
        ToolEntry {
            name: name.to_string(),
            original_name: name.to_string(),
            description: desc.to_string(),
            backend_name: backend.to_string(),
            input_schema: json!({"type": "object"}),
            tags: Vec::new(),
        }
    }

    fn make_entry_with_tags(name: &str, desc: &str, backend: &str, tags: Vec<String>) -> ToolEntry {
        ToolEntry {
            name: name.to_string(),
            original_name: name.to_string(),
            description: desc.to_string(),
            backend_name: backend.to_string(),
            input_schema: json!({"type": "object"}),
            tags,
        }
    }

    fn make_registry() -> Arc<ToolRegistry> {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "exa",
            vec![make_entry("web_search", "Search the web", "exa")],
        );
        reg.register_backend_tools(
            "github",
            vec![make_entry("get_repo", "Get a GitHub repo", "github")],
        );
        reg.register_backend_tools(
            "tavily",
            vec![
                make_entry_with_tags(
                    "search",
                    "Tavily search",
                    "tavily",
                    vec!["premium".to_string()],
                ),
                make_entry("extract", "Tavily extract", "tavily"),
            ],
        );
        reg
    }

    fn make_config(desc: &str, members: Vec<GroupMembership>) -> ToolGroupConfig {
        ToolGroupConfig {
            description: desc.to_string(),
            members,
            permissions: GroupPermission::default(),
            enabled: true,
            tags: Vec::new(),
        }
    }

    #[test]
    fn test_from_config_empty() {
        let tg = ToolGroups::from_config(HashMap::new());
        let reg = make_registry();
        assert!(tg.list_groups(&reg).is_empty());
    }

    #[test]
    fn test_from_config_disabled_group() {
        let mut configs = HashMap::new();
        configs.insert(
            "admin".to_string(),
            ToolGroupConfig {
                description: "admin tools".to_string(),
                members: vec![GroupMembership::ByBackend {
                    backend: "exa".to_string(),
                }],
                permissions: GroupPermission::default(),
                enabled: false,
                tags: vec![],
            },
        );
        let tg = ToolGroups::from_config(configs);
        let reg = make_registry();
        // Disabled groups don't appear in list
        let groups = tg.list_groups(&reg);
        assert!(groups.is_empty());
        // tools_in_group returns empty for disabled
        assert!(tg.tools_in_group("admin", &reg).is_empty());
    }

    #[test]
    fn test_list_groups_enabled() {
        let mut configs = HashMap::new();
        configs.insert(
            "search_group".to_string(),
            make_config(
                "Search tools",
                vec![GroupMembership::ByBackend {
                    backend: "exa".to_string(),
                }],
            ),
        );
        configs.insert(
            "dev_group".to_string(),
            make_config(
                "Dev tools",
                vec![GroupMembership::ByBackend {
                    backend: "github".to_string(),
                }],
            ),
        );
        let tg = ToolGroups::from_config(configs);
        let reg = make_registry();
        let groups = tg.list_groups(&reg);
        assert_eq!(groups.len(), 2);
        // Order is non-deterministic in HashMap, check by name
        let search_group = groups.iter().find(|g| g.name == "search_group").unwrap();
        assert_eq!(search_group.description, "Search tools");
        assert_eq!(search_group.tool_count, 1);
        let dev_group = groups.iter().find(|g| g.name == "dev_group").unwrap();
        assert_eq!(dev_group.description, "Dev tools");
        assert_eq!(dev_group.tool_count, 1);
    }

    #[test]
    fn test_resolve_explicit_membership() {
        let mut configs = HashMap::new();
        configs.insert(
            "explicit_group".to_string(),
            make_config(
                "Explicit tools",
                vec![GroupMembership::Explicit {
                    tools: vec!["exa.web_search".to_string(), "github.get_repo".to_string()],
                }],
            ),
        );
        let tg = ToolGroups::from_config(configs);
        let reg = make_registry();
        let tools = tg.tools_in_group("explicit_group", &reg);
        assert_eq!(tools.len(), 2);
        assert!(tools.contains(&"exa.web_search".to_string()));
        assert!(tools.contains(&"github.get_repo".to_string()));
    }

    #[test]
    fn test_resolve_explicit_nonexistent_tool() {
        let mut configs = HashMap::new();
        configs.insert(
            "partial_group".to_string(),
            make_config(
                "Partial",
                vec![GroupMembership::Explicit {
                    tools: vec!["exa.web_search".to_string(), "nonexistent.tool".to_string()],
                }],
            ),
        );
        let tg = ToolGroups::from_config(configs);
        let reg = make_registry();
        let tools = tg.tools_in_group("partial_group", &reg);
        // Only the existing tool is returned
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0], "exa.web_search");
    }

    #[test]
    fn test_resolve_by_backend() {
        let mut configs = HashMap::new();
        configs.insert(
            "tavily_group".to_string(),
            make_config(
                "All Tavily",
                vec![GroupMembership::ByBackend {
                    backend: "tavily".to_string(),
                }],
            ),
        );
        let tg = ToolGroups::from_config(configs);
        let reg = make_registry();
        let tools = tg.tools_in_group("tavily_group", &reg);
        assert_eq!(tools.len(), 2);
        assert!(tools.contains(&"tavily.search".to_string()));
        assert!(tools.contains(&"tavily.extract".to_string()));
    }

    #[test]
    fn test_resolve_by_tag() {
        let mut configs = HashMap::new();
        configs.insert(
            "premium_group".to_string(),
            make_config(
                "Premium features",
                vec![GroupMembership::ByTag {
                    tag: "premium".to_string(),
                }],
            ),
        );
        let tg = ToolGroups::from_config(configs);
        let reg = make_registry();
        let tools = tg.tools_in_group("premium_group", &reg);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0], "tavily.search");
    }

    #[test]
    fn test_resolve_by_tag_no_match() {
        let mut configs = HashMap::new();
        configs.insert(
            "empty_group".to_string(),
            make_config(
                "Nothing matches",
                vec![GroupMembership::ByTag {
                    tag: "nonexistent_tag".to_string(),
                }],
            ),
        );
        let tg = ToolGroups::from_config(configs);
        let reg = make_registry();
        let tools = tg.tools_in_group("empty_group", &reg);
        assert!(tools.is_empty());
    }

    #[test]
    fn test_resolve_mixed_membership() {
        let mut configs = HashMap::new();
        configs.insert(
            "mixed".to_string(),
            make_config(
                "Mixed",
                vec![
                    GroupMembership::Explicit {
                        tools: vec!["github.get_repo".to_string()],
                    },
                    GroupMembership::ByBackend {
                        backend: "exa".to_string(),
                    },
                ],
            ),
        );
        let tg = ToolGroups::from_config(configs);
        let reg = make_registry();
        let tools = tg.tools_in_group("mixed", &reg);
        assert_eq!(tools.len(), 2);
        assert!(tools.contains(&"exa.web_search".to_string()));
        assert!(tools.contains(&"github.get_repo".to_string()));
    }

    #[test]
    fn test_resolve_dedup_overlapping_membership() {
        // Explicitly include a tool that's also pulled in by backend
        let mut configs = HashMap::new();
        configs.insert(
            "dedup".to_string(),
            make_config(
                "Dedup test",
                vec![
                    GroupMembership::Explicit {
                        tools: vec!["exa.web_search".to_string()],
                    },
                    GroupMembership::ByBackend {
                        backend: "exa".to_string(),
                    },
                ],
            ),
        );
        let tg = ToolGroups::from_config(configs);
        let reg = make_registry();
        let tools = tg.tools_in_group("dedup", &reg);
        assert_eq!(tools.len(), 1, "duplicate tools should be deduplicated");
    }

    #[test]
    fn test_get_config() {
        let mut configs = HashMap::new();
        configs.insert("search".to_string(), make_config("Search", vec![]));
        let tg = ToolGroups::from_config(configs);
        assert!(tg.get_config("search").is_some());
        assert!(tg.get_config("nonexistent").is_none());
    }

    #[test]
    fn test_permissions() {
        let mut configs = HashMap::new();
        configs.insert(
            "admin_group".to_string(),
            ToolGroupConfig {
                description: "admin".to_string(),
                members: vec![],
                permissions: GroupPermission {
                    allow: vec!["role:admin".to_string()],
                    deny: vec!["user:guest".to_string()],
                },
                enabled: true,
                tags: vec![],
            },
        );
        let tg = ToolGroups::from_config(configs);
        let perms = tg.permissions();
        assert_eq!(perms.len(), 1);
        let admin_perm = perms.get("admin_group").unwrap();
        assert_eq!(admin_perm.allow, vec!["role:admin"]);
        assert_eq!(admin_perm.deny, vec!["user:guest"]);
    }

    #[test]
    fn test_search_in_group() {
        let mut configs = HashMap::new();
        configs.insert(
            "search_group".to_string(),
            make_config(
                "Search tools",
                vec![
                    GroupMembership::ByBackend {
                        backend: "exa".to_string(),
                    },
                    GroupMembership::ByBackend {
                        backend: "tavily".to_string(),
                    },
                ],
            ),
        );
        let tg = ToolGroups::from_config(configs);
        let reg = make_registry();
        let results = tg.search_in_group("search_group", &reg, "search", 10, None);
        // Both exa.web_search and tavily.search should match "search"
        assert_eq!(results.len(), 2);
        let names: Vec<&str> = results.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"exa.web_search"));
        assert!(names.contains(&"tavily.search"));
    }

    #[test]
    fn test_search_in_group_disabled() {
        let mut configs = HashMap::new();
        configs.insert(
            "disabled_group".to_string(),
            ToolGroupConfig {
                description: "disabled".to_string(),
                members: vec![GroupMembership::ByBackend {
                    backend: "exa".to_string(),
                }],
                permissions: GroupPermission::default(),
                enabled: false,
                tags: vec![],
            },
        );
        let tg = ToolGroups::from_config(configs);
        let reg = make_registry();
        let results = tg.search_in_group("disabled_group", &reg, "search", 10, None);
        assert!(results.is_empty());
    }

    #[test]
    fn test_unknown_group_returns_empty() {
        let tg = ToolGroups::from_config(HashMap::new());
        let reg = make_registry();
        assert!(tg.tools_in_group("nonexistent", &reg).is_empty());
        assert!(tg.entries_in_group("nonexistent", &reg).is_empty());
        assert!(
            tg.search_in_group("nonexistent", &reg, "query", 10, None)
                .is_empty()
        );
    }

    #[test]
    fn test_entries_in_group() {
        let mut configs = HashMap::new();
        configs.insert(
            "exa_group".to_string(),
            make_config(
                "Exa",
                vec![GroupMembership::ByBackend {
                    backend: "exa".to_string(),
                }],
            ),
        );
        let tg = ToolGroups::from_config(configs);
        let reg = make_registry();
        let entries = tg.entries_in_group("exa_group", &reg);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "exa.web_search");
        assert_eq!(entries[0].backend_name, "exa");
    }
}
