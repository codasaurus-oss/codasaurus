//! REST API route handlers for the Codasaurus web dashboard.
//!
//! Each sub-module exposes a `pub fn router() -> Router<AppState>` that its
//! handlers use, and they are merged by `api::router()` below.

pub mod audit;
pub mod auth;
pub mod bootstrap;
pub mod errors;
pub mod github;
pub mod learning;
pub mod mcp;
pub mod openapi;
pub mod rate_limit;
pub mod rbac;
pub mod repos;
pub mod reviews;
pub mod settings;
pub mod setup;
pub mod stats;
pub mod users;

use axum::middleware;
use axum::Router;

use crate::db::DbPool;
use crate::redis::StateStore;

/// Resolve App ID and private key from DB settings, falling back to the environment.
pub(crate) async fn github_app_credentials(pool: &DbPool) -> Option<(String, String)> {
    let app_id = crate::db::config::get_config(pool, "github_app_id")
        .await
        .ok()
        .flatten()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("GITHUB_APP_ID")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })?;
    let private_key = crate::db::config::get_config(pool, "github_private_key")
        .await
        .ok()
        .flatten()
        .filter(|k| !k.trim().is_empty())
        .or_else(crate::github_jwt::resolve_private_key_from_env)?;
    Some((app_id, private_key))
}

/// Shared application state available to every handler via `State<AppState>`.
#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub redis: StateStore,
}

/// Public routes (setup wizard + auth). No session required.
pub fn public_router() -> Router<AppState> {
    Router::new()
        .nest("/api/setup", setup::router())
        .nest("/api/auth", auth::router())
        .merge(openapi::router())
}

/// Authenticated dashboard routes.
pub fn protected_router() -> Router<AppState> {
    Router::new()
        .nest("/api/stats", stats::router())
        .nest("/api/repos", repos::router())
        .nest("/api/reviews", reviews::router())
        .nest("/api/settings", settings::router())
        .nest("/api/learning", learning::router())
        .nest("/api/mcp", mcp::router())
        .nest("/api/github", github::router())
        .nest("/api/audit", audit::router())
        .nest("/api/users", users::router())
}

/// Build the full API route tree. Prefer [`build_router`] when you have state
/// so auth middleware can be applied correctly.
pub fn router() -> Router<AppState> {
    public_router().merge(protected_router())
}

/// Build API router with auth middleware bound to `state`.
pub fn build_router(state: AppState) -> Router {
    let protected = protected_router()
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::auth_middleware,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit::api_rate_limit_middleware,
        ))
        .with_state(state.clone());

    public_router().with_state(state).merge(protected)
}
