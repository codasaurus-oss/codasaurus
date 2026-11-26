//! MCP (Model Context Protocol) connectors — lets the review LLM call
//! external tools (e.g. Context7 for current library docs) during review.
//!
//! Remote HTTP/SSE servers only, instance-wide config, and per-repo policy via
//! `.codasaurus.toml`'s `checks.mcp_tools`. Configured servers are enabled by
//! default; offline mode remains structurally inert.

pub mod cache;
pub mod catalog;
pub mod client;

use crate::db::DbPool;
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

/// One enabled MCP server exposed to LiteLLM for a review.
#[derive(Debug, Clone)]
pub struct McpServerSpec {
    pub alias: String,
    pub allowed_tools: Vec<String>,
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
    pub oauth_connected: bool,
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
            oauth_connected: false,
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
            oauth_connected: false,
            is_custom: true,
        });
    }

    out
}

/// All servers known to this instance: catalog entries plus any custom ones,
/// merged with their DB-stored enabled/key state. Never returns raw keys.
pub async fn list_servers(pool: &DbPool) -> Vec<ServerInfo> {
    let mut servers = list_servers_from_config(&server_configs(pool).await);
    for server in &mut servers {
        server.oauth_connected = crate::db::mcp_oauth::has_token(pool, &server.id)
            .await
            .unwrap_or(false);
    }
    servers
}

/// Reconcile enabled servers into LiteLLM and return safe tool allowlists.
/// Empty in offline mode or when a server's `tools/list` call fails (fails open
/// — one broken server doesn't kill review).
pub async fn list_enabled_servers(pool: &DbPool) -> Vec<McpServerSpec> {
    if crate::registry::offline_mode() {
        return Vec::new();
    }

    let config = server_configs(pool).await;
    let mut servers = Vec::new();
    for server in list_servers_from_config(&config)
        .into_iter()
        .filter(|server| server.enabled && !server.base_url.is_empty())
    {
        let api_key = config
            .get(&format!("mcp_{}_api_key", server.id))
            .cloned()
            .unwrap_or_default();
        let api_key = match crate::db::mcp_oauth::token_for_server(pool, &server.id).await {
            Ok(Some(token)) => token.access_token,
            Ok(None) => api_key,
            Err(error) => {
                tracing::warn!(server = %server.id, error = %error, "could not read MCP OAuth token");
                api_key
            }
        };
        servers.push((server, api_key));
    }
    let server_count = servers.len();
    let gateway = crate::llm::gateway::GatewayClient::from_env();
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_MCP_FETCHES));
    let mut tasks = JoinSet::new();

    for (index, (server, api_key)) in servers.into_iter().enumerate() {
        let semaphore = Arc::clone(&semaphore);
        let gateway = gateway.clone();
        tasks.spawn(async move {
            let Ok(_permit) = semaphore.acquire().await else {
                return (index, None);
            };
            let tools = cache::get_or_fetch_tools(&server.id, &server.base_url, &api_key).await;
            let allowed_tools: Vec<String> = tools
                .into_iter()
                .filter_map(|tool| {
                    let name = tool.get("name")?.as_str()?;
                    if is_write_tool(name) {
                        return None;
                    }
                    Some(name.to_string())
                })
                .collect();
            if allowed_tools.is_empty() {
                return (index, None);
            }
            let alias = format!("codasaurus_{}", server.id);
            let Some(gateway) = gateway else {
                tracing::warn!(server = %server.id, "LiteLLM gateway unavailable; skipping MCP server");
                return (index, None);
            };
            if let Err(error) = gateway
                .upsert_mcp_server(&alias, &server.base_url, &api_key, &allowed_tools)
                .await
            {
                tracing::warn!(server = %server.id, error = %error, "LiteLLM MCP server reconciliation failed");
                return (index, None);
            }
            (index, Some(McpServerSpec { alias, allowed_tools }))
        });
    }

    let mut batches: Vec<Option<McpServerSpec>> = vec![None; server_count];
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok((index, spec)) => batches[index] = spec,
            Err(error) => tracing::warn!(error = %error, "MCP tool discovery task failed"),
        }
    }
    batches.into_iter().flatten().collect()
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
