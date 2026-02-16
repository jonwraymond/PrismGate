use anyhow::Result;
use serde_json::Value;
use std::sync::{Arc, LazyLock};
use tracing::{info, warn};

use crate::backend::BackendManager;
use crate::config::{BackendConfig, Transport};
use crate::registry::ToolRegistry;

/// Regex for valid backend names: alphanumeric start, then alphanumeric/underscore/hyphen, max 64 chars.
static BACKEND_NAME_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9_-]{0,63}$").unwrap());

/// Validate a backend name against the safe character set.
fn validate_backend_name(name: &str) -> Result<()> {
    if !BACKEND_NAME_RE.is_match(name) {
        anyhow::bail!("invalid backend name '{name}': must match [a-zA-Z0-9][a-zA-Z0-9_-]{{0,63}}");
    }
    Ok(())
}

/// Handle the register_manual meta-tool.
///
/// Parses a call template JSON, creates a BackendConfig, spawns the backend,
/// discovers tools, and registers them.
pub async fn handle_register(
    manager: &Arc<BackendManager>,
    registry: &Arc<ToolRegistry>,
    call_template: Value,
    max_dynamic_backends: usize,
) -> Result<String> {
    // Parse the call template
    let obj = call_template
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("manual_call_template must be a JSON object"))?;

    // Extract and validate name
    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'name' in call template"))?
        .to_string();

    validate_backend_name(&name)?;

    // Enforce dynamic backend limit (re-registering an existing dynamic backend doesn't count as new)
    if !manager.is_dynamic(&name).await && manager.dynamic_count().await >= max_dynamic_backends {
        anyhow::bail!(
            "dynamic backend limit reached ({max_dynamic_backends}). \
             Remove a dynamic backend first or increase max_dynamic_backends in config."
        );
    }

    // Determine transport
    let transport = if obj.get("url").is_some() {
        Transport::StreamableHttp
    } else {
        Transport::Stdio
    };

    // Build BackendConfig from template
    let command = obj
        .get("command")
        .and_then(|v| v.as_str())
        .map(String::from);
    let args: Vec<String> = obj
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let url = obj.get("url").and_then(|v| v.as_str()).map(String::from);

    // Log what's being registered (security audit trail)
    match transport {
        Transport::Stdio => {
            warn!(
                backend = %name,
                command = ?command,
                args = ?args,
                "register_manual: spawning stdio backend from MCP client request"
            );
        }
        Transport::StreamableHttp => {
            warn!(
                backend = %name,
                url = ?url,
                "register_manual: connecting HTTP backend from MCP client request"
            );
        }
    }

    let config = BackendConfig {
        transport,
        namespace: None,
        command,
        args,
        env: obj
            .get("env")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default(),
        cwd: obj.get("cwd").and_then(|v| v.as_str()).map(String::from),
        url,
        headers: obj
            .get("headers")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default(),
        timeout: std::time::Duration::from_secs(30),
        max_concurrent_calls: None,
        semaphore_timeout: std::time::Duration::from_secs(60),
        required_keys: Vec::new(),
        retry: Default::default(),
        prerequisite: None,
        rate_limit: None,
    };

    let tool_count = manager.add_backend(&name, config, registry).await?;
    manager.mark_dynamic(&name).await;

    let msg = format!("Registered backend '{name}' with {tool_count} tools");
    info!("{msg}");
    Ok(msg)
}

/// Handle the deregister_manual meta-tool.
///
/// Only dynamically registered backends can be deregistered. Statically
/// configured backends (from the config file) are protected.
pub async fn handle_deregister(
    manager: &Arc<BackendManager>,
    registry: &Arc<ToolRegistry>,
    name: &str,
) -> Result<String> {
    if !manager.is_dynamic(name).await {
        // Distinguish "not found" from "static/protected"
        let configured = manager.get_configured_names().await;
        if configured.contains(&name.to_string()) {
            anyhow::bail!(
                "backend '{name}' is a static (config-file) backend and cannot be deregistered. \
                 Only dynamically registered backends can be removed at runtime."
            );
        } else {
            anyhow::bail!("backend '{name}' not found.");
        }
    }

    warn!(backend = %name, "deregister_manual: removing dynamic backend");
    manager.remove_backend(name, registry).await?;
    manager.unmark_dynamic(name).await;
    Ok(format!("Deregistered backend '{name}'"))
}
