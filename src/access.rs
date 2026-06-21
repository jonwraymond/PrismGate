//! Config-based tool-level access control / RBAC primitives.
//!
//! Provides pattern-based allow/deny rules that gate access to backend tools
//! at the gateway level. Rules are evaluated at tool-call time in both the
//! direct-call path (`call_tool_by_dotted_name`) and the sandbox path
//! (`__call_tool` callback).
//!
//! # Pattern syntax
//!
//! | Pattern             | Matches                                        |
//! |---------------------|------------------------------------------------|
//! | `*`                 | All tools on all backends                      |
//! | `backend.*`         | All tools from a specific backend              |
//! | `backend.tool_name` | Exact match for a specific backend+tool pair   |
//! | `*.tool_name`       | A specific tool from any backend               |
//!
//! # Evaluation order
//!
//! 1. Deny rules are checked first (explicit denies always win).
//! 2. Allow rules are checked next (first match grants access).
//! 3. If no rule matches, the `default_policy` decides.

use serde::{Deserialize, Serialize};

/// Top-level access control configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccessControlConfig {
    /// Default policy when no rule matches. Default: `allow`.
    #[serde(default)]
    pub default_policy: AccessPolicy,

    /// Ordered list of access rules. Deny rules are evaluated before allow rules.
    #[serde(default)]
    pub rules: Vec<AccessRule>,
}

impl Default for AccessControlConfig {
    fn default() -> Self {
        Self {
            default_policy: AccessPolicy::Allow,
            rules: Vec::new(),
        }
    }
}

/// Default access policy when no rule matches a given tool.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum AccessPolicy {
    /// Allow access unless explicitly denied.
    #[default]
    Allow,
    /// Deny access unless explicitly allowed.
    Deny,
}

/// A single access control rule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccessRule {
    /// Tool patterns to allow. Supports `*`, `backend.*`, `backend.tool`, `*.tool`.
    #[serde(default)]
    pub allow: Vec<String>,

    /// Tool patterns to deny. Takes priority over allow rules.
    #[serde(default)]
    pub deny: Vec<String>,
}

impl AccessControlConfig {
    /// Check whether a tool call is permitted.
    ///
    /// `backend` is the backend name (e.g., `"exa"`) and `tool` is the
    /// original tool name (e.g., `"web_search_exa"`).
    pub fn check(&self, backend: &str, tool: &str) -> bool {
        // 1. Check deny rules — first match blocks access
        for rule in &self.rules {
            for pattern in &rule.deny {
                if match_pattern(pattern, backend, tool) {
                    return false;
                }
            }
        }

        // 2. Check allow rules — first match grants access
        for rule in &self.rules {
            for pattern in &rule.allow {
                if match_pattern(pattern, backend, tool) {
                    return true;
                }
            }
        }

        // 3. Fall back to default policy
        matches!(self.default_policy, AccessPolicy::Allow)
    }
}

/// Match a pattern against a backend + tool pair.
///
/// Patterns:
/// - `*` → matches everything
/// - `backend.*` → matches all tools from that backend
/// - `backend.tool_name` → exact match
/// - `*.tool_name` → matches that tool from any backend
fn match_pattern(pattern: &str, backend: &str, tool: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    if let Some(suffix) = pattern.strip_suffix(".*") {
        // "backend.*" — match all tools from this backend
        return suffix == backend;
    }

    if let Some(prefix) = pattern.strip_prefix("*.") {
        // "*.tool_name" — match this tool from any backend
        return prefix == tool;
    }

    // Exact match: "backend.tool_name"
    if let Some(dot) = pattern.find('.') {
        let pat_backend = &pattern[..dot];
        let pat_tool = &pattern[dot + 1..];
        return pat_backend == backend && pat_tool == tool;
    }

    // Single-segment pattern (no dot): match as backend name? No — ambiguous.
    // Treat as exact backend match (match all tools from that backend).
    pattern == backend
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(default_policy: AccessPolicy, rules: Vec<AccessRule>) -> AccessControlConfig {
        AccessControlConfig {
            default_policy,
            rules,
        }
    }

    fn rule(allow: Vec<&str>, deny: Vec<&str>) -> AccessRule {
        AccessRule {
            allow: allow.into_iter().map(String::from).collect(),
            deny: deny.into_iter().map(String::from).collect(),
        }
    }

    #[test]
    fn test_default_allow_allows_everything() {
        let ac = AccessControlConfig::default();
        assert!(ac.check("exa", "web_search"));
        assert!(ac.check("tavily", "search"));
        assert!(ac.check("any", "any_tool"));
    }

    #[test]
    fn test_default_deny_blocks_everything() {
        let ac = config(AccessPolicy::Deny, vec![]);
        assert!(!ac.check("exa", "web_search"));
        assert!(!ac.check("tavily", "search"));
    }

    #[test]
    fn test_wildcard_allow_overrides_deny_default() {
        let ac = config(AccessPolicy::Deny, vec![rule(vec!["*"], vec![])]);
        assert!(ac.check("exa", "web_search"));
        assert!(ac.check("any", "any_tool"));
    }

    #[test]
    fn test_backend_wildcard_allow() {
        let ac = config(AccessPolicy::Deny, vec![rule(vec!["exa.*"], vec![])]);
        assert!(ac.check("exa", "web_search"));
        assert!(ac.check("exa", "find_similar"));
        assert!(!ac.check("tavily", "search"));
    }

    #[test]
    fn test_tool_wildcard_allow() {
        let ac = config(AccessPolicy::Deny, vec![rule(vec!["*.web_search"], vec![])]);
        assert!(ac.check("exa", "web_search"));
        assert!(ac.check("tavily", "web_search"));
        assert!(!ac.check("exa", "find_similar"));
    }

    #[test]
    fn test_exact_match_allow() {
        let ac = config(
            AccessPolicy::Deny,
            vec![rule(vec!["exa.web_search"], vec![])],
        );
        assert!(ac.check("exa", "web_search"));
        assert!(!ac.check("exa", "find_similar"));
        assert!(!ac.check("tavily", "web_search"));
    }

    #[test]
    fn test_deny_takes_priority_over_allow() {
        // Allow all, but deny exa.* — exa tools should be blocked
        let ac = config(AccessPolicy::Deny, vec![rule(vec!["*"], vec!["exa.*"])]);
        assert!(!ac.check("exa", "web_search"));
        assert!(!ac.check("exa", "find_similar"));
        assert!(ac.check("tavily", "search"));
    }

    #[test]
    fn test_deny_specific_tool_in_allowed_backend() {
        // Allow exa.* but deny exa.web_search
        let ac = config(
            AccessPolicy::Deny,
            vec![rule(vec!["exa.*"], vec!["exa.web_search"])],
        );
        assert!(!ac.check("exa", "web_search"));
        assert!(ac.check("exa", "find_similar"));
    }

    #[test]
    fn test_multiple_rules() {
        let ac = config(
            AccessPolicy::Deny,
            vec![rule(vec!["exa.*"], vec![]), rule(vec!["tavily.*"], vec![])],
        );
        assert!(ac.check("exa", "web_search"));
        assert!(ac.check("tavily", "search"));
        assert!(!ac.check("other", "tool"));
    }

    #[test]
    fn test_no_dot_pattern_matches_backend() {
        // A pattern without a dot is treated as a backend name
        let ac = config(AccessPolicy::Deny, vec![rule(vec!["exa"], vec![])]);
        assert!(ac.check("exa", "web_search"));
        assert!(ac.check("exa", "any_tool"));
        assert!(!ac.check("tavily", "search"));
    }

    #[test]
    fn test_deny_wildcard_blocks_all() {
        let ac = config(AccessPolicy::Allow, vec![rule(vec![], vec!["*"])]);
        assert!(!ac.check("exa", "web_search"));
        assert!(!ac.check("any", "any_tool"));
    }

    #[test]
    fn test_first_deny_wins_across_rules() {
        // First rule denies exa.*, second rule allows * — deny should win
        let ac = config(
            AccessPolicy::Allow,
            vec![rule(vec![], vec!["exa.*"]), rule(vec!["*"], vec![])],
        );
        assert!(!ac.check("exa", "web_search"));
        assert!(ac.check("tavily", "search"));
    }

    #[test]
    fn test_match_pattern_wildcard() {
        assert!(match_pattern("*", "exa", "web_search"));
        assert!(match_pattern("*", "", ""));
    }

    #[test]
    fn test_match_pattern_backend_wildcard() {
        assert!(match_pattern("exa.*", "exa", "web_search"));
        assert!(match_pattern("exa.*", "exa", "any"));
        assert!(!match_pattern("exa.*", "tavily", "search"));
    }

    #[test]
    fn test_match_pattern_tool_wildcard() {
        assert!(match_pattern("*.web_search", "exa", "web_search"));
        assert!(match_pattern("*.web_search", "tavily", "web_search"));
        assert!(!match_pattern("*.web_search", "exa", "other"));
    }

    #[test]
    fn test_match_pattern_exact() {
        assert!(match_pattern("exa.web_search", "exa", "web_search"));
        assert!(!match_pattern("exa.web_search", "exa", "other"));
        assert!(!match_pattern("exa.web_search", "tavily", "web_search"));
    }

    #[test]
    fn test_match_pattern_no_dot() {
        // Single segment matches backend name
        assert!(match_pattern("exa", "exa", "any"));
        assert!(!match_pattern("exa", "tavily", "any"));
    }
}
