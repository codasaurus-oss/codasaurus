use axum::extract::{Path, State};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::bot::{github_api_headers, github_get, github_post, next_github_link, GITHUB_CLIENT};
use crate::db;
use crate::retry::RetryConfig;

use super::errors::ApiError;
use super::AppState;

#[derive(Deserialize)]
pub struct UpdateRepoBody {
    pub config_json: String,
    pub active: bool,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_repos))
        .route("/sync", post(sync_repos))
        .route("/{id}", get(get_repo))
        .route("/{id}", put(update_repo))
        .route("/{id}", delete(delete_repo))
}

/// GET /api/repos
#[utoipa::path(get, path = "/api/repos", tag = "repos", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn list_repos(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    super::rbac::require_role(&state.pool, &headers, super::rbac::MinRole::Viewer).await?;
    let repos = db::repos::list_repos(&state.pool).await?;
    Ok(Json(json!(repos)))
}

/// POST /api/repos/sync — fetch all installations + repos from GitHub and
/// store them in the local database.  Returns the number of repos synced.
#[utoipa::path(post, path = "/api/repos/sync", tag = "repos", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn sync_repos(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    super::rbac::require_maintainer(&state, &headers).await?;
    let (app_id, private_key) = super::github_app_credentials(&state.pool)
        .await
        .ok_or_else(|| ApiError::bad_request("No GitHub App configured"))?;

    let jwt = crate::github_jwt::create_app_jwt(&app_id, &private_key)
        .map_err(|e| ApiError::internal_error(format!("JWT error: {e}")))?;

    let client = GITHUB_CLIENT
        .as_ref()
        .ok_or_else(|| ApiError::internal_error("GitHub API client not available"))?;
    let jwt_auth = format!("Bearer {jwt}");
    let jwt_headers = github_api_headers(&jwt_auth).map_err(ApiError::internal_error)?;

    let installations: Vec<serde_json::Value> = github_get(
        client,
        &jwt_headers,
        "https://api.github.com/app/installations",
        "sync_list_installations",
        &RetryConfig::api_default(),
    )
    .await
    .map_err(|e| ApiError::bad_request(format!("GitHub API: {e}")))?;

    let mut total = 0usize;
    for inst in &installations {
        let Some(inst_id) = inst["id"].as_i64() else {
            continue;
        };
        total += sync_installation_repos(&state, client, &jwt_headers, inst_id).await?;
    }

    Ok(Json(json!({ "status": "ok", "synced": total })))
}

/// Mint an installation token, page all its repos, and persist them.
/// One bad installation never fails the whole sync.
async fn sync_installation_repos(
    state: &AppState,
    client: &reqwest::Client,
    jwt_headers: &axum::http::HeaderMap,
    inst_id: i64,
) -> Result<usize, ApiError> {
    let Some(repos) = installation_repos(client, jwt_headers, inst_id).await? else {
        return Ok(0);
    };
    let creates: Vec<db::models::RepoCreate> = repos
        .iter()
        .filter_map(|repo| {
            let full_name = repo["full_name"].as_str()?;
            let (owner, name) = full_name.split_once('/')?;
            Some(db::models::RepoCreate {
                github_id: repo["id"].as_i64(),
                full_name: full_name.to_string(),
                owner: owner.to_string(),
                name: name.to_string(),
                default_branch: repo["default_branch"].as_str().map(str::to_string),
                installation_id: inst_id,
                private: repo["private"].as_bool().unwrap_or(false),
            })
        })
        .collect();
    Ok(db::repos::upsert_repos(&state.pool, &creates)
        .await?
        .try_into()
        .unwrap_or(usize::MAX))
}

/// Installation token + paged repo list. `None` when this installation
/// yields no usable token (already logged); the sync moves on.
async fn installation_repos(
    client: &reqwest::Client,
    jwt_headers: &axum::http::HeaderMap,
    inst_id: i64,
) -> Result<Option<Vec<serde_json::Value>>, ApiError> {
    use crate::retry::{is_reqwest_error_retryable, retry_async};

    let token_resp: serde_json::Value = github_post(
        client,
        jwt_headers,
        &format!("https://api.github.com/app/installations/{inst_id}/access_tokens"),
        None,
        "sync_installation_token",
        &RetryConfig::api_default(),
    )
    .await
    .map_err(|e| ApiError::bad_request(format!("Token request: {e}")))?;

    let Some(token) = token_resp["token"].as_str() else {
        return Ok(None);
    };
    let token_headers =
        github_api_headers(&format!("Bearer {token}")).map_err(ApiError::internal_error)?;

    let mut all_repos: Vec<serde_json::Value> = Vec::new();
    let mut page_url: Option<String> =
        Some("https://api.github.com/installation/repositories?per_page=100".into());
    let mut pages = 0u32;

    while let Some(url) = page_url.take() {
        pages += 1;
        let resp = retry_async(
            &RetryConfig::api_default(),
            "sync_list_repos",
            &is_reqwest_error_retryable,
            || async {
                client
                    .get(&url)
                    .headers(token_headers.clone())
                    .send()
                    .await
                    .and_then(|response| response.error_for_status())
                    .map_err(Into::into)
            },
        )
        .await
        .map_err(|e| ApiError::bad_request(format!("Repos request: {e}")))?;

        page_url = next_github_link(resp.headers());

        let page_data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ApiError::bad_request(format!("Repos response: {e}")))?;

        if let Some(repos) = page_data["repositories"].as_array() {
            all_repos.extend(repos.iter().cloned());
        }

        if pages >= 10 || all_repos.len() >= 1000 {
            break;
        }
    }
    Ok(Some(all_repos))
}

/// GET /api/repos/:id
#[utoipa::path(get, path = "/api/repos/{id}", tag = "repos", params(("id" = i64, Path, description = "Resource id")), responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn get_repo(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (_user, repo) = super::rbac::require_repo_read(&state, &headers, id).await?;
    Ok(Json(json!(repo)))
}

/// PUT /api/repos/:id
#[utoipa::path(put, path = "/api/repos/{id}", tag = "repos", params(("id" = i64, Path, description = "Resource id")), responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn update_repo(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<UpdateRepoBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    super::rbac::require_maintainer(&state, &headers).await?;
    // Don't persist broken JSON the dashboard could never parse back.
    if !body.config_json.trim().is_empty()
        && serde_json::from_str::<serde_json::Value>(&body.config_json).is_err()
    {
        return Err(ApiError::bad_request("config_json must be valid JSON"));
    }
    db::repos::update_repo(&state.pool, id, &body.config_json, body.active).await?;
    Ok(Json(json!({ "status": "ok" })))
}

/// DELETE /api/repos/:id
#[utoipa::path(delete, path = "/api/repos/{id}", tag = "repos", params(("id" = i64, Path, description = "Resource id")), responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn delete_repo(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    super::rbac::require_maintainer(&state, &headers).await?;
    db::repos::delete_repo(&state.pool, id).await?;
    Ok(Json(json!({ "status": "ok" })))
}
