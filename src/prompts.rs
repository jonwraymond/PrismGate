use std::sync::Arc;

use rmcp::{ErrorData as McpError, model::*};

use crate::backend::BackendManager;
use crate::registry::ToolRegistry;

/// Return the list of available prompts.
pub fn list_prompts() -> Vec<Prompt> {
    vec![
        Prompt {
            name: "discover".to_string(),
            title: Some("Discover Tools".to_string()),
            description: Some(
                "Guided workflow for discovering gatemini tools progressively".to_string(),
            ),
            arguments: None,
            icons: None,
            meta: None,
        },
        Prompt {
            name: "find_tool".to_string(),
            title: Some("Find Tool".to_string()),
            description: Some(
                "Search for tools matching a task and get the top match's schema".to_string(),
            ),
            arguments: Some(vec![PromptArgument {
                name: "task".to_string(),
                title: Some("Task Description".to_string()),
                description: Some("What you need to accomplish".to_string()),
                required: Some(true),
            }]),
            icons: None,
            meta: None,
        },
        Prompt {
            name: "backend_status".to_string(),
            title: Some("Backend Status".to_string()),
            description: Some("Health and status of all backends with tool counts".to_string()),
            arguments: None,
            icons: None,
            meta: None,
        },
    ]
}

/// Handle get_prompt for gatemini prompts.
pub async fn get_prompt(
    name: &str,
    arguments: Option<JsonObject>,
    registry: &Arc<ToolRegistry>,
    backend_manager: &Arc<BackendManager>,
) -> Result<GetPromptResult, McpError> {
    match name {
        "discover" => Ok(discover_prompt(registry)),
        "find_tool" => {
            let task = arguments
                .as_ref()
                .and_then(|args| args.get("task"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    McpError::invalid_params("Required argument 'task' missing".to_string(), None)
                })?;
            Ok(find_tool_prompt(task, registry))
        }
        "backend_status" => Ok(backend_status_prompt(registry, backend_manager)),
        _ => Err(McpError::invalid_params(
            format!("Unknown prompt: {name}"),
            None,
        )),
    }
}

fn discover_prompt(registry: &ToolRegistry) -> GetPromptResult {
    let tool_count = registry.tool_count();
    let backend_count = registry.backend_count();

    let text = format!(
        "# Gatemini Tool Discovery\n\n\
         You're connected to gatemini, an MCP gateway aggregating **{tool_count} tools** across **{backend_count} backends**.\n\n\
         ## Step 1: See what's available\n\
         Use `@gatemini://backends` to see all backends and their tool counts.\n\n\
         ## Step 2: Search for what you need\n\
         Use `search_tools` with a task description. It defaults to brief mode (~60 tokens/result):\n\
         ```\n\
         search_tools(task_description=\"web search\", limit=5)\n\
         ```\n\n\
         ## Step 3: Get full schema (only when needed)\n\
         Use `tool_info` with `detail=\"full\"` for the complete input schema:\n\
         ```\n\
         tool_info(tool_name=\"web_search_exa\", detail=\"full\")\n\
         ```\n\
         Or load via resource: `@gatemini://tool/web_search_exa`\n\n\
         ## Step 4: Execute\n\
         Use `call_tool_chain` to run TypeScript that calls backend tools:\n\
         ```typescript\n\
         const result = await exa.web_search_exa({{ query: \"MCP protocol\" }});\n\
         return result;\n\
         ```\n\n\
         ## Tips\n\
         - Brief mode saves 80-98% tokens on discovery\n\
         - Only load full schemas when you're ready to call a tool\n\
         - Use `@gatemini://tools` for a compact index of all {tool_count} tools\n"
    );

    GetPromptResult {
        description: Some("Guided workflow for progressive tool discovery".to_string()),
        messages: vec![PromptMessage::new_text(PromptMessageRole::Assistant, text)],
    }
}

fn find_tool_prompt(task: &str, registry: &ToolRegistry) -> GetPromptResult {
    // Search for tools matching the task
    let results = registry.search(task, 5);

    let mut text = format!("# Tools for: {task}\n\n");

    if results.is_empty() {
        text.push_str("No tools found matching this task. Try a different description.\n");
    } else {
        text.push_str("## Search Results\n\n");
        text.push_str("| # | Tool | Backend | Description |\n");
        text.push_str("|---|------|---------|-------------|\n");
        for (i, entry) in results.iter().enumerate() {
            let desc = first_sentence(&entry.description);
            text.push_str(&format!(
                "| {} | `{}` | {} | {} |\n",
                i + 1,
                entry.name,
                entry.backend_name,
                desc
            ));
        }

        // Include full schema for the top match
        if let Some(top) = results.first() {
            text.push_str(&format!(
                "\n## Top Match: `{}`\n\n\
                 **Backend:** {}\n\
                 **Description:** {}\n\n\
                 **Input Schema:**\n```json\n{}\n```\n\n\
                 **Execute with:**\n```typescript\n\
                 const result = await {}.{}({{ /* params */ }});\n\
                 return result;\n\
                 ```\n",
                top.name,
                top.backend_name,
                top.description,
                serde_json::to_string_pretty(&top.input_schema).unwrap_or_default(),
                top.backend_name.replace('-', "_"),
                top.name,
            ));
        }
    }

    GetPromptResult {
        description: Some(format!("Tools matching: {task}")),
        messages: vec![PromptMessage::new_text(PromptMessageRole::Assistant, text)],
    }
}

fn backend_status_prompt(
    registry: &ToolRegistry,
    backend_manager: &BackendManager,
) -> GetPromptResult {
    let statuses = backend_manager.get_all_status();

    let mut text = format!(
        "# Gatemini Backend Status\n\n\
         **Total:** {} backends, {} tools\n\n\
         | Backend | Status | Available | Tools |\n\
         |---------|--------|-----------|-------|\n",
        statuses.len(),
        registry.tool_count(),
    );

    for status in &statuses {
        let tool_count = registry.get_by_backend(&status.name).len();
        let available = if status.available { "Yes" } else { "No" };
        text.push_str(&format!(
            "| {} | {:?} | {} | {} |\n",
            status.name, status.state, available, tool_count
        ));
    }

    GetPromptResult {
        description: Some("Health and status of all backends".to_string()),
        messages: vec![PromptMessage::new_text(PromptMessageRole::Assistant, text)],
    }
}

fn first_sentence(text: &str) -> String {
    if let Some(idx) = text.find(". ") {
        text[..=idx].to_string()
    } else if let Some(idx) = text.find(".\n") {
        text[..=idx].to_string()
    } else if text.ends_with('.') {
        text.to_string()
    } else if text.len() > 120 {
        format!("{}...", &text[..120])
    } else {
        text.to_string()
    }
}
