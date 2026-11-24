//! MCP (Model Context Protocol) connectors — lets the review LLM call
//! external tools (e.g. Context7 for current library docs) during review.
//!
//! Remote HTTP/SSE servers only, instance-wide config, per-repo opt-in via
//! `.codasaurus.toml`'s `checks.mcp_tools`. Structurally inert in offline
//! mode: [`list_enabled_tools`] always returns empty there.

pub mod cache;
pub mod catalog;
pub mod client;

use crate::db::DbPool;
use anyhow::{bail, Result};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

const MAX_CONCURRENT_MCP_FETCHES: usize = 4;

/// Default-deny write-looking tools because PR content can steer the model.
/// This is a heuristic safety net, not a replacement for operator allowlisting.
fn is_write_tool(name: &str) -> bool {
    const WRITE_PREFIXES: &[&str] = &[
        "create",
        "update",
        "delete",
        "remove",
        "write",
        "put",
        "post",
        "patch",
        "set",
        "archive",
        "close",
        "merge",
        "publish",
        "send",
        "edit",
        "add",
        "invite",
        "revoke",
        "ban",
        "execute",
        "run",
        "trigger",
        "deploy",
        "assign",
        "move",
        "rename",
        "restore",
        "approve",
        "reject",
        "comment",
        "reply",
        "upload",
        "commit",
        "push",
        "fork",
        "star",
        "watch",
        "subscribe",
        "unsubscribe",
        "cancel",
        "resolve",
        "reopen",
        "lock",
        "unlock",
        "pin",
        "unpin",
    ];
    let lower = name.to_ascii_lowercase();
    lower.split('_').any(|segment| {
        WRITE_PREFIXES
            .iter()
            .any(|prefix| segment.starts_with(prefix))
    })
}

/// One tool exposed to the LLM, qualified with its server so names never collide.
#[derive(Debug, Clone)]
pub struct McpToolSpec {
    /// `"<server_id>__<tool_name>"`
    pub qualified_name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Instance-wide server record merged from the static catalog + DB overrides.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ServerInfo {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub docs_url: Option<String>,
    pub enabled: bool,
    pub key_configured: bool,
    pub is_custom: bool,
}

async fn server_configs(pool: &DbPool) -> std::collections::HashMap<String, String> {
    crate::db::config::get_all_config(pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|entry| (entry.key, entry.value))
        .collect()
}

fn list_servers_from_config(config: &std::collections::HashMap<String, String>) -> Vec<ServerInfo> {
    let mut out = Vec::new();

    for entry in catalog::CATALOG {
        let enabled = config
            .get(&format!("mcp_{}_enabled", entry.id))
            .is_some_and(|value| value == "true");
        let key_configured = config
            .get(&format!("mcp_{}_api_key", entry.id))
            .is_some_and(|value| !value.is_empty());
        out.push(ServerInfo {
            id: entry.id.to_string(),
            name: entry.name.to_string(),
            base_url: entry.base_url.to_string(),
            docs_url: Some(entry.docs_url.to_string()),
            enabled,
            key_configured,
            is_custom: false,
        });
    }

    let custom_ids: Vec<String> = config
        .get("mcp_custom_server_ids")
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default();
    for id in custom_ids {
        let enabled = config
            .get(&format!("mcp_{id}_enabled"))
            .is_some_and(|value| value == "true");
        let key_configured = config
            .get(&format!("mcp_{id}_api_key"))
            .is_some_and(|value| !value.is_empty());
        let name = config
            .get(&format!("mcp_{id}_name"))
            .cloned()
            .unwrap_or_else(|| id.clone());
        let base_url = config
            .get(&format!("mcp_{id}_base_url"))
            .cloned()
            .unwrap_or_default();
        out.push(ServerInfo {
            id,
            name,
            base_url,
            docs_url: None,
            enabled,
            key_configured,
            is_custom: true,
        });
    }

    out
}

/// All servers known to this instance: catalog entries plus any custom ones,
/// merged with their DB-stored enabled/key state. Never returns raw keys.
pub async fn list_servers(pool: &DbPool) -> Vec<ServerInfo> {
    list_servers_from_config(&server_configs(pool).await)
}

/// Tool specs for every enabled server, in OpenAI-compatible shape. Empty in
/// offline mode or when a server's `tools/list` call fails (fails open —
/// one broken server doesn't kill review).
pub async fn list_enabled_tools(pool: &DbPool) -> Vec<McpToolSpec> {
    if crate::registry::offline_mode() {
        return Vec::new();
    }

    let config = server_configs(pool).await;
    let servers: Vec<_> = list_servers_from_config(&config)
        .into_iter()
        .filter(|server| server.enabled && !server.base_url.is_empty())
        .map(|server| {
            let api_key = config
                .get(&format!("mcp_{}_api_key", server.id))
                .cloned()
                .unwrap_or_default();
            (server, api_key)
        })
        .collect();
    let server_count = servers.len();
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_MCP_FETCHES));
    let mut tasks = JoinSet::new();

    for (index, (server, api_key)) in servers.into_iter().enumerate() {
        let semaphore = Arc::clone(&semaphore);
        tasks.spawn(async move {
            let Ok(_permit) = semaphore.acquire().await else {
                return (index, Vec::new());
            };
            let tools = cache::get_or_fetch_tools(&server.id, &server.base_url, &api_key).await;
            let specs = tools
                .into_iter()
                .filter_map(|tool| {
                    let name = tool.get("name")?.as_str()?;
                    if is_write_tool(name) {
                        return None;
                    }
                    Some(McpToolSpec {
                        qualified_name: format!("{}__{name}", server.id),
                        description: tool
                            .get("description")
                            .and_then(|value| value.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        input_schema: tool
                            .get("inputSchema")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({"type": "object"})),
                    })
                })
                .collect();
            (index, specs)
        });
    }

    let mut batches = vec![Vec::new(); server_count];
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok((index, specs)) => batches[index] = specs,
            Err(error) => tracing::warn!(error = %error, "MCP tool discovery task failed"),
        }
    }
    batches.into_iter().flatten().collect()
}

/// Truncated at 8000 chars — bounds what a tool result can cost in the next
/// LLM round-trip, matching `llm::truncate_chars`'s cap on diffs.
const MAX_TOOL_RESULT_CHARS: usize = 8000;

pub async fn call_tool(pool: &DbPool, qualified_name: &str, arguments: &Value) -> Result<String> {
    let Some((server_id, tool_name)) = qualified_name.split_once("__") else {
        bail!("malformed MCP tool name: {qualified_name}");
    };

    // Re-check the live allowlist: injected content or a config change can
    // introduce a tool name that was unavailable when tools were offered.
    let config = server_configs(pool).await;
    let server = list_servers_from_config(&config)
        .into_iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| anyhow::anyhow!("unknown MCP server: {server_id}"))?;
    if !server.enabled || server.base_url.is_empty() {
        bail!("MCP server {server_id} is not enabled");
    }
    let api_key = config
        .get(&format!("mcp_{server_id}_api_key"))
        .cloned()
        .unwrap_or_default();
    let available = cache::get_or_fetch_tools(&server.id, &server.base_url, &api_key).await;
    let allowed = available
        .iter()
        .any(|t| t.get("name").and_then(|v| v.as_str()) == Some(tool_name));
    if !allowed {
        bail!("tool {tool_name} is not in {server_id}'s current tool list");
    }
    if is_write_tool(tool_name) {
        bail!("tool {qualified_name} looks state-mutating; write tools are not permitted");
    }

    let result = client::tools_call(&server.base_url, &api_key, tool_name, arguments).await;

    crate::db::audit::log_event(pool, "mcp.tool_called", None, Some(qualified_name), None).await;

    let text = match &result {
        Ok(v) => v.to_string(),
        Err(e) => format!("Error calling {qualified_name}: {e}"),
    };
    let truncated: String = text.chars().take(MAX_TOOL_RESULT_CHARS).collect();
    Ok(truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_tool_classification_covers_prefixes_and_segments() {
        for name in [
            "create_issue",
            "UpdateIssue",
            "repo__delete_branch",
            "foo_set_status",
            "execute_command",
        ] {
            assert!(is_write_tool(name), "{name}");
        }

        for name in [
            "get_issue",
            "search_docs",
            "list_repositories",
            "fetch_summary",
        ] {
            assert!(!is_write_tool(name), "{name}");
        }
    }
}
