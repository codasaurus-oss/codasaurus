//! Role-based access helpers for the dashboard API.
//!
//! Hierarchy: owner > maintainer > viewer.

use axum::http::HeaderMap;

use crate::db;
use crate::db::models::UserView;

use super::auth;
use super::errors::ApiError;
use super::AppState;

/// Minimum role required for an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MinRole {
    Viewer,
    Maintainer,
    Owner,
}

impl MinRole {
    fn rank(self) -> u8 {
        match self {
            MinRole::Viewer => 1,
            MinRole::Maintainer => 2,
            MinRole::Owner => 3,
        }
    }
}

pub fn role_rank(role: &str) -> u8 {
    match role {
        "owner" | "admin" => 3, // admin = legacy alias during migration
        "maintainer" => 2,
        "viewer" => 1,
        _ => 0,
    }
}

pub fn is_valid_role(role: &str) -> bool {
    matches!(role, "owner" | "maintainer" | "viewer")
}

pub fn normalize_role(role: &str) -> &str {
    match role {
        "admin" => "owner",
        r if is_valid_role(r) => r,
        _ => "viewer",
    }
}

/// Load the authenticated user from the session cookie.
pub async fn current_user(pool: &db::DbPool, headers: &HeaderMap) -> Result<UserView, ApiError> {
    let token = auth::extract_token(headers)
        .ok_or_else(|| ApiError::unauthorized("Authentication required"))?;
    let user = db::sessions::get_session_user(pool, &token)
        .await
        .map_err(ApiError::internal_error)?
        .ok_or_else(|| ApiError::unauthorized("Authentication required"))?;
    Ok(UserView {
        email: user.email,
        role: normalize_role(&user.role).to_string(),
        is_bootstrap: user.is_bootstrap,
    })
}

/// Require the session user to have at least `min` role.
pub async fn require_role(
    pool: &db::DbPool,
    headers: &HeaderMap,
    min: MinRole,
) -> Result<UserView, ApiError> {
    let user = current_user(pool, headers).await?;
    if role_rank(&user.role) < min.rank() {
        return Err(ApiError::forbidden(
            "Insufficient permissions for this action",
        ));
    }
    Ok(user)
}

pub async fn require_owner(state: &AppState, headers: &HeaderMap) -> Result<UserView, ApiError> {
    require_role(&state.pool, headers, MinRole::Owner).await
}

pub async fn require_maintainer(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<UserView, ApiError> {
    require_role(&state.pool, headers, MinRole::Maintainer).await
}

/// Require a Viewer session and return the repo.
pub async fn require_repo_read(
    state: &AppState,
    headers: &HeaderMap,
    repo_id: i64,
) -> Result<(UserView, db::models::Repo), ApiError> {
    let user = require_role(&state.pool, headers, MinRole::Viewer).await?;
    let repo = db::repos::get_repo(&state.pool, repo_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Repo {repo_id} not found")))?;
    Ok((user, repo))
}

/// Resolve a review and its repo in one query after a single auth lookup.
pub async fn require_review_read(
    state: &AppState,
    headers: &HeaderMap,
    review_id: i64,
) -> Result<(UserView, db::reviews::ReviewWithRepo), ApiError> {
    let user = require_role(&state.pool, headers, MinRole::Viewer).await?;
    let review = db::reviews::get_review_with_repo(&state.pool, review_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Review {review_id} not found")))?;
    Ok((user, review))
}
