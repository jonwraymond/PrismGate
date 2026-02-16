//! Virtual backend for composite tools defined in config as TypeScript snippets.
//!
//! Composite tools are multi-step orchestrations that call real backend tools.
//! They appear in the registry like any other tool but their `call_tool` is
//! handled by the V8 sandbox (or direct-call fast path).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use super::{Backend, BackendState, STATE_HEALTHY, STATE_STOPPED};
use crate::config::CompositeToolConfig;
use crate::registry::ToolEntry;

/// Virtual backend name used for all composite tools.
pub const COMPOSITE_BACKEND_NAME: &str = "__composite";

/// A virtual backend that holds composite tool definitions.
/// It never spawns a child process — tool execution is delegated to the
/// sandbox layer which calls real backends.
pub struct CompositeBackend {
    tools: HashMap<String, CompositeToolConfig>,
    state: AtomicU8,
}

impl CompositeBackend {
    pub fn new(tools: HashMap<String, CompositeToolConfig>) -> Self {
        Self {
            tools,
            state: AtomicU8::new(STATE_HEALTHY),
        }
    }
}

#[async_trait]
impl Backend for CompositeBackend {
    fn name(&self) -> &str {
        COMPOSITE_BACKEND_NAME
    }

    async fn start(&self) -> Result<()> {
        // No-op: virtual backend, always ready.
        self.state.store(STATE_HEALTHY, Ordering::Release);
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.state.store(STATE_STOPPED, Ordering::Release);
        Ok(())
    }

    async fn call_tool(&self, tool_name: &str, _arguments: Option<Value>) -> Result<Value> {
        // Composite tools are NOT executed here — they're executed via
        // call_tool_chain which has access to the sandbox and all backends.
        // If someone calls a composite tool directly, return the code so
        // the caller knows to use call_tool_chain instead.
        if self.tools.contains_key(tool_name) {
            anyhow::bail!(
                "Composite tool '{}' must be executed via call_tool_chain, not direct call. \
                 Use: call_tool_chain(\"const r = await __composite.{}({{...}}); return r;\")",
                tool_name,
                tool_name
            );
        }
        anyhow::bail!("composite tool '{}' not found", tool_name)
    }

    async fn discover_tools(&self) -> Result<Vec<ToolEntry>> {
        let default_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "params": {
                    "type": "object",
                    "description": "Parameters passed to the composite tool"
                }
            }
        });

        Ok(self
            .tools
            .iter()
            .map(|(name, config)| ToolEntry {
                name: name.clone(),
                original_name: name.clone(),
                description: config.description.clone(),
                backend_name: COMPOSITE_BACKEND_NAME.to_string(),
                input_schema: config
                    .input_schema
                    .clone()
                    .unwrap_or_else(|| default_schema.clone()),
                tags: vec!["composite".to_string()],
            })
            .collect())
    }

    fn is_available(&self) -> bool {
        self.state.load(Ordering::Acquire) == STATE_HEALTHY
    }

    fn state(&self) -> BackendState {
        super::state_from_atomic(&self.state)
    }

    fn set_state(&self, state: BackendState) {
        super::store_state(&self.state, state);
    }
}
