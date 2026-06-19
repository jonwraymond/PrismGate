//! Role-Based Access Control for PrismGate.
//!
//! Four roles in a privilege hierarchy:
//!   Admin > Operator > Developer > ReadOnly
//!
//! Each backend and optionally each tool has a minimum required role.
//! A session's role must meet or exceed the required level to call a tool.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};
use rmcp::JsonSchema;

use crate::registry::ToolRegistry;

// ---------------------------------------------------------------------------
// Role definition
// ---------------------------------------------------------------------------

/// RBAC privilege levels. Ordinal determines privilege: higher = more access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Read-only: can list tools, search, and view tool info. Cannot execute.
    ReadOnly = 0,
    /// Developer: can execute most tools (default level).
    Developer = 1,
    /// Operator: can execute tools that manage infrastructure (restart backends, etc.).
    Operator = 2,
    /// Admin: unrestricted access. Can register/deregister backends.
    Admin = 3,
}

impl Default for Role {
    fn default() -> Self {
        Role::Admin // Backward compatible: existing deployments get full access.
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::ReadOnly => write!(f, "read_only"),
            Role::Developer => write!(f, "developer"),
            Role::Operator => write!(f, "operator"),
            Role::Admin => write!(f, "admin"),
        }
    }
}

impl std::str::FromStr for Role {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "admin" => Ok(Role::Admin),
            "operator" => Ok(Role::Operator),
            "developer" => Ok(Role::Developer),
            "read_only" | "readonly" => Ok(Role::ReadOnly),
            other => Err(format!(
                "unknown role '{}'. Valid: admin, operator, developer, read_only",
                other
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// ACL configuration (lives in config.yaml)
// ---------------------------------------------------------------------------

/// Per-tool ACL overrides inside a backend's `acl` block.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ToolAclConfig {
    /// Minimum role to call this specific tool. Overrides the backend-level default.
    #[serde(default)]
    pub min_role: Option<Role>,
}

/// ACL configuration block for a single backend.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BackendAclConfig {
    /// Minimum role required to call ANY tool on this backend.
    /// Default: Developer (if RBAC is enabled globally).
    #[serde(default)]
    pub min_role: Option<Role>,

    /// Per-tool overrides. Key = tool name (without namespace prefix).
    #[serde(default)]
    pub tools: HashMap<String, ToolAclConfig>,
}

/// Global RBAC configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RbacConfig {
    /// Enable RBAC enforcement. Default: false (backward compatible).
    #[serde(default)]
    pub enabled: bool,

    /// Default role for sessions that don't specify one.
    /// Default: Admin (backward compatible — existing sessions are unrestricted).
    #[serde(default)]
    pub default_role: Role,

    /// Per-backend ACL overrides. Key = backend name (as in config.backends).
    #[serde(default)]
    pub backends: HashMap<String, BackendAclConfig>,
}

impl Default for RbacConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_role: Role::Admin,
            backends: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// RBAC engine
// ---------------------------------------------------------------------------

/// Resolves the minimum required role for a tool call.
///
/// Priority:
///   1. Per-tool override in `rbac.backends.<name>.tools.<tool>` 
///   2. Per-backend default in `rbac.backends.<name>.min_role`
///   3. Global default: Developer (if RBAC enabled)
///
/// If RBAC is disabled, always returns Ok(()) — no enforcement.
pub struct RbacEngine {
    config: RbacConfig,
    /// Backend-level default role from config.backends.<name>.rbac_level
    backend_defaults: HashMap<String, Role>,
}

impl RbacEngine {
    /// Build a new engine from the global RBAC config and backend-level defaults.
    ///
    /// `backend_defaults` maps backend name -> min_role from BackendConfig.rbac_level.
    pub fn new(config: RbacConfig, backend_defaults: HashMap<String, Role>) -> Self {
        Self {
            config,
            backend_defaults,
        }
    }

    /// Is RBAC enforcement enabled?
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Get the default session role for new connections.
    pub fn default_role(&self) -> Role {
        self.config.default_role
    }

    /// Resolve the minimum required role for a given backend + tool combination.
    pub fn required_role(&self, backend_name: &str, tool_name: &str) -> Role {
        // Check per-tool override first
        if let Some(be_acl) = self.config.backends.get(backend_name) {
            if let Some(tool_acl) = be_acl.tools.get(tool_name) {
                if let Some(role) = &tool_acl.min_role {
                    debug!(
                        backend = backend_name,
                        tool = tool_name,
                        role = %role,
                        "RBAC: per-tool override"
                    );
                    return *role;
                }
            }

            // Then per-backend ACL default
            if let Some(role) = &be_acl.min_role {
                debug!(
                    backend = backend_name,
                    role = %role,
                    "RBAC: per-backend ACL default"
                );
                return *role;
            }
        }

        // Then BackendConfig.rbac_level
        if let Some(role) = self.backend_defaults.get(backend_name) {
            debug!(
                backend = backend_name,
                role = %role,
                "RBAC: backend config default"
            );
            return *role;
        }

        // Global fallback: Developer
        Role::Developer
    }

    /// Check whether a session role is allowed to call a tool on a backend.
    ///
    /// Returns Ok(()) if allowed, Err with denial reason if not.
    pub fn check_permission(
        &self,
        session_role: Role,
        backend_name: &str,
        tool_name: &str,
    ) -> Result<(), RbacDenial> {
        if !self.config.enabled {
            return Ok(());
        }

        let required = self.required_role(backend_name, tool_name);

        if session_role >= required {
            debug!(
                session_role = %session_role,
                required_role = %required,
                backend = backend_name,
                tool = tool_name,
                "RBAC: access granted"
            );
            Ok(())
        } else {
            warn!(
                session_role = %session_role,
                required_role = %required,
                backend = backend_name,
                tool = tool_name,
                "RBAC: access denied"
            );
            Err(RbacDenial {
                session_role,
                required_role: required,
                backend: backend_name.to_string(),
                tool: tool_name.to_string(),
            })
        }
    }

    /// Check whether a session role is allowed to use the register_manual / deregister_manual tools.
    /// These are always gated to Admin.
    pub fn check_admin_action(&self, session_role: Role) -> Result<(), RbacDenial> {
        if !self.config.enabled {
            return Ok(());
        }

        if session_role >= Role::Admin {
            Ok(())
        } else {
            Err(RbacDenial {
                session_role,
                required_role: Role::Admin,
                backend: "__meta__".to_string(),
                tool: "register_manual".to_string(),
            })
        }
    }
}

/// Returned when a tool call is denied by RBAC.
#[derive(Debug, Clone)]
pub struct RbacDenial {
    pub session_role: Role,
    pub required_role: Role,
    pub backend: String,
    pub tool: String,
}

impl std::fmt::Display for RbacDenial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RBAC denied: role '{}' requires '{}' for '{}.{}'",
            self.session_role, self.required_role, self.backend, self.tool
        )
    }
}

impl std::error::Error for RbacDenial {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn admin_config() -> RbacConfig {
        RbacConfig {
            enabled: true,
            default_role: Role::Admin,
            backends: HashMap::new(),
        }
    }

    #[test]
    fn role_ordering() {
        assert!(Role::Admin > Role::Operator);
        assert!(Role::Operator > Role::Developer);
        assert!(Role::Developer > Role::ReadOnly);
        assert!(Role::ReadOnly < Role::Admin);
    }

    #[test]
    fn role_from_str() {
        assert_eq!("admin".parse::<Role>(), Ok(Role::Admin));
        assert_eq!("operator".parse::<Role>(), Ok(Role::Operator));
        assert_eq!("developer".parse::<Role>(), Ok(Role::Developer));
        assert_eq!("read_only".parse::<Role>(), Ok(Role::ReadOnly));
        assert_eq!("readonly".parse::<Role>(), Ok(Role::ReadOnly));
        assert!("READONLY".parse::<Role>().is_ok());
        assert!("unknown".parse::<Role>().is_err());
    }

    #[test]
    fn disabled_rbac_allows_all() {
        let config = RbacConfig {
            enabled: false,
            ..Default::default()
        };
        let engine = RbacEngine::new(config, HashMap::new());

        // ReadOnly session should be able to do anything when RBAC is off
        assert!(engine
            .check_permission(Role::ReadOnly, "sensitive", "delete_all")
            .is_ok());
        assert!(engine.check_admin_action(Role::ReadOnly).is_ok());
    }

    #[test]
    fn default_fallback_is_developer() {
        let engine = RbacEngine::new(admin_config(), HashMap::new());
        assert_eq!(engine.required_role("any_backend", "any_tool"), Role::Developer);
    }

    #[test]
    fn backend_default_from_config() {
        let mut backend_defaults = HashMap::new();
        backend_defaults.insert("prod_db".to_string(), Role::Operator);

        let engine = RbacEngine::new(admin_config(), backend_defaults);
        assert_eq!(engine.required_role("prod_db", "query"), Role::Operator);
        assert_eq!(engine.required_role("other_backend", "query"), Role::Developer);
    }

    #[test]
    fn per_backend_acl_overrides_config_default() {
        let mut backends = HashMap::new();
        backends.insert(
            "prod_db".to_string(),
            BackendAclConfig {
                min_role: Some(Role::Admin), // Override to Admin
                tools: HashMap::new(),
            },
        );

        let config = RbacConfig {
            enabled: true,
            default_role: Role::Admin,
            backends,
        };

        let mut backend_defaults = HashMap::new();
        backend_defaults.insert("prod_db".to_string(), Role::Operator);

        let engine = RbacEngine::new(config, backend_defaults);
        // ACL says Admin, backend config says Operator — ACL wins
        assert_eq!(engine.required_role("prod_db", "query"), Role::Admin);
    }

    #[test]
    fn per_tool_acl_overrides_backend_default() {
        let mut tools = HashMap::new();
        tools.insert(
            "drop_table".to_string(),
            ToolAclConfig {
                min_role: Some(Role::Admin),
            },
        );

        let mut backends = HashMap::new();
        backends.insert(
            "prod_db".to_string(),
            BackendAclConfig {
                min_role: Some(Role::Developer),
                tools,
            },
        );

        let config = RbacConfig {
            enabled: true,
            default_role: Role::Admin,
            backends,
        };

        let engine = RbacEngine::new(config, HashMap::new());
        assert_eq!(engine.required_role("prod_db", "select"), Role::Developer);
        assert_eq!(engine.required_role("prod_db", "drop_table"), Role::Admin);
    }

    #[test]
    fn check_permission_grants_when_role_sufficient() {
        let engine = RbacEngine::new(admin_config(), HashMap::new());
        assert!(engine
            .check_permission(Role::Developer, "backend", "tool")
            .is_ok());
        assert!(engine
            .check_permission(Role::Admin, "backend", "tool")
            .is_ok());
    }

    #[test]
    fn check_permission_denies_when_role_insufficient() {
        let engine = RbacEngine::new(admin_config(), HashMap::new());
        let result = engine.check_permission(Role::ReadOnly, "backend", "tool");
        assert!(result.is_err());

        let denial = result.unwrap_err();
        assert_eq!(denial.session_role, Role::ReadOnly);
        assert_eq!(denial.required_role, Role::Developer); // default fallback
    }

    #[test]
    fn escalation_attempt_blocked() {
        // ReadOnly tries to call an Operator-level tool
        let mut backend_defaults = HashMap::new();
        backend_defaults.insert("infra".to_string(), Role::Operator);

        let engine = RbacEngine::new(admin_config(), backend_defaults);
        let result = engine.check_permission(Role::Developer, "infra", "restart");
        assert!(result.is_err());

        let denial = result.unwrap_err();
        assert_eq!(denial.session_role, Role::Developer);
        assert_eq!(denial.required_role, Role::Operator);
    }

    #[test]
    fn admin_action_requires_admin() {
        let engine = RbacEngine::new(admin_config(), HashMap::new());

        assert!(engine.check_admin_action(Role::Admin).is_ok());
        assert!(engine.check_admin_action(Role::Operator).is_err());
        assert!(engine.check_admin_action(Role::Developer).is_err());
        assert!(engine.check_admin_action(Role::ReadOnly).is_err());
    }

    #[test]
    fn denial_display_format() {
        let denial = RbacDenial {
            session_role: Role::ReadOnly,
            required_role: Role::Admin,
            backend: "prod_db".to_string(),
            tool: "drop_table".to_string(),
        };
        assert_eq!(
            denial.to_string(),
            "RBAC denied: role 'read_only' requires 'admin' for 'prod_db.drop_table'"
        );
    }

    #[test]
    fn serde_roundtrip_role() {
        let yaml = "admin";
        let role: Role = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(role, Role::Admin);

        let yaml = "developer";
        let role: Role = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(role, Role::Developer);
    }

    #[test]
    fn serde_roundtrip_rbac_config() {
        let config = RbacConfig {
            enabled: true,
            default_role: Role::Operator,
            backends: {
                let mut m = HashMap::new();
                m.insert(
                    "prod".to_string(),
                    BackendAclConfig {
                        min_role: Some(Role::Admin),
                        tools: {
                            let mut t = HashMap::new();
                            t.insert(
                                "nuclear_launch".to_string(),
                                ToolAclConfig {
                                    min_role: Some(Role::Admin),
                                },
                            );
                            t
                        },
                    },
                );
                m
            },
        };

        let yaml = serde_yaml::to_string(&config).unwrap();
        let deserialized: RbacConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(config, deserialized);
    }

    #[test]
    fn default_config_backward_compatible() {
        let config = RbacConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.default_role, Role::Admin);
        assert!(config.backends.is_empty());
    }
}
