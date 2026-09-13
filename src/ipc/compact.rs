//! Print the compaction card for this session via the running daemon; never auto-starts it.
#[cfg(unix)]
pub async fn run(format: crate::cli::CompactFormat) -> anyhow::Result<()> {
    use anyhow::Context;
    use rmcp::{ServiceExt, model::CallToolRequestParams};

    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        let stream = tokio::net::UnixStream::connect(super::socket::default_socket_path())
            .await
            .context("cannot connect to daemon; compact never starts a daemon")?;
        let client = ().serve(tokio::io::split(stream)).await?;
        let result = client
            .call_tool(
                CallToolRequestParams::new("session_search").with_arguments(
                    serde_json::json!({"card": true})
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
            "daemon refused compact card: {:?}",
            result.content
        );
        let text = result
            .content
            .first()
            .and_then(|c| match &c.raw {
                rmcp::model::RawContent::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .context("compact card response lacks text")?;
        if format == crate::cli::CompactFormat::Json {
            println!("{text}");
        } else {
            let card: serde_json::Value = serde_json::from_str(&text)?;
            println!("Session: {}", card["session_key"]);
            println!("Events:  {}", card["event_count"]);
            if let Some(handles) = card["open_handles"].as_array()
                && !handles.is_empty()
            {
                println!("Open handles:");
                for h in handles {
                    println!("  - {h}");
                }
            }
            if let Some(tools) = card["recent_tools"].as_array()
                && !tools.is_empty()
            {
                println!("Recent tools:");
                for t in tools {
                    println!("  - {t}");
                }
            }
            if let Some(decisions) = card["decisions"].as_array()
                && !decisions.is_empty()
            {
                println!("Decisions:");
                for d in decisions {
                    println!("  - {d}");
                }
            }
            if let Some(constraints) = card["constraints"].as_array()
                && !constraints.is_empty()
            {
                println!("Constraints:");
                for c in constraints {
                    println!("  - {c}");
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("compact timed out; outcome unknown")?
}

#[cfg(not(unix))]
pub async fn run(_format: crate::cli::CompactFormat) -> anyhow::Result<()> {
    anyhow::bail!(
        "daemon compact requires Unix; use the session_search(card=true) MCP tool in direct mode"
    )
}
