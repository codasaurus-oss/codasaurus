//! MCP connector configuration — instance-wide server list + API keys.
//! Kept separate from `settings.rs` because custom-server keys are a dynamic
//! namespace (`mcp_<id>_*`) that doesn't fit the static ALLOWED_KEYS/
//! SENSITIVE_KEYS arrays there.

use axum::extract::{Path, State};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::db;
use crate::mcp;

use super::errors::ApiError;
use super::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/servers", get(list_servers))
        .route("/servers/{id}", put(upsert_server))
        .route("/servers/{id}", delete(delete_server))
        .route("/{id}/test", post(test_server))
}

#[derive(Deserialize)]
pub(crate) struct UpsertServerBody {
    enabled: bool,
    /// Blank/omitted keeps the existing stored key unchanged.
    #[serde(default)]
    api_key: Option<String>,
    /// Required for custom servers; ignored for catalog servers.
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

async fn update_custom_server_ids(pool: &db::DbPool, id: &str, add: bool) -> Result<(), ApiError> {
    let raw = db::config::get_config(pool, "mcp_custom_server_ids").await?;
    let mut ids: Vec<String> = match raw {
        Some(raw) => serde_json::from_str(&raw).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "mcp_custom_server_ids is corrupt; resetting");
            Vec::new()
        }),
        None => Vec::new(),
    };
    let changed = if add {
        if ids.iter().any(|i| i == id) {
            false
        } else {
            ids.push(id.to_string());
            true
        }
    } else {
        let before = ids.len();
        ids.retain(|i| i.as_str() != id);
        ids.len() != before
    };
    if changed {
        db::config::set_config(
            pool,
            "mcp_custom_server_ids",
            &serde_json::to_string(&ids).expect("server id list is serializable"),
        )
        .await?;
    }
    Ok(())
}

/// GET /api/mcp/servers
#[utoipa::path(get, path = "/api/mcp/servers", tag = "mcp", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn list_servers(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    super::rbac::require_maintainer(&state, &headers).await?;
    let servers = mcp::list_servers(&state.pool).await;
    Ok(Json(json!({ "servers": servers })))
}

/// PUT /api/mcp/servers/:id
#[utoipa::path(put, path = "/api/mcp/servers/{id}", tag = "mcp", params(("id" = i64, Path, description = "Resource id")), responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn upsert_server(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<UpsertServerBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let actor = super::rbac::require_owner(&state, &headers).await?;

    if mcp::catalog::find(&id).is_none() {
        let base_url = body
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| ApiError::bad_request("base_url is required for a custom server"))?
            .to_string();
        crate::ssrf::validate_http_url_resolved(&base_url, false)
            .await
            .map_err(ApiError::bad_request)?;
        db::config::set_config(&state.pool, &format!("mcp_{id}_base_url"), &base_url).await?;
        if let Some(name) = body
            .name
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            db::config::set_config(&state.pool, &format!("mcp_{id}_name"), name).await?;
        }
        update_custom_server_ids(&state.pool, &id, true).await?;
    }

    db::config::set_config(
        &state.pool,
        &format!("mcp_{id}_enabled"),
        if body.enabled { "true" } else { "false" },
    )
    .await?;

    if let Some(key) = body.api_key.as_deref() {
        if !key.is_empty() && !key.contains('•') {
            db::config::set_config(&state.pool, &format!("mcp_{id}_api_key"), key).await?;
        }
    }

    db::audit::log_event(
        &state.pool,
        "mcp.server_updated",
        Some(&actor.email),
        Some(&id),
        None,
    )
    .await;

    Ok(Json(json!({ "status": "ok" })))
}

/// DELETE /api/mcp/servers/:id
#[utoipa::path(delete, path = "/api/mcp/servers/{id}", tag = "mcp", params(("id" = i64, Path, description = "Resource id")), responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn delete_server(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let actor = super::rbac::require_owner(&state, &headers).await?;

    db::config::delete_config(&state.pool, &format!("mcp_{id}_enabled")).await?;
    db::config::delete_config(&state.pool, &format!("mcp_{id}_api_key")).await?;
    if mcp::catalog::find(&id).is_none() {
        db::config::delete_config(&state.pool, &format!("mcp_{id}_base_url")).await?;
        db::config::delete_config(&state.pool, &format!("mcp_{id}_name")).await?;
        update_custom_server_ids(&state.pool, &id, false).await?;
    }

    db::audit::log_event(
        &state.pool,
        "mcp.server_removed",
        Some(&actor.email),
        Some(&id),
        None,
    )
    .await;

    Ok(Json(json!({ "status": "ok" })))
}

/// POST /api/mcp/:id/test — initialize + tools/list against the configured server.
#[utoipa::path(post, path = "/api/mcp/{id}/test", tag = "mcp", params(("id" = i64, Path, description = "Resource id")), responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn test_server(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    super::rbac::require_maintainer(&state, &headers).await?;

    let base_url = db::config::get_config(&state.pool, &format!("mcp_{id}_base_url"))
        .await?
        .filter(|v| !v.is_empty())
        .or_else(|| mcp::catalog::find(&id).map(|e| e.base_url.to_string()))
        .ok_or_else(|| ApiError::bad_request("Server is not configured"))?;
    let api_key = db::config::get_config(&state.pool, &format!("mcp_{id}_api_key"))
        .await?
        .unwrap_or_default();

    crate::ssrf::validate_http_url_resolved(&base_url, false)
        .await
        .map_err(ApiError::bad_request)?;

    mcp::client::initialize(&base_url, &api_key)
        .await
        .map_err(|e| ApiError::bad_request(format!("MCP initialize failed: {e}")))?;
    let tools = mcp::client::tools_list(&base_url, &api_key)
        .await
        .map_err(|e| ApiError::bad_request(format!("MCP tools/list failed: {e}")))?;

    Ok(Json(json!({
        "status": "ok",
        "ok": true,
        "tool_count": tools.len(),
    })))
}
