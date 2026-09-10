//! Explicit clean slate via the existing daemon MCP socket; never auto-starts it.
use std::path::PathBuf;

#[cfg(unix)]
pub async fn run(socket_path: Option<PathBuf>) -> anyhow::Result<()> {
    use anyhow::Context;
    use rmcp::{ServiceExt, model::CallToolRequestParams};

    let path = socket_path.unwrap_or_else(super::socket::default_socket_path);
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        let stream = tokio::net::UnixStream::connect(&path)
            .await
            .with_context(|| {
                format!(
                    "cannot connect to daemon at {}; nothing purged",
                    path.display()
                )
            })?;
        let (read, write) = tokio::io::split(stream);
        let client = ().serve((read, write)).await?;
        let result = client
            .call_tool(
                CallToolRequestParams::new("purge_session").with_arguments(
                    serde_json::json!({"confirm": true})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await;
        let _ = client.cancel().await;
        let result = result?;
        anyhow::ensure!(
            result.is_error != Some(true),
            "daemon refused purge: {:?}",
            result.content
        );
        println!("{}", serde_json::to_string(&result)?);
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("purge timed out; outcome unknown, inspect daemon statistics before retrying")?
}

#[cfg(not(unix))]
pub async fn run(_socket_path: Option<PathBuf>) -> anyhow::Result<()> {
    anyhow::bail!("daemon purge requires Unix; use the purge_session MCP tool in direct mode")
}
