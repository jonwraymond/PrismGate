//! Read-only profiling over the existing MCP daemon connection.
#[cfg(unix)]
pub async fn run(backend: Option<&str>, format: crate::cli::ProfileFormat) -> anyhow::Result<()> {
    use anyhow::Context;
    use rmcp::{ServiceExt, model::ReadResourceRequestParams};
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        let stream = tokio::net::UnixStream::connect(super::socket::default_socket_path())
            .await
            .context("cannot connect to daemon; profile never starts a daemon")?;
        let client = ().serve(tokio::io::split(stream)).await?;
        let response = client
            .read_resource(ReadResourceRequestParams::new("prismgate://profile"))
            .await;
        let _ = client.cancel().await;
        let response = serde_json::to_value(response?)?;
        let text = response["contents"][0]["text"]
            .as_str()
            .context("profile resource lacks text")?;
        let mut data: serde_json::Value = serde_json::from_str(text)?;
        let rows = data["backends"]
            .as_array_mut()
            .context("invalid profile backends")?;
        if let Some(name) = backend {
            rows.retain(|row| row["backend"].as_str() == Some(name));
            anyhow::ensure!(!rows.is_empty(), "no recorded calls for requested backend");
        }
        if format == crate::cli::ProfileFormat::Json {
            println!("{}", serde_json::to_string_pretty(&data)?);
        } else {
            println!("Completed calls since process startup/reset; latency in ms");
            println!("BACKEND\tCALLS\tERRORS\tP50\tP95\tP99");
            for row in rows {
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    row["backend"],
                    row["calls"],
                    row["errors"],
                    row["latency"]["p50_ms"],
                    row["latency"]["p95_ms"],
                    row["latency"]["p99_ms"]
                );
            }
        }
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("profile request timed out")?
}

#[cfg(not(unix))]
pub async fn run(_backend: Option<&str>, _format: crate::cli::ProfileFormat) -> anyhow::Result<()> {
    anyhow::bail!(
        "daemon profiling requires Unix; read prismgate://profile through MCP in direct mode"
    )
}
