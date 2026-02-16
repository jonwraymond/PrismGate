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
use tracing::debug;

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
            let result = run_sandbox(main_handle, manager, registry, tools, &code, timeout, heap_size);
            let _ = tx.send(result);
        })?;

    rx.await.map_err(|_| anyhow::anyhow!("sandbox thread panicked"))?
}

/// Run the sandboxed execution on a dedicated thread.
#[cfg(feature = "sandbox")]
fn run_sandbox(
    main_handle: tokio::runtime::Handle,
    manager: Arc<BackendManager>,
    registry: Arc<ToolRegistry>,
    tools: Vec<crate::registry::ToolEntry>,
    code: &str,
    timeout: Duration,
    max_heap_size: usize,
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
                    let bn = backend_name.clone();
                    let tn = tool_name.clone();
                    let result = handle
                        .spawn(async move {
                            mgr.call_tool(&bn, &tn, arguments).await
                        })
                        .await
                        .map_err(|e| {
                            rustyscript::Error::Runtime(format!("task join error: {e}"))
                        })?;

                    match result {
                        Ok(value) => Ok(value),
                        Err(e) => {
                            let err_str = e.to_string();
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
                            Err(rustyscript::Error::Runtime(err_str))
                        }
                    }
                })
            },
        )
        .map_err(|e| anyhow::anyhow!("failed to register __call_tool: {e}"))?;

    // Generate preamble with tool accessor objects
    let preamble = bridge::generate_preamble(&tools);

    // Wrap user code in an async main function if not already exported
    let full_code = if code.contains("export default") || code.contains("export async function main") {
        format!("{preamble}\n{code}")
    } else {
        format!(
            "{preamble}\nexport default async function main() {{\n{code}\n}}",
        )
    };

    let module = Module::new("sandbox.ts", &full_code);
    let module_handle = runtime
        .load_module(&module)
        .map_err(|e| anyhow::anyhow!("sandbox module load error: {e}"))?;

    let result: Value = runtime
        .call_entrypoint(&module_handle, rustyscript::json_args!())
        .map_err(|e| anyhow::anyhow!("sandbox execution error: {e}"))?;

    debug!("sandbox execution complete");

    serde_json::to_string_pretty(&result).map_err(|e| anyhow::anyhow!("result serialization: {e}"))
}
