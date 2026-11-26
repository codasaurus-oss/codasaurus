//! Generic OAuth 2.1 authorization-code + PKCE flow for MCP servers.
//! Client credentials are instance settings; tokens are owner-scoped secrets.

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::errors::ApiError;
use super::AppState;
use crate::db;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/start/{id}", get(start))
        .route("/callback", get(callback))
        .route("/disconnect/{id}", post(disconnect))
}

#[derive(Serialize)]
pub struct OAuthStatus {
    pub connected: bool,
}

fn base64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

async fn owner_id(state: &AppState, headers: &axum::http::HeaderMap) -> Result<i64, ApiError> {
    let actor = super::rbac::require_owner(state, headers).await?;
    db::users::get_user_by_email(&state.pool, &actor.email)
        .await
        .ok()
        .flatten()
        .map(|user| user.id)
        .ok_or_else(|| ApiError::unauthorized("Session user not found"))
}

#[derive(Deserialize)]
struct Metadata {
    authorization_endpoint: String,
    token_endpoint: String,
}

async fn metadata(base_url: &str) -> Result<Metadata, ApiError> {
    let url = format!(
        "{}/.well-known/oauth-authorization-server",
        base_url.trim_end_matches('/')
    );
    let client = reqwest::Client::new();
    let value: serde_json::Value = client
        .get(&url)
        .send()
        .await
        .map_err(|e| ApiError::bad_request(format!("OAuth discovery failed: {e}")))?
        .json()
        .await
        .map_err(|e| ApiError::bad_request(format!("Invalid OAuth metadata: {e}")))?;
    serde_json::from_value(value)
        .map_err(|e| ApiError::bad_request(format!("Invalid OAuth endpoints: {e}")))
}

async fn start(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let owner = owner_id(&state, &headers).await?;
    let server = crate::mcp::list_servers(&state.pool)
        .await
        .into_iter()
        .find(|server| server.id == id)
        .ok_or_else(|| ApiError::bad_request("Unknown MCP server"))?;
    let config = db::config::get_config(&state.pool, &format!("mcp_{id}_oauth_client_id"))
        .await?
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request("Set an OAuth client id for this server first"))?;
    let meta = metadata(&server.base_url).await?;
    let verifier = base64url(uuid::Uuid::new_v4().as_bytes());
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = base64url(&digest);
    let oauth_state = base64url(uuid::Uuid::new_v4().as_bytes());
    db::mcp_oauth::save_state(&state.pool, &oauth_state, owner, &id, &verifier).await?;
    let redirect = format!(
        "{}/api/mcp/oauth/callback",
        std::env::var("PUBLIC_URL")
            .unwrap_or_else(|_| "http://localhost:3000".into())
            .trim_end_matches('/')
    );
    let url = reqwest::Url::parse(&meta.authorization_endpoint)
        .map_err(|_| ApiError::bad_request("Invalid authorization endpoint"))?;
    let mut url = url;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &config)
        .append_pair("redirect_uri", &redirect)
        .append_pair("state", &oauth_state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("scope", "read");
    Ok(Json(json!({ "authorize_url": url.to_string() })))
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: String,
    state: String,
}

async fn callback(
    State(state): State<AppState>,
    Query(query): Query<CallbackQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let pending = db::mcp_oauth::consume_state(&state.pool, &query.state)
        .await?
        .ok_or_else(|| ApiError::bad_request("Invalid or expired OAuth state"))?;
    let server = crate::mcp::list_servers(&state.pool)
        .await
        .into_iter()
        .find(|server| server.id == pending.server_id)
        .ok_or_else(|| ApiError::bad_request("Unknown MCP server"))?;
    let meta = metadata(&server.base_url).await?;
    let client_id = db::config::get_config(
        &state.pool,
        &format!("mcp_{}_oauth_client_id", pending.server_id),
    )
    .await?
    .unwrap_or_default();
    let client_secret = db::config::get_config(
        &state.pool,
        &format!("mcp_{}_oauth_client_secret", pending.server_id),
    )
    .await?
    .unwrap_or_default();
    let redirect = format!(
        "{}/api/mcp/oauth/callback",
        std::env::var("PUBLIC_URL")
            .unwrap_or_else(|_| "http://localhost:3000".into())
            .trim_end_matches('/')
    );
    let mut form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", query.code.clone()),
        ("redirect_uri", redirect),
        ("client_id", client_id.clone()),
        ("code_verifier", pending.code_verifier.clone()),
    ];
    if !client_secret.is_empty() {
        form.push(("client_secret", client_secret));
    }
    let response = reqwest::Client::new()
        .post(&meta.token_endpoint)
        .form(&form)
        .send()
        .await
        .map_err(|e| ApiError::bad_request(format!("Token exchange failed: {e}")))?;
    if !response.status().is_success() {
        return Err(ApiError::bad_request("OAuth token exchange was rejected"));
    }
    let token: serde_json::Value = response
        .json()
        .await
        .map_err(|e| ApiError::bad_request(format!("Invalid token response: {e}")))?;
    let access = token["access_token"]
        .as_str()
        .ok_or_else(|| ApiError::bad_request("Token response has no access_token"))?;
    let refresh = token["refresh_token"].as_str();
    let expires_in = token["expires_in"].as_u64().unwrap_or(3600);
    db::mcp_oauth::save_token(
        &state.pool,
        &pending.server_id,
        pending.owner_id,
        access,
        refresh,
        Some(chrono::Utc::now() + chrono::Duration::seconds(expires_in as i64)),
    )
    .await?;
    Ok(Json(json!({ "status": "ok" })))
}

async fn disconnect(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let owner = owner_id(&state, &headers).await?;
    db::mcp_oauth::delete_token(&state.pool, &id, owner).await?;
    Ok(Json(json!({ "status": "ok" })))
}
