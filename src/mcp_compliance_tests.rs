//! MCP protocol compliance tests.
//!
//! Tests gatemini as an MCP server (front-door) using an in-process rmcp client
//! connected via `tokio::io::duplex`. Validates protocol version, capabilities,
//! tool listing/calling, resources, prompts, and error handling.

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use rmcp::model::*;
    use rmcp::ServiceExt;
    use tokio::sync::Semaphore;

    use crate::backend::BackendManager;
    use crate::registry::ToolRegistry;
    use crate::server::GateminiServer;
    use crate::testutil::{MockBackend, insert_mock};

    /// Create a GateminiServer with mock backends, connect via duplex,
    /// return the rmcp client peer for protocol testing.
    async fn setup_mcp_client() -> (
        rmcp::service::Peer<rmcp::RoleClient>,
        Arc<MockBackend>,
        Arc<ToolRegistry>,
    ) {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("test-backend", Duration::ZERO);
        insert_mock(&manager, &registry, &mock).await;

        let server = GateminiServer::new(
            Arc::clone(&registry),
            manager,
            std::path::PathBuf::from("/tmp/test-cache.json"),
            true,
            10,
            Arc::new(Semaphore::new(8)),
        );

        let (client_io, server_io) = tokio::io::duplex(65536);
        let (server_read, server_write) = tokio::io::split(server_io);
        let (client_read, client_write) = tokio::io::split(client_io);

        // Spawn server side
        let _server_handle = tokio::spawn(async move {
            match server.serve((server_read, server_write)).await {
                Ok(service) => {
                    let _ = service.waiting().await;
                }
                Err(e) => {
                    eprintln!("test server error: {e}");
                }
            }
        });

        // Client side — handshake
        let client_service = ()
            .serve((client_read, client_write))
            .await
            .expect("client handshake failed");

        let peer = client_service.peer().clone();
        // Keep the service alive in background
        tokio::spawn(async move {
            let _ = client_service.waiting().await;
        });

        (peer, mock, registry)
    }

    // --- 4A: Front-door tests (gatemini as server) ---

    #[tokio::test]
    async fn test_initialize_handshake() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();

        let server = GateminiServer::new(
            registry,
            manager,
            std::path::PathBuf::from("/tmp/test-cache.json"),
            true,
            10,
            Arc::new(Semaphore::new(8)),
        );

        let (client_io, server_io) = tokio::io::duplex(65536);
        let (server_read, server_write) = tokio::io::split(server_io);
        let (client_read, client_write) = tokio::io::split(client_io);

        tokio::spawn(async move {
            match server.serve((server_read, server_write)).await {
                Ok(service) => {
                    let _ = service.waiting().await;
                }
                Err(e) => panic!("server failed: {e}"),
            }
        });

        let client_service = ()
            .serve((client_read, client_write))
            .await
            .expect("handshake failed");

        // Verify peer info
        let peer_info = client_service.peer_info().expect("no peer info");
        // rmcp negotiates to the highest mutually supported version
        assert!(
            peer_info.protocol_version >= ProtocolVersion::V_2025_03_26,
            "expected protocol version >= 2025-03-26, got {}",
            peer_info.protocol_version
        );

        // Verify capabilities
        let caps = &peer_info.capabilities;
        assert!(caps.tools.is_some(), "tools capability missing");
        assert!(caps.resources.is_some(), "resources capability missing");
        assert!(caps.prompts.is_some(), "prompts capability missing");
    }

    #[tokio::test]
    async fn test_tools_list_returns_7_meta_tools() {
        let (peer, _, _) = setup_mcp_client().await;
        let tools = peer.list_all_tools().await.unwrap();

        assert_eq!(tools.len(), 7, "expected 7 meta-tools, got {}", tools.len());

        let names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
        assert!(names.contains(&"search_tools".to_string()));
        assert!(names.contains(&"list_tools_meta".to_string()));
        assert!(names.contains(&"tool_info".to_string()));
        assert!(names.contains(&"call_tool_chain".to_string()));
        assert!(names.contains(&"register_manual".to_string()));
        assert!(names.contains(&"deregister_manual".to_string()));
        assert!(names.contains(&"get_required_keys_for_tool".to_string()));
    }

    #[tokio::test]
    async fn test_tools_list_schema_validity() {
        let (peer, _, _) = setup_mcp_client().await;
        let tools = peer.list_all_tools().await.unwrap();

        for tool in &tools {
            assert!(
                !tool.name.is_empty(),
                "tool name should not be empty"
            );
            assert!(
                tool.description.is_some(),
                "tool '{}' should have a description",
                tool.name
            );

            // Verify inputSchema is a valid JSON Schema object
            let schema = &tool.input_schema;
            let schema_val = serde_json::to_value(schema).unwrap();
            assert_eq!(
                schema_val.get("type").and_then(|v| v.as_str()),
                Some("object"),
                "tool '{}' inputSchema type should be 'object'",
                tool.name
            );
            assert!(
                schema_val.get("properties").is_some(),
                "tool '{}' inputSchema should have 'properties'",
                tool.name
            );
        }
    }

    #[tokio::test]
    async fn test_tools_call_search_success() {
        let (peer, _, _) = setup_mcp_client().await;

        let result = peer
            .call_tool(CallToolRequestParams {
                meta: None,
                name: "search_tools".to_string().into(),
                arguments: Some(serde_json::json!({"task_description": "echo"}).as_object().unwrap().clone()),
                task: None,
            })
            .await
            .unwrap();

        assert!(!result.content.is_empty(), "search should return content");
        assert!(
            !result.is_error.unwrap_or(false),
            "search should not be an error"
        );
    }

    #[tokio::test]
    async fn test_tools_call_tool_info_brief() {
        let (peer, _, _) = setup_mcp_client().await;

        let result = peer
            .call_tool(CallToolRequestParams {
                meta: None,
                name: "tool_info".to_string().into(),
                arguments: Some(
                    serde_json::json!({"tool_name": "echo_tool"})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
                task: None,
            })
            .await
            .unwrap();

        assert!(!result.is_error.unwrap_or(false));

        // Verify response contains tool info
        let text = extract_text(&result);
        assert!(
            text.contains("echo_tool"),
            "brief info should contain tool name"
        );
    }

    #[tokio::test]
    async fn test_tools_call_tool_info_full() {
        let (peer, _, _) = setup_mcp_client().await;

        let result = peer
            .call_tool(CallToolRequestParams {
                meta: None,
                name: "tool_info".to_string().into(),
                arguments: Some(
                    serde_json::json!({"tool_name": "echo_tool", "detail": "full"})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
                task: None,
            })
            .await
            .unwrap();

        assert!(!result.is_error.unwrap_or(false));

        let text = extract_text(&result);
        // Full info should contain the input schema
        assert!(
            text.contains("properties") || text.contains("input_schema"),
            "full info should contain schema details"
        );
    }

    #[tokio::test]
    async fn test_tools_call_error_invalid_params() {
        let (peer, _, _) = setup_mcp_client().await;

        // Call tool_info without required tool_name param
        let result = peer
            .call_tool(CallToolRequestParams {
                meta: None,
                name: "tool_info".to_string().into(),
                arguments: Some(serde_json::Map::new()),
                task: None,
            })
            .await;

        // Should either return an error result or a protocol error
        match result {
            Ok(r) => assert!(r.is_error.unwrap_or(false), "should be an error result"),
            Err(_) => {} // Protocol error is also acceptable
        }
    }

    #[tokio::test]
    async fn test_tools_call_nonexistent_tool() {
        let (peer, _, _) = setup_mcp_client().await;

        let result = peer
            .call_tool(CallToolRequestParams {
                meta: None,
                name: "tool_info".to_string().into(),
                arguments: Some(
                    serde_json::json!({"tool_name": "does_not_exist"})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
                task: None,
            })
            .await
            .unwrap();

        assert!(
            result.is_error.unwrap_or(false),
            "should return error for nonexistent tool"
        );
        let text = extract_text(&result);
        assert!(
            text.contains("not found"),
            "error should mention 'not found'"
        );
    }

    #[tokio::test]
    async fn test_resources_list() {
        let (peer, _, _) = setup_mcp_client().await;

        let result = peer.list_resources(None).await.unwrap();
        let names: Vec<String> = result
            .resources
            .iter()
            .map(|r| r.raw.name.clone())
            .collect();

        assert!(names.contains(&"overview".to_string()));
        assert!(names.contains(&"backends".to_string()));
        assert!(names.contains(&"tools".to_string()));
    }

    #[tokio::test]
    async fn test_resource_templates_list() {
        let (peer, _, _) = setup_mcp_client().await;

        let result = peer.list_resource_templates(None).await.unwrap();
        let uris: Vec<String> = result
            .resource_templates
            .iter()
            .map(|t| t.raw.uri_template.clone())
            .collect();

        assert!(
            uris.iter().any(|u| u.contains("tool/")),
            "should have tool/{{name}} template"
        );
        assert!(
            uris.iter().any(|u| u.contains("backend/")),
            "should have backend/{{name}} template"
        );
    }

    #[tokio::test]
    async fn test_resources_read_overview() {
        let (peer, _, _) = setup_mcp_client().await;

        let result = peer
            .read_resource(ReadResourceRequestParams {
                meta: None,
                uri: "gatemini://overview".to_string(),
            })
            .await
            .unwrap();

        assert!(!result.contents.is_empty());
        let text: String = result
            .contents
            .first()
            .and_then(|c| match c {
                ResourceContents::TextResourceContents { text, .. } => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_default();

        // Overview should mention tool and backend counts
        assert!(
            text.contains("tool") || text.contains("backend"),
            "overview should contain tool/backend info"
        );
    }

    #[tokio::test]
    async fn test_prompts_list() {
        let (peer, _, _) = setup_mcp_client().await;

        let result = peer.list_prompts(None).await.unwrap();
        let names: Vec<String> = result.prompts.iter().map(|p| p.name.clone()).collect();

        assert!(names.contains(&"discover".to_string()));
        assert!(names.contains(&"find_tool".to_string()));
        assert!(names.contains(&"backend_status".to_string()));
    }

    #[tokio::test]
    async fn test_prompts_get_with_args() {
        let (peer, _, _) = setup_mcp_client().await;

        let result = peer
            .get_prompt(GetPromptRequestParams {
                meta: None,
                name: "find_tool".to_string(),
                arguments: Some({
                    let mut args = serde_json::Map::new();
                    args.insert("task".to_string(), serde_json::json!("search web"));
                    args
                }),
            })
            .await
            .unwrap();

        assert!(
            !result.messages.is_empty(),
            "prompt response should have messages"
        );
    }

    #[tokio::test]
    async fn test_prompts_get_nonexistent() {
        let (peer, _, _) = setup_mcp_client().await;

        let result = peer
            .get_prompt(GetPromptRequestParams {
                meta: None,
                name: "nonexistent_prompt".to_string(),
                arguments: None,
            })
            .await;

        assert!(result.is_err(), "nonexistent prompt should return error");
    }

    // --- 4B: Back-door test (gatemini as client to backends) ---

    #[tokio::test]
    async fn test_backend_tool_call_params() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("param-test", Duration::ZERO);
        insert_mock(&manager, &registry, &mock).await;

        let args = serde_json::json!({"key": "value", "count": 42});
        manager
            .call_tool("param-test", "echo_tool", Some(args.clone()))
            .await
            .unwrap();

        let log = mock.call_log().await;
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].0, "echo_tool");
        assert_eq!(log[0].1, Some(args));
    }

    // --- Helper ---

    fn extract_text(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|c| match &c.raw {
                RawContent::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
