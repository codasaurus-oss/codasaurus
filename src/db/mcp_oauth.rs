//! Persistence for the generic MCP OAuth flow. Tokens are secrets and are
//! never returned by any API response; only their presence is reported.

use chrono::{DateTime, Utc};
use sqlx::prelude::FromRow;

use super::{db_execute, DbPool};

#[derive(Debug, FromRow)]
pub struct OAuthState {
    pub state: String,
    pub owner_id: i64,
    pub server_id: String,
    pub code_verifier: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct OAuthToken {
    pub server_id: String,
    pub owner_id: i64,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

pub async fn save_state(
    pool: &DbPool,
    state: &str,
    owner_id: i64,
    server_id: &str,
    verifier: &str,
) -> Result<(), sqlx::Error> {
    db_execute!(
        pool,
        "INSERT INTO mcp_oauth_states (state, owner_id, server_id, code_verifier)
         VALUES (?, ?, ?, ?)
         ON CONFLICT (state) DO UPDATE SET
           owner_id = excluded.owner_id, server_id = excluded.server_id,
           code_verifier = excluded.code_verifier, created_at = now()",
        state,
        owner_id,
        server_id,
        verifier
    )?;
    Ok(())
}

/// Single-use state consumption; states older than ten minutes are ignored.
pub async fn consume_state(pool: &DbPool, state: &str) -> Result<Option<OAuthState>, sqlx::Error> {
    sqlx::query_as::<_, OAuthState>(
        "DELETE FROM mcp_oauth_states WHERE state = $1
         AND created_at > now() - interval '10 minutes' RETURNING *",
    )
    .bind(state)
    .fetch_optional(pool.as_pg())
    .await
}

pub async fn save_token(
    pool: &DbPool,
    server_id: &str,
    owner_id: i64,
    access_token: &str,
    refresh_token: Option<&str>,
    expires_at: Option<DateTime<Utc>>,
) -> Result<(), sqlx::Error> {
    db_execute!(
        pool,
        "INSERT INTO mcp_oauth_tokens (server_id, owner_id, access_token, refresh_token, expires_at, updated_at)
         VALUES (?, ?, ?, ?, ?, now())
         ON CONFLICT (server_id, owner_id) DO UPDATE SET
           access_token = excluded.access_token,
           refresh_token = excluded.refresh_token,
           expires_at = excluded.expires_at,
           updated_at = now()",
        server_id,
        owner_id,
        access_token,
        refresh_token,
        expires_at
    )?;
    Ok(())
}

pub async fn get_token(
    pool: &DbPool,
    server_id: &str,
    owner_id: i64,
) -> Result<Option<OAuthToken>, sqlx::Error> {
    sqlx::query_as::<_, OAuthToken>(
        "SELECT * FROM mcp_oauth_tokens WHERE server_id = $1 AND owner_id = $2",
    )
    .bind(server_id)
    .bind(owner_id)
    .fetch_optional(pool.as_pg())
    .await
}

pub async fn delete_token(
    pool: &DbPool,
    server_id: &str,
    owner_id: i64,
) -> Result<(), sqlx::Error> {
    db_execute!(
        pool,
        "DELETE FROM mcp_oauth_tokens WHERE server_id = ? AND owner_id = ?",
        server_id,
        owner_id
    )?;
    Ok(())
}

pub async fn has_token(pool: &DbPool, server_id: &str) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM mcp_oauth_tokens WHERE server_id = $1)",
    )
    .bind(server_id)
    .fetch_one(pool.as_pg())
    .await
}

/// Any owner's token for a server; reviews run as the instance, not a user.
pub async fn token_for_server(
    pool: &DbPool,
    server_id: &str,
) -> Result<Option<OAuthToken>, sqlx::Error> {
    sqlx::query_as::<_, OAuthToken>(
        "SELECT * FROM mcp_oauth_tokens WHERE server_id = $1 ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(server_id)
    .fetch_optional(pool.as_pg())
    .await
}
