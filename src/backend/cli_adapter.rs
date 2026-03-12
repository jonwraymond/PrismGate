//! CLI adapter backend — wraps arbitrary CLI tools as MCP tool providers.
//!
//! Each tool is defined via a command template with `{{param}}` placeholders.
//! Commands are executed via `sh -c` (Unix) or `cmd /C` (Windows), with
//! optional stdin piping and configurable output parsing (json/text/lines).

use std::collections::HashMap;
use std::sync::atomic::AtomicU8;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::{debug, error, info};

use super::{Backend, BackendState, STATE_HEALTHY, STATE_STARTING, STATE_STOPPED, STATE_UNHEALTHY};
use super::{is_available_from_atomic, state_from_atomic, store_state};
use crate::config::{BackendConfig, CliOutputFormat, CliToolConfig};
use crate::registry::ToolEntry;

/// Default max concurrent calls for CLI adapter backends.
pub(crate) const DEFAULT_CLI_ADAPTER_MAX_CONCURRENT: u32 = 5;

/// A backend that wraps CLI tools via shell command templates.
#[derive(Debug)]
pub struct CliAdapterBackend {
    name: String,
    tools: HashMap<String, CliToolConfig>,
    env: HashMap<String, String>,
    cwd: Option<String>,
    timeout: Duration,
    health_check: Option<String>,
    state: AtomicU8,
}

/// Adapter file format — just health_check + tools.
#[derive(serde::Deserialize)]
struct AdapterFile {
    #[serde(default)]
    health_check: Option<String>,
    #[serde(default)]
    tools: HashMap<String, CliToolConfig>,
}

impl CliAdapterBackend {
    /// Create a new CLI adapter backend from config.
    ///
    /// Tools are loaded from inline `tools` config, or from an external
    /// `adapter_file` (with `~` expansion). The adapter file can also
    /// supply a `health_check` command.
    pub fn new(name: String, config: BackendConfig) -> Result<Self> {
        let mut tools = config.tools.clone().unwrap_or_default();
        let mut health_check = config.health_check.clone();

        // Load from adapter file if specified
        if let Some(ref adapter_path) = config.adapter_file {
            let expanded = shellexpand::tilde(adapter_path);
            let path = std::path::Path::new(expanded.as_ref());
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read adapter file '{}'", expanded))?;
            let adapter: AdapterFile = serde_yaml_ng::from_str(&content)
                .with_context(|| format!("failed to parse adapter file '{}'", expanded))?;

            // Merge: adapter file tools are added, inline tools override on conflict
            for (tool_name, tool_config) in adapter.tools {
                tools.entry(tool_name).or_insert(tool_config);
            }

            // Adapter file health_check used if not set inline
            if health_check.is_none() {
                health_check = adapter.health_check;
            }
        }

        if tools.is_empty() {
            anyhow::bail!(
                "cli-adapter backend '{}' has no tools defined (inline or via adapter_file)",
                name
            );
        }

        Ok(Self {
            name,
            tools,
            env: config.env.clone(),
            cwd: config.cwd.clone(),
            timeout: config.timeout,
            health_check,
            state: AtomicU8::new(STATE_STARTING),
        })
    }

    /// Build a Command with the backend's env and cwd.
    fn build_shell_command(&self, rendered_command: &str) -> Command {
        let mut cmd = if cfg!(target_os = "windows") {
            let mut c = Command::new("cmd");
            c.args(["/C", rendered_command]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", rendered_command]);
            c
        };

        for (key, value) in &self.env {
            cmd.env(key, value);
        }

        if let Some(ref cwd) = self.cwd {
            cmd.current_dir(cwd);
        }

        cmd
    }

    /// Run the health check command and return success/failure.
    async fn run_health_check(&self) -> Result<()> {
        let Some(ref check_cmd) = self.health_check else {
            return Ok(());
        };

        let mut cmd = self.build_shell_command(check_cmd);
        let output = tokio::time::timeout(self.timeout, cmd.output())
            .await
            .map_err(|_| anyhow::anyhow!("health check timed out for '{}'", self.name))?
            .with_context(|| format!("failed to execute health check for '{}'", self.name))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "health check failed for '{}' (exit {}): {}",
                self.name,
                output.status.code().unwrap_or(-1),
                stderr.trim()
            );
        }

        Ok(())
    }
}

/// Render a template string by substituting `{{key}}` placeholders with argument values.
///
/// - `Value::String` → raw string (no quotes)
/// - Other types → JSON serialization
/// - Missing keys → placeholder left as-is
pub fn render_template(template: &str, args: &Value) -> String {
    let Some(obj) = args.as_object() else {
        return template.to_string();
    };

    let mut result = template.to_string();
    for (key, value) in obj {
        let placeholder = format!("{{{{{}}}}}", key);
        let replacement = match value {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        };
        result = result.replace(&placeholder, &replacement);
    }
    result
}

/// Parse command output according to the configured format.
pub fn parse_output(stdout: &str, format: &CliOutputFormat) -> Value {
    match format {
        CliOutputFormat::Json => {
            serde_json::from_str(stdout).unwrap_or_else(|_| Value::String(stdout.to_string()))
        }
        CliOutputFormat::Text => Value::String(stdout.to_string()),
        CliOutputFormat::Lines => {
            let lines: Vec<Value> = stdout
                .lines()
                .map(|l| Value::String(l.to_string()))
                .collect();
            Value::Array(lines)
        }
    }
}

#[async_trait]
impl Backend for CliAdapterBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&self) -> Result<()> {
        self.state
            .store(STATE_STARTING, std::sync::atomic::Ordering::Release);

        // Run health check if configured
        if let Err(e) = self.run_health_check().await {
            error!(backend = %self.name, error = %e, "health check failed on start");
            self.state
                .store(STATE_UNHEALTHY, std::sync::atomic::Ordering::Release);
            return Err(e);
        }

        self.state
            .store(STATE_HEALTHY, std::sync::atomic::Ordering::Release);
        info!(
            backend = %self.name,
            tools = self.tools.len(),
            "cli-adapter backend started"
        );
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.state
            .store(STATE_STOPPED, std::sync::atomic::Ordering::Release);
        info!(backend = %self.name, "cli-adapter backend stopped");
        Ok(())
    }

    async fn call_tool(&self, tool_name: &str, arguments: Option<Value>) -> Result<Value> {
        let tool_config = self.tools.get(tool_name).ok_or_else(|| {
            anyhow::anyhow!(
                "tool '{}' not found in cli-adapter backend '{}'",
                tool_name,
                self.name
            )
        })?;

        let args = arguments.unwrap_or(Value::Object(Default::default()));

        // Render command template
        let rendered_cmd = render_template(&tool_config.command, &args);
        debug!(backend = %self.name, tool = %tool_name, command = %rendered_cmd, "executing cli tool");

        // Build and configure command
        let mut cmd = self.build_shell_command(&rendered_cmd);
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // Set up stdin if template is configured
        let has_stdin = tool_config.stdin.is_some();
        if has_stdin {
            cmd.stdin(std::process::Stdio::piped());
        } else {
            cmd.stdin(std::process::Stdio::null());
        }

        // Spawn process
        let mut child = cmd.spawn().with_context(|| {
            format!(
                "failed to spawn command for tool '{}' on backend '{}'",
                tool_name, self.name
            )
        })?;

        // Write stdin if configured
        if let Some(ref stdin_template) = tool_config.stdin {
            let rendered_stdin = render_template(stdin_template, &args);
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(rendered_stdin.as_bytes()).await?;
                stdin.shutdown().await?;
            }
        }

        // Wait for output with timeout — kill child on timeout to prevent zombies.
        // We use wait_with_output which takes ownership, so we grab the PID first
        // for kill-on-timeout. On timeout the future is dropped but the child
        // process may survive; we send SIGKILL via the saved PID.
        let child_pid = child.id();
        let output = match tokio::time::timeout(self.timeout, child.wait_with_output()).await {
            Ok(result) => result.with_context(|| {
                format!(
                    "failed to wait for tool '{}' on backend '{}'",
                    tool_name, self.name
                )
            })?,
            Err(_) => {
                // Kill the child process to prevent orphans
                if let Some(pid) = child_pid {
                    #[cfg(unix)]
                    unsafe {
                        libc::kill(pid as i32, libc::SIGKILL);
                    }
                }
                anyhow::bail!(
                    "tool '{}' on backend '{}' timed out after {:?}",
                    tool_name,
                    self.name,
                    self.timeout
                );
            }
        };

        // Check exit code
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            anyhow::bail!(
                "tool '{}' on backend '{}' failed (exit {})\nstderr: {}\nstdout: {}",
                tool_name,
                self.name,
                output.status.code().unwrap_or(-1),
                stderr.trim(),
                stdout.trim()
            );
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.is_empty() {
            debug!(backend = %self.name, tool = %tool_name, stderr = %stderr.trim(), "cli tool stderr");
        }

        Ok(parse_output(&stdout, &tool_config.output))
    }

    async fn discover_tools(&self) -> Result<Vec<ToolEntry>> {
        let entries = self
            .tools
            .iter()
            .map(|(name, config)| ToolEntry {
                name: name.clone(),
                original_name: name.clone(),
                description: config.description.clone(),
                backend_name: self.name.clone(),
                input_schema: config.input_schema.clone(),
                tags: vec!["cli-adapter".to_string()],
            })
            .collect::<Vec<_>>();

        info!(backend = %self.name, tools = entries.len(), "discovered cli-adapter tools");
        Ok(entries)
    }

    fn is_available(&self) -> bool {
        is_available_from_atomic(&self.state)
    }

    fn state(&self) -> BackendState {
        state_from_atomic(&self.state)
    }

    fn set_state(&self, state: BackendState) {
        store_state(&self.state, state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_template_simple() {
        let args = serde_json::json!({"name": "world", "count": 5});
        assert_eq!(
            render_template("echo '{{name}}' {{count}}", &args),
            "echo 'world' 5"
        );
    }

    #[test]
    fn test_render_template_missing_param() {
        let args = serde_json::json!({"name": "world"});
        assert_eq!(
            render_template("echo '{{name}}' {{missing}}", &args),
            "echo 'world' {{missing}}"
        );
    }

    #[test]
    fn test_render_template_json_value() {
        let args = serde_json::json!({"data": {"nested": true}});
        assert_eq!(
            render_template("echo '{{data}}'", &args),
            r#"echo '{"nested":true}'"#
        );
    }

    #[test]
    fn test_render_template_null_value() {
        let args = serde_json::json!({"name": null});
        assert_eq!(render_template("echo '{{name}}'", &args), "echo ''");
    }

    #[test]
    fn test_render_template_boolean_value() {
        let args = serde_json::json!({"flag": true});
        assert_eq!(render_template("echo {{flag}}", &args), "echo true");
    }

    #[test]
    fn test_render_template_no_args() {
        let args = Value::Null;
        assert_eq!(render_template("echo hello", &args), "echo hello");
    }

    #[test]
    fn test_parse_output_json() {
        let result = parse_output(r#"{"key": "value"}"#, &CliOutputFormat::Json);
        assert!(result.is_object());
        assert_eq!(result["key"], "value");
    }

    #[test]
    fn test_parse_output_json_array() {
        let result = parse_output(r#"[1, 2, 3]"#, &CliOutputFormat::Json);
        assert!(result.is_array());
        assert_eq!(result.as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_parse_output_json_invalid() {
        let result = parse_output("not json", &CliOutputFormat::Json);
        assert!(result.is_string());
        assert_eq!(result.as_str().unwrap(), "not json");
    }

    #[test]
    fn test_parse_output_text() {
        let result = parse_output("hello world\n", &CliOutputFormat::Text);
        assert_eq!(result.as_str().unwrap(), "hello world\n");
    }

    #[test]
    fn test_parse_output_lines() {
        let result = parse_output("line1\nline2\nline3", &CliOutputFormat::Lines);
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0], "line1");
        assert_eq!(arr[1], "line2");
        assert_eq!(arr[2], "line3");
    }

    #[test]
    fn test_parse_output_lines_empty() {
        let result = parse_output("", &CliOutputFormat::Lines);
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 0); // empty string produces no lines
    }

    #[tokio::test]
    async fn test_call_tool_echo() {
        let mut tools = HashMap::new();
        tools.insert(
            "greet".to_string(),
            CliToolConfig {
                description: "Say hello".to_string(),
                input_schema: serde_json::json!({"type": "object", "properties": {"name": {"type": "string"}}}),
                command: "echo 'Hello, {{name}}!'".to_string(),
                stdin: None,
                output: CliOutputFormat::Text,
            },
        );

        let backend = CliAdapterBackend {
            name: "test".to_string(),
            tools,
            env: HashMap::new(),
            cwd: None,
            timeout: Duration::from_secs(10),
            health_check: None,
            state: AtomicU8::new(STATE_HEALTHY),
        };

        let result = backend
            .call_tool("greet", Some(serde_json::json!({"name": "World"})))
            .await
            .unwrap();

        assert_eq!(result.as_str().unwrap().trim(), "Hello, World!");
    }

    #[tokio::test]
    async fn test_call_tool_with_stdin() {
        let mut tools = HashMap::new();
        tools.insert(
            "upper".to_string(),
            CliToolConfig {
                description: "Convert to uppercase".to_string(),
                input_schema: serde_json::json!({"type": "object", "properties": {"text": {"type": "string"}}}),
                command: "tr '[:lower:]' '[:upper:]'".to_string(),
                stdin: Some("{{text}}".to_string()),
                output: CliOutputFormat::Text,
            },
        );

        let backend = CliAdapterBackend {
            name: "test".to_string(),
            tools,
            env: HashMap::new(),
            cwd: None,
            timeout: Duration::from_secs(10),
            health_check: None,
            state: AtomicU8::new(STATE_HEALTHY),
        };

        let result = backend
            .call_tool("upper", Some(serde_json::json!({"text": "hello world"})))
            .await
            .unwrap();

        assert_eq!(result.as_str().unwrap().trim(), "HELLO WORLD");
    }

    #[tokio::test]
    async fn test_call_tool_json_output() {
        let mut tools = HashMap::new();
        tools.insert(
            "json_echo".to_string(),
            CliToolConfig {
                description: "Echo JSON".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
                command: r#"echo '{"status": "ok"}'"#.to_string(),
                stdin: None,
                output: CliOutputFormat::Json,
            },
        );

        let backend = CliAdapterBackend {
            name: "test".to_string(),
            tools,
            env: HashMap::new(),
            cwd: None,
            timeout: Duration::from_secs(10),
            health_check: None,
            state: AtomicU8::new(STATE_HEALTHY),
        };

        let result = backend.call_tool("json_echo", None).await.unwrap();

        assert!(result.is_object());
        assert_eq!(result["status"], "ok");
    }

    #[tokio::test]
    async fn test_call_tool_lines_output() {
        let mut tools = HashMap::new();
        tools.insert(
            "list".to_string(),
            CliToolConfig {
                description: "List items".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
                command: "printf 'a\\nb\\nc'".to_string(),
                stdin: None,
                output: CliOutputFormat::Lines,
            },
        );

        let backend = CliAdapterBackend {
            name: "test".to_string(),
            tools,
            env: HashMap::new(),
            cwd: None,
            timeout: Duration::from_secs(10),
            health_check: None,
            state: AtomicU8::new(STATE_HEALTHY),
        };

        let result = backend.call_tool("list", None).await.unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0], "a");
        assert_eq!(arr[1], "b");
        assert_eq!(arr[2], "c");
    }

    #[tokio::test]
    async fn test_call_tool_nonzero_exit() {
        let mut tools = HashMap::new();
        tools.insert(
            "fail".to_string(),
            CliToolConfig {
                description: "Always fails".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
                command: "exit 1".to_string(),
                stdin: None,
                output: CliOutputFormat::Text,
            },
        );

        let backend = CliAdapterBackend {
            name: "test".to_string(),
            tools,
            env: HashMap::new(),
            cwd: None,
            timeout: Duration::from_secs(10),
            health_check: None,
            state: AtomicU8::new(STATE_HEALTHY),
        };

        let result = backend.call_tool("fail", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exit 1"));
    }

    #[tokio::test]
    async fn test_call_tool_not_found() {
        let backend = CliAdapterBackend {
            name: "test".to_string(),
            tools: HashMap::new(),
            env: HashMap::new(),
            cwd: None,
            timeout: Duration::from_secs(10),
            health_check: None,
            state: AtomicU8::new(STATE_HEALTHY),
        };

        let result = backend.call_tool("missing", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_health_check_success() {
        let backend = CliAdapterBackend {
            name: "test".to_string(),
            tools: {
                let mut t = HashMap::new();
                t.insert(
                    "dummy".to_string(),
                    CliToolConfig {
                        description: "dummy".to_string(),
                        input_schema: serde_json::json!({"type": "object"}),
                        command: "true".to_string(),
                        stdin: None,
                        output: CliOutputFormat::Text,
                    },
                );
                t
            },
            env: HashMap::new(),
            cwd: None,
            timeout: Duration::from_secs(10),
            health_check: Some("true".to_string()),
            state: AtomicU8::new(STATE_STARTING),
        };

        backend.start().await.unwrap();
        assert_eq!(backend.state(), BackendState::Healthy);
    }

    #[tokio::test]
    async fn test_health_check_failure() {
        let backend = CliAdapterBackend {
            name: "test".to_string(),
            tools: {
                let mut t = HashMap::new();
                t.insert(
                    "dummy".to_string(),
                    CliToolConfig {
                        description: "dummy".to_string(),
                        input_schema: serde_json::json!({"type": "object"}),
                        command: "true".to_string(),
                        stdin: None,
                        output: CliOutputFormat::Text,
                    },
                );
                t
            },
            env: HashMap::new(),
            cwd: None,
            timeout: Duration::from_secs(10),
            health_check: Some("false".to_string()),
            state: AtomicU8::new(STATE_STARTING),
        };

        let result = backend.start().await;
        assert!(result.is_err());
        assert_eq!(backend.state(), BackendState::Unhealthy);
    }

    #[tokio::test]
    async fn test_discover_tools() {
        let mut tools = HashMap::new();
        tools.insert(
            "tool_a".to_string(),
            CliToolConfig {
                description: "Tool A".to_string(),
                input_schema: serde_json::json!({"type": "object", "properties": {"x": {"type": "string"}}}),
                command: "echo a".to_string(),
                stdin: None,
                output: CliOutputFormat::Text,
            },
        );
        tools.insert(
            "tool_b".to_string(),
            CliToolConfig {
                description: "Tool B".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
                command: "echo b".to_string(),
                stdin: None,
                output: CliOutputFormat::Text,
            },
        );

        let backend = CliAdapterBackend {
            name: "my-cli".to_string(),
            tools,
            env: HashMap::new(),
            cwd: None,
            timeout: Duration::from_secs(10),
            health_check: None,
            state: AtomicU8::new(STATE_HEALTHY),
        };

        let entries = backend.discover_tools().await.unwrap();
        assert_eq!(entries.len(), 2);

        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"tool_a"));
        assert!(names.contains(&"tool_b"));

        for entry in &entries {
            assert_eq!(entry.backend_name, "my-cli");
            assert!(entry.tags.contains(&"cli-adapter".to_string()));
        }
    }

    #[tokio::test]
    async fn test_env_and_cwd() {
        let mut tools = HashMap::new();
        tools.insert(
            "env_test".to_string(),
            CliToolConfig {
                description: "Test env vars".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
                command: "echo $MY_VAR".to_string(),
                stdin: None,
                output: CliOutputFormat::Text,
            },
        );

        let mut env = HashMap::new();
        env.insert("MY_VAR".to_string(), "hello_from_env".to_string());

        let backend = CliAdapterBackend {
            name: "test".to_string(),
            tools,
            env,
            cwd: Some("/tmp".to_string()),
            timeout: Duration::from_secs(10),
            health_check: None,
            state: AtomicU8::new(STATE_HEALTHY),
        };

        let result = backend.call_tool("env_test", None).await.unwrap();
        assert_eq!(result.as_str().unwrap().trim(), "hello_from_env");
    }

    #[tokio::test]
    async fn test_stop() {
        let backend = CliAdapterBackend {
            name: "test".to_string(),
            tools: HashMap::new(),
            env: HashMap::new(),
            cwd: None,
            timeout: Duration::from_secs(10),
            health_check: None,
            state: AtomicU8::new(STATE_HEALTHY),
        };

        backend.stop().await.unwrap();
        assert_eq!(backend.state(), BackendState::Stopped);
    }

    #[test]
    fn test_new_no_tools_fails() {
        let config = BackendConfig {
            transport: crate::config::Transport::CliAdapter,
            namespace: None,
            command: None,
            args: Vec::new(),
            env: Default::default(),
            cwd: None,
            url: None,
            headers: Default::default(),
            timeout: Duration::from_secs(30),
            max_concurrent_calls: None,
            semaphore_timeout: Duration::from_secs(60),
            required_keys: Vec::new(),
            retry: Default::default(),
            prerequisite: None,
            rate_limit: None,
            tags: Vec::new(),
            fallback_chain: Vec::new(),
            tools: None,
            adapter_file: None,
            health_check: None,
            instance_mode: Default::default(),
            pool: Default::default(),
        };

        let result = CliAdapterBackend::new("empty".to_string(), config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no tools defined"));
    }
}
