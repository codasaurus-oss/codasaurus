use axum::extract::{ConnectInfo, Path, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use crate::db;
use crate::redis::StateStore;

use super::errors::ApiError;
use super::rbac;
use super::AppState;

const SESSION_COOKIE: &str = "codasaurus_session";
const SESSION_MAX_AGE: &str = "604800"; // 7 days

const LOGIN_RATE_LIMIT: u32 = 10;
const LOGIN_RATE_WINDOW: Duration = Duration::from_secs(15 * 60);

#[derive(Deserialize)]
pub struct LoginBody {
    pub email: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub user: UserInfo,
}

#[derive(Serialize)]
pub struct MeResponse {
    pub authenticated: bool,
    pub user: Option<UserInfo>,
}

#[derive(Serialize)]
pub struct UserInfo {
    pub email: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_provider: Option<String>,
    #[serde(default)]
    pub is_bootstrap: bool,
}

#[derive(Deserialize)]
pub struct AcceptInviteBody {
    pub email: Option<String>,
    pub password: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/me", get(me))
        .route("/logout", post(logout))
        .route("/oidc/status", get(oidc_status))
        .route("/oidc/login", get(oidc_login))
        .route("/oidc/callback", get(oidc_callback))
        .route("/invite/{token}", get(invite_info))
        .route("/invite/{token}/accept", post(accept_invite))
}

/// Extract the session token from the Cookie header.
pub(crate) fn extract_token(headers: &axum::http::HeaderMap) -> Option<String> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    for pair in cookie.split(';') {
        let pair = pair.trim();
        if let Some(val) = pair.strip_prefix(&format!("{SESSION_COOKIE}=")) {
            return Some(val.to_string());
        }
    }
    None
}

/// Secure cookies by default. Opt out with `CODASAURUS_INSECURE_COOKIES=1` or
/// whenever `PUBLIC_URL` isn't `https://` (plain `http://` can't store a
/// `Secure` cookie at all — the browser silently drops it, so anything short
/// of explicitly opting into HTTPS would just log everyone out after login).
fn cookie_should_be_secure() -> bool {
    if std::env::var("CODASAURUS_SECURE_COOKIES")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    {
        return true;
    }
    if std::env::var("CODASAURUS_INSECURE_COOKIES")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    {
        return false;
    }
    if let Ok(url) = std::env::var("PUBLIC_URL") {
        if !url.to_ascii_lowercase().starts_with("https://") {
            return false;
        }
    }
    true
}

fn set_cookie(token: &str) -> String {
    let mut cookie = format!(
        "{SESSION_COOKIE}={token}; HttpOnly; Path=/; Max-Age={SESSION_MAX_AGE}; SameSite=Lax"
    );
    if cookie_should_be_secure() {
        cookie.push_str("; Secure");
    }
    cookie
}

fn clear_cookie() -> String {
    let mut cookie = format!("{SESSION_COOKIE}=; HttpOnly; Path=/; Max-Age=0; SameSite=Lax");
    if cookie_should_be_secure() {
        cookie.push_str("; Secure");
    }
    cookie
}

fn auth_rate_key(peer: IpAddr, headers: &axum::http::HeaderMap, email: &str) -> String {
    let ip = super::rate_limit::client_ip(peer, headers, super::rate_limit::trusted_proxies());
    let digest = Sha256::digest(format!("{ip}\0{}", email.trim().to_lowercase()).as_bytes());
    format!("rl:auth:{}", hex::encode(digest))
}

async fn check_auth_rate_limit(
    store: &StateStore,
    peer: IpAddr,
    headers: &axum::http::HeaderMap,
    email: &str,
) -> Result<(), ApiError> {
    super::rate_limit::check(
        store,
        &auth_rate_key(peer, headers, email),
        LOGIN_RATE_LIMIT,
        LOGIN_RATE_WINDOW,
    )
    .await
    .map_err(|retry_after| {
        ApiError::too_many_requests_after(
            "Too many attempts. Try again in 15 minutes.",
            retry_after.as_secs().max(1),
        )
    })
}

pub(crate) async fn require_session(
    pool: &crate::db::DbPool,
    headers: &axum::http::HeaderMap,
) -> Result<String, StatusCode> {
    let token = extract_token(headers).ok_or(StatusCode::UNAUTHORIZED)?;
    let email = db::sessions::get_session(pool, &token)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    Ok(email)
}

/// Axum middleware that requires a valid session cookie.
pub async fn auth_middleware(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: Next,
) -> Result<Response, ApiError> {
    require_session(&state.pool, req.headers())
        .await
        .map_err(|_| ApiError::unauthorized("Authentication required"))?;
    Ok(next.run(req).await)
}

/// POST /api/auth/login
#[utoipa::path(post, path = "/api/auth/login", tag = "auth", responses((status = 200, description = "OK")))]
pub(crate) async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
    Json(body): Json<LoginBody>,
) -> Result<impl IntoResponse, ApiError> {
    check_auth_rate_limit(&state.redis, peer.ip(), &headers, &body.email).await?;

    let user = db::users::verify_password(&state.pool, &body.email, &body.password)
        .await
        .map_err(|_| ApiError::unauthorized("Invalid email or password"))?
        .ok_or_else(|| ApiError::unauthorized("Invalid email or password"))?;

    // A session row is the login — failing to create one must fail the
    // request, not return 200 with a cleared cookie that looks logged out.
    let token = db::sessions::create_session(&state.pool, &user.email)
        .await
        .map_err(ApiError::internal_error)?;
    db::audit::log_event(
        &state.pool,
        "user.login",
        Some(&user.email),
        Some("user"),
        None,
    )
    .await;
    let cookie = set_cookie(&token);

    Ok((
        [(header::SET_COOKIE, cookie)],
        Json(LoginResponse {
            user: UserInfo {
                email: user.email,
                role: rbac::normalize_role(&user.role).to_string(),
                auth_provider: None,
                is_bootstrap: user.is_bootstrap,
            },
        }),
    ))
}

/// GET /api/auth/me
#[utoipa::path(get, path = "/api/auth/me", tag = "auth", responses((status = 200, description = "OK")))]
pub(crate) async fn me(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Json<MeResponse> {
    if let Some(token) = extract_token(&headers) {
        if let Ok(Some(user)) = db::sessions::get_session_user(&state.pool, &token).await {
            return Json(MeResponse {
                authenticated: true,
                user: Some(UserInfo {
                    email: user.email,
                    role: rbac::normalize_role(&user.role).to_string(),
                    auth_provider: Some(user.auth_provider),
                    is_bootstrap: user.is_bootstrap,
                }),
            });
        }
    }

    Json(MeResponse {
        authenticated: false,
        user: None,
    })
}

/// POST /api/auth/logout
#[utoipa::path(post, path = "/api/auth/logout", tag = "auth", responses((status = 200, description = "OK")))]
pub(crate) async fn logout(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if let Some(token) = extract_token(&headers) {
        let _ = db::sessions::delete_session(&state.pool, &token).await;
    }
    (
        [(header::SET_COOKIE, clear_cookie())],
        Json(json!({ "status": "ok" })),
    )
}

#[utoipa::path(get, path = "/api/auth/oidc/status", tag = "auth", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn oidc_status() -> Json<serde_json::Value> {
    Json(json!({ "enabled": crate::oidc::OidcConfig::enabled() }))
}

fn oidc_login_error(err: impl std::fmt::Display) -> ApiError {
    tracing::error!(error = %err, "failed to start OIDC sign-in");
    ApiError::bad_request("Unable to start sign-in")
}

#[utoipa::path(get, path = "/api/auth/oidc/login", tag = "auth", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn oidc_login(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let cfg = crate::oidc::OidcConfig::from_env()
        .ok_or_else(|| ApiError::bad_request("OIDC is not configured"))?;
    let (url, _state) = crate::oidc::authorization_url(&cfg, &state.redis)
        .await
        .map_err(oidc_login_error)?;
    Ok(axum::response::Redirect::temporary(&url))
}

#[derive(Deserialize)]
pub(crate) struct OidcCallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

#[utoipa::path(get, path = "/api/auth/oidc/callback", tag = "auth", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn oidc_callback(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<OidcCallbackParams>,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(err) = params.error {
        return Err(ApiError::bad_request(format!("OIDC error: {err}")));
    }
    let code = params
        .code
        .ok_or_else(|| ApiError::bad_request("missing code"))?;
    let state_param = params
        .state
        .ok_or_else(|| ApiError::bad_request("missing state"))?;
    let code_verifier = crate::oidc::take_pending(&state.redis, &state_param)
        .await
        .ok_or_else(|| ApiError::bad_request("invalid or expired OIDC state"))?;
    let cfg = crate::oidc::OidcConfig::from_env()
        .ok_or_else(|| ApiError::bad_request("OIDC is not configured"))?;
    let email = crate::oidc::exchange_code(&cfg, &code, &code_verifier)
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    let existing = db::users::get_user_by_email(&state.pool, &email)
        .await
        .map_err(ApiError::internal_error)?;
    // The user row is only needed for its side effect (create on first OIDC
    // sign-in); the session below keys off the verified email.
    if existing.is_none() {
        match db::invites::accept_oidc(&state.pool, &email)
            .await
            .map_err(ApiError::internal_error)?
        {
            Some(_) => {}
            None => {
                let open_join = crate::util::env_flag("OIDC_ALLOW_OPEN_JOIN");
                if open_join {
                    db::users::upsert_oidc_user(&state.pool, &email, "viewer")
                        .await
                        .map_err(ApiError::internal_error)?;
                } else {
                    return Err(ApiError::bad_request(
                        "No invite found for this email. Ask an admin for an invite link.",
                    ));
                }
            }
        }
    }
    let token = db::sessions::create_session(&state.pool, &email)
        .await
        .map_err(ApiError::internal_error)?;
    db::audit::log_event(&state.pool, "user.login", Some(&email), Some("user"), None).await;
    let cookie = set_cookie(&token);
    Ok((
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, cookie),
            (header::LOCATION, "/#/app/dashboard".into()),
        ],
    ))
}

/// GET /api/auth/invite/:token
#[utoipa::path(get, path = "/api/auth/invite/{token}", tag = "auth", params(("token" = String, Path, description = "Invite token")), responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn invite_info(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let invite = db::invites::get_pending_by_token(&state.pool, &token)
        .await?
        .ok_or_else(|| ApiError::not_found("Invite not found or expired"))?;
    Ok(Json(json!({
        "email": invite.email,
        "role": invite.role,
        "expires_at": invite.expires_at,
        "email_locked": invite.email.is_some(),
    })))
}

/// POST /api/auth/invite/:token/accept
#[utoipa::path(post, path = "/api/auth/invite/{token}/accept", tag = "auth", params(("token" = String, Path, description = "Invite token")), responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn accept_invite(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
    Path(token): Path<String>,
    Json(body): Json<AcceptInviteBody>,
) -> Result<impl IntoResponse, ApiError> {
    let invite = db::invites::get_pending_by_token(&state.pool, &token)
        .await?
        .ok_or_else(|| ApiError::not_found("Invite not found or expired"))?;

    let email = if let Some(locked) = &invite.email {
        if let Some(provided) = body.email.as_ref().map(|e| e.trim().to_lowercase()) {
            if !provided.is_empty() && provided != locked.to_lowercase() {
                return Err(ApiError::bad_request(
                    "This invite is locked to a different email",
                ));
            }
        }
        locked.clone()
    } else {
        let e = body
            .email
            .as_ref()
            .map(|e| e.trim().to_lowercase())
            .filter(|e| !e.is_empty())
            .ok_or_else(|| ApiError::bad_request("Email is required"))?;
        if !e.contains('@') {
            return Err(ApiError::bad_request("Invalid email address"));
        }
        e
    };

    check_auth_rate_limit(&state.redis, peer.ip(), &headers, &email).await?;

    if let Err(msg) = db::users::validate_password_policy(&body.password, &email) {
        return Err(ApiError::bad_request(msg));
    }

    if db::users::get_user_by_email(&state.pool, &email)
        .await?
        .is_some()
    {
        return Err(ApiError::bad_request(
            "A user with that email already exists",
        ));
    }

    let user = db::invites::accept_local(&state.pool, &invite, &email, &body.password)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref db_err) = e {
                if db_err.message().contains("UNIQUE") {
                    return ApiError::bad_request("A user with that email already exists");
                }
            }
            if let sqlx::Error::Protocol(ref msg) = e {
                if msg.contains("Invite not found") {
                    return ApiError::bad_request(msg.clone());
                }
            }
            ApiError::internal_error(e)
        })?;

    let session = db::sessions::create_session(&state.pool, &user.email)
        .await
        .map_err(ApiError::internal_error)?;
    db::audit::log_event(
        &state.pool,
        "user.accept",
        Some(&user.email),
        Some("invite"),
        Some(invite.id),
    )
    .await;

    Ok((
        [(header::SET_COOKIE, set_cookie(&session))],
        Json(json!({
            "user": {
                "email": user.email,
                "role": user.role,
            }
        })),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;
    use http_body_util::BodyExt;

    #[test]
    fn set_cookie_includes_httponly() {
        let c = set_cookie("abc");
        assert!(c.contains("HttpOnly"));
        assert!(c.contains("SameSite=Lax"));
    }

    #[tokio::test]
    async fn oidc_login_redis_error_body_omits_detail() {
        let err = oidc_login_error(anyhow::anyhow!("SENTINEL_OIDC_REDIS_DETAIL"));
        let body = String::from_utf8(
            err.into_response()
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(
            !body.contains("SENTINEL_OIDC_REDIS_DETAIL"),
            "response leaked Redis detail: {body}"
        );
        assert_eq!(body, r#"{"error":"Unable to start sign-in"}"#);
    }
}
