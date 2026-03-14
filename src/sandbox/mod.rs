pub mod bridge;

#[cfg(feature = "sandbox")]
use std::sync::Arc;
#[cfg(feature = "sandbox")]
use std::time::Duration;

#[cfg(feature = "sandbox")]
use anyhow::Result;
#[cfg(feature = "sandbox")]
use serde_json::Value;
#[cfg(feature = "sandbox")]
use tracing::{debug, info, warn};

#[cfg(feature = "sandbox")]
use crate::backend::BackendManager;
#[cfg(feature = "sandbox")]
use crate::registry::ToolRegistry;

/// Execute TypeScript code in a sandboxed V8 runtime with access to
/// registered tools as async functions.
///
/// The sandbox runs on a dedicated OS thread (V8 isolates are !Send).
/// Tool calls are dispatched back to the main tokio runtime via Handle::spawn.
#[cfg(feature = "sandbox")]
pub async fn execute(
    registry: &Arc<ToolRegistry>,
    manager: &Arc<BackendManager>,
    code: &str,
    timeout: Duration,
    max_heap_size: Option<usize>,
    session_id: Option<u64>,
) -> Result<String> {
    let main_handle = tokio::runtime::Handle::current();
    let manager = Arc::clone(manager);
    let tools = registry.get_all();
    let code = code.to_string();
    let heap_size = max_heap_size.unwrap_or(50 * 1024 * 1024); // 50MB default

    debug!(
        code_len = code.len(),
        timeout_ms = timeout.as_millis(),
        tool_count = tools.len(),
        "executing code in sandbox"
    );

    let registry = Arc::clone(registry);
    let (tx, rx) = tokio::sync::oneshot::channel();

    std::thread::Builder::new()
        .name("gatemini-sandbox".to_string())
        .spawn(move || {
            let result = run_sandbox(
                main_handle,
                manager,
                registry,
                tools,
                &code,
                timeout,
                heap_size,
                session_id,
            );
            let _ = tx.send(result);
        })?;

    rx.await
        .map_err(|_| anyhow::anyhow!("sandbox thread panicked"))?
}

/// Run the sandboxed execution on a dedicated thread.
#[cfg(feature = "sandbox")]
#[allow(clippy::too_many_arguments)]
fn run_sandbox(
    main_handle: tokio::runtime::Handle,
    manager: Arc<BackendManager>,
    registry: Arc<ToolRegistry>,
    tools: Vec<crate::registry::ToolEntry>,
    code: &str,
    timeout: Duration,
    max_heap_size: usize,
    session_id: Option<u64>,
) -> Result<String> {
    use rustyscript::{Module, Runtime, RuntimeOptions};
    use std::pin::Pin;

    let mut runtime = Runtime::new(RuntimeOptions {
        timeout,
        max_heap_size: Some(max_heap_size),
        default_entrypoint: Some("main".to_string()),
        ..Default::default()
    })
    .map_err(|e| anyhow::anyhow!("failed to create sandbox runtime: {e}"))?;

    // Register __call_tool: dispatches tool calls to the main tokio runtime
    // where the rmcp backend services live.
    let mgr = manager;
    let handle = main_handle;
    let reg = registry;
    runtime
        .register_async_function(
            "__call_tool",
            move |args: Vec<Value>| -> Pin<Box<dyn std::future::Future<Output = std::result::Result<Value, rustyscript::Error>>>>
            {
                let mgr = mgr.clone();
                let handle = handle.clone();
                let reg = reg.clone();
                Box::pin(async move {
                    if args.len() < 2 {
                        return Err(rustyscript::Error::Runtime(
                            "__call_tool requires (backend_name, tool_name, [arguments])".to_string(),
                        ));
                    }

                    let backend_name = args[0]
                        .as_str()
                        .ok_or_else(|| {
                            rustyscript::Error::Runtime("backend_name must be a string".to_string())
                        })?
                        .to_string();

                    let tool_name = args[1]
                        .as_str()
                        .ok_or_else(|| {
                            rustyscript::Error::Runtime("tool_name must be a string".to_string())
                        })?
                        .to_string();

                    let arguments = if args.len() > 2 && !args[2].is_null() {
                        Some(args[2].clone())
                    } else {
                        None
                    };

                    // Dispatch to the main tokio runtime where rmcp services live
                    let mgr_for_restart = mgr.clone();
                    let args_for_retry = arguments.clone();
                    let bn = backend_name.clone();
                    let tn = tool_name.clone();
                    let sid = session_id;
                    let result = handle
                        .spawn(async move {
                            mgr.call_tool(&bn, &tn, arguments, sid).await
                        })
                        .await
                        .map_err(|e| {
                            rustyscript::Error::Runtime(format!("task join error: {e}"))
                        })?;

                    match result {
                        Ok(value) => Ok(value),
                        Err(e) => {
                            let err_str = e.to_string();

                            // On-demand restart for stopped backends
                            if err_str.contains("not available") && err_str.contains("Stopped") {
                                info!(backend = %backend_name, tool = %tool_name,
                                      "attempting on-demand restart for stopped backend");
                                let restart_reg = reg.clone();
                                let restart_bn = backend_name.clone();
                                let restart_mgr = mgr_for_restart.clone();
                                let restart_result = handle
                                    .spawn(async move {
                                        restart_mgr.restart_backend(&restart_bn, &restart_reg).await
                                    })
                                    .await
                                    .map_err(|e| rustyscript::Error::Runtime(format!("restart join: {e}")))
                                    .and_then(|r| r.map_err(|e| rustyscript::Error::Runtime(format!("restart failed: {e}"))));

                                if let Ok(tool_count) = restart_result {
                                    info!(backend = %backend_name, tools = tool_count, "on-demand restart succeeded, retrying call");
                                    // Retry the tool call once
                                    let retry_mgr = mgr_for_restart;
                                    let retry_bn = backend_name.clone();
                                    let retry_tn = tool_name.clone();
                                    let retry_sid = session_id;
                                    let retry_result = handle
                                        .spawn(async move {
                                            retry_mgr.call_tool(&retry_bn, &retry_tn, args_for_retry, retry_sid).await
                                        })
                                        .await
                                        .map_err(|e| rustyscript::Error::Runtime(format!("retry join: {e}")))?;
                                    return match retry_result {
                                        Ok(value) => Ok(value),
                                        Err(e) => Err(rustyscript::Error::Runtime(e.to_string())),
                                    };
                                } else {
                                    warn!(backend = %backend_name, "on-demand restart failed, returning enhanced error");
                                    // Intentionally falls through to the "not available" error enhancement below
                                }
                            }

                            // Enhance error if tool is cached but backend isn't ready
                            if (err_str.contains("not found") || err_str.contains("still starting"))
                                && reg.get_by_name(&tool_name).is_some()
                            {
                                return Err(rustyscript::Error::Runtime(format!(
                                    "Backend '{}' is still starting. Tool '{}' is cached \
                                     but the backend hasn't connected yet. Try again shortly.",
                                    backend_name, tool_name
                                )));
                            }

                            // Backend stopped or transport closed
                            if err_str.contains("not available") || err_str.contains("Transport closed") {
                                return Err(rustyscript::Error::Runtime(format!(
                                    "Backend '{}' is not available for tool '{}'. \
                                     The backend may have stopped or lost connection.\n\
                                     To check status: load @gatemini://backend/{}\n\
                                     To see all backends: load @gatemini://backends",
                                    backend_name, tool_name, backend_name
                                )));
                            }

                            Err(rustyscript::Error::Runtime(err_str))
                        }
                    }
                })
            },
        )
        .map_err(|e| anyhow::anyhow!("failed to register __call_tool: {e}"))?;

    // Generate preamble with tool accessor objects
    let preamble = bridge::generate_preamble(&tools);

    // Collect backend names (both original and sanitized) for error hints.
    let backend_names: std::collections::HashSet<String> = tools
        .iter()
        .flat_map(|t| {
            let sanitized = bridge::sanitize_identifier(&t.backend_name);
            if sanitized == t.backend_name {
                vec![sanitized]
            } else {
                vec![sanitized, t.backend_name.clone()]
            }
        })
        .collect();

    // Wrap user code in an async main function if not already exported
    let full_code =
        if code.contains("export default") || code.contains("export async function main") {
            format!("{preamble}\n{code}")
        } else {
            format!("{preamble}\nexport default async function main() {{\n{code}\n}}",)
        };

    let module = Module::new("sandbox.ts", &full_code);
    let module_handle = runtime
        .load_module(&module)
        .map_err(|e| enhance_sandbox_error(e, &backend_names))?;

    let result: Value = runtime
        .call_entrypoint(&module_handle, rustyscript::json_args!())
        .map_err(|e| enhance_sandbox_error(e, &backend_names))?;

    debug!("sandbox execution complete");

    serde_json::to_string_pretty(&result).map_err(|e| anyhow::anyhow!("result serialization: {e}"))
}

/// Enhance sandbox errors with actionable hints for common LLM mistakes.
#[cfg(feature = "sandbox")]
fn enhance_sandbox_error(
    err: rustyscript::Error,
    backend_names: &std::collections::HashSet<String>,
) -> anyhow::Error {
    let msg = err.to_string();

    // Pattern: `const auggie = await auggie.codebase_retrieval(...)` shadows the backend const.
    // V8 error: "ReferenceError: Cannot access 'X' before initialization"
    if msg.contains("Cannot access '")
        && msg.contains("before initialization")
        && let Some(start) = msg.find("Cannot access '")
    {
        let rest = &msg[start + 15..];
        if let Some(end) = rest.find('\'') {
            let var_name = &rest[..end];
            if backend_names.contains(var_name) {
                return anyhow::anyhow!(
                    "sandbox execution error: {msg}\n\n\
                     HINT: `{var_name}` is a backend name. Writing `const {var_name} = await {var_name}.tool(...)` \
                     shadows the backend variable before it's read. \
                     Use a different name: `const {var_name}Result = await {var_name}.tool(...)`"
                );
            }
        }
    }

    // Pattern: `X is not defined` — might be a misspelled backend name
    if msg.contains("is not defined")
        && let Some(start) = msg.find("ReferenceError: ")
    {
        let rest = &msg[start + 16..];
        if let Some(end) = rest.find(" is not defined") {
            let var_name = rest[..end].trim();
            // Check for close matches against backend names
            let suggestion = backend_names
                .iter()
                .find(|bn| {
                    // Simple typo detection: off by case or underscore/hyphen
                    bn.eq_ignore_ascii_case(var_name)
                        || bn.replace('_', "-") == var_name
                        || bn.replace('-', "_") == var_name
                })
                .cloned();
            if let Some(correct) = suggestion {
                return anyhow::anyhow!(
                    "sandbox execution error: {msg}\n\n\
                     HINT: Did you mean `{correct}`? Available backends with similar names: {correct}"
                );
            }
        }
    }

    anyhow::anyhow!("sandbox execution error: {msg}")
}
