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
                // All tools whose backend carries this tag (entry.tags is inherited from backend config)
                for entry in registry.get_all() {
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
