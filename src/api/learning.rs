//! Learning rules admin API.

use axum::extract::{Path, State};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde_json::json;

use crate::learning::store::LearningStore;

use super::errors::ApiError;
use super::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/rules", get(list_rules))
        .route("/rules/{id}", delete(delete_rule))
        .route("/rules/{id}/approve", post(approve_rule))
        .route("/rules/{id}/archive", post(archive_rule))
}

#[utoipa::path(get, path = "/api/learning/rules", tag = "learning", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn list_rules(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    super::rbac::require_maintainer(&state, &headers).await?;
    let store = LearningStore::from_pool(&state.pool);
    let rules = store.list_rules().await.map_err(ApiError::internal_error)?;
    let items: Vec<serde_json::Value> = rules
        .into_iter()
        .map(|r| {
            json!({
                "id": r.id,
                "detector": r.detector,
                "file_pattern": r.file_pattern,
                "message_pattern": r.message_pattern,
                "action": r.action.as_str(),
                "reason": r.reason,
                "status": r.status,
                "source_count": r.source_count,
                "repo_full_name": r.repo_full_name,
                "created_at": r.created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "rules": items })))
}

/// Shared 200/404 mapping for rule mutations: auth stays in the caller
/// (it must run before the store write), this only maps the outcome.
fn rule_outcome(
    id: String,
    changed: Result<bool, anyhow::Error>,
    not_found: &str,
) -> Result<Json<serde_json::Value>, ApiError> {
    if changed.map_err(ApiError::internal_error)? {
        Ok(Json(json!({ "status": "ok", "id": id })))
    } else {
        Err(ApiError::not_found(not_found))
    }
}

#[utoipa::path(post, path = "/api/learning/rules/{id}/approve", tag = "learning", params(("id" = String, Path, description = "Rule id")), responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn approve_rule(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    super::rbac::require_maintainer(&state, &headers).await?;
    let store = LearningStore::from_pool(&state.pool);
    let changed = store.approve_rule(&id).await;
    rule_outcome(id, changed, "rule not found or not suggested")
}

#[utoipa::path(post, path = "/api/learning/rules/{id}/archive", tag = "learning", params(("id" = String, Path, description = "Rule id")), responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn archive_rule(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    super::rbac::require_maintainer(&state, &headers).await?;
    let store = LearningStore::from_pool(&state.pool);
    let changed = store.archive_rule(&id).await;
    rule_outcome(id, changed, "rule not found")
}

#[utoipa::path(delete, path = "/api/learning/rules/{id}", tag = "learning", params(("id" = String, Path, description = "Rule id")), responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn delete_rule(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    super::rbac::require_maintainer(&state, &headers).await?;
    let store = LearningStore::from_pool(&state.pool);
    let changed = store.delete_rule(&id).await;
    rule_outcome(id, changed, "rule not found")
}
